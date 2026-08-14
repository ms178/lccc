# scripts/

Developer and research tooling. None of these are needed to build LCCC.

| Script | Purpose |
|---|---|
| `build_lccc_o1_j2.sh` | Build the compiler under the project policy: Rust opt-level 1, exactly two Cargo jobs, swap active. |
| `asmdiff.py` | Whole-object differential against GNU as: section bytes, relocations, and symbols. See `tests/asm-diff/README.md`. |
| `insndiff.py` | Per-instruction encoding differential against GNU as. Reduces an encoding bug to a single mnemonic in one step; supports `--sweep` over register/immediate matrices. A shorter-than-GAS encoding is reported as `BETTER` only after the tool disassembles both forms and confirms they decode identically. |
| `encdiff.py` | Multi-assembler encoding differential: LCCC against GNU as **and** the Clang, GCC, ICC and ICX integrated assemblers over the Compiler Explorer API. Judges LCCC against the *shortest legal encoding any oracle produced*, not against GAS alone. |
| `gen_asmdiff_corpus.py` | Generate the `tests/asm-diff/*.casefile` corpora. Every case is validated against GNU as before being written. |
| `godbolt.py` | Compiler Explorer client. Fetches GCC/Clang/ICC/ICX code generation so the Intel compilers can serve as reference oracles without a local Intel toolchain. |
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

A shorter encoding is never accepted on size alone. Both `encdiff.py` and
`insndiff.py` disassemble the two candidates and require them to decode to the
same instruction; `asmdiff.py` does the same for a whole object when a case is
marked `betterok`. This is not ceremony -- the first version of the fold was
4 bytes shorter *and wrong* for `%r12`/`%r13`, because those register numbers
mean something different in the base slot than in the index slot, and only the
round-trip check caught it.

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
make -j"$(nproc)" MAKEINFO=true all-gas all-binutils
```

`godbolt.py` needs only network access:

```bash
scripts/godbolt.py list --filter icx
scripts/godbolt.py compare kernel.c --flags "-O3 -march=x86-64-v3" \
    --local ./target/release/lccc
```
