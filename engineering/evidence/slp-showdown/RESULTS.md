# BB-SLP Oracle Showdown — Session 48 (CE hard data)

Source: `slp_showdown.c` (12 straight-line SLP pattern functions; only
basic-block SLP can vectorize them — no loops).

Command:
```
scripts/godbolt.py compare engineering/evidence/slp-showdown/slp_showdown.c \
  --local target/fastbuild/lccc --flags "-O2 -march=x86-64-v3" \
  --oracles gcc16.2,clang --artifact-dir .../oracle --json .../manifest.json
```

## Result

|             | lccc | gcc 16.2 | clang 23.1 |
|-------------|------|----------|------------|
| total insns | 69   | 50       | 50         |
| vectorized  | 12/12| 12/12    | 12/12      |

**Vectorization coverage: 12/12 — full parity with GCC and Clang** (the
first LCCC revision that vectorizes any of these patterns).

## Per-function

function    lccc gcc clang  | note
add_b16        5   4    4
add_q4         7   5    5  | frame only
copy_b16       5   3    3  | frame only
copy_d2        3   3    3  | exact parity
copy_q4        6   4    4  | frame only (vmovdqu pair == gcc)
copy_w4        3   3    3  | exact parity
ksub_w4        8   4    4  | gcc's pcmpeqd+vpaddd all-ones idiom
madd_q4        8   6    6  | frame only
mixed          7   5    5  | frame only
mul_h8         5   4    4  | one staging move
sub_d2         5   4    4  | one staging move
xor_q4         7   5    5  | frame only (vmovdqu/vpxor/vmovdqu == gcc)

## Analysis

The COMPUTE instructions are instruction-for-instruction identical to GCC
on every function (e.g. xor_q4: `vmovdqu/vpxor mem,ymm0,ymm0/vmovdqu/
vzeroupper`). The +19 total gap decomposes as:

1. **Dead stack frames (~2 insns/function)**: every vector value gets a
   protected 32/16-byte slot; values that the deferral/memfold machinery
   never materializes still reserve frame space (`subq $152/addq $152`).
   Follow-up: slot elision for fully-deferred values — requires proving
   the emit-time bail paths (`try_elide_vec_load`'s pending-memfold
   conflict, defensive flushes) cannot fire for analysis-admitted
   values. NOT done in v1: the bails are the deferral's soundness net.
2. **ksub_w4 (4 insns)**: GCC's `vpcmpeqd` all-ones + `vpaddd` idiom for
   `x - 1` vs LCCC's const materialization (movl/movd/pshufd). Needs a
   ones-vector intrinsic family.
3. **One staging move** in mul_h8/sub_d2 (xmm0 handoff).

Benchmark A/B (isolated, taskset-pinned): sha256_transform 0.989 (1.1%
faster with SLP — the 4xU32 schedule-word pack), glibc_memcmp 1.003
(neutral; fires only in cold check code), all other corpus benchmarks
byte-identical binaries (no SLP fires). No regressions.

## Post-audit verification (2026-09-16, S29)

After the v2 soundness overhaul (three P0 classes closed: dangling lane
uses via the canonical use walkers, the store→load forwarding hazard via
rule (e), invisible vector memory ops via canonical classification; plus
the C11 6.7.3.1 restrict escape), the corpus re-verified:

- Vectorization coverage: **12/12 unchanged** (recount must include the
  F64 mnemonics movupd/movapd/vsubpd — a movdqu-only grep undercounts
  by exactly the three F64 functions).
- Total instructions: **69 — identical** to the pre-audit binary.
- Oracle rank corpus (102 functions, -O2 -march=x86-64-v3, vs
  best-of-gcc16.2/clang23.1/icc/icx): gap 2669 → 2668, **zero
  regressions**, sha256_transform −1.

The soundness work cost no measurable codegen quality, and lccc now
compiles the shifted-alias interleaved-ALU shape CORRECTLY where
gcc 16.2 -O2 miscompiles it (verified: gcc reads the pre-store value
where the scalar chain forwards).
