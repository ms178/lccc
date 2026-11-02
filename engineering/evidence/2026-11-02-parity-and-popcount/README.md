# Parity and popcount — 2026-11-02

The design and the numbers live in
[§18 of `SCALAR-PREDICATE-FOLDS.md`](../../llvm-missed/SCALAR-PREDICATE-FOLDS.md);
this directory holds what a re-run needs.

## The gate

```sh
CCC=target/fastbuild/lccc bash tests/regression/check_parity_and_popcount.sh
```

Six parts, ~2 s: the x86-64 fold shape (`setnp`, no `imul`, no branch) and the
`-mpopcnt` path; the AArch64 table under `-mgeneral-regs-only` and the SIMD
`cnt` path; the RISC-V table and the branch-free count; the attribution
(`CCC_SIMPLIFY_SKIP=parity`); pinned instruction counts; and value agreement
with `tests/regression/parity_and_popcount.c` at `-O0`/`-O1`/`-O2`/`-O3`/`-Os`
plus GCC 16.2 at `-O2`.

## The models

LCCC's static cross-target binaries do not run under qemu-user in this sandbox
(aarch64 SIGILL, riscv64 SIGSEGV), so the emitted sequences were re-implemented
in C and compared bit for bit with `__builtin_parity` over an exhaustive
domain (every `u8`/`u16`, 4096 arithmetic `u32`/`u64` operands, the edges,
every single-bit operand):

```sh
cc -O2 -o x86_flag_model x86_flag_model.c && ./x86_flag_model   # x86-64 + i686 fold chains
cc -O2 -o table_model   table_model.c   && ./table_model        # RISC-V / AArch64 table lookup
```

Both print `mismatches=0`.  `clang-23 --target={aarch64,riscv64}-linux-gnu -c`
assembles the LCCC dumps of the same probes, which is what catches register
field and immediate encoding mistakes that a C model cannot.

## Measured instruction counts

`-O2`, total instruction lines of the function including `ret`, directives and
labels stripped (`count.py` in the session log did the same thing);

| probe | before | now | GCC 16.2 | Clang 23.1.4 |
| --- | --- | --- | --- | --- |
| `__builtin_parity(u32)` x86-64 | 18 | 8 | 7 | 7 |
| `__builtin_parityll(u64)` x86-64 | 22 | 11 | 10 | 10 |
| `__builtin_parity(u32)` i686 | 23 | 13 | 14 | 14 |
| `__builtin_parityll(u64)` i686 | 27 | 16 | 17 | 17 |
| `__builtin_parity(u32)` AArch64 | 15 | 13 | 14 | 8 |
| `__builtin_parityll(u64)` AArch64 | 15 | 13 | 15 | 10 |
| `__builtin_parity(u32)` RISC-V | 29 | 17 | 15 | 14 |
| `__builtin_parityll(u64)` RISC-V | 29 | 17 | 16 | 14 |
| `__builtin_popcount(u32)` RISC-V | 29 | 24 | 21 | 21 |

"before" is `CCC_SIMPLIFY_SKIP=parity` with the same source (the count plus
`andl $1`), or the previous backend emission for the RISC-V count.

## Reproducing the source

```c
typedef unsigned u32; typedef unsigned long long u64;
int par32(u32 x) { return __builtin_parity(x); }
int par64(u64 x) { return __builtin_parityll(x); }
int pc32(u32 x)  { return __builtin_popcount(x); }
int pc64(u64 x)  { return (int)__builtin_popcountll(x); }
```

```sh
for t in "" -i686 -arm -riscv; do
  target/fastbuild/lccc$t -O2 -S parity.c -o parity$t.s
done
clang-23 --target=aarch64-linux-gnu -O2 -mgeneral-regs-only -S parity.c -o par-arm-clang.s
clang-23 --target=riscv64-linux-gnu -O2 -S parity.c -o par-riscv-clang.s
```
