# x86 intrinsic census

`expected.txt` is the record of `scripts/x86_intrinsic_census.py`: of the
intrinsics GCC 16.2's x86 headers define, how many a program can call under
GCC, and how many of those it cannot call under lccc, per header.

The macro gates (`check_x86_march_macros.py`, `check_x86_mflag_macros.py`)
hold the macros lccc defines to GCC's. A macro is a promise that the
intrinsics behind it exist; this census measures the promise. As recorded,
lccc lacks 5,736 of 7,021 (81.7%): the AVX-512 headers account for 4,400 of
them (`avx512fintrin.h` 1,282 of 1,394), AVX10.2 for 470, and the rest are
AVX2 (88 of 221), F16C (all 6, with `__F16C__` defined), the SSE4.2 string
comparisons, and the system extensions whose options lccc refuses (ADX,
XSAVE*, FSGSBASE, RDPID, RTM, CLWB, ...).

```
python3 scripts/x86_intrinsic_census.py --lccc target/fastbuild/lccc --check
python3 scripts/x86_intrinsic_census.py --lccc target/fastbuild/lccc --list f16cintrin.h
python3 scripts/x86_intrinsic_census.py --lccc target/fastbuild/lccc --update
```

`--check` fails on any difference from the record, in either direction: an
intrinsic that went missing is a regression, and one that was added is
recorded with `--update` in the change that adds it. `--list HEADER` prints
the names lccc lacks: the worklist.

The census is about declarations. Whether a call that compiles computes
what GCC's does is for the execution suites (`tests/simd`).
