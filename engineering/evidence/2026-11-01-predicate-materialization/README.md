# Predicate materialization — measurements

Evidence for §13 of
[`engineering/llvm-missed/SCALAR-PREDICATE-FOLDS.md`](../../llvm-missed/SCALAR-PREDICATE-FOLDS.md):
`c ? 1 : 0` becomes `c != 0`, `c ? 0 : 1` becomes `c == 0` (or the inverted
comparison itself), and the mask merges `(a != 0) | (b != 0)` -> `(a | b) != 0`
and `(a == 0) & (b == 0)` -> `(a | b) == 0`.

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
| x86-64 | 3,116 | **83** | 8 | 794 | 0 | **−218** |
| i686 | 3,118 | **20** | **0** | 1,847 | 0 | **−98** |
| AArch64 | 3,118 | **19** | **0** | 1,847 | 0 | **−103** |

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
| `tests/benchmark/programs/linux_list_sort.c` | 232 | 246 | `merge_and_count` 38 -> 50: the comparison feeding the merge's ordering decision becomes a `setcc`-based value, and if-conversion/rotation picks a straight-line form with 3 more branches instead of the pre-fold branchy one |
| `tests/regression/loop_rotate_pressure_gate.c` | 169 | 176 | same class: a rotation decision re-evaluated against the smaller loop body |
| `tests/regression/bb_slp_v3.c` | 1,344 | 1,346 | instruction-selection knock-on in an SLP unit (+2) |
| `tests/regression/vec_u16_saturating_sub.c` | 1,379 | 1,381 | same (+2) |
| `tests/regression/simd_sse2_arith.c` | 453 | 454 | register-allocation knock-on (+1) |
| `tests/regression/sqlite_vdbe_peephole.c` | 229 | 230 | one branch is removed (a `jne`/`jmp` pair collapses into a straight-line `cmov` form) and one spill is added: +1 static, one branch less |

These were measured with the fold on and off on the same binary, and every one of
them is a downstream shape decision (vectorization, rotation, allocation), not a
longer lowering of the same instruction: the fold itself replaces one instruction
with one, or splices two for three.

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
