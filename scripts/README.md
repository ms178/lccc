# scripts/

Developer and research tooling. None of these are needed to build LCCC.

| Script | Purpose |
|---|---|
| `build_lccc_fast.sh` | Incremental `fastbuild` profile (target compiler opt-level 1, no LTO), pinned to Rust/Cargo 1.98.0. Use for RA/codegen loops. |
| `build_lccc_o1_j2.sh` | Ship-quality release: Rust opt-level 1, two Cargo jobs, thin LTO, swap active, Rust/Cargo 1.98.0. |
| `ensure_swap.sh` | Idempotently verify swap or recreate/activate the disposable 8 GiB `/swapfile` after a constrained-harness root reset; both compiler build scripts invoke it. |
| `arena_session_restore.sh` | Rehydrates swap, the persisted Rust/Cargo 1.98.0 toolchain, host multilib/kernel packages, git metadata and fastbuild after an Arena reset. |
| `prepare_kernel_tree.sh` | Recreate Linux 6.18.44 with the linux-cachymod patch series and generated boot headers after a harness wipe. |
| `build_kernel_boot.sh` | Build all x86 real-mode setup objects with LCCC (`-ffunction-sections`), link with `lccc-ld --gc-sections`, preserve non-relocation boot payloads through a build-local `KEEP` script, enforce the authentic 32 KiB ASSERTs, and require flat-image byte identity with available BFD/LLD oracles. |
| `realmode_corpus.sh` | Compare LCCC/GCC executable text per `arch/x86/boot` C file under the real `-m16 -Os` flags. |
| `asmdiff.py` | Whole-object differential against GNU as: section bytes, relocations, and symbols. See `tests/asm-diff/README.md`. |
| `insndiff.py` | Per-instruction encoding differential against GNU as. Reduces an encoding bug to a single mnemonic in one step; supports `--sweep` over register/immediate matrices. A shorter-than-GAS encoding is reported as `BETTER` only after the tool disassembles both forms and confirms they decode identically. |
| `encdiff.py` | Multi-assembler encoding differential: LCCC against GNU as **and** the Clang, GCC, ICC and ICX integrated assemblers over the Compiler Explorer API. Judges LCCC against the *shortest legal encoding any oracle produced*, not against GAS alone. |
| `gen_encoding_sweep.py` | Generate instructions that have MORE THAN ONE legal encoding (accumulator short forms, imm8 sign-extension, redundant REX, VEX2-vs-VEX3, scale-1 index folds, ...). These are the only places an encoding can be improved, and most are invisible to a structural-coverage corpus. |
| `gen_asmdiff_corpus.py` | Generate the `tests/asm-diff/*.casefile` corpora. Every case is validated against GNU as before being written. |
| `godbolt.py` | Compiler Explorer client. Fetches GCC/Clang/ICC/ICX code generation so the Intel compilers can serve as reference oracles without a local Intel toolchain. |
| `codegen_oracle.py` | Batch local/Compiler-Explorer comparison with deterministic artifacts and architecture-aware x86 or AArch64 instruction/load/store/stack-traffic statistics; AArch64 defaults to GCC 16.1, GCC trunk, and Clang 22.1. |
| `aarch64_execute_suite.py` | GCC `gcc.c-torture/execute` differential runner: LCCC assembly is validated by a mandatory GAS 2.47, linked for AArch64, and executed against GCC/optional Clang under QEMU. |
| `x86_gcc_torture.py` | Full native x86-64 GCC `gcc.c-torture/execute` matrix. Splits compilation and linking so every PASS covers LCCC object generation **and standalone `lccc-ld`**, uses GCC only as eligibility/CRT oracle, understands unconditional `dg-options`, enforces timeouts, and writes deterministic JSON/failure logs. |
| `lccc-snapshot.sh` | Harness-wipe-resistant autosave: commit, squashed `ms178-1.patch`, series, tarball, bundle, ledger. |
| `bench_kernels.py` | Separate-TU kernel timing harness. Supports a CPU-pinned, AB/BA-balanced `lccc-alt` compiler arm via `--lccc-alt-env NAME=VALUE`, verifies checksums, and hashes only the timed function. |
| `benchmark_fp_memfold_ab.py` | Builds the stencil5 scalar-FP memory-fold treatment/control and retains randomized CPU-pinned paired samples plus a bootstrap interval. Results are explicitly VM screening, not PMU evidence. |
| `benchmark_reduction_vecreg_ab.py` | Builds register- and stack-accumulator variants of the F32 sum+dot workload and retains randomized CPU-pinned paired samples with a bootstrap interval. |
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
