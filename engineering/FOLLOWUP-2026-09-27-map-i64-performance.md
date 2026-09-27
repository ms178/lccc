# Follow-up: measured x86-64 I64/U64 map lowering and unroll cost

**Base:** upstream `origin/main` `e9f572eb36f5cd98c9ea1f077bc2922bd001177e` (PR #640, including the initial two-lane I64/U64 map admission). This follow-up is limited to generated-code performance and correctness; it does not change the ABI, vector lane width, or the `.eh_frame` work in the base.

## Decision and implementation

The x86-64 I64/U64 map lowering remains deliberately **two 64-bit lanes in a
128-bit XMM register**, including when AVX2 is enabled. A 256-bit I64 intrinsic
would require a complete intrinsic contract, register allocator/backend
lowering, ABI treatment, and differential corpus before it could be considered;
this change does not take that shortcut. i686 continues to use the scalar path.

A two-lane operation leaves memory-level parallelism on the table for a simple
map, so `src/passes/vectorize.rs` now uses an adaptive unroll factor:

| Map-expression size | Two-lane copies per counted iteration | Elements covered | Byte step |
| --- | ---: | ---: | ---: |
| 0–3 expression nodes (load/op/load and invariant variants) | 4 | 8 | 64 |
| 4–8 nodes | 2 | 4 | 32 |
| 9+ nodes | 1 | 2 | 16 |

The complete `packed_width`, rather than the single-intrinsic width, drives the
small-trip guard, quotient and signed-bias arithmetic, byte IV step, fused
remainder start, remainder call, stride and alias window. Every unrolled copy
clears the offset-sensitive load/expression cache and receives a fresh
expression value. Invariant broadcasts use a separate cache and are emitted
once in the preheader. Thus the optimization cannot silently reuse lane zero or
skip every other pair of elements.

For x86 vector memory operations, the unrolled copy offset is passed as an
optional constant displacement to `VecLoadI64x2` and `VecStoreI64x2`. The x86
emitter folds 16, 32 and 48 bytes into `disp(base,index)`, so the hot loop keeps
one byte-IV register and does not materialize `byte_iv + displacement` with a
per-copy LEA. The AArch64 path does not consume this x86-only optional argument;
its map lowering remains conservative, and the transform retains an SSA offset
there if a NEON two-lane map reaches that path. No target receives a fake
four-lane I64 operation.

`CCC_NO_MAP_I64_UNROLL=1` disables only this adaptive I64/U64 unroll decision,
which makes an apples-to-apples runtime A/B possible. `CCC_NO_MAP_VEC=1` remains
the complete scalar fallback and is used by the correctness gate.

## Correctness gate

`tests/regression/map_i64_two_lane.c` and
`tests/regression/check_map_i64_two_lane.sh` are the release gate. The runtime
program sweeps every trip count from 0 through 259 and checks against volatile
scalar references. It covers unsigned 64-bit wraparound, invariant subtraction,
untouched tails, and shifted/overlapping destinations. The gate compares LCCC
with both GCC and the `CCC_NO_MAP_VEC=1` LCCC control for `-march=x86-64-v3`
and baseline `-march=x86-64`, then assembles the i686 `-m32 -msse2` form.

The assembly contract additionally requires, for `sub64`, exactly four
independent `vpsubq` operations on AVX2 and four `psubq` operations on SSE2,
a 64-byte vector-loop step, no `%ymm` register in the I64 map, and no
16/32/48-byte displacement LEA in the vector body. This pins the intended
four-copy/two-lane implementation without asserting a wider lane ABI. The
validated result was:

```
map-i64-two-lane: x86-64 AVX2/SSE2 runtime+GCC+scalar, i686 scalar assembly PASS
```

The targeted regression corpus also passed the I64 map tests and
`vectorize_int_map_lanes.c` with `CCC_VALIDATE_SSA=1`.

## Assembly and oracle evidence

For `tests/regression/map_i64_two_lane.c::sub64` at `-O2 -march=x86-64-v3`,
the current LCCC assembly has four `movdqu`/`vpsubq`/store groups at offsets
0, 16, 32 and 48, increments the byte IV by 64, and then enters the scalar
remainder. The current compiler-explorer static survey reports these
instruction counts:

| Compiler | Static instructions | Interpretation |
| --- | ---: | --- |
| LCCC local | 55 | two-lane XMM body plus scalar remainder |
| GCC 16.2 | 33 | 256-bit YMM body plus scalar remainder |
| Clang 23.1.0 | 50 | 256-bit/YMM multi-copy body |
| ICC 2021.10.0 | 74 | alignment/peel and XMM multi-copy variants |
| ICX latest | 25 | 256-bit YMM body plus compact remainder |

These are static assembly counts, not throughput claims. Full oracle output is
committed in `engineering/evidence/map64-oracle-2026-09-27.json`. It was
collected with `scripts/codegen_oracle.py` and the requested GCC 16.2, Clang
23.1, ICC 2021.10 and ICX latest oracles.

## Runtime A/B evidence

The reproducible harness is `scripts/bench_kernels.py`. It compiles one kernel
translation unit per arm, links one common GCC-compiled driver, interleaves
arms, pins timed children to CPU 0, checks the volatile result, and reports the
best of 15 samples with 10,000 kernel calls per sample. The sandbox is a Xeon
2.60 GHz hypervisor VM with no usable PMU; it is not the target i7-14700KF.

At `-O2 -march=x86-64-v3`, the adaptive unroll versus the same compiler with
`CCC_NO_MAP_I64_UNROLL=1` measured:

| Arm | Best time | Relative result |
| --- | ---: | ---: |
| LCCC adaptive unroll | 1.676 ms | 1.454× faster than rolled LCCC |
| LCCC rolled two-lane | 2.438 ms | control |
| host GCC 14.2 | 1.384 ms | faster than LCCC in this VM |

The same protocol with `CCC_NO_MAP_VEC=1` measured LCCC at 1.648 ms versus
3.442 ms for the scalar control (2.089×), while GCC 14.2 measured 1.442 ms.
With baseline `-march=x86-64`, LCCC measured 1.613 ms versus 3.412 ms scalar
(2.115×); host GCC 14.2 measured 2.412 ms. These runs support the narrower
claim that the two-lane map and the adaptive unroll improve this synthetic
kernel against their local LCCC controls. They do **not** establish superiority
over GCC/Clang, package-wide gains, or native Raptor Lake throughput. The raw
unroll result is committed in
`engineering/evidence/map64-unroll-run-2026-09-27.json`; the normalized
metadata and limitations are in
`engineering/evidence/map64-unroll-displacement-2026-09-27.json`.

The local image has GCC 14.2 and no installed Clang binary for this runtime
screen. GCC 16.2, Clang 23.1, ICC 2021.10 and ICX latest therefore remain
static oracles here, not runtime timing arms. Hardware PMU measurements on an
i7-14700KF and larger gzip/zlib-ng/Expat/SQLite/Linux/glibc workload screens
remain follow-up work rather than implied evidence.

## Reproduction

From the repository root, after the project-policy fastbuild (`-O1`, maximum
`-j2`) build:

```sh
./scripts/build_lccc_fast.sh
CCC=target/fastbuild/lccc bash tests/regression/check_map_i64_two_lane.sh
CCC_VALIDATE_SSA=1 python3 tests/regression/run_regression.py \
  --lccc target/fastbuild/lccc --filter vectorize_int_map_lanes -j 2
python3 scripts/bench_kernels.py --kernels map64_sub \
  --lccc-alt-env CCC_NO_MAP_I64_UNROLL=1 --reps 15 --inner 10000 \
  --flags='-O2 -march=x86-64-v3'
python3 scripts/codegen_oracle.py tests/regression/map_i64_two_lane.c \
  --function sub64 --local target/fastbuild/lccc \
  --flags '-O2 -march=x86-64-v3' --local-flags '-O2 -march=x86-64-v3' \
  --oracles gcc16.2,clang,icc,icx --artifact-dir /tmp/oracle-map64
```

The full local CI mirror remains the release gate. A fastbuild/test pass is
necessary but does not replace `ci_local.sh`, Clippy, the snapshot apply-check,
or the final patch/bundle/ledger publication.
