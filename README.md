# LCCC

## A performance-focused C compiler written in Rust

LCCC is a self-contained C compiler, assembler, and ELF linker for generated-code research. It lowers C through a typed SSA IR into target-specific code generation and optimization, with an emphasis on useful machine code for real workloads rather than synthetic scores.

Current targets:

- x86-64 Linux
- i686 Linux and 16-bit real-mode output paths
- AArch64 ELF
- RISC-V 64 ELF

The default toolchain is entirely in-tree. GCC-backed assembler and linker fallbacks are available as explicit Cargo features when a host or target requires them.

## Quick start

LCCC tracks stable Rust and currently requires Rust **1.98.1 or newer**.

```sh
# Fast local compiler build: Rust -O1, incremental, two Cargo jobs.
./scripts/build_lccc_fast.sh

# Research/release build: Rust -O1, release profile, two Cargo jobs.
./scripts/build_lccc_o1_j2.sh

# Compile and run a C program.
target/fastbuild/lccc -O2 hello.c -o hello
./hello
```

The build helpers provision swap when needed on constrained hosts. The project policy is deliberate: LCCC self-builds use `-O1 -j2`; `fastbuild` is for iteration, while the release helper is for measured compiler binaries.

## What is implemented

The active compiler stack includes:

- C11/C17-oriented frontend and GNU/C2x extensions, including `__VA_OPT__`, inline assembly, atomics, variadic calls, TLS address spaces, and target-aware plain-`char` signedness.
- Typed SSA IR with verification gates, scalar simplification, GVN/PRE, dead-store elimination, address CSE, loop transforms, tail-call elimination, PGO, and several profitability-gated vectorizers.
- Segment-aware linear-scan register allocation with a production tier-2 graph-coloring path, ABI hints, spill-slot width tracking, and an optional verifier.
- Native assembly and ELF linking for all four target families, including i686 multilib discovery, AArch64 relocations/atomics, RISC-V ABI details, archives, shared objects, linker scripts, TLS, PLT/GOT, and build IDs.
- Generated-code diagnostics and kill switches for controlled A/B experiments. See [`engineering/agent/RULES.md`](engineering/agent/RULES.md) before changing a default.

Rust 1.98.1 adoption is intentionally selective. Compiler-generated labels now use the standard integer `NumBuffer` path rather than general formatting, and assembler/codegen delimiter parsing uses `str::strip_circumfix`. Both changes preserve emitted spelling while reducing hot code-generation formatting work or removing manual byte slicing; speculative syntax churn is avoided.

## Validation and performance evidence

The repository has two complementary corpora:

1. **683 regression and benchmark-program tests** for compile/run behavior and GCC differential output. The current rebased validation passed **670**, with **13 honest `SKIP-COMPARE` cases** where GCC cannot compile the test; no cases failed.
2. **33 deterministic benchmark programs** spanning compiler kernels and extracts from gzip, zlib-ng, Expat, SQLite, glibc, and Linux. The runner uses randomized paired compiler order, an excluded warm-up, repeated wall-clock samples, output comparison, compiler/version capture, and retained raw JSON.

Fresh full-corpus screening on 2026-09-06, using commit `f7f88be8` and nine paired rounds, produced:

- **33/33** LCCC/GCC outputs correct
- geometric mean LCCC/GCC ratio **0.7314**; arithmetic mean **0.9530**
- best row: `fib`, **0.016×** GCC; slowest row: `expat_xml_scan`, **1.407×** GCC
- VM classification: CPU-pinned wall-clock screening; **no usable PMU** was available, so these figures are not hardware-counter claims

Selected rows (ratio below 1 is faster):

| Workload | LCCC/GCC | What it exercises |
|---|---:|---|
| `fib` | 0.016 | recursive specialization |
| `constant_recursion` | 0.033 | constant recursion folding |
| `libm_round_family` | 0.419 | scalar floating-point library shapes |
| `gzip_crc32` | 0.882 | checksum table loop |
| `zlib_ng_adler32` | 1.007 | wide integer accumulation |
| `sqlite_varint` | 1.292 | branch-heavy integer decoding |
| `expat_xml_scan` | 1.407 | UTF-8 parser and hash path |

The complete per-round evidence and exact reproduction command are in [`engineering/evidence/benchmarks/2026-09-06-rust198-3e1b71dd/`](engineering/evidence/benchmarks/2026-09-06-rust198-3e1b71dd/). The canonical runner is [`tests/benchmark/run_benchmarks.py`](tests/benchmark/run_benchmarks.py); CI runs the complete registered corpus through `.github/scripts/ci-bench.py`, not a five-program smoke set.

For generated-code guardrails, `.github/scripts/ci-codegen-gate.py` checks assembly instruction, stack-memory, move, callee-save, and vector metrics for the golden workload set. Use `scripts/check_benchmark_outputs.sh` for a fast correctness-only sweep.

## Development checks

```sh
export PATH="$HOME/.cargo/bin:$PATH"
cargo fmt --all -- --check
cargo check --all-targets
cargo check --all-targets --all-features
cargo test --all-targets
python3 tests/regression/run_regression.py --lccc target/release/lccc -j 2
python3 .github/scripts/ci-codegen-gate.py --lccc target/release/lccc --summary
LCCC_BIN=target/release/lccc ./scripts/check_benchmark_outputs.sh
```

Architecture and pass documentation:

- [`docs/getting-started.md`](docs/getting-started.md)
- [`docs/architecture.md`](docs/architecture.md)
- [`docs/optimization-passes.md`](docs/optimization-passes.md)
- [`docs/benchmarks.md`](docs/benchmarks.md)
- [`engineering/STATE.md`](engineering/STATE.md)
- [`engineering/README.md`](engineering/README.md)
- [`backlog.md`](backlog.md)

## Licensing

LCCC uses a dual-license boundary:

- LCCC-specific contributions may be used under MIT, Apache-2.0, or BSD-2-Clause.
- CCC-derived frontend, IR, optimizer, backend, assembler, and linker code is CC0 1.0 Universal.
- Workload-derived benchmark files retain their upstream licenses and provenance.

See [`LICENSING.md`](LICENSING.md), [`LICENSE-MIT`](LICENSE-MIT), [`LICENSE-APACHE`](LICENSE-APACHE), and [`LICENSE-BSD`](LICENSE-BSD).

Project repository: <https://github.com/ms178/lccc>
