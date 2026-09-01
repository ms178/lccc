# Follow-up: IV closure widening — designed, implemented, measured, reverted;
# and the correctness gate that should have existed first

Session date: 2026-09-01 (continuation)
Base: `ms178/lccc` main @ `ece5fe42`
Snapshot: `S40`

---

## 1. What was attempted, and why it is the right design

Widening only the induction variable is inherently limited. The counter is
rarely used bare: `a[i-1]`, `a[i+1]`, `a[(i & 7) + 1]`, `a[i * 3]` are the
stencil, recurrence and struct-stride shapes, and each produces a **narrow
derived value** that then feeds addressing. `analyze_iv_uses` bailed on any
such use, so a loop like

```c
for (int i = 2; i <= 64; i++) { s[i] = s[i-1] + p; acc ^= s[(i&7)+1]; }
```

kept two `movslq` per iteration, one on the loop-carried dependency path.

So the design was to widen the counter's whole **derived closure**, admitting a
narrow op only when its value range is *proven* to fit in i32 by interval
arithmetic seeded from the counter's own range — because `sext(a op b)` equals
`sext(a) op sext(b)` exactly when the 32-bit `op` does not wrap. A loop whose
bounds are not compile-time constants yields no range and is declined.

It worked on the target shape: **both `movslq` eliminated** (2 → 0), and the
same-window A/B was genuinely good:

| kernel | closure on/off |
|---|---|
| `sieve` | **0.735** (−26.5%) |
| `loop_patterns` | **0.935** (−6.5%) |
| `tls_seg_access` | **0.943** (−5.7%) — the worst kernel in the corpus |
| `histogram` | 0.972 |
| `nbody` | 1.032 (loss) |

## 2. Why it was reverted: a miscompile

`sqlite_varint` printed `8e8824b0a241168` where GCC and the un-widened build
both print `deedcdd4edc1c0f1`.

Two soundness bugs were found and fixed along the way, and both are worth
recording because they are easy to repeat:

1. **The `Or` range bound was provably wrong.** `hi | c` is not an upper bound:
   for `x ∈ [1,2]`, `c = 2`, the values are `1|2 = 3` and `2|2 = 2`, so the
   maximum is 3 while `hi | c` is 2. An under-estimated interval admits a
   member whose real range does not fit — exactly the unsoundness the table
   exists to prevent. `Or` needs bitwise range analysis; it was removed.
2. **The trip bound was read from the wrong comparison.** The first version
   took any `Cmp` in the loop body that mentioned the counter. An inner
   `if (i < k)` is not the trip bound, and a range derived from it is a
   fabrication. Fixed to use only the comparison whose result the loop
   *header's* branch consumes.

Neither fixed `sqlite_varint`. Instrumenting the collector showed the real
defect:

```
widened phi Value(119)
  closure member v37 = And(v81, 31)        <- v81 is not v119
  closure member v48 = Mul(v76, 104729)    <- v76 is not v119
```

**The collector admits ops whose operand is not in the closure at all**, so the
range proof is attached to values it does not describe. That invalidates the
entire safety argument, not one arm of it.

I could not localize it with confidence in the time available, and a
miscompile must never ship. Reverted. The design stands; the membership test
needs rebuilding with the seed relation enforced explicitly (and asserted)
rather than inferred from a map lookup that evidently succeeds for values that
were never inserted.

## 3. The infrastructure defect this exposed — and fixed

**The miscompile passed all 563 regression tests.**

`tests/benchmark/programs/*.c` are the largest, most realistic programs in the
tree — SQLite varint, zlib-ng adler32, Expat scanning, glibc memcmp, Linux
find_bit — and they were only ever compiled by `run_benchmarks.py`, which takes
minutes because it times everything. Nobody ran them during development, and
the regression corpus contains nothing with that loop shape.

`scripts/check_benchmark_outputs.sh` closes the gap: compile, run, diff against
the GCC oracle, **no timing**, so it finishes in ~4 minutes and can run on every
change. It compares at `-O0 -O1 -O2 -O3`, because an
optimisation-level-specific miscompile is still a miscompile and `-O0` vs `-O3`
disagreeing is the cheapest possible signal.

It found a bug in itself on the first run, which is worth keeping in the file:
comparing lccc `-O0` against gcc `-O2` failed `tce_sum`, because that kernel
recurses ten million deep and only survives via tail-call elimination. Both
compilers must be asked the *same* question, so the reference is now rebuilt at
the matching level. **152/152 pass.**

Had this gate existed, the closure miscompile would have been caught in four
minutes instead of surviving a full green test run.

---

## 4. Validation of the shipped state

| Gate | Result |
|---|---|
| `cargo test --lib` | **1480 pass / 0 fail / 6 ignored** |
| `./scripts/run_regression_suite.sh` | **PASS=563 FAIL=0 SKIP=15**, AB-diff 0 |
| `scripts/check_benchmark_outputs.sh` | **PASS=152 FAIL=0** (new) |
| `ir_verify_sweep.py --levels O0..Oz` | **0 violations** / 3378 configs |

---

## 5. To do next

1. **Rebuild the closure membership test.** Track the seed relation
   explicitly — record, for each admitted member, *which* closure value it
   derives from, and `debug_assert!` that the operand really is that value.
   The measured prize is large and already quantified above (`sieve` −26.5%,
   `tls_seg_access` −5.7%, `loop_patterns` −6.5%), and the range machinery,
   the two soundness fixes and the test corpus are all still in this document.
2. **Run the new gate in CI.** It is fast enough to be unconditional.
3. Unchanged: `PF-CLS-1` classifier chain, MachInst clobber modelling +
   `Call`, `PF-TLS-1`, def-dominates-use in the verifier.

---

## 6. Kritik und Selbstkritik

- I shipped nothing this cycle except a test gate, and the honest reason is
  that I wrote a value-changing transform before writing the correctness gate
  that could tell me whether it was safe. The gate took twenty minutes and
  would have redirected the whole session.
- Two of my three "fixes" during debugging were guesses that changed no
  behaviour. The instrumentation that actually localized the defect — printing
  each admitted member with its operand — should have been the first step, not
  the fourth.
- The measured wins are real and were obtained honestly (same-window A/B, all
  outputs compared). They are not a reason to ship an unsound pass, and I am
  explicitly not arguing that they are.
