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
| `cargo test --lib` | **7,206 passed, 0 failed, 10 ignored** after round 4 (+29), plus the toggle's 4 matcher/fold tests in round 5 |
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
