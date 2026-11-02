# Fuzzer seed corpus

`good.c` builds a **freestanding, fully self-contained** executable: no libc,
no dynamic loader, and it runs and returns a checkable exit status.

That property is the whole point. An earlier seed used `printf`, so every
mutant died at symbol resolution with "undefined reference to printf" and the
campaign never reached layout, relocation or emission — 250 mutants found
nothing. With this seed the same fuzzer found two real crashes within minutes
(an integer-overflow bounds check and an unbounded allocation).

Regenerate the binary seeds with:

    ./generate_seeds.sh

## Two complementary fuzzers

`fuzz_ld.py` is a **byte mutator**: it flips bits and smashes fields in the
seed files above. That is the right tool for the header/table parser, where
the question is "does a malformed length crash us?".

Its limit is that almost every mutant dies at the first validity check, so the
machinery *behind* the parser -- symbol resolution, relocation application,
section merging, COMDAT groups, TLS -- is rarely reached. A large campaign can
be green while an entire emitter goes untested.

`fuzz_elf_grammar.py` attacks from the other side. It **builds** a
structurally valid ELF64 relocatable from a grammar (sections, a locals-first
symbol table, `.rela` sections with patched `sh_link`, string tables), then
applies exactly one *semantically* hostile choice:

  * a relocation whose offset runs past the end of its section
  * a relocation naming a symbol index that does not exist
  * a symbol whose `st_shndx` names a nonexistent section
  * a COMDAT group that lists itself, or a member/signature index out of range
  * `SHF_MERGE|SHF_STRINGS` with `entsize` 0, or without a NUL terminator
  * an alignment of 2^40, or one that is not a power of two
  * a TLS relocation against a non-TLS section
  * two strong definitions of one symbol in one object
  * a COMMON symbol of size 2^62
  * 2000 overlapping relocations on one small section
  * 400-character and empty section names

Because the file parses, the linker gets far enough for the interesting code
to run. Measured: **every** generated object is accepted by `readelf -h`, and
roughly half the links reach the emitter.

Inputs are rotated across seven modes -- exec, shared, `-r`, `-T`,
`--gc-sections`, `--icf=all`, `--emit-relocs` -- so a mutant can reach the
script and relocatable emitters, not only the userspace one.

### The contract being checked

The linker may accept a mutant, or reject it with a diagnostic. It may not
crash, hang, or claim success while emitting a file `readelf` rejects. Only
those outcomes are reported, each with a reproducer command line.

### Validating the fuzzer itself

A fuzzer that finds nothing is indistinguishable from a fuzzer that tests
nothing, so it was validated against a planted bug: a `panic!` on an
out-of-range relocation offset in `emit_exec.rs`. The campaign reported

```
[PANIC] mode=exec
    mutation: reloc offset past end of section
    repro: lccc-ld -o /tmp/.../out84 /tmp/.../m84_0.o
    stderr: lccc-ld: internal error: planted bug: reloc offset 4294967712 past image 4128
```

naming the mutation class and giving a working reproducer. Re-do this whenever
the generator changes.

### Usage

```
python3 tests/linker/fuzz_elf_grammar.py --lccc <path>/lccc-ld [--iters N] [--seed S] [--keep]
```

Exit status is non-zero when anything is found; `--keep` preserves the
reproducers.
