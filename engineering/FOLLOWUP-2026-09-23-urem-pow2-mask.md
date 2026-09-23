# Follow-up: the urem-by-power-of-two mask, and the review of PR #595

Session of 2026-09-23. Base `321e6f8` (PR #595 merged). Head after this work:
`d3c0c5f`.

## 1. What the review found, and what I verified

The review of PR #595 raised four findings. I re-derived each from the source
and from the compiler rather than accepting or rejecting them, because a review
cannot compile. The verdicts differ per item, and two of the review's own
factual claims turned out to be wrong.

### F1 -- the urem mask was built in i64: **AGREE, and it is the review's best find**

`src/passes/simplify.rs`'s `SRem | URem` arm built its mask with

```rust
let mask = (1i64 << shift) - 1;
```

while `const_power_of_two` -- widened in the same PR -- can return 127 for the
128-bit types.  I reproduced it end to end, with the divisor as a literal in a
`noinline` function so the fold actually fires:

```
$ lccc -O2 i128_divrem_pow2.c      # before the fix
FAILED case=urem 2^64 (89 bad)
REM  BAD k= 64 got=00000000000000000000000000000000 want=...ffffffffffffffff
REM  BAD k= 65 got=00000000000000000000000000000001 want=...1ffffffffffffffff
REM  BAD k=100 got=00000000000000000000000fffffffff want=...fffffffffffffffff
```

The review predicted the exact arithmetic and it is exactly right: `1i64 << 64`
is `1` (the shift amount is masked modulo 64) so the mask for 2^64 became 0;
`1i64 << 65` is `2` so the mask became 1; `1i64 << 100` is `2^36` so it masked
with only the low 36 bits.  `k = 127`'s `2^36 - 1`-style truncation matches
too.  Silent wrong code: the program assembled, ran, and printed plausible
numbers.

**The review understated one thing and I want it on the record.** It framed the
`2^63` dev-profile panic as a general hazard ("correct in release by
wrap-around luck, panics in any dev-profile build") which reads as
pre-existing.  It is not.  The predicate it depends on required a POSITIVE
i64:

```rust
let val = c.to_i64()?;
if val > 0 && (val & (val - 1)) == 0 { Some(val.trailing_zeros()) }
```

so `shift` could never exceed **62** and neither the shift wrap nor the
subtraction overflow was reachable.  **Both halves of F1 were opened by the
widening in PR #595: the predicate's recognition range was widened without
widening its one downstream consumer.**  That is the whole lesson of the bug,
and it is mine.

#### The fix, and why it is not the review's patch

The review proposed an inline `u128` mask with a two-arm `match`.  That is
correct, and I did something slightly different on purpose:

`IrConst::low_mask(ty, bits)` in `src/ir/constants.rs` -- a single
width-correct constructor, accumulating in `u128` and building the 128-bit
case as `IrConst::I128` directly (`from_i64` sign-extends from bit 63 and
cannot carry a wider mask).  The defect existed because a *range* was widened
without its *consumer*; the durable fix is to give every such consumer one
place that already knows how to be correct, and to unit-test that place once
rather than re-deriving the width arithmetic at each site.  The inline version
fixes today's bug; the constructor fixes the class.

`bits` is clamped to the type's width, so an over-wide mask saturates instead
of shifting out of range, and no width or `bits` value can panic.

### F2 -- the const-mul cache invariant: **AGREE on the finding, DISAGREE on the facts and on one placement**

The review's conclusion -- document and enforce the invariant -- is right, and
I implemented it.  Two of its supporting claims are wrong, and a wrong reason
attached to a right conclusion is how the next engineer gets misled:

* *"the mul sequence contains no cache-consulting loads (raw text emits only)"*
  -- false.  `emit_i128_mul_const_impl` opens with `self.operand_to_rax_rdx(lhs)`,
  which is exactly such a load.  It is *sound* because it runs before the first
  clobber, but that is a different invariant from "there are none", and it is
  the difference between a rule that can be followed and one that cannot.
  The comment now states the rule that actually holds: exactly ONE
  cache-consulting load is permitted, at the head, before any clobber.
* *"the identical note applies to `emit_i128_prep_shift_lhs_impl`"* -- that
  function is `self.operand_to_rax_rdx(lhs)` and nothing else.  It performs no
  clobbering emit, so it has no window of its own; the window belongs to the
  shift sequences it opens.  I applied the invalidation to the sequence heads
  (`emit_i128_mul_const_impl` and `emit_i128_prep_shift_lhs_impl`, which is
  the head of both shift sequences) via a named
  `begin_raw_i128_sequence()`, so the rule is stated once and cannot drift
  between call sites.

Evidence rather than assertion: `operand_to_rax_rdx` contains **no**
`reg_cache` reference anywhere in its body (checked mechanically over
lines 3745-3888), so the i128 pair path neither consults nor populates the
scalar cache, and the entry invalidation cannot pessimize it.
`scripts/census_ab.py` confirms zero delta (below).

### F3 -- test coverage: **AGREE**

Correct on all three counts: the div/mod suite was 64-bit only (zero
`__int128`), there was no direct unit test for `const_power_of_two`, and
`udiv i128` by 2^k for k in [64,127] was legal and untested.  All three are
now covered; see section 3.

### F4 -- the gate never proves the failure path; `parse_disp` hex: **AGREE**

Correct that the assemble-failure verdict was never exercised end to end -- a
tool that always reported clean would have passed every existing phase.  Phase 5
now builds a synthetic bad trace and demands exit 1 *and* the offending pass
named, then repairs the dump and demands a clean verdict again, so the check is
demonstrably on the operand and not on the presence of an extra dump.

On hex: I checked the actual failure mode before deciding, because the review
called it theoretical.  Rust's `"0x10".parse::<i64>()` fails, so a hex
displacement was classified as a **symbol** -- which renders correctly and
whose `0x10+8(%rax)` form GAS evaluates as an expression, so it was indeed not
a wrong-code path.  But it silently disabled the displacement arithmetic in
`compose_sib`, so it is a real (if invisible) loss of folding, and it cost six
lines to remove.  One correction to my own first attempt, caught by the test I
wrote for it: `i64::from_str_radix` is *signed* and rejects
`0xffffffffffffffff`, so the unsigned parse is reinterpreted as a bit pattern.

### What the review did not examine

I swept the class rather than the instance.  Every shift-to-mask construction
in the tree was enumerated:

| site | verdict |
|---|---|
| `simplify.rs` `SRem\|URem` mask | **the bug** (fixed) |
| `try_mul_power_of_two` (`simplify.rs:1791`) | sound -- passes a SHIFT AMOUNT (<=127), not a mask; `from_i64` carries it exactly, and `const_power_of_two` masks to the width so the shift is always narrower than the type |
| `iv_strength_reduce.rs:720` | sound -- guarded by `(0..64).contains(&shift)` |
| `backend/arm/codegen/alu.rs:414`, `generation.rs:1251,1298`, `memory.rs:330,562` | sound -- backend-local shift ranges, never fed by the widened predicate |

`try_mul_power_of_two` is the sibling the review did not look at, because it is
the other `const_power_of_two` consumer.  I verified it empirically rather than
by reading: `x * 2^k` for `u128` at k = 1, 8, 63, 64, 65, 100, 127 is correct
at -O1 and -O2.  The mask site is the only wrong-code site in the class.

I also re-derived the review's methodology row 2 (128-bit shifts by >= 64)
against the compiler: `emit_i128_lshr_const_impl` has explicit `==64` and `>64`
arms, and `udiv i128` by 2^k at k = 64, 65, 100, 126, 127 returns exact
quotients.  Agreed, and now covered by a test instead of by argument.

## 2. Verification

### Mutation proof, release profile (the miscompile)

```
tests/regression/i128_divrem_pow2.c
  -O1  OLD rc=1  FAILED case=urem 2^64 (89 bad)     NEW rc=0  ALL-OK
  -O2  OLD rc=1  FAILED case=urem 2^64 (89 bad)     NEW rc=0  ALL-OK
  -O3  OLD rc=1  FAILED case=urem 2^64 (89 bad)     NEW rc=0  ALL-OK
  -Os  OLD rc=1  FAILED case=urem 2^64 (89 bad)     NEW rc=0  ALL-OK
  gcc  -O2 rc=0  ALL-OK
```

### Mutation proof, DEV profile (the panic)

The unit tests are only worth anything if they run where the panic lives, so I
reintroduced the i64 construction and ran them under `cargo test --profile dev`
(overflow-checks **on**):

```
---- low_mask_never_panics ----
panicked at src/ir/constants.rs:657: attempt to subtract with overflow
---- low_mask_saturates_to_all_ones ----
panicked at src/ir/constants.rs:657: attempt to shift left with overflow
test result: FAILED. 1 passed; 4 failed
```

Both predicted panics, four of five tests failing.  With the fix in place the
same tests pass in the dev profile (5 + 6).  The file is restored automatically
by the proof script's `trap`.

### Performance: the fix is free

`scripts/census_ab.py --opt=-O2` over 749 files / 3135 functions:

```
file  fn  insns  rrmov  stkref  push  acc      (all NEW-OLD)
TOTAL       0      0       0     0    0
gate: HOT stkref 8418->8418, insns 74760->74760 => PASS
```

Zero in every column, so neither the mask fix nor the entry invalidation costs
anything.  Two methodology notes, because the first run of this measurement was
misleading:

* The new test file must be EXCLUDED from the A/B.  It is a reproducer for a
  miscompile the old compiler has, so the old side has no valid baseline: the
  old compiler folds `x % 2^64` to `x & 0` and then eliminates `urem_pow64`
  entirely, and the census correctly reports it as `[MISSING SIDE]` with a
  spurious +13 instruction "regression".  With the file excluded the delta is
  exactly zero across the other 3135 functions.
* The census columns are instruction counts and register traffic, not operand
  text; a change that alters no instruction count can still change correctness,
  which is exactly this fix.

### Assembly evidence that the fold still fires

```
urem_pow64:                          # NEW, -O2
    mov  $0xffffffffffffffff,%rcx
    and  %rcx,%rax                   # low half & (2^64 - 1)
    xor  %esi,%esi
    and  %rsi,%rdx                   # high half & 0
```

Exactly `x % 2^64 == lo`, one instruction per half.  Whole file: 42 `andq`, and
**zero** `__udivti3`/`__umodti3` calls -- the strength reduction is still
applied, which is what acceptance criterion 2 asked for.

## 3. Test inventory added

* `tests/regression/i128_divrem_pow2.c` (+`.flags`, `-O2`) -- 24 dividends x 12
  divisors `{2^1, 2^31, 2^32, 2^60, 2^62, 2^63, 2^64, 2^65, 2^96, 2^100, 2^126,
  2^127}` for both `/` and `%`, dividends straddling every boundary the divisors
  split, with non-power-of-two controls (`3`, `2^63+1`, `2^64+1`, `2^128-1`)
  whose reference is independent of the fold, plus signed `%` controls.
* 5 unit tests for `IrConst::low_mask`: exact `2^k - 1` for every `k` in
  `0..=width` across ten types; the i64 boundary pinned individually; all-ones
  saturation; never-panics at `bits` = 0/1/63/64/65/127/128/129/`u32::MAX`;
  and bit-identical agreement with the old `from_i64` construction for every
  `k` below 64 bits, so the change is provably a no-op there.
* 6 unit tests for `const_power_of_two`: the `to_i64` truncation trap and its
  mirrors, the full `2^0..2^127` range for both 128-bit types, the 64-bit
  boundary including all-ones at four widths, narrow signed types, off-width
  bit masking, and rejection of non-constants and zero.
* `tests/regression/i128_mul_pow2.c` (+`.flags`, `-O2`) -- the multiply
  consumer, which carries a shift AMOUNT and so should never have been wrong.
  A guard, not a reproducer, and labelled as one.  It is mutation-proven all
  the same, and the proof demonstrates something worth stating: perturbing the
  multiply's shift amount by one makes THIS file fail
  (`FAILED case=mul 2^2 (125 bad)`) while `i128_divrem_pow2.c` stays ALL-OK on
  the same compiler -- so the two files guard **orthogonal** consumers rather
  than duplicating each other.
* 3 unit tests for hex displacements and 1 gate phase.

The regression test's shape requirements are documented in the file because
each was measured, not assumed -- in particular that the divisor must be a
literal (a runtime divisor never reaches the fold, and a first draft of this
very test passed against the broken compiler for exactly that reason).

## 4. State: green everywhere, and how that was established

| gate | result |
|---|---|
| `ci_local.sh --fast` (frozen tree, nothing else running) | **60 passed, 0 failed**, 3 skipped |
| `cargo-test` (unit suite) | **3179 passed / 0 failed** |
| `cargo fmt --all -- --check` | PASS |
| `cargo clippy --all-targets --profile fastbuild --locked -j 1 -- -D warnings` | PASS |
| regression corpus | **781 passed / 0 failed** |
| `regression-corpus-ssa` (`CCC_VALIDATE_SSA=1`, skipped by `--fast`) | PASS -- 781 / 0 |
| `peephole-whitespace-invariance` (skipped by `--fast`) | PASS |
| dev-profile unit tests (overflow-checks ON) | 11 / 11 |

Two process notes, because they affect how much the numbers above are worth:

* **An earlier `--fast` run was discarded as untrustworthy.**  While it was in
  flight I rebuilt `target/fastbuild/lccc` from a mutated source to prove
  `i128_mul_pow2.c` can fail, and several gates in that run would have executed
  against the mutated binary.  No reading of that log can tell which.  The run
  reported here was made on a frozen, committed tree with nothing else running,
  after `scripts/build_lccc_fast.sh` from a verified-clean source.  A green
  result obtained from a binary of unknown provenance is not a result.
* The `--fast` skips are covered explicitly rather than assumed: the SSA corpus
  and the whitespace gate both exercise code this change touches (the pass and
  the peephole), so both were run by hand.

Still open, unchanged and unstarted: **dead frames -- 483 of 1412 framed
functions (34.2%) reserve a frame no instruction references, 15,200 B plus two
instructions each.**  It changes `%rsp` at every call site, so it needs the
alignment argument *and* the kernel boot gate, and the linux-cachymod tree is
absent from this workspace.  Also open: the `clz32`/`ctz32`/`hweight8` oracle
gaps and the `cmovgl`-after-`cmpl` builder.
