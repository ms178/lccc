# Legal SSE2 low-dword products and packed maps

Local date: 2026-10-25 (Europe/Berlin). Runtime samples began at
2026-10-24T22:32 UTC. This is generated-program runtime work, not compiler
latency work. It is a focused instruction/feature audit, not a complete SDM
review or a claim of universal optimality.

## Defect and change

`VecMulI32x4` dispatched directly to `pmulld` even for plain x86-64, SSE3,
SSSE3 and explicit SSE4.1 denials. The SSE2 `pmuludq` reconstruction existed
but was not called. Modern host execution could not expose the ISA leak.
The new gate demonstrably fails the original compiler on `default/product_dot`.

The dispatcher now calls the existing ISA-aware implementation. Its SSE2
sequence selects odd dwords with non-destructive `pshufd $0xf5`, eliminating
a copy-and-shift pair. The reconstruction takes seven SIMD instructions,
excluding input loads/result storage. Both full 64-bit `pmuludq` products
are shuffled before `punpckldq` selects their low dwords; high product bits
cannot contaminate neighboring lanes. Low 32-bit products are independent
of signedness. The existing allocator reservation of xmm2 is retained.

With a legal backend available, the map vectorizer no longer refuses the
four-lane I32/U32 multiply solely because SSE4.1 is absent. SSE4.1 retains
`pmulld`; AVX2 retains its direct packed multiply. No FP transformation,
reassociation permission, ISA default, or benchmark-specific dispatch changed.

## Paired runtime evidence

`map-runtime.json` is the canonical benchmark runner's raw report: 15 paired
rounds, two excluded warmups, randomized compiler order, CPU 0 affinity,
all samples retained. All four arms printed `beb203e8a1705383` and exited
zero on every run. Every arm's minimum runtime exceeded 416 ms.

| Plain x86-64 arm | Median | Minimum |
|---|---:|---:|
| Unchanged LCCC | 969.03 ms | 954.86 ms |
| Candidate LCCC | 506.99 ms | 497.87 ms |
| GCC 16.2 | 512.66 ms | 496.19 ms |
| Clang 23.1.3 | 429.28 ms | 416.27 ms |

Paired candidate/before ratio: **0.51990 [0.51484, 0.53474]**, equivalent
to **1.923× [1.870×, 1.942×]** speedup. The old map really was scalar;
its retained disassembly contains no illegal `pmulld`. This runtime comparison
therefore does not reward the candidate for replacing an illegal instruction
with a legal but slower one.

Candidate/GCC: **0.99791 [0.97344, 1.00998]** (no demonstrated difference).
Candidate/Clang: **1.17793 [1.17415, 1.18648]** (remaining ~18% gap).
This is a cache-resident synthetic pointwise map, not a codec/application
geomean. VM screening only: PMU unavailable, hypervisor present; GCC wall-time
CV was 4.7%. MAD outliers were retained, never discarded. Do not generalize
this result to all products or all memory sizes.

GCC 16.2 and Clang 23.1.3 were restored from manifest-verified repository
cache tags. GNU as/ld 2.47.20260726 were built from the verified 2.47 source
archive before the retained comparisons; `effective-tools.txt` records the
actual oracle driver/link commands. The Clang wrapper explicitly selects the GCC 16.2 runtime/toolchain rather
than the host GCC 12 installation; its complete contents are retained as
`clang23-gcc16.sh`. The earlier default-runtime screen was superseded by
this full 15-pair rerun. Both LCCC binaries were built using the
Rust 1.97.0 cached toolchain, fastbuild, -j2, GCC 16.2 host driver, with 8 GiB
swap. Rust 1.97 is the cached toolchain, not a claim of latest stable Rust.

## Independent correctness and feature checks

- 808 deterministic cases per arm: negative/zero counts and every length
  through 97; high-bit/overflowing inputs; signed wraparound; same-input dot
  products; dual reductions; FP values live across xmm2 scratch use; explicit
  four-lane SLP stores and vectorized maps. Inputs and output end at protected
  pages, covering unaligned starts and every scalar tail.
- Thirteen LCCC arms: default, SSE2, SSE3, SSSE3, SSE4.1, SSE4.2, AVX, AVX2,
  explicit SSE4.1 denial, scalar-XMM homes disabled, vector homes disabled,
  both home classes disabled, and final VEX promotion disabled. Exact stdout
  and successful exits match independent GCC and Clang executables.
- SSE2 SIMD opcode allowlist plus mandatory packed products: disabling
  vectorization cannot make the gate pass. SSE4.1/AVX2 must retain direct
  multiplication. The gate is in hosted codegen CI and local targeted CI.
- Two executable assembly mutants are automatically checked by that gate:
  wrong odd-lane selectors (28 sites), and swapped even/odd products
  (14 sites). Both execute successfully but produce wrong output. The latter
  preserves dot sums; lane-sensitive map/SLP hashes are essential to kill it.
- Four targeted SSE2 intrinsic-matrix rows (`pmuludq`, `pshufd`, `paddd`,
  `pcmpeqd`) pass the existing ratchet/oracle checker.
- Twelve deterministic differential synthetic fuzz cases pass with GCC/Clang
  at O2, zero failures/skips. The SSE2 matmul gate passes unchanged.
- Complete gzip longest_match assembly is identical before/after under SSE2
  and AVX2: stack references cannot have increased. SSE4.1 and AVX2 map
  assembly is likewise byte-identical. See `unchanged-code.json`.
- Warning-clean fastbuild units: 4 SSE2-filtered, 164 vectorizer and 146
  register-allocator tests pass (314 total); both compiler/linker binaries
  also build with `RUSTFLAGS="-D warnings"`.
- Benchmark harness unit tests and the unchanged strict shard-plan contract
  pass. The new benchmark belongs to hosted shard b and has a bounded
  Callgrind size; it is not excluded from hosted correctness coverage.

## Reproduce

Use verified modern oracle/binutils directories on PATH, then:

```sh
cargo build --profile fastbuild -j2 --bin lccc --bin lccc-ld
python3 tests/regression/check_sse2_i32_product.py
python3 tests/benchmark/run_benchmarks.py --only i32_product_map \
  --compilers lccc,gcc,clang --gcc gcc-16 --clang /path/to/clang23-gcc16.sh \
  --candidate before=/absolute/path/to/unchanged-lccc \
  --cflag=-march=x86-64 --reps 15 --warmup 2 \
  --artifact-dir /absolute/path/to/artifacts --json /absolute/path/to/report.json \
  --strict
```

The full per-arm assembly is reproducible from the gate; its fingerprints
and commands are retained in `feature-matrix.json`. Raw runtime samples,
compiler/binary fingerprints and output checks are retained in the runtime
report. Text transcripts normalize trailing whitespace only;
the structured runtime data and compiler output hashes are unchanged. Compiler Explorer access failed because that host is unavailable in
this environment; the local verified GCC/Clang comparisons are the evidence.
No new local ICX measurement or complete architectural audit is claimed.

## Next tuning work

Investigate the remaining Clang map gap across cache sizes and unroll factors
before choosing another codegen change. Ordered strict-FP reciprocal packing
still requires a dedicated two-lane SSE2 lowering; the existing AVX2 guard
has not been weakened here. Neither backlog item is represented as fixed.
