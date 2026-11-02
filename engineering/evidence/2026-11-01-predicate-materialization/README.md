# Predicate materialization — measurements

Evidence for §13 of
[`engineering/llvm-missed/SCALAR-PREDICATE-FOLDS.md`](../../llvm-missed/SCALAR-PREDICATE-FOLDS.md):
`c ? 1 : 0` becomes `c != 0`, `c ? 0 : 1` becomes `c == 0` (or the inverted
comparison itself), and the mask merges `(a != 0) | (b != 0)` -> `(a | b) != 0`
and `(a == 0) & (b == 0)` -> `(a | b) == 0`.

It also carries the two folds §13 and §14 needed to reach the comparators they
describe: the boolean-leaf spelling of a select (`Cast(cmp)` read as `{1, 0}`,
which turns an inlined lexicographic comparator back into a single `cmp`) and
`booldiff`, the trichotomy fold for `(a > b) - (a < b)` compared against zero.

The `before` column is the `CCC_NO_PREDICATE_MATERIALIZE=1` build, which is
exactly the pre-fold spelling of every shape — the fold is the only thing that
knob turns off. The knob and the fold share one resolution, and the fold runs in
the **post-if-conversion** `bit_idioms` invocation only: converting `c ? 1 : 0`
before the earlier passes have had their turn costs more than it gains (§13
records the `signext_move_cross_block.c` measurement that found this).

| File | What it holds |
| --- | --- |
| `predicate-materialize.c` | 11 shapes (both polarities, int and `u32`/`u64` carriers, plain-int conditions) and 6 controls that must not fold |
| `names.txt` | the probe function names the counter counts |
| `predicate-counts.txt` | the four columns of the table below |
| `predicate-lccc-before.s`, `predicate-lccc-after.s`, `predicate-gcc.s`, `predicate-clang.s` | the dumps those columns came from (`-O2`, gcc 16.2.0, clang 23.1.4) |
| `cross-target.sh` | rebuilds the fold-on/fold-off counts on all four targets |
| `cross-target-counts.txt` | its output, checked in |
| `sweep-x86_64.json`, `sweep-i686.json`, `sweep-aarch64.json` | the corpus A/B, run with `sweep.py` (`--knob CCC_NO_PREDICATE_MATERIALIZE`) |
| `sweep-booldiff.json`, `sweep-booldiff-i686.json`, `sweep-booldiff-arm.json` | the corpus A/B for the trichotomy fold alone (`--knob CCC_SIMPLIFY_SKIP --knob-value booldiff`) |
| `../../tests/regression/three_way_boolean_compare.c` | the valence test: every predicate × every width × both operand orders against a branch-written (`volatile`-forced) reference, plus the refusal shapes -- mixed classes, different pairs, a zero-extending carrier, NaN |
| `sweep.py`, `count.py` | the sweep and the counter, shared with the sibling evidence directories |

## x86-64, one column per compiler (`-O2`)

| function | before | after | gcc | clang |
| --- | --- | --- | --- | --- |
| `from_cmp` `a != b ? 1 : 0` | 5 | **4** | 4 | 4 |
| `from_cmp_neg` `a != b ? 0 : 1` | 5 | **4** | 4 | 4 |
| `from_lt` `a < b ? 1 : 0` | 5 | **4** | 4 | 4 |
| `from_lt_neg` `a < b ? 0 : 1` | 5 | **4** | 4 | 4 |
| `zero_or_one` `c ? 1u : 0u` | 5 | **4** | 4 | 4 |
| `one_or_zero` `c ? 0 : 1` | 5 | **4** | 4 | 4 |
| `mask_of_sign` `x < 0 ? 0u : 1u` | 5 | **4** | 4 | 4 |
| `zero_or_one64` `c ? 1ull : 0ull` | 5 | **4** | 4 | 4 |
| `or_or` `(a != 0) \| (b != 0)` | 9 | **4** | 4 | 4 |
| `and_eq` `(a == 0) & (b == 0)` | 9 | **4** | 4 | 4 |
| `or3` (the chain form) | 13 | **5** | 5 | 5 |
| `keep_and_ne` `(a != 0) & (b != 0)` | 9 | 9 | 7 | 7 |
| `keep_or_eq` `(a == 0) \| (b == 0)` | 9 | 9 | 7 | 7 |
| `keep_xor` `(a != 0) ^ (b != 0)` | 9 | 9 | 7 | 7 |
| `keep_three_two` `c ? 3 : 2` | 5 | 5 | 4 | 4 |
| `keep_one_two` `c ? 1 : 2` | 5 | 5 | 4 | 4 |
| `keep_shared` (`t ? 0 : 1`, `t` also stored) | 9 | 9 | 7 | 7 |

Every shape lands exactly on both oracles. The controls are the point of the
table: `keep_and_ne` is the counter-example that makes the merge a law rather
than a pattern (`a = 1, b = 2` shares no bit and is nonzero twice), `keep_or_eq`
is its De Morgan-mixed sibling, `keep_xor` is a different operation, the two
constant arms are not booleans, and `keep_shared` is the negative polarity with a
boolean the fold must not test a second time.

## Cross-target (`cross-target-counts.txt`)

Fold off -> on, same probe, `-O2`. No function on any target got longer; every
target improves, and i686/AArch64/RISC-V gain more than x86-64 because their
`setcc`/`cset`/`sltiu` forms were further from the `mov`/`cmov` spelling:

| shape | x86-64 | i686 | AArch64 | RISC-V |
| --- | --- | --- | --- | --- |
| `from_cmp` | 5 -> 4 | 11 -> 9 | 13 -> 10 | 16 -> 9 |
| `zero_or_one` | 5 -> 4 | 12 -> 6 | 12 -> 9 | 12 -> 12 (unchanged) |
| `or_or` | 9 -> 4 | 19 -> 13 | 16 -> 11 | 20 -> 14 |
| `or3` | 13 -> 5 | 24 -> 16 | 23 -> 15 | 30 -> 18 |

## Corpus A/B (fold off vs on, `-O2`)

The sweep compiles every `.c` under `tests/` and `engineering/` twice with the
same binary, with `CCC_NO_PREDICATE_MATERIALIZE=1` as the only difference, and
compares instruction counts. `verdict: 0` means the fold changes no acceptance
decision; `both-reject` files fail identically in both builds (missing headers,
`-m32`, target intrinsics).

| target | files | shorter | longer | both-reject | verdict changes | net instructions |
| --- | --- | --- | --- | --- | --- | --- |
| x86-64 | 3,124 | **85** | 8 | 794 | 0 | **−211** |
| i686 | 3,124 | **20** | **0** | 1,853 | 0 | **−98** |
| AArch64 | 3,124 | **19** | **0** | 1,888 | 0 | **−103** |

i686 and AArch64 never lengthen a file; the eight x86-64 files that do are the
table below. `sweep.py --timeout` exists because
`tests/corpus/clang-c/Sema/many-logical-ops.c` takes ~100 s with *and* without
the knob (measured both ways) -- a pre-existing compile-time outlier, not this
fold.

The wins beyond the probes are not contrived shapes; they are the idiom in real
test code:

| file | on | off | delta | shape |
| --- | --- | --- | --- | --- |
| `tests/regression/iv_widen_scaled_index.c` | 309 | 369 | **−60** | `a[i] * (a[i] > 0 ? 1 : 0)` |
| `tests/regression/vec_class_union_ifcombine.c` | 712 | 753 | **−41** | `d[i] = (cls(c)) ? 1 : 0`, now a correct vector mask store |
| `vec_class_union_ifcombine_sse2.c` | 719 | 760 | **−41** | the `-march=x86-64-v3` twin |
| `tests/regression/range_fold_branch.c` | 1,093 | 1,121 | **−28** | `in * 7 + (in ? 1 : 0)` |
| `tests/regression/bool_thread_fse_tail_overrun.c` | 899 | 917 | **−18** | boolean-valued tail guard |
| `tests/regression/ext_vector_temp_init.c` | 500 | 514 | **−14** | `? 1 : 0` in a vector element initializer |
| `tests/regression/vec_byte_lane_map.c` (+`_sse2`) | 1,179 | 1,193 | **−14** | byte map with a `? 1 : 0` element |

### The eight longer files, named

| file | before | after | why |
| --- | --- | --- | --- |
| `tests/regression/vec_signed_range_fusion.c` | 854 | 924 | **vectorized**: with the select gone, `d[i] = (c >= 'a' && c <= 'z') ? 1 : 0` parses as a map expression and gets a 4-lane SSE2 body. `k_s_classify` is 46 instructions with 6 packed compare/logic ops in the fold-on build and 14 scalar instructions with **no** packed op at all in the fold-off build, so the extra static instructions buy four elements per iteration; the `& 1` materialization is part of that body |
| `tests/regression/vec_signed_range_fusion_sse2.c` | 827 | 901 | same kernel, `-march=x86-64-v3` corpus twin |
| `tests/regression/predicate_materialize_mask_store.c` | 408 | 460 | the probe for the vectorizer bug below: with the fold on, this kernel gets a 4-lane vector body instead of 8 scalar iterations, so the longer file is four elements per iteration |
| `tests/regression/loop_rotate_pressure_gate.c` | 169 | 176 | same class: a rotation decision re-evaluated against the smaller loop body |
| `tests/regression/bb_slp_v3.c` | 1,344 | 1,346 | instruction-selection knock-on in an SLP unit (+2) |
| `tests/regression/vec_u16_saturating_sub.c` | 1,379 | 1,381 | same (+2) |
| `tests/regression/simd_sse2_arith.c` | 453 | 454 | register-allocation knock-on (+1) |
| `tests/regression/sqlite_vdbe_peephole.c` | 229 | 230 | one branch is removed (a `jne`/`jmp` pair collapses into a straight-line `cmov` form) and one spill is added: +1 static, one branch less |

These were measured with the fold on and off on the same binary, and every one of
them is a downstream shape decision (vectorization, rotation, allocation), not a
longer lowering of the same instruction: the fold itself replaces one instruction
with one, or splices two for three.

## The trichotomy fold, alone (`booldiff`)

§14's fold is switched off by `CCC_SIMPLIFY_SKIP=booldiff` and by nothing else,
so the same sweep A/Bs it directly: default build versus `booldiff` skipped,
everything else equal.

| target | files | shorter | longer | both-reject | verdict changes | net instructions |
| --- | --- | --- | --- | --- | --- | --- |
| x86-64 | 3,124 | 1 | **0** | 794 | 0 | **−1,384** |
| i686 | 3,124 | 0 | **0** | 1,853 | 0 | 0 |
| AArch64 | 3,124 | 0 | **0** | 1,888 | 0 | 0 |

The one shorter file is the valence test itself
(`tests/regression/three_way_boolean_compare.c` -- every inlined comparator in
it loses four to eight instructions). The i686 and AArch64 rows are zero
because neither target can *compile* that file here (this sandbox has no 32-bit
or cross libc: 1,853 and 1,888 files are `both-reject`, and the valence test is
one of them), so for those two targets the sweep proves the invariant the fold
must keep -- nothing moved, nothing got longer -- while the probe table below
proves the fold itself fires there, and `check_predicate_materialize.sh` builds
those probes on every target as part of CI. No other file in the
corpus asks a question about the difference of two indicators, which is the
reason the corpus total is small: the fold answers a shape that appears in code
that writes its three-way comparison out by hand, and the fold is deliberately
not extended to the `(a < b) ? -1 : (a > b)` spelling -- there the value comes
from two selections, so nothing proves the two indicators are complementary.

Zero longer files is the number that matters here, and it is not an accident of
the corpus: the rewrite deletes at least four instructions (two `setcc`, two
`movzbl`, the `sub`, one `cmp`) and adds one `cmp`, and every link in the chain
has to have a single reader before it fires. The measurements behind the
probes, on the four targets:

| probe | x86-64 | i686 | AArch64 | RISC-V |
| --- | --- | --- | --- | --- |
| `int t = (a > b) - (a < b); return t <= 0` | 9 -> **4** | 19 -> **9** | 16 -> **10** | 17 -> **9** |
| `... return t < 0` | 9 -> **4** | 19 -> **9** | 16 -> **10** | 17 -> **8** |
| `long t = (a > b) - (a < b); return t <= 0` | 10 -> **5** | 19 -> **9** | 19 -> **13** | 19 -> **11** |
| `int t = (a >= b) - (a > b); return t > 0` | 9 -> **4** | 19 -> **9** | 16 -> **10** | 17 -> **9** |

GCC 16.2 spends 10 instructions on the first row and Clang 23.1.4 four, so the
fold leaves one target behind nobody and one ahead of GCC by six. The remaining
`xorl` is §13's materialization, and `three_way_val` keeps a `movzbl`/`movslq`
pair only because the value is returned in a 64-bit register.

`tests/regression/three_way_boolean_compare.c` is what keeps this honest: 8
widths × 16 predicates (every comparison, both operand orders, both signedness
classes, the unsigned-outer forms and two controls) over 11 edge values each,
20,000 seeded pairs per width family and a full ±128 × ±128 sweep -- 231,000
comparisons, each checked against a reference that writes the branch out. It
prints `VALIDATION FAILED (n)` with the first mismatch; mutating the `<=` mapping
to `>=` produces 231,000 of them.

## The miscompile this round found, and the design that removed its class

The first version of the indicator fold tabulated its answers by hand and
peeled every integer cast between the difference and the comparison. Both
choices were wrong, and both were caught by measurement rather than by reading:

| defect | how it showed | after |
| --- | --- | --- |
| peeling a **zero-extension** (`unsigned char d = (a > b) - (a < b); return (int)d < 0;`) folded to `a < b` | the probe printed `1001456958` where LCCC-with-the-knob-off, GCC 16.2 and Clang 23.1.4 all printed `2984546946` | the peel refuses to cross a widening cast whose source is unsigned (`peel_sign_preserving_casts`); the probe prints `2984546946` |
| the hand table mirrored the predicate for the reversed operand order **before** applying the unsigned rules, so `(a < b) - (a > b) >u 0` was answered "never" | `VALIDATION FAILED (125610)` in the valence test once the reversed-unsigned rows were added | the fold enumerates the three trichotomy states instead of tabulating: there is no order of operations left to get wrong |

The second defect is why the fold is written as an enumeration now. Three rows
(two indicators × one question), evaluated and then *named* by
`int_cmp_from_trichotomy_mask` -- the helper the select-leaf fold already used,
so the two folds share one mapping instead of two tables that must be kept in
step. It also made the nested-indicator pair (`(a >= b) - (a > b)`, the equality
indicator) and any same-class predicate pair fall out for free, which is what
§14 measures as 9 → 4 on x86-64.

## The zero-extension rule (§15)

While the indicator fold was refusing to cross a zero-extension, the *unsigned*
question it answers was still open on its own: `(int)(unsigned char)c < 0` was
four instructions here and two in both oracles. `simplify_cmp` now folds the
four signed questions against zero through a zero-extension (`< 0` is false,
`>= 0` is true, `<= 0`/`> 0` are `== 0`/`!= 0` at the narrow width), which is
what turns the `unsigned char` carrier of the probe below from ten instructions
into two.

## The vectorizer bug this round found

`k_s_classify` in `vec_signed_range_fusion.c` failed its own validation at `-O2`
once the fold was in place: the vector body stored a packed compare
(`0xFFFFFFFF`/`0` per lane) where the C program stores `1`/`0`. The scalar
remainder loop was correct, so only trips long enough to enter the body were
wrong. The map vectorizer assumed a comparison's packed value could be stored
directly; `rewrite_zero_one_selects` already materialized `& 1` for the
`Select(c, 1, 0)` spelling of the same value, and a mask-shaped root now gets the
same treatment. `tests/regression/predicate_materialize_mask_store.c` (flags
`-O2`) pins it: it fails against a build without the normalization and passes
with it, and the `check_predicate_materialize.sh` gate runs the same kernel at
`-O1`/`-O2`/`-O3`.

## Reproducing

```sh
scripts/build_lccc_fast.sh
cd engineering/evidence/2026-11-01-predicate-materialization
C=../../../target/fastbuild/lccc
$C -O2 -S predicate-materialize.c -o predicate-lccc-after.s
CCC_NO_PREDICATE_MATERIALIZE=1 $C -O2 -S predicate-materialize.c -o predicate-lccc-before.s
./cross-target.sh "$C" predicate-materialize.c
python3 sweep.py --bin "$C" --roots ../../../tests ../../../engineering --jobs 2
```

`ccc-i686`/`ccc-arm`/`ccc-riscv` are `target/fastbuild/lccc-i686` and friends, as
the sweep's `--bin` argument.
