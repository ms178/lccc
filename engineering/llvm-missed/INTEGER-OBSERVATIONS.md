# integer observations: 25 implementation families, 45 native witnesses

These are implemented transformations, not a list of proposals. Direction,
operand-order, width and predicate variants are grouped conservatively below.
The previous conditional-negation fold is not counted as a new implementation.
All scalar proofs use 32/64-bit integer semantics; C fixtures use unsigned
arithmetic and avoid zero inputs to undefined CLZ/CTZ builtins.

| # | Implementation | Native witnesses | Proof / guard |
|---:|---|---|---|
| 1 | Odd-multiplier equality preimages | `odd_mul`, `mul_eq` | Invert an odd coefficient modulo 2^N, including nonzero targets. |
| 2 | OR with modular negation zero | `or_neg` | x \| -x is zero iff x is zero. |
| 3 | Isolated lowest bit zero | `isolate_low` | x & -x is zero iff x is zero, including the sign bit. |
| 4 | Triangular and odd-factor shift zero | `or_shl`, `or_lshr`, `xor_shl`, `xor_lshr`, `add_shl`, `sub_shl`, `rsub_shl` | Positive constant shifts: triangular XOR/OR or an odd modular factor; never shift zero. |
| 5 | Byte-permuted zero observations | `swap_zero` | Byte reversal preserves zero. |
| 6 | Rotated zero observations | `rotate_zero` | Rotations are fixed-width permutations. |
| 7 | Population-count zero observations | `pop_zero` | No set bits iff input is zero. |
| 8 | Affine-equivalent select observations | `add_xor`, `rsub_xor`, `sub_xor` | Normalize both arm preimages, not their different integer values. |
| 9 | Full population count | `pop_full` | All N bits set iff popcount is N. |
| 10 | Leading-zero boundary | `clz_zero` | Count zero iff the high bit is set. |
| 11 | Trailing-zero boundary | `ctz_zero` | Count zero iff the low bit is set. |
| 12 | Shifted zero intervals | `lshr_zero`, `ashr_zero` | Replace a right-shift zero observation with an unsigned interval. |
| 13 | Shifted upper intervals | `lshr_full`, `ashr_full` | Replace the extreme right-shift bucket by an unsigned threshold. |
| 14 | Left-shift zero masks | `shl_zero` | Only the low N-k bits affect zero after a left shift. |
| 15 | OR-mask equality | `or_mask` | (x\|m)==m iff (x&~m)==0. |
| 16 | Complemented mask observations | `not_mask` | Move bit complement into the demanded constant. |
| 17 | Unsigned quotient zero | `div_zero` | x/d==0 iff x<d, d nonzero, before magic-division expansion. |
| 18 | Population-count input bound | `pop_le` | Each set bit contributes at least one, so popcount(x)<=x unsigned. |
| 19 | Shared binary equality cancellation | `cancel_add`, `cancel_sub`, `cancel_rsub`, `cancel_xor` | Cancel shared Add/Sub/XOR operands for Eq/Ne only, no no-wrap assumptions. |
| 20 | Shared unary equality cancellation | `cancel_neg`, `cancel_not`, `cancel_swap` | Cancel matching bijections, not ordered comparisons. |
| 21 | Shared rotate equality cancellation | `cancel_rotate` | Cancel identical rotations, including common variable counts. |
| 22 | Byte-swap Boolean conjugation | `swap_and`, `swap_or`, `swap_xor`, `swap_not` | A byte permutation commutes with bitwise Boolean algebra. |
| 23 | Byte-swap whole-byte shift conjugation | `swap_shl`, `swap_lshr` | Reverse shift direction only for constant whole-byte shifts. |
| 24 | Byte-swap select conjugation | `swap_select` | Cancel arm/output permutations without deleting condition evaluation. |
| 25 | Remainder-invariant low-bit demand | `urem_low` | If 2^k divides d, low k bits of x%d equal those of x, even for non-power-of-two d. |

## Integration and cost discipline

`src/passes/bit_idioms_observe.rs` normalizes the *observation* rather than
speculatively computing both selected values. Arm predicates must normalize to
the same root, width, mask, target and relation. Signedness-only casts preserve
bits and can be crossed; narrowing, widening and FP casts cannot. Eq/Ne
cancellation does not imply signed/unsigned ordering cancellation.

Recursion is bounded to depth 12 and 64 visited nodes per observation, including
shared nested selects. The late sweep reuses existing map allocations after all
coordinate-based producer edits finish. Producers are never repurposed. Fresh
masked-test values are inserted immediately before their consumer, with unique
IDs and copied source spans; already-canonical masked tests are reused, making
the pass idempotent rather than minting IDs every iteration. The early sweep
runs only in functions containing unsigned div/rem, before constant-division
strength reduction destroys those operations. Both entries honor the exact
`bit_idioms` disable token.

Forty-four of 45 native witnesses have fewer instructions than the saved
pre-batch compiler at both O2 and O3. The remaining high-unsigned right-shift
bucket removes the shift but still needs a large-immediate materialization:
same instruction count, not a claimed size or runtime win. No timed speedup or
universal superiority is inferred from instruction counts.

The native driver checks each witness separately, not just a minimum aggregate
win count. It runs O0/O1/O2/O3/Os/O3+g, requires exact GCC 16.2, optionally compares
exact Clang 23.1.3, enables `CCC_VERIFY_IR=abort`, checks disabled-policy bodies,
and retains volatile loads, side-effecting condition calls and shared producer
uses. Negative C cases distinguish even multiplication and zero-distance XOR
from valid zero-reflecting maps. Rust tests interpret actual pre/post IR across
all 45 cases, both widths/signed views/polarities, small-domain slices and high
bits, then require a second run to allocate nothing and report zero changes.
Additional negative tests reject width changes, non-byte shifts, arithmetic
byte-swap shifts and invalid remainder moduli. Odd inverses are checked for all
odd low-16-bit coefficients plus 64-bit boundaries.

## Same-ISA compiler comparison

`integer-observations-code-size.json` records all 45 bodies using the same
`-O3 -march=x86-64-v3 -S` flags, exact GCC 16.2 and Clang 23.1.3, and includes
the C-source hash. Counts include RET and exclude labels/directives/comments.

| Comparator | LCCC fewer instructions | Equal | LCCC more |
|---|---:|---:|---:|
| Saved pre-batch PR compiler | 44 | 1 | 0 |
| GCC 16.2 | 13 | 30 | 2 |
| Clang 23.1.3 | 17 | 25 | 3 |

This provides concrete wins beyond the upstream compiler on these witnesses,
not a universal performance ranking. Remaining deficits include return-width
cleanup for the low-bit boolean, a high unsigned comparison immediate and
selected-value register moves. The CLZ-zero boolean is materialized directly by
logical sign-bit extraction, matching both oracles' three-instruction body.

Reproduce with `scripts/compare_integer_observations.py --baseline <saved-compiler>
--gcc <gcc-16.2> --clang <clang-23.1.3> --output <report.json>`; the baseline
is the saved first-batch compiler, not an inferred disabled-policy stand-in.
All new correctness and mechanism checks also run with IR verification enabled.

## Upstream scope and an important rejected proposal

* #233753 motivates the observation-through-select generalization; it is not
  counted as 25 independent upstream issues.
* #230022: the direct non-power-of-two remainder low-bit example is covered.
  The report's other shifted-demand/masked-multiplication examples remain work.
* #220800: the direct unsigned population-count bound is covered; general
  path-sensitive transitive bound elimination is not claimed.
* #205014: Eq/Ne common-addend cancellation is covered and extended to other
  bijections. Ordered comparisons without no-wrap evidence are rejected.
* #33226 remains deferred for **variable-count flag reuse**. The constant-count
  interval reductions here neither assume count nonzero dynamically nor read
  flags from BMI2 shifts.
* #62514's proposed unconditional bswap return is wrong for arbitrary `c`:
  `c=0,x=1` must return zero. We do not transplant that proposal. Eliminating an
  equal *observation* is different from deleting a value-select condition.

`coverage.json` is now the cumulative review ledger; `zero-choice.json` remains
an immutable record of the first batch. Use `--ledger .../coverage.json` when
regenerating the index. The index still distinguishes limited coverage from
unreviewed/deferred reports. Linker/unwind equivalence and the remaining bulk
inventory are not silently declared solved by this scalar batch.
