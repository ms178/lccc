# SIMD correctness probes

These are regression probes, not a claim of compiler performance superiority.
The 44-row `scoreboard_kernels.c` now has a strict checksum-only gate:

```sh
python3 tests/simd/check_scoreboard.py --check-clock-failure --reference gcc-16
python3 tests/simd/check_scoreboard.py --reference clang-23 --reps 17 --json /tmp/scoreboard.json
```

The independent 44-name inventory rejects missing, unknown and duplicate rows,
malformed checksums and invalid timings; timings are never compared as an
acceptance criterion. Reference compilation/execution failure stops the gate.
Each candidate at O1/O2/O3 is compared against the scalar O0 reference with
contraction off and all C warnings treated as errors. `REPS=1` is the fast
correctness case; 1..4000 is selectable, and matmul always executes at least
once. The optional clock-failure test links an injected failing clock into separate reference/candidate binaries and requires exit 70 plus its diagnostic (not a compiler error, signal or silent success).
The reduction sink uses defined unsigned wrapping; shift inputs convert
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
counts real GEP edges (not alias metadata), protects fallback offsets,
and revisits shared operands when their final attributable use disappears.
Ambiguous definitions and ordinary uses remain materialized. This keeps
address folding and useful offset elimination enabled, rather than disabling
unrolling/vectorization to avoid the symptom.

Eleven Rust analysis cases cover the cross-address root, shared-offset fixed point,
store fallback, ordinary uses, ambiguous definitions, effectful/FP producers, incomplete metadata and a 256-case independent oracle. Target tests cover default refusal and 156 x86 store profiles. Runner tests cover
failure propagation, stale artifacts, guard construction, command parsing,
JSON completeness, invalid timeouts and pipe-inheriting descendant termination. GCC CI runs the default and mixed-sign
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

## LightPixel / unsigned dword min/max

```sh
python3 tests/simd/check_lightpixel.py --require-native \
  --lccc target/fastbuild/lccc --reference gcc-16 --json /tmp/lightpixel.json
python3 tests/simd/check_lightpixel.py --require-native \
  --lccc target/fastbuild/lccc --reference clang-23
python3 tests/simd/benchmark_lightpixel.py --reference gcc-16 --rounds 24 \
  --compiler candidate=target/fastbuild/lccc --compiler gcc16=gcc-16 \
  --compiler clang23=clang-23 --json /tmp/lightpixel-timing.json
```

The independent reference-built O0 harness checks 65,536 cases / 12 kernels
at O2/O3 with SSE2-only, SSE4.1-only and AVX2 targets. It exercises arbitrary
uint32 wrapping scale multiplication, exact three-byte guarded sources,
four-byte guarded destinations, overlapping outputs, interfering aliases,
volatile byte accesses, scattered sinks and full-width unsigned min/max.
Compilation/runtime failure, incomplete receipts and missing native codegen
fail the gate. Timings do not decide correctness. `benchmark_lightpixel.py`
uses a common reference-built driver, checks hashes against O0 kernels, pins
to one available CPU and reverses arm order on alternate rounds; it measures
external-call microkernels, **not Firefox/rasterflood-svg**. CPU frequency and
other virtual-host tenants remain uncontrolled.

See `engineering/LIGHTPIXEL-AUDIT-2026-11-01.md` and its raw JSON receipts.
The native U32 min/max component improves, but lccc's exact three-channel
RGB/alpha graph remains scalar. Do not describe this gate as proof that the
LLVM PR227303 profitability optimization is implemented.

## Implementation-audit reproducer pack (2026-11-08)

Evidence and fix plans: `engineering/SIMD-IMPLEMENTATION-AUDIT-2026-11-08.md`
(§9c); measured outcomes and the red-team re-audit in §9c.9.  The pack grew
from six additive files to fifteen: the original corpus, six independent
drivers, three oracles/goldens, a provenance record and a unittest pack.

| File | Role |
|------|------|
| `audit_repro_2026-11-08.c` | C99 entry-point library; `list` enumerates all eight |
| `probe_utils.py` | shared process/CPUID/diagnostic layer, no side effects |
| `check_audit_2026-11-08.py` | 15-row driver: 9 hardened `fixed` controls + 6 controls |
| `simd_shape_matrix.c` | 29 bit-exact differential shapes, three tiers |
| `check_simd_shape_matrix.py` | 12 optimization/ISA flag sets, sanitizer + bench legs |
| `simd_isa_matrix.c` | 42 shapes across x86 tiers, NEON, SVE and RVV |
| `check_simd_isa_matrix.py` | 12 host flag sets + 7 QEMU cross legs, sanitizer + bench |
| `gen_isa_permissions.py` / `isa_permissions_gcc16.json` | GCC16-derived feature-permission golden (97 wrappers) |
| `check_isa_permissions.py` | permission-table consistency gate |
| `check_assembler_widths.py` | GNU as 2.47 byte-exact VEX.L=1 width parity (16 cases) |
| `check_yarpgen.py` | seeded UB-free YARPGen 2.0 differential fuzz, 10 legs |
| `check_lccc_capability.py` / `lccc_capability_gaps.json` | pinned LCCC capability-debt ratchet |
| `test_audit_pack.py` | unittest pack wired by the existing CI discovery step |

All seven drivers have a pure `--self-test`; `test_audit_pack.py` runs every
one of them in CI.  Combined self-test count: **117 checks** (39 + 15 + 16 +
5 + 13 + 18 + 11).

### Finding driver

```sh
python3 tests/simd/check_audit_2026-11-08.py --list-rows
python3 tests/simd/check_audit_2026-11-08.py --self-test
python3 tests/simd/check_audit_2026-11-08.py --lccc target/fastbuild/lccc --reference gcc-16
python3 tests/simd/check_audit_2026-11-08.py --lccc target/fastbuild/lccc --strict
python3 tests/simd/check_audit_2026-11-08.py --lccc "$(which gcc)" --strict  # harness oracle
```

Nine rows were reproduced at runtime (SA-01/02/03/04/05/06/07/13/33), the
corresponding compiler defects were fixed, and each row was promoted from
`open` to `fixed` — a hard control that must print `PASS`.  A failing control
is `REGRESSION` in both modes; an injected compiler ICE is also `REGRESSION`,
never an expected `OPEN-CONFIRMED`.  Rows tagged `open` (none remain) would
print `OPEN-CONFIRMED`; a passing open row is `STALE-OPEN`.
`ERROR` is infrastructure and always hard; `SKIP` means the host lacks the
CPUID feature needed to execute the compiled probe and is never hard.
`--strict` inverts the open rows into required passes for post-fix validation.
A compiler without the findings (GCC) is strict-green — that is the harness's
own end-to-end check.  `--json` writes a schema-1 receipt and creates parent
directories; `--compile-timeout` / `--run-timeout` bound every child process.

Compile-only rows classify a failed compile as a pass **only** when stderr is
a real diagnostic.  An ICE, a timeout, an empty stderr or a missing toolchain
is `ERROR`.  Asm rows prove the kernel was emitted (function label plus a
multiply) before asserting that an instruction is absent; `sa33_asm` also
requires `vzeroupper` after `vinserti32x4`.  Runtime rows gate on host CPUID,
not on target predefined macros, and `-ffp-contract` is pinned for SA-02 with
an `=off` control.

The lccc-dependent driver is still **not** a hard CI gate: hosted CI builds
the compiler in a separate lane, so a red compiler build would otherwise be
indistinguishable from a red harness.  What CI *does* gate is the harness
itself through `test_audit_pack.py` (39 tests, including permanent injected
wrong-code mutations and all seven driver self-tests).  Locally the full
driver is strict-green against the fastbuild binary: 15/15 rows `PASS`.

### Shape matrix

```sh
python3 tests/simd/check_simd_shape_matrix.py --candidate gcc --reference gcc --sanitize --bench
python3 tests/simd/check_simd_shape_matrix.py --candidate target/fastbuild/lccc --quick
```

Every shape has a vectorizable `fast()` and a de-optimized `ref()` oracle
(volatile induction variable).  For each of the 12 flag sets the driver checks
that the two digests agree within the build and that both match a shared
`-O0` reference build.  Any compiler diagnostic is an infrastructure error —
zero warnings is a hard requirement.  `--sanitize` adds an ASan+UBSan leg;
`--bench` reports median ns/iteration per shape on the host CPU without
polluting the compared stdout stream.  Shapes come from `tests/bench/k_*.c`,
`tests/benchmark/kernel_corpus/k*.c`, the C sources in `tests/simd/`, and the
ms178/archpkgbuilds packages (zlib-ng, gzip, zstd, SQLite, expat, curl,
glibc, Linux jhash, LightPixel, simdjson, Chromium/Skia, FFmpeg,
libjpeg-turbo); `list` prints each shape's provenance.  The corpus is C99 and
portable to x86-64, i686, AArch64 and RISC-V: no POSIX calls, no `_Alignas`,
no strict-aliasing puns, AVX-512 behind predefined macros with scalar
fallbacks.

### CI integration

`test_audit_pack.py` is discovered by the existing
`python3 -m unittest discover -s tests/simd -p 'test_*.py'` gate step.  Adding
a test file changes **no workflow text**, so `.github/ci-lane-plan.json`
stays byte-identical and `tests/regression/check_ci_lane_plan.py` stays green.
Adding a step to a sharded lane would instead require regenerating the whole
plan — deliberately avoided.


## Full-ISA shape matrix (2026-11-08)

```sh
python3 tests/simd/check_simd_isa_matrix.py --candidate gcc-16 --reference gcc-16 --sanitize --bench
python3 tests/simd/check_simd_isa_matrix.py --candidate clang-23 --reference gcc-16 --sanitize --bench
python3 tests/simd/check_simd_isa_matrix.py --candidate gcc-16 --reference gcc-16 --cross
python3 tests/simd/check_isa_permissions.py
python3 tests/simd/check_assembler_widths.py --require-gas-247
python3 tests/simd/check_yarpgen.py --seeds 48 --json /tmp/yarpgen.json
python3 tests/simd/check_lccc_capability.py
```

Measured on the host (Xeon family 6 model 106, AVX-512 F/DQ/BW/VL/VNNI,
GCC 16.2.0, Clang 23.1.5, GNU as 2.47.20260726, QEMU 11.1.3): GCC16 12/12
configurations x 42 shapes, Clang23 12/12 x 42 shapes (both with an ASan/
UBSan leg and a median-ns bench leg), and 11/11 QEMU cross legs covering
AArch64 NEON, AArch64 SVE, RVV at VLEN 128/256/512/1024 and i686 SSE2 —
**0 problems**.  `available=` reporting prevents a disabled ISA from being
counted as exercised; missing tools are SKIP, never PASS.

`check_lccc_capability.py` compiles the same strict 42-shape corpus against
`target/fastbuild/lccc` and ratchets the exact pre-existing capability debt
(4 GCC warning groups with no LCCC implementation, 3 absent intrinsics).
A NEW gap is a regression; a gap that disappears is a hard failure until the
golden is reviewed.  No diagnostic is filtered as SKIP.  This is why the
reference gate above is described as a *reference-toolchain* gate rather than
as evidence that LCCC implements the whole corpus.

## YARPGen differential fuzzing

Provenance: `engineering/toolchains/yarpgen-provenance.json`; provision with
`python3 scripts/provision_yarpgen.py` (pinned commit, github.com only).
`-s <seed> --std=c --max-array-dims=3` keeps generated images inside the
supported small code model; an oversized image is classified SKIP **only**
when every leg fails with the same 32-bit relocation overflow.  A partial
failure is a hard FAIL.  The default 240 s per-child timeout is ratcheted
against the measured 151 s worst case (seed 1029, GCC 16.2 `-O3
-march=native -funroll-loops`); a 90 s default previously produced a bogus
compiler failure whose reason was truncated out of the receipt.  `-mcmodel=large` is now refused with an actionable
diagnostic instead of silently degrading to the small model and failing at
link time with advice the user had already followed.

## Measured red-team evidence (§9c.9)

* Fastbuild `cargo build --profile fastbuild --locked -j 1 -D warnings`:
  clean; `cargo test --profile fastbuild --lib`: 8477 passed / 0 failed /
  10 ignored.
* `check_audit_2026-11-08.py --strict`: 15/15 `PASS`.
* `check_assembler_widths.py --require-gas-247`: 16 cases, 8 byte-identical
  encodings, 0 problems.
* GDB 18.1 on the LCCC `-O2 -g` binary: SA-06 returns 0x480bac99 /
  0x8d833099 and SA-07 returns 0xf29489f3, byte-identical to GCC 16.2 `-O0`;
  no ICE.  `objdump` of the vectorized kernel shows 8 `paddd`, 7 `pshufd`,
  6 `pslld`, 6 `psrld`, 6 `por`, 8 `pxor` — the fix keeps SIMD, it does not
  disable vectorization.
* Valgrind 3.19 on static binaries: 14 glibc `__libc_cleanup_*` false
  positives in BOTH the LCCC and the GCC16 reference build, 0 heap bytes in
  use at exit, 0 diagnostic frames reaching the kernels in the LCCC build.
* YARPGen 2.0 quick: 24/24 seeds x 4 legs, 0 problems; code-model probe
  PASS.
