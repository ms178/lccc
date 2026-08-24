# Session 76 (2026-08-24) — v4: VEX 3-operand scalar-FP exploitation

**Base:** `ms178/lccc` `main` at `b7f1819` (PR #228 = the v3 unroller).

## Root cause found by hot-block analysis

The scalar-FP emitters stage the first operand into the destination with
`movsd %A,%D` and then apply an ALREADY 3-operand VEX op
(`vOP %S,%D,%D`) — a pure 2-operand-ISA habit. On nbody's inner
pair-interaction: **7–9 wasted copies per iteration**, each an extra uop
*and* an extra dependency-chain link on the critical FP path.

Calibration facts established this session:

- `gcc -fno-tree-vectorize` nbody = 22ms ≈ vectorized 21ms — **GCC's 4×
  edge is scalar code quality** (366 insns/0 stack-refs vs our 594/159),
  not vectorization.
- lccc's nbody output is **bit-identical to `gcc -O0`** (the FP-semantics
  gold standard) at both 500k and 5M steps; the earlier gcc -O3
  "divergence" at 500k was GCC's own reassociation.
- Fully-unrolled nbody: XMM scan shows 976 candidates / 245 spilled (8
  long-lived invariant masses + dt consume the 14-register pool) — the
  FP expansion budget correctly blocks it (measured 96ms vs 84ms rolled).
- GPR side is healthy (74/75 assigned): the 151 `movq` stack refs are
  slot-HOMED marching pointers (IVSR recurrences), not spills — RA-01b
  (remat-through-Copy / SIB preference) remains the deep fix.

## The fix: two new default-on peephole passes

1. **`fuse_mov_scalar_fp_into_vex_op`** — register staging:
   `movsd %A,%D; vOP %S,%D,%D` → `vOP %S,%A,%D` for
   vmul/vadd/vsub/vdiv/vmin/vmax {sd,ss}. Operand ROLES are preserved
   exactly (no commutativity assumption): `D:=A; D:=D op S` becomes
   `D:=A op S`. Only Nop/Empty may separate the pair (labels admit
   control flow; any other line can observe or clobber the staged
   register).

2. **`fold_scalar_fp_memory_into_vex_op`** — memory-source staging:
   `movsd MEM,%D; vCOMM %D,%X,%X` → `vCOMM MEM,%X,%X` for the COMMUTATIVE
   ops only (vmul/vadd {sd,ss}): AT&T's first source position is the
   Intel src2 slot — the memory-legal operand. Requires %D dead after the
   op: block-local textual-uniqueness deadness proof (the relay_and_lea
   style), 64-active-line bound, labels/branches/calls end the scan
   conservatively.

Plus `CCC_CUNROLL_FP_BUDGET` / `CCC_CUNROLL_INT_BUDGET` env knobs for
budget A/B experiments (defaults unchanged: 256/512).

## Results

nbody (500k steps, `-O3 -march=x86-64-v3`, 5-round median):
**84ms → 68ms = 1.22× vs previous main**; GCC 21ms (gap 4.0× → 3.2×).
Output bit-identical to `gcc -O0` throughout.

## Validation

- 1167/1167 unit tests.
- Full 467-file regression corpus: 452 passed, 3 environmental i686 SIGSYS.
- Pristine `b7f1819` + patch → rebuild → nbody battery + corpus re-verified.
- Patch: applies clean, zero debug leftovers (XMM instrumentation removed
  pre-commit).
