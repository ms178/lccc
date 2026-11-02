# Raptor Lake optimization backlog — 2026-10-29

This is a hypothesis and experiment plan, not a claim that these optimizations
are implemented or that any speedup has been measured. The supplied report is
for a different compiler revision than this checkout. Its per-workload wall,
cycle, and instruction ratios are the prioritization signal; current-tree
compiler/assembly screening is only a source of leads. The physical Raptor Lake
host and post-change paired runs are still required before accepting any
runtime-performance claim.

## Higher-priority, testable hypotheses

| Priority | Report case(s) | Candidate optimization | Correctness/performance fence |
|---:|---|---|---|
| 1 | `zlib_ng_slide_hash` (P/E) | U16 unsigned saturating-subtract to `psubusw`/`vpsubusw` is implemented. Next isolate rolling-hash update and slide-loop bounds; consider vectorizing only independent lanes and specialize invariant window sizes only when proven constant. | Existing signed/nonzero/overlap/tail and ISA-negative regression. Add separate loop-bound and differential controls. Pin the report's exact source revision before claiming this addresses remaining cycles. |
| 2 | `fp64_product_map` (E) | Internal static worker gets a known `n=512` but LCCC emits a generic count-controlled worker; GCC 16.2 emits `fp_product_map.constprop.0`. Investigate generic internal-call specialization/constant cloning for immutable scalar arguments, including noinline cases, so the worker can use a fixed trip count without changing source semantics. | Must honor external/address-taken/weak/asm-label/variadic/ABI rules and `noinline`; specialization may clone but not inline. Regression with several call sites (same and conflicting constants), runtime-sized calls, recursion, function pointers, and strict FP. Compare code size and cycles on P and E cores. |
| 3 | `sqlite_vdbe_serial` (P/E) | Keep the 12-way serial-type semantics, but compare switch lowering strategies: hot-case layout from profile, range clustering, and branch/dispatch cost. Inspect big-endian decode separately from dispatch; avoid assuming a jump table is slower. | Differential decoder across all serial types, truncation/length boundaries, signed 24/48-bit values, floats, constants, text/blob. Track indirect-branch misses and cycles, not instruction count alone. |
| 4 | `zlib_ng_compare256` (E) | Analyze unaligned 256-byte compare: wide-load grouping, mismatch-mask extraction, and early-exit branch behavior. If all bytes are compared even after a mismatch, consider a first-mismatch fast path. | Never widen beyond valid object/page bounds. Test every mismatch position, equal inputs, all alignments, short tails, inaccessible guard page after buffers, and scalar fallback. Measure branch misses and cycles. |
| 5 | `arith_loop` (E) | The report has fewer retired instructions but more cycles: inspect recurrence depth and ports. Candidate transformations are reassociation only where C overflow/FP rules allow, scheduling independent chains, and register-home/spill reduction. | Preserve `-fwrapv` behavior and all intermediate observables; compare exact output. Use assembly/dependency analysis and P-core/E-core timings; do not optimize for instruction count alone. |
| 6 | `sqlite_varint` (P) | Common 1-byte and 2-byte varints may benefit from a low-branch fast path, with a generic continuation-byte fallback. Investigate the serial dependency and unpredictable branch pattern first. | Exhaustive short encodings plus random 64-bit values, malformed/truncated encodings, overflow policy, and differential oracle. Include cache/branch PMU evidence. |
| 7 | `hot_web_loop` (P) | Inspect loop-carried values, PHI edge copies, and register homes. Candidate is better rematerialization or edge-copy coalescing where liveness and flags allow. | Liveness/interference assertions, pressure stress variants, no extra spills, and exact runtime output; include code-size and ISA ceilings. |
| 8 | `sha256_transform` (E) | Lower cycles despite fewer instructions is the goal. Explore schedule of independent rounds, rotate/boolean instruction selection, message-schedule overlap, and (only with explicit target-feature dispatch) SHA extensions. | Known-answer tests, streaming/block-boundary tests, baseline ISA-negative checks, and safe runtime dispatch. Never make SHA-NI mandatory for generic x86-64. |
| 9 | `loop_patterns` (E) | Split its aggregate into the individual loop kernels; identify which case causes the 1.165 ratio before changing a general loop pass. Candidate areas: induction strength reduction, loop exit compares, vector remainders, and branch layout. | Preserve each kernel's arithmetic/alias semantics; test each independently and the aggregate. Reject transformations that trade a local win for regressions in other loop shapes. |
| 10 | `byte_load_reassembly` (P/E) | The current compiler already has a byte-load combining pass; the open issue may be specific decode shape, alias/provenance, or the report-revision delta. Inspect the exact hot loop and bounds before widening loads or adding combines. | Prove same-object range, endianness, alignment legality, volatility/atomic constraints, and no page-crossing overread. Guard-page and exact-boundary tests required. |
| 11 | `linux_list_sort` (P/E) | A fresh correctness-only current-tree GCC 16.2/LCCC screen shows LCCC's `merge_and_count` has direct field compares, not an indirect comparator call. Thus the earlier indirect-call lead is not reproduced by this build. Remaining candidate is pointer-chasing/merge-loop work: tail handling, branch layout, node field loads, and sort bin/count bookkeeping. | Keep stable tie handling and exact permutation checks. Use the report's exact revision and input size, profile pointer-load latency/branch misses, and validate both P/E cores; do not rewrite the algorithm from synthetic timings. |

## Other top-15 rows and broader levers

* `zlib_ng_slide_hash`: the SIMD saturation instruction is a code-shape
  correction only. Analyze other loop work and compare post-change on the same
  physical host before attributing any rank change.
* `byte_load_reassembly`: do not blindly widen loads; current `-O2` assembly is
  the place to identify whether the hot path is actually still bytewise.
* `fp64_product_map`: the current screen confirms an opportunity to compare
  constant-trip specialization, but no runtime result has been taken.
* `linux_list_sort`: current assembly falsifies one specific earlier lead; the
  problem may instead be memory latency or merge-loop branch behavior.
* Across all rows, separate front-end, branch, load-to-use, dependency, and
  frequency effects using paired medians plus cycles/instructions/branch/load
  PMU counters. Report P-core CPU 0 and E-core CPU 16 separately, retain
  warmups/repetitions, and use confidence intervals for short tasks.

## Current screening evidence

On the Arena VM, the fastbuild LCCC binary and cached GCC 16.2.0 compiled and
passed strict correctness-only runs for `fp64_product_map`,
`byte_load_reassembly`, `linux_list_sort`, and `sqlite_vdbe_serial`; no timing
was collected. Disassembly showed:

* LCCC `fp_product_map` keeps a generic `n` loop and groups two packed F64
  vectors per iteration; GCC emitted a `constprop.0` clone with a fixed 512
  element loop and one packed vector per iteration.
* LCCC `merge_and_count` compares node keys directly in the merge loop; the
  earlier current-tree observation of an indirect comparison is stale or from
  a different build. This does not explain the report's runtime ratio.
* LCCC's SQLite serial-type decoder uses a jump table in the examined binary.
  This is not evidence that replacing the table is profitable; inspect target
  perf counters and the exact report revision first.
* The byte-load test's setup and known-answer checks dominate its static
  assembly listing. The timed loop must be isolated before deciding whether
  its current loads are widened or whether another path explains the report.

All of these observations come from a VM screening build with no Raptor Lake
runtime measurements. The reproducible command was:

```sh
source ~/.cache/lccc/agent-toolchains/env.sh
python3 tests/benchmark/run_benchmarks.py \
  --only fp64_product_map,linux_list_sort,sqlite_vdbe_serial,byte_load_reassembly \
  --compilers lccc,gcc --skip-perf --strict \
  --artifact-dir /tmp/raptor-followup-screen \
  --json /tmp/raptor-followup-screen/results.json
```
