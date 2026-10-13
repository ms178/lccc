# `artifacts/repros/` — the compiler's scar tissue, and how to replay it

Every file here was produced by the differential fuzzers (`scripts/fuzz_diff.py`,
`scripts/csmith_diff.py`, `scripts/yarpgen_diff.py`, `scripts/gen_fp_stress.py`)
or reduced by hand from a real failure.  They are the project's permanent
evidence: a crash reproducer also documents how a crash was fixed, and a
miscompile reproducer is a claim about lccc that is only worth keeping while it
is *checked*.

**They are replayed, not archived.**  `scripts/check_repro_replay.py` compiles
each file with the current LCCC build and with GCC under *identical* flags, runs
both, and requires identical stdout and exit status (files without `main` are
compile-only checks).  It is wired into CI (`repro-replay-selftest` and
`repro-replay`) and into `scripts/ci_local.sh` (fast set); run it before
touching codegen:

```bash
python3 scripts/check_repro_replay.py --self-test   # prove the harness fires
python3 scripts/check_repro_replay.py               # replay both legs
python3 scripts/check_repro_replay.py --legs v1     # one leg while bisecting
python3 scripts/check_repro_replay.py --list        # print the plan, run nothing
```

## The one methodology rule: pin the ISA on BOTH compilers

An unpinned replay (lccc's default vs GCC's default) reports **29 failures** on
a tree that has none, because the two compilers default to different ISA
baselines — lccc's documented project baseline is `x86-64-v3` (FMA available,
and its backend contracts `a*b+c` at every level, including `-O0`), while GCC
defaults to plain `x86-64` (no FMA).  The comparison then measures FMA
contraction, not codegen:

| flags handed to both compilers | files agreeing with GCC | notes |
|---|---|---|
| (none) | 68 / 97 | measures the default-baseline difference, not bugs |
| `-march=x86-64` | 94 / 97 | strict: no FMA anywhere, so a difference is semantic |
| `-march=x86-64-v3 -mno-fma` | 94 / 97 | AVX2 codegen exercised, contraction removed |

The gate runs both pinned legs, so the strict leg cannot be lost to a
"just match the default target" simplification.  The three remaining files are
**declared** in `NON_SELF_CONTAINED` inside the gate (they `#include
"csmith.h"`, which the csmith tool generates and this repository does not
vendor); the declaration is checked in both directions, so the list cannot rot
into a silent skip list.

## Naming contract (the gate's parser is the authority)

    <name>_<seed>_-O<n>.c                        optimisation level
    <name>_<K>-<V>_-O<n>.c                       one environment variable
    <name>__env_<K>-<V>[,<K2>-<V2>]_-O<n>.c      one or more (legacy spelling)

The `_<K>-<V>_` / `__env_` segment carries the environment the reproducer was
captured under: `miscompile_gen_fp_stress_seed20260936_CCC_RA_GLOBAL_LOCATION-1_-O0.c`
must be replayed with `CCC_RA_GLOBAL_LOCATION=1`, or it is a different program.
Both spellings are accepted (the `__env_` form is the pre-AUD-HYGIENE-1 shape;
the `[K=V,...]` form it replaced was dropped because brackets and `=` are
shell-hostile in file names).  A name whose knob/value segment cannot be parsed
fails the gate rather than being replayed at a guessed configuration.

## Known, documented deviations

* **FMA contraction at `-O0`.** With FMA permitted, lccc contracts `a*b+c` even
  at `-O0` (measured: 10 `vfmadd`/`vfmsub` sites in
  `miscompile_gen_fp_stress_seed20260936_-O0.c` at `-march=x86-64-v3 -O0`),
  while GCC contracts only from `-O2`.  The C standard leaves contraction
  implementation-defined, so this is legal, but it makes `-O0` output differ
  from GCC on FP-heavy code.  Tracked in the follow-up document; a
  `-ffp-contract=` flag is the obvious knob if a project needs GCC-identical
  `-O0` numerics.
* The `miscompile_gen_fp_stress_*` family was originally filed *because* of
  that default-baseline mismatch; all 24 agree under both pinned legs.

As of the 2026-10-13 audit session: 97 files, 94 replay clean per leg, 3
declared non-self-contained (`crash_csmith_*`/`miscompile_csmith_*` need the
Csmith runtime headers from `tests/corpus`), 0 failures.

## Rebuilding this directory

`scripts/fuzz_diff.py` is the differential fuzzer that writes here
(`DEFAULT_REPRO_DIR`).  Keep the naming contract when it does, or the replay
gate will refuse the file.
