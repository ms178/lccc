# scripts/

Developer and research tooling. None of these are needed to build LCCC.

| Script | Purpose |
|---|---|
| `build_lccc_o1_j2.sh` | Build the compiler under the project policy: Rust opt-level 1, exactly two Cargo jobs, swap active. |
| `asmdiff.py` | Whole-object differential against GNU as: section bytes, relocations, and symbols. See `tests/asm-diff/README.md`. |
| `insndiff.py` | Per-instruction encoding differential against GNU as. Reduces an encoding bug to a single mnemonic in one step; supports `--sweep` over register/immediate matrices. |
| `gen_asmdiff_corpus.py` | Generate the `tests/asm-diff/*.casefile` corpora. Every case is validated against GNU as before being written. |
| `godbolt.py` | Compiler Explorer client. Fetches GCC/Clang/ICC/ICX code generation so the Intel compilers can serve as reference oracles without a local Intel toolchain. |
| `gen_lcccsimd.py`, `strip_scalar_dups.py` | SIMD intrinsic header generation helpers. |

## Oracles

The differential tools need a reference assembler:

```bash
export LCCC_GAS=/path/to/as
export LCCC_OBJCOPY=/path/to/objcopy
```

Any recent binutils works. To build a known-good one from source:

```bash
curl -O https://ftp.gnu.org/gnu/binutils/binutils-2.47.tar.xz
tar xf binutils-2.47.tar.xz && mkdir bu-build && cd bu-build
../binutils-2.47/configure --prefix=$PWD/../bu-247 \
  --disable-gdb --disable-gdbserver --disable-sim --disable-readline \
  --disable-libdecnumber --disable-nls --disable-werror \
  --disable-gprofng --disable-gprof --disable-plugins --with-system-zlib
make -j"$(nproc)" MAKEINFO=true all-gas all-binutils
```

`godbolt.py` needs only network access:

```bash
scripts/godbolt.py list --filter icx
scripts/godbolt.py compare kernel.c --flags "-O3 -march=x86-64-v3" \
    --local ./target/release/lccc
```
