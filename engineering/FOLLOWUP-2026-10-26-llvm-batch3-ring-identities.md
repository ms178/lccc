# FOLLOWUP 2026-10-26 — LLVM missed-optimization campaign, batch 3

Batches 1 and 2 worked on scalar demand observation and then on memory shape and
control flow. Batch 3 returns to the middle of the scalar spectrum with a
different question: which of the corpus's outstanding reports are **exactly
true in the operand ring**, so that a fold can be justified without borrowing a
no-wrap flag, a poison rule, a dominance fact or a range assumption that lccc
deliberately never synthesises?

The answer is a new `bit_idioms` sub-pass, `bit_idioms_identities`, with five
rewrites, plus a review ledger of 500 reports that records, per report, either
the measured verdict or the specific premise that puts it out of reach.

Everything below is a tool result on this machine: lccc built from this branch
with the `fastbuild` profile, oracle `gcc-16` = GCC 16.2.0, host `gcc` 12.2,
`-O2` on both sides, instruction counts read from the emitted function body.

## The cohort

* Query: `repo:llvm/llvm-project is:issue "missed optimization" in:title`
  (763 reports in total, the first 500 sampled in creation order).
* Ledger: [`engineering/llvm-batch3-review.json`](llvm-batch3-review.json) —
  one row per report with `verdict`, `evidence` (`measured` / `text-rule`) and
  the reason.

| verdict | reports | evidence |
| --- | ---: | --- |
| `folded-this-batch` | 5 | measured A/B in this document |
| `already-at-oracle` | 16 | measured against GCC 16.2 |
| `confirmed-gap-deferred` | 32 | measured against GCC 16.2, reason recorded |
| `needs-ub-premise` | 108 | text rule: `poison`/`nsw`/`nuw`/`inbounds`/`assume`/`range` premise |
| `loop-or-cfg` | 86 | text rule: loop or control-flow restructuring |
| `scalar-unprobed` | 54 | scalar, but the report does not pin one expressible function |
| `vector-or-simd` | 53 | text rule: lanes, shuffles, per-lane saturation |
| `fp-semantics` | 44 | text rule: NaN / signed zero / fast-math |
| `memory-or-abi` | 44 | text rule: memory or ABI lowering |
| `non-x86-target` | 27 | text rule: AArch64/RISC-V/GPU/… lowering |
| `other-language` | 21 | MLIR/Flang/libcxx/static analyzer |
| `x86-only-isa` | 10 | EVEX/APX/`ccmp`/`sbb` instruction selection |

58 reports carry a measurement; the remaining 442 carry a rule match that names
the exact token that decided them, so a reader can re-derive a verdict instead
of trusting a label. The 54 `scalar-unprobed` rows are the explicit follow-up
queue: scalar shapes whose report body does not pin down a single C function to
compile.

## Built in this batch

### `src/passes/bit_idioms_identities.rs`

The sub-pass is dispatched from `bit_idioms_facts::fold` after the complement
family and before the shift observers, and it is gated by the same authority
contract as every other family: `CCC_NO_RING_IDENTITIES=1` switches the whole
sub-pass off for A/B and for bisecting a miscompile.

Every rewrite here replaces **one consumer** with an equivalent expression. No
producer is repurposed, no memory operation is touched, and no rewrite assumes
that a value is in range, non-poison, or dominated by anything. The five folds:

#### 1. `x % C` compared against `x` → one add and one unsigned compare

Issue #215060 and #230409. `x % C == x` holds exactly when `x / C` is zero,
i.e. when `x` lies in the window `[1 - |C|, |C| - 1]`. That window is a
contiguous signed range containing zero, so shifting it down by its lower bound
maps it onto `0 ..= 2|C| - 2` and a single **unsigned** compare decides
membership; the `!=` spelling is the strict complement of the same compare.

`|C|` is taken as a signed magnitude, so `C = INT_MIN` — whose negation is not
representable — still yields `[INT_MIN + 1, INT_MAX]`, the correct window. A
zero divisor is division by zero at source level and is never rewritten.

```text
before                                     after
movabsq $2361183241434822607, %rcx         addq    $249, %rdi
imulq  %rcx                                cmpq    $498, %rdi
sarq   $5, %rdx                            setbe   %al
movq   %rdx, %r8                           (xorl %eax,%eax)
shrq   $63, %r8
addq   %r8, %rdx                           GCC 16.2 needs 13 instructions
imulq  $250, %rdx, %rdx                    for the same function.
subq   %rdx, %rdi
cmpq   %rdi, ...
sete   %al
```

The `private` precondition (the remainder has exactly one user) is what makes
the rewrite a strict win instead of a copy of the same work: the whole
magic-multiply chain dies.

#### 2. `((x ^ m) >>u n) ^ m` with `m` the sign broadcast of `x` → `x >>s n`

Issue #204851. When `m` is zero the two xors vanish and both shifts agree; when
`m` is all ones the pair of xors is exactly the two's complement identity
`x >>s n == ~(~x >>u n)`. `m` is recognised in either of the two forms a front
end produces: `0 - (x >>u w-1)` and `x >>s w-1`.

The count is **not** required to be a constant. lccc's shift semantics mask an
out-of-range count (`wrapping_shr` in the constant folder), and `AShr` masks it
the same way, so the identity is exact for every count value — a runtime count
included, which is the common shape in real code. The alternative reading, that
an out-of-range count is undefined, would have forced a constant-count
restriction and lost the entire benefit.

#### 3. Saturated min/max selects, and zero-test selects

Issue #206153 and the family around #230424. `min(x, INT_MAX)`, `max(x,
INT_MIN)`, `max(x, 0u)` and `min(x, UINT_MAX)` all return `x` for **every** bit
pattern, because the bound can only be selected when the operand already equals
it. The complement is equally important: one step inside the ring is a real
clamp, and the gate pins that it survives (`ring_clamp_imax` must keep its
`cmov`).

A clamp against zero reaches this pass as a boolean test rather than an
inequality — `x > 0u` is canonicalised to `x != 0` before the rewrite sees it —
so `x != 0 ? x : 0` and `x == 0 ? 0 : x` are folded too, while the two reversed
arm orders (`x != 0 ? 0 : x`) are refused: they are genuine zeroing choices, not
identities.

#### 4. `~(0 - x)` → `x - 1` and `0 - ~x` → `x + 1`

Issue #60173. Pure two's complement; one instruction replaces two.

#### 5. Refusals

The fold declines, by construction, every shape whose correctness needs a fact
this sub-pass cannot prove:

* a shared remainder, a shared sign mask, or a shared inner unary (the producer
  would survive, so the rewrite would add an instruction rather than remove
  one);
* a mask that is not this value's sign broadcast — a `x >> 31` on an *unsigned*
  operand is a 0/1 mask, not a broadcast, and is left alone;
* a divisor or shift operand of a different width than the compare;
* narrow remainders: `x % 250` on an `int16_t` is promoted to `int` before the
  remainder is formed, so the dividend is a sign-extended copy and not the same
  SSA operand. The gate records this as a documented boundary
  (`promotion_boundary` in the JSON) and fails if the shape starts folding
  without a matching proof.

## Measured

### Gate: `tests/regression/check_llvm_ring_identities.py`

32 folded forms, each one smaller than the same form compiled with the
controlling pass disabled — the ring-identity sub-pass for the identity
families, the division strength reduction for the four power-of-two forms,
which are measured for divider removal rather than for length — plus one clamp
that must survive, plus four documented promotion-boundary refusals. All three
disable controls (`CCC_NO_RING_IDENTITIES=1`, `CCC_DISABLE_PASSES=bit_idioms`,
`CCC_DISABLE_PASSES=divconst`) produce identical program output, and the native
binary agrees with GCC 16.2 at `-O0`, `-O1`, `-O2`, `-O3`, `-Os`, `-O3 -g` and
`-O2 -march=x86-64-v3`.

| form | control off | fold on | idiom |
| --- | ---: | ---: | --- |
| `ring_srem_eq_250` | 14 | 5 | remainder window |
| `ring_srem_ne_250` | 14 | 5 | remainder window |
| `ring_srem_eq_min` | 8 | 6 | remainder window |
| `ring_srem_ne_min` | 8 | 6 | remainder window |
| `ring_srem_eq_neg7` | 14 | 5 | remainder window |
| `ring_srem_eq32` | 14 | 5 | remainder window |
| `ring_srem_ne32` | 14 | 5 | remainder window |
| `ring_sign_shift` | 9 | 4 | sign-mask shift |
| `ring_sign_shift64` | 9 | 4 | sign-mask shift |
| `ring_not_neg` | 4 | 2 | complement of negation |
| `ring_neg_not` | 4 | 2 | complement of negation |
| `ring_not_neg16` | 5 | 4 | complement of negation |
| `ring_neg_not64` | 4 | 2 | complement of negation |
| `ring_min_imax` | 4 | 2 | saturated bound or zero test |
| `ring_max_imin` | 4 | 2 | saturated bound or zero test |
| `ring_min_umax` | 5 | 2 | saturated bound or zero test |
| `ring_max_u0` | 4 | 2 | saturated bound or zero test |
| `ring_min_imax64` | 6 | 2 | saturated bound or zero test |
| `ring_max_imin64` | 6 | 2 | saturated bound or zero test |
| `ring_zero_select` | 4 | 2 | saturated bound or zero test |
| `ring_zero_constant_eq` | 5 | 2 | saturated bound or zero test |
| `ring_or_chain_value` | 5 | 2 | implied-condition select |
| `ring_or_chain_zero` | 6 | 2 | implied-condition select |
| `ring_nested_select` | 7 | 2 | implied-condition select |
| `ring_boolean_product` | 13 | 2 | implied-condition select |
| `ring_bool_product_self` | 9 | 2 | implied-condition select |
| `ring_bool_product_test` | 6 | 4 | boolean product |
| `ring_bool_product_cmp` | 6 | 4 | boolean product |
| `ring_srem_pow2_32` | 8 | 8 | power-of-two remainder (control: `divconst`) |
| `ring_srem_pow2_64` | 8 | 8 | power-of-two remainder (control: `divconst`) |
| `ring_srem_pow2_neg` | 8 | 8 | power-of-two remainder (control: `divconst`) |
| `ring_sdiv_pow2_32` | 7 | 7 | power-of-two quotient (control: `divconst`) |

Against the oracle on the same sources: the remainder window is 5 instructions
where GCC 16.2 needs 13, the sign-mask shift is 4 where GCC needs 7, and the
saturated bound, the zero-test select, the implied-condition select and the
boolean product all reach the same two instructions GCC emits. The one narrow
exception is `ring_not_neg16` at 4 against GCC's 2: the 16-bit form carries the
ABI-mandated `movswq`/`movzwl` pair that narrows the operand to its
representation and truncates the result, which GCC elides because it happens to
need neither — a backend narrow-integer concern, not a fold concern. The four
power-of-two forms are one to two instructions longer than GCC's (`8` against
`7` for the remainder, `7` against `5` for the quotient) while carrying no
`idiv`: the difference is the return-home copy plus, for the quotient, the
`lea`-fused conditional add that GCC uses and the 32-bit `lea` this backend
does not select yet.

### Differential A/B

A 2000-probe program (remainders at every sign boundary and `INT_MIN` divisor,
runtime shift counts `0..40` including out-of-range counts, complements, zero
tests and saturated bounds) was built three ways and executed:

| build | output |
| --- | --- |
| lccc `-O2`, sub-pass on | `d8acf871068069bd` |
| lccc `-O2`, `CCC_NO_RING_IDENTITIES=1` | `d8acf871068069bd` |
| gcc-16 `-O2` | `d8acf871068069bd` |

The two lccc assembly outputs differ by 1556 lines, so the agreement is not an
artefact of the fold never firing.

### Unit tests

Eight tests in `bit_idioms_identities_tests.rs`, of which three are exhaustive
rather than sampled:

* every divisor of the byte ring (255 of them) × `Eq`/`Ne` × both operand
  orders × all 256 values of `x`, against a truncating-division reference;
* the sign-mask shift for every count in `0 ..= 2w+1` at 8, 16 and 32 bits, in
  both mask forms and both xor orders;
* the saturated min/max select at all eight widths in both arm orders, and the
  four reversed orders that must be refused.

The remainder of the suite pins the extremal divisors at 64 bits, the
runtime-count acceptance, and each refusal listed above.

## Deferred: measured gaps

> **Batch 4 update.** Every row below was re-transcribed, re-measured and
> re-verdicted in [`llvm-batch4-red-team.md`](llvm-batch4-red-team.md), with the
> raw counts in [`llvm-batch4-audit.json`](llvm-batch4-audit.json). #154025 and
> #84432 are closed by the select-lattice and boolean-product rules, #92074's
> divider is gone, and five rows turned out to be at the oracle once the idiom
> is spelled canonically.

These reports were probed, reproduced and measured, and are **not** in this
batch. Each is a follow-up with its own prerequisite, recorded so the next
batch starts from evidence rather than from a title.

| issue | idiom | lccc | GCC 16.2 | prerequisite |
| --- | --- | ---: | ---: | --- |
| #92074 | `x % 16` | 9 | 7 | div-by-const routes powers of two through the magic multiply; a dedicated `(x + t) >>s k` path belongs in `div_by_const` |
| #72512 | `(a + 1) - b == 1` | 6 | 4 | reassociation across a wrapping add |
| #107228 | `((a - 1) & ~a) >> 63` → `a == 0` | 6 | 5 | known-bits/demanded-bits analysis |
| #200538 | `(1u >> x) != 0` | 7 | 4 | range fact on a runtime shift count |
| #157370 | `(x % 10) % 5` | 15 | 14 | divisor divisibility reasoning |
| #122442 | `(x % 100) / 10` | 12 | 10 | combined div/rem decomposition |
| #157524 | `(a - b) < 0 && a < b` | 9 | 4 | no-overflow reasoning on the subtract |
| #85844 | `a > -1 ? 0 : (a \| 3)` | 7 | 6 | range-based select canonicalisation |
| #117436 | `x != y ? (x \| y) != 0 : 0` | 8 | 4 | implication across two compares |
| #200556 | `(x & 1) ? 0 : x` | 6 | 4 | demanded bits before the select |
| #74976 | `__builtin_ctz(~x)` | 5 | 4 | count-trailing-ones recognition |
| #86171 | rotate with multiply | 11 | 10 | funnel-shift family with multiply reassociation |
| #74433 | `if (x) return 1; return 0;` | 6 | 4 | branch-to-`setcc` control flow |
| #84432 | `x * (x \|\| (b && c))` | 13 | 2 | boolean algebra with a multiply |
| #163084 | `sdiv`-exact + `icmp ugt` | 13 | 7 | divisibility test plus scaled bound |
| #92966 | xor of two predicates | 17 | 7 | boolean canonicalisation |
| #154025 | `(a \| b \| c) ? a : 0` | 5 | 2 | or-chain canonicalisation |
| #141479 | `(b <= ~a) && (a > ~b)` | 11 | 9 | contradiction detection between complements |
| #86176 | `a + (c ? 1 : 2)` | 7 | 4 | constant-select folding |
| #108451 | `(x - y) + y * n` | 5 | 4 | value-numbering reassociation |
| #71948 | `((a << 1) + b) - ((a + b) & a)` | 6 | 5 | shift/add reassociation |
| #193898 | or-of-shift-add | 5 | 4 | or-chain reassociation |
| #187057 | `(x + smin(y,0)) + smax(y,0)` | 10 | 9 | two-extreme select identity |
| #156898 | `trunc(Pow2 >> X)` | 5 | 4 | known bits on the shifted mask |
| #95629 | `(a ^ (b \|\| c)) && c` | 11 | 10 | boolean algebra |
| #64305 | `(a < b) ? a : a % b` | 10 | 8 | Euclid remainder form |
| #63149 | masked 31-bit compare | 6 | 5 | masked-compare narrowing |
| #85830 | `(sext(a) & mask) == C` | 6 | 4 | cast narrowing |
| #85939 | widen-then-shift | 5 | 4 | shift-operand narrowing |
| #150954 | `__builtin_clz(x) ^ 31` | 3 | 2 | bit-count intrinsic recognition |
| #190907 | `(x << 1) * (c ? 2 : 0)` | 8 | 7 | multiply reassociation through a select |
| #71533 | `select` on `a % 2`, squares | 12 | 9 | div/rem decomposition with a parity test |

Reports measured at or better than the oracle (16 of them, e.g. #83699 at 5 vs
GCC's 10, #230763 `trunc` relation at 2, #143630 and #76623 at parity) are
`already-at-oracle` in the ledger: they need no work, and the ledger says so
with the two counts.

## Reproducing

```bash
cargo build --profile fastbuild --bin lccc
python3 tests/regression/check_llvm_ring_identities.py --gcc gcc-16
cargo test --profile fastbuild --lib facts::identities
```

The gate is registered in `.github/workflows/ci.yml` (frontend lane) and
mirrored as `llvm-ring-identities` in `scripts/ci_local.sh`; the lane split was
recomputed by `scripts/ci_lane_split.py` rather than edited by hand.
