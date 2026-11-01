# Scalar predicate folds: differences, intervals, membership, sign magnitude

This is the code half of the 2026-10-31 re-inventory. The audit itself — 151
reports, one verdict each, and the eight gaps it measured — lives in
[AUDIT-2026-10-31.md](AUDIT-2026-10-31.md). This document covers what the
compiler now does differently, why each rewrite is admissible, and what the
three oracles say.

Everything here was driven by measurement, not by taste: every rewrite was
first reproduced as a three-compiler instruction-count gap on x86-64 at `-O2`,
then implemented, then re-measured against the same three compilers.

**Round 2 (2026-11-01)** closed two of the items this document listed as "not
done" — the lowest-set-bit matcher and a ring-correct interval width that lets
windows crossing zero fold — and added the `0 - x` negation canonicalisation
they depend on. Sections 6, 7 and 8 below are new; the measurements table and
the "What is not done" list are updated in place.

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
| matcher (`bit_idioms.rs`) | `fold_lowest_set_bit` | `1 << Ctz(~x)` → `(x + 1) & ~x`, plus the exact `~x == 0` arm |
| rule (`simplify.rs`) | `zeroneg` | `0 - x` → `Neg x` (the modular negate, canonically spelled) |
| tests (`bit_idioms_mask_interval_tests.rs`) | — | production-matcher tests for the two matchers |
| gate (`check_predicate_differences_and_ranges.py`) | `predicate-differences-and-ranges` | the three passes |
| gate (`check_mask_and_interval_folds.py`) | `mask-and-interval-folds` | the two matchers |
| gate (`check_lowest_set_bit_fold.sh`) | `lowest-set-bit-fold` | the lowest-set-bit matcher and `zeroneg` |
| tests (`bit_idioms_lowest_bit_tests.rs`) | — | executes the rewritten block against the reference semantics |

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

### 8. Ring-correct interval width — `interval`

```c
int f(int x) { return x > -8 && x < 8; }      /* 7 -> 5, GCC 5 */
```

`interval` stores its bounds the way `predicate` normalised them: masked into the
operand ring. A window that starts below zero therefore stores its lower bound
*above* its upper one (`[-7, 7]` at 32 bits is `[0xFFFF_FFF9, 7]`), and the
"empty window" guard — a plain 64-bit `lower > upper` — read every crossing
window as ~2^64 values wide and refused it. That is why `x > 3 && x < 10` folded
and `x > -8 && x < 8` did not: same pass, same shape, one negative constant.

`Interval::width` now counts the span in the operand ring (`(upper - lower) mod
2^W + 1`), which gives 15 for `[-7, 7]`, 0 for an inverted window
(`[-5, -9]`), and leaves the `> 2^31-1` refusal to `emit` as before. All eight
mixed-strictness combinations of `> -8 / >= -7 / < 8 / <= 7` now fold to the same
biased compare (`leal 7(%rdi); cmpl $15; setb`), and the gate's brute-force
sweep — which previously skipped every such window by the same guard — now
asserts them.

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
CCC_SIMPLIFY_SKIP=zeroneg`: the lowest-set-bit matcher owns the `lsb32`/`lsb64`
rows, `zeroneg` owns the packed-mask negates, and the interval width fix is not
gated by a knob — it is attributed by the eight crossing-window cases in
`predicate_differences_and_ranges.c`, each of which the gate asserts is a single
compare. The pass-off run leaves `range_crossing_control` (`x > -8 && x < 1`, a
different pair of constants) at two compares, so the gate cannot pass by
accident.

## Verification

| Check | Result |
| --- | --- |
| `cargo test --profile fastbuild --locked --lib` | **7,144 passed, 0 failed, 10 ignored** |
| `check_predicate_differences_and_ranges.py` | PASS — values at `-O0`…`-O3`, per-fold mechanism, five controls, attribution, GCC 16.2 length oracle (1.7 s) |
| `check_lowest_set_bit_fold.sh` | PASS — both widths, both arms of the zero select, plain forms pinned at ≤5/≤6 |
| `Interval::width` unit tests | crossing window `[-7, 7]` = 15, inverted `[-5, -9]` = 0, `[7, 7]` = 1, 64-bit span unchanged |
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

Round 2 repeats the sweep over the same corpus with the two new IR rules as the
only difference (`CCC_NO_LOWEST_SET_BIT=1 CCC_SIMPLIFY_SKIP=zeroneg` versus the
default):

| metric | value |
|---|---|
| files compiled both ways | 2,790 (710 skipped for missing headers) |
| shorter with the rules | **16** |
| longer with the rules | 1 |
| net instruction change | **−986** |
| largest wins | `packed_mask_shifts.c` −499, `packed_vector_relations.c` −446 (both `zeroneg`: `negw`/`negq` replacing `xorl; subq`) |
| the one longer file | `ext_vector_layout.c` +1 |

That single +1 is worth naming rather than hiding: `main` there is a
663-instruction boolean-chain checker, and the two Compiler-IRs are identical in
shape (502 instructions, 116 blocks, before and after — verified with
`CCC_DUMP_EACH_PASS`), so the extra instruction comes from how the backend
allocates the `Neg` operand, not from the fold. It is recorded here so a future
sweep can check whether it disappears when narrow `Neg` selection improves.

## What is not done

* **`1u << __builtin_ctz(y)` (no complement)** — refused on purpose: with the
  exact `Ctz(0) == width` arm it is no shorter than the count path (5/6 vs 8/8
  with the arm), and Clang's four-instruction form depends on `ctz(0)` being
  undefined. It becomes worth revisiting when the backend can select `blsi`
  under BMI1, where the isolated bit is one instruction without an arm.
* **Narrow negate truncation** — `unsigned short f(unsigned short x) { return -x; }`
  still reloads through a 16-bit truncation (`movzwl; negl; movzwl`) where a
  single `negw` would do; `zeroneg` produces the `Neg`, so this is purely the
  backend's narrow `Neg` selection.
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
  [audit-2026-10-31-sources.json](audit-2026-10-31-sources.json). Round 2 cleared
  two of the eight measured gaps (the lowest-set-bit idiom and the crossing
  window, which was not on the list because the pass looked like it already
  handled intervals — it handled the non-crossing ones only); six remain.
