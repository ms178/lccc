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

## Decision rule

A row may move to `prototype` only after its proposed mechanism is falsifiable
and a target microbenchmark is added.  A row is `rejected` if the code-quality
gain does not survive the broader suite, violates correctness, or costs
unacceptable compile time/code size.  Rejected rows stay here with the measured
reason rather than disappearing from institutional memory.
