# Scalar predicate folds: differences, intervals, membership, sign magnitude

This is the code half of the 2026-10-31 re-inventory. The audit itself — 151
reports, one verdict each, and the eight gaps it measured — lives in
[AUDIT-2026-10-31.md](AUDIT-2026-10-31.md). This document covers what the
compiler now does differently, why each rewrite is admissible, and what the
three oracles say.

Everything here was driven by measurement, not by taste: every rewrite was
first reproduced as a three-compiler instruction-count gap on x86-64 at `-O2`,
then implemented, then re-measured against the same three compilers.

**Round 3 (2026-11-01)** adds the two rewrites §10 and §11 describe: comparison
dominance (`(x != 0) & (x > 4)` is `x > 4`) and the truncated complement
(`trunc32(not64(sext64 v))` is `~v`). Both were reproduced as three-compiler
gaps on a 53-idiom scalar probe (`/home/user/probe3/probe.c` in the session that
wrote them; the shapes are reproduced in the gate scripts), and both are
measured on the same 2,791-file sweep as the earlier rounds.

**Round 2 (2026-11-01)** closed two of the items this document listed as "not
done" — the lowest-set-bit matcher and a ring-correct interval width that lets
windows crossing zero fold — and added the `0 - x` negation canonicalisation
they depend on, plus a truncated-negate fold (§9) that removes the last
extension/truncation round trip around a 32-bit negation. Sections 6–9 below
are new; the measurements table and the "What is not done" list are updated in
place.

## Names

The passes are named for what they do, not for when they were written. Nothing
in the tree is called a batch:

| Artifact | Name | What it is |
| --- | --- | --- |
| pass (`src/passes/difference_compare.rs`) | `diffcmp` | `(a - b) <op> 0` → `a <op> b` |
| pass (`src/passes/interval_compare.rs`) | `interval` | range and membership tests → one unsigned compare |
| pass (`src/passes/sign_magnitude.rs`) | `signmag` | `|x|` and `|a - b|` → mask, XOR, subtract |
| matcher (`bit_idioms.rs`) | `fold_shift_pair_into_mask` | `(x << k) >>u k` → `x & (2^(W-k)-1)` |
| matcher (`bit_idioms.rs`) | `fold_unit_interval_test` | `(x == 0) \| (x == 1)` → `x <=u 1` |
| matcher (`bit_idioms_compare_dominance.rs`) | `fold_compare_dominance` | `(x != 0) & (x > 4)` → `x > 4`; a join of two comparisons of one value is its stronger (or weaker) operand |
| matcher (`bit_idioms_bit_select.rs`) | `fold_bit_select` | `x \| 1 << n` → `BitSet`, `x & ~(1 << n)` → `BitClear`, `x ^ 1 << n` → `BitToggle`, `(x & 1 << n) != 0` → `BitTest`, and `(x ^ m) ^ m` → `x` |
| matcher (`bit_idioms.rs`) | `fold_lowest_set_bit` | `1 << Ctz(~x)` → `(x + 1) & ~x`, plus the exact `~x == 0` arm |
| rule (`simplify.rs`) | `zeroneg` | `0 - x` → `Neg x` (the modular negate, canonically spelled) |
| rule (`simplify.rs`) | `negcast` | `(i32)(-(i64)(i32)v)` → `-(i32)v` (a truncation distributes over a negate) |
| rule (`simplify.rs`) | `notcast` | `(i32)(~(i64)(i32)v)` → `~(i32)v` (a truncation distributes over a complement) |
| tests (`bit_idioms_mask_interval_tests.rs`) | — | production-matcher tests for the two matchers |
| gate (`check_predicate_differences_and_ranges.py`) | `predicate-differences-and-ranges` | the three passes |
| gate (`check_mask_and_interval_folds.py`) | `mask-and-interval-folds` | the two matchers |
| gate (`check_lowest_set_bit_fold.sh`) | `lowest-set-bit-fold` | the lowest-set-bit matcher and `zeroneg` |
| gate (`check_narrow_negate_fold.sh`) | `narrow-negate-fold` | `zeroneg`, `negcast` and `notcast`, with the untouched widths pinned as controls |
| gate (`check_compare_dominance_fold.sh`) | `compare-dominance-fold` | `fold_compare_dominance`, both directions, with the two non-dominant shapes pinned as controls |
| gate (`check_bit_select_fold.sh`) | `bit-select-fold` | `fold_bit_select` on x86-64/i686/AArch64/RISC-V, with the non-single-bit shapes pinned as controls |
| tests (`bit_idioms_lowest_bit_tests.rs`) | — | executes the rewritten block against the reference semantics |
| tests (`bit_idioms_compare_dominance_tests.rs`) | — | evaluates the join and the returned `Copy` over the whole 8-/16-bit ring |
| tests (`bit_idioms_bit_select_tests.rs`) | — | evaluates the original shift shape and the returned bit operation over every ring |

## The five rewrites

### 1. Difference compare — `diffcmp`

```c
int f(int a, int b) { return (a - b) > 0; }        /* 5 -> 4 instructions */
```

`(a - b) <op> 0` reads the two subtrahends directly; the subtraction then has no
reader and DCE retires it. The rule is typed because modular subtraction is not
order-preserving:

* `Eq`/`Ne` fold at every width and signedness — a difference is zero exactly
  when the operands are equal.
* Unsigned `(a - b) >u 0` ↔ `a != b` and `(a - b) <=u 0` ↔ `a == b`; every other
  unsigned order form is refused, because wrapping turns "below" into "far
  above".
* Signed order (`Slt`/`Sle`/`Sgt`/`Sge`) is admitted only when the module's pass
  policy says signed overflow is undefined (`wrap_signed_overflow == false`, the
  C default). A module built with wrapping signed arithmetic keeps the
  subtraction — the test suite pins both branches.

The subtraction must have exactly one reader, the comparison. Its flags are
free in the original shape (`subl` sets them, `setg` reads them), so folding a
*shared* difference inserts a comparison where the backend had none. Measured
before the guard: +13 instructions in `div_mod_by_const_exhaustive.c` and +5 in
`sign_test_cmov.c`. With the guard both files are exactly as they were.

### 2. Interval and membership predicates — `interval`

```c
int      f(int x)      { return (x > 3) & (x < 10); }   /* 9 -> 5 */
int      g(int x)      { return x > 3 && x < 10; }      /* 7 -> 5 */
int      h(unsigned x) { return x == 0 || x == 1; }      /* 7 -> 4 */
unsigned k(unsigned x) { return (x < 4) | (x > 9); }    /* 9 -> 5 */
```

Four shapes collapse into one unsigned compare against a wrapped difference:

```text
(x >= L) & (x <= U)   ==   (uW)(x - L) <  U - L + 1
(x <  L) | (x >  U)   ==   (uW)(x - L) >  U - L
(x == A) | (x == A+1) ==   (uW)(x - A) <  2
```

The wrap is the trick: a value below the interval becomes a huge unsigned
number, so one compare covers both bounds. A window of `2^31` or more destroys
that argument and is refused, as is an empty window, a mixed `==`/`!=` pair, a
non-consecutive membership set (`{0, 2}` keeps both compares) and two predicates
on different values.

Both this pass and `diffcmp` run **late** — after every vectorizer and after
`range_check` — and that placement is a measurement, not a preference. Run
early, the interval fold took apart exactly the shapes the passes downstream
want: an `and` of two compares is what the vectorizers' mask-fusion matchers
recognize. Over the 3,016-file test corpus the early placement cost **141
instructions in `vec_range_mask_fusion.c`**, 19 in `range_fold_branch.c`, 6 in
`expat_name_scan_kernel.c` and 5 in `range_check_fold.c`, while the late
placement keeps every one of those files at its original size *and* keeps the
scalar wins below.

Two further details come straight out of that sweep. A window whose lower bound
is zero emits no subtraction at all — the first version emitted `subl $0` and
paid one instruction per folded test. And the if-converted `&&`/`||` spelling
only exists after if-conversion, so the late placement is also the earliest
point at which those shapes are visible at all.

### 3. Sign magnitude — `signmag`

```c
int f(int a, int b) { return a > b ? a - b : b - a; }   /* 8 -> 7 */
int g(int x)        { return x < 0 ? -x : x; }          /* 6 -> 6, branchless */
```

```text
s = x >>s (W-1);  |x| = (x ^ s) - s
d = a - b;        s = d >>s (W-1);  |a - b| = (d ^ s) - s
```

Both identities are exact in the ring — `Neg` of the type minimum wraps to the
type minimum and the identity agrees there — so no undefined-behaviour argument
is needed. Lowered this way the abs-difference select loses its conditional move
and matches GCC's count.

`|x|` itself is **count-neutral** (6 → 6): the three-instruction identity costs
the same as LCCC's conditional-move lowering because the return-value copy is
still needed. GCC reaches four instructions by reusing the negation's own flags
in the conditional move — an instruction-selection gap, tracked here rather than
claimed as a win. The gate reports it as a note, not as a pass.

### 4. Shift pair → mask — `fold_shift_pair_into_mask`

```c
uint32_t f(uint32_t x) { return (x << 3) >> 3; }        /* 4 -> 3 */
```

The surviving mask must fit an unsigned 32-bit immediate. Wider masks need a
materialized constant: `(x << 1) >>u 1` on `uint64_t` became `movabsq $0x7fff…;
andq` — four instructions and ten bytes of constant where the pair was four and
eight, and GCC emits `movq; btrq $63`. A 3,016-file asm sweep found that case in
`clang-c/C2y/n3369_3.c`; the guard now refuses it.

### 5. Unit-interval membership — `fold_unit_interval_test`

```c
int f(unsigned x) { return (x == 0) | (x == 1); }        /* 9 -> 4 */
```

`(x == 0) | (x == 1)` is `x <=u 1` and `(x != 0) & (x != 1)` is `x >u 1`. The
`{0, 2}` control keeps both compares.

### 6. Lowest-set-bit isolation — `fold_lowest_set_bit`

```c
unsigned      f(unsigned x)           { return 1u << __builtin_ctz(~x); }     /* 9 -> 8 */
unsigned long long g(unsigned long long x) { return 1ull << __builtin_ctzll(~x); } /* 12 -> 8 */
```

The shift count is masked modulo the operand width — every backend emits a bare
`shl %cl`, and `fold_binop` evaluates the same way — so the expression is
exactly

```text
1 << (ctz(~x) mod W) == ~x & -~x == (x + 1) & ~x     for ~x != 0
                     == 1                            for ~x == 0
```

and `-~x IS x + 1`, so the negate folds into the increment and only an `add`
and an `and` remain.

**The zero arm is not decoration.** At `~x == 0` the masked count wraps to
`1 << 0`, so the answer is `1`, while `(x + 1) & ~x` is `0`. Dropping the arm
would be a miscompile that the C source cannot show (`__builtin_ctz(0)` is
undefined behaviour) but that the IR can: `Ctz(0) == width` is part of its
contract, and `Shl` masks its count, so `1 << Ctz(0)` is well defined *there*.
One `cmov` restores the case on the flags the `and` already set — `and` clears
ZF exactly when the isolated bit is absent — so the sequence stays branch-free
and needs no test of its own.

The x86-64 win is mostly the 64-bit zero fixup: a 64-bit `Ctz` costs
`test/jnz/mov $64/jmp/bsfq` plus the count staging and the shift, while the
isolated-bit form is `not/add`, `and`, `mov $1`, `cmov` at both widths. The
remaining two instructions against GCC (8 versus 6) are register-allocation
copies, not fold work: the same shape written by hand in C
(`unsigned y = ~x, m = y & -y; return m ? m : 1u;`) also compiles to six.

**The plain `1 << Ctz(y)` is deliberately left alone.** Measured, the count path
is 5 instructions at 32 bits and 6 at 64 (the 32-bit zero fixup is the
`mov $32,%ecx` that `bsf` leaves untouched), while `y & -y` plus its zero arm is
8 at both widths. Recognising it would be a pessimisation, so the matcher
requires the complement, and `check_lowest_set_bit_fold.sh` pins the plain form's
instruction counts (≤5, ≤6) so a future change cannot quietly regress it. Clang
reaches 4 on both widths — it applies `y & -y` with no zero arm, which is only
sound because `ctz(0)` is undefined in C; the IR `Ctz(0) == width` is not, so the
arm cannot be dropped here. With
BMI1 available the isolated bit is a single `blsi`; that selection belongs in the
backend and is the reason to revisit the general form later.

### 7. Constant-zero negation — `zeroneg`

```c
unsigned f(unsigned x) { return 0u - x; }    /* 4 -> 3 */
```

`0 - x` is the modular negate. The identity is trivial; the spelling is not.
`Sub` has to materialise the zero (`xorl; subq`) while `Neg` is one `negl`, and
every consumer that understands a negation — the `x & -x` recognition above,
`try_cancel_add_sub`, the `bit_idioms` count-source rules, the backends' negate
and flag folds — only speaks `Neg`. Source negation already arrives as `Neg`;
this is the spelling that macro-expanded and unsigned code uses.

It is not applied to floats (`0.0 - x` is `+0.0` at `x = +0.0`, and `-x` is
`-0.0`), and `0 - 0` still folds to the constant first. The narrow-type case is
the interesting one in practice: the packed-mask kernels build masks as
`(u16)(0 - (u16)(a < b))`, and the `Neg` spelling lets the backend emit one
`negw` where the subtraction needed a full-width `xorl; subq` plus the reloads
that the fixed accumulator forces — 297 instructions off `packed_mask_tree_16_256`
and 499 off `packed_mask_shifts.c` as a whole, with the packed-mask gate green in
all 15 mode/level combinations (`correct: true`).

### 8. Domain-ordered interval width — `interval`

```c
int f(int x) { return x > -8 && x < 8; }      /* 7 -> 5, GCC 5 */
```

`interval` stores its bounds the way `predicate` normalised them: masked into the
operand ring. A window that starts below zero therefore stores its lower bound
*above* its upper one (`[-7, 7]` at 32 bits is `[0xFFFF_FFF9, 7]`), and the
"empty window" guard — a plain 64-bit `lower > upper` — read every crossing
window as ~2^64 values wide and refused it. That is why `x > 3 && x < 10` folded
and `x > -8 && x < 8` did not: same pass, same shape, one negative constant.

`Interval::width` now counts the span **in the comparison's own domain** —
sign-extended bounds for a signed window, plain unsigned for an unsigned one.
That is the whole subtlety, and getting it half right is a miscompile rather
than a missed fold: the signed window `[-7, 7]` and the unsigned predicate
`x >= 0xFFFF_FFF7u && x <= 10u` store the *same* masked bound pair, yet the
first has 15 values and the second has none. Counting the span around the ring
fabricates a 20-value window that accepts both `0..10` and
`0xFFFF_FFF7..0xFFFF_FFFF` — the phantom window `range_fold_branch.c` A11
reproduces live, at u32 and u64 width, which the SSA corpus caught on this
branch before it landed. A signed interval is contiguous in the ring, so
`emit`'s `(x - L) < width` stays exact; an unsigned inverted pair is empty and
`emit` refuses it. All eight mixed-strictness combinations of
`> -8 / >= -7 / < 8 / <= 7` fold to the same biased compare
(`leal 7(%rdi); cmpl $15; setb`), and the gate's brute-force sweep — which
previously skipped every such window by the old guard — now asserts them.

### 9. Truncated negate — `negcast`

```c
int f(int x) { return -(x != 0); }        /* 7 -> 6 */
```

The C integer promotions carry a 32-bit negation at 64 bits:
`trunc32(neg64(sext64(cmp))`. Negation is a ring operation and the extension's
high bits die in the truncation, so the whole expression is `-(i32)cmp` and
both the extension and the second copy disappear:

```text
before   testl %edi,%edi; setne %sil; movzbl %sil,%esi; movslq %esi,%rdx;
         negq %rdx; movl %edx,%eax
after    testl %edi,%edi; setne %sil; movzbl %sil,%esi; negl %esi;
         movl %esi,%eax                                    (GCC: 3, via sbb)
```

The fold is restricted to a 64-bit source and a 32-bit destination, because
that is the only width pair where it measured as a pure win. The two shapes it
does not touch were measured too, and both were net pessimisations over the
2,791-file sweep:

* a **32-bit source** (`trunc16(neg32(ext32(v)))`, the packed-mask idiom
  `(u16)(0 - (u16)(a < b))`) removes two instructions per site but flipped the
  vectorizer's decision in `packed_mask_shifts.c` to a worse 128-bit form: +60
  instructions across four functions against -1 or -2 at each scalar site;
* a **16-bit destination** leaves a partial-register negate whose consumers
  re-extend it, and register allocation paid for it (`bb_slp_unary_demote.c`
  +3, `sib_index_narrow_sext.c` +1, both spills rather than folds).

Both are pinned as controls in `check_narrow_negate_fold.sh`: their instruction
counts must not move, so widening the fold later has to arrive with a fresh
measurement instead of slipping in. The same gate records the one site where
`zeroneg` costs an instruction (`x & (u16)(0 - x)`: 5 with the subtraction,
6 with the narrow negate, because the 16-bit consumers mask it) rather than
hiding it — see "What is not done".

### 10. Comparison dominance — `fold_compare_dominance`

```c
int f1(int x)       { return (x != 0) & (x > 4); }    /* 9 -> 4 */
int f2(int x)       { return (x > 0)  & (x > 99); }   /* 9 -> 4 */
unsigned f3(unsigned x) { return (x < 10) & (x < 5); } /* 9 -> 4 */
int f4(int x)       { return (x != 0) | (x > 4); }    /* 9 -> 4 */
```

Two comparisons of the same value joined by `&` or `|` are how C spells a range
guard (`if (n && n > 4)`, `p && p->len > 8` once `p` has been proved non-null,
`assert(i < n && i < max)`). If one comparison's value set contains the other's,
the join collapses to one of its own operands: `A & B` is `B` when `B` implies
`A`, `A | B` is `A` under the same implication. Both oracles fold all four
shapes to four instructions; LCCC emitted the second compare, its
materialization, the logic op and the joining move (nine).

```text
before   testl %edi,%edi; setne %sil; movzbl %sil,%esi; cmpl $4,%edi;
         setg %dl; movzbl %dl,%edx; andl %edx,%esi; movl %esi,%eax
after    xorl %eax,%eax; cmpl $4,%edi; setg %al
```

Three properties keep it cheap to trust:

* **The replacement copies an operand of the join verbatim.** No compare is
  rebuilt, no type is re-derived and no boolean carrier is chosen: `(x != 0) &
  (x > 4)` becomes a `Copy` of the `x > 4` value, whatever its width and
  whatever casts sit around it. A wrong answer is therefore impossible unless
  the implication itself is wrong.
* **The implication is decided in the comparison's own ordering, exactly.** A
  compare denotes an interval, a singleton (`==`) or a co-singleton (`!=`) over
  the ordering it was written in; containment is decided over `i128` with
  sentinel infinities, with a sign- or zero-extension of the ring constant. Two
  ordering comparisons are only combined when they agree on the ordering
  (`x < 5` and `x <u 5` are refused); `Eq`/`Ne` participate in either. Sets that
  are empty *in their own ring* (`u32 x < 0`) are simply unmatched — in `i128`
  they look non-empty, so the fold misses and never guesses.
* **Both operands must be comparison results**, so `&`/`|` really are the
  Boolean conjunction/disjunction; a bitwise join of wider values is refused.

The first draft attributed `(x != 0) & (x > 4)` to `x != 0` — it kept the
operand that is implied rather than the one that implies. The instruction
counts fell the same way in both directions (9 → 4), so the matcher tests do not
assert an operand: they evaluate the ORIGINAL pair and the returned `Copy` over
every value of the 8- and 16-bit rings and compare the answers. The sweep is
what the first draft failed.

### 11. Truncated complement — `notcast`

```c
int f(int x, int n) { return x & ~(1 << n); }   /* 9 -> 8 */
```

The C promotions carry a 32-bit complement at 64 bits in exactly the shapes the
negate of §9 appears in: `~(1 << n)` reaches the backend as
`trunc32(not64(sext64(shl 1, n)))`, so the bit-clear pays `cltq; notq` where
`notl` is the whole complement. `Not` is bitwise, so it commutes with truncation
outright — the low bits of `~ext(v)` ARE `~v` — and the fold needs no width pair
restriction and no signedness argument, unlike the negate. What it does need is
a NARROWING outer cast: `ext(~v)` and `~ext(v)` differ in the high bits, so a
widening step cannot be dropped, and the two controls in
`check_narrow_negate_fold.sh` (`~(unsigned long)x`, and the unsigned spelling
that never widened) pin that.

### 12. Single-bit selection — `bit_select`

```c
unsigned set(unsigned x, unsigned n)   { return x | (1u << n); }        /* 6 -> 3 */
unsigned clr(unsigned x, unsigned n)   { return x & ~(1u << n); }       /* 8 -> 3 */
unsigned tog(unsigned x, unsigned n)   { return x ^ (1u << n); }        /* 6 -> 3 */
int      test(unsigned x, unsigned n)  { return (x & (1u << n)) != 0; } /* 7 -> 4 */
```

§11's shape ends in a truncation, and its last paragraph named the remaining gap
as instruction selection. It was: `x | 1u << n` reaches the backend as a
materialized constant, a variable shift pinned to `%cl`, and the logic op, and
no IR operation said "set bit n". `IrBinOp` now has `BitSet`, `BitClear` and
`BitToggle` next to the existing `BitTest`, all four with the same contract —
the index is taken **modulo the operand width**, so an index of W or more (an
undefined C shift that may sit in dead code) still compiles and still agrees
with the constant folder, and they are typed I32/U32/I64/U64 with the wide-int
pair path refusing 64-bit operations on a 32-bit target. `BitToggle` arrived a
round later than the other two: it is the third *modify* spelling, and the one
whose x86 mnemonic (`btc`) the first round left on the table.

`fold_bit_select` creates them, and it only ever replaces the instruction it
matched: the shift (and the complement above it) must be single-use, so DCE
deletes them and nothing needs the pass's producer-repurposing bookkeeping. All
four shapes are accepted in both operand orders, the complement may be spelled
`Not` or `Xor -1`, and the index is read *through* integer casts — `index mod W`
survives any of them (a narrowing cast drops bits that are multiples of 2^8, a
sign-extending one adds 2^(8k-1), and every W here divides those), which is what
removes the count's widened copy (`movslq %esi, %rdx`) from the 64-bit forms.

The toggle has one shape the other three do not: its own inverse. `x ^ m ^ m` is
`x`, and the earlier XOR of that pair has usually been folded to a `BitToggle`
before the outer one is reached, so the boolean pass that cancels `a ^ a` would
see two *different* operations. Folding both XORs is arithmetically right and
still a pessimisation — the shape measured 2 instructions before the fold and 5
after — so the matcher folds `BitToggle(BitToggle(x, n), n)` to a **copy of `x`**
and lets DCE finish it: 2 before, 2 after, pinned by the gate. The two masks have
to be the same `n` for that: `x ^ (1u << n) ^ (1u << (n + 1))` is not an identity
and stays two toggles.

The equality direction, `(x & 1 << n) == 0`, is deliberately *not* folded:
`BitTest` is a 0/1 value, so the negated form needs a second instruction on top
of it and the compare would survive anyway; the inequality direction folds
because `BitTest`'s 0/1 result *is* the answer. That is the one row of this
section's probe corpus that stays at its old count, and the gate pins it.

Backend lowerings, all of them measured in this section's probe corpus:

| Target | `BitSet` | `BitClear` | `BitToggle` | Note |
| --- | --- | --- | --- | --- |
| x86-64, i686 | `bts` | `btr` | `btc` | one instruction in place of the base; the register-direct form refuses the shapes where staging the base would destroy the index, and the accumulator form covers the rest (8-bit has no `btsb`/`btcb`, so it reduces the index and uses the 64-bit form) |
| AArch64 | `mov w3, #1` + `lsl` + `orr` | … + `bic` | … + `eor` | three instructions, the same count the shift/OR spelling costs, which is why the toggle's AArch64 counts do not move: the win there is the eliminated shift, not the mnemonic |
| RISC-V | `andi` (index mod W) + `li` + `sll` + `or` | … + `xori` + `and` | … + `xor` | no `bts`/`btr`/`btc` (Zbs), so the portable sequence with the index reduced explicitly |

### 13. Predicate materialization — `predicate`

```c
int f1(int a, int b) { return a != b ? 1 : 0; }   /* 5 -> 4 */
int f2(int a, int b) { return a != b ? 0 : 1; }   /* 5 -> 4 */
unsigned f3(int c)   { return c ? 1u : 0u; }      /* 5 -> 4 */
int f4(int a, int b) { return (a != 0) | (b != 0); }  /* 9 -> 4 */
int f5(int a, int b) { return (a == 0) & (b == 0); }  /* 9 -> 4 */
```

C spells a boolean result two ways, and they are the same program: `return a != b;`
lowers to the comparison itself, which every backend emits as one `setcc` off the
flags the compare already set, while `return a != b ? 1 : 0;` lowers to a `Select`
over the two constants. A `Select` needs its arms in registers, so x86 emitted
`mov $1,%ecx; mov $0,%eax; cmovne` where the comparison alone is
`xor %eax,%eax; setne %al`. Both oracles canonicalize the two spellings to the
same IR (gcc and clang both spend 4 instructions on every row above, before and
after), and LCCC spent 5 -- one instruction per boolean, forever, purely from the
spelling.

`fold_predicate_select` rewrites the select to the comparison it is hiding, and it
never needs a new value:

* `select(c, 1, 0)` where `c` *is* a comparison of the select's type becomes a
  **copy** of it -- the select only copied the boolean through two constants.
  Through a wider carrier (`(int)t` for a `_Bool`-shaped `t`, the shape
  `int_ptr_provenance_alias` stores) it becomes the **cast** that restates the
  carrier: the value is already a boolean, so only its type was ever in question.
* `select(c, 0, 1)` is `!c`. When the comparison behind `c` has this select as its
  only reader the fold answers with that comparison **inverted**, so the negated
  case costs the same as the plain one (no second compare); when it does not, and
  `c` is already a boolean, the select is left alone -- a zero test on top of a
  `setcc` pays for the materialization twice (`setne %dil; movzbl %dil,%edi;
  xorl %eax,%eax; testl %edi,%edi; setne %al`, measured, 5 where the select was 4).
* Any other integer condition gets the zero test it was hiding: `select(c, 1, 0)`
  is `(c != 0)` and `select(c, 0, 1)` is `(c == 0)`, so `c` needs only to be as
  wide as the result. That is the `c ? 1u : 0u` row, where the condition is a
  plain `int` and no comparison exists.

The arms are read **in the select's ring**, because the frontend emits the two
constants at a wider `IrConst` type than the select (`Const(I64(1))` on a `U32`
select, measured) -- refusing that mismatch would refuse every `? 1u : 0u`.
Everything else is refused: arms that are equal (`select(c,1,1)` is `simplify`'s
constant fold, not a comparison), arms outside `{0,1}` (`c ? 3 : 2` has no
comparison form), float and vector results (no `setcc`), and a condition narrower
than the result (it would have to be widened first, which is not free and needs a
value of its own).

`fold_predicate_merge` covers the mask algebra of the same family: a disjunction is
zero exactly when both halves are, so `(a != 0) | (b != 0)` is `(a | b) != 0` and
its De Morgan dual `(a == 0) & (b == 0)` is `(a | b) == 0`. Both replace two
`setcc` selectors and a logic op with one disjunction and one compare (9 -> 4), and
the chain form folds again (`(a != 0) | (b != 0) | (c != 0)` is 13 -> 5). The two
**mixed** forms are refused, and one of them is the reason this rewrite needs a
proof rather than a pattern: `(a != 0) & (b != 0)` is **not** `(a & b) != 0` --
`a = 1, b = 2` shares no set bit but is nonzero twice -- and `(a == 0) | (b == 0)`
is about two separate zeros, which no conjunction expresses. The matcher accepts
only the two laws above, requires both comparisons to have exactly one reader (so
the splice removes two instructions instead of adding one), and requires the
comparisons' carrier to match the join's.

The merge needs one value of its own for the disjunction, and the `bit_idioms` scan
forbids inserting values under coordinates it still has to visit, so the pair is
queued as an `Expansion` and spliced in after the scan (`queue_predicate_merge`),
with `defs`/`use_counts` updated immediately so later matchers see the post-rewrite
truth.

Two pipeline facts were learned by measuring, and both are pinned by the gate:

* **The fold runs only in the post-if-conversion invocation.** The select
  `c ? 1 : 0` is a shape the earlier passes consume: `int t = (x == 4); if (x == 4)
  r += 1; if (x == 4) r += 2;` folds the two `+=` into ONE select with the arm
  constants `3`/`0` (5 instructions), and converting `t` to a comparison before
  that pass has had its turn leaves `t + 2` arithmetic on the side (6 instructions,
  +3 for the function, measured on `signext_move_cross_block.c`).
* **A stored comparison is 0/1; a stored lane mask is 0/-1.** With the select gone,
  `d[i] = lo <= c && c <= hi ? 1 : 0` becomes a comparison feeding the map
  vectorizer, which stores the packed compare -- the raw mask. The loop
  `k_s_classify` in `vec_signed_range_fusion.c` then wrote `0xFFFFFFFF` where the C
  program writes `1`, and it *passed* the corpus sweep (which compares counts, not
  answers) at `-O2`: the sweep only noticed the file got longer. The vectorizer now
  materializes `& 1` for any mask-shaped root value tree, and
  `tests/regression/predicate_materialize_mask_store.c` reproduces the miscompile
  at `-O2` (verified: it fails without the normalization and passes with it). This
  is a pre-existing vectorizer bug that no earlier test reached, because the shape
  needed the select to be gone to parse; the fix is the reason the fold's
  `vec_signed_range_fusion` rows cost +70 static instructions (a 4-lane SIMD body
  and its prologue) and run ~3.3x fewer dynamic instructions.

### 14. Three-way indicator comparisons — `booldiff`

```c
int le(int a, int b) { int t = (a > b) - (a < b); return t <= 0; }  /* 9 -> 4 */
int lt(int a, int b) { int t = (a > b) - (a < b); return t < 0; }   /* 9 -> 4 */
long w(int a, int b) { long t = (a > b) - (a < b); return t <= 0; } /* 10 -> 5 */
int eq(int a, int b) { int t = (a >= b) - (a > b); return t > 0; }  /* 9 -> 4 */
```

§13's fold leaves a comparison where a select used to be, and the next shape in
every inlined comparator is the sign value built out of two of those
comparisons. `(a > b) - (a < b)` is the sign of `a - b`, so *any* `cmp` of it
against zero is a `cmp` of `a` and `b`: the two `setcc`s, the two `movzbl`s, the
subtraction and the second comparison are all one instruction's worth of
question. Clang folds this (4 instructions); GCC 16.2 does not (10), so the
fold ends one instruction behind the best oracle only because of the `xor
%eax,%eax` that materializes the answer.

`fold_cmp_of_boolean_indicators` accepts both spellings, because which one
arrives depends on whether the rest of the pass has already folded the
subtraction away: `indicator <op> indicator` directly, and `diff <op> 0` where
`diff` is their subtraction.

#### The design: enumerate the trichotomy, do not tabulate it

Both indicators are predicates over the same operand pair and the same ordering
class, so the pair `(a, b)` can only be in one of three states -- `a < b`,
`a == b`, `a > b` -- and within one class exactly one holds. A predicate over
that class is therefore exactly a three-row truth table
(`indicator_regions`), the difference `d = first - second` is `-1`, `0` or `1`
per row, and the question -- `d <op> 0`, or `first <op> second` -- evaluates to
one row set of booleans. There are exactly eight such row sets, and every one of
them is one comparison of the operands or a constant:

| rows (`<`, `==`, `>`) | answer | | rows | answer |
| --- | --- | --- | --- | --- |
| `000` | constant `0` | | `100` | `a < b` |
| `111` | constant `1` | | `110` | `a <= b` |
| `010` | `a == b` | | `001` | `a > b` |
| `101` | `a != b` | | `011` | `a >= b` |

`trichotomy_answer` *is* that table, so the mapping is total and cannot mislabel
an answer. The first version of this fold hand-tabulated the same information
instead, and it was wrong: it mirrored the predicate for the reversed operand
order *before* applying the unsigned-collapse rules, so `(a < b) - (a > b) >u 0`
was answered "never true" (`-1` is `UINT_MAX`, so it is true exactly when
`a > b`). Enumerating three rows removes the class of bug, not just the
instance: there is no order of operations to get wrong, and the unsigned
question is evaluated on the value it is about (`-1` as the all-ones pattern)
rather than derived from the signed one.

The enumeration also makes two shapes free that a hand table would have to be
extended for:

* **nested pairs** -- `(a >= b) - (a > b)` is the *equality* indicator, not a
  sign, so `> 0` is `a == b` (9 -> 4 on x86-64, matching both oracles);
* **any predicate pair in the same class** -- `(a == b) - (a < b) <= 0` is
  `a != b`, and the same predicate twice (`(a > b) - (a > b) <= 0`) is a
  constant.

What the enumeration cannot fix is where the *values* come from, and that is the
second half of the design.

#### The casts: which extension is it?

The difference reaches the comparison through whatever casts the C source asked
for, and peeling them is only sound while the signed reading of the
`{-1, 0, 1}` pattern survives. A same-width cast keeps the bits and a truncation
keeps the low bits (so `-1` becomes the all-ones pattern of the narrower type),
but a widening cast is transparent **only when it sign-extends**. The case that
proves it:

```c
int f(int a, int b) { unsigned char d = (a > b) - (a < b); return (int)d < 0; }
```

`d` holds `255` where the difference is negative, and `(int)d` reads `255`, not
`-1` -- so the answer is `0` for every input. A peel that ignores *which*
extension it is folds this to `a < b` and returns `1` whenever `a < b`. That
miscompile was in this round's first implementation and was caught by a
differential probe against GCC 16.2 and Clang 23.1.4 (both returned `0`):

| build | checksum of the probe |
| --- | --- |
| LCCC with the first `booldiff` implementation | 1001456958 |
| LCCC with `CCC_SIMPLIFY_SKIP=booldiff` | 2984546946 |
| GCC 16.2 | 2984546946 |
| Clang 23.1.4 | 2984546946 |

`peel_sign_preserving_casts` now refuses to cross a zero-extension, and
`sign_preserving_peel_stops_at_a_zero_extension` pins all three cases
(truncation, sign-extension, zero-extension). The shape is in the valence test
as `zext_uchar_{lt0,gt0,ule0}`, and re-introducing the unconditional peel makes
that test report `VALIDATION FAILED (30450)` with 20,300 of the failures in the
`zext`/`trunc_uchar` rows.

#### What is refused, and why

* **Mixed ordering classes** -- `(a >s b) - (a <u b)` is not a sign: with
  `a = -1, b = 1` both indicators are false, so the difference is not the sign of
  anything. Refused, and pinned by both the gate and the valence test.
* **Different operand pairs** -- `(a > b) - (c < d)` relates nothing.
* **Floating operands** -- with a NaN both comparisons are false, so the
  difference is `0` and every question about it answers from `0`; no trichotomy
  of the operands can express that. Refused, and pinned with a NaN in the
  valence test.
* **A second reader of the difference** is *not* a refusal any more: the folded
  comparison is never longer than the one it replaces (one `cmp` for one `cmp`),
  so folding one of several readers is a free win and the rest of the chain dies
  with the last reader. The gate asserts the multi-reader shape never gets
  longer rather than that it never moves.

#### The proof

The proof that this fold is right is not the four-instruction count:

* `tests/regression/three_way_boolean_compare.c` checks every predicate over
  every width and signedness against a branch-written reference, on the extrema
  (`INT_MIN`, `INT_MAX`, `ULLONG_MAX`), the values around zero, 40,000 seeded
  pairs and a full ±128 × ±128 sweep -- 231,000 comparisons, plus the eight
  refusal shapes above, NaN included. The reference value is forced through a
  `volatile` before it is questioned, so the *driver* computes the expectation
  and the file cannot agree with its own bug. It passes under LCCC, GCC 16.2 and
  Clang 23.1.4.
* `boolean_indicator_folds_agree_with_the_trichotomy_for_every_predicate_pair`
  is exhaustive where the C file is sampled: all 10 × 10 indicator predicate
  pairs × 10 outer predicates × both operand orders × both spellings = 1,000
  combinations, each checked against an oracle that evaluates the *original* C
  question with `IrCmpOp::eval_i64` on seven value pairs -- including
  `(-1, 1)` and `(1, -1)`, where the signed and unsigned classes disagree. It
  also asserts completeness: a same-class pair must fold, a mixed-class pair
  must not.
* Mutating the `<=` mapping to `>=` makes the C file report
  `VALIDATION FAILED (231000)`; mutating the cast peel makes it report
  `VALIDATION FAILED (30450)`.

#### Measurements

`g = (a > b)`, `l = (a < b)`, so `(g, l)` is `(0, 1)` when `a < b`, `(0, 0)` when
`a == b` and `(1, 0)` when `a > b`, and the raw trichotomy the fold enumerates
reads:

| question | `a < b` | `a == b` | `a > b` | answer |
| --- | --- | --- | --- | --- |
| `g == l` / `g != l` | `0` / `1` | `1` / `0` | `0` / `1` | `a == b` / `a != b` |
| `g < l` | `1` | `0` | `0` | `a < b` |
| `g <= l` | `1` | `1` | `0` | `a >= b` |
| `g > l` | `0` | `0` | `1` | `a > b` |
| `g >= l` | `0` | `1` | `1` | `a <= b` |
| `s = g - l`, `s <= 0` / `s < 0` | `1` / `1` | `1` / `0` | `0` / `0` | `a >= b` / `a < b` |
| `s <=u 0` | `0` | `1` | `0` | `a == b` |
| `s >u 0` | `1` | `0` | `1` | `a != b` |
| `s <u 0` / `s >=u 0` | `0` / `1` | `0` / `1` | `0` / `1` | constant `0` / `1` |

Two entries are worth stating because both are easy to get wrong and the gate
pins them: `g <= l` is `a >= b`, not `a < b` (the indicators being equal is
`a == b`, which satisfies `<=`), and the unsigned rows are not the signed ones
with a different flag -- `-1` is the all-ones value, so `s <u 0` is never true,
`s >=u 0` always is, and the other two collapse onto equality.

Fold off -> on, same binary, `-O2`, on the four targets:

| probe | x86-64 | i686 | AArch64 | RISC-V |
| --- | --- | --- | --- | --- |
| `int t = (a > b) - (a < b); return t <= 0` | 9 -> **4** | 19 -> **9** | 16 -> **10** | 17 -> **9** |
| `... return t < 0` | 9 -> **4** | 19 -> **9** | 16 -> **10** | 17 -> **8** |
| `long t = (a > b) - (a < b); return t <= 0` | 10 -> **5** | 19 -> **9** | 19 -> **13** | 19 -> **11** |
| `int t = (a >= b) - (a > b); return t > 0` (nested) | 9 -> **4** | 19 -> **9** | 16 -> **10** | 17 -> **9** |

On x86-64 the four-instruction answer is `xorl; cmpl; setle; ret` -- equal to
Clang 23.1.4 and six ahead of GCC 16.2. The `xorl` is the materialized boolean
of §13, not a leftover of this fold; the `three_way_val` row keeps one
`movzbl`/`movslq` pair because the result is returned in a 64-bit register. No
target gets longer on any probe, which is the invariant the cross-target gate
section and the sweeps check.

### 15. A zero-extension compared against zero — `simplify_cmp`

```c
int zx_lt0(unsigned char c) { return (int)c < 0; }   /* 4 -> 2, gcc 2, clang 2 */
int zx_le0(unsigned char c) { return (int)c <= 0; }  /* 4 -> 4, but `sete` */
```

A zero-extension is non-negative, so it answers the *signed* comparisons
against zero on its own: `(int)(unsigned char)c` is in `[0, 255]`, so `< 0` is
false, `>= 0` is true, and `<= 0`/`> 0` are `c == 0` and `c != 0` at the narrow
width (the extension is injective, so `zext(x) == 0` iff `x == 0`).
`simplify_cmp` already folded the same four questions for an
unsigned-*typed* operand (`unsigned u; return u < 0;` was two instructions);
it did not look through the extension, so the same question asked about a byte
cost four. Both oracles fold it.

This one is a three-line rule (a widening cast whose source is unsigned), and it
is what the `booldiff` probe below measures when the difference is carried in an
`unsigned char`: the whole indicator chain becomes dead once the comparison is a
constant, so the shape goes from ten instructions to two.

`CCC_SIMPLIFY_SKIP=cmp` attributes it, and the gate asserts both counts
(`zx_lt0` 2 with the rule, 4 without; `zx_le0` 4 either way, because `Eq 0` and
`Ule 0` against a byte both select `testb` plus one `setcc` -- only the
predicate moves). The *values* are covered by `three_way_boolean_compare.c`'s
`zext_uchar_*` shapes, which is also the shape that caught the cast-peel
miscompile in §14.

## What the LLVM tracker already knows about these shapes

The round started from a differential probe, and the gaps it found have
open `llvm-project` issues of their own, which is the useful cross-check: the
shapes are not LCCC-specific.

| issue | shape | status here |
| --- | --- | --- |
| [#228923](https://github.com/llvm/llvm-project/issues/228923) | clang 23 turns an inlined lexicographic comparator into a `setcc`/`and`/`or` chain where clang 22 branched, 18-88% slower in `std::ranges::sort` | **the reason §13 runs after if-conversion**: converting `c ? 1 : 0` earlier left live arithmetic behind, and the `simplify` fix that reads `Cast(cmp)` as the `{1,0}` leaf is what keeps the comparator itself a `cmp`. LCCC now folds the comparator to a single `cmp` (36 instructions in `merge_and_count`, where the pre-fix build had 50 and the fold-off build 36) |
| [#45213](https://github.com/llvm/llvm-project/issues/45213) | division by multiplication moves the numerator into `%rax` although the magic number is the commutative operand | open here too: `div_const` is 9 (GCC 8, Clang 5). A copy-coalescing fix, not a fold |
| [#33226](https://github.com/llvm/llvm-project/issues/33226) | comparison of a shift against zero keeps a redundant copy | open here: `int f(unsigned i, unsigned j, int a, int b) { if (0 == (i >> j)) return a; return b; }` is 9 (GCC 6; the `test` itself is already optimal -- the cost is a register copy and the duplicated `ret`) |
| [#205978](https://github.com/llvm/llvm-project/issues/205978) | `cmovne` re-reads memory when the value is already in a register (a WAR hazard in the scheduler) | not observed here: LCCC's aggregate-return lowering spills instead of re-reading (`struct a` return of `d[1]` writes through the stack), which is a different -- and separately documented -- cost |
| [#197671](https://github.com/llvm/llvm-project/issues/197671) | unnecessary comparison plus a duplicate addition for `__builtin_add_overflow` on AArch64 | open here on x86-64 as well: `add_ovf` is 8 (both oracles 4) |

## The sweep behind it

This round began as a measurement, not a fold: a 76-function differential probe
against gcc 16.2.0 and clang 23.1.4 at `-O2` over the scalar-predicate idioms the
project has not covered yet, to find where LCCC still loses. Counted inside the
function body, worst first (LCCC before this round / gcc / clang):

| function | LCCC | gcc | clang | status |
| --- | --- | --- | --- | --- |
| `pop32` (`__builtin_popcount`) | 17 | 5 | 16 | deferred: linking policy (see *What is not done*) |
| `par32` | 18 | 7 | 7 | deferred: same |
| `next_pow2` | 19 | 18 | 18 | no gap |
| `div_const` (unsigned magic division) | 9 | 8 | 5 | open: magic-number selection |
| `avg_u` (`(a & b) + ((a ^ b) >> 1)`) | 9 | 9 | 5 | open: instruction selection |
| `or_or` (`(a != 0) \| (b != 0)`) | 9 | 4 | 4 | **fixed here** |
| `and_and` (`(a != 0) & (b != 0)`) | 9 | 7 | 7 | refused: not a rewrite |
| `add_ovf` (`__builtin_add_overflow`) | 8 | 4 | 4 | open: same-width `seto` path |
| `sign_of` (`(x > 0) - (x < 0)`) | 8 | 6 | 6 | open: `sbb` form |
| `seclect`/`cond_neg`/`neg_sel` | 6-7 | 5 | 5 | open: `abs` selection |
| `align_up` | 7 | 4 | 5 | open: `neg`+`and` spelling |
| `and03`/`low_byte` (`x & 0xff` stores) | 4/3 | 3/2 | 3/2 | open: byte-extract selection |
| `sat_add` | 5 | 4 | 4 | open: `cmovae` form |
| `bool_ne`/`bool_le`/`zero_or_one`/`one_or_zero`/`max_mask`/`bool_from_cmp` | 5 | 4 | 4 | **fixed here** |
| `add_bytes` (`(u8)(a + b)`) | 5 | 2 | 2 | open: narrow ALU lattice |
| `absdiff`/`iabs`/`clamp` | 6-9 | 4-7 | 4-7 | open: `mov`+`neg`+`cmovs` |

The two rows this round closes are the boolean-materialization family, because it
is the one with a law behind it rather than an instruction-selection target: every
row above is a *different* gap, and the rest are listed here so the next round
starts from data. The wins that were already LCCC's stay wins (`sext_mul` 3 vs 4,
`rd32` 3 vs gcc 11 / clang 2, `clz32`/`ctz32` ahead of gcc).

## Measurements

x86-64, `-O2`, instructions counted inside each function body including `ret`.
GCC 16.2.0 and Clang 23.1.4 are the installed oracles; ICX 2026.2 is not
installed in this sandbox, so its column is absent rather than guessed.

| Shape | Before | After | GCC 16.2 | Clang 23.1.4 | Verdict |
| --- | --- | --- | --- | --- | --- |
| `(a - b) > 0` (int) | 5 | **4** | 4 | 4 | matches |
| `(a - b) >= 0` (int) | 5 | **4** | 4 | 4 | matches |
| `(a - b) == 0` (int) | 4 | 4 | 4 | 4 | already folded by `simplify` |
| `(x > 3) & (x < 10)` (int) | 9 | **5** | 5 | 5 | matches |
| `x > 3 && x < 10` (int) | 7 | **5** | 5 | 5 | matches |
| `x >= 4 && x <= 9` | 5 | 5 | 5 | 5 | already folded elsewhere |
| `x == 0 \|\| x == 1` (unsigned) | 7 | **4** | 4 | 4 | matches |
| `(x < 4) \| (x > 9)` (unsigned) | 9 | **5** | 5 | 5 | matches |
| `a > b ? a - b : b - a` (int) | 8 | **7** | 7 | 5 | matches GCC |
| `x < 0 ? -x : x` (int) | 6 | 6 | 4 | 4 | count-neutral; selection gap |
| `(x << 3) >> 3` (uint32) | 4 | **3** | 3 | 3 | matches |
| `1u << __builtin_ctz(~x)` | 9 | **8** | 6 | 4 | round 2; the rest is allocation |
| `1ull << __builtin_ctzll(~x)` | 12 | **8** | 6 | 4 | round 2, −33 % |
| `1u << __builtin_ctz(y)` (plain) | 5 | 5 | 5 | 4 | refused on purpose (see §6) |
| `1ull << __builtin_ctzll(y)` (plain) | 6 | 6 | 5 | 4 | refused on purpose (see §6) |
| `0u - x` / `0ull - x` | 4 | **3** | 3 | 3 | round 2 (`zeroneg`), now level |
| `x & (0u - x)` | 5 | 5 | 4 | 4 | count-neutral; the copy is allocation |
| `x > -8 && x < 8` (crossing window) | 7 | **5** | 5 | 5 | round 2, ring-correct width |
| `x > -8 && x <= 7` / `x >= -7 && x < 8` | 7 | **5** | 5 | 5 | round 2, all strictness mixes |
| `-(x != 0)` (int) | 7 | **6** | 3 | 4 | round 2 (`negcast`); the rest is the missing `neg`/`sbb` idiom |
| `(unsigned short)-x` | 4 | 4 | 3 | 3 | unchanged: the 32 → 16 fold measured as a loss |
| `(x != 0) & (x > 4)` | 9 | **4** | 4 | 4 | round 3 (`fold_compare_dominance`), now level |
| `(x != 0) & (x >= 7)` unsigned | 9 | **4** | 4 | 4 | round 3, now level |
| `(x > 0) & (x > 99)` | 9 | **4** | 4 | 4 | round 3, now level |
| `(x < 10) & (x < 5)` unsigned | 9 | **4** | 4 | 4 | round 3, now level |
| `(x != 0) \| (x > 4)` | 9 | **4** | 4 | 4 | round 3: the disjunction keeps the guard |
| `x & ~(1 << n)` | 9 | **8** | 3 | 3 | round 3 (`notcast`); the rest is `btr` selection |
| `x & ~(1u << n)` unsigned | 8 | 8 | 3 | 3 | never widened; the rest is `btr` selection |
| `~(unsigned long)x` | 4 | 4 | 3 | 3 | control: a widening cast keeps the 64-bit complement |
| `unsigned char c; return (int)c < 0` | 4 | **2** | 2 | 2 | round 6 (`cmp`): the constant `0` |
| `unsigned char c; return (int)c <= 0` | 4 | 4 | 4 | 4 | round 6: count-neutral, `sete` instead of `setbe` |
| `int t = (a > b) - (a < b); return t <= 0` | 9 | **4** | 10 | 4 | round 6 (`booldiff`): level with Clang, six ahead of GCC |
| `... return t < 0` | 9 | **4** | 10 | 4 | round 6, same answer from the same `cmp` |
| `long t = (a > b) - (a < b); return t <= 0` | 10 | **5** | 10 | 4 | round 6: the `movzbl`/`movslq` pair is the 64-bit return |
| `(a > b) <= (a < b)` | 9 | **4** | 4 | 4 | round 6, the form that survives once the `Sub` folds away |
| `int t = ...; return (unsigned)t <= 0u` | 10 | **4** | 4 | 4 | round 6: collapses onto `==`, not onto an ordering |
| `int t = ...; return (unsigned)t > 0u` | 10 | **4** | 4 | 4 | round 6: collapses onto `!=` |
| `((a >= b) - (a > b)) > 0` | 9 | **4** | 4 | 4 | round 6: the nested pair is the equality indicator, so this is `a == b` |
| `((a > b) - (c < d)) <= 0` | 10 | 10 | 11 | 7 | control: different operand pairs; already ahead of GCC |
| `((a > b) - ((int)((unsigned)a < (unsigned)b))) <= 0` | 9 | 9 | 8 | 6 | control: mixed signedness is not complementary |
| `unsigned char d = (a > b) - (a < b); return (int)d < 0` | 10 | **2** | 2 | 2 | control: a zero-extending carrier -- always false, and now folded as such (§15); this is the shape that caught the first implementation's cast peel |
Honest attribution, measured by re-running with
`CCC_DISABLE_PASSES=diffcmp,interval,signmag`: `diff_gt`/`diff_ge` 5 → 4,
`range_bitand` 9 → 5, `range_short` 7 → 5, `member_short` 7 → 4 and
`absdiff_select` 8 → 7 are these passes' wins. `diff_eq`, `diff_une`,
`range_closed`, `range_unsigned`, `outside_or`, `outside_short` and
`member_bitand` are already folded by `simplify`, `range_check` and
`set_membership`; for those the passes only guarantee "not worse". Five
near-miss controls (`(a - b) > 5`, `(x > 3) | (x > 10)`, `{0, 2}` membership,
two different values, and a select whose arms disagree) keep their original
shape at every level.

Round 2 was attributed the same way, with `CCC_NO_LOWEST_SET_BIT=1
CCC_SIMPLIFY_SKIP=zeroneg,negcast`: the lowest-set-bit matcher owns the
`lsb32`/`lsb64` rows, `zeroneg` owns the packed-mask negates, `negcast` owns the
`-(x != 0)` row (7 with the switch, 6 without), and the interval width fix is not
gated by a knob — it is attributed by the eight crossing-window cases in
`predicate_differences_and_ranges.c`, each of which the gate asserts is a single
compare. The pass-off run leaves `range_crossing_control` (`x > -8 && x < 1`, a
different pair of constants) at two compares, so the gate cannot pass by
accident.

## Verification

| Check | Result |
| --- | --- |
| `cargo test --lib` | **7,515 passed, 0 failed, 10 ignored** after round 6 (+29 in round 4, 4 in round 5, 4 in round 6: the 1,000-combination enumeration oracle, the cast-peel test, the zero-extension rule test and the direct-form spelling test) |
| `check_compare_dominance_fold.sh` | PASS — four dominant shapes in both directions, 8/16/32/64-bit widths, `CCC_NO_COMPARE_DOMINANCE=1` restores every one of them, the two non-dominant shapes pinned at 9 in both builds, `-O0`…`-O3` + the switch + GCC agree on a 65,536-row checksum |
| `check_bit_select_fold.sh` | PASS — set/clear/toggle/test at 8/16/32/64 bits and both operand orders, the dead `shl`/`not`/`xor` producers, `CCC_NO_BIT_SELECT=1` restoring every count, the non-single-bit controls pinned in both builds, never-longer lengths on i686/AArch64/RISC-V, and `-O0`…`-Os` + the switch + GCC agreeing on an executed checksum |
| `bit_idioms_bit_select_tests.rs` | 16 tests — the four shapes over every ring and both operand orders, the `Not`/`Xor -1` complement spellings, the cast-peeled index, the shift-at-another-width refusal, and the toggle involution |
| `bit_idioms_compare_dominance_tests.rs` | 14 tests — exhaustive 8-bit product of all ten comparison kinds × six constants in both joins, a reduced 16-bit product, and the refusal tables (different value, different width, mixed ordering, non-comparison operand, float) |
| `notcast` unit tests | `truncated_complement_of_an_extension_folds_to_the_narrow_complement`, `truncated_complement_controls` (widening cast, different-width extension, already-narrow operand) |
| `check_predicate_differences_and_ranges.py` | PASS — values at `-O0`…`-O3`, per-fold mechanism, five controls, attribution, GCC 16.2 length oracle (1.7 s) |
| `check_lowest_set_bit_fold.sh` | PASS — both widths, both arms of the zero select, plain forms pinned at ≤5/≤6 |
| `check_narrow_negate_fold.sh` | PASS — `negcast`/`notcast` ownership, `zeroneg` shapes, widening/float controls, the untouched widths pinned exactly, `-O0`…`-O3` + all three switches + GCC agree on a 65,536-row checksum |
| `Interval::width` unit tests | crossing window `[-7, 7]` = 15, inverted `[-5, -9]` = 0, `[7, 7]` = 1, unsigned wrapping window (`u32` and `u64`) refused, brute-force sweeps at every signed width |
| `check_mask_and_interval_folds.py` | PASS (0.6 s) |
| Exhaustive matcher tests | 8/16-bit rings brute-forced for interval and membership; 32/64-bit boundary sweeps; refusal tables |
| Corpus A/B, 3,016 `tests/**/*.c` | **27 files shorter, 0 longer, net −147 instructions** with the three passes on versus off |
| `three_way_boolean_compare.c` (`-O2`) | PASS — every predicate over 8 widths, both operand orders and both signedness classes against a branch-written reference whose value is forced through a `volatile`, on the extrema, 40,000 seeded pairs and a ±128 × ±128 sweep, plus the refusal shapes (mixed classes, different pairs, a zero-extending carrier, NaN); `VALIDATION OK` under LCCC, GCC 16.2 and Clang 23.1.4, `VALIDATION FAILED (231000)` when the `<=` mapping is mutated to `>=`, and `VALIDATION FAILED (30450)` when the cast peel is made unconditional |
| `boolean_indicator_folds_agree_with_the_trichotomy_for_every_predicate_pair` | 1,000 combinations — 10 × 10 indicator predicate pairs × 10 outer predicates × both operand orders × both spellings, each checked against an oracle that evaluates the original C question on seven value pairs; also asserts that a same-class pair folds and a mixed-class pair does not |
| `sign_preserving_peel_stops_at_a_zero_extension` | the three cast cases: truncation and sign-extension are peeled, zero-extension is not |
| `zero_extensions_answer_signed_comparisons_against_zero` | eight cases — the four signed questions for `u8 -> i32` and `u16 -> i64`, plus the three refusals (signed source, same-width cast, no cast) |
| `check_predicate_materialize.sh` section 7 | PASS — `le`/`lt`/`rev`/`byte`/`nested_eq` at ≤5 instructions with exactly one `cmp` and one `setcc` and no `sub`, the two refusals (`keep_pairs`, `keep_mixed`) identical in both builds, and `keep_second_reader` never longer |
| `check_predicate_materialize.sh` section 7b | PASS — the same probes on i686, AArch64 and RISC-V: `le` is strictly shorter with the fold on every one, and no shape on any target gets longer |
| `check_predicate_materialize.sh` section 8 | PASS — the valence test (`three_way_boolean_compare.c`) returns `VALIDATION OK` inside the gate, so a green gate cannot come from counts alone |
| Corpus A/B for `booldiff`, 3,124 files | **0 files longer**, no verdict change; the only shorter file is the valence test itself (2,612 -> 1,717), because no other corpus file asks a question about the difference of two indicators. The i686 and AArch64 sweeps cannot compile that file (this sandbox has no cross libc), so for them the claim is the weaker one the invariant needs: no file moved at all |

## Corpus-wide result

The final A/B compiles every one of the 3,016 test sources twice — once with
`CCC_DISABLE_PASSES=diffcmp,interval,signmag` and once without — and counts the
instructions in each output:

| metric | value |
|---|---|
| files compiled both ways | 3,016 (765 skipped: they need headers this sandbox lacks) |
| shorter with the passes | **27** |
| longer with the passes | **0** |
| net instruction change | **−147** |
| largest single win | `predicate_differences_and_ranges.c` −60 |
| largest vector win | `vec_range_mask_fusion.c` and its SSE2 twin −10 each |

Round 2 repeats the sweep over the same corpus with the three new IR rules as
the only difference (`CCC_NO_LOWEST_SET_BIT=1
CCC_SIMPLIFY_SKIP=zeroneg,negcast` versus the default):

| metric | value |
|---|---|
| files compiled both ways | 2,791 (710 skipped for missing headers) |
| shorter with the rules | **16** |
| longer with the rules | 1 |
| net instruction change | **−986** |
| largest wins | `packed_mask_shifts.c` −499, `packed_vector_relations.c` −446 (both `zeroneg`: `negw`/`negq` replacing `xorl; subq`) |
| the one longer file | `ext_vector_layout.c` +1 |

Round 3 repeats the sweep over the same 2,802-file glob with the two new rules
as the only difference, once per rule:

| metric | `fold_compare_dominance` | `notcast` |
|---|---|---|
| files compiled both ways | 2,086 (716 skipped for missing headers) | 2,086 (716 skipped) |
| shorter with the rule | **1** | **1** |
| longer with the rule | 0 | 0 |
| net instruction change | **−10** | **−1** |
| the file | `predicate_differences_and_ranges.c` −10 | `bb_slp_unary_demote.c` −1 |

Round 4 repeats it for the single-bit selection fold, which is switched off as a
whole by `CCC_NO_BIT_SELECT=1`, over every `.c` file under `tests/` and
`engineering/`:

| target | files compiled both ways | shorter | longer | verdict changes | net instructions |
|---|---|---|---|---|---|
| x86-64 | 3,078 | **3** | 0 | 0 | **−47** |
| i686 | 3,078 | 1 | 0 | 0 | −33 |
| AArch64 | 3,078 | 1 | 0 | 0 | −11 |

Round 5 adds the toggle to the same fold and repeats the sweep:

| target | files compiled both ways | shorter | longer | verdict changes | net instructions |
|---|---|---|---|---|---|
| x86-64 | 3,079 | **6** | 0 | 0 | **−68** |
| i686 | 3,079 | 2 | 0 | 0 | −57 |
| AArch64 | 3,079 | 1 | 0 | 0 | −11 |

The x86-64 wins beyond the probes are `tests/regression/simd_avx2_defer_chain.c`
(178 → 164), `tests/benchmark/programs/glibc_memcmp.c` (158 → 156) and
`tests/benchmark/programs/linux_find_bit.c` (123 → 122) — real code that spells
the idiom, which is the point: the fold is a parity fix for code that already
asks for `bts`/`btr`/`btc`, not a transformation that fires everywhere. `both
reject` (775 / 1,808 / 1,853) is corpus noise that fails identically in both
builds — missing `-I` paths, `-m32`, target intrinsics — and `verdict: 0` is the
invariant the sweep exists to keep. Lists, scripts and per-function tables:
[`engineering/evidence/2026-11-01-bit-selection/`](../evidence/2026-11-01-bit-selection/README.md)
and
[`engineering/evidence/2026-11-01-bit-toggle/`](../evidence/2026-11-01-bit-toggle/README.md).

Both are honest near-zeroes on this corpus, and the reason is the corpus, not
the rules: the test tree is 2,086 single-function files whose guards are written
as `&&` (already if-converted into branches by `cfg_simplify`) rather than as
the `&`/`|` of two comparisons these folds answer, and it contains almost no
`x & ~(1 << n)` bit-clears. The `-10` is `predicate_differences_and_ranges.c`
itself, whose five near-miss controls are exactly the shapes the round-3 probe
added to it. The evidence for the folds is therefore the probe measurements
above (9 → 4 on four idioms, 9 → 8 on the bit-clear), each level with or within
one instruction of both oracles, plus the exhaustive matcher tests — not this
sweep. What the sweep does establish is that neither rule pessimises anything:
zero longer files across 2,086 translation units for each.

`negcast` contributes nothing to that total and costs nothing: it fires on the
32-bit-negation shapes the probe corpus carries (`-(x != 0)`, 7 → 6) and no test
file in this corpus happens to contain one. The same sweep is what restricted it
in the first place: the unrestricted version measured **−929** with **three**
longer files, and the +60 it added to `packed_mask_shifts.c` was a vectorization
flip, not a fold (§9).

The one +1 is worth naming rather than hiding: `main` there is a
663-instruction boolean-chain checker, and the two Compiler-IRs are identical in
shape (502 instructions, 116 blocks, before and after — verified with
`CCC_DUMP_EACH_PASS`), so the extra instruction comes from how the backend
allocates the `Neg` operand, not from the fold. It is recorded here so a future
sweep can check whether it disappears when narrow `Neg` selection improves.

## What is not done

* **`(x ^ 1 << n) ^ 1 << n` as a *fold* rather than a copy** — handled at 2
  instructions (see §12), but only when the two masks are the same value. A
  spelling whose two `1u << n` are not CSE'd produces two `BitToggle`s that the
  identity would cancel; recognising them needs a structural index comparison
  (`BitToggle(BitToggle(x, a), b)` with `a == b` by value, not by SSA id), which
  no pass in this tree has. Rare enough to leave documented rather than to add a
  new value-equality mechanism for.
* **`__builtin_popcount` / parity below `-mpopcnt`** — the remaining gap is a
  linking decision, not a fold, and the numbers say so. With `-mpopcnt` LCCC
  already selects the instruction: `pop32` is `popcntl; mov; ret` (3, equal to
  GCC's 3 and one behind Clang's 2), `parity32` is `popcntl; andl $1; mov; ret`
  (4, equal to GCC) and `pop_and1` the same (4). Without `-mpopcnt` LCCC inlines
  the 17-instruction SWAR where GCC emits a call to libgcc's `__popcountdi2`
  (5) and Clang inlines 16; taking GCC's route would make every LCCC object
  depend on `libgcc` for one builtin, which the kernel and freestanding targets
  cannot assume, so the baseline stays inline. Parity is the same story from the
  other side: without `popcnt` GCC's `shr/xor/xorb/setnp` chain is 7 and LCCC's
  SWAR plus `andl $1` is 18 — worth revisiting only together with the popcount
  decision.
* **`1u << __builtin_ctz(y)` (no complement)** — refused on purpose: with the
  exact `Ctz(0) == width` arm it is no shorter than the count path (5/6 vs 8/8
  with the arm), and Clang's four-instruction form depends on `ctz(0)` being
  undefined. It becomes worth revisiting when the backend can select `blsi`
  under BMI1, where the isolated bit is one instruction without an arm.
* **Narrow negate at 8/16 bits** — `unsigned short f(unsigned short x) { return -x; }`
  still reloads through a truncation (`movzwl; negl; movzwl`) where GCC spends
  three instructions. The 32 → 16 fold of §9 recovers that and was measured as
  a *loss* (+60 on `packed_mask_shifts.c` from a vectorization flip, +4 from two
  spills), so it is not applied; the fix belongs in the narrow ALU lattice --
  an `And` at the narrow width should not need both operands masked
  (`x & (u16)(0 - x)`: 6 instructions now, 5 with the subtraction), which is the
  one site where `zeroneg` costs an instruction.
* **32-bit `bsf` zero fixup** — `Ctz` at 32 bits relies on `bsf` leaving its
  destination untouched and so needs a `mov $32, %ecx` that GCC/Clang do not
  pay. An instruction-selection detail of `Ctz`, not of the fold.
* **`|x|` instruction selection** — the identity is in place, but the
  conditional-move path still costs two instructions more than GCC's because it
  does not reuse the negation's flags. A backend change, not an IR fold.
* **Signed 8/16-bit difference compares** — refused on purpose; the C frontend
  promotes those widths, so the shape does not arise, and admitting it would
  require a no-overflow proof the IR does not carry.
* **The other 143 audited reports** stay deferred with their reasons in
  [audit-2026-10-31-sources.json](audit-2026-10-31-sources.json). Round 2 closed
  three items this list used to carry: the lowest-set-bit idiom, the truncated
  negate (at 64 → 32 bits; its 32 → 16 half is the narrow-ALU item above), and
  the crossing window -- which was never on the list, because `interval` looked
  like it already handled intervals and handled only the non-crossing ones. The
  backend items above and the remaining audited reports are the queue.
