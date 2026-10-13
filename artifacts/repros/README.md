# `artifacts/repros/` — the compiler's scar tissue, and how to replay it

Every `.c` file here was captured when lccc crashed, miscompiled, or disagreed
with GCC. They are **replayed on every CI run** by
`scripts/check_repro_replay.py` (gates `repro-replay-selftest` and
`repro-replay`), which compiles each one with lccc *and* GCC under identical
flags and requires identical exit status and output.

## The one methodology rule: pin the ISA

An unpinned replay (lccc's default vs GCC's default) reports **29 failures** on
a tree that has none, because the two compilers default to different ISA
baselines — lccc's documented project baseline is `x86-64-v3` (FMA available,
and its backend contracts `a*b+c` at every level, including `-O0`), while GCC
defaults to plain `x86-64` (no FMA). The comparison then measures FMA
contraction, not codegen:

| flags handed to both compilers | files agreeing with GCC | notes |
|---|---|---|
| (none) | 68 / 97 | measures the default-baseline difference, not bugs |
| `-march=x86-64` | 94 / 97 | strict: no FMA anywhere, so a difference is semantic |
| `-march=x86-64-v3 -mno-fma` | 94 / 97 | AVX2 codegen exercised, contraction removed |

The gate runs both pinned legs. The three remaining files are **declared** in
`NON_SELF_CONTAINED` in the gate (they `#include "csmith.h"`, which the csmith
tool generates and this repository does not vendor); the declaration is checked
in both directions, so the list cannot rot into a silent skip list.

## Naming contract (the gate's parser is the authority)

    <name>_<seed>_-O<n>.c                     optimisation level
    <name>__env_<K>-<V>[,<K2>-<V2>]_-O<n>.c   environment variables for the run

The `__env_` segment replaced the original `[K=V,...]` form (AUD-HYGIENE-1:
brackets and `=` in file names are shell-hostile). A file name that fits
neither shape fails the gate rather than being replayed at a guessed level.

## Known, documented deviations

* **FMA contraction at `-O0`.** With FMA permitted, lccc contracts `a*b+c` even
  at `-O0` (measured: 10 `vfmadd`/`vfmsub` sites in
  `miscompile_gen_fp_stress_seed20260936_-O0.c` at `-march=x86-64-v3 -O0`),
  while GCC contracts only from `-O2`. The C standard leaves contraction
  implementation-defined, so this is legal, but it makes `-O0` output differ
  from GCC on FP-heavy code. Tracked in the follow-up document; a
  `-ffp-contract=` flag is the obvious knob if a project needs GCC-identical
  `-O0` numerics.
* The `miscompile_gen_fp_stress_*` family was originally filed *because* of
  that default-baseline mismatch; all 24 agree under both pinned legs.

## Rebuilding this directory

`scripts/fuzz_diff.py` is the differential fuzzer that writes here
(`DEFAULT_REPRO_DIR`). Keep the naming contract when it does, or the replay
gate will refuse the file.
