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

Generated-code performance is evaluated using paired, deterministic execution benchmarks across 39 workloads and algorithms comparing **LCCC**, **GCC 14.2**, and **Clang 19.1.7** under identical flags (`-O2`) and CPU pinning.

All 39 benchmark outputs are verified for **100% byte-for-byte correctness and algorithmic equivalence** against reference compilers.

> **Provenance & interpretation.** The medians below are a fresh re-measure
> from 2026-09-19 (base `56858cbc`, after the worst-15 campaign waves landed:
> two-block unrolling, half-wide SLP, packed FMA contraction, struct-field
> streams, Adler-32 loop epic, and the WO-1..8 red-team audit) of
> `tests/benchmark/run_benchmarks.py` — paired randomized rounds (9 timed reps
> after 2 excluded warm-ups, seed 20260810), CPU pinning, raw-sample
> retention, GCC 14.2 + Clang 19.1.7 as references; JSON evidence:
> `engineering/evidence/godbolt/s59-rank/runtime-main.json`. Read the table as
> a *screening matrix*: the large `constant_recursion` / `ackermann` / `fib`
> ratios reflect LCCC's aggressive recursive-specialization (rec2iter) that
> GCC deliberately does not perform, while the remaining honest losses
> (`expat_xml_scan`, `sha256_transform`, `zstd_count`, `linux_find_bit`)
> carry root-cause analyses and fix backlogs in
> [`engineering/journal/`](engineering/journal/) and
> [`backlog.md`](backlog.md). Against the identical protocol's previous
> checkpoint (2026-09-17, pre-campaign), same-host paired medians improved:
> `mandelbrot` −33% (ratio 1.69 → 1.14), `sha256_transform` −20% (1.50 →
> 1.20), `loop_patterns` −34% (now 0.84× — faster than GCC), `matmul` −27%
> (0.93 → 0.69), `spectral_norm` −11%, `hash_table` −28%.

### Benchmark Results (39 Workloads & Kernels)

| Benchmark | Category / Stress Focus | LCCC Median | GCC Median | Clang Median | LCCC / Best Ref | Verdict |
|---|---|---:|---:|---:|---:|:---:|
| `expat_xml_scan` | Expat UTF-8 XML name-token scan | 45.45 ms | 34.43 ms | 31.27 ms | **1.453×** | PASS |
| `aarch64_select_patterns` | conditional increment, narrow compare, and select pressure | 104.52 ms | 106.01 ms | 76.33 ms | **1.369×** | PASS |
| `linux_find_bit` | Linux sparse find_next_andnot_bit | 13.45 ms | 9.91 ms | 10.79 ms | **1.358×** | PASS |
| `zstd_count` | Zstandard fast unaligned match counting | 9.67 ms | 7.58 ms | 7.94 ms | **1.275×** | PASS |
| `sha256_transform` | SHA-256 64-step block transform / rotate and schedule | 287.61 ms | 239.56 ms | 243.91 ms | **1.201×** | PASS |
| `loop_patterns` | scalar loop transforms | 31.57 ms | 37.59 ms | 27.08 ms | **1.166×** | PASS |
| `chacha20_block` | ChaCha20 20-round ARX block cipher / register pressure | 235.01 ms | 224.67 ms | 202.50 ms | **1.161×** | PASS |
| `linux_rbtree` | Linux kernel intrusive Red-Black tree insertion and search | 15.46 ms | 13.36 ms | 14.39 ms | **1.157×** | PASS |
| `nbody` | N-body simulation / FP structs | 246.22 ms | 213.73 ms | 229.58 ms | **1.152×** | PASS |
| `hash_table` | hash table / pointer chasing | 13.016 s | 12.052 s | 11.371 s | **1.145×** | PASS |
| `mandelbrot` | Mandelbrot / FP branch-heavy inner loop | 1.016 s | 892.59 ms | 955.04 ms | **1.138×** | PASS |
| `ascii_case_fold` | ASCII parser case-fold byte loop / branch selection | 0.81 ms | 0.79 ms | 0.71 ms | **1.136×** | PASS |
| `sieve` | sieve of Eratosthenes / stores | 54.27 ms | 49.02 ms | 48.13 ms | **1.128×** | PASS |
| `histogram` | 256-bin histogram / indexed increment and reduction | 1.45 ms | 1.38 ms | 1.29 ms | **1.126×** | PASS |
| `tls_seg_access` | glibc TLS access shapes | 8.52 ms | 8.94 ms | 7.58 ms | **1.124×** | PASS |
| `sqlite_varint` | SQLite 1–9 byte varint decoder | 22.62 ms | 20.15 ms | 21.75 ms | **1.123×** | PASS |
| `switch_dispatch` | switch lowering / dispatch | 527.28 ms | 476.56 ms | 486.07 ms | **1.106×** | PASS |
| `glibc_strstr` | glibc two-way string search / needle shift table | 4.172 s | 3.887 s | 3.956 s | **1.073×** | PASS |
| `spectral_norm` | spectral norm / dense floating point | 193.39 ms | 180.92 ms | 180.98 ms | **1.069×** | PASS |
| `binary_trees` | binary trees / allocation and recursion | 2.113 s | 1.983 s | 2.002 s | **1.066×** | PASS |
| `strlen_bench` | string operations / byte loops | 217.76 ms | 211.65 ms | 205.29 ms | **1.061×** | PASS |
| `lz4_compress` | LZ4 fast block compression / 4-byte hash matching | 2.24 ms | 2.12 ms | 2.29 ms | **1.057×** | PASS |
| `fannkuch` | Fannkuch-Redux / permutations | 2.383 s | 2.256 s | 2.419 s | **1.056×** | PASS |
| `zlib_ng_adler32` | zlib-ng Adler-32 NMAX accumulator | 38.24 ms | 38.74 ms | 36.22 ms | **1.056×** | PASS |
| `struct_copy` | struct copy / ABI and memory | 17.96 ms | 21.34 ms | 17.09 ms | **1.051×** | PASS |
| `arith_loop` | 32-variable arithmetic loop / register pressure | 94.93 ms | 91.97 ms | 93.60 ms | **1.032×** | PASS |
| `ring_fifo` | masked ring FIFO enqueue/dequeue / dependent loads | 0.62 ms | 0.61 ms | 0.60 ms | **1.025×** | PASS |
| `qsort` | quicksort via libc / branches | 111.71 ms | 110.92 ms | 110.69 ms | **1.009×** | PASS |
| `glibc_memcmp` | glibc aligned-word memcmp path | 5.59 ms | 5.60 ms | 5.55 ms | **1.007×** | PASS |
| `tce_sum` | tail-recursive accumulator / TCE | 0.63 ms | 0.64 ms | 0.63 ms | **1.003×** | PASS |
| `double_reduction` | two independent accumulators per loop | 105.43 ms | 115.50 ms | 111.55 ms | **0.945× (1.06× faster)** | PASS |
| `binary_search` | sorted-table binary search / branch-heavy lookup | 0.65 ms | 0.70 ms | 0.69 ms | **0.931× (1.07× faster)** | PASS |
| `gzip_crc32` | GNU gzip CRC-32 scalar table loop | 134.90 ms | 154.15 ms | 153.29 ms | **0.880× (1.14× faster)** | PASS |
| `bitops` | bit manipulation / integer selection | 197.51 ms | 300.43 ms | 250.80 ms | **0.788× (1.27× faster)** | PASS |
| `matmul` | dense matrix multiply / FP and cache | 3.66 ms | 5.32 ms | 5.09 ms | **0.719× (1.39× faster)** | PASS |
| `libm_round_family` | glibc libm scalar rounding entry points | 201.02 ms | 486.65 ms | 619.05 ms | **0.413× (2.42× faster)** | PASS |
| `ackermann` | Ackermann / deep recursion | 0.84 ms | 60.96 ms | 118.46 ms | **0.014× (72.92× faster)** | PASS |
| `constant_recursion` | constant recursive specialization | 0.84 ms | 61.18 ms | 118.48 ms | **0.014× (72.92× faster)** | PASS |
| `fib` | recursive Fibonacci / recurrence recognition | 0.88 ms | 129.03 ms | 211.25 ms | **0.007× (146.13× faster)** | PASS |

### Summary Statistics

- **LCCC / GCC Geometric Mean Ratio:** **`0.7047`** *(LCCC outperforms GCC in overall geometric mean across the 39-benchmark suite)*
- **LCCC / Fastest Available Reference Geometric Mean Ratio:** **`0.7399`**
- **Correctness Rate:** **39 / 39 (100.0%)** exact matching test verifications.

> **Worst-15 codegen campaign (2026-09-17 → 2026-09-19).** The companion
> static-codegen survey (`scripts/codegen_oracle.py --rank`, `-O2
> -march=x86-64-v3` vs GCC 16.2 / Clang 23.1 / ICX latest / ICC 2021.10 on
> Compiler Explorer) re-ranked all 102 functions: the total instruction gap
> to the best oracle is 2,874 (2,864 at the 2026-09-17 checkpoint —
> structurally flat: the campaign's wins are runtime wins like `mandelbrot`
> −33% and `sha256_transform` −20%, which static screening does not capture);
> the remaining worst-15 kernels (`linux_rbtree`, `csv_field_sum`,
> `zlib_ng_adler32`, `nbody`, `chacha20_block`, `sha256_transform`,
> `moving_stats`, `loop_patterns`, `i686_alu_chains`, `glibc_strstr`,
> `strlen_bench`, `struct_copy`, `matmul`, `expat_xml_scan`,
> `vecreg_new_ops`) are tracked with per-kernel root causes in
> `engineering/evidence/godbolt/s59-rank/before.md`.

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
use `tests/benchmark/run_benchmarks.py` for timing. Verified 2026-09-19
(base `56858cbc`):

| Target (unit measured) | LCCC | GCC 16.2 | Clang 23.1 | ICC | ICX | Best | LCCC vs Best |
|---|---:|---:|---:|---:|---:|---:|---:|
| `glibc_strstr` (whole TU; `two_way_short_needle` now inlined by LCCC, Clang, ICC and ICX — GCC still outlines it at 144 insns) | **197** | 351 | 319 | 211 | 265 | **LCCC** | **1.78× smaller than GCC** |
| `sha256_transform` (whole TU) | 358 | 275 | **267** | 438 | 1145 | Clang | 0.75× (68 spills vs Clang's 35; 70 vector insns vs Clang's 44) |

The oracle's purpose is to *find and rank codegen gaps*, kernel by kernel, not
to declare overall victory: every cell above is reproducible via
`scripts/codegen_oracle.py <source> --all-functions --local ./target/fastbuild/lccc`
(the named-static form inlines under LCCC/Clang/ICC/ICX in these kernels,
hence the whole-TU rows). The sha256 gap halved over the 2026-09-06
checkpoint (444 → 358) via the two-block unroller + half-wide SLP waves; the
remaining gap is spill discipline in the schedule loop and is tracked as a
worst-15 item in `engineering/evidence/godbolt/s59-rank/before.md`.

---

## License

LCCC is dual-licensed under:
- **MIT License** (`LICENSE-MIT`) OR **Apache License 2.0** (`LICENSE-APACHE`) OR **BSD 2-Clause** (`LICENSE-BSD`) for all LCCC original contributions, backend extensions, optimizations, and benchmarks.
- **CC0 1.0 Universal** (Public Domain Dedication) for Anthropic CCC upstream base code.
- Individual workload benchmark kernels retain their upstream open-source licenses (GPLv2+, LGPLv2.1+, BSD, Zlib, Public Domain) as detailed in [`tests/benchmark/WORKLOAD_PROVENANCE.md`](tests/benchmark/WORKLOAD_PROVENANCE.md).
