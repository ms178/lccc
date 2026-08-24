# Session 74 — FP binop miscompile (root cause + fix), five more x86 peepholes, A/B gate

Date: 2026-08-24
Base: `d35353c` (latest `ms178/lccc` main; PRs #219/#220/#221 included).
Build: fastbuild profile, Rust 1.98.0, `-j2`, 8 GiB swap (`scripts/ensure_swap.sh`).

## 1. A real miscompile in `main`: FP binop destination aliasing

`emit_float_binop_into_reg` leaves the LHS in the XMM home of the result and
then applies the RHS. Three RHS materialisations wrote **into that same
register** first:

```text
addc:                                  # double addc(double x) { return x + 1.5; }
    movsd %xmm0, %xmm2                 # xmm2 = x
    movsd .LCFP_0(%rip), %xmm2         # xmm2 = 1.5   <-- x destroyed
    vaddsd %xmm2, %xmm2, %xmm2         # 1.5 + 1.5
```

Observed on `d35353c` (LCCC vs GCC 15, same source):

| expression | LCCC before | correct |
| --- | ---: | ---: |
| `x + 1.5`, x=1.0 | 3.0 | 2.5 |
| `x - 1.5`, x=1.0 | 0.0 | −0.5 |
| `x * 3.0`, x=2.0 | 9.0 | 6.0 |
| `x + 2.5f`, x=1.0f | 5.0 | 3.5 |
| `dot4` (4-element reduction) | 65.0 | 70.0 |

Provenance: the constant/GPR RHS paths used to stage the value in `%xmm1`
(correct); a previous session rewrote them to write `%reg` directly while
fixing a genuine `%xmm1`-aliasing bug in `load_fp_to_reg` (where writing the
target IS the contract). The `load_fp_to_reg` half of that change is right and
is kept; the binop half is the regression fixed here.

A second, older bug in the same function: when the destination register is the
**home of the right operand**, `load_fp_to_reg(lhs, .., reg)` destroys the RHS.
`acc = v[i] - acc` (`alternating_sub`) hits it.

### Fix

`emit_float_binop_into_reg` now classifies where the operands live and never
writes the destination while it holds an operand:

* dest holds lhs → `vop <rhs-src2>, %reg, %reg` (unchanged, was already right
  for XMM-home and stack-slot right operands);
* dest holds rhs, commutative (`add`/`mul`) → operands swap;
* dest holds rhs, `sub`/`div` → stage the LHS in the `xmm1` codegen scratch and
  use the 3-operand VEX form `vop %reg, %xmm1, %reg`;
* new helper `fp_binop_src2` materialises the RHS **as a source**: an XMM home
  or stack slot in place, a constant as a rip-relative memory operand
  (`vaddsd .LCFP_0(%rip), %xmm2, %xmm2` — the encoding the copysign path
  already emits), anything else through the GPR path into `xmm1`.

`xmm0`/`xmm1` are safe scratch by construction: `phys_reg_name` hands out
`xmm2..xmm15` for allocated F32/F64 homes.

Not adopted: folding `0.0 + x` to a plain load of `x`. It is wrong for
`x = -0.0` (the sum is `+0.0`) and the regression test pins that.

Regression: `tests/regression/fp_binop_dest_aliasing.c` (constants on both
sides, `float` and `double`, the dot-product reduction, `sub`/`div` with the
destination holding the RHS, and the signed-zero case) at `-O0/-O1/-O2/-O3`.

**Effect on the suite:** `tests/regression` went from *462 passed / 12 failed*
to *475 passed / 1 failed*. Running the 37 executable programs in
`tests/benchmark/programs` against GCC-built references: **main produces wrong
output for `nbody`** (36/37 match), the fixed tree matches all 37. The single
remaining regression-suite failure is the absent `aarch64-linux-gnu-gcc` cross
compiler. The eleven "FP/SIMD" failures that the
previous session's notes recorded as environment limits were all this
miscompile — that attribution was wrong and is corrected here.

## 2. Five new peepholes (default-on)

New module `flag_peepholes.rs` (flags-lifetime aware) and two more passes in
`relay_and_lea.rs`. All are gated by the standard skip set.

| pass | rewrite | why it is sound |
| --- | --- | --- |
| `retarget_producer_into_copy` | `movzbl (%rdx,%r12), %eax; movq %rax, %r10` → `movzbl (%rdx,%r12), %r10d` | only *pure* producers (no read-modify-write), producer must not mention the copy destination, width rule below, source register provably dead |
| `fold_copy_add_into_lea` | `movq %r9, %rax; addl $1, %eax` → `leal 1(%r9), %eax` | `lea` is flag-neutral, so the flags the `add` wrote must be provably dead; copy may be wider than the arithmetic, never narrower |
| `fold_copy_shift_into_lea` | `movq %rax, %r12; shlq $2, %r12` → `leaq 0(,%rax,4), %r12` | same flags rule; `local_patterns::fuse_copy_and_operation` documents this rewrite in its header comment but never implemented it |
| `fold_setcc_test_cmov` | `cmpb $0,%r11b; sete %r10b; movzbl %r10b,%r10d; …; testq %r10,%r10; cmovneq %r8,%rbx` → `cmpb $0,%r11b; …; cmoveq %r8,%rbx` | nothing between the `setCC` and the `cmov` may read or write flags, and the boolean register must be provably dead (three instructions removed) |
| `fold_copy_and_mask_into_movz` | `movl %eax,%r9d; andl $255,%r9d` → `movzbl %al,%r9d` | flags-dead rule; `movzbl` zeroes bits 8..63 exactly like the mask |

`fold_lea_into_load` was generalised from the single-base form to full
`DISP(%base,%index,scale)` addresses (still windowed, still with the two
deadness proofs).

### Width rules (the part that silently breaks otherwise)

* A 32-bit write zero-extends, so a 32-bit producer may feed a `movq` copy; a
  64-bit producer under a `movl` copy may **not** be retargeted (the copy
  discarded bits 32..63).
* For copy+arithmetic fusion the copy must be at least as wide as the
  arithmetic: `movq %r9,%rax; addl $1,%eax` folds to `leal 1(%r9), %eax`
  (32-bit result, zero-extended, 64-bit base register), whereas
  `movl %r9d,%eax; addq $1,%rax` must not become `leaq 1(%r9), %rax` — the
  `movl` zeroed the upper half that `%r9` still has.
* The LEA base/index is always spelled with the 64-bit name so the address
  computation stays in 64-bit mode.

### Flags model

`flag_peepholes::flags_effect` classifies a line as WRITES / READS / NEUTRAL,
with **unknown mnemonics treated as readers** (blocking both queries).
`flags_dead_after` walks forward: a writer first ⇒ dead, a reader first ⇒ live,
`ret` ⇒ dead (EFLAGS is not preserved across the SysV boundary), directives are
skipped, `push`/`pop` are transparent, any other control flow ⇒ conservative
"live".

## 3. Measurements

Kernel corpus (`scripts/kernel_count.py`, LCCC `-O2` vs GCC `-O2`,
per-function instruction counts):

| kernel | main | session 73 | **now** | GCC |
| --- | ---: | ---: | ---: | ---: |
| adler8 | 105 | 98 | **91** | 63 |
| crc32k | 30 | 29 | **27** | 18 |
| cntz | 22 | 22 | **18** | 13 |
| isort | 47 | 47 | **43** | 23 |
| my_strlen | 14 | 13 | **13** | 10 |
| hsh | 20 | 19 | **19** | 13 |
| bswp32 | 15 | 15 | **14** | 11 |
| **total** | **375** | **363** | **343** | **264** |

Whole-corpus A/B on the 38 programs in `tests/benchmark/programs`
(`scripts/peephole_ab.py`, new): **6964 → 6753 instructions (−3.03%)**, every
program equal or better, and **identical stdout + exit status everywhere**.
Largest wins: `gzip_crc32` −26, `zlib_ng_adler32` −23, `spectral_norm` −20,
`hash_table` −14, `bitops` −10.

`scripts/peephole_ab.py` is the A/B harness itself: it compiles each program
with and without the pass set, counts instructions, **and** compares runtime
output. A pass that changes behaviour anywhere fails the gate regardless of its
instruction count.

## 4. Validation

```text
cargo test --profile fastbuild --lib            1127 passed, 0 failed
                                                (26 new peephole unit tests)
tests/regression/run_regression.sh              475 passed, 2 skipped, 1 failed
                                                (aarch64 cross-gcc absent)
tests/correctness/run_correctness.py            50/50 vs GCC
scripts/peephole_ab.py                          38/38 programs, behaviour identical
tests/fuzz/differential_fuzz.py                 seeds 0:1700 x {O0,O2,O3,Os}
                                                6800 programs, 0 failures
tests/fuzz/phi_cfg_fuzz.py                      seeds 0:200 x {O0,O2,O3}
                                                600 programs, 0 failures
tests/benchmark/programs vs GCC                 37/37 identical stdout+status
                                                (main: 36/37 — nbody wrong)
tests/regression/peephole_reg_shuffle.c         new: executes every rewritten
                                                shape, -O0..-O3/-Os, vs GCC
kernel corpus execution                         15 kernels, -O0/-O1/-O2/-O3/-Os,
                                                LCCC objects + GCC driver, output
                                                identical to the all-GCC build
build                                           RUSTFLAGS="-D warnings" clean;
                                                clippy clean for the new modules
```

## 5. Reviewed and rejected

* **`scalar_storage_order` bitfield support (external patch).** Measured
  against GCC on the same source: for
  `struct __attribute__((scalar_storage_order("big-endian"))) S {unsigned a:9,b:1,c:1,d:1,e:1;}`
  with `{341,1,1,1,1}` GCC stores `aa f8 00 00`, that patch stores
  `00 00 f8 aa` (the storage unit is never byte-swapped), and plain scalar
  members — which the attribute also governs — are not handled at all
  (`unsigned x = 0x01020304` stays `04 03 02 01`). Values only round-trip
  inside one translation unit. Accepting the attribute while laying memory out
  differently from GCC is worse than today's silent ignore for the attribute's
  only real use case (wire/hardware formats). Correct design is recorded in the
  backlog (FE-SSO).
* **Register-allocator occupancy / segment-fill infrastructure (external
  patch).** Its own measurements show the occupancy path miscompiling
  `zlib_ng_adler32` and `switch_dispatch`, so it ships disabled; the residual
  fill is also opt-in. The default path is byte-identical to baseline, i.e. the
  delta is inert here, cannot be re-measured in this environment (the gate
  needs the workload harness), and is written against an older base. The idea
  (rank residual fill by multi-piece first, then group pressure, drop
  single-piece junk below a use threshold) is recorded as RA-05 with the
  reported numbers.

## 6. Follow-up

1. **RA-03/PF-05 — adler32 accumulator spills** (91 vs GCC 63, the single
   largest remaining kernel gap): the eight unrolled byte temporaries win the
   registers and `s1`/`s2` live in the frame.
2. **LICM for `leaq sym(%rip)`** (crc32 27 vs 18). Note the earlier backlog
   entry "crc SIB fold" was *wrong*: x86-64 cannot combine a rip-relative
   displacement with an index register, which is exactly why GCC hoists
   `leaq table(%rip), %rsi` out of the loop. Hoisting is the fix.
3. **isort (43 vs 23)**: `(j+1)*4` should be strength-reduced to a pointer
   walk; the remaining `cltq`/`movslq` pairs come from `int` index widening.
4. **`fold_setcc_test_cmov` for branches** (`setCC` + `test` + `jne` → `jCC`)
   — the same proof, applied to conditional jumps rather than `cmov`.
5. **Diagnose unsupported `scalar_storage_order`** instead of silently
   ignoring it (FE-SSO).
