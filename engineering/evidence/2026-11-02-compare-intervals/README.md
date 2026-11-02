# The operand interval fold — `cmprange`

`simplify_cmp` used to answer a comparison by looking at the constant in front of
it. This round it also looks at the *interval the other operand can hold*: a
widening cast's source type, or the compare width's own domain. When the
interval decides the question, the comparison is a constant.

The rule is §16 of [SCALAR-PREDICATE-FOLDS.md](../../llvm-missed/SCALAR-PREDICATE-FOLDS.md).
The knob is `CCC_SIMPLIFY_SKIP=cmprange` (on unless skipped), so every number
below can be reproduced exactly.

## Files

| file | what it is |
| --- | --- |
| `intervals.c` | the probe: every comparison whose interval decides it, **one comparison per function**, plus five refusals |
| `intervals-lccc.s`, `intervals-gcc.s` | `-O2 -S` from LCCC and from the GCC available in this sandbox (GCC 12.2; CI runs 16.2 — the same shapes) |
| `sweep-cmprange.json` | the corpus A/B over the same 3,128 files as `2026-11-01-predicate-materialization`, run with `--knob CCC_SIMPLIFY_SKIP --knob-value cmprange` |

## Instruction counts (`-O2`, body only, `ret` included)

| probe | interval off | interval on | gcc |
| --- | --- | --- | --- |
| `uchar_le` `(unsigned char)x <= 255` | 4 | **2** | 2 |
| `uchar_lt` `(unsigned char)x < 256` | 5 | **2** | 2 |
| `uchar_gt` `(unsigned char)x > 255` | 4 | **2** | 2 |
| `uchar_eq` `(unsigned char)x == 300` | 5 | **2** | 2 |
| `ushort_le` `(unsigned short)x <= 65535` | 4 | **2** | 2 |
| `uint_le` `(unsigned int)x <= 4294967295u` (from `long`) | 4 | **2** | 2 |
| `schar_lt` `(signed char)x < 128` | 5 | **2** | 2 |
| `schar_gt` `(signed char)x > 127` | 4 | **2** | 2 |
| `schar_ge` `(signed char)x >= -128` | 4 | **2** | 2 |
| `schar_eq` `(signed char)x == -200` | 5 | **2** | 2 |
| `sshort_le` `(short)x <= 40000` | 5 | **2** | 2 |
| `int_gt_max` `x > INT_MAX` | 4 | **2** | 2 |
| `int_lt_min` `x < INT_MIN` | 4 | **2** | 2 |
| `uint_le_max` `x <= 4294967295u` | 4 | **2** | 2 |
| `uint_gt_max` `x > 4294967295u` | 4 | **2** | 2 |
| `int_ule_neg1` `(unsigned)x <= (unsigned)-1` | 4 | **2** | 2 |
| `keep_uchar_lt` `(unsigned char)x < 100` | 4 | 4 | 4 |
| `keep_schar_gt` `(signed char)x > -100` | 4 | 4 | 4 |
| `keep_schar_ult` `(unsigned)(signed char)x < 300` | 5 | 5 | 4 |
| `keep_uint_ge_max` `x >= 4294967295u` | 4 | 4 | 4 |
| `keep_int_eq` `x == 5` | 4 | 4 | 4 |

Sixteen comparisons lose two or three instructions each and land exactly on GCC's
sequence; the five refusals keep their compare, which is the point of testing
them (a fold that answers one of those is wrong, not fast). `keep_schar_ult` is
the one refusal GCC still beats: it reads an unsigned question about a
sign-extended value (`notl; shrl $31` — the sign bit decides every case), a
cross-domain rule this fold does not implement yet.

## Corpus A/B

`sweep.py --knob CCC_SIMPLIFY_SKIP --knob-value cmprange`, `-O2`, 3,128 files
(`tests` + `engineering`), plus the pre-existing `many-logical-ops.c` timeout
asymmetry the sibling evidence directory documents:

| with the rule | files |
| --- | --- |
| shorter | 13 |
| longer | **0** |
| byte-identical | 2,320 |
| both configurations reject | 794 |

Net −545 bytes of text. Most of it: `range_fold_branch.c` 1093 → 1011,
`three_way_boolean_compare.c` 2162 → 2092, then `ivw_wrapv_range.c`,
`range_fold_edges.c`, `vec_range_mask_fusion{,_sse2}.c`,
`vec_u16_saturating_sub.c`. A fold that replaces a comparison with a constant
cannot make a function longer, and the sweep says it did not.

## What this round found

Two defects, both caught before the fold left the branch:

1. **A constant wider than the compare.** The first version read the constant as
   a plain `i64`. `x >=s 4294967295` on a 32-bit operand asks about the pattern
   `0xffffffff`, which is `-1` signed; read as 4294967295 the interval says
   "false for every x" and miscompiles. The fold now carries both readings of
   the constant's low `ty` bits (`c_unsigned`, `c_signed`) and the unit test
   `operand_intervals_decide_comparisons` pins three cases of it.
2. **A pair-dependent fold in `range_fold_branch`.** `(unsigned char)x >= 0 &&
   (unsigned char)x <= 255` used to fold as a *pair*. Making the first half a
   constant (which §15's zero rule does) left the second half as a narrowed
   compare — `cmpb $255, %dil; setbe` — and the gate failed on CI. The interval
   rule answers each half on its own, and the gate now checks nine
   single-comparison probes (`single_*`) with the fold disabled as its mutation
   control: 14 failures with `CCC_SIMPLIFY_SKIP=cmp`, 0 without.

Found while diffing the probe against GCC and fixed in the same round (x86
instruction selection): a 32-bit compare against a constant with the top bit set
was staged through a register (`movl $4294967295, %ecx; cmpl %ecx, %edi`) even
though the encoding takes the bit pattern verbatim. `x == 4294967295u` and every
compare against a hash constant with the top bit set (`2654435761u` → `cmpl
$-1640531535`) now cost one instruction less; the instruction-selection unit test
`compare_against_wide_constant_uses_the_imm32_pattern` pins the 32-bit immediate,
the 64-bit sign-extended control, and the 64-bit-pattern refusal.

## Reproducing

```sh
scripts/build_lccc_fast.sh
cd engineering/evidence/2026-11-02-compare-intervals
C=../../../target/fastbuild/lccc
$C -O2 -S intervals.c -o intervals-lccc.s
CCC_SIMPLIFY_SKIP=cmprange $C -O2 -S intervals.c -o intervals-off.s
gcc -O2 -S intervals.c -o intervals-gcc.s
python3 ../2026-11-01-predicate-materialization/sweep.py --bin "$C" \
    --roots ../../../tests ../../../engineering --jobs 2 \
    --knob CCC_SIMPLIFY_SKIP --knob-value cmprange --json sweep-cmprange.json
```

The CI gate that holds the shape is
[`check_range_fold_branch.sh`](../../../tests/regression/check_range_fold_branch.sh)
§4b, which asserts each single comparison is constant *and* executes the probe.
