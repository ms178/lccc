# Session 74 (2026-08-24) — Red-team audit of upstream PRs #222–#226 + v2 patch

**Base:** `ms178/lccc` `main` at `6df4956` (PRs #222–#226 merged since session
73's `daf3f48` base). Re-cloned after a harness workspace wipe; session-73
commits reconstructed from the persisted S01/S02 snapshot patches +
worklog design notes.

## Red-team audit of upstream's newly-merged work

### 742ca66 — `-ffp-contract=on`, segment-aware regalloc, stencil vectorization, relay/LEA peepholes

**Verdict: AGREE — sound, well-engineered, the right architectural calls.**

- **OP-36 tri-state FpContract**: `FpContract { Off, OnExpr, Fast }` replaces
  the bool throughout driver/passes/backends. `-ffp-contract=on` now works
  with GCC semantics: the frontend tags every FP Mul/Add/Sub with its
  statement-root id (`IrFunction::fp_expr_tags`), and the backend fuses
  `a*b+c` into FMA only when the tags match. This is the PROPER solution to
  the statement-boundary problem session 65 identified — far deeper than the
  flavor-level fix I had prototyped. Agree completely.
- **RA-05 segment-aware regalloc**: `LiveRange.segs: SegmentList` with
  `segments_conflict(other, adjacent_ok)` — the same core design I
  independently implemented in session 73. Their version is clean and
  well-tested. Phase 2f generalized to all targets with ranked candidate
  selection (multi-piece values first — better than my version). Agree.
- **OP-05a stencil pattern**: constant-tap stencil loops (affine `a*iv+b`
  tap offsets off a shared base). Complementary to my elementwise map
  expression trees (sqrt/div/sub/multi-stream) — both are needed; they
  target different loop shapes. The session-73 map trees rebase cleanly on
  top of the stencil code.
- **relay/LEA peepholes**: move-relay elimination + windowed LEA fold.
  Sound (two independent deadness proofs: block-local write-before-read with
  barrier abort, or whole-function textual uniqueness with ABI-implicit-read
  refusal). Good defensive engineering.

### 88c22c7 — flags-aware peephole cleanups, FP dest-aliasing test

**Verdict: AGREE — the FP dest-aliasing test (fp_binop_dest_aliasing.c) is
exactly the regression-safety net the FMA contraction path needed.**

### 4a4520c — exact peephole liveness, copy coalescing, PF-06 displacement peeling

**Verdict: AGREE — this is the most substantial piece and it's excellent.**

- **Exact peephole liveness** (liveness.rs, 671 LOC): a REAL backward dataflow
  fixpoint over the 16 GP register families with a successor graph. This
  replaces the old syntactic approximations (block-local write-before-read +
  whole-function "no other mention") with the actual answer to "is this
  register read on any path from here?". Critically: **conservative by
  construction** — unanalysable functions (indirect jumps, jump tables, tail
  calls to symbols) return `None` and callers fall back to their syntactic
  proofs; unknown mnemonics assumed to read everything; inline asm
  reads/writes everything; `ret` live = return regs + callee-saved; `call`
  reads arg regs + rax + r10, clobbers caller-saved. Tested: loop-body-write-
  dead-when-only-prologue-reads, value-live-across-back-edge-not-dead,
  return-value-live-at-ret, argument-regs-live-into-call, indirect-jump-
  makes-unanalysable. This is the RIGHT design for a text-based peephole.
- **copy_coalesce.rs** (346 LOC): uses the exact liveness to coalesce
  register copies — addressing the self-move/reg-move misses my session-73
  IS-30 diagnostic detector was designed to flag. The fix machinery subsumes
  the diagnostic.
- **dead_writes.rs** (849 LOC): dead store elimination with the exact
  liveness as the deadness proof. Sound barriers (calls, inline asm,
  volatile).
- **PF-06 displacement peeling**: peeling loop-invariant displacement
  computations out of loops.

**Architectural note**: the OpenAI audit (§10) said peephole-on-text is
"structurally too late". Upstream's response: make the text peephole
POWERFUL enough (exact liveness) rather than migrating to MachInst. This is
a legitimate incremental choice — the dataflow fixpoint is sound, and a
MachInst migration is a multi-month project. Agree with the incremental
approach, while noting the long-term architectural tension remains (the
peephole is still cleaning up RA/coalescing misses; the RA-side segment
work in 742ca66 addresses the root cause in parallel).

### fe89f5f — target-aware integer constant hoisting

**Verdict: AGREE — eliminates per-iteration `movabsq` in div-heavy loops.
Sound, measured (`sum_div7` etc.).**

## What this session adds (complementary, not duplicative)

### FMA3 ISA gate (correctness)

`-ffp-contract=fast` alone no longer emits `vfmadd231s{s,d}` on baseline
(SSE2) targets — SIGILL on pre-Haswell hardware. `supports_fused_float_mul_
add/sub` now require the FMA3 ISA feature (`-mfma` / enabling `-march`) in
addition to `FpContract != Off`, exactly like GCC. `enable_fma` reaches
`CodegenOptions` (previously preprocessor `__FMA__` macros only); the
vectorizer's `VecFma`/`VecMadd` contraction is gated the same way via
`vectorize::set_x86_fma_enabled` (thread-local, set by `run_passes`).

### OP-05a elementwise map expression trees

Generalized the map vectorizer from the fixed affine family to bounded
expression trees (≤12 nodes) over up to four IV-indexed load streams: FP
Add/Sub/Mul/Div, integer Add/Mul, FP sqrt. New `VecSub/VecDiv/VecSqrt`
intrinsics with x86 lowering. NEON fail-closed (arm backend silently no-ops
unknown SIMD ops). Pre-mutation op validation. New
`vectorize_function_contract` flavor: `-ffp-contract=fast` without
`-fassociative-math` contracts map loops (GCC parity). Fixed a real
`emit_avx_map_fma` scratch-clobber miscompile (ymm2 scratch vs assigned
homes).

### FP call-result XMM-direct stores (IS-29a partial)

Mixed SSE/Integer eightbyte i128 call results store xmm0/xmm1 directly to
their slots instead of %rax/%rdx relay.

## Godbolt oracle + A/B benchmark findings

**Oracle FP-defect hunt** (godbolt.py, GCC 16.2 / Clang / ICX comparison
on nbody/spectral/libm kernels):

- **FMA contraction is CORRECT**: nbody emits 45 `vfmadd231sd` at
  `-O3 -march=x86-64-v3 -ffp-contract=fast` — the store-accumulator
  `fx[i] += dx*mag` pattern fuses correctly (the earlier "store-FMA-fold
  gap" finding was on a stale binary; the new base's `detect_mul_add_fusions`
  + `emit_scalar_fma231` handle the load-accumulator FMA pattern).
- **No FP-correctness defects found** in the scalar FP path.

**A/B structural benchmark** (lccc vs GCC, `-O3 -march=x86-64-v3`):

| Workload | lccc insns/stk/vec | gcc insns/stk/vec | Gap root cause |
|---|---|---|---|
| gzip_crc32 | 174/0/0 | 38/0/0 | table init counted; CRC loop scalar |
| expat_xml_scan | 261/13/0 | 179/0/4 | branchy scan not vectorized |
| nbody | 594/159/0 | 366/0/81 | **multi-store scatter** (OP-05b) |
| spectral_norm | 212/11/0 | 151/4/8 | invariant×load dot (OP-05b) |
| matmul | 134/8/16 | 91/3/32 | under-unrolled vs gcc |

The dominant remaining gap is **vectorization breadth** (nbody/spectral stay
scalar — multi-store scatter + cross-stream recurrences), NOT FP
correctness. The FMA ISA gate + contraction are correct. OP-05b
(multi-store scatter vectorization, runtime alias versioning) is the
highest-priority next item.

## Validation

- `cargo build --profile fastbuild --locked` clean with `-D warnings`.
- `cargo test --lib`: **1167 passed, 0 failed, 6 ignored**.
- Full `run_regression.py` (466 files): **451 passed, 3 failed** — all 3
  are the environmental i686 SIGSYS cases (raw `int $0x80` blocked by the
  container; byte-identical asm under kill switches).
- `vectorize_map_expr_tree.c` passes lccc + GCC with/without contraction.
- FMA ISA gate verified: 0 `vfmadd` without `-mfma`, 2+ with.
