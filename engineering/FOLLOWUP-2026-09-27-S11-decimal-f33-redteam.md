# Follow-up: S10 rebase onto 19cf2b3c (#651) + S11 decimal F33 red-team

**Base:** upstream `19cf2b3c` (PR #651). S10 rebased the 6-commit stack
(6/6 replayed, conflicts resolved preserving both sides), revalidated
(asm-diff 1290/1290 x86-64 + 583/583 i686 against GAS 2.47 oracles,
battery 94/94, lib 3719/3719, harness PASS), and fixed a latent
`asmdiff.py --32` runner bug (lccc never got `-m32`: i686 went 7/15 to
15/15 on x87+pc8). S11 is a line-by-line red-team audit of the decimal
const core (`src/common/decimal.rs`, full file) that found and fixed
three silent wrong-code bugs plus two hardening issues, all GCC-proven,
with 4 unit pins, 10 battery pins, and a new differential fuzzer.
Patch: `ms178-1.patch` (snapshot entry `S11-…`, base `19cf2b3c`).

## S11.0: P0 — sequential sticky-bit rounding double-rounds (FIXED)

`round_to_prec` and the subnormal path rounded digit-by-digit with a
sticky bit. That double-rounds: an intermediate round-UP corrupts the
boundary digit while the stale sticky bit double-counts on a later
exact half. Witness `1.000000451DF` (= 1000000451e-9, prec 7):
sequential gives `1000001e-6`, correct single-decision gives
`1000000e-6`. Proven three ways: Python `Fractions` simulation of both
algorithms, `gcc -S` ground truth (`0x2F8F4240` = 797917760 for both
`1.000000451DF` and the second witness `1.0000004999DF`), and a failing
lccc probe (emitted 797917761 pre-fix). The battery missed it: no
prior case combined a multi-step removal with an UP-then-half/carry
near the boundary, so all 94 pins were single-step-shaped.

Fix: direct single-decision rounding in both paths. New primitive
`cmp_suffix_to_half` compares the entire dropped suffix against half a
kept unit once (first digit decides unless exactly 5, then tail
nonzero-ness, then kept parity). `round_to_prec` truncates + at most
one exact carry-shift; the subnormal path computes
coef = round-half-even(D / 10^shift) directly (shift > n → zero in
O(1), which subsumes the F28 hang cap bit-identically; shift == n →
0/1 by compare; shift < n → keep + one decision). Side effect: O(n²)
→ O(n) in literal length (the old loop re-scanned on every removal;
a compile-time DoS on mega-literals is gone by construction). No
existing pin changes (all are single-step or decisive-first-digit,
each hand-verified against the new rule before running). Regression
pins: `rounding_is_single_decision_no_double_round` (451/4999 → down,
501 → up), `subnormal_rounding_is_single_decision` (2.495e-101DF → 2;
old code gave 3), battery `rr451/rr4999/rr501/sub2495`.

MPFR/decNumber/strtod all round once; nobody chains rounded steps.
The old `div10_half_even` doc ("discarded by an earlier loop
iteration") now carries an explicit do-not-chain warning; its
`sticky` arm is retained only to pin the single-step primitive's
contract for exact-truncation callers (production passes false with
an exact zero removed digit).

## S11.1: P0 — decoder steering drops large+big-exponent values (FIXED)

`decode_bid32/64` required the large form's third steering bit clear
(`(v & 0x10000000) == 0`), but that bit is the exponent MSB, not
steering: Intel/754 large ⟺ top bits 11 after special exclusion, with
a full 8/10-bit exponent at 28..21 / 60..51. So any large coefficient
with biased exp ≥ 128/512 (D32 coef ≥ 2^23 with e ≥ 27; D64 coef ≥
2^53 with e ≥ 114) misrouted into the small arm and decoded to
garbage coef/exp. Reachability: `_Decimal64 g = 9000000e27DF;` folds
through `decode_bid32` (cross-width conversion in const_arith.rs,
casts in const_eval.rs:558/569) and produced SILENT wrong bits.
Emission was always correct (encoder bit-exact vs GCC), which is why
the bit-comparing battery never caught it.

Fix: delete the over-strict clause in both decoders (2 lines) +
module-doc correction ("steering bits 30..28 == 110" was imprecise
for exp ≥ 128). Ground truth: `gcc -S` gives 9000000e27DF =
0x70095440, 8388608e27DF = 0x70000000, 99000000000000000e114DD =
0x700B2BFF5F46C000 (bit 60 set, exp 513, coef 9900000000000000).
Regression pins: `decode_large_with_exponent_msb` (both widths +
encoder roundtrips), battery `lg9e27/lg8e27/lg99e114/cvDfDd` (the
last is the DF→DD conversion end-to-end: 0x3520000000895440). The
battery's own Python oracle replicated the over-strict check and is
fixed identically (it only mattered for value-cases, but oracles must
not disagree).

Lesson (process): never hand-transcribe hex — the first draft of the
new pin wrote 0x70095400 for gcc's 0x70095440 (9000000 = 0x895440,
not 0x895400); the test caught it immediately (8999936 ≠ 9000000).
Always paste machine output.

## S11.2: parse exponent overflow flips sign (FIXED)

`parse_decimal_literal` parsed the exponent magnitude into i64 with a
signed overflow sentinel (`-(1<<40)` when negated) and then negated
again: `1e-99999999999999999999DF` folded to +infinity instead of
zero. Fix: parse the unsigned magnitude, saturate positive, apply the
sign once (the `clamp(±2^30) as i32` tail was already exact).
Regression pins: `parse_exponent_overflow_keeps_sign` (exponent pins
±2^30 + end-to-end zero/+Inf), battery `peUnder/peOver` (GCC emits
0/0x78000000 with -Woverflow; lccc matches the values — diagnostic
parity stays open, see below).

## S11.3: hardening (no behavior change)

- `digits_val`: `debug_assert!(len <= 38)` guards the true u128 wrap
  boundary (callers pass ≤ prec+1 ≤ 35; fails fast in debug if the
  contract ever breaks, release-identical).
- `encode_fields`: the post-rounding "renormalize" re-round became a
  `debug_assert!(len <= prec)` (`round_to_prec` now guarantees it;
  re-rounding a rounded value is exactly the S11.0 sin).
- Exponent accumulation uses `saturating_add` (unreachable overflow
  on absurd inputs becomes saturation, never wrap).
- `(x & 1) == 1` parenthesized at both half-even sites (was correct
  by precedence, now unambiguous); misleading "Sticky below a half
  rounds up" test comment rewritten (the case rounds DOWN).
- `binary_to_decimal_digits` stays `pub` (external fuzzers use it);
  in-crate callers are bounded (≤ ~113-bit mantissas), so the
  theoretical mega-input DoS is not reachable from the compiler.

## S11.4: audit verdicts (full decimal.rs read, chunks 1–4 + decoders)

CORRECT, no change: `mul_small` (u64 accumulator proven: 9·5^13
overflows u32, fits u64), `binary_to_decimal_digits` arithmetic
(`(-(exp2 as i64)) as u32` safe for all i32), `bid_to_int` (loop
≤ 39 iterations by early-zero + checked-mul — no hang, no fix),
`fit_int` (unsigned-neg C wrap + `wrapping_neg` for i128::MIN;
declining far-out-of-range is conservative-correct: falls back to
the runtime op), `f128_to_int` (exact mantissa shifting; 2^60+1
trap covered), all special classifiers (bit 26/58 = G4 ✓),
`decode_bid128` (small-form-only is complete: 10^34-1 < 2^113),
all pre-existing GCC pins (each re-derived by hand under the new
rounding rule before running — zero changes, zero surprises).

## S11.5: validation evidence (S11)

- Build: zero warnings (rustc + `cargo clippy --all-targets` clean).
- Unit: `decimal` 23/23 (19 old + 4 new); full lib 3723/0/7 ignored.
- Battery: `PASS=104 FAIL=0` (was 94; +10 F33 pins, all bit-exact;
  both script copies updated identically, preambles differ by design).
- Differential fuzz (NEW `scripts/fuzz_decimal_vs_gcc.py`, committed):
  4 seeds × 930 cases = 3720 randomized literals/conversions/int and
  float sources, 0 divergences vs GCC. Bit-exact for D32/D64
  literals, ints, widening conversions; value-exact for D128,
  narrowing conversions, float sources.
- End-to-end probes: rr/rr2/lg/lg2 (rounding witnesses, subnormal,
  parse overflow, large+expMSB both widths, DF→DD conversion) all
  bit-match `gcc -S`.
- Harness: `decimal_implicit_conversions` PASS.
- Pre-fix failure proven, not assumed: rr.c probe emitted 797917761
  pre-fix (gcc 797917760); the new battery pins fail on the old
  binary by construction (451/4999/sub2495/cvDfDd/peUnder).

## S11.6: fuzzer notes (generator bugs fixed, all mine)

- Unsuffixed integer constants > 2^64-1: lccc ERRORS (constraint
  violation, standard-permitted), gcc warns-and-accepts via __int128.
  The fuzzer builds __int128-range values from ULL shift-or
  expressions (both fold exactly, silently). The frontend divergence
  is loud, not wrong-code: no fix here.
- `5DF` (no dot/exponent) is invalid C (DF needs a floating
  constant); the generator now spells `5.` (battery-proven form).
- `repr(float)` spellings, overflow→Inf literals, and NaN sources all
  run warning-tolerant on the gcc side (rc 0); only hard errors fail.

## Open (next session)

- Tier-1 audit continues: `src/common/const_arith.rs` decimal arms,
  `src/ir/lowering/const_eval.rs` (beyond the cast sites read here),
  sema guard, `global_init.rs`, `backend/common.rs` G4 arms.
- Micro-caveats (carried): decimal `-NaN` narrowing, lccc `-Woverflow`
  diagnostic parity (GCC warns on peUnder/peOver-style folds; lccc is
  silent), DF-via-DD/double-rounding route check, dead harness
  `expected_exit_code` param.
- CI stamp: S11 is UNGATED (local evidence above; full CI per
  validation-economy covers the rest). Next session: confirm CI green
  on the S11 tree, then continue the audit.
- Frontend (observed, not a defect): unsuffixed decimal integer
  constants above 2^64-1 are a hard error in lccc vs warning+__int128
  in GCC. Both conforming; matching GCC's leniency is a product
  decision, not a bug fix — do not "fix" without explicit direction.
