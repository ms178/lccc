# Follow-up: PR #705 — lane-mask provenance, value-context ranges, and the stress oracle

Session of 2026-10-01.  Base: `ms178/lccc` main `8db75621` (PR #704) + PR #705
(`70a8e3fa`..`9e06b0f4`).  Read this before touching the byte/word map parser
(`parse_byte_map_expr`) or `scripts/vectorize_stress.py`.

## Verdict on the review of PR #705

The Review AI was right on all four findings, and its central diagnosis —
*"range is not mask provenance, and a comparison's packed-mask range is not its
C-value range"* — is exactly the bug.  Every finding was reproduced with an
executable test before any code was changed (`tests/regression/vec_mask_provenance.c`
failed **31** kernel/type combinations on the PR binary).

| Finding | Verdict | Evidence |
|---|---|---|
| P1 numeric `[-1,0]` mistaken for a mask | **Agree, reproduced.** | `x + (p > 0 ? -1 : 0)` gave `+1` instead of `-1` at byte *and* word lanes, in the vector body **and** the scalar remainder (n = 1 fails). The `&`/`|` → `MaskConj` recovery had the same hole (`(a ? -1 : 0) & (b ? -1 : 0)`, `|`, `^`). |
| P1 demask after range decisions | **Agree, reproduced.** | `(x > 5) > 255` (byte), `> 65535` (all widths), `< 256`: the constant wrapped on the packed range `[-1,0]` and the compare inverted. |
| P1 stress `full` profile is UB | **Agree, reproduced with UBSan.** | `cond_inc_i32`, `cmp_sub_i64`, `cmp_acc_map_i32`, `cond_inc_i64` all trap (`INT_MAX + 1`, `INT64_MIN - 1`). GCC `-O0` is not an oracle for UB. |
| P2 census misses real SIMD | **Agree.** | `pxor/vpxor/xorps` were excluded unconditionally and packed loads/stores were never counted; the "ends in `sd`" test also dropped packed `vpabsd`/`vpmaxsd`. |

What the review did **not** find (it cannot run code) — three further, *pre-existing*
miscompiles that the new tests exposed:

1. **Out-of-domain compare constants were wrapped modulo the lane width for
   WIDE compares** (`try_wrap_lane_cmp_constant`).  `uint8_t x > -1` (always 1)
   became `x >u 255` (always 0); `x < 300`, `x == 256`, `x < -86`, `int8_t x == 170`,
   `x >= -200`, `x > 300 ? 1 : 2` … — **14 kernels** diverged from GCC on the PR
   baseline.  The wrap is only exact for a compare *carried out at the lane width*
   (`Instruction::Cmp::ty.size() == elem_bytes`).
2. **Same-width signedness casts kept the operand's interval** (`i8`→`u8`):
   `(uint8_t)x > 0x7F` on an `int8_t` stream compared the stale `[-128,127]`.
3. A **truncated mask** (`(uint8_t)(a > b)`) lost its mask provenance behind the
   lane-domain interval.

Where the review was imprecise: it suggests keeping *genuine mask-valued select
arms* normalised before the union.  Doing that unconditionally would reject
`(a && b) ? x : y` written as `select(a, b, 0)` as a blend condition (a regression
in vectorization).  A select of two masks (or a mask and the constant 0) is itself
a mask whose C value is the 0/1 of its packed lanes, so it keeps raw arms; only
*mixed* arms (`c ? (a > k) : 2`) are normalised.

## What changed

`src/passes/vectorize.rs`

* `is_lane_mask` is **structural provenance**: `Cmp`, `MaskConj`, or a `Select`
  whose arms are masks/zero with ≥ 1 genuine mask (`select_arms_are_masks`).
  A numeric `p ? -1 : 0` is *not* a mask.
* `demask_operand` requires provenance **and** `MASK_RANGE`.
* `And`/`Or` → `MaskConj` recovery requires `is_lane_mask` on both operands.
* **Value-context normalisation happens before any range decision**: `Cmp`
  operands are demasked before domain selection / constant wrapping; `Select` arms
  are demasked before the range union and the min/max / short-circuit folds;
  truncation of a mask demasks first (exact `[0,1]`).
* `demask_value_positions` keeps the arms of a mask-valued select packed when the
  select sits in a *condition* position (a blend keys on the top bit; an arm
  demasked to 0/1 would never take the true path) and demasks them in value
  positions.
* Constant wrapping gated on `ty.size() == elem_bytes`.
* Same-width casts re-domain the interval (lane-width reinterpretation → target
  domain; wider → keep only a non-negative interval; no-op / mask → unchanged).

Tests (all failing on the PR baseline, all passing now; hashes equal GCC's):

* `tests/regression/vec_mask_provenance.c` (+ `_sse2`): 37 kernels × 6 lane types ×
  71 trip counts × 3 misalignments, each result checked against an in-program
  *volatile-load scalar oracle* (no vectorizer can touch it) **and** diffed with GCC.
  Defined C (UBSan-clean).
* `tests/regression/vec_narrow_cmp_constants.c`: 26 kernels × 6 types; includes the
  legitimate lane-typed spellings (`(uint8_t)x == 0xAA`) so the fix cannot "win" by
  refusing to vectorize them.
* Rust unit tests in `map_expr_interpreter_tests`: provenance is structural;
  numeric `-1/0` trees are untouched by the rewrite and keep their C value in
  *both* interpreters; mask-select is a mask in conditions and a bool in values;
  compare-of-compare sees the boolean (checked against an independent source-level
  oracle, > 1000 lane/constant combinations).
* `scripts/test_vectorize_stress.py` (wired into `ci_local.sh` + `ci.yml`): census
  fixtures (known copy / XOR / zero-idiom / scalar listings) and a UBSan run of the
  signed accumulating reference kernels.

`scripts/vectorize_stress.py`: `$U` placeholder (unsigned twin); `cond_inc`,
`cmp_sub`, `cmp_acc_map` compute in the unsigned twin; `is_packed_simd` is
operand-aware.

## Evidence / method (no PMU here)

Correctness evidence only: exhaustive trip-count × alignment × lane-type sweeps with
an independent oracle, UBSan on every reference program, A/B against
`CCC_DISABLE_PASSES=vectorize,slp,slp_late,iv_widen`.  **No performance numbers are
claimed**: this change only removes miscompiles and (deliberately) keeps wide
compares with out-of-domain constants scalar.

## To-do for the next agent

1. **Run the full `scripts/ci_local.sh` (mode=full) on this tree** — only the
   vectorizer-relevant gates, the whole `run_regression.py` corpus and the
   relevant `cargo test` modules were run in this session.
   `scripts/check_ci_gate_parity.py` could not run (no PyYAML on the sandbox);
   the two new gate lines mirror each other textually.
2. **Fold, don't scalarise, impossible wide compares.**  `x_u8 > -1`, `x < 300`,
   `x == 256` are now (soundly) left scalar.  Range analysis knows the answer
   (`always true` / `always false`): emit a constant mask instead.  Low risk, tiny
   code; mostly a robustness win (these are the shapes sanitizer-clean code rarely
   writes, but macro-expanded range checks do).
3. **Full `vectorize_stress.py` UBSan sweep**: this session ran the three fixed
   kernels × {i8,i16,i32,i64,u32}.  Run every generated case under UBSan once
   (`gcc -fsanitize=undefined -fno-sanitize-recover=all`), and consider making it a
   mode of the script (`--ubsan-reference`) so the oracle's validity is always
   checked, not just for the kernels named in the review.
4. The dword (`parse_tree`) path requires a literal `Cmp` select condition and was
   not changed; the final `demask_value_positions` pass is structural there and
   was verified by the i32/u32 rows of the new tests.  A `(a && b) ? x : y`
   *value* form at dword lanes is still scalar (no regression, a possible win).
5. **Performance watch** (from the review): each `mask_to_bool` adds two
   `MapExpr` nodes against the 20-node cap.  Measure on the benchmark corpus
   (`scripts/bench_kernels.py`) on real hardware whether any previously
   vectorized map now falls back to scalar; none was observed in the regression
   corpus run here.
6. Fuzz the parser with the structural oracle: generate random `MapExpr` trees
   *from C expressions* (not hand-built) through `scripts/fuzz_diff.py`'s
   `stress_suite` engine with `vec_mask_provenance.c`'s volatile-oracle pattern.
