# Scalar predicate folds: differences, intervals, membership, sign magnitude

This is the code half of the 2026-10-31 re-inventory. The audit itself — 151
reports, one verdict each, and the eight gaps it measured — lives in
[AUDIT-2026-10-31.md](AUDIT-2026-10-31.md). This document covers what the
compiler now does differently, why each rewrite is admissible, and what the
three oracles say.

Everything here was driven by measurement, not by taste: every rewrite was
first reproduced as a three-compiler instruction-count gap on x86-64 at `-O2`,
then implemented, then re-measured against the same three compilers.

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
| tests (`bit_idioms_mask_interval_tests.rs`) | — | production-matcher tests for the two matchers |
| gate (`check_predicate_differences_and_ranges.py`) | `predicate-differences-and-ranges` | the three passes |
| gate (`check_mask_and_interval_folds.py`) | `mask-and-interval-folds` | the two matchers |

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

A shared subtraction keeps its value: the pass replaces one comparison node and
never deletes an instruction.

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

The pass runs twice: before `bit_idioms` for the bitwise `&`/`|` spelling, and
again after if-conversion for the `&&`/`||` spelling, which only becomes a select
once the CFG diamonds are gone. That second placement is what takes
`x > 3 && x < 10` from seven instructions to five.

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
| `1u << __builtin_ctz(~x)` | 9 | 9 | 6 | 4 | **not done**, see below |

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

## Verification

| Check | Result |
| --- | --- |
| `cargo test --profile fastbuild --locked` | **7,092 passed, 0 failed** (19 new) |
| `check_predicate_differences_and_ranges.py` | PASS — values at `-O0`…`-O3`, per-fold mechanism, five controls, attribution, GCC 16.2 length oracle (1.5 s) |
| `check_mask_and_interval_folds.py` | PASS (0.6 s) |
| Exhaustive matcher tests | 8/16-bit rings brute-forced for interval and membership; 32/64-bit boundary sweeps; refusal tables |
| Corpus A/B, 3,016 `tests/**/*.c` | per-pass instruction totals and every file that grew, recorded below |

## What is not done

* **`1u << __builtin_ctz(~x)`** — the lowest-set-bit idiom (`y & -y`). Nine
  instructions against Clang's four (`blsi`). This is the next bit-idiom
  matcher; it needs a `Shl(1, Ctz(y))` recognition plus the negation.
* **`|x|` instruction selection** — the identity is in place, but the
  conditional-move path still costs two instructions more than GCC's because it
  does not reuse the negation's flags. A backend change, not an IR fold.
* **Signed 8/16-bit difference compares** — refused on purpose; the C frontend
  promotes those widths, so the shape does not arise, and admitting it would
  require a no-overflow proof the IR does not carry.
* **The other 143 audited reports** stay deferred with their reasons in
  [audit-2026-10-31-sources.json](audit-2026-10-31-sources.json); the eight
  measured gaps are the queue for the next round.
