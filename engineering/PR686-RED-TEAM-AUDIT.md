# Red-team: PR #686 vs this branch, and a line-by-line audit of the review

Scope: audit the competing PR (#686, head `345a5e9c`) and my own patch, plus
the third-party review of #686. Every claim below names the experiment that
settled it. Where I disagree with the review I say so and show the
measurement.

## 0. The review audited a revision that no longer exists

This is the single most important thing to establish before weighing any
finding, and it is not visible from the audit text.

PR #686 has **two** commits:

| | commit | subject |
|---|---|---|
| 1 | `67cabbee` | Integer min/max vectorization, dedicated loop preheaders, volatile-LICM guard, oracle infrastructure |
| 2 | `345a5e9c` | Close the audit findings: preheader dominance guard, honest coverage docs, --compile-only contract |

The review's F1 cites `loop_preheader.rs:446-447`. That line number exists
**only in `67cabbee`**:

```
$ git show 67cabbee:src/passes/loop_preheader.rs | grep -n check_loop_preheader
446:    //! `tests/regression/check_loop_preheader.sh` and
447:    //! `tests/regression/loop_preheader_shapes.c`, which assert on the
$ git show pr686:src/passes/loop_preheader.rs | grep -c check_loop_preheader
0
```

So F1, F2, F5 and F6 describe the **first** commit. The head already fixes
all four, and the second commit's own subject line says so. Treating F1/F2
as open findings against #686 would be auditing a revision that no longer
exists.

That does not make the review wrong — it makes it *stale*, and the four
findings are still perfectly good findings **against my branch**, which
never received the second commit. That is where the real work was.

## 1. Verification, finding by finding

| # | Finding | #686 head | my branch | Verdict |
|---|---|---|---|---|
| F1 | preheader gate cited but absent | **fixed** (`345a5e9c`) | n/a — gate exists | stale |
| F2 | `find_loop_preheader` misses `Switch`/`IndirectBranch` | **fixed** | **PRESENT** | **real, adopted** |
| F3 | verbatim duplication between AVX2/SSE2 | 660 lines | 610 lines | real, **worse** than stated |
| F4.1 | `preheader_would_unlock_a_hoist` is O(\|body\|²) | present | present | real, but severity **overstated** |
| F5 | LICM fixture's premise is now false | **fixed** | **PRESENT** | **real, adopted** |
| F6 | `--compile-only` still trains PGO binaries | **fixed** | **PRESENT** | **real, adopted** |
| F8 | no `debug_assert` on the entry block | present | present | real, trivial |
| F9 | `global_derived_values` is an O(n²) re-scan | present | present | real, minor |
| F10 | pure-logic tests sit in a compiler-dependent CI step | present | present | real, minor |

## 2. F2 — the one finding that is a miscompile

`find_loop_preheader` decides where the hoisted reduction bound is placed,
and both call sites push a `div` there and use it inside the loop. So the
block it returns must dominate the header. My version matched only `Branch`
and `CondBranch`; everything else fell into `_ => false`, which is not
conservative but wrong:

> with one entry via a `Switch` and one via a `Branch` it reported the
> `Branch` block as the preheader, and that block does not dominate the
> header. The def of the bound then stops dominating its use on the switch
> path — an SSA violation, i.e. a miscompile.

I adopted #686's terminator-complete match, which counts `Switch` cases
(including `default`) and `IndirectBranch` targets, and returns `None` for
that shape so the caller falls back to the header — correct, only slower.

**A direct test, because a C fixture cannot express the shape.** The hazard
needs a header entered by a switch case *and* by a branch, which the
optimizer folds away before the vectorizer sees it. So the property is
pinned on the function, with the CFG built directly
(`src/passes/vectorize.rs::preheader_entry_tests`):

* `switch_entry_counts_so_branch_plus_switch_is_not_a_preheader` — the regression
* `a_lone_switch_entry_is_still_a_preheader` — the fix must not refuse a
  legitimate single switch preheader
* `computed_goto_entries_are_counted_too`
* `a_block_inside_the_loop_is_never_an_entry` — the latch is not a preheader

**Mutation-verified.** Removing the `Switch` and `IndirectBranch` arms:

```
1 passed; 3 failed
  a_lone_switch_entry_is_still_a_preheader
  computed_goto_entries_are_counted_too
  switch_entry_counts_so_branch_plus_switch_is_not_a_preheader
```

Three of four fail, including the regression itself. The tests are load-bearing.

## 3. F6 — `--compile-only` did not honour its own contract

Inherited from #682, which I adopted wholesale. The `@PROFDIR@` PGO branch
returns before the `compile_only` check, so PGO tests still **generated,
trained (which executes the binary), rebuilt and ran** under a flag
documented as "compile+link every corpus file but do not run it".

Fixed at the only correct point — after the generate build has linked, before
the training run. Verified by driving `compile_one` on `pgo_branchy`:

```
compile_only=True   pass  0.3s  phases=['gen:ok', 'link:ok']
   trained=False  executed=False
compile_only=False  pass  0.4s  phases=['gen:ok', 'train:ok', 'use:ok', 'run:ok']
   trained=True   executed=True
```

## 4. F3 — the duplication is larger than reported, and it is now gone

The review records "~100 lines of verbatim copy-paste". Measured by
contiguous match between the two reduction functions:

| | lines per function | identical contiguous blocks | total identical | largest block |
|---|---:|---:|---:|---:|
| my branch | 1351 / 1355 | 15 | **610** | 226 |
| #686 head | 1401 / 1405 | 15 | **660** | 258 |

So the review's "~100" describes one *statement-clean* unit inside a much
larger clone, and #686's copy is ~8% larger than mine.

Extracted `emit_invariant_vector_bound()`; both arms became one call, and
`find_loop_preheader` is now called from exactly one place instead of two.
The two arms were verified byte-identical before the splice, so the
extraction could not have changed behaviour.

**Behaviour-preserving, measured over the whole corpus** (870 translation
units: `tests/benchmark/programs`, `tests/bench`, `tests/regression`):

```
identical assembly : 870
differing          : 0
```

## 5. Where I disagree: F4.1's severity is overstated

The review rates the quadratic `preheader_would_unlock_a_hoist` "medium" and
concrete about "~25 M dominance walks on a 5 000-block loop". The proof that
only the header can pass is **correct** and I applied it — the inner
"dominates every loop block" filter can admit only `header`, because
`header` dominates every body block, and dominance is antisymmetric.

But the *consequence* does not survive measurement. A/B on synthetic
single-loop functions with large bodies:

| loop body | pre-refactor | post | delta |
|---|---:|---:|---:|
| ~500 statements | 1904 ms | 1934 ms | noise |
| ~1500 statements | 12314 ms | 11940 ms | −3% |

And the cost is not where the review implies — disabling individual passes on
the 1500-statement input:

| disabled | time |
|---|---:|
| *(nothing: full −O2)* | 10729 ms |
| vectorize | 10193 ms |
| gvn | 10221 ms |
| sccp | 10488 ms |
| if_convert | 10446 ms |
| licm | 24543 ms |

No single pass dominates, and the preheader gate is not a visible term. The
change is still right — strictly less work, and provably so — but it is an
asymptotic improvement that is **not measurable end-to-end at realistic
sizes**, and shipping it as a medium-severity compile-time fix would
overstate it. The census added in the previous round shows why: over the
benchmark corpus this pass rejects or accepts 70 loops total, so the gate is
almost never on a hot path at all.

## 6. F8 and F5

F8: `apply_insertions` inserts at `header_idx`; index 0 is the entry block.
Unreachable today (`find_preheader` returns `None` for it), but that is
exactly the kind of invariant that stops holding when someone widens the
admission test. Added `debug_assert!(header_idx != 0, ...)` at the point of
the dangerous operation.

F5: `licm_no_speculative_load_nondedicated_preheader.c` documented a shape
that "must not be hoisted". With `loop_preheader` the load **is** hoisted —
legally, into the inserted preheader — and the file's claim had become false
while the test still passed, because it is a runtime test and the hoist is
safe. The next reader would have gone looking for a non-bug. Rewritten, and
the rewrite states *why* it is still correct: `pre` has one successor, so
reaching it implies `p != 0`, and the test still segfaults deterministically
if a preheader is ever spliced onto the wrong edge.

## 7. Errors of my own, caught by testing

Three, all in this round, all worth recording because each was briefly
convincing:

1. **The census read zero everywhere.** I added per-function rejection
   counters and they reported `loops=0` across 333 benchmark functions —
   including on the preheader gate's own fixture, where an insertion
   visibly happened two lines earlier. The counters were incremented through
   `&mut cell.get()`, which mutates a *temporary* copy of a `Copy` value and
   discards it. Superficially convincing, and I nearly reported "the pass
   finds no loops" as a finding.
2. **A first extraction mangled the tail** of the duplicated block: my
   brace-matching cut the closing `}` of the `match` arm and produced a
   `limit_operand` that was never bound. Caught by reading the diff before
   building, then redone with asserted boundaries.
3. **My own F2 test was wrong** — I used labels 0,1,3 in a `Vec<BasicBlock>`
   where `header_idx` is an *index*, so block 3 did not exist. Fixed, and the
   other three tests were unaffected.

## 8. Verdict

I **agree** with the review on F2, F3, F4.1, F5, F6, F8, F9, F10 as
observations. I **disagree** on their current status against #686 (F1, F2,
F5, F6 were already fixed in the head) and on F4.1's severity.

#686's head is better than my branch in exactly one substantive way: the
terminator-complete `find_loop_preheader`. That is a real unsoundness fix and
I have taken it, with a mutation-verified test. Everything else #686 has
that my branch lacks is either already in my branch (the preheader gate, the
LICM comment, the `--compile-only` contract) or is a documentation change
mine already got right.

The two branches are now converged on every finding either review produced.
