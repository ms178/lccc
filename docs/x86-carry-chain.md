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
`adc`, and a loop over limbs runs four of them a trip with the carry in the
flag from one to the next.

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

## Four limbs a trip

Inside one iteration the carry lives in the flag. Round a loop it cannot:
the loop's own counting and compare write the flags. So each trip takes the
carry out at the bottom (`setc`) and puts it back at the top (`bt`), and
that path -- flag, register, flag -- is what the loop runs at: three steps a
limb, where a chain of `adc` takes one. Every limb loop of the libraries ran
in 0.75 s for 2,048 limbs 400,000 times, whatever its spelling.

`src/passes/limb_unroll.rs` splits such a loop by its trip count `d`:

```text
  guard:  is d known exactly, and at least four?
  main:   four copies of the body a trip, d >> 2 trips
  L:      the loop as it was, for the d & 3 limbs that are left,
          and for all of them where the guard said no
```

`carry_chain` then finds four adders in a row, and the main loop of GMP's
`mpn_add_n` is

```asm
.Lmain:
    movq  (%rdi), %rax
    btl   $0, %esi
    adcq  (%r10), %rax
    movq  %rax, (%r11)
    movq  8(%rdi), %rax
    adcq  8(%r10), %rax
    movq  %rax, 8(%r11)
    movq  16(%rdi), %rax
    adcq  16(%r10), %rax
    movq  %rax, 16(%r11)
    movq  24(%rdi), %rax
    adcq  24(%r10), %rax
    movl  $0, %esi
    setc  %sil
    movq  %rax, 24(%r11)
    leaq  32(%r11), %r11
    leaq  32(%r10), %r10
    leaq  32(%rdi), %rdi
    subq  $1, %r9
    testq %r9, %r9
    jne   .Lmain
```

**Which loops.** One block of work, tested at the top (`for`, `while`) or at
the bottom (`do ... while`), one way in and one way out; a counter stepped
by one, up or down, in 32 or 64 bits, against something the loop does not
change (`!=`, `<` and `>` signed or unsigned, `<=` and `>=` against a
constant that has a neighbour, `while (n)`, `do ... while (--n)`,
`do ... while (n-- != 1)`); at most 32 instructions; and a carry that goes round
the loop through exactly one full adder, which is asked of the matcher the
carry pass itself uses (`carry_chain::Carried`). Limbs through an index or
through stepped pointers, or both.

**Nothing but the limb.** The trip has to be the loads, the stores, the
addresses they go through and the constant steps of the counter and the
pointers (`only_a_limb`). The chain holds only if nothing between two
adders writes the flags, and a sum or a compare of the data does: such a
trip takes the carry out for every limb as before, in four times the code.
Measured, 2,048 limbs 400,000 times, with a build that split every loop of
one adder, and the loop as it is:

| the trip also does | split | as it is | GCC 16.2 | Clang 23.1.5 |
|---|---:|---:|---:|---:|
| `h += t` | 1.065 | 0.784 | 1.314 | 1.347 |
| `h ^= t` | 1.064 | 0.768 | 1.312 | 1.339 |

Both are left as they are. A limb that is kept (`last = t`) is a move and
is split: 0.388 s, for 0.786 s unsplit (0.809 s split before the parked
carry below was forwarded).

**The same program.** `d` is the distance from the counter's first value to
its limit in the direction of the step, modulo 2^n in the counter's type.
With `!=` the loop runs exactly `d` iterations; with `<` or `>` it does if
the first value is on the near side of the limit, and the guard enters the
main loop only then. The guard asks for `d >= 4` unsigned. Each trip of the
main loop is four consecutive iterations the loop would have run, with
every load and store in the order it had: no load is speculated, and a
store that the next iteration loads (`r == a + 1`) is loaded as before.
What is left goes to the loop as the source wrote it. The unit tests run
every shape, as it was and split, on a machine for the IR over the counts
round a trip and the values at which a counter wraps, and compare the two
runs load by load and store by store.

**What the chain needed besides the copies**, each found by reading the
assembly and each held by a test:

* A copy names what the counter or a pointer is in it from the trip's own
  value (`8(p)`, `16(p)`), not from the copy before (`((p + 8) + 8)`): an
  `add` between two limbs writes the flag the carry is in.
* The step for the next trip stands behind the last adder, also where the
  stepped value is read behind the loop; an address that one access reads
  stands directly in front of it.
* The main loop counts its trips down by itself (`subq $1`), so that the
  copies need no compare.
* **The allocator.** A value was taken for "alive across the loop" when its
  live range began before the loop and ended behind it, holes or not, and
  the parameters of a split loop (alive in the guard and in the old loop,
  not in the main loop) lost their registers to it.
  `LiveRange::alive_within` asks for the header and the latch themselves.
  Over 2,281 files of the corpus at `-O2`: 110 instructions and 18 pushes
  fewer; 26 files change. The rule is worth 7 % at eight limbs and 14 % at
  sixteen (GMP's `mpn_add_n`, 0.307 s to 0.286 s and 0.240 s to 0.207 s for
  409.6 million limbs), nothing from 256 limbs on.

  Two more changes go with it, each for a program the rule alone made
  slower:

  * `zstd_count` (+5 % to +7 %). The limit of its compare loop was a span
    of a loop it is dead in and lost its register for that; with a
    register for it, the *recurrence floor* (a loop's carried value that
    finds no register takes the cheapest one that is no recurrence) took
    the register of `state = state * 1664525 + 1013904223`. That sum is
    read four times in the body, so it is no member of the phi's web and
    carried no mark; its slot sat between one query and the next. A value
    that a copy carries round a loop (`carried_round_loop`: the phi, what
    is copied back into it and the definitions between the two) is no
    victim of the floor now.
  * `csv_field_sum` (+3 %). With no parameter in the frame any more, the
    other of its two block orders won, and in that one `c = *++p` kept both
    latch copies (12 instructions a digit for 10): a temporary behind the
    loop held the phi's register while the value copied back was alive,
    and every other register was taken by an envelope. The temporary now
    takes the register that value leaves (`rehome_phi_blockers`): where a
    value is alive it is the one holder of its register.

  The three together over the 87 programs of `tests/benchmark/programs`
  that compile alone: 15,972 to 15,943 instructions, 424 to 420 pushes,
  1,017 to 1,006 stack operands, 1,552 to 1,541 register moves; eleven
  programs change. Wall clock against the allocator as it was (interleaved
  runs, median / minimum): `glibc_dl_hash` 0.949 / 0.960, `linux_rbtree`
  0.971 / 0.982, `zlib_ng_huffman_heap` 0.986 / 0.982, the other eight
  between 0.991 and 1.008 (`zstd_count` 1.001 / 0.991, `csv_field_sum`
  1.001 / 1.008). The gate's own record of it is the note of 2026-11-25 in
  `.github/scripts/ci-codegen-gate.py`.
* **A carry parked in the frame.** With one more value alive across the
  loop there is no register for the carry between two copies, and it is
  kept in the frame: `setc %cl; movzbl %cl, %eax; movl %eax, 4(%rsp)` ...
  `btl $0, 4(%rsp)`. The flag has not been written in between, so
  `forward_carry_flag` now drops that `bt` as well (`parked_in`): between
  the store and the test every line that names the frame is a plain move
  that leaves the tested byte alone, nothing moves `%rsp`, and the function
  takes no address in its frame, so that a store through a pointer in
  between -- the limb itself -- is not a store into the slot
  (`frame_is_private`; the discipline of `eliminate_never_read_stores`,
  which follows the stack's depth and knows how far an address reaches,
  where this refuses the function). A directive that emits data refuses it
  too -- `.byte 0x48, 0x89, 0xe0` is `mov %rsp, %rax` and names nothing --
  by the one law for directives (`LineInfo::is_steppable`), which
  `scripts/check_peephole_directive_step.py` holds every walker of the
  peephole to. The store into a slot nobody reads then
  goes with the other dead stores, and the boolean nobody reads behind it
  (`drop_unread_carries`, peephole phase 5c). Hand-unrolled source had the
  same broken chains before.

**What it costs.** Code: a split loop is 44 to 65 instructions where it was
16 to 22 (the fourteen limb loops of the fixture: 681 instructions for 230;
Clang, which unrolls them too, has 560, GCC, which does not, 308). The pass
does not run at `-Os`, nor where the carry pass does not. Short numbers pay
for the guard, 409.6 million limbs in numbers of `N` limbs, seconds:

| `N` | GMP unsplit | split | GCC | Clang | mini-gmp unsplit | split | GCC | Clang |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 1 | 1.764 | 1.800 | 1.904 | 1.771 | 1.790 | 1.790 | 1.926 | 1.753 |
| 2 | 0.885 | 0.896 | 1.023 | 0.926 | 0.898 | 0.925 | 1.048 | 0.993 |
| 3 | 0.675 | 0.706 | 0.766 | 0.711 | 0.732 | 0.716 | 0.806 | 0.800 |
| 4 | 0.570 | 0.444 | 0.683 | 0.591 | 0.547 | 0.444 | 0.616 | 0.611 |
| 5 | 0.493 | 0.423 | 0.570 | 0.524 | 0.448 | 0.402 | 0.547 | 0.565 |
| 8 | 0.410 | 0.288 | 0.460 | 0.485 | 0.391 | 0.289 | 0.433 | 0.427 |
| 16 | 0.368 | 0.210 | 0.457 | 0.372 | 0.372 | 0.247 | 0.409 | 0.406 |

Below four limbs the split costs up to 5 % (best of 3); from four on it
wins.

`CCC_DISABLE_PASSES=limb_unroll` leaves the loops as they are.

## Measurements

Instructions per function at `-O2` over the adder functions of
`tests/regression/carry_chain_adders.c` that all four compilers keep as
functions (86 of them): before the work, now, GCC, Clang.

| | before | one `adc` a trip | lccc | GCC 16.2 | Clang 23.1.5 |
|---|---:|---:|---:|---:|---:|
| all 86 | 1,987 | 1,225 | 1,676 | 1,171 | 1,433 |
| the 72 that are no limb loop | | 995 | 995 | 863 | 873 |
| `hdr_add4` (4 limbs, `_addcarry_u64`) | 46 | 13 | 13 | 13 | 13 |
| `hdr_sub4` | 65 | 16 | 16 | 14 | 14 |
| `cmp_add4` (4 limbs, comparisons) | 39 | 16 | 16 | 17 | 15 |
| `hdr_addn` (limb loop, intrinsic) | 36 | 16 | 44 | 18 | 47 |
| `lib_gmp_add_n` | 27 | 16 | 62 | 19 | 42 |
| `lib_gmp_sub_n` | 30 | 16 | 62 | 19 | 42 |
| `lib_mini_add_n` | 25 | 16 | 44 | 21 | 45 |
| `lib_mini_sub_n` | 24 | 16 | 44 | 22 | 40 |
| `lib_ossl_add_words` | 29 | 22 | 65 | 20 | 51 |
| `lib_i128_add_n` | 22 | 16 | 44 | 24 | 37 |
| `lib_ovf_add_n` (`__builtin_add_overflow`) | 23 | 16 | 44 | 17 | 16 |
| `lib_bool_add_n` (`_Bool` carry, `\|\|`) | 36 | 16 | 48 | 24 | 45 |
| `two_acc3` (`c[0] += lo; hi += c[0] < lo; ...`) | 13 | 9 | 9 | 5 | 5 |
| `two_add_ssaaaa` (GMP `longlong.h`) | 7 | 7 | 7 | 5 | 5 |

"One `adc` a trip" is the loop unsplit (`CCC_DISABLE_PASSES=limb_unroll`).
A split loop is larger by the main loop and its guard; what it buys is
time.

Run time of the limb loops, seconds: unsplit, split, GCC, Clang. The loops
are the five the libraries write, taken as they are written, over 2,048
limbs 400,000 times and over 256 limbs 3,200,000 times; before the work
they took 1.245, 1.498, 1.553, 0.913 and 1.002 s at 2,048 limbs (an earlier
sitting).

| 2,048 limbs | one `adc` a trip | lccc | GCC 16.2 | Clang 23.1.5 |
|---|---:|---:|---:|---:|
| GMP `mpn_add_n` (generic C) | 0.764 | 0.377 | 0.999 | 0.867 |
| GMP `mpn_sub_n` | 0.749 | 0.375 | 0.998 | 0.873 |
| mini-gmp `mpn_add_n` | 0.756 | 0.377 | 0.902 | 0.937 |
| mini-gmp `mpn_sub_n` | 0.806 | 0.375 | 1.076 | 0.789 |
| `unsigned __int128` sum | 0.781 | 0.387 | 0.566 | 0.786 |

| 256 limbs | one `adc` a trip | lccc | GCC 16.2 | Clang 23.1.5 |
|---|---:|---:|---:|---:|
| GMP `mpn_add_n` | 0.741 | 0.358 | 0.988 | 0.842 |
| GMP `mpn_sub_n` | 0.753 | 0.358 | 0.986 | 0.852 |
| mini-gmp `mpn_add_n` | 0.772 | 0.374 | 0.920 | 0.894 |
| mini-gmp `mpn_sub_n` | 0.755 | 0.369 | 1.063 | 0.752 |
| `unsigned __int128` sum | 0.741 | 0.373 | 0.603 | 0.759 |

All four programs print the same checksum for each loop.

## Not done

* **The carry still leaves the flag once in four limbs.** `bt` at the top
  of a trip, `mov $0; setc` at its bottom, because `subq $1` writes the
  carry. Counting the trips with `dec`, which leaves CF alone, keeps the
  carry in the flag round the back edge: GMP's `mpn_add_n` written that way
  by hand runs in 0.262 s where lccc's loop takes 0.377 s. The `testq`
  behind `subq $1` is redundant as well.
* **Limb loops that are not split.** Of the 320 plain loops of seeds 1 to
  80 (`check_carry_chain_fuzz.py --limbs --census --start 1 --seeds 80`)
  154 are split. Of the other 166, 52 have a constant count of twelve at
  most and go to the loop unroller. The rest are not taken yet, for one of
  these reasons or several: the carry starts as `p >> 63`, which is not
  known to be a bit (32 of the 34 such loops); the index is counted from
  the other end, `a[n - i]` (20); `while (n-- > 0)`, whose step stands in
  the loop's header (19); an `int` counter, which arrives as a 64-bit
  counter tested through its truncation (13 more); the borrow of an
  `unsigned __int128` difference taken as `(t >> 64) & 1`, a join the carry
  pass does not know (10); and 35 others, 26 of which are no adder to the
  carry pass in the first place (32-bit limbs through
  `__builtin_add_overflow` or through a 64-bit sum among them) and 9 of
  which are one and are left whole.
* **A trip with more than its limb** (`h += t`) is not split, which is
  right as long as the copies would take the carry out for every limb
  (table above). Keeping the chain there needs the other work moved out of
  the chain's way.
* **A carry parked in a frame that has an address taken** is put back with
  `bt` as before: `frame_is_private` does not follow the stack's depth and
  refuses any function with a `lea` of its frame.
* **Registers in a split loop.** Each copy's sum gets a register of its
  own where one would do (`%rax` is free between two limbs), which costs
  pushes in the functions that have more alive across the loop.
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
| `tests/regression/check_carry_chain_shape.py` | The assembly of those functions: the number of `adc`/`sbb` in each, exactly; no `setc`/`bt` inside a straight chain; no more instructions than today. 88 functions, 67 with a chain, 150 links; and for the 14 limb loops, four `adc`/`sbb` in a row in straight-line code, exactly. With `carry` off 58 of the pins break and with `limb_unroll` off all 14 of the trips, which are the negative controls. |
| `tests/regression/check_carry_chain_fuzz.py` | Random adders (see the file's header): programs of ten functions assembled from the pieces above and from pieces that make them *not* adders, compared with GCC's program line by line through every optimization level and with the pass, the peephole and register allocation off in turn. One planted wrong carry as the negative control. CI runs eight programs (seeds 9 to 16) and the fixture through seven configurations. The window was chosen by breaking the compiler: with the compare behind a store allowed to name memory, seeds 13 and 14 differ from GCC (and the fixture reports 9,214 failures); with the registers it names allowed to be written in between, seeds 10 and 16 do. Run by hand through all fourteen configurations without a difference: 2,400 functions of the generator as it was before the accumulator forms (seeds 1 to 240), and 240 of the present one (seeds 1 to 24), whose programs GCC at `-O0`, GCC at `-O2` and Clang with every undefined-behaviour and integer check trapping print alike (seeds 1 to 40). |
| `tests/regression/check_carry_chain_fuzz.py --limbs` | Random limb loops: programs of eight loops, each one adder a trip in one of fourteen spellings of a counted loop (up, down, `while (m)`, `do ... while (--m)`, from a start that may be behind the limit, to a constant, through an index, through stepped pointers or both, with the counter, the pointers and the last limb read behind the loop). Every other loop is plain, an adder the optimizer knows and nothing else in the trip; the rest are any spelling, now and then with a sum, a call or an edited carry in the trip. Compared with GCC's program through the seven configurations and with the split off; one planted wrong carry as the negative control; and a floor under the share of the loops that are split (26 of the 64 of seeds 9 to 16, in each of the ten loop spellings the pass takes). `--census` lists the loops. |
| `src/passes/carry_chain.rs` | 22 unit tests: each spelling, and each refusal. |
| `src/passes/limb_unroll_tests.rs` | 16 unit tests. The loops are run on a machine for the IR, as they were and split: every shape (top and bottom tested, index and pointers, up and down, every test, 32 and 64 bits, signed and unsigned) over the counts from nothing to three trips and a remainder and the values where a counter wraps, with the result apart, in place and one limb above an operand; the two runs agree load by load and store by store. And each refusal. |
| `src/backend/x86/codegen/alu_carry_tests.rs` | The emitter, executed on a model: every home of `x`, `y` and `c`. |
| `peephole/passes/multiword_tests.rs` | The flag forwarding (from a register and from a slot of the frame), the unread carry and the carry taken out in place, executed before and after on a model of the registers, memory and CF; their refusals. |
| `src/backend/live_range.rs` | A value that is dead from a loop's header to its latch is no span of that loop; alive at either, or without hole data, it is one. What a copy carries round a loop: the phi, the value copied back and up to three definitions between them, not through another phi, and a web with any of its members; the floor takes the next cheapest register instead. |
| `tests/regression/check_loop_carried_homes.py` | The two programs the span rule alone made slower, at `-O2`: the state of `zstd_count`'s generator is not stored to the frame where it is computed and not loaded from it on the way back to the multiplication; `csv_field_sum`'s digit loop has no register move and ten instructions. The matchers first refuse the code that had the defects, kept as five witnesses; the head as it was pushed without the two changes fails with five findings. |
| `src/backend/regalloc.rs` | A temporary in a phi's register takes the register the back-edge source leaves, also with a value around the loop in it; not when it begins or ends where that piece of the source's life does, not for a source in a copy group or in a pair applied before, not into a register of neither pool; and the veto of `CCC_PHI_COALESCE_SKIP` moves nothing. Each condition fails its test when taken out (twelve mutants). |

`CCC_DISABLE_PASSES=carry` switches the carry pass off (and the split with
it), `CCC_DISABLE_PASSES=limb_unroll` the split alone,
`CCC_PEEPHOLE_SKIP=multiword` the peepholes (`widen_carry` that one alone);
`CCC_DUMP_EACH_PASS=1` prints the IR "before carry", "after carry" and
"after iter=0 limb_unroll".
