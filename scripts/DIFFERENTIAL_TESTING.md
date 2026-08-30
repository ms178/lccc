# Differential Testing Harnesses (yarpgen / csmith)

Two infinite differential-testing harnesses, adopted verbatim from John Regehr's
`claudes-c-compiler` **yarpgen** branch (CC0, https://github.com/regehr/claudes-c-compiler).
Regehr (University of Utah) is the author of YARPGen and a co-author of Csmith —
the industry-standard tools for finding compiler miscompiles. These harnesses
generate random C99 programs, compile them with LCCC (and GCC/Clang), run each
binary, and flag any behavior mismatch (stdout or exit code). **Any disagreement
is a compiler bug** — the reproduced case is kept under `work_root/case_*`.

They are exactly the tools that produced the regression corpus in
`tests/regression/regress_*.c` (see the porting notes in
`tests/regression/README_REGEHR_YARPGEN.md`).

## Prerequisites

* `yarpgen` — https://github.com/VoR0n0k/yarpgen  (build → `yarpgen/build/yarpgen`)
* `csmith` — https://github.com/csmith-project/csmith  (optional; needed only for `csmith_diff.py`)
* An LCCC build: `target/fastbuild/lccc` (fastbuild) or `target/release/lccc`

## yarpgen_diff.py

```bash
./scripts/yarpgen_diff.py \
  --yarpgen ~/yarpgen/build/yarpgen \
  --ccc ./target/fastbuild/lccc \
  --gcc gcc \
  --clang clang \
  --jobs 4 \
  --work-root ./yarpgen_cases \
  --keep-skipped
```

* Runs forever, in parallel, generating one case per iteration.
* Each case = `driver.c` + `func.c`. All three compilers build them with
  `-std=c99 -w`; every binary is then run and stdout+exit-code compared.
* **Mismatch** ⇒ prints the failing case dir (kept) and a summary of the
  differing outputs. This is the trigger for a bug-fix cycle.
* `--keep-skipped` preserves cases where one compiler failed to compile/run
  (may indicate a parse/codegen gap rather than a miscompile).
* Adjust `--jobs` to your core count; on a constrained host start with `-j2`.

## csmith_diff.py

```bash
./scripts/csmith_diff.py --csmith /path/to/csmith/build/src/csmith \
  --ccc ./target/fastbuild/lccc --gcc gcc --clang clang \
  --num-cases 500 --jobs 2
```

Csmith programs are more complex than yarpgen's and stress deep aggregate /
bit-field / pointer semantics — the exact areas of the `regress_*_init` cluster.

## Why this matters

LCCC's correctness bar is the hard constraint of the project: a fast compiler
that miscompiles is worthless. These harnesses give a **reproducible,
automated** path to keep finding (and then locking in as regression tests) the
real bugs that a compiler-reuse corpus like yarpgen/csmith is best at exposing.
Every bug they find should be reduced, ported into `tests/regression/`, and
fixed — see the follow-up docs in `docs/`/`engineering/` for the session-by-session
log of cases found and fixed.
