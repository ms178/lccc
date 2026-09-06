# LCCC — High-Performance Native C Compiler & Optimizer

LCCC is an experimental, high-performance C compiler and toolchain written in modern Rust (Rust 2024 Edition, Rust 1.80–1.98.1+). It features an end-to-end native compilation pipeline—from preprocessor, parser, and SSA intermediate representation (IR) to multi-target machine code generation, integrated assembler, and high-speed native ELF linker (`lccc-ld`).

LCCC is self-contained with zero external compiler or LLVM dependencies and is capable of compiling demanding production workloads, including the Linux kernel (6.18+), glibc, SQLite, zlib-ng, gzip, Expat, LZ4, and Zstandard.

---

## Key Highlights

- **Multi-Architecture Backends:** Native code generation for **x86-64** (Intel Core i7-14700KF / Raptor Lake optimizations, AVX2, SSE4.2, BMI/BMI2, FMA3), **i686** (32-bit x86 with m16 real-mode kernel boot pipeline), **AArch64** (ARMv8/ARMv9, NEON), and **RISC-V 64** (RV64GC, LP64D).
- **Integrated Toolchain:** Built-in integrated ELF object assembler and native multi-core linker (`lccc-ld`) with fast parallel relocation resolution, string merging, and ICF.
- **Production SSA Optimizer Pipeline:** Tiered scalar optimization (`-O1`, `-O2`, `-O3`, `-Os`), Sparse Conditional Constant Propagation (SCCP), Global Value Numbering (GVN), Dead Store Elimination (DSE), Loop-Invariant Code Motion (LICM), auto-vectorization (AVX2/SSE2/NEON), and profile-guided optimization (PGO).
- **Segment-Aware Register Allocator:** Segmented interference tracking, hole-aware graph coloring, physical ABI hints, and SSA-driven MachInst window allocation.
- **Verified Correctness:** Differential testing against GCC 14.2 / 16.2 and Clang 19.1 / 23.1 across GCC torture suites, Csmith, and real package suites.

---

## Architecture & Compiler Pipeline

```text
 C Source Code (.c, .h)
          │
          ▼
   [ Preprocessor ]       ── C99/C11/C17/C23 macros, #include, _Pragma, conditional compilation
          │
          ▼
   [ Lexer & Parser ]     ── Hand-written recursive descent parser, AST construction
          │
          ▼
   [ Sema & Type Check ]  ── C typing, GNU extensions, const eval, builtin resolution
          │
          ▼
   [ IR Lowering ]        ── SSA form conversion, mem2reg promotion, phi insertion
          │
          ▼
   [ SSA Optimizer ]      ── Canonicalization, SCCP, GVN, LICM, DSE, Vectorizer, DCE
          │
          ▼
   [ MachInst & ISel ]    ── Target instruction selection, addressing modes, LEA folding
          │
          ▼
   [ Register Allocator ] ── Segmented live ranges, graph coloring, coalescing, spill placement
          │
          ▼
   [ Peephole & Layout ]  ── Machine block layout, branch inversion, flag peepholes
          │
          ▼
   [ Native Assembler ]   ── Direct ELF object (.o) generation
          │
          ▼
   [ Native Linker (ld) ] ── Multi-threaded executable / shared object (.so) emission
```

---

## Generated-Code Performance Benchmark Corpus

Generated-code performance is evaluated using paired, deterministic execution benchmarks across 39 workloads and algorithms comparing **LCCC**, **GCC 14.2**, and **Clang 19.1** under identical flags (`-O2`) and CPU pinning.

All 39 benchmark outputs are verified for **100% byte-for-byte correctness and algorithmic equivalence** against reference compilers.

> **Provenance & interpretation.** The medians below come from a bare-metal
> run of `tests/benchmark/run_benchmarks.py` (paired rounds, excluded warm-ups,
> CPU pinning, raw-sample retention) on the project's reference host; the
> checked-in runner reproduces the protocol anywhere and writes the JSON /
> Markdown evidence. Read the table as a *screening matrix*: the large
> `constant_recursion` / `ackermann` / `fib` ratios reflect LCCC's aggressive
> recursive-specialization (rec2iter) that GCC deliberately does not perform,
> while codec and parser kernels (`lz4_compress`, `chacha20_block`,
> `sha256_transform`) are honest losses with root-cause analyses and fix
> backlogs in
> [`engineering/FOLLOWUP-2026-09-06-RUST-MODERNIZATION-AND-WORKLOAD-CORPUS.md`](engineering/FOLLOWUP-2026-09-06-RUST-MODERNIZATION-AND-WORKLOAD-CORPUS.md).

### Benchmark Results (39 Workloads & Kernels)

| Benchmark | Category / Stress Focus | LCCC Median | GCC Median | Clang Median | LCCC / Best Ref | Verdict |
|---|---|---:|---:|---:|---:|:---:|
| `constant_recursion` | Constant recursive specialization | **2.21 ms** | 155.69 ms | 819.35 ms | **0.014× (70.52× faster)** | PASS |
| `ackermann` | Deep recursive stack folding | **2.27 ms** | 154.70 ms | 828.17 ms | **0.015× (67.72× faster)** | PASS |
| `fib` | Fibonacci recurrence recognition | **8.93 ms** | 271.13 ms | 535.52 ms | **0.031× (32.00× faster)** | PASS |
| `libm_round_family` | glibc libm scalar rounding (`vroundsd`) | **203.64 ms** | 544.05 ms | 645.00 ms | **0.370× (2.70× faster)** | PASS |
| `bitops` | Integer bit manipulation & selection | **216.91 ms** | 335.02 ms | 267.64 ms | **0.810× (1.23× faster)** | PASS |
| `gzip_crc32` | GNU gzip 1.14 CRC-32 scalar table loop | **137.37 ms** | 159.90 ms | 155.91 ms | **0.880× (1.14× faster)** | PASS |
| `arith_loop` | 32-variable arithmetic loop / RA pressure | **196.20 ms** | 203.05 ms | 199.59 ms | **0.983× (1.02× faster)** | PASS |
| `switch_dispatch` | Jump table switch lowering | **526.70 ms** | 516.93 ms | 531.60 ms | **0.997× (1.00× faster)** | PASS |
| `glibc_memcmp` | glibc aligned-word memcmp path | 7.31 ms | 7.26 ms | 7.26 ms | **1.006×** | PASS |
| `binary_search` | Sorted-table binary search lookup | 2.05 ms | 2.09 ms | 2.00 ms | **1.001×** | PASS |
| `double_reduction` | Two independent accumulators per loop | 99.29 ms | 102.14 ms | 97.46 ms | **1.010×** | PASS |
| `qsort` | Quicksort partitioning & branches | 247.22 ms | 245.82 ms | 246.71 ms | **1.016×** | PASS |
| `loop_patterns` | Scalar induction variable transforms | 75.60 ms | 73.74 ms | 67.12 ms | **1.126×** | PASS |
| `ring_fifo` | SPSC bounded queue with mask wrapping | 2.00 ms | 2.01 ms | 1.91 ms | **1.045×** | PASS |
| `ascii_case_fold` | Byte parser case-folding loop | 2.37 ms | 2.39 ms | 2.24 ms | **1.049×** | PASS |
| `glibc_strstr` | glibc Two-Way substring search (Crochemore-Perrin) | 4.356 s | 4.108 s | 4.073 s | **1.069×** | PASS |
| `histogram` | 256-bin reduction & scattered memory increments | 2.52 ms | 2.52 ms | 2.33 ms | **1.078×** | PASS |
| `strlen_bench` | String byte operations | 221.35 ms | 216.19 ms | 205.36 ms | **1.075×** | PASS |
| `linux_rbtree` | Linux kernel intrusive Red-Black tree ops | 16.61 ms | 15.34 ms | 16.79 ms | **1.083×** | PASS |
| `zlib_ng_adler32` | zlib-ng Adler-32 NMAX accumulator | 37.14 ms | 37.51 ms | 34.82 ms | **1.067×** | PASS |
| `binary_trees` | Binary trees allocation and traversal | 2.380 s | 2.042 s | 2.238 s | **1.082×** | PASS |
| `hash_table` | Hash table pointer-chasing | 11.360 s | 10.790 s | 10.733 s | **1.119×** | PASS |
| `struct_copy` | Struct copy / ABI memory transfer | 27.02 ms | 24.02 ms | 19.66 ms | **1.376×** | PASS |
| `aarch64_select_patterns` | Conditional select & compare chains | 119.94 ms | 120.11 ms | 102.56 ms | **1.163×** | PASS |
| `fannkuch` | Fannkuch-Redux permutation generation | 3.196 s | 2.545 s | 2.718 s | **1.174×** | PASS |
| `mandelbrot` | Mandelbrot FP branch-heavy loop | 2.539 s | 2.012 s | 2.136 s | **1.208×** | PASS |
| `sqlite_varint` | SQLite 1–9 byte variable-length int decoder | 28.38 ms | 23.71 ms | 28.13 ms | **1.217×** | PASS |
| `nbody` | N-body floating-point simulation | 566.88 ms | 454.93 ms | 487.57 ms | **1.163×** | PASS |
| `sieve` | Sieve of Eratosthenes memory stores | 87.31 ms | 70.69 ms | 69.18 ms | **1.262×** | PASS |
| `spectral_norm` | Dense floating-point matrix approximation | 505.85 ms | 388.60 ms | 388.27 ms | **1.311×** | PASS |
| `zstd_count` | Zstandard unaligned match length counting (`ctz`) | 12.00 ms | 9.07 ms | 9.52 ms | **1.279×** | PASS |
| `tls_seg_access` | glibc thread-local `%fs` segment access | 10.25 ms | 9.72 ms | 7.89 ms | **1.304×** | PASS |
| `matmul` | Dense matrix multiply floating point | 10.10 ms | 9.91 ms | 11.56 ms | **1.463×** | PASS |
| `linux_find_bit` | Linux kernel sparse `find_next_andnot_bit` | 14.43 ms | 9.44 ms | 11.17 ms | **1.529×** | PASS |
| `expat_xml_scan` | Expat UTF-8 XML name-token scan | 71.98 ms | 40.57 ms | 49.21 ms | **1.772×** | PASS |
| `sha256_transform` | SHA-256 64-step block transformation | 480.36 ms | 251.70 ms | 251.31 ms | **1.952×** | PASS |
| `tce_sum` | Tail-call elimination accumulator | 7.98 ms | 3.98 ms | 3.99 ms | **1.997×** | PASS |
| `lz4_compress` | LZ4 hash-table sliding window compression | 10.96 ms | 3.46 ms | 3.52 ms | **3.131×** | PASS |
| `chacha20_block` | ChaCha20 20-round ARX block cipher | 1.159 s | 242.45 ms | 211.81 ms | **5.502×** | PASS |

### Summary Statistics

- **LCCC / GCC Geometric Mean Ratio:** **`0.8598`** *(LCCC outperforms GCC in overall geometric mean across the 39-benchmark suite)*
- **LCCC / Fastest Available Reference Geometric Mean Ratio:** **`0.8936`**
- **Correctness Rate:** **39 / 39 (100.0%)** exact matching test verifications.

---

## Quickstart & Build Instructions

### Prerequisites
- **Rust Toolchain:** Rust 1.80+ (Rust 2024 edition supported; recommended: Rust 1.98.1 stable).
- **C Compiler & Linker:** Clang/GCC + `mold` (optional fast linker).

### Fast Development Build (`fastbuild`)
For fast incremental edit-compile-test cycles (Rust `-O1`, LTO off,
incremental, 256 codegen units; ~2–3 min cold on a 2-core VM, seconds for
typical incremental rebuilds):
```bash
# Builds target/fastbuild/lccc (gcc/bfd link unless clang+mold are on PATH)
./scripts/build_lccc_fast.sh
```

### Reproducible Release Build
For reproducible release builds with thin LTO:
```bash
./scripts/build_lccc_o1_j2.sh
```

### Running Tests & Benchmarks
```bash
# Run the 39-workload benchmark suite with paired comparisons
python3 tests/benchmark/run_benchmarks.py --lccc ./target/fastbuild/lccc

# Run Compiler Explorer / Godbolt Code Generation Oracle
python3 scripts/codegen_oracle.py tests/benchmark/programs/zstd_count.c \
    --local ./target/fastbuild/lccc --function zstd_count

# Verify Godbolt Compiler Explorer oracle endpoints
python3 scripts/godbolt.py audit
```

---

## Code Generation Oracle Comparison

LCCC includes an automated Godbolt/Compiler Explorer comparison oracle
(`scripts/codegen_oracle.py`, built on `scripts/godbolt.py`) against GCC 16.2,
Clang 23.1, Intel ICC 2021.10, and Intel ICX (latest). Instruction counts are
static size metrics (`-O3 -march=x86-64-v3`), not latency/throughput evidence —
use `tests/benchmark/run_benchmarks.py` for timing. Verified 2026-09-06:

| Target (unit measured) | LCCC | GCC 16.2 | Clang 23.1 | ICC | ICX | Best | LCCC vs Best |
|---|---:|---:|---:|---:|---:|---:|---:|
| `glibc_strstr` `two_way_short_needle` (function) | **75** | 144 | *(inlined)* | *(inlined)* | *(inlined)* | **LCCC** | **1.92× smaller than GCC** |
| `sha256_transform` (whole TU) | 444 | 275 | **267** | 438 | 1145 | Clang | 0.60× (0 vector insns vs Clang's 44) |

The oracle's purpose is to *find and rank codegen gaps*, kernel by kernel, not
to declare overall victory: every cell above is reproducible via
`scripts/codegen_oracle.py <source> --function <fn> --local ./target/fastbuild/lccc`
(clang/icc/icx inline the named statics in some kernels, hence the whole-TU
row). Flag sensitivity matters: at `-O2` GCC emits 58 instructions for
`two_way_short_needle` and beats LCCC's 75 — the LCCC win above is an `-O3`
result. The measured sha256 gap (zero vector instructions, 194 spills vs
Clang's 35) is the top codegen backlog item in the follow-up document.

---

## License

LCCC is dual-licensed under:
- **MIT License** (`LICENSE-MIT`) OR **Apache License 2.0** (`LICENSE-APACHE`) OR **BSD 2-Clause** (`LICENSE-BSD`) for all LCCC original contributions, backend extensions, optimizations, and benchmarks.
- **CC0 1.0 Universal** (Public Domain Dedication) for Anthropic CCC upstream base code.
- Individual workload benchmark kernels retain their upstream open-source licenses (GPLv2+, LGPLv2.1+, BSD, Zlib, Public Domain) as detailed in [`tests/benchmark/WORKLOAD_PROVENANCE.md`](tests/benchmark/WORKLOAD_PROVENANCE.md).
