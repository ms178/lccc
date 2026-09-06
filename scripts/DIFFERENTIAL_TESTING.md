# Differential Testing Harnesses (fuzz_diff.py)

`scripts/fuzz_diff.py` is the unified differential-testing entry point for
LCCC. It generates random C programs, compiles each with LCCC **and every
selected reference compiler** (GCC, Clang) across the requested optimization
levels, runs every binary, and flags any behavior mismatch (stdout, stderr, or
exit code). **Any disagreement is a compiler bug** — the reproducer is saved
under `--repro-dir` (default `artifacts/repros/`).

The harness descends from the standalone testers adopted from John Regehr's
`claudes-c-compiler` **yarpgen** branch (CC0,
https://github.com/regehr/claudes-c-compiler). Regehr (University of Utah) is
the author of YARPGen and a co-author of Csmith — the industry-standard tools
for finding compiler miscompiles. Those tools found the regression corpus in
`tests/regression/regress_*.c` (see the porting notes in
`tests/regression/README_REGEHR_YARPGEN.md`).

## Engines

| Engine | Generator | Dependencies |
|---|---|---|
| `synthetic` | Built-in randomized C generator (fixed-shape programs with random constants) | none |
| `csmith` | [Csmith](https://github.com/csmith-project/csmith) | `csmith` binary + runtime headers |
| `yarpgen` | [YARPGen](https://github.com/VoR0n0k/yarpgen) | `yarpgen` binary |
| `stress_suite` | Repository stress generators (`gen_fp_stress`, `gen_gep_chain_stress`, `gen_slot_stress`) + a bounded `unroll_stress.py` sweep | none |

Every advertised engine must be a real implementation: `--check-engines`
verifies the dispatch wiring and is wired into CI. (This guard exists because
the csmith/yarpgen engines once regressed to silent no-op stubs that reported
`TOTAL: 0` with exit status 0 — a validation vacuum.)

## Common usage

```bash
# Dependency-free smoke: 100 synthetic programs vs gcc+clang at -O0..-O3.
./scripts/fuzz_diff.py --engine synthetic --count 100 -j4

# Real Csmith: complex aggregates / bit-fields / pointer semantics.
./scripts/fuzz_diff.py --engine csmith \
  --csmith /usr/bin/csmith --include /usr/include/csmith --count 500 -j2

# YARPGen: whole-program random C99 with driver.c + func.c.
./scripts/fuzz_diff.py --engine yarpgen --yarpgen ~/yarpgen/build/yarpgen --count 200 -j2

# Repository stress generators + bounded unroller sweep.
./scripts/fuzz_diff.py --engine stress_suite --seed 1234

# CI wiring guard.
./scripts/fuzz_diff.py --check-engines
```

`--count 0` (and the legacy `--tests 0`) means *run forever*.

## Reference compilers

`--refs gcc,clang` (default). **All** selected references must build and run
the program and agree with each other before LCCC is judged; a divergence
against any one reference is a failure. References missing from `PATH` are
filtered out; having none is an error.

## Legacy invocations (csmith_diff.py / yarpgen_diff.py)

`scripts/csmith_diff.py` and `scripts/yarpgen_diff.py` remain as thin wrappers
that translate the historical flag spellings and forward to the matching
engine:

| Legacy flag | Unified spelling |
|---|---|
| `--ccc PATH` | `--lccc PATH` |
| `--clang PATH` / `--gcc PATH` | appended to `--refs` |
| `--jobs N` | `-j N` |
| `--tests N` | `--count N` |
| `--seed-start N` | `--seed N` |
| `--out-dir D` | `--repro-dir D` |
| `--compile-timeout` / `--run-timeout` / `--include` / `--csmith` / `--yarpgen` / `--refs` | unchanged |
| `--keep-passing` / `--keep-skipped` / `--keep-all` / `--progress-every` / `--work-root` / `--continue-on-divergence` | accepted, reported, ignored |

Like the pre-unification testers, the wrappers run forever unless bounded
(`--count N` / legacy `--tests N`).

## Why this matters

LCCC's correctness bar is the hard constraint of the project: a fast compiler
that miscompiles is worthless. These harnesses give a **reproducible,
automated** path to keep finding (and then locking in as regression tests) the
real bugs that random-program corpora are best at exposing. Every bug they
find should be reduced, ported into `tests/regression/`, and fixed — see the
follow-up docs in `docs/`/`engineering/` for the session-by-session log of
cases found and fixed.
