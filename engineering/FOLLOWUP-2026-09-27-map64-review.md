# I64 map review follow-up

Base: `563fb0bcb39488a97a6aa69858458196cf91710c` (merged #653).

## Findings, not scores

* F1 is partly valid. The transform rejected constant trips <= its unrolled
  width, but the claim that 3..8 necessarily remain scalar is false on this
  main: compiling restrict subtraction at baseline x86-64 gives 1, 2, and 4
  `psubq` instructions for 3, 4, and 8 respectively via unroll/SLP. The early
  map analyzer also independently rejects <=4, so changing only the proposed
  transform threshold cannot make that entry handle 3/4. Preserve the existing
  SLP route. Shrink the factor for known trips; within the existing <=16 tiny
  loop window, additionally shrink until at most one scalar I64 tail remains.
  For 15 the suggested width-only repair would do nothing and retain seven
  scalar elements. The additional remainder-cost condition fixes that case
  without changing remainder construction or overlap checks.
* F2 identifies a worthwhile separate project, not a demonstrated defect or
  automatic 2x speedup. Width is a complete intrinsic/backend contract, not a
  load/store naming change. AVX2 DOES support 64-bit equality/signed greater
  compares, logical qword shifts (including variable forms), and bitwise mask
  selection; it lacks native general qword multiply and arithmetic right
  shift. Unsigned comparisons require appropriate lowering. Any wider path
  needs operation-specific legality, register-allocation coverage, differential
  tests, and 15-sample 256x2 versus 128x4 measurements before selection. No
  256-bit path or ABI change is included here.
* F3 is valid. Snapshot the presence-based switch with `var_os` in run_passes,
  refresh per TU, and consult it only for I64/U64 on x86. A unit test pins the
  true-to-false same-worker refresh. Internal pass callers remain responsible
  for setting policy/ISA state; environment lookup is not an internal API.
* F4: no ARM performance evidence supports the new unroll. Explicitly leave
  NEON rolled; this is containment, not a claim of ARM acceleration.
* F5: the single shell gate already exercises multiple runtime functions and
  counts 0..259, so “one case” understates coverage. Its label split really is
  fragile. The replacement finds a backward branch enclosing packed ops,
  asserts actual displaced loads AND stores, and tests renamed labels and
  deliberately corrupted offsets/opcodes. It pins the rolled switch, x2 tier
  (two sub PLUS two add instructions, not two total), and a single hoisted
  invariant broadcast. Fixed-trip runtime and codegen tests cover every count
  3..16; the original runtime fixture now also checks tier2 and invariant maps.
* F6 is a reasonable heuristic refinement, not measured proof of regression:
  invariant broadcast work is not replicated. Separate unroll_cost from the
  unchanged syntax/node budget. Invariants still occupy registers, and the
  heuristic is not an exact pressure or throughput model.
* Minor items: checked arithmetic/expect documents internal bounded invariants,
  not untrusted-input panic paths. Debug-string cache keys remain unchanged:
  replacing them with ordinary floating Operand equality could conflate +0/-0
  and mishandle NaNs in this shared FP/integer emitter. That deserves a bitwise
  structural-key change with its own tests, not an incidental rewrite.

## Validation and limits

The expanded AVX2/SSE2 runtime/GCC/scalar/rolled and i686 codegen gate passes.
Pure policy tests cover counts 0..259, powers of two, small-tail bounds and
invariant cost versus syntax count. Overlap/remainder construction is unchanged.

Fresh 15-sample `bench_kernels.py` run, same synthetic 1024-element kernel:
`--kernels map64_sub --reps 15 --inner 2000 --flags '-O2 -march=x86-64-v3'
--lccc-alt-env CCC_NO_MAP_I64_UNROLL=1`. Best samples: default 0.303 ms,
rolled 0.317 ms, GCC 0.176 ms (raw distributions in the accompanying JSON).
This VM run does NOT reproduce the old 1.45x magnitude: only 1.047x here, and
GCC remains faster. No Raptor Lake or PMU claim is warranted. The large-trip
subtraction policy is unchanged; these numbers do not establish a performance
win from this follow-up. Its concrete improvements are policy consistency,
short fixed-trip packed coverage, ARM containment and regression sensitivity.

Full CI status is recorded by the exact-tree snapshot ledger, not inferred
from a successful focused gate. Fastbuild uses -O1 / -j2 and active 8 GiB swap.
