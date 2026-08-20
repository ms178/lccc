# Pre-Phase 1 baseline (March 2026) — annotated August 2026

This document is a **historical lab notebook**. Numbers (Rust 1.93, GCC
15.2, 499 tests, 11 KB / 256 B frames, “-O2 same as -O0”) describe the
tree at 2026-03-19. They are **not** current KPIs.

## What remains true

- RA still runs before/during stack layout via
  `run_regalloc_and_merge_clobbers`.
- Loop-depth weighting `10^depth` (capped at 4) still exists.
- i7-class x86-64 is still the performance target.

## What is false today

| March claim | August 2026 |
|-------------|-------------|
| `regalloc.rs` 573 lines, 3-phase greedy | ~3612 lines, linear scan + waves |
| `liveness.rs` 1211 lines | ~2163, with segments |
| `ccc/src/backend/...` | `src/backend/...` |
| ~5% values in regs | much higher on integer leaves; still weak on spanning/FP/SIMD |
| -O2 identical to -O0 | optimizer tiers exist independently of this RA work |
| prologue.rs:81 is the only integration | split_ranges pre-pass; XMM; folded_index_uses |
| Next step: write live_range.rs | **done** |

## How to read the March experiments

The “CCC faster than GCC -O0 on the stress test” observation was about
**unoptimized GCC storing every local**, vs CCC already using some
callee-saved regs. That is not a win vs GCC -O2 / ICX.

The 256 B vs GCC 32 B stack comparison mixed callee-saved save area
with local slots. Always split:

```
frame = alloca + spilled SSA + callee-saved + alignment
```

when comparing compilers.

## Current baseline procedure

Do **not** extend this file with new timings. Put new numbers in
`hotspots/ra/` and [RESEARCH_REPORT.md](RESEARCH_REPORT.md).
