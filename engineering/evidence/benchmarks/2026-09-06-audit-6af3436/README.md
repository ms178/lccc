# Benchmark screening evidence — 2026-09-06 audit session

**Run:** full 39-kernel corpus, LCCC vs GCC vs Clang, `-O2`, 9 paired timed
rounds + 1 excluded warm-up per compiler, randomized compiler order within
rounds, `--cpu none` (VM), `--strict`.

**Why this run exists:** the audited patch deleted the previous evidence
directory (`2026-09-06-rust198-3e1b71dd`) while keeping its numbers in the
README, and the CI had been accidentally reverted to a five-benchmark
snapshot. This run (a) re-validates the expanded corpus end-to-end on the
fixed tree, and (b) restores a checked-in raw-sample record.

**Environment (screening — no PMU):**
- 2-core KVM VM, Debian 13, 8 GiB swap
- LCCC: fastbuild (Rust -O1), session commit `403021ef` on main `6af3436`
- GCC 14.2.0 (Debian), Clang 19.1.7
- Note: the first attempt at this run was discarded — timed-out stress-test
  orphans were burning CPU concurrently (see follow-up doc §7.1); this run
  started on an idle host.

**Contents:**
- `results.json` — full raw samples, per-compiler stats, bootstrap CIs,
  correctness verdicts, environment metadata, commands
- `results.md` — compact report
- `terminal.txt` — run transcript
- `artifacts/` — per-benchmark binaries, disassembly, section sizes

**Interpretation:** VM wall-clock screening only. Absolute numbers differ
from the bare-metal reference host (README table); use for relative
LCCC/GCC/Clang screening and regression deltas, not as
microarchitectural evidence.
