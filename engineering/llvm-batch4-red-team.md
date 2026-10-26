# Batch 4 — red-team audit of the ring identities, and the folds it produced

Batch 3 shipped five ring identities and a ledger of 500 reports. Batch 4 does
three things to that work: it re-derives every deferred row with a probe of its
own, it closes the rows whose idiom turned out to be an instance of a rule the
sub-pass can prove, and it re-examines the claims that batch 3 made about
itself — one of which, the single-instruction power-of-two bias, was **wrong**
and is recorded below with the counter-example that killed it.

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
| #117436 | see above | | | |
| #72512 | `(a + 1) - b == 1` | 6 / 4 | 6 / 4 | reassociation across a wrapping add |
| #107228 | `((a - 1) & ~a) >> 63` | 6 / 5 | 6 / 5 | known-bits analysis |
| #157370 | `(x % 10) % 5` | 15 / 14 | 20 / 19 | **regressed by count, see below** |
| #122442 | `(x % 100) / 10` | 12 / 10 | 17 / 14 | same |

Two rows are worth their own paragraph.

**`(x % 10) % 5` and `(x % 100) / 10`.** With the division-strength pass off,
these compile to two `idiv` instructions — 10 and 9 instructions. With it on,
each division becomes a magic multiply, and the pair costs 20 and 17. The magic
multiply is the better *latency* choice for one division, but for a chain the
right answer is a divisor/quotient decomposition: `(x % 100) / 10` is
`(x - 100 * (x / 100)) / 10` only if the second division is rebuilt, whereas the
quotient chain `x / 100 → t`, `x / 10 = 10 * t + (x / 10) % 10` reuses the first
magic constant. That is a real, sizeable follow-up and it is recorded as such
rather than papered over: the current behaviour is a net loss on instruction
count for chained division and a win on latency, and the ledger says so.

**Rows that moved without a fold.** #86171, #193898, #156898, #63149 and #71533
are at or better than the oracle in the re-transcription while batch 3 recorded
a gap. In each case the prose idiom of the report admits more than one C
spelling and the batch-3 probe picked one that lost to the front end's
canonicalisation (an unnormalised rotate count, a mixed-signedness shift-add, a
different mask constant). The re-transcription is the one in
`llvm-batch4-audit.json`; the honest reading is that these reports were never
about a missed fold lccc lacks, and they are recorded as `at-oracle` with the
counts that show it.

## Differential red team

`research/fuzz_identities.py` draws random programs from the idiom families —
remainder windows at every width, sign-mask shifts in both mask forms with
runtime counts, complements of negation, saturated bounds, zero-test selects in
all four arm orders, or-chains, nested boolean selects, boolean products, and
power-of-two division and remainder — and builds each program three ways: folds
on, folds off (`CCC_NO_RING_IDENTITIES=1`, `CCC_DISABLE_PASSES=bit_idioms,divconst`),
and GCC 16.2. Every program is executed and its digest compared.

The generator is deliberately UB-free, because a differential oracle is only an
oracle for defined programs: signed negation, signed multiply and a `-1`
divisor are kept away from `INT_MIN`, and shift counts stay below the width.
With those constraints removed, all eight seed programs "failed" — against
GCC, with the two lccc builds agreeing — which is the expected signature of
undefined behaviour and the reason the matrix is constrained.

150 random programs, 0 mismatches.

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
