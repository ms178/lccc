# TASK-RA-06A — reload-at-use splitting + arithmetic-chain copy webs

IDs: RA-06, RA-06a, PF-05 · Priority: **P0** · Base: origin/main @ f657de55
· The single largest measured codegen gap (3–5× spill traffic vs LLVM).

## Objective

Teach the linear scan to split live ranges at uses instead of demoting the
whole remaining lifetime: when a value crosses a high-pressure region
(calls first), insert a reload immediately before the use. In parallel,
extend copy webs through the arithmetic chain so ONE range carries a
recurrence (adler's `s1`/`s2` partial sums are separate scan values today;
promoting only the copy leaders measured +28 % runtime — the web must be
one range).

## Files

`src/backend/live_range.rs` (scan, `enable_splitting` stub gate),
`src/backend/regalloc.rs` (sweep-eviction traffic model — reuse it),
`src/backend/split_ranges.rs` (IR-level split foundation), emit-side
reload placement.

## Acceptance

- Adler-32 kernel ≤ 1.15× GCC; adler stack refs 78 → <30.
- gzip `longest_match` stack-mem must not rise (hard veto).
- xmltok/inflate TU stack-mem ratios drop below 4× (re-measure; RA-05a
  addressed the interference half already).
- `CCC_VERIFY_REGALLOC=1` clean over the whole correctness corpus.
- Kill switch: extend the `enable_splitting` gate (`CCC_NO_LIFETIME_SPLIT`
  if a new switch is needed) — bisectable, default-on after validation.

## Validation battery

`cargo test --lib` · full `run_regression.py` · 300/300 O2/O3 differential
fuzz · 600 phi-CFG + 540 alias · gzip 1.14 30/30 + roundtrip · zlib-ng
ctest · expat ctest · kernel corpus 15/15 output-identical · adler/gzip A/B
interleaved best-of-3 with checksums.

## Do not

- Do not promote copy leaders without the web extension (measured +28 %
  runtime, reverted — DECISIONS.md "Register allocation").
- Do not key segment decisions on raw per-value segments (need the merged
  coalesce-leader union; `sqlite_yy_shift` miscompiled once).
- Do not model evicted occupancy as closed windows (half-open `[start, cut)`
  — verifier encodes both).
- Do not attempt a clean-slate allocator rewrite (SGSA rejected: RA-05a→06a
  incremental path is strictly safer).
