# Post-merge red-team audit: x86-64 i128 multiply and `typeof` recovery

Date: 2026-11-11. Baseline: `fc8350a46fc97688d10d0f12183e911ab6e7c58d`
(`main`, merge of PR #1174). Branch: `arena/a815e8b4-lccc`.

## Environment and hard constraints

* Pinned `harness-cache-8` (`d66c5ac881e2`) restored with
  `scripts/restore_agent_toolchain_cache.py`; verified 12 files.
* Rust 1.99.1, GCC 16.2.0, Clang 23.1.5, GDB 18.1, Valgrind 3.19.0,
  binutils 2.47, qemu-user 11.1.3, PyYAML 6.0.2 / pyelftools 0.31 /
  capstone 5.0.3.
* Exactly 4 GiB active swap (`/swapfile`), `CARGO_BUILD_JOBS=1`,
  `RUSTFLAGS=-D warnings`, fastbuild preset used for every compiler build.
* No local ICX 2026.2, `pahole`, `objtool` or `perf`. Hosted authenticated ICX
  evidence is used instead; local claims do not include those tools.
* No claim of overall compiler victory, wall-clock gain, green full local
  contract, final green CI, per-function maximal efficiency, or zero issues
  until each is measured and reported.

## 1. x86-64 generic 128x128 multiply

### Finding

`src/backend/x86/codegen/i128_ops.rs::emit_i128_mul_impl` emitted 13
instructions with `pushq %rdx; pushq %rax; ...; popq %rax; popq %rdi`. The
peephole removed one save pair at `-O2`, leaving a 9-instruction sequence with
one transient 8-byte RSP shift:

```asm
    pushq %rdx
    movq %rsi, %r9
    popq %rdi
    imulq %rcx, %rdi
    movq %rax, %rsi
    imulq %r9, %rsi
    mulq %rcx
    addq %rdi, %rdx
    addq %rsi, %rdx
```

`emit_i128_prep_binop_impl` documents the historical gzip mtime
miscompile caused by push/pop RSP shifts. The current peephole is stronger than
when that comment was written: `dead_code.rs::rsp_delta_step` /
`canonical_frame_off` canonicalise shifted `%rsp` displacements, and
`store_forwarding.rs` invalidates every slot mapping at `Push`/`Pop`.
Therefore this audit does **not** assert a present miscompile. It does assert a
real design and performance debt: a stack round-trip on every dynamic 128x128
product, plus continued reliance on a text model correctly reconstructing a
transient stack delta.

### Alternatives considered

1. **Keep push/pop, trust the peephole.** Rejected: two memory operations,
   two RSP writes, and a correctness dependency on every text pass that keys
   slots by textual displacement.
2. **Use a 9-instruction register-only sequence with a redundant
   `movq %r8, %rax` before `mulq`.** Rejected: `%rax` already holds `lhs.lo`
   after `emit_i128_prep_binop_impl`; the move is dead.
3. **Introduce `UMulHigh` IR ops / shared cross-ISA lowering.** Rejected for
   this patch: i686 rejects high multiplies, AArch64/RISC-V already have
   shorter target-specific sequences, and a shared IR change would be a
   multi-week allocator/legalisation programme (RAP-04/RAP-05), not a
   reviewable fix.
4. **8-instruction register-only schoolbook product.** Selected.

### Selected sequence

After `emit_i128_prep_binop_impl`: `%rax=lhs.lo`, `%rdx=lhs.hi`,
`%rcx=rhs.lo`, `%rsi=rhs.hi`. `%r8/%r9/%rdi/%rsi` are withdrawn from the
general caller-saved pool for wide bodies by
`regalloc.rs::x86_body_has_i128_ops`, with point-local admission guarded by
`collect_x64_wide_hazard_points`. The new sequence uses `%rdi`, `%r8`, `%rcx`
and `%rsi` only:

```asm
    movq %rcx, %rdi      # rdi = rhs.lo
    movq %rax, %r8       # r8  = lhs.lo
    movq %rdx, %rcx      # rcx = lhs.hi
    imulq %rdi, %rcx     # rcx = lhs.hi * rhs.lo (mod 2^64)
    imulq %rsi, %r8      # r8  = lhs.lo * rhs.hi (mod 2^64)
    mulq %rdi            # rdx:rax = lhs.lo * rhs.lo
    addq %rcx, %rdx      # high += lhs.hi * rhs.lo
    addq %r8, %rdx       # high += lhs.lo * rhs.hi
```

The 128-bit product modulo 2^128 is `(a0*b0 mod 2^64) + 2^64 *
(high(a0*b0) + a1*b0 + a0*b1 mod 2^64)`. Signed and unsigned products have
identical bit patterns, so two-operand `imulq` (wrapping 64-bit low halves) is
correct for both `I128` and `U128`. `mulq` does not consume flags, and the two
`addq` carries above bit 63 are deliberately discarded.

Instruction count: 13 emitted -> 8 emitted; observed `-O2` text 9 -> 8; stack
operations 4 -> 0. No wall-clock gain is claimed.

## 2. Other push/pop sites, audited line by line

| Site | Verdict | Reason |
|---|---|---|
| `i128_ops.rs::emit_i128_divrem_call_impl` | keep | Two pushes stage RHS; `rsp_frame_size += 16` before the LHS slot reload, then restores it before pops. Pops place RHS low/high into `%rdx/%rcx` per SysV `__udivti3` ABI. Existing `x86_fpo_i128_div_push_depth.c` pins FPO depth. |
| `emit.rs::prep_i128_binop` (x86) | **delete** | Unreferenced twin of the live register-only prep helper; it still contained the historical push/pop sequence. Removing it eliminates a dead, hazardous alternative. |
| `alu.rs` SWAR popcount | keep | Explicit `%rcx`/`%rdx` saves with no call; `dead_scratch_saves` deletes provably dead pairs. Replacing them with `%r8/%r9` would require a new allocator liveness proof; no measured hotspot evidence justifies that risk here. |
| `atomics.rs::with_rsp_bias` | keep | Every pushed temporary biases `rsp_frame_size` through a scoped helper; no unadjusted `%rsp` slot access in the window. |
| `f128.rs::emit_f128_store_f64_via_x87` | keep | `pushq`, `fldl (%rsp)`, then `addq $8, %rsp` restores canonical RSP *before* `emit_instr_rbp` writes the frame slot; no biased displacement is needed. |
| i686 `emit_i128_mul_impl` | keep | Explicit `esp_adjust` accounting and `%esp` RHS slots. C has no `__int128` on i686; the path is a container primitive, not a shared fix target. |

## 3. Cross-ISA scope

| Backend | Generic multiply | Decision |
|---|---|---|
| x86-64 | old: 13 instr, 4 stack ops; new: 8 instr, 0 stack ops | fixed |
| AArch64 | `mul`, `umulh`, `madd`, `madd` | unchanged; already register-only |
| RISC-V | `mul`, `mulhu`, two `mul`, two `add` | unchanged; already register-only |
| i686 | 8 `%esp`-based instructions with `esp_adjust` | unchanged; distinct stack contract |

The regression suite compiles raw AArch64/RISC-V assembly and ELF objects,
checks `umulh`/`mulhu`, rejects pushes in their multiply bodies, and compiles
the existing i686 16-byte struct-return carrier to prove the i386 path still
assembles. Hosted CI's cross-execute and authenticated ICX lanes remain the
authority for runtime cross-ISA claims; local qemu execution is not claimed
unless its gate actually ran.

## 4. `typeof` unresolved fallback

The DA follow-up asked for a reachable reproducer or deletion of the TODO with
a probe matrix. We probed 15 unresolvable shapes against LCCC, GCC 16.2 and
Clang 23.1.5; every compiler rejected every shape with a named unresolved
operand. A 7-result positive `typeof` matrix (identifier, address-of, deref,
binary, subscript, member, `__typeof__`, statement-expression scope) is
bit-identical. The fallback in
`ExprTypeChecker::resolve_type_spec_with_scope` is therefore documented as
recovery-only: sema has already emitted the diagnostic, and a second one at
the fallback would double-report. `CType::Int` is retained only so the
declaration has a type while the TU is known to fail.

## 5. Regression design

* `tests/corpus/test_x86_i128_mul_redteam.py`: source-level 8-instruction
  ratchet, historical-sequence negative control, dead-helper removal,
  cross-ISA source invariants, 64-case differential vs GCC/Clang/ICX,
  `-O2`/`-O3`, default / `LCCC_NO_PEEPHOLE` / `CCC_NO_PEEPHOLE`, text assembly
  inspection, ELF `e_machine` checks, and Capstone decoding of the emitted
  `wide_mul` symbol when pyelftools/capstone are present.
* `tests/regression/i128_dynamic_mul_rsp_{O2,O3,raw,nopeeph}.c` plus one
  shared header: auto-discovered compile/run/compare fixtures at both
  optimisation levels and both peephole bypasses.
* `tests/corpus/test_typeof_unresolved_fallback.py`: source-level TODO
  ratchet, 15-shape negative matrix, positive differential matrix.
* `tests/regression/typeof_recovery_matrix.c`: auto-discovered positive
  `typeof` runtime oracle.
* Rust unit tests in `i128_ops.rs`: register-only emitter contract, historical
  negative control, and a 1,000-case 64-bit limb model compared against
  `u128::wrapping_mul`.

## 6. Validation log

### Baseline, before source changes (`fc8350a`)

* `scripts/build_lccc_fast.sh`: rc=0, 5m24s, all five fastbuild binaries
  produced under `RUSTFLAGS=-D warnings` with exactly 4 GiB active swap.
* `test_x86_i128_mul_redteam.py`: `Ran 6 tests`, `FAILED (failures=8)`.
  The 8 failures are the 8 expected source/assembly assertions that pin the
  old 13-instruction emitter: 1 function-shape, 1 dead-helper, and 6
  `-O2`/`-O3` × default/raw/pass-disabled subtests. The 3 structural
  cross-ISA/historical controls already pass.
* `test_typeof_unresolved_fallback.py`: `Ran 4 tests`, `FAILED (failures=1)`.
  The 15-shape negative matrix and positive differential matrix already pass;
  only the source-level TODO ratchet fails, as designed.
* `LCCC_NO_PEEPHOLE=1 -O2 -fomit-frame-pointer -S` on the new dynamic fixture
  shows the historical `pushq %rdx / pushq %rax / ... / popq %rdi / mulq %r8`
  sequence in `wide_mul` (`/tmp/baseline_raw.s:349-363`).
* Raw AArch64 assembly shows `umulh` + two `madd`; raw RISC-V assembly shows
  `mul` / `mulhu` / `mul` / `add` / `mul` / `add`. Both are already
  register-only, so no cross-ISA edit is warranted.

### Post-fix gates

Filled after the source patch, rebuild, and strict revalidation. No result is
asserted here before the command producing it has exited successfully.
