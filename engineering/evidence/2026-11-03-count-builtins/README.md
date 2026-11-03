# Count builtins: an aliased highbit fold on x86-64, and RISC-V's bit scan

Round 10 of the count/bitscan work (2026-11-03).  Round 9
(`2026-11-03-narrow-counts`) fixed what happens to a narrow count *after* it is
computed; this round is about the two places where the count itself is still
expensive: the `(W-1) - clz(x)` shape that every "highest set bit" idiom
compiles to, and the RISC-V backend, which had no instruction to lower `clz` to
at all.

```
python3 measure.py probe.c [<baseline>/fastbuild] target/fastbuild
```

`measure.py` compiles the probe with each driver's `-S`, counts the instruction
lines of every function up to and including its `ret`, and prints
`before -> after` per target.  The baseline column below was measured on
`0238c77` (this round's base) before either change; the "after" column is this
round.

## What changed

1. **x86-64: the aliased highbit fold.**  `(W-1) - clz(x)` and `W - clz(x)` are
   lowered as `mov $W-1|$W+mask, D; bsr S, D; xor mask, D; mov $K, R; sub D, R`.
   The peephole already folded that window when `R != S`, but the emitter also
   produces `R == S` whenever the subtraction's destination is the register the
   scan reads (a shift count, `%cl`).  The preload cannot go there without
   destroying the operand, so the aliased form keeps the whole computation in
   the dead `D` and leaves through one `lea` (addend and move in one
   instruction) or one `mov` when the addend vanishes.  `np2` on x86-64 is 7
   instructions, level with Clang's 7 and one behind gcc's 6, and the two
   removals are the two the fold was written for: the mask `xor` and the
   constant preload of `R`.
2. **RISC-V: `clz`/`ctz` are branch-free.**  Both were per-bit loops: a mask
   shifted right until a set bit was found, two branches per trip, up to 32
   (or 64) trips.  On a 64-bit operand `clzll(1)` executed ~320 instructions.
   `clz` is now `bits - bitlen` with `bitlen` taken as the population count of
   the operand smeared down (`x |= x >> 1; x >> 2; ...`), and `ctz` is the
   population count of `(x - 1) & ~x`; both reuse the already branch-free SWAR
   count, and both keep the defined-at-zero contract (`Clz(0) == width`)
   without a special case, which is what the loops' `beqz` arm used to do.

## Measured

`-O2`, instructions inside the function body including `ret`:

| probe | x86-64 | i686 | AArch64 | RISC-V | RISC-V before | gcc 16.2 x86 | clang 23.1.4 x86 |
|---|---|---|---|---|---|---|---|
| `np2`      | 7 | 17 | 16 | 48 | 27 | 6 | 7 |
| `np2b`     | 6 | 16 | 15 | 45 | 24 | 5 | 6 |
| `np2ll`    | 9 | 45 | 15 | 46 | 23 | 6 | 7 |
| `np2_guard`| 11 | 29 | 22 | 61 | 40 | 8 | 9 |
| `fls`      | 4 | 17 | 16 | 49 | 28 | 6 | 5 |
| `highbit`  | 3 | 14 | 13 | 41 | 20 | 2 | 2 |
| `msb_pow2` | 5 | 16 | 15 | 45 | 24 | 5 | 5 |

The RISC-V column is the honest trade: **static size roughly doubles** (the
`highbit` body grows from 80 to 150 bytes) and the *dynamic* cost becomes
constant.  The loop's trip count was the answer itself, so a 64-bit count on a
small operand ran 64 iterations of five instructions with two unpredictable
branches each; the branch-free form executes a fixed ~41 instructions with no
branch at all.  Constant time also matters independently of speed: the kernel
is the intended consumer, and a data-dependent-branch bit scan is a timing
oracle.  The base ISA has no bit scan and the backend deliberately does not
require Zbb (`riscv/README.md`), so this is the same class of answer Clang
inlines (smear plus SWAR count) rather than GCC's `__clzdi2` call.

## What is certified, and how

`tests/regression/check_count_builtins_runtime.py` (new, registered in
`ci-lane-plan.json`, `ci.yml` and `ci_local.sh`) links freestanding kernels for
`__builtin_clz/ctz`, `clz/l` and `clzll` against in-image shift-loop references
and runs them natively on x86-64 and under QEMU for i686, AArch64 and RISC-V at
`-O0/-O1/-O2/-O3/-Os`: 20 executions.  The sweep is exhaustive over the low
half-word, covers every single-bit operand with both neighbours and the
complements, and adds 100k randomized 64-bit draws.  Operands arrive through
`volatile`, so a constant folder cannot stand in for the lowering.  The gate
was mutation-checked (a deliberately off-by-one `clz` exits non-zero on
RISC-V), so a green run means the counts were compared, not merely emitted.

Before this gate, `clz`/`ctz` were certified only for *instruction selection*
on x86 (`check_clz_ctz_target_isa.sh`) and by a hosted-only differential
(`clz_ctz_popcount_target_isa.c`), which cannot run the cross backends; the
three non-host backends had no value-level check at all.

## Found, not fixed in this round

* **i686 still pays seven instructions for `highbit`** (`bsr`-based scan plus
  the `31 - clz` subtract, dumped in the PR).  The fold cannot be ported
  verbatim: i686's linear peephole pass has no register liveness, and `%edx` is
  half of the 64-bit return pair, so "D is dead after the window" cannot be
  proven there.  It needs the CFG liveness the i686 pass already computes for
  `eliminate_dead_reg_moves` to be exposed to the local window pass.
* **AArch64's `highbit` is one `clz` and three copies**
  (`clz w0,w0; mov x4,x0; mov x5,#31; sub w5,w5,w4; mov x0,x5`): the
  accumulator copies this round's scope names, plus a 64-bit `mov` of a 32-bit
  value.  A retarget-the-copy pass would make it three instructions.
* **RISC-V ignores the `ClzNonZero`/`CtzNonZero` hints.**  The trait's default
  implementation forwards them to `emit_int_clz`, which is why the old loop
  kept its `beqz` arm; the branch-free form does not need the hint, but a
  future non-zero specialisation (or a Zbb path) would.
* **`riscv/codegen/README.md` had drifted**: it still described `POPCOUNT` as
  Kernighan's loop (round 8 replaced it with SWAR) and `CLZ`/`CTZ` as per-bit
  scans.  All three sections are corrected here.
