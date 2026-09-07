# scripts/

Developer and research tooling. None of these are needed to build LCCC.

| Script | Purpose |
|---|---|
| `rust_toolchain.sh` | Shared selector: resolves the current channel from `rust-toolchain.toml` (currently stable Rust 1.98.1), while honoring an explicit bisection/reproduction override. Its behavior is covered by `tests/regression/check_rust_toolchain_selector.sh`. |
| `build_lccc_fast.sh` | Incremental `fastbuild` profile (target compiler opt-level 1, no LTO), using the manifest-selected current Rust/Cargo release, exactly two Cargo jobs, and warnings denied by default. Use for RA/codegen loops. |
| `build_lccc_o1_j2.sh` | Ship-quality release: Rust opt-level 1, two Cargo jobs, thin LTO, swap active, manifest-selected current Rust/Cargo release, and warnings denied by default. |
| `ensure_swap.sh` | Idempotently verify swap or recreate/activate the disposable 8 GiB `/swapfile` after a constrained-harness root reset; both compiler build scripts invoke it. |
| `arena_session_restore.sh` | Rehydrates swap, the manifest-selected current Rust/Cargo toolchain, host multilib/kernel packages, git metadata and fastbuild after an Arena reset. |
| `prepare_kernel_tree.sh` | Recreate Linux 6.18.44 with the linux-cachymod patch series and generated boot headers after a harness wipe. |
| `build_kernel_boot.sh` | Build all x86 real-mode setup objects with LCCC (`-ffunction-sections`), link with `lccc-ld --gc-sections`, preserve non-relocation boot payloads through a build-local `KEEP` script, enforce the authentic 32 KiB ASSERTs, and require flat-image byte identity with available BFD/LLD oracles. |
| `realmode_corpus.sh` | Compare LCCC/GCC executable text per `arch/x86/boot` C file under the real `-m16 -Os` flags. |
| `asmdiff.py` | Whole-object differential against GNU as: section bytes, relocations, and symbols. See `tests/asm-diff/README.md`. |
| `insndiff.py` | Per-instruction encoding differential against GNU as. Reduces an encoding bug to a single mnemonic in one step; supports `--sweep` over register/immediate matrices. A shorter-than-GAS encoding is reported as `BETTER` only after the tool disassembles both forms and confirms they decode identically. |
| `encdiff.py` | Multi-assembler encoding differential: LCCC against GNU as **and** the Clang, GCC, ICC and ICX integrated assemblers over the Compiler Explorer API. Judges LCCC against the *shortest legal encoding any oracle produced*, not against GAS alone. |
| `gen_encoding_sweep.py` | Generate instructions that have MORE THAN ONE legal encoding (accumulator short forms, imm8 sign-extension, redundant REX, VEX2-vs-VEX3, scale-1 index folds, ...). These are the only places an encoding can be improved, and most are invisible to a structural-coverage corpus. |
| `gen_asmdiff_corpus.py` | Generate the `tests/asm-diff/*.casefile` corpora. Every case is validated against GNU as before being written. |
| `godbolt.py` | Compiler Explorer client. Fetches GCC/Clang/ICC/ICX code generation so the Intel compilers can serve as reference oracles without a local Intel toolchain. |
| `codegen_oracle.py` | Batch local/Compiler-Explorer comparison with deterministic artifacts and architecture-aware x86 or AArch64 instruction/load/store/stack-traffic statistics; AArch64 defaults to GCC 16.1, GCC trunk, and Clang 22.1. `--rank` is the whole-corpus survey absorbed from the deleted `codegen_scoreboard.py`: every function of every source is ranked by instruction-count gap to the best oracle, worst first, with the scoreboard's historical defaults preserved (`-O2 -march=x86-64-v3`, `--min-insns`, `-v`, `--baseline`, the `gaps` JSON schema, and the always-exit-0 survey contract — compiler failures go to stderr; `--oracles ''` is the scoreboard's local-only same-binary A/B mode). Both report modes share ONE metric implementation; the old oracle-vs-scoreboard divergence (non-VEX packed SSE mnemonics undercounted) is fixed, and remote compiles share the scoreboard's `att-v2` digest cache. |
| `check_codegen_refactor_identity.py` | Byte-for-byte assembly gate for pure codegen refactors. It compiles a corpus with before/after LCCC binaries under a scrubbed default `CCC_*`/`LCCC_*` environment and honours adjacent `.flags` files; the report separately canonicalizes only LCCC PGO-generation process-ID suffixes, which are intentionally nondeterministic. |
| `reproduce_historical_peephole_utf8.py` | Extracts and executes the exact historical UTF-8-corrupting helper. It resolves the current Rust channel from `rust-toolchain.toml` even though the intentionally historical snippet is compiled with its original Rust-2021 semantics. |
| `aarch64_execute_suite.py` | GCC `gcc.c-torture/execute` differential runner: LCCC assembly is validated by a mandatory GAS 2.47, linked for AArch64, and executed against GCC/optional Clang under QEMU. |
| `x86_gcc_torture.py` | Full native x86-64 GCC `gcc.c-torture/execute` matrix. Splits compilation and linking so every PASS covers LCCC object generation **and standalone `lccc-ld`**, uses GCC only as eligibility/CRT oracle, understands unconditional `dg-options`, enforces timeouts, and writes deterministic JSON/failure logs. `--from-list FILE` re-runs only the tests named in a previous report (one JSON `test` value per line, blank/`#` lines ignored, unknown names warn+skip, intersects positional tests and `--filter`); the partial-run JSON gains a `from_list` field so evidence tooling can see the filtering. |
| `lccc-snapshot.sh` | Harness-wipe-resistant autosave: commits then atomically publishes a verified squashed `ms178-1.patch`, series, tarball, bundle, and ledger with SHA-256 records. Set `LCCC_BASE_REF` when re-anchoring a new session after an upstream rebase. |
| `bench_kernels.py` | Separate-TU kernel timing harness. Supports a CPU-pinned, AB/BA-balanced `lccc-alt` compiler arm via `--lccc-alt-env NAME=VALUE`, verifies checksums, and hashes only the timed function. |
| `fuzz_diff.py` | Unified differential-testing harness and single entry point for every differential engine (`synthetic`, `csmith`, `yarpgen`, `stress_suite` + the nine absorbed `tests/fuzz` engines `differential`, `phi_cfg`, `intcmp_thread`, `m32`, `regparm`, `slot_rmw`, `alias_m32`, `alu_torture`, `aarch64`). Compiles every generated program with LCCC **and all selected references** across the requested `-O` levels, compares stdout/stderr/exit status, and saves miscompile reproducers. The `stress_suite` engine sweeps `--count` seeds per generator and evaluates every case under the cartesian product of repeatable `--config-env KEY=VALUE` axes; a case where the reference AND lccc die by the same signal is a GEN-BUG failure (generator bug, never a silent skip). The generator-config engines import the historical `tests/fuzz` generators (historical flags, seeds, and gcc oracle preserved; `Oz` maps to `-Os` on references); the standalone-pipeline engines (nostdlib int80 or cross-compilation) are forwarded to their `tests/fuzz` script and adopt its exit code, SKIPping cleanly when the host lacks ELF32/int80 or the cross toolchain (int80-blocked hosts fall back to compile-only validation). `--check-engines` is the CI wiring guard against stub engines. |
| `x86_gcc_torture_i686.py` | Compatibility wrapper: forwards to `scripts/x86_gcc_torture.py --arch=i686` (all flags, including `--from-list`, pass through unchanged). |
| `csmith_diff.py`, `yarpgen_diff.py` | Compatibility wrappers: translate the historical flag spellings (`--ccc`, `--clang`, `--gcc`, `--jobs`, `--tests`, `--seed-start`, ...) and forward to `scripts/fuzz_diff.py --engine csmith|yarpgen`. |
| `run_gep_chain_stress.sh` | Compatibility wrapper (bash): forwards to `scripts/fuzz_diff.py --engine stress_suite` over seeds `first..last` (`LCCC_BIN`→`--lccc`, `GCC_BIN`→`--refs`, trailing args→`--opts`), driving `gen_gep_chain_stress.py` for folded-address (chained-GEP) liveness. A seed where GCC and lccc both die by the same signal is reported GEN-BUG and fails the run. |
| `run_slot_stress.sh` | Compatibility wrapper (bash): forwards to `scripts/fuzz_diff.py --engine stress_suite` over seeds `first..last` with the four-way CCC layout matrix declared via `--config-env CCC_NO_TIER2_GRAPH=1`/`CCC_NO_SMALL_SLOTS=1` (default | no-tier2 | no-small-slots | both), driving `gen_slot_stress.py` for stack-slot allocation. |
| `perf_ab.py` | Unified A/B screening engine, default metric = runtime: interleaved AB/BA rounds, empty-process floor exclusion, geomean verdicts, JSON/Markdown. `--metric insns` is the static gate absorbed from the deleted `peephole_ab.py`: every program compiles to assembly in both arms for per-function and total instruction counts AND to binaries whose (exit status, stdout) pairs must match exactly — a pass that changes observable behaviour is a miscompile, exit 1. Presets carry the B-side environment (`--preset peephole_skip` = peephole_ab's default `CCC_PEEPHOLE_SKIP` pass list; `--preset peephole` = whole optimizer off), `--env KEY=VALUE`/`--skip PASS` drive custom arms, `--no-run` disables the execution check, and positional sources extend the corpus. Both arms are scrubbed of ambient `CCC_*`/`LCCC_*` variables. |
| `benchmark_fp_memfold_ab.py` | Thin wrapper forwarding to `scripts/perf_ab.py --preset fp_memfold` (interleaved AB/BA screening with empty-process floor exclusion; see `perf_ab.py`). |
| `benchmark_reduction_vecreg_ab.py` | Thin wrapper forwarding to `scripts/perf_ab.py --preset reduction_vecreg`. |
| `benchmark_vecreg_new_ops_ab.py` | Thin wrapper forwarding to `scripts/perf_ab.py --preset vecreg_ops`. |
| `benchmark_vector_remainder_ab.py` | Thin wrapper forwarding to `scripts/perf_ab.py --preset vector_remainder`. |
| `bench_iv_widen_ab.sh` | Compatibility wrapper (bash): forwards to `scripts/perf_ab.py --preset iv_widen` (same `CCC_NO_IV_WIDEN=1` kill switch, interleaved AB/BA rounds with output equality). Historical interface preserved: positional kernels and `KERNELS`→`--only` (default: the 8-kernel set), `N`→`--reps`, `LCCC_BIN`→`--compiler-a`; `CPU` is accepted but legacy-ignored (perf_ab interleaves the arms instead of pinning separate blocks). |
| `ra_quality_census.py` | Register-allocation quality census: classifies every instruction of every function into RA buckets (insns/rrmov/stkref/push/acc) for LCCC vs GCC (Clang optional), with `.type`-based function boundaries. `--ab-env KEY=VALUE` (repeatable) runs the same-binary A/B experiment absorbed from `ra_ab_census.py`, keeping its exit-1 hot-code regression gate (kernel spill traffic, instruction count tiebreak; `--no-gate` disables); `--kernels` censuses the curated kernel_corpus hot-function map absorbed from `kernel_count.py`, with `--filter` substring narrowing. |
| `ra_ab_census.py` | Compatibility wrapper: forwards to `scripts/ra_quality_census.py --ab-env` (`--env KEY=VALUE` translated; `--opt`/`--cflag`/`--top`/`--json`/positional files pass through). The historical tool never consulted GCC/Clang, so the wrapper turns those optional columns off; the JSON export keeps the historical top-level keys. |
| `kernel_count.py` | Compatibility wrapper: forwards to `scripts/ra_quality_census.py --kernels` — per-function instruction counts of the kernel corpus, LCCC vs GCC, at -O2. Historical positional substring filters become `--filter` values; the `LCCC`/`GCC`/`CFLAGS` environment interface is unchanged; the unified run reports the full RA bucket set plus the LCCC-wins summary. |
| `symstr_migrate.py` | Span-driven `String` → `SymStr` migration helper. Its diagnostic build resolves the manifest-selected Rust channel, locates the persisted Cargo proxy, and denies warnings unless explicitly opted out. |
| `gen_lcccsimd.py`, `strip_scalar_dups.py` | SIMD intrinsic header generation helpers. |

## Why more than one oracle

`insndiff.py` and `asmdiff.py` compare against a single local GNU as. That is
the right tool for *correctness* -- "do we agree with binutils" -- but it
cannot answer *quality*. When several encodings of an instruction are legal,
matching GAS only proves we match GAS; it says nothing about whether a shorter
legal encoding exists.

`encdiff.py` exists for that second question. It assembles the same
instruction with every reachable oracle and scores LCCC against the best
result any of them produced. That is how the scale-1 index fold was found:
for `mov -1(,%rdi,1),%rcx`, GAS, clang, gcc and icx all emit 8 bytes while ICC
emits 4, because ICC folds a scale-1 index into the base slot and drops the
SIB byte. Comparing only against GAS would have shown a clean pass forever.

`encdiff.py` batches many instructions into one Compiler Explorer request
(each probe fenced by `ud2` inside its own naked function) and queries the
oracles concurrently, so a 1,900-instruction sweep against five assemblers
takes ~30 s rather than ~2 h.

A shorter encoding is never accepted on size alone. Both `encdiff.py` and
`insndiff.py` disassemble the two candidates and require them to decode to the
same instruction; `asmdiff.py` does the same for a whole object when a case is
marked `betterok`. This is not ceremony -- the first version of the fold was
4 bytes shorter *and wrong* for `%r12`/`%r13`, because those register numbers
mean something different in the base slot than in the index slot, and only the
round-trip check caught it.

Two verdicts record deliberate refusals, so they are not mistaken for work
left undone:

* `DECLINED-FP` -- clang and icx exchange the sources of `vaddps`/`vmulps`
  (and the pd/scalar forms) to reach the 2-byte VEX prefix. x86 FP add and
  multiply are not bit-commutative: with two NaN inputs the result carries
  SRC1's payload. Measured here, `vaddps` over (0x7fc00001, 0x7fc00002) gives
  0x7fc00001 one way and 0x7fc00002 the other, so the byte is not taken.
  Bitwise FP ops (`vandps`/`vorps`/`vxorps`) have no NaN rule and were
  measured bit-identical under exchange, so those ARE shortened.
* `DECLINED-WRONG` -- ICC encodes `xchg %eax, %eax` as the one-byte `0x90`.
  That is not equivalent: the xchg is a 32-bit register write and zeroes the
  upper half of RAX, while NOP does nothing. Measured from
  RAX=0x1122334455667788, `87 c0` leaves 0x0000000055667788.

## Oracles

The differential tools need a reference assembler:

```bash
export LCCC_GAS=/path/to/as
export LCCC_OBJCOPY=/path/to/objcopy
export LCCC_OBJDUMP=/path/to/objdump   # used to verify shorter encodings
```

`encdiff.py` additionally needs outbound network access for the remote
oracles. Run it with `--offline` to restrict it to the local assembler; that
is what CI uses when it has no network. Remote answers are cached under
`.godbolt-cache/`, so a tuning loop hits the network once per instruction.

Any recent binutils works. To build a known-good one from source:

```bash
curl -O https://ftp.gnu.org/gnu/binutils/binutils-2.47.tar.xz
tar xf binutils-2.47.tar.xz && mkdir bu-build && cd bu-build
../binutils-2.47/configure --prefix=$PWD/../bu-247 \
  --disable-gdb --disable-gdbserver --disable-sim --disable-readline \
  --disable-libdecnumber --disable-nls --disable-werror \
  --disable-gprofng --disable-gprof --disable-plugins --with-system-zlib
make -j"$(nproc)" MAKEINFO=true all-gas all-binutils all-ld
```

`godbolt.py` needs only network access:

```bash
scripts/godbolt.py list --filter icx
scripts/godbolt.py compare kernel.c --flags "-O3 -march=x86-64-v3" \
    --local ./target/fastbuild/lccc

# AArch64 structural comparison (ARM64 GCC 16.1 on Compiler Explorer):
python3 scripts/codegen_oracle.py test.c --function hot_loop \
    --arch aarch64 --local target/fastbuild/lccc-arm \
    --local-flags '-O2' --flags '-O2' \
    --artifact-dir /tmp/aarch64-oracle

# Native x86-64: compile with LCCC, link through standalone lccc-ld, execute.
# The default is the full -O0/-O1/-O2/-O3/-Os matrix; use -j2 on constrained VMs.
python3 scripts/x86_gcc_torture.py \
    --suite /path/to/gcc/gcc/testsuite/gcc.c-torture/execute \
    --lccc target/fastbuild/lccc --lccc-ld target/fastbuild/lccc-ld \
    -j2 --json /tmp/x86-torture.json --failure-log /tmp/x86-torture.log

# AArch64 differential execution. The harness rejects every assembler whose
# --version is not 2.47.
python3 scripts/aarch64_execute_suite.py \
    /path/to/gcc/gcc/testsuite/gcc.c-torture/execute \
    --lccc target/fastbuild/lccc-arm \
    --assembler /path/to/binutils-2.47/aarch64-linux-gnu-as \
    --jobs 2 --flags=-O2 --json /tmp/aarch64-torture.json
```
