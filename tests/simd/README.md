# SIMD correctness probes

These are regression probes, not a claim of compiler performance superiority.
The 44-row `scoreboard_kernels.c` now has a strict checksum-only gate:

```sh
python3 tests/simd/check_scoreboard.py --reference gcc-16
python3 tests/simd/check_scoreboard.py --reference clang-23 --reps 17 --json /tmp/scoreboard.json
```

The independent 44-name inventory rejects missing, unknown and duplicate rows,
malformed checksums and invalid timings; timings are never compared as an
acceptance criterion. Reference compilation/execution failure stops the gate.
Each candidate at O1/O2/O3 is compared against the scalar O0 reference with
contraction off and all C warnings treated as errors. `REPS=1` is the fast
correctness case; 1..4000 is selectable, and matmul always executes at least
once. The reduction sink uses defined unsigned wrapping; shift inputs convert
to unsigned before left shift. FP reduction receipts retain every result bit,
and matrix hashing stays within each row's C array bounds. GCC 16.2 UBSan
reproduced the old signed-sink overflow; the repaired full default corpus
completed without a UBSan diagnostic.

**Timing is still exploratory:** saxpy initializes inside its timed call,
sqrt includes negative inputs, and repeated kernels are not an order-balanced,
affinity-controlled hardware benchmark. These known limitations are not
concealed by the new checksum gate. The initial performance table cannot be
reproduced with changed checksums as if it were an unchanged baseline.

## Interleaved-access regression

```sh
python3 -m unittest discover -s tests/simd -p 'test_*.py'
python3 tests/simd/aos_pair_shapes.py --lccc target/fastbuild/lccc --reference gcc-16
python3 tests/simd/aos_pair_shapes.py --sweep --guarded \
  --lccc target/fastbuild/lccc --reference clang-23 --json /tmp/aos.json
python3 tests/simd/aos_pair_shapes.py --guarded --mixed --type int16_t --stride 4 \
  --lccc target/fastbuild/lccc --reference gcc-16
```

The default retains seven source shapes and tests `-O1/-O2/-O3`. `--sweep`
selects 18 trip counts around unroll/vector boundaries, including zero.
`--guarded` puts each object immediately against either the start or the end
of a writable page surrounded by inaccessible pages (Linux mmap API). The
zero-trip case allocates one sentinel element. `--stride 2|3|4` changes pair
spacing; the three-field control stays three-strided. `--type` selects float,
double, int32_t or int16_t. `--mixed` selects bounded mixed-sign inputs.
The Cartesian product is selectable, not all run by default.

Every output element, including untouched fields/padding, is compared using
hexadecimal floating representation after exact promotion to double. All
current integer inputs/intermediates fit their types for the selected trips;
FP contraction is disabled on both compilers. The reference is built at `-O0`.
This tests finite values, not the complete NaN/exception/rounding-mode domain.
It is not an alias-overlap or randomized-seed legality fuzzer.

Compiler commands are parsed into argv without a shell. Compilation failure,
nonzero exit, signal, timeout, missing executable, wrong output length and
output mismatch fail the suite. Reference failure never supplies an empty
expected result. Executables are unlinked before compilation; all generated
sources/executables live in a private temporary directory, removed afterward.
Set `--json` to retain case-level receipts outside the checkout.

## Root cause and invariants

SV-01's missing machine index is a **backend dead-offset analysis** defect,
not a missing definition in unroller IR. Disabling unrolling merely avoids
creating the address combinations that expose it.

One address can peel through `i+1`, while another retains `i+1` as its SIB
index. The old per-address walk could suppress that index's definition even
though the allocator had assigned it a register for the second address.
`dead_indexed_offset_producers` now protects every selected index globally,
counts only guaranteed load-fold uses (stores may need offset rematerialization),
and revisits shared operands when their final attributable use disappears.
Ambiguous definitions and ordinary uses remain materialized. This keeps
address folding and useful offset elimination enabled, rather than disabling
unrolling/vectorization to avoid the symptom.

Five Rust unit cases cover the cross-address root, shared-offset fixed point,
store fallback, ordinary uses and ambiguous definitions. Runner tests cover
failure propagation, stale artifacts, guard construction, command parsing,
JSON completeness and invalid timeouts. GCC CI runs the default and mixed-sign
four-strided page-boundary probes; the existing authenticated ICX 2026.2 lane
runs the same cases against its vendor snapshot.

## Assembly oracles

`aos_index_roots.c` is a small stable input for the repository's Godbolt
comparator. Network-free comparison with verified local compilers:

```sh
python3 scripts/godbolt.py compare --offline tests/simd/aos_index_roots.c \
  --function aos_crossadd --flags='-O2 -mavx2 -mfma -ffp-contract=off' \
  --local target/fastbuild/lccc --oracles '' \
  --local-ref gcc16=/path/to/gcc-16 --local-ref clang23=/path/to/clang-23 \
  --artifact-dir /tmp/aos-oracle --json /tmp/aos-oracle/manifest.json
```

The local screening observed 41/9/29 static instructions for lccc/GCC
16.2/Clang 23.1.4. These counts are **not runtime results**; the general
vectorization gap remains. GitHub reported LLVM 23.1.4 as latest and returned
404 for `llvmorg-23.1.5` during this session. Remote Godbolt and Intel's PPA
are outside this sandbox's network allowlist; ICX execution is delegated to
its authenticated hosted lane, not claimed as locally performed.
