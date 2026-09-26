# Follow-up: measured x86-64 I64 map lowering and remaining codegen gaps

**Base:** upstream `origin/main` `8624a5d069f9473adeefdaae26378f69681d8bb4` (PR #639; fetched again 2026-09-27). The `.eh_frame` hardening was rebased onto this base; see `FOLLOWUP-2026-09-26-pr638-eh-frame-bounds.md`. This follow-up concerns **generated-code performance**, not an unwinder-speed claim. The patch is relative to this base, not to the old `a89e685` checkout.

## Finding and change

`VecSubI64x2` was already wired to x86-64 SSE2 `psubq` (VEX `vpsubq` when enabled), but the map analyzer rejected I64/U64 loads and stores. Simply admitting them under AVX2 would be **incorrect**: the map transform computed `32 / 8 = 4` elements per iteration while every I64 load/sub/store intrinsic handled only **two**. It would silently skip half the destination. No such transform was active before this change. Admit I64/U64 only on the **x86-64 target**, use 16-byte/two-lane XMM operations even with AVX2, and retain the original scalar fallback for i686 (whose backend cannot lower the register-based Vec*I64x2 family), NEON, SIMD-disabled targets, and shapes outside the existing conservative map grammar. The target permission is refreshed per translation unit rather than inferred from the compiler-host architecture. This enables subtraction, addition and supported existing elementwise operations without changing the arithmetic semantics or the alias/dependence guard.

`tests/regression/map_i64_two_lane.c` checks every trip count from 0 through 259, unsigned 64-bit wraparound, operand order with invariant subtraction, untouched tails, and overlapping/shifted destinations against volatile scalar references. `check_map_i64_two_lane.sh`, installed as a **fast CI gate**, checks runtime output against both GCC and LCCC's `CCC_NO_MAP_VEC=1` control on `-march=x86-64-v3` and baseline `-march=x86-64`; pins `vpsubq`/`psubq` in the generated `sub64`, verifies that AVX2 did not accidentally use YMM, and assembles the **i686** control (`-m32 -msse2`) without a packed I64 map operation. The existing `vectorize_int_map_lanes.c` regression also exercises I64/U64 output against GCC under `-O3`. This is a target-specific, recoverable speed improvement; the switch permits an A/B and an emergency scalar fallback. After the change, `-O2 -march=x86-64-v3` assembly for each of the four existing gzip CRC32, zlib-ng Adler32, Expat XML scan and SQLite varint proxy programs was **byte-identical** between the default and `CCC_NO_MAP_VEC=1` control: this particular optimization does not change their codegen. A second seven-round correctness-checked runtime screen passed all four; the Expat LCCC/Clang paired ratio was 1.311 (pre-change 1.312), despite noisy absolute times. Saved output: `/home/user/research/workloads-map64-final.json`. This does not establish that unrelated map-heavy real programs have no regressions.

## Reproducible A/B measurement

Run from repository root after the `fastbuild` (`-O1`, `-j2`) build:

```sh
python3 scripts/bench_kernels.py --kernels map64_sub \
  --lccc-alt-env CCC_NO_MAP_VEC=1 --reps 9 --inner 10000 \
  --flags='-O2 -march=x86-64-v3' --save map64-bench.json
bash tests/regression/check_map_i64_two_lane.sh
```

The committed deterministic kernel `tests/bench/k_map64_sub.c` performs 1,024 64-bit subtracts per `bench_run`, reads an output and mutates an input to prevent hoisting. The common reference-compiled driver, identical compiler flags, output check, CPU-0 pinning, interleaved arms and nine samples are handled by `bench_kernels.py`. In this **Xeon 2.60 GHz hypervisor VM**, with no usable PMU and no i7-14700KF, best-of-nine times were:

| Arm | Time (ms) | Relative to LCCC scalar |
| --- | ---: | ---: |
| LCCC two-lane (new) | **1.601** | **1.61× faster** |
| LCCC `CCC_NO_MAP_VEC=1` | 2.571 | baseline |
| host GCC **14.2** | 0.910 | faster than the new LCCC path |
| host Clang **19.1** | 1.301 | faster than the new LCCC path |

Exact times and distinct timed-function code hashes are in `engineering/evidence/map64-bench-2026-09-27.json`; the harness refuses a result mismatch. The speed gain is **only this kernel on this VM**. It does not establish superiority over GCC/Clang, on different memory sizes, or on the user's i7-14700KF. The i686 lowering is deliberately scalar; the test compiles/assembles that target but does not time it. No regression on the four existing workloads was inferred from this synthetic microbenchmark.

## Other workload and oracle evidence (pre-optimization tree)

A four-workload paired-median screen with seven randomized, pinned rounds and output checks used `-O2 -march=x86-64-v3` and host GCC 14.2 / Clang 19.1. LCCC divided by the fastest reference: gzip CRC32 **0.879** (faster); zlib-ng Adler32 **1.094** (slower); Expat XML scan **1.312** (slower); SQLite varint **1.162** (slower). Expat and several others varied by >5% across rounds, so treat their ordering and magnitudes as *screening*, not a statistically robust hardware result. Source: `/home/user/research/workloads-pr639.json` and `.md` from `tests/benchmark/run_benchmarks.py`. An existing timed-kernel screen also gave Adler DO8 0.307 ms vs GCC 3.318 ms / Clang 3.223 ms, but memchr was 6.949 ms vs Clang 4.018 ms; no universal win is claimed. Different fixtures and timing protocols must not be merged into one aggregate.

The **separate, untimed** Compiler Explorer survey (`scripts/codegen_oracle.py`, GCC **16.2**, Clang **23.1**, ICC, ICX; four workload files) found an Expat `main` count of 154 LCCC vs 94 GCC, predominantly corpus initialization: GCC emits two YMM stores per 60-byte fragment, whereas LCCC copies each byte. Switching both oracles to `-fPIE` gave 154 vs 98, so the gap is not solely a PIE/non-PIE addressing artifact. It is **not proof** that the initialization dominates the 64-pass scan. SQLite `sqlite_get_varint` was 121 vs 115 Clang instructions; a small count gap alone does not explain the measured whole-program difference. Survey artifacts: `/home/user/research/oracle-pr639.json`, `/home/user/research/expat-fpie/`. No GCC 16.2, Clang 23.1, ICC or ICX **runtime** timing was available; their oracle results are static assembly comparisons, not throughput claims.

## Validation boundary and next investigation

A rebased **pre-map-change** fast CI was 83 pass / 0 fail / 3 skip and was saved as S05. The first map-change fast CI exposed a missing **hosted CI mirror** for the new local gate (83 pass / 1 fail / 3 skip); the workflow was fixed, not bypassed. The next exact-tree run passed **84 / 0 / 3** and was snapshotted as S06. After this final documentation update, only the matching `target/ci_local.pass` and `/home/user/artifacts/SNAPSHOT_LEDGER.md` certify the delivered tree; do not substitute S05 or S06 for a later edit. The fast profile intentionally skips slow SSA/corpus/benchmark-output gates; a full slow CI, cross-target ARM/RISC-V runtime, full upstream zlib-ng/gzip/Expat/SQLite projects, glibc and kernel builds, native Raptor Lake PMU counters, and runtime linker-oracle differentials remain **unperformed**. The reported benchmark programs are realistic kernels/proxies, not full package builds.

For further optimization, first localize Expat's 64-pass scan and SQLite's decoder using controlled per-function measurements and inspect their assembly/spill costs; do not optimize a `main` initialization merely because it has more static instructions. On the I64 map kernel, GCC 14.2 and Clang 19.1 remain faster: the two-lane limitation and scalar remainder/unrolling are candidates for profiling, **not** permission to change lane width without a complete 256-bit I64 intrinsic/ABI implementation and differential tests. Re-run exact-tree fast CI and atomically snapshot every validated change before any further experiment.
