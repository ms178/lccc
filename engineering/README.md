# Engineering documentation

**This directory is the only live engineering doc tree.**  
User-facing site remains `docs/` (Jekyll). Session diaries remain `docs/history/`.

| Path | What |
|------|------|
| [`STATE.md`](STATE.md) | Compiler as it is on `main` now |
| [`agent/README.md`](agent/README.md) | Next implementation agent — start here |
| [`agent/RULES.md`](agent/RULES.md) | Non-negotiable constraints |
| [`agent/BACKLOG.md`](agent/BACKLOG.md) | Ranked work items by subsystem |
| [`agent/SEQUENCE.md`](agent/SEQUENCE.md) | First ten tickets in order |
| [`evidence/godbolt/`](evidence/godbolt/) | Compiler Explorer scoreboard + asm |
| [`evidence/workloads/`](evidence/workloads/) | gzip / zlib-ng / Expat LCCC vs GCC |
| [`subsystems/`](subsystems/) | Current design of each subsystem |

Measure with `scripts/godbolt.py` and `tests/benchmark/run_benchmarks.py`.  
Build: `scripts/build_lccc_fast.sh` → `target/fastbuild/lccc`.
