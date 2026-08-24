# Session 75 (v7) — Agent C patch audit + PF-06 + miscompile fix

Date: 2026-08-24
Base: `acb61d1` (origin/main after PR #224). Branch `v7-work` builds on top
of v6 (`1042c64` PF-06 SIB infra) with Agent C's session-75 patch plus v7's
own follow-ups.

## 1. Audit of Agent C's session-75 patch

Source: `ms178-1-AgentC.patch` (2715 lines) authored against `daf3f48`.
Applied cleanly on top of `acb61d1` + `1042c64`.

### 1.1 What was applied unchanged

- `peephole/passes/liveness.rs` — exact CFG dataflow over 16 GP families.
  Conservative for indirect jumps, tail calls to symbols, unknown
  mnemonics, missing CFI. Provides the primary deadness answer for the
  peephole layer; the two syntactic proofs (block-local scan +
  whole-function "no other mention") remain as a fallback union for
  sub-register cases the dataflow must be pessimistic about.
- `copy_coalesce::coalesce_register_copies` — parameter-shuffle retirement.
  Sound: requires full 64-bit `movq`, source dead after the copy, source
  mentioned nowhere else, copy sits in the straight-line entry run,
  destination has no implicit ABI reader (`ret`, call, shift).
- `dead_writes::eliminate_dead_pure_writes` — deletes pure writes (LEA,
  `movz*`, `setCC`, `cmov`, copies) to dead registers. Soundness: includes
  cmov (its old value is dead too, since the family dies).
- `dead_writes::fold_load_test_into_cmp` — `movsbq (%rbx), %rsi; testq
  %rsi, %rsi` → `cmpb $0, (%rbx)`. Flag-for-flag identical.
- `dead_writes::fold_accumulator_roundtrip` — `movq %r8, %rax; OP %r10,
  %rax; movq %rax, %r8` → `OP %r10, %r8`. Sound when scratch is dead
  after copy-back, accumulator not read between copies, operand is not
  the accumulator/scratch.
- `dead_writes::reuse_redundant_loads` — repeated load of same address
  becomes a copy. Crosses fall-through-only labels (block domination).
  Stops at writes-to-memory, address-register writes, call, ret,
  join-point labels.
- `flag_peepholes::eliminate_redundant_self_test` — `andl $1, %esi; testq
  %rsi, %rsi` → `andl $1, %esi`. Width rules: 32-bit logical op + 64-bit
  test only valid when consumers test ZF (because SF differs).
- `flag_peepholes::narrow_dead_sign_extension` — `movslq %edx, %r8` →
  `movl %edx, %r8d` when 64-bit half is never read. Unblocks copy folds.
- `relay_and_lea` store relay — `movl %A, %D; movl %D, MEM` → `movl %A, MEM`.
- `BACKLOG` PF-04 marked DONE; PF-08..PF-14 shipped; PF-15..PF-17
  designed/open.
- `check_gpr_leaf_param_codegen.sh` — accepts either the 3-move
  parallel parameter shuffle OR the fully-coalesced form. When any move
  is still emitted, all must be present and correctly ordered.

### 1.2 The miscompile I found and fixed

Agent C's `reuse_redundant_loads` substituted a second sign-extending-
to-64-bit load (`movsbq/movswq/movslq MEM, %rX`) with `movl %src, %dst`,
which zero-extends the upper 32 bits of `%dst` — destroying the sign
extension the second load would have produced.

For a `0x80` byte the original produced `%rax = 0xFFFFFFFFFFFFFF80` and
the rewrite produced `%rax = 0x00000000FFFFFF80` — a miscompile whenever
the second load's 64-bit value was consumed.

The fix picks the copy width by the load's WIDTH CLASS:

| Load class | Copy | Why |
|---|---|---|
| `movq MEM, %rX` | `movq` | full 64-bit copy preserves all bits |
| `movsbq/movswq/movslq MEM, %rX` (sign-extending to 64-bit) | `movq` | first load's `%src` already holds the sign-extended 64-bit value; `movl` would zero-extend |
| `movl/movzbl/movzwl/movzbq/movzwq/movsbl/movswl MEM, %rX` | `movl` | upper 32 bits zero in both load and copy |
| `movb/movw MEM, %rX` | refused | preserve upper bits; `movl/movq` clobbers |

Three new unit tests (`repeated_sign_extending_load_uses_movq_to_preserve_upper_bits`,
`repeated_movslq_load_uses_movq`, `repeated_movzbl_load_still_uses_movl`)
lock the sign-extending case in.

## 2. PF-06: add(iv, const) SIB displacement peeling (the v6 soundness gap, closed)

The v6 commit `1042c64` built the `IndexedGepInfo.disp` infrastructure
but REVERTED the actual `add(iv, const)` peeling after 5 regression
failures exposed a soundness bug:

> "When the GEP's base already contains the index value, the SIB
> double-counts the index: `q = gep(p, iv); load(gep(q, add(iv, 1)))`
> would produce SIB `1(q, iv, 1)` = `q + iv + 1` = `(p + iv) + iv + 1`
> = `p + 2*iv + 1`."

v7 re-enables the peeling with TWO soundness gates:

### 2.1 Gate 1: base-contains-iv (the documented v6 concern)

`base_chain_contains_iv(defs, base_id, iv_id)` walks the GEP base's SSA
definition chain. If the IV value appears transitively, the SIB would
double-count the index → refuse the fold.

### 2.2 Gate 2: iv-update Copy coalescing (the actual v6 miscompile)

The v6 "double-counting" math is actually SOUND when `disp = const *
scale`. The REAL v6 miscompile is different: when the add's RESULT is
used as the source of a Copy that redefines the IV (`v_iv = copy
v_add_result`), the RA coalesces the add's result with the IV's
register. After coalescing, the add `v_add_result = v_iv + const`
becomes an in-place `add $const, %reg` that overwrites the IV's
register. If the scheduler places this add BEFORE the SIB load (which
uses the IV's register as the SIB index), the SIB reads the NEW IV
value (= old IV + const) instead of the OLD IV value.

Pre-scan the function for any `Copy { dest: iv, src: v_add_result }`
shape and record the (iv, add_result) pairs. If the add's RESULT is in
that set, refuse the fold.

**Critical detail**: the check uses `(iv_id, add_result_id)`, NOT
`(iv_id, off_id)`. The iv-update Copy uses the add's RESULT, which is
the value BEFORE the outer `shl/mul` peel steps. `off_id` is the
OUTERMOST value (e.g. the `shl`'s result), which is NOT what the
iv-update Copy uses.

This catches:

- `accumulator_pointer_load`: `v49 = add(v69, 1); v51 = gep(v50, v49);
  v52 = load(v51); ...; v69 = copy(v49)` — the iv-update Copy uses v49
  (the add's result). The (69, 49) pair is in the set. Fold refused.
  CORRECT.
- `affine_map_vectorization`'s shifted_overlap_f64 reference loop:
  `v378 = add(v428, 1); v380 = shl(v378, 3); v381 = gep(&rd, v380);
  ...; v428 = copy(v378)` — the iv-update Copy uses v378 (the add's
  result), NOT v380 (the shl's result). The (428, 378) pair catches
  this. CORRECT.

It does NOT refuse the sound `prefix_sum` case:
`v11 = sub(v25, 1); v13 = shl(v11, 2); v14 = gep(v2, v13); v15 =
load(v14); ...; v24 = add(v25, 1); v25 = copy(v24)` — the iv-update
Copy uses v24 (a different add), NOT v11 (the sub we're peeling). The
(25, 11) pair is NOT in the set. Fold fires, producing
`mov -0x4(%rdi, %r12, 4), %r9d` — a single SIB load replacing the
4-instruction `lea -1(%r12); cltq; lea 0(,%rax,4); mov` chain.

### 2.3 Kill switch

`CCC_NO_PF06_ADD_PEL=1` disables ONLY the `add/sub(iv, const)` peeling.
The existing SIB fold without displacement stays on. Useful for
bisecting if another soundness corner case appears.

### 2.4 Measurements

`scripts/kernel_count.py` (per-function instruction counts vs system
GCC at -O2):

| Kernel | v6 | v7 | GCC |
|---|---:|---:|---:|
| isort | 43 | 40 | 23 |
| adler8 | 89 | 89 | 63 |
| others | unchanged | unchanged | — |
| **TOTAL** | **325** | **322** | **264** |

(Other kernels were already at their v6 best after Agent C's
peepholes; PF-06's `add(iv, const)` only fires on patterns where the
GEP's offset is `add(iv, const)`, e.g. `a[j+1]`, `arr[i-1]`.)

## 3. Validation

```
cargo test --lib --profile fastbuild --locked -j 2     1165 passed; 0 failed
tests/regression/run_regression.py                     451 passed; 3 failed (i686 env-only);
                                                       9 skipped-compare; 2 skipped-run
tests/correctness/run_correctness.py                   50 passed; 0 failed
CCC_VERIFY_REGALLOC=1 correctness                       50 passed; 0 failed
.github/scripts/ci-codegen-gate.py                     7/7 PASS (within tolerance)
tests/fuzz/differential_fuzz.py (O2,O3, 0:300)         600/600 PASS
```

## 4. What v7 does NOT close

The worst benchmarks remain:

| Benchmark | v6 | v7 | Root cause |
|---|---:|---:|---|
| spectral_norm | 4.779× | ~4.8× | non-reduction FP vectorization (multi-store stencil) |
| nbody | 3.889× | ~3.8× | multi-store FP across two IVs + field-sensitive disambiguation |
| loop_patterns | 2.435× | ~2.8× | NOISE — codegen identical between v6 and v7; VM is 2-vCPU |
| adler32 | 1.580× | 1.59× | RA-06: arithmetic-chain copy webs (next-use-aware eviction) |
| expat | 1.609× | ~1.67× | hash-multiply chains (imul) |
| find_bit | 1.809× | ~1.96× | sparse-bit branchy decision tree (gcc andn+cmov) |
| struct_copy | 1.493× | ~1.48× | aggregate copy + ABI |
| sqlite_varint | 1.361× | ~1.28× | branchy int stores (improved by Agent C's load_reuse) |
| sieve | 1.304× | ~1.16× | branchy int stores (improved; VM-noisy) |
| mandelbrot | 1.653× | ~1.65× | FP branch-heavy loop |
| fannkuch | 1.784× | ~1.76× | permutations |

Closing the FP gaps (spectral_norm, nbody, mandelbrot) requires
multi-store stencil analysis — the stencil vectorizer only handles
reductions today. Closing adler32 requires RA-06 (next-use-aware
eviction with copy-web coalescing for arithmetic chains). Closing
find_bit requires recognizing the C ffs decision tree and lowering to
`tzcnt` (or matching GCC's `andn+cmov` shape).

These are documented in BACKLOG §16.4 and remain open for v8.
