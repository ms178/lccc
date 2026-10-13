# Historical reproducers (crash + miscompile corpus)

Every file here was produced by the differential fuzzers (`scripts/fuzz_diff.py`,
`scripts/csmith_diff.py`, `scripts/yarpgen_diff.py`, `scripts/gen_fp_stress.py`)
or reduced by hand from a real failure.  They are the project's permanent
evidence: a crash reproducer also documents how a crash was fixed.

**They are replayed, not archived.** `scripts/repros_replay.py` compiles each
file with the current LCCC build and with GCC, runs both, and requires identical
stdout and exit status (files without `main` are compile-only checks).  Run it
before touching codegen:

```bash
python3 scripts/repros_replay.py            # report
python3 scripts/repros_replay.py --check    # CI ratchet (new failures are red)
```

Two rules the replay harness encodes, both learned the hard way:

* the GCC arm is compiled with `-march=x86-64-v3`, because that is LCCC's
  documented default code-generation target.  Comparing a v3 binary against a
  baseline-x86-64 binary reports the ISA difference (FMA contraction, AVX2
  vectorisation) as a miscompile -- which is exactly how a whole family of
  `miscompile_gen_fp_stress_*` files came to be filed while both results were
  conforming;
* the optimization level is taken from the file name (`..._-O0.c`), so the
  reproducer is replayed the way it was found.

As of the 2026-10-13 audit session: 97 files, 94 replay clean, 3 skipped
(`crash_csmith_*`/`miscompile_csmith_*` need the Csmith runtime headers from
`tests/corpus`), 0 failures.
