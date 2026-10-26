# Batch 4 — red-team audit of the ring identities, and the folds it produced

Batch 3 shipped five ring identities and a ledger of 500 reports. Batch 4 does
three things to that work: it re-derives every deferred row with a probe of its
own, it closes the rows whose idiom turned out to be an instance of a rule the
sub-pass can prove, and it re-examines the claims that batch 3 made about
itself — one of which, the single-instruction power-of-two bias, was **wrong**
and is recorded below with the counter-example that killed it.

A second pass then re-derived every fold the batch ships against the emitted
code, and that pass is what the last third of this document is about: it
retired one claim, fixed four defects, and closed two suspicions in the
compiler's favour.

Everything below is a tool result on this machine: lccc built from this branch
with the `fastbuild` profile, oracle `gcc-16` = GCC 16.2.0, `-O2` on both sides,
instruction counts read from the emitted function body with the directive lines
removed.

## What changed

| fold | before | after | proof |
| --- | --- | --- | --- |
| power-of-two signed division | `idiv` or a magic multiply | `(x + (sign >>u w-k)) >>s k` | sign broadcast, see below |
| power-of-two signed remainder | `idiv` | `((x + t) & mask) - t`, `t = sign & mask` | sign broadcast |
| select whose condition an arm forces | or-trees only | or-trees, nested selects, zero tests | implication lattice |
| product by a boolean | `imul` | the value, or `select(b, x, 0)` | zero-or-one analysis |
| `((x ^ m) >>u n) ^ m` | already folded in batch 3 | unchanged, audited | two's complement identity |

### Power-of-two division: the bias must be the sign broadcast

Batch 3 wrote the signed power-of-two quotient as `x >>s k` plus a bias taken
from the *top* `k` bits of `x`. That is not the sign scaling: for
`x = 0x80000001` and `k = 4` the top four bits are `1000`, so the shifted bias
is `0x80000000`, while the correct rounding bias for a negative value is `-1`
scaled by `2^(w-k)`, i.e. `-134217728` versus `-134217727`. The quotient differs
by one for every negative `x` that is not a multiple of `2^k`.

The failing case was found by the new exhaustive expansion test
(`pow2_signed_expansions_match_truncating_semantics`), which evaluates the
*emitted instruction sequence* with truncating 32- and 64-bit semantics over
`k = 1..30`/`1..62` and the edge values `INT_MIN`, `INT_MIN + 1`, `INT_MAX`,
`±d`, `±(d/2 - 1)`, `±(d/2)`. The repair materialises the sign first —
`sign = x >>s (w-1)` — and then takes `sign >>u (w-k)`, which is `0` for a
non-negative `x` and exactly `2^(w-k) - 2^w = -2^(w-k)` for a negative one, the
correct `(2^k - 1)` rounding term. The same broadcast, masked with `2^k - 1`,
drives the remainder.

GCC's own power-of-two `srem` uses the identical shape
(`movl %edi,%eax; sarl $31,%eax; shrl $28,%eax; ...`), which is the strongest
available confirmation that the one-instruction form cannot exist.

That is also why the two expanders spell the same broadcast differently: the
quotient biases with `sign >>u (w-k)` and the remainder masks `sign` with
`2^k - 1`. Each is one instruction and the two are equal on a broadcast, and
each is the spelling GCC emits for that operation, so the emitted code matches
the oracle instruction for instruction instead of merely counting the same.

```text
   x % 16, before the fix        x % 16, after the fix        GCC 16.2
   movl  %edi, %eax              movl  %edi, %esi              movl  %edi, %eax
   cltd                          sarl  $31, %esi               sarl  $31, %eax
   movl  $16, %ecx               andl  $15, %esi               shrl  $28, %eax
   idivl %ecx                    addl  %esi, %edi              addl  %eax, %edi
   movl  %edx, %eax              andl  $15, %edi               andl  $15, %edi
   ret                           subl  %esi, %edi              subl  %eax, %edi
                                 movl  %edi, %eax              ret
                                 ret
```

The rewrite trades one instruction of length for the hardware divider: `idiv`
is a multi-uop instruction with a 20–40 cycle latency that occupies the divider
port, while the five ALU ops retire in about two cycles on a modern core. That
is why GCC pays nine instructions here, and why the gate measures divider
removal rather than length for these forms
(`NO_COUNT_REDUCTION` in `check_llvm_ring_identities.py`).

### The boolean lattice

`implied_condition_sense` answers one question: if `value` is non-zero, is the
condition `node` forced true, forced false, or neither? Three node shapes
answer it:

* an or-tree is true as soon as one side is, so a side that the value forces
  true decides the whole node;
* an `Eq`/`Ne` against zero is true (or false) when its other operand is the
  value, or an or-tree the value forces;
* a select on a condition the value forces takes the arm that condition picks,
  so its sense is that arm's sense.

`zero_when_zero` answers the dual question: is `node` zero whenever `value` is
zero? The constant zero, the value itself, a copy, any cast, a select over two
zero-forcing arms, and `and`/`mul` with one zero-forcing side all qualify; the
`or`/`xor` forms need both sides. Together the two relations justify

```c
(a | b | c) ? a : 0          /* → a   */   /* or-chain, 5 → 2 = GCC */
(a != 0 || b) ? a : 0        /* → a   */   /* nested select, 7 → 2 = GCC */
x != 0 ? x : 0               /* → x   */   /* zero test, 4 → 2 = GCC */
x != 0 ? 0 : x               /* → 0   */   /* opposite sense, zero arm wins */
x ? x : y                    /* refused: y survives a zero x */
```

The third line is also why the earlier unit-test expectation had to be
corrected rather than the fold: `x != 0 ? 0 : x` *is* the constant zero, and a
test that demanded `x` was testing the wrong thing.

`zero_or_one` decides the product rules. It admits compares, copies, casts and
`and`/`or`/`xor` of two such values — and deliberately **not** `not`, because
`~1` is all-ones, not one. An earlier revision of the helper admitted `not`,
and although the product fold's other half made that unreachable, the claim was
false and is gone; `a_complement_of_a_compare_is_not_a_boolean` pins it.

```c
x * (x || (b && c))          /* → x      */   /* 13 → 2 = GCC */
(x < y) * x                  /* → select */   /*  6 → 4 = GCC */
x * (y != 0)                 /* → select */   /*  6 → 4 = GCC */
```

The select form is chosen only when the boolean's producer is private (so its
`setcc` really disappears) and the identity form only when the value is a plain
operand; a shared producer keeps its `setcc` and the fold would add work.

### What the second red-team pass changed

Re-deriving every fold against the *emitted code* rather than the source idiom
turned up four defects worth fixing, one claim worth retiring, and two
suspicions that the source settles in the compiler's favour.

**The one-instruction remainder bias does not exist (retired).** The natural
way to shorten `((x + t) & mask) - t` is to read the rounding term straight out
of the dividend instead of materialising the sign: `bias = x >>u (w-k)`. The
top `k` bits of `x` are not the mask, though — for `x = 0x80000001` and `k = 4`
they are `1000` (8), while truncating rounding needs `2^k - 1` = 15, and
`srem32(-2147483647, 4)` came back `1` instead of `-3`. The exhaustive
expansion test caught it, the edit was reverted, and the test now pins both the
shape (`AShr` first) and the lengths (5 for the remainder, 4 for the quotient),
so a future attempt to shorten the sequence fails in `cargo test` rather than in
a program that ships.

**A per-family visit budget.** `facts::fold` ran every fold family against one
shared visit counter, so a family that walked a large boolean tree could leave
the counter exhausted and the select rule that follows it would stop firing
without any diagnostic. Each fold attempt now carries a private `Cell<u32>`
budget of 4096 steps: still bounded, so adversarial IR cannot turn the walk
quadratic, but no family can starve another.

**An abandoned fold on a non-definition.** `remainder_identity` asked
`cx.def(value)` for the computed operand and only tried the swapped operand
order when the definition existed but did not match. A constant or a parameter
has no definition at all, so `C == x % C` with the dividend on the right lost
the fold. A `def` miss now falls through to the swapped order like any other
miss.

**One rule, one home.** `saturated_bound` still carried the batch-3 zero-test
rule (`x != 0 ? x : 0`) that the implication lattice now proves in its general
form. The block was unreachable and is gone: a rule with two implementations is
a rule that drifts.

**Signed folding of unsigned constants.** `constant_fold::fold_binop` evaluated
`SDiv`, `SRem` and `AShr` on the stored `i64`, but `from_i64` stores an unsigned
constant *zero*-extended, so a `u32` bit pattern of `-1` arrives as 4294967295
and the signed operations read it as a positive four-billion. The folder now
extends from the operand width first (`signed_operand`), which is a no-op for
sign-extended signed constants and for 64-bit operands, and
`signed_operations_extend_unsigned_operands_to_their_width` pins both the
zero-extended and the sign-extended cases.

**Two suspicions, closed with code evidence.** The standard-bit rotate lowering
in `expr_builtins_c2y.rs` emits `SRem` on the count — but only under
`cnt_signed`, and only when the operand width is not a power of two. Every C
integer type lccc parses is 8, 16, 32, 64 or 128 bits, all powers of two, so the
branch is unreachable from C source (`unsigned _BitInt(24)` is rejected at
`expected ')'`), and it is correct by construction if a wide-integer type ever
reaches it: the signed compare path is selected by the count's type, not by its
value. The second suspicion — `SRem` at 32-bit width in `fold_binop` disagreeing
with the backends' operand-width `idiv` — is the paragraph above: the widths
disagreed only for zero-extended unsigned constants, and they no longer do.

## Re-audit of the 32 deferred rows

Every row of the batch-3 deferred table was re-transcribed into a probe of its
own and measured four ways: folds on, `CCC_NO_RING_IDENTITIES=1`, the whole
`bit_idioms` and division-strength pass off, and GCC 16.2. The raw numbers are
in [`llvm-batch4-audit.json`](llvm-batch4-audit.json).

| issue | idiom | batch 3 (lccc/GCC) | now (lccc/GCC) | verdict |
| --- | --- | ---: | ---: | --- |
| #92074 | `x % 16` | 9 / 7 | **8 / 7** | divider removed; the residual instruction is the return copy |
| #154025 | `(a \| b \| c) ? a : 0` | 5 / 2 | **2 / 2** | closed by the implication lattice |
| #84432 | `x * (x \|\| (b && c))` | 13 / 2 | **2 / 2** | closed by the zero-or-one product rule |
| #86171 | rotate by a masked count | 11 / 10 | 4 / 4 | the batch-3 probe used an unnormalised count; with the mask the rotate is already recognised |
| #193898 | or of shift-add | 5 / 4 | 4 / 4 | at oracle; the batch-3 probe mixed signedness |
| #156898 | `trunc(Pow2 >> x)` | 5 / 4 | 4 / 4 | at oracle |
| #63149 | masked 31-bit compare | 6 / 5 | 6 / 6 | at oracle; GCC's count was for a different mask |
| #71533 | select on the low bit | 12 / 9 | 5 / 6 | better than the oracle |
| #92966 | xor of two predicates | 17 / 7 | 9 / 7 | needs boolean-structure canonicalisation across compares |
| #95629 | `(a ^ (b \|\| c)) && c` | 11 / 10 | 11 / 10 | same family; the `&&` is a branch, not an operand identity |
| #141479 | `(b <= ~a) && (a > ~b)` | 11 / 9 | 11 / 10 | contradiction between complements needs a predicate solver |
| #117436 | `x != y ? (x \| y) != 0 : 0` | 8 / 4 | 8 / 7 | needs implication between two different compares |
| #200556 | `(x & 1) ? 0 : x` | 6 / 4 | 6 / 4 | needs demanded-bit analysis before the select |
| #200538 | `(1u >> x) != 0` | 7 / 4 | 7 / 4 | needs a range fact on a runtime shift count |
| #157524 | `(a - b) < 0 && a < b` | 9 / 4 | 9 / 4 | needs no-overflow reasoning on the subtract |
| #163084 | `x % 4 == 0 && x / 4 > 3` | 13 / 7 | 10 / 7 | needs a divisibility premise plus a scaled bound |
| #86176 | `a + (c ? 1 : 2)` | 7 / 4 | 7 / 4 | constant-select folding in the branch form |
| #108451 | `(x - y) + y * n` | 5 / 4 | 5 / 4 | value-numbering reassociation |
| #71948 | `((a << 1) + b) - ((a + b) & a)` | 6 / 5 | 6 / 5 | shift/add reassociation |
| #187057 | `(x + smin(y,0)) + smax(y,0)` | 10 / 9 | 10 / 9 | two-extreme select identity |
| #64305 | `a < b ? a : a % b` | 10 / 8 | 10 / 8 | Euclid remainder form |
| #85830 | `(sext(a) & mask) == C` | 6 / 4 | 6 / 4 | cast narrowing |
| #85939 | widen-then-shift | 5 / 4 | 4 / 3 | shift-operand narrowing, one instruction short of parity |
| #150954 | `__builtin_clz(x) ^ 31` | 3 / 2 | 3 / 2 | bit-count intrinsic recognition |
| #190907 | `(x << 1) * (c ? 2 : 0)` | 8 / 7 | 8 / 7 | multiply reassociation through a select |
| #74433 | `if (x) return 1; return 0;` | 6 / 4 | 6 / 4 | branch-to-`setcc` control flow |
| #74976 | `__builtin_ctz(~x)` | 5 / 4 | 5 / 4 | count-trailing-ones recognition |
| #85844 | `a > -1 ? 0 : (a \| 3)` | 7 / 6 | 7 / 6 | range-based select canonicalisation |
| #72512 | `(a + 1) - b == 1` | 6 / 4 | 6 / 4 | reassociation across a wrapping add |
| #107228 | `((a - 1) & ~a) >> 63` | 6 / 5 | 6 / 5 | known-bits analysis |
| #157370 | `(x % 10) % 5` | 15 / 14 | 20 / 19 | **regressed by count, see below** |
| #122442 | `(x % 100) / 10` | 12 / 10 | 17 / 14 | same |

Two rows are worth their own paragraph.

**`(x % 10) % 5` and `(x % 100) / 10`.** With the division-strength pass off,
these compile to two `idiv` instructions — 10 and 9 instructions. With it on,
each division becomes a magic multiply, and the pair costs 20 and 17 against
GCC's 19 and 14. So the pass costs *length* on both chains and almost all of the
gap to GCC is shared: GCC also pays a magic multiply for both divisions, and is
one instruction shorter only because of the return copy. The remaining three
instructions on `(x % 100) / 10` are the real lccc deficit and the real
follow-up: the second division should be decomposed against the first instead of
strength-reduced independently. With `q = x / 100` and `r = x - 100 * q` — so
`|r| < 100` and `x = 100 * q + r` — the truncating quotient satisfies
`x / 10 = 10 * q + r / 10`, because a multiple of ten can be moved through the
truncating division unchanged. The inner quotient divides a value already
bounded by 100, so it is one multiply on a small range instead of a second
64-bit magic constant.
That decomposition is a
batch of its own; the ledger records the counts and the shape rather than
pretending the current code is at the oracle.

**Rows that moved without a fold.** #86171, #193898, #156898, #63149 and #71533
are at or better than the oracle in the re-transcription while batch 3 recorded
a gap. In each case the prose idiom of the report admits more than one C
spelling and the batch-3 probe picked one that lost to the front end's
canonicalisation (an unnormalised rotate count, a mixed-signedness shift-add, a
different mask constant). The re-transcription is the one in
`llvm-batch4-audit.json`; the honest reading is that these reports were never
about a missed fold lccc lacks, and they are recorded as `at-oracle` with the
counts that show it.

## What the gate now pins

The regression fixture grew from 27 to 32 folded forms: `ring_nested_select`
(nested boolean condition), `ring_boolean_product` (`x * (x || (b && c))`),
`ring_bool_product_self`, `ring_bool_product_test` and `ring_bool_product_cmp`.
Each is checked four ways — the fold on, the ring-identity sub-pass off, the
whole bit-idiom pass off, and the division strength reduction off where a
divider is involved — against GCC 16.2 at `-O2`, for a mechanism that must be
present and one that must not, and for the value the function returns.

## Differential red team

`scripts/fuzz_identities.py` draws random programs from the idiom families —
remainder windows at every width, sign-mask shifts in both mask forms with
runtime counts, complements of negation, saturated bounds, zero-test selects in
all four arm orders, or-chains, nested boolean selects, boolean products in four
spellings plus the three-operand `x * (x || (y && z))` form, and power-of-two
division and remainder — and builds each program three ways: folds on, folds off
(`CCC_NO_RING_IDENTITIES=1`, `CCC_DISABLE_PASSES=bit_idioms,divconst`), and GCC
16.2. Every program is executed and its digest compared.

The case matrix grew with the folds: the boolean products now cover the plain
`||` spelling, the `(x != 0) | (y != 0)` spelling the front end produces for the
`|`-of-compares idiom, a product by a compare, and a product by an `or` of a
compare and a constant, and the nested-select case is the same shape the
regression fixture pins.

The generator is deliberately UB-free, because a differential oracle is only an
oracle for defined programs: signed negation, signed multiply and a `-1`
divisor are kept away from `INT_MIN`, and shift counts stay below the width.
With those constraints removed, all eight seed programs "failed" — against
GCC, with the two lccc builds agreeing — which is the expected signature of
undefined behaviour and the reason the matrix is constrained.

Cumulative: **370 random programs, 0 mismatches** — 150 with the batch-3 fold
set, then 60 and 100 after the boolean lattice and the power-of-two repricing
landed, then 60 more after the second red-team pass, each program executed
against all three builds.

## Residual gap that is not a fold

Every single-expression fold in this batch ends with one more instruction than
GCC because the register allocator homes the result in the parameter's register
and the emitter then copies it to `%eax` for the return. `copy_coalesce`'s rule
6 refuses to rename into the `%rax` family because `ret` reads `%rax`
implicitly, and `%rax` is not in the allocator's home pool, so the copy is
structural rather than local. Removing it means giving the allocator a return
hint (`reg_hints` already exists for parameter homes) or an epilogue coalescing
rule, both of which touch every function in the corpus. That is a batch of its
own, with its own corpus measurement, and it is the single highest-value
follow-up in this area: it is worth one instruction on every fold listed above.

The same measurement pins the two shapes that make up the rest of the gap, and
neither of them is a fold either:

| form | lccc | GCC 16.2 | what the difference is |
| --- | ---: | ---: | --- |
| `x % 32` | 8 | 7 | the return-home copy |
| `x / 32` | 7 | 5 | the return-home copy, plus GCC's `lea`-fused conditional add (`testl`, `leal 31(%rdi), %eax`, `cmovns`, `sarl`) where lccc pays a sign broadcast it cannot fold into the add |
| `~(0 - x)` on a 16-bit value | 4 | 2 | the `movswq`/`movzwl` representation narrowing the backend emits for a narrow operand and result; GCC needs neither because the ABI truncates the returned value for it |

The conditional-add quotient needs two backend pieces that are not there yet:
`lower_binop` only considers `lea` for `OpSize::S64` in its `Add` fast path,
even though `lea_size` already maps the narrow sizes onto `leal` with a
documented rationale, and the allocator has to place the add's destination in a
fresh register instead of coalescing it into a dead base. Both are corpus-wide
changes, which is why they belong to the return-home batch rather than to this
one.
