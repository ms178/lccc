# Benchmark evidence — S13 codegen-gate fix (2026-09-27)

**Run:** full 41-kernel corpus, LCCC vs GCC, `-O2`, 9 paired timed rounds +
1 excluded warm-up per compiler, randomized compiler order within rounds,
CPU-pinned (taskset cpu 0), `--strict`. CI-exact bench.yml invocation.

**Why this run exists:** S13 re-removed the bogus x86-64 indexed-fold
staging rule (S08's accidental revert of F15) that tripped the
sqlite_varint/expat golden gates. This run proves the fixed tree is
runtime-correct on every kernel and records the LCCC-vs-GCC runtime
standing of the shipped codegen (static gate metrics alone cannot show
runtime).

**Environment (screening — no PMU, `perf` not installed):**
- 2-core KVM VM (Intel Xeon @ 2.60 GHz), Debian 13, taskset-pinned
- LCCC: fastbuild, session commit `685da054` on upstream main `563fb0bc`
  (PR #653)
- GCC: `/usr/bin/gcc` (Debian 14); see `results.json` provenance for versions
- Caveat: shared/VM host; sub-20 ms medians are noise-dominated (the runner
  flags these per benchmark); treat small-`n` ratios near 1.0 as ties.

**Contents:**
- `results.json` — curated per-benchmark medians, LCCC/GCC paired ratios,
  95% bootstrap CIs, correctness verdicts, aggregates, provenance
  (per-round samples dropped for size; the full 1.1 MB raw JSON is kept
  out-of-tree in the workspace artifacts area alongside the snapshot
  ledger)
- `results.md` — compact generated report (full 41-row table)

**Verdict: 41/41 correct (`--strict` gate PASS).**
Aggregate LCCC/GCC: geometric mean **0.724**, arithmetic mean **0.947**
(<1 = LCCC faster). Best: `fib` 72.9× (rec2iter), `ackermann`/`constant_recursion`
~60×+, `libm_round_family` 4.3×, `bitops` 1.75×, `double_reduction` 1.35×,
`chacha20_block` 1.26×. Worst: `lz4_match_extend` 1.94× slower,
`nbody` 1.41×, `expat_xml_scan` 1.25×, `hash_table` 1.18× — all tracked as
S14 codegen follow-ups (none is a regression: the S13 fix only removes
instructions; pre-fix vs post-fix static metrics confirm strict
improvement on the two gate workloads). S13-relevant rows: `sqlite_varint`
0.97 (tie-or-win), `expat_xml_scan` 1.25 (static moves improved 33→26 all
the same — the runtime gap is code-shape, not the fold).

**Interpretation:** VM wall-clock screening only. Use for relative
LCCC/GCC screening and regression deltas, not as absolute numbers.
