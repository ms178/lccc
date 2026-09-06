# Current engineering state

This document describes the checked-in compiler at `f7f88be8` (rebased on
mainline base `3e1b71dd`) and is refreshed with each validated change. The source is based on
`3e1b71dd04372b3c4907d402dd1ea8fc1efb897e` and uses Rust 1.98.1, edition 2024,
and `rust-version = "1.98.1"`.

## Architecture

```text
C source
  -> lexer/parser/sema and target ABI lowering
  -> typed SSA IR
  -> verification + optimization pipeline
  -> target code generation / register allocation
  -> textual assembly
  -> in-tree assembler and ELF linker
```

The supported backend families are x86-64, i686, AArch64, and RISC-V 64. The
standalone `lccc-ld` path shares the ELF/linker-common machinery with the
compiler driver. GCC-backed assembler/linker paths remain explicit Cargo
features; the default path is the in-tree implementation.

### Frontend and IR

- The frontend covers the C11/C17 core plus the GNU/C2x features exercised by
the regression and workload suites: inline assembly, atomics, variadics,
`__VA_OPT__`, TLS segment/address-space forms, `_Pragma`, and target-aware
plain-`char` signedness.
- The typed SSA IR has structural verification between passes. Phis, block
predecessors, terminators, value spans, and asm-goto edges are checked.
- `-O0` is deliberately conservative and uses canonical stack homes. `-O1`
uses light optimization; `-O2` enables the full default pipeline; `-O3`, `-Os`,
and `-Oz` add their documented size/unroll policies.

### Optimization and generated code

- Scalar simplification, constant/cast folding, GVN/PRE, backedge PRE,
dead-store elimination, address CSE, copy propagation, tail-call elimination,
loop transforms, and epilogue/block layout passes are active where their
soundness and profitability gates permit.
- Reduction, widening, stencil, elementwise-map, interleave, and plain-copy
vectorizers are profitability-gated. PGO trip/body data may enable a transform;
flat or insufficient profiles do not.
- Register allocation uses segment-aware liveness and linear scan, with a
tier-2 hole-aware graph-coloring path for eligible functions. ABI physical hints,
spill-slot width tracking, and the optional `CCC_VERIFY_REGALLOC=1` verifier are
active. `CCC_NO_TIER2_GRAPH` restores scan-only allocation for bisection.
- MachInst instruction selection is used only within its measured loop-size
policy. Calls, floating stores without a suitable register class, and other
unsupported classes stay on the conservative text path.
- Floating-point contraction defaults to `Off`; `-ffp-contract=fast` and
`-ffast-math` opt into broader contraction. FMA emission is ISA-gated.
- Loop rotation remains opt-in (`CCC_LOOP_ROTATE=1`) until the known
non-canonical miscompile shapes are eliminated.

### Rust 1.98.1 pass

Adoption is limited to changes with a clear maintenance or compiler-host cost:

- `core::fmt::NumBuffer` formats the decimal suffix of fresh compiler-generated
labels with caller-owned storage. This is on the shared `CodegenState` path used
by all backends and preserves exact `.Lprefix_N` spelling.
- `str::strip_circumfix` replaces manual byte-range slicing in AArch64 inline-asm
constraint parsing, AArch64 peephole memory parsing, and balanced-parenthesis
normalization. The surrounding checks remain conservative, so malformed or
non-ASCII input cannot turn into an unchecked slice.
- No algebraic floating-point APIs or semantic-changing language features were
introduced: generated-code correctness has priority over novelty.

## Validation state

The following results are from this pass on the rebased source. The benchmark
report and raw samples are under
`engineering/evidence/benchmarks/2026-09-06-rust198-3e1b71dd/`.

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | pass |
| `cargo check --all-targets` | pass |
| `cargo check --all-targets --all-features` | pass; GCC-linker feature now also compiles the standalone built-in linker exports |
| `cargo test --all-targets` | **2,020 passed**, 0 failed, 6 ignored |
| `tests/regression/run_regression.py --lccc target/release/lccc -j 2` | **670 passed**, 0 failed; 13 honest GCC-incompatible `SKIP-COMPARE`; 683 total |
| `scripts/check_benchmark_outputs.sh` | **156 passed**, 0 failed, 0 skipped across the full program corpus and `-O0`…`-O3` |
| `.github/scripts/ci-codegen-gate.py` | pass for gzip, zlib-ng, Expat, SQLite, glibc memcmp, hash table, and stencil sentinel |
| full benchmark runner | **33/33 correct**; paired nine-round VM screening; geometric mean LCCC/GCC `0.7314` |

The compiler binary used for release measurements came from
`scripts/build_lccc_o1_j2.sh`: Rust 1.98.1, release profile, compiler
opt-level 1, Cargo jobs 2, with warnings denied. `fastbuild` remains the normal
iteration profile and uses the same compiler opt-level policy with incremental
compilation and LTO disabled.

The benchmark VM is hypervisor-backed and has no usable PMU. Wall-clock values
are therefore screening evidence. Assembly metrics, deterministic outputs,
repeated paired timing, and differential correctness are retained; no hardware
counter claim is made.

## Benchmark and workload policy

- CI runs the complete registered corpus in `tests/benchmark/run_benchmarks.py`,
not the former five-program smoke list. The `.github/scripts/ci-bench.py` name is
now a compatibility entry point to that canonical runner.
- The corpus includes synthetic compiler kernels and checked-in extracts from
gzip, zlib-ng, Expat, SQLite, glibc, and Linux. Full-project runners live under
`tests/workloads/` and are separate from the fast CI corpus.
- Paired rounds randomize compiler order, exclude warm-up, retain every sample,
record CPU/PMU/toolchain metadata, compare output to GCC, and report geometric
and arithmetic aggregates. Ratios below 1 mean LCCC was faster.
- `.github/scripts/ci-codegen-gate.py` protects assembly-quality metrics for the
golden workload subset. `scripts/check_benchmark_outputs.sh` is the fast
correctness gate and compares every checked-in benchmark program at four C
optimization levels.

## Guardrails and known limitations

- Do not enable loop rotation globally, MachInst on large loops, unsafe SROA
copy-out, or unvalidated high-pressure register-allocation modes. Use the
kill-switch and negative-results ledger before any default change.
- The built-in compiler/linker supports the tested ELF targets, but full parent
project support still depends on headers, CRT objects, libraries, and target
runtime availability. The complete workload scripts document those external
requirements.
- The VM has no PMU and cannot establish bare-metal microarchitectural wins.
Promising results require repeated timing plus assembly/differential evidence,
then confirmation on suitable hardware.

Authoritative follow-up material:

- [`agent/RULES.md`](agent/RULES.md) — safety and fastbuild/O1/J2 policy
- [`DECISIONS.md`](DECISIONS.md) — negative results and kill switches
- [`agent/BACKLOG.md`](agent/BACKLOG.md) — measured work queue
- [`../docs/architecture.md`](../docs/architecture.md) — subsystem map
- [`../docs/benchmarks.md`](../docs/benchmarks.md) — runner protocol
