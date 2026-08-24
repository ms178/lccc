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

---

# Session 77 addendum (v5) — widening I32→I64 reductions + benchmark sweep

**Base:** `b7f1819` → PR #229 merged the v4 VEX scalar-FP work.

## The 7-worst-benchmark sweep (runtime lccc/gcc, -O3 -march=x86-64-v3)

| # | Benchmark | lccc | gcc | ratio | Root cause |
|---|-----------|------|-----|-------|------------|
| 1 | loop_patterns | 403ms | 37ms | 0.09 | sum_array now vectorized (this session); conditional-sum + dot-product + LCG-init stay scalar (GCC: compare/blend + vpmulld) |
| 2 | nbody | 685ms | 207ms | 0.30 | multi-store scatter (OP-05b) + marching-pointer homes (RA-01b) |
| 3 | libm_round_family | 389ms | 185ms | 0.47 | FP call-result accumulator path (IS-29a) |
| 4 | linux_find_bit | 26ms | 16ms | 0.62 | andn/cmov idiom coverage |
| 5 | mandelbrot | 1154ms | 864ms | 0.75 | escape-loop FP chain |
| 6 | spectral_norm | 241ms | 182ms | 0.76 | A(i,j) computed-invariant in dot-product |
| 7 | fannkuch | 3339ms | 2566ms | 0.77 | perm-rotation loop un-vectorized (GCC: vpshufd) |

(Correctness reference established: lccc's nbody is bit-identical to
`gcc -O0` at 500k AND 5M steps — the FP-semantics gold standard. The
gcc -O3 differences are GCC's own reassociation.)

## v5 improvement: widening I32→I64 reduction vectorization

`long s = 0; for (...) s += arr[i];` over an `int[]` was pure scalar
(`movslq; addq`, 1 element/iteration). New composite intrinsic
`VecWidenAddI32x4ToI64x2` (AVX2): 4 elements/iteration with full I64
precision per lane. Two correctness traps found by differential testing
and documented in the lowering: 256-bit loads double-count lanes 4..7
(IV advances 4), and vextracti128 cannot extract dword lanes 2..3 after
a 128-bit store (vpunpckhqdq is the correct mover; vpermq's immediate
selects QWORD lanes). Assembler gains the vpmovsxdq VEX encoding.

Verified exact vs GCC for n=1/10/100/1000 on the multi-call debug
harness; loop_patterns full-program output byte-identical.

## Remaining gaps (ranked for v6)

1. **Conditional reductions** (`if (a[i]>0) s += a[i]`): GCC emits
   vpcmpgtd+blend; lccc emits a branchy scalar loop (3× on that kernel).
   Needs select-reduction support in the reduction family — the if_convert
   pass already produces Select IR, so the gap is reduction-analyzer
   recognition of Select-shaped conditional adds.
2. **Integer dot-product** (`(long)a[i]*b[i]`): needs vpmuldq-class
   widening multiply (I32×I32→I64) — same shape as the widening sum
   plus a widening multiply intrinsic.
3. **LCG init loops**: GCC vectorizes LCG+mod via vpmulld lanes; requires
   a general integer map vectorizer for nonlinear recurrences (the map
   trees handle elementwise only; the recurrence is the blocker).
4. fannkuch rotation (vpshufd), mandelbrot escape loop, spectral A(i,j) —
   OP-05b scatter/computed-invariant class.
