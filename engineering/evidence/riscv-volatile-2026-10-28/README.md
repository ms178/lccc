# RISC-V volatile and jump-range audit

## Correctness evidence

The pre-change RV64 compiler dropped repeated `volatile` reads of local,
parameter and wide objects. The new gate checks an initialized stack slot
followed by two loads, as well as byte, wide, member, indirect and global
accesses and repeated stores. It has a nonvolatile negative control. The old
compiler failed three checks in each of nine assembly modes; the final compiler
passes five optimization levels, `-g`, disabled unwind tables and both
peephole escape hatches. For the no-unwind case, the entire assembly matches
the explicitly unoptimized output; an unrelated ordinary function still
benefits from local peepholes in the normal case. The fixture also links and
executes without libc under QEMU at all five levels. These tests do not prove
all volatile forms or GCC-compatible access widths.

Volatile instructions are marked while IR metadata is available. Complete,
proven CFI/type/size-bounded affected functions are opaque to the textual
peephole; ambiguous bounds or markers cause a whole-translation-unit fallback.
Identical-block merging and global DSE remain disabled for a unit with opaque
functions, because their whole-file census cannot see hidden uses. The
entry-copy coalescer and range-sensitive rewrites treat hidden lines as
barriers; unaffected, independently visible branch ranges remain eligible.
The unit suite includes debug rows, nested and malformed markers, hidden label
references and an adversarial local-symbol boundary. Removing the latter
barrier made its test fail by deleting a definition used by the hidden body.

A separate near-jump audit found that `.space` and alignment directives could
invalidate the old line-count range proof. In the checked-in C fixture, the old
compiler produced a `j` whose `R_RISCV_JAL` relocation was 1,048,580 bytes;
Rust LLD rejected it as out of range. The corrected compiler keeps the far
jump, and the object links and executes under QEMU. Unknown byte-emitting
or alignment directives block this shortening, and the instruction budget now
accounts conservatively for pseudo expansion. This is bounded coverage, not a
proof for arbitrary assembler macros or all target binaries.

The final focused run passed 43 RISC-V peephole unit tests, strict Clippy and
`cargo fmt --check`, the full volatile regression script, the new assembly and
QEMU gate, and RISC-V relaxation execution at `-O0` and `-O2` through both
LCCC's linker and Rust LLD 23.1.1. GNU RISC-V cross-binutils/GCC were absent;
the byte-for-byte RVC oracle and GNU cross-link legs were **skipped**.

## Static cost screen

An initial implementation disabled range-sensitive rewrites for *every*
function in a unit containing volatile code. In one synthetic file with 80
ordinary address-taken/branching functions, appending one volatile function
increased their total relocatable-object symbol sizes from 47,874 to 50,594
bytes at `-O0`, and from 18,618 to 21,338 bytes at `-O2` (14.6% at `-O2`).
That was not a runtime measurement. After narrowing the range barriers to the
relevant visible intervals, the same ordinary functions have identical
assembly lines and identical total symbol sizes with and without the appended
volatile function: 47,874 bytes at `-O0` and 18,618 bytes at `-O2`.
Before/after compiler output for the original no-volatile input is also
identical at those levels. The final values, hashes and counts are in
`ordinary-observed.json`.

This screen does **not** measure volatile-heavy functions, workloads where
whole-file DSE or block merging would matter, linking-time relaxation, compile
time, real RISC-V hardware or dynamic performance. Whole-function opacity and
the remaining disabled global passes can still cost substantial performance.

## Reproduction

With `readelf`, the pre-change and current RV64 drivers available:

```sh
python3 engineering/evidence/riscv-volatile-2026-10-28/ordinary-cost.py \
  --before /private/pre-change/lccc-riscv \
  --after target/fastbuild/lccc-riscv --output /tmp/lccc-rv-cost
python3 tests/regression/check_riscv_volatile_access.py \
  --compiler target/fastbuild/lccc-riscv --require-qemu
```

The second command requires Clang, a RISC-V-capable GNU/LLD linker and
`qemu-riscv64`; without `--require-qemu` it explicitly reports a runtime skip
when those tools are unavailable.
