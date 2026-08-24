# Session 78 (2026-08-24) — v6: conditional-sum if-conversion + README refresh

**Base:** `ms178/lccc` `main` at `9ab0d34` (v5 merged as PR #231).

## The improvement: conditional sums now if-convert

`if (arr[i] > 0) s += arr[i]` — the dominant C conditional-reduction
idiom — stayed a branchy scalar loop (62ms on the 10M-element kernel vs
GCC's 21ms). Root cause chain, found by CCC_DEBUG_IFCONV tracing:

1. **if_convert's arm-load coverage compared raw SSA pointer ids.** The
   diamond's two arms each rebuild `GEP(GlobalAddr("array"), shl(cast(iv),2))`
   with fresh SSA ids — the pred's load could never "cover" the arm's load,
   so the diamond was rejected as speculative. Fixed by canonical address
   keys: GlobalAddr bases canonicalize by SYMBOL (the frontend emits a
   fresh GlobalAddr per source use), GEP offsets trace through
   Cast/Shl/Copy chains to the underlying index.
2. **The arm instruction budget counted address materialization.** Each arm
   is {Cast,Shl,Copy,Shl,GEP,Load,Cast,Add} — 8 raw instructions, at the
   MAX_ARM_INSTS=8 limit, and the pred (with its load) exceeds it. Fixed by
   an EFFECTIVE budget: pure address chains (GEP/Copy/GlobalAddr/IV-scaling
   Shl) are discounted — the backend folds them into one SIB memory
   operand and GVN would dedup them across arms anyway.

Result: the diamond converts to `cmov`, the kernel drops **62→38ms
(1.7×)**, output byte-identical to GCC.

## The safe stop: Select-guard vectorization

The full vectorized form (vpcmpgtd+blend masked add, as GCC emits) was
prototyped: reduction-analyzer Select-guard recognition + x86 late
vectorize rerun (mirroring AArch64's `latevec`) + soundness allowances.
The transform MISCOMPILED — it emitted the unguarded widening add
(correct result = the unconditional sum; caught immediately by
differential testing: 735339920561088 vs 3064670858719214). The masked
add needs blend emission in the widening-reduction body, which is real
transform surgery. **The soundness holes were reverted; the recognition
skeleton and the x86 `vectorize_function_late` entry point are kept**
(inert until the masked-add emitter exists). This is the documented v7
item: "conditional-sum Select guard needs a masked vector add".

## README benchmark refresh (26 kernels, -O3 -march=x86-64-v3, 3-round medians)

Geomean 0.72; excluding fib/ackermann algorithmic wins, 0.89 — LCCC is
faster on 17 of 26 kernels. Notable ratios: matmul 0.46 (2.17× faster),
gzip_crc32 0.86, histogram 0.80, linux_find_bit 0.71, sqlite_varint 0.72,
fannkuch 0.75, mandelbrot 0.75, sieve 0.78, spectral_norm 0.80. Remaining
structural gaps: loop_patterns 0.13 (masked-add transform), nbody 0.31
(OP-05b scatter), libm_round 0.47 (IS-29a).

## Validation

1167/1167 unit tests; 469-file corpus: 454 pass, 3 environmental i686
SIGSYS; conditional-sum output byte-identical; widening reduction exact
(n=1/10/100/1000); nbody bit-identical to GCC -O0.
