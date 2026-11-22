# x86 intrinsic census

`expected.txt` is the record of `scripts/x86_intrinsic_census.py`: of the
intrinsics GCC 16.2's x86 headers define, how many a program can call under
GCC, and how many of those it cannot call under lccc, per header.

The macro gates (`check_x86_march_macros.py`, `check_x86_mflag_macros.py`)
hold the macros lccc defines to GCC's. A macro is a promise that the
intrinsics behind it exist; this census measures the promise. As recorded,
lccc lacks 6,183 of 7,502 (82.4%) in 104 headers: the AVX-512 headers
account for 4,886 of them (`avx512fintrin.h` 1,283 of 1,395, AVX-512 FP16
all 904), AVX10.2 for 761, XOP and FMA4 for 152, and the rest are AVX2 (88
of 221), F16C (all 6, with `__F16C__` defined), the SSE4.2 string
comparisons, 3DNow! (all 28: `-m3dnow` is refused), and the system
extensions whose options lccc refuses (ADX, XSAVEOPT/XSAVEC/XSAVES,
FSGSBASE, RDPID, RTM, CLWB, ...). Complete, with nothing missing, are 22
headers: SSE through SSSE3 (`xmmintrin.h` 157, `emmintrin.h` 237,
`pmmintrin.h` 11, `tmmintrin.h` 32), AVX (183), FMA (32), AVX-VNNI with
its INT8 and INT16 forms (32), SHA and SHA512, AES and PCLMUL, VPCLMULQDQ,
BMI2, LZCNT, POPCNT, RDSEED, XSAVE, PRFCHW, MWAIT and `mm_malloc.h`.

```
python3 scripts/x86_intrinsic_census.py --lccc target/fastbuild/lccc --check
python3 scripts/x86_intrinsic_census.py --lccc target/fastbuild/lccc --list f16cintrin.h
python3 scripts/x86_intrinsic_census.py --lccc target/fastbuild/lccc --update
```

`--check` fails on any difference from the record, in either direction: an
intrinsic that went missing is a regression, and one that was added is
recorded with `--update` in the change that adds it. `--list HEADER` prints
the names lccc lacks: the worklist.

GCC is asked what its headers let a program call (`-fsyntax-only`, every
extension enabled); lccc has to make an object file of the caller, because
its `-fsyntax-only` stops in front of the lowering, where a wrapper without
an instruction behind it is refused.

The headers are `*intrin.h`, `mm3dnow.h` and `mm_malloc.h`. (`cpuid.h` is
left out: lccc has no such header and reads the system compiler's.) What a
header defines is read from its text, and the reading is held to GCC's own
account: GCC is asked which functions it compiled out of each header
(`-aux-info`, with and without `__OPTIMIZE__`), and a definition GCC lists
that the reader did not find stops the tool. The headers write a
definition in more ways than a pattern foresees (the name after one blank,
two or none; indented; `__inline` or `__inline__`; a blank line in front of
the body), and a reader that misses some reports a census that is short,
which reads like a better one: the record before this check counted 7,021
of the 7,502 (no XOP, no AVX10.2, none of the 128-bit casts).

The census is about availability. Whether a call that compiles computes
what GCC's computes is `scripts/x86_intrinsic_values.py`'s question: it
runs every intrinsic lccc has that the processor executes and compares the
results with GCC 16.2's (1,261 intrinsics in 7,300 tests on an AVX-512 Xeon,
at `-O2` and `-O0`), and it asks lccc for the constants GCC refuses where
an intrinsic takes an immediate (122 intrinsics; each has to be refused
where the call stands). What neither can see is an intrinsic without a
value: `scripts/check_x86_prefetch.py` compares the prefetch instructions.
