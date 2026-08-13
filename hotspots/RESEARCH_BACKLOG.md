# Performance research backlog

This backlog is ranked by the requested heuristic:

```text
(expected generated-code impact × affected workload breadth × confidence) / implementation cost
```

VM ratios are **screening evidence only**.  They rank investigation; they do
not estimate a portable or Raptor Lake gain.  Every item needs a correctness
reproducer, assembly comparison, and bare-metal PMU confirmation before an
integration decision.

| Priority | Optimization / question | Evidence | Expected gain | Affected workloads | Difficulty | Research / source | Prototype | Result | Status |
|---:|---|---|---|---|---|---|---|---|---|
| 1 | Acyclic static leaf-function inline budget and post-inline simplification | `expat_xml_scan`: source predicate lowered to 13 blocks while old no-loop limit was 12; debug/assembly prove hot call survived | 1.222× target-kernel VM speedup; bare-metal gain unknown | parsers, tokenizers, string/byte loops, helpers across all C workloads | Medium | LCCC inliner audit; classic inline cost-model literature | Cap 12→16 + unit test + debug explanation | 21-round A/B: 0.8180 ratio, +165 B text; final test/fuzz gates pass | `P1 integrated, hardware follow-up` |
| 2 | Bounded static-loop inlining | zlib-ng Adler: hot static 121-IR-instruction loop helper had ≤2 direct call sites but was excluded by generic loop cap | 1.206× on target kernel; hardware gain unknown | compression, hashing, checksums, small static loop helpers | Medium | Inline clone-cost model | 40-inst freely cloned tier; 128-inst/16-block tier limited to ≤2 calls | 31-round ratio 0.82945; `.text` 2530→2083 B; workload geomean 0.93784 | `P1 integrated, hardware follow-up` |
| 3 | Branch-heavy varint selection and shift/CSE lowering | SQLite varint screening ratio 2.069; aggressive 189-inst decoder-inline spike was measured | Still open | SQLite/VDBE, parsers, compact formats | Medium | SQLite `sqlite3GetVarint`; branch/switch lowering research | Large acyclic inline spike rejected | 1.0028 ratio with +1119 B `.text`; no gain | `screening` |
| 4 | Safe default IVSR + equivalent-address coalescing | Forced IVSR had a nested-matmul wrong result; safe disjoint loops remove redundant index work | 1.122× on `loop_patterns`; code-size reduction for repeated `a[i]` | array loops, numerical kernels, data transforms | Medium | IVSR / loop dependence analysis | Skip overlapping loops; coalesce `(IV,stride,base)` GEP groups | 24/24 correctness, 500 fuzz cases, `.text` 1664→1640 on loop patterns | `P0/P1 integrated, hardware follow-up` |
| 5 | CRC recurrence/addressing and table-load scheduling | gzip CRC-32 screening ratio 1.450 | Likely limited by recurrence; validate rather than chase a misleading scalar win | compression, archives, networking | Low–Medium | GNU gzip scalar CRC; recurrence/SIMD CRC research | No | Candidate only after inspecting dependency chain and ISA legality | `screening` |
| 6 | Aligned bulk comparison / early-exit code layout | glibc memcmp screening ratio 1.587; result is <20 ms and therefore noisy | Unknown; increase work scale or batch calls before prioritizing | glibc-like string/memory code, parsers | Medium | glibc `memcmp`; vector/scalar memcmp design | No | Current timing flagged short; do not optimize yet | `screening` |

| 7 | PGO flat-profile dominance gate + label-independent hot-site inlining | zlib-ng adler32 pre-fix: `-fprofile-use` regressed -17% (flat profile perturbed the inliner) | neutral at worst; expat -28% → ~1.01× | all PGO workloads | Low (merged) | LCCC PGO audit | `has_spread()` + `inline_decisions_active()` + entry-count-ratio force-inline | 31-round A/B: adler32 0.827→0.997, expat 0.718→1.014; no regressions | `P1 integrated, hardware follow-up` |
| 8 | Cost-aware devirtualization (skip stable single-target sites) | op_dispatch: promotion of a loop-invariant single target regressed +28% (BTB already predicts it) | 0 on single-target; 1.07× on genuinely multi-valued dispatch | virtual-dispatch / callback-heavy code, vtables | Low (merged) | Intel branch-prediction guidance; PGO value profiling | `LCCC_PGO_PROMOTE_STABLE=95` gate in `promote.rs` | op_dispatch 49.9→40.1 ms (neutral); multi_dispatch 1.07× | `P1 integrated, hardware follow-up` |
| 9 | Backend profile-driven branch inversion (hot successor falls through, no block reorder) | expat pre-fix: layout reorder scattered the hot loop (131→248 ms) | 0 (neutral) as designed; removes taken branches on hot paths | branch-heavy loops, parsers, byte scanners | Medium (merged) | Intel branch-prediction; LLVM MachineBlockPlacement | `cond_fallthrough` hint → `emit_cond_branch_blocks_impl` | no regression on expat/gzip/adler32 | `P1 integrated, hardware follow-up` |
| 10 | Use-def chain infrastructure (shared optimizer context) | every pass rescans all instructions; constant/copy propagation blocked | O(passes·insts)→O(1) per value; unlocks forward const/copy prop | all workloads, compile time | High | LLVM SSA, "high_use_def_chains" | Not started | — | `screening` |
| 11 | PGO loop-versioned devirtualization of a single-invocation indirect call | hot loop-invariant indirect call promoted per-iteration still adds overhead | possible when the guard is hoisted out of the loop | callback tables, dispatch loops | High | LLVM loop versioning | Not started; requires call-site-to-loop dominance analysis | — | `screening` |
| 12 | Sample-based PGO (perf/LBR) to complement instrumentation | instrumentation overhead and flat-profile noise limit some use cases | lower training overhead; catches functions inlined in training | large binaries, whole-program PGO | High | LLVM `-fprofile-sample-use`; GCC AutoFDO | Not started | — | `screening` |
| 13 | Instruction scheduling for Raptor Lake port/resource pressure | no scheduler; latency-bound chains stall | ~1.1× on latency-bound code | FP, memory-bound kernels | High | Intel Optimization Manual ch. on scheduling | Not started | — | `screening` |

| 14 | Aggregate by-value ABI & wide copy lowering (struct_copy) | struct_copy: field-by-field copy through %rax accumulator (137 movs vs GCC's 29) | **21×** on struct_copy; large for any struct-heavy C | structs, VMs/emulators, ABI-heavy code | High | SysV AMD64 ABI; GCC/LLVM aggregate copy | Wide/vectorized copies + in-register aggregate passing + sret | 21.06× measured; no prototype yet | `screening` |
| 15 | Broaden FP vectorization beyond the reduction idiom | spectral_norm 9.3×, nbody 3.9×, mandelbrot 3.2×; GCC vectorizes the FP inner loops + FMA | several-× on FP-heavy kernels | scientific, math, graphics | High | LLVM loop/SLP vectorization; Intel FMA | Non-reduction loop vectorization + FMA fusion | measured gaps; no prototype | `screening` |
| 16 | Byte-scanning / branch-heavy codegen (expat) | expat 3.3×; GCC folds char classification into compare/range/bit-test; LCCC keeps many small branches | ~3× on expat; broad for parsers/tokenizers | parsers, tokenizers, byte loops | Medium | GCC/LLVM branch lowering; bit-test sequences | Fewer, wider tests; bit-test lowering | 3.29× measured; no prototype | `screening` |

## Decision rule

A row may move to `prototype` only after its proposed mechanism is falsifiable
and a target microbenchmark is added.  A row is `rejected` if the code-quality
gain does not survive the broader suite, violates correctness, or costs
unacceptable compile time/code size.  Rejected rows stay here with the measured
reason rather than disappearing from institutional memory.
