# Multi-word addition and subtraction as `adc`/`sbb` chains (x86-64)

C has no carry. Code that adds numbers wider than a register spells the
carry out as comparisons, limb by limb:

```c
s = a + b;   t = s + c;   c = (s < a) | (t < s);   r[i] = t;
```

`_addcarry_u64` and `_subborrow_u64` (`include/x86gprintrin.h`) are written
that way, `__builtin_addcll` and `__builtin_add_overflow` are lowered to it,
`unsigned __int128` arithmetic is split into it, and GMP, mini-gmp and
OpenSSL write it by hand, each a little differently. The machine has one
instruction for the whole line. Before this work lccc emitted the line as
written (46 instructions for four limbs of `_addcarry_u64`); now it emits
`adc`.

Everything below is held by tests (last section). The numbers were measured
with GCC 16.2 and Clang 23.1.5 on an Ice Lake-SP (family 6, model 106), each
table in one sitting; run times are the best of 5.

## What is recognised

`src/passes/carry_chain.rs` rewrites a full adder -- two additions, two
comparisons and what joins them -- into the two halves of one IR intrinsic,
adjacent and on the same three operands:

```text
    t   = Carry{word}(x, y, c)      ; x + y + c
    out = Carry{out}(x, y, c)       ; its carry, 0 or 1
```

(`CarryOp { borrow, out, wide }`: the same for `x - y - c` and its borrow,
at 32 or 64 bits.) The spellings that are matched:

| | |
|---|---|
| the comparison | against either operand: `s < a`, `s < b`, `t < s`, `t < c`; a borrow as `a < b` or as "the difference grew", `d > a` |
| the order | sum first; carry first (`(a + c) + b`: mini-gmp, OpenSSL); the subtrahends added up first (`b += cy; cy = b < cy; cy += a < b; r = a - b`: mini-gmp); the carry computed ahead of the word |
| the join | `\|`, `+`, `^`, `\|\|` (which arrives as `c1 ? 1 : c2`), and `join != 0` for a `_Bool` carry |
| the bit | any of the three operands of a sum; either subtrahend of a difference |
| the ends | a first addition whose comparison feeds only the next limb (the pair with carry 0: `add`, `setc`); a last addition `(x + y) + carry` whose own carry nobody reads (the word alone: `adc`) |
| loops | a carry that goes round a loop through a phi, joined by `\|` or by `+` |

What is *not* an adder, and is left as written:

* a carry in that is not proven 0 or 1. `__builtin_addcll(a, b, n, &o)` adds
  any `n`; `adc` adds one bit.
* three words. `(s < a) + (t < s)` of `a + b + w` counts up to two carries;
  `|` of them is a different function again.
* a difference whose minuend is the bit: `1 - y - w` borrows twice for
  `y = w = 2^64 - 1`.
* a signed comparison, or one against a value that is not an operand.
* a count of carries round a loop (`cy += s < a; cy += t < s` with `cy`
  never reset).

## Why the rewrite is sound

*One at most of the two partial carries is set.* For `x + y + c` with `c`
a bit the sum is below `2 * 2^64`, so it carries once at most; hence `|`,
`+` and `^` of the partial carries are the same bit, which is the carry out.
For `x - y - c` with `y` or `c` a bit the subtrahends are `2^64` together at
most, so the difference borrows once at most. With the subtrahends added
first, `y + c` wraps only to 0 (`y` all ones, `c` one), and nothing is
below 0: the wrap and the borrow exclude each other.

*What a bit is* (`booleans`): a comparison, a carry out, the constants 0
and 1, `v & 1`, and a conversion, copy, `&`, `|`, `^`, select or phi of
nothing but bits. It is computed as a greatest fixed point: every candidate
is assumed a bit, and one that reads anything else is struck together with
its readers. So a carry that enters a loop as 0 or 1 and leaves each trip
as a carry is a bit by induction over the trips.

*The sum of two partial carries* is a bit only as the join of an adder one
of whose operands is a bit. It is a candidate that stands while such an
operand stands; round a loop that operand is the sum itself, and the fixed
point keeps both or strikes both.

*Placement.* The pair reads `x`, `y` and `c`, which the old instructions
read at the same place or earlier. It takes the place of the second
addition; where no comparison reads that word (`d < c`, `a < b + c`) the
word is looked for in the block of the join, and one that stands behind the
join is computed where the join was, under a new name. Both halves then
stand ahead of every reader of the word and of the join. The old
comparisons, the join and the first addition are left to dead-code
elimination and survive where something else reads them.

*Width.* A carry out is 0 or 1 in a whole register, like a comparison's
result. A join of more than 32 bits becomes a conversion of the carry out
from `U32`, placed directly behind the pair (in place it cost a `movslq` a
trip and kept the loop's phi from being coalesced).

## What is emitted

`emit_carry_impl` (`src/backend/x86/codegen/alu.rs`): `x` goes to `%rax`;
the carry in is put into CF *last*, after every load (`btl $0, %reg`,
`btl $0, slot`, `stc`, or nothing for a constant 0); `adc`/`sbb` reads `y`
from its register, from memory or as an immediate; `setc` takes the carry
out. Loading an operand may write the flags (`xorl` for a zero, the
alignment of a stack slot), which is why the flag is set after the loads.
When both `y` and `c` need the loader the flag is carried across the loads
with `pushfq`/`popfq`; no generated program of the tests below reaches that
path, and it is exercised by the emitter's unit tests, which execute the
emitted lines on a model of the registers, the stack and CF.

Between two links of a chain the carry never has to leave the flag. The
peephole `multiword::forward_carry_flag` removes
`setc %cl; movzbl %cl,%r; ...; btl $0,%r` when nothing in between writes a
flag and the boolean has no other reader, and `load_op_fuse` folds a limb's
load into the `adc`. Four limbs of `_addcarry_u64` are then

```asm
    movq  (%rsi), %rax
    addq  (%rdx), %rax
    movq  %rax, (%rdi)
    movq  8(%rsi), %rax
    adcq  8(%rdx), %rax
    ...
```

13 instructions, the same count as GCC and Clang (46 before).

Where a carry is added instead of chained (`hi += lo < x`, the column sums
of a schoolbook product), the folds `multiword` already had apply: the
compare that recomputes the carry of the addition above it is dropped
(`fold_carry_compare`), and the widened carry becomes `adc $0`. The sum is
usually stored before its carry is read (`c[0] += lo; hi += c[0] < lo`), so
the fold now looks past flag-neutral lines that write neither register of
the compare: `two_acc3` went from 13 instructions to 9. Behind such lines
the compare may name registers and immediates only. A store in between may
be to the very word it would read again (`r[1] = s; hi += s < x[0]` with
`x == r + 1` compares the sum with itself, and that is no carry), which
`two_stored_over_addend` and its two neighbours in the fixture hold.

Both walks -- from a `bt` up to the `setc` it repeats, from a compare up to
its addition -- step over any flag-neutral line, so "does this line write
the register" is asked of the peephole's over-approximating oracle
(`load_op_fuse::may_write_families`): `xchg` writes both its operands,
`mulx` two of its three, and a mnemonic nobody listed writes everything.

A carry that does leave the flag -- round a loop, or into a result -- is
taken out as `movl $0, %r; setc %rb` (`multiword::widen_carry_in_place`,
late, once), not as `setc %cl; movzbl %cl, %r`. Both are two instructions;
the first has one of them between the flag and the register, the second
two. Round a limb loop that path is the loop (`bt`, `adc`, `setc`,
extension), and where the extension is not eliminated at rename (this
machine, Zen) it was one cycle in four: with the old form mini-gmp's
subtraction loop ran slower as `sbb` (0.990 s) than it had as comparisons
(0.913 s).

## Measurements

Instructions per function at `-O2` over the adder functions of
`tests/regression/carry_chain_adders.c` that all four compilers keep as
functions (86 of them): before the work, now, GCC, Clang.

| | before | lccc | GCC 16.2 | Clang 23.1.5 |
|---|---:|---:|---:|---:|
| all 86 | 1,987 | 1,225 | 1,171 | 1,433 |
| `hdr_add4` (4 limbs, `_addcarry_u64`) | 46 | 13 | 13 | 13 |
| `hdr_sub4` | 65 | 16 | 14 | 14 |
| `cmp_add4` (4 limbs, comparisons) | 39 | 16 | 17 | 15 |
| `hdr_addn` (limb loop, intrinsic) | 36 | 16 | 18 | 47 |
| `lib_gmp_add_n` | 27 | 16 | 19 | 42 |
| `lib_gmp_sub_n` | 30 | 16 | 19 | 42 |
| `lib_mini_add_n` | 25 | 16 | 21 | 45 |
| `lib_mini_sub_n` | 24 | 16 | 22 | 40 |
| `lib_ossl_add_words` | 29 | 22 | 20 | 51 |
| `lib_i128_add_n` | 22 | 16 | 24 | 37 |
| `lib_ovf_add_n` (`__builtin_add_overflow`) | 23 | 16 | 17 | 16 |
| `lib_bool_add_n` (`_Bool` carry, `\|\|`) | 36 | 16 | 24 | 45 |
| `two_acc3` (`c[0] += lo; hi += c[0] < lo; ...`) | 13 | 9 | 5 | 5 |
| `two_add_ssaaaa` (GMP `longlong.h`) | 7 | 7 | 5 | 5 |

Run time of the limb loops, seconds: before the work, now, GCC, Clang. The
loops are the five the libraries write, taken as they are written, over
2,048 limbs 400,000 times and over 256 limbs 3,200,000 times.

| 2,048 limbs | before | lccc | GCC 16.2 | Clang 23.1.5 |
|---|---:|---:|---:|---:|
| GMP `mpn_add_n` (generic C) | 1.245 | 0.746 | 0.996 | 0.870 |
| GMP `mpn_sub_n` | 1.498 | 0.747 | 0.994 | 0.867 |
| mini-gmp `mpn_add_n` | 1.553 | 0.748 | 0.882 | 0.909 |
| mini-gmp `mpn_sub_n` | 0.913 | 0.746 | 1.050 | 0.774 |
| `unsigned __int128` sum | 1.002 | 0.744 | 0.577 | 0.767 |

| 256 limbs | before | lccc | GCC 16.2 | Clang 23.1.5 |
|---|---:|---:|---:|---:|
| GMP `mpn_add_n` | 1.251 | 0.743 | 0.976 | 0.835 |
| GMP `mpn_sub_n` | 1.506 | 0.757 | 0.987 | 0.843 |
| mini-gmp `mpn_add_n` | 1.550 | 0.756 | 0.899 | 0.900 |
| mini-gmp `mpn_sub_n` | 0.914 | 0.743 | 1.042 | 0.741 |
| `unsigned __int128` sum | 1.003 | 0.748 | 0.734 | 0.749 |

A loop with a constant trip count is unrolled four times, and the chain
then runs through the four limbs of a trip: 2.002 s before, 0.380 s now,
GCC 0.750 s, Clang 0.505 s for 2,048 limbs 400,000 times.

## Not done

* **The carry of a loop leaves the flag once a trip.** `bt` at the top,
  `mov $0; setc` at the bottom: three steps a limb, which is what bounds
  every loop above at 0.75 s. Two things are faster. GCC's `__int128` loop
  adds the carry as a word (`add; adc $0`), two steps a limb: 0.577 s at
  2,048 limbs. And the flag can stay in the flag across the back edge if
  the loop test does not write it (`lea`/`dec`/`jnz`): mini-gmp's
  `mpn_add_n` rewritten by hand that way ran in 0.50 s. The peephole
  rewrites lines in place and cannot add the lines the loop's entry and
  exit need for that.
* **Counted carries.** Where no operand is a bit the source wants
  `add; setc; add; adc $0`: `cmp_add_plus_word` 13 instructions against
  GCC's 8, `lib_count_n` 35/28, `lib_count_carried_n` 27/19,
  `lib_i128_add3_n` 30/22.
* **Two words.** `two_acc3` is 9 instructions where GCC has 5: the sum and
  the carry are not added into memory (`add %reg, mem`, `adc $0, mem`).
  `two_add_ssaaaa` is `lea; lea; cmp; adc $0` (7) where GCC has `add; adc`
  (5): the head and the tail of a two-word chain are found only in a
  function that also has a full adder.
* **A carry in spelled `c != 0` on a byte** costs `test; setne; movzbl; bt`
  where GCC has `addb $-1, %dil`: `hdr_add64` 12/5, `hdr_carry_only` 9/4.
* **In place.** `adc %reg, mem` is not formed: `hdr_in_place` 16/10.
* `lib_ossl_sub_words` (30; GCC 33 and Clang 60 do not find it either),
  `lib_i128_sub_n` (21/22).
* At `-O1` and `-Og` the pass does not run.
* i686: the pass is x86-64 only; 64-bit arithmetic there is already
  `add`/`adc` on register pairs.

## How it is held

| | |
|---|---|
| `tests/regression/carry_chain_adders.c` | 92 functions in every spelling above, alone, in chains, in loops, across branches and calls, with constants, with either result unused, with a result stored over an operand; each result checked against an adder that computes in 32-bit halves. In the regression corpus. |
| `tests/regression/check_carry_chain_shape.py` | The assembly of those functions: the number of `adc`/`sbb` in each, exactly; no `setc`/`bt` inside a straight chain; no more instructions than today. 88 functions, 67 with a chain, 94 links. With the pass off 58 of the pins break, which is the negative control. |
| `tests/regression/check_carry_chain_fuzz.py` | Random adders (see the file's header): programs of ten functions assembled from the pieces above and from pieces that make them *not* adders, compared with GCC's program line by line through every optimization level and with the pass, the peephole and register allocation off in turn. One planted wrong carry as the negative control. CI runs eight programs (seeds 9 to 16) and the fixture through seven configurations. The window was chosen by breaking the compiler: with the compare behind a store allowed to name memory, seeds 13 and 14 differ from GCC (and the fixture reports 9,214 failures); with the registers it names allowed to be written in between, seeds 10 and 16 do. Run by hand through all fourteen configurations without a difference: 2,400 functions of the generator as it was before the accumulator forms (seeds 1 to 240), and 240 of the present one (seeds 1 to 24), whose programs GCC at `-O0`, GCC at `-O2` and Clang with every undefined-behaviour and integer check trapping print alike (seeds 1 to 40). |
| `src/passes/carry_chain.rs` | 22 unit tests: each spelling, and each refusal. |
| `src/backend/x86/codegen/alu_carry_tests.rs` | The emitter, executed on a model: every home of `x`, `y` and `c`. |
| `peephole/passes/multiword_tests.rs` | The flag forwarding and the carry taken out in place, executed before and after on a model of the registers, memory and CF; their refusals. |

`CCC_DISABLE_PASSES=carry` switches the pass off, `CCC_PEEPHOLE_SKIP=multiword`
the peepholes (`widen_carry` that one alone); `CCC_DUMP_EACH_PASS=1` prints
the IR "before carry" and "after carry".
