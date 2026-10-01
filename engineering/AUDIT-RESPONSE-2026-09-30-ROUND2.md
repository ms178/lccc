# Response to the second Review AI audit

Date: 2026-09-30. Reviewer: Review AI, auditing this branch's own work.
Verdict taken as a proposal, not as truth: every claim below was reproduced
before I acted on it, and two claims turned out to be *stronger* than stated.

Scoring the reviewer gave the shipped state: production 8.5/10, tests 5/10,
docs 6/10, integration 3/10, process 8/10, overall 6.5/10.

---

## F1 — BLOCKER: the `--require-oracles` gate was inert. **CONFIRMED, and my claim about it was false.**

The reviewer's exact invocation:

```
$ python3 tests/linker/run_linker_tests.py --lccc /bin/true --strict -v \
      --require-oracles definitely-missing
>>> EXIT CODE = 0
--- stdout: (empty — zero tests executed) ---
--- stderr: FAIL: required differential oracles not registered: definitely-missing
```

`main()` **returns** its exit code; it never calls `sys.exit`. The bottom of
the file was a bare `main()`. So `return 2` was discarded and the process
exited 0.

**I had verified this the wrong way, and said so in a commit message.** That
commit claimed "Verified end-to-end … exit 2 naming lld". What I ran was
`m.main()` in-process, which does return 2. Nobody had run the process and
looked at its status. A gate that claims more than it does, described in the
document whose purpose is to condemn exactly that, committed by me.

Worse than inert. Before the flag, a missing `lld` narrowed the quorum and the
suite still ran 298 non-script-path cases. With the flag, it bails out before
executing anything — and because the early return precedes the `--json`
write, the `if: failure()` artifact upload never fired either. Every channel
reported PASS on a run that executed zero tests.

**Fixed**: `sys.exit(main())`, plus a **process-level** regression test. The
existing `missing_required_oracles` test is the wrong layer and passed
happily for the entire time the CLI contract was broken. Four new cases run
the real script under `subprocess` and assert `returncode` and stderr,
including one that also fails if a run reporting gate failure produced any
stdout, and two controls that must still proceed so the check cannot become a
blanket blocker.

**Swept** the repo with an AST pass for `if __name__ == "__main__":` followed
by a bare `main()`: 26 sites, of which exactly one returned a real exit code
(this one). The other candidates either call `sys.exit` internally or return
tuples/benchmark values, where a bare `main()` is harmless. So the fix is
complete, and the sweep is recorded rather than assumed.

---

## F2 — the volatile lint's self-test could not fail. **CONFIRMED.**

Deleting `_is_debug_only_use` and reverting `_has_real_use` to the pre-PR
body left the self-test **19/19 green**. The case I shipped as the feature's
coverage was:

```rust
eprintln!("volatile={volatile}");
```

`volatile` there is inside a string literal, and `_strip_comments` blanks
literals. The use window never saw it. The case reported a violation for an
unrelated reason and would have with the feature deleted.

Replaced with forms where `volatile` is a macro **argument**:
`dbg!(ptr, volatile)`, `assert!(!volatile, "x")`, `dbg!((ptr, 1), volatile)`
— violation with the feature, clean without.

**"Does this case discriminate" is a property of the detector, not of the
source text**, and inspection is exactly what failed me here. So
`--self-test` now runs a discrimination proof: it mutates the feature in
process (`DELETE`, and `WEAK` — the pre-PR "a logging macro appears earlier
in this line" test) and **requires the suite to go red**. `DELETE` kills 3
cases, `WEAK` kills 1. Before this change `DELETE` killed 0.

---

## F3 — known gaps were reported as failures. **CONFIRMED.**

The two cases documenting holes we do not close were pinned `expect=clean`,
so the day someone fixed one the suite would go red and progress would look
like a regression. They are now flagged `known_gap`: still-a-gap is
informational, and a case that starts reporting a violation prints
`KNOWN GAP CLOSED` and counts as progress. Verified in both directions.

---

## F4 — the exemption had a false positive. **CONFIRMED.**

"A logging macro appears earlier in this line segment" exempts any `volatile`
sharing a statement with a debug call, including a real guard:

```rust
if dbg!(ptr) || volatile { return true; }   // volatile is a SIBLING TERM
```

Dropping that `volatile` is a dropped observable-access guard — the exact
bug the lint exists to catch. The exemption is now a depth scan: the token
must be lexically inside the macro's delimiters, so a nested argument list
counts and a sibling term does not.

This **narrows** the exemption set, so the regression risk is new violations
on real code. Checked: the ratchet is clean across all 496 Rust files at the
new strictness.

---

## F5 — the first-compare order test could not fail. **CONFIRMED, and the reviewer's proposed cause was slightly off.**

`flags_compare_keeps_the_first_of_the_pair` used `movq %rsi, %rdx` as the
intervening instruction and asserted the survivor precedes the first `cmov`.
But that `mov` is a **dead relay**: move-relay elimination deletes it, and
the shape collapses to `cmp; cmp; cmov`, in which *every* compare precedes
the only `cmov`. True whether the pass folded forwards or backwards.

Proven, not assumed — hand-mutating the fold to `mark_nop(&mut infos[j])`:

```
flags_compare_folds_across_a_cmov_that_writes_a_different_register  FAILED
  "the first compare must survive"      <- cmov is BETWEEN the compares: discriminates
flags_compare_keeps_the_first_of_the_pair                          ok   <- vacuous
```

The reviewer's mechanism was "mark_nop targets a different index". The real
cause is the DCE'd `mov`, and it matters: fixing only the index would have
left the assertion vacuous anyway. Fixed by anchoring to
`prefetcht0 (%rsi)`, which has a memory side effect, is flag-preserving, and
survives DCE. The same mutation now fails this test too. The count assertion
is kept for its own unique coverage — that the fold fires across a
non-flag-writing instruction at all.

This is the **second** test in this series that was green for a reason other
than the property it claimed. Both had the same cause: asserting a property
whose truth under the failure it was meant to detect I had not checked.

---

## L1 — the "compiled out" claim was overstated. **ACCEPTED.**

I had written that a `debug_assert!` "enforces nothing in the binary *or* in
`cargo test`". CI is not blind to it: `ci.yml` runs the suite a second time
with `--config 'profile.fastbuild.debug-assertions=true'`. Accurate claim: it
is inert in the shipping binary and in the default local fastbuild run —
which is exactly why the invariant also lives in the gate.

## L2 — `plain_gp_operand` accepted a prefix. **CONFIRMED.**

`register_family_fast` dispatches on the first four bytes and never inspects
the length, so `%raxfoo` read as `%rax` and `%rdiblah` as `%rdi`. Right trade
for a hot prefix lookup, wrong for a predicate whose job is "is this token
*exactly* a bare register?" — which is the allowlist the memory-operand fix
introduced. Now requires an exact `REG_NAMES` spelling; two unit tests.

## L3 — the gate asserted a different equation than the code. **CONFIRMED, and the reviewer understated it.**

`Census::accounted()` sums **six** buckets. The gate's awk summed **five**.
The sixth, `planned`, was missing twice over: it *is* incremented in the real
pass, and `report_census` **never emitted it**, so the gate could not have
seen it even had it looked. The two equations coincided only because the
reported round is the fixpoint, where nothing is planned.

`planned` is now reported and the awk sums all six. Verified mechanically:
parsed from both files, the bucket sets are now **equal** (modulo two
pre-existing report-name/field-name renames), and all seven keys appear in
real emitted census lines. Accounting itself untouched.

## L4 — the "reproducible miscompile" claim. **ACCEPTED, withdrawn.**

I had called the old memory-operand weakness "a *reproducible* miscompile,
one line of C away". I never produced a C program that compiles to wrong
output. What exists is a unit test over a synthetic fixture showing the pass
deleting the wrong compare — a demonstrated precondition violation, which is
what the fix and its test rest on. "Reproducible miscompile" implies an
end-to-end demonstration I do not have, and that gap matters to anyone
deciding how urgently to backport. Reworded to state exactly what was shown.

---

## What the reviewer got wrong

**F1's mechanism, partly.** The reviewer reported the missing-`lld` scenario
returning 0, which is right, and I initially could not reproduce it: a
top-level `--require-oracles definitely-missing` probe exits **2**, because
Python's uncaught `SystemExit` from an argparse/`sys.exit` path behaves
differently than a discarded `return`. The distinction is the whole bug, and
it took reading `main()` for the `return 2` — not `sys.exit(2)` — to settle
it. A reviewer reporting a reproduction that does not reproduce is worth
checking before rewriting code.

**F5's cause**, as above: not the `mark_nop` index but the DCE'd `mov`.

## Things the reviewer did not flag

An AST sweep of the whole repo for discarded `main()` returns found 26 bare
`main()` sites and exactly one that discards a real exit code. Worth
recording: the class of bug is common, and here it occurred exactly once.

---

## Validation

`scripts/ci_local.sh --fast`: **119 passed, 0 failed, 5 skipped, ALL GATES
GREEN**, stamp on tree `826b23f0`.

Getting there required repairing the sandbox, which had been reset: no Rust
toolchain, no swap (1 GiB RAM, rustc killed by the OOM killer), and no
`gcc-multilib`, `g++-multilib`, or `lld`. Those accounted for **all nine**
failures in the first gate run — none were caused by this branch. Worth
recording because the first run read `110 passed, 9 failed` and three of those
nine were gates this work had previously passed.

The linker suite is now **301 pass, 0 fail, 0 warn, 0 skip** with both
`bfd` and `lld` oracles, exit 0 — the first time its exit code has ever been
meaningful.

## Left undone, deliberately

`liveness.rs:995` `ret_live` precision. Core fail-closed analysis; needs its
own cycle and a corpus-wide Callgrind. See
`FOLLOWUP-2026-09-30-affine-exit-compare.md`.

The LCCC `-m32` C++ include path does not add GCC's
`/usr/include/x86_64-linux-gnu/c++/14/32`. This is a real gap, found here
because the linker suite's `i386_dso_emit_semantics` test exercises it — but
it is a pre-existing compiler limitation, not a regression, and fixing the
compiler's multilib include discovery is its own change. Recorded here
rather than folded into a test-fix commit.
