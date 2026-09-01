# Follow-up: the vectorized loop counter, fixed properly

Session date: 2026-09-01 (continuation)
Base: `ms178/lccc` main @ `38e3277f`
Snapshots: `S24`, `S25`

---

## 1. The NAK, and why it was right

The previous session found that vectorizing a loop corrupts any use of the loop
counter *after* the loop, and responded by **declining to vectorize** such
loops. That was correct but it was a workaround: it bought correctness with
throughput when both were available.

## 2. What the defect actually is

Both addressing schemes redefine what the counter counts:

* **byte-offset scheme** — steps `elem_size * vec_width` **bytes** per iteration;
* **element-index scheme** — divides the trip count, so the counter numbers
  **vector iterations**.

Inside the loop only addresses read it, so neither is visible there. A use
*after* the loop reads a number that is no longer the element index:

```c
for (; n < max; n++) acc += v[n];
return acc + (n >> 2);
```

| | result for `max == 32` |
|---|---|
| byte-offset scheme | 528 (`n` == 128) |
| element-index scheme | 497 (`n` == 4) |
| **GCC 16.2 / Clang 23.1 / ICC / ICX** | **504** (`n` == 32) |

## 3. The fix, which costs zero instructions

The transform **already builds** a scalar remainder loop whose induction
variable counts ELEMENTS: its preheader converts the vector counter back to an
element index (`v27 >> 2` for 4-byte elements), and it steps by one until it
reaches the ORIGINAL trip bound. **Its value on exit is exactly the final value
the source-level counter would have had.**

So the correct value already exists in the function. No arithmetic has to be
synthesized — the escaping uses simply have to name it.

That is strictly better than both alternatives:

* better than declining to vectorize — **measured 4.5x faster** (0.0125 s vs
  0.0557 s on a 4096-element reduction × 20 000);
* better than materializing `trips * vec_width + remainder` in the exit block —
  that adds instructions *and* has to be kept in agreement with the remainder
  loop on every exit path, which is precisely the fragility that produced the
  original bug.

**Dominance is structural, not assumed.** The vector loop's only exit goes to
the remainder preheader, and the remainder loop's only exit goes to the
original loop exit; every path leaving the nest passes through the remainder
header, so its phi dominates everything downstream. The rewrite is restricted
to blocks that existed *before* the remainder was created, which excludes the
remainder's own blocks — their use of the vector counter is the legitimate
byte-to-element conversion and must not be touched.

### 3.1 The exhaustive test found a second gap immediately

The first version of the fix touched only `transform_reduction_avx2`. The new
sweep caught that an **i64** reduction goes through `transform_reduction_sse2`
(2-wide) and still leaked a byte count — `max == 2` returned 19 instead of 5.
Both paths are fixed.

That is the value of sweeping element sizes rather than testing one type:
`sum_only` (counter does not escape) was correct for i64 all along, so only a
test that varied *both* the element size and whether the counter escapes could
separate the two.

### 3.2 What the test covers

`tests/regression/vectorized_counter_escapes.c` sweeps trip counts **0..34**
across eight shapes, comparing every result against GCC:

* trip counts that are **exact multiples** of the vector width, so the
  remainder loop runs **zero** times and its phi must still carry the
  preheader's start value;
* trip counts **below** the vector width, so the *vector* loop runs zero times;
* **zero** trip count, where the counter must remain 0;
* **4- and 8-byte** elements, which scale the byte counter differently and
  route through different transforms;
* the counter read **several times** and in several expressions;
* an **unsigned** counter;
* the counter **compared** rather than consumed arithmetically;
* an **early-exit** loop, whose final counter is *not* the loop bound and which
  therefore no `trips * vec_width` formula could ever reconstruct — only the
  real counter can.

## 4. GAS 2.47 for the MachInst differential

A differential is only as authoritative as the assembler behind it: passing
against Debian's 2.44 proves the text was valid for *that* release, not the one
the project targets. `find_assembler` now searches `$LCCC_GAS`, then the cache
`scripts/ensure_gas_247.sh` provisions, then system `as`, then `gcc -x
assembler`; it prints the binary **and its version** on every run and warns
explicitly when it is not 2.47.

GAS **2.47.20260726** was provisioned and all 28 MachInst tests — including the
4000-instruction randomized stress — pass against it.

## 5. Oracle standing

`long long reduce_esc(const int*, int)` at `-O3 -march=x86-64-v3`:

| compiler | instructions |
|---|---|
| **lccc** | **51** |
| ICX (latest) | 33 |
| ICC 2021.10 | 70 |
| Clang 23.1 | 73 |
| GCC 16.2 | 76 |

lccc beats GCC, Clang and ICC on this shape; ICX is still ahead.

---

## 6. Validation

| Gate | Result |
|---|---|
| `cargo test --lib` | **1468 pass / 0 fail / 6 ignored** |
| `./scripts/run_regression_suite.sh` | **PASS=561 FAIL=0 SKIP=15**, AB-diff 0 |
| `ir_verify_sweep.py --levels O0..Oz` | **0 violations** / 3366 configs |
| MachInst differential + fuzz | all accepted by **GAS 2.47** |
| `bench_kernels.py` | no regressions; geomean 0.673x vs GCC |

---

## 7. To do next, in priority order

### 7.1 `namechars` — the counting form of the classifier (worst kernel, 0.44x)

Still an **11-branch chain** per byte where GCC vectorises and ICX uses a
binary search. The cause is precise and unchanged:

* `int f(char)` returning 0/1 → if-converts to a `Select` chain → `range_fold`
  folds each `lo <= c && c <= hi` to one unsigned range test → `set_membership`
  merges `[a-z]`+`[A-Z]` into a single test on `c & ~32`. This path works;
  `k_classify` gets it.
* `if (pred) n++;` → the hit edges of every test converge on a **shared
  increment block** before the join, so no diamond is formed, no `Select` is
  produced, and the entire chain downstream is starved. `k_namechars` gets
  this.

The two bench kernels deliberately bracket the problem. The fix is a new
if-conversion pattern: *n* test blocks each branching to a common single-block
`HIT`, which joins with a phi `[hit_value, miss_value]`. That is exactly the
`Phi [(Const(1), T_1), …]` shape `set_membership`'s matcher already documents;
it needs to accept a non-constant hit value too. Recognising it converts the
whole chain to branchless arithmetic in one step.

### 7.2 Vectorisation breadth

GCC's 149/243-instruction outputs on `scan`/`match_len` are SIMD. lccc's scalar
`memchr` ties it here, but that will not hold on wider data.

### 7.3 Encoding-level differential

The MachInst differential proves GAS 2.47 *accepts* the text. The stronger
property is that the bytes match what GAS produces for the intended
instruction; `insndiff.py`/`encdiff.py` exist and could be pointed at the
MachInst corpus.

### 7.4 Def-dominates-use in the IR verifier; linker oracle (lld 23.1, mold 2.42,
bfd 2.47)

Both unchanged from previous sessions.

---

## 8. Notes for whoever picks this up

- **"Correct but slower" is not done.** The bail-out passed every test and was
  still the wrong answer; the information needed to do it properly (the
  remainder loop's element counter) was already sitting in the function.
- **Look for the value before synthesizing it.** The instinct was to compute
  `trips * vec_width + remainder` in the exit block. The transform had already
  computed the right number for its own purposes.
- **Sweep the dimensions, not a point.** One trip count and one element type
  would have shipped the AVX2 half of this fix and left i64 silently broken.
  The sweep found it on the first run.
- **A differential inherits the authority of its oracle.** Probing MachInst
  against whatever `as` is installed quietly weakens the strongest test in the
  suite; pin it and say so out loud.
