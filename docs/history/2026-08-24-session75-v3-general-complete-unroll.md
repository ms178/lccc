# Session 75 (2026-08-24) — v3: general complete unrolling + FP defect hunt

**Base:** `ms178/lccc` `main` at `4589b5b` (PR #227 = the v2 patch merged).

## Godbolt oracle FP-defect hunt results

Method: `scripts/godbolt.py compare` + local GCC differential on the FP
benchmark kernels + full 5M-step nbody output comparison.

**Correctness defects found and fixed** (both in the NEW general complete
unroller, caught by differential testing BEFORE shipping):

1. **Cloned terminators did not rename value uses.** A cloned inner-loop
   `CondBranch` referenced the ORIGINAL compare value — dead after the
   outer rewrite — producing an infinite loop. Fix:
   `replace_values_in_terminator` on every clone terminator.
2. **Header phi → Copy took `incoming.first()`.** When the latch edge is
   listed first, the dead phi became a self-referential cycle with its own
   back-edge value (`i = i+1`), leaving iteration 0's IV garbage and
   silently skipping its body (nbody E2 divergence: -1.68 vs -4.13).
   Fix: take the NON-LATCH (init) incoming.

**Verified-correct FP behavior (no defects):** scalar FMA contraction
(45 `vfmadd231sd` on nbody at `-O3 -v3 -ffp-contract=fast`), the OP-36
tri-state contract, map-tree/stencil/reduction vectorization, and the FMA3
ISA gate. After the fixes, nbody 5M-step output is **bit-identical to GCC**
(`-0.169075164 / -0.169083134`) at -O0/-O2/-O3.

## The structural improvement: general complete unrolling

`try_complete_unroll_general` (loop_unroll.rs): complete-unrolls
constant-trip loops of ANY block shape, including bodies that contain inner
loops — the triangular `for(i<n){for(j=i+1;n)}` shape of nbody
advance/energy that previously never unrolled. Key design:

- Iteration 0 reuses the original header+body; iterations 1..N are full
  clones with per-clone values/labels, IV constants, carried-phi chaining,
  inner-loop phi-edge relabeling, and latch-back-edge redirection.
- `resolve_const_operand`: const-chain evaluator so the fixpoint cascades
  outer→inner in ONE `unroll_loops` call (the unroller runs BEFORE the
  pipeline's constant folder; the unrolled outer leaves the inner init as
  `Add(const,1)` which must resolve without waiting for constfold).
- Pure intrinsics (sqrt/FMA-class) are now eligible in both complete-unroll
  shapes (any-Intrinsic rejection permanently disqualified nbody's FP
  bodies).
- **FP-aware expansion budget** (256 for FP-heavy bodies vs 512 integer):
  measured on nbody, fully unrolling `advance`'s FP-dense body yields
  1183 insns/476 stack-refs vs 594/159 un-unrolled — simultaneously-live FP
  temps exceed the linear-scan XMM pool and the spills cancel the gain.
  The budget blocks the unprofitable case while keeping integer/leaf-FP
  cascades.

**Effect where it fires:** struct-field loops with constant trips now fold
to `bodies+N(%rip)` direct addressing (66 such folds on nbody; the a_/b_
shape battery goes from loop+stack to straight-line RIP-folded code, 0
stack refs).

## Remaining FP gaps (root-caused, next session)

A/B at `-O3 -march=x86-64-v3` (lccc insns/stk/vec vs gcc):

| Workload | lccc | gcc | Root cause |
|---|---|---|---|
| nbody | 594/159/0 | 366/0/81 | advance's FP-dense body: unroll blocked by spill budget (correct call); vectorization of multi-store scatter = OP-05b |
| spectral_norm | 212/11/0 | 151/4/8 | `A(i,j)` computed-invariant inside dot-product: needs computed-invariant recognition + non-trapping div proof + vector int div — a project, not a patch |
| libm_round_family | 288/4/1 | 182/1/24 | FP call-result accumulator path (IS-29a remainder) |
| matmul | 134/8/16 | 91/3/32 | under-unrolled vs gcc (2× vs 4×) |

Runtime (500k-step nbody / N=500 spectral): lccc 0.26×/0.34× of gcc —
dominated by the vectorization-breadth gaps above, NOT by FP correctness
(which is now bit-exact).

The marching-pointer stack relay (`leaq bodies; movq %rax,344(%rsp)`
pattern) remains the deepest RA-side item: IVSR's pointer recurrences get
slot-homed when the RA runs out of registers. RA-01-class remat extension
or SIB-index preference for those recurrences is the follow-up.

## Validation

- 1167/1167 unit tests (nested-loop test updated to the new contract).
- Full 466-file regression corpus: 451 passed, 3 failed (environmental
  i686 SIGSYS only, byte-identical asm under kill switches).
- nbody 5M-step: bit-identical to GCC at -O0/-O2/-O3.
- Shape battery (affine/sqrt/nested): all match GCC references.
- New regression `unroll_nested_complete.c` covers the cascade + both
  fixed hazards.
