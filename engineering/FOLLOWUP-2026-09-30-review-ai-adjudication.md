# Review-AI audit adjudication (7.8/10) — what I agreed with, what I rejected, and why

Date: 2026-09-30
Tree: `8032ecbe` + this session's fixes, base `f9bef39b` (unchanged upstream)
Verdict on the audit: **5 of 6 findings confirmed, 1 partially rejected, 1 fix
improved on, and 3 of its "agreements" independently re-derived.**

The audit's headline claim — *"nothing I found is a miscompile or soundness
defect"* — survives every check below. Its most valuable contribution is F1, and
it is valuable precisely because it contradicted me and I was wrong.

## F1 (High) — CONFIRMED. The comment was false, and I had defended it.

I did not take this on faith and I did not take my own prior position on faith
either. Three independent tests, all from a rebuilt compiler:

1. **Static.** `lv` is written twice in `fold_copy_into_lea_base` and read never
   (the only other mention is a comment). `FileLiveness` has **no `Drop` impl**
   (`grep -rn "impl Drop for FileLiveness" src/` is empty), so `lv = lv2;` has no
   side effects of any kind. A dead store in the *compiler's own source* cannot
   alter the compiler's *behaviour*: lccc is a deterministic function of its
   input, so the comment's stated mechanism ("it changed this loop's control
   flow") is not a thing that can exist.
2. **The cited numbers do not reproduce.** The comment claims `sqlite_varint`
   256 -> 270 and `gzip_crc32` 65 -> 69. Measured whole-TU counts on this tree
   are **410** and **86**, at both `-O2` and `-O2 -march=x86-64-v3` — neither
   value corresponds to either cited number, so the measurement was taken on
   something other than this tree.
3. **Direct experiment.** Deleted exactly what the audit prescribed (the decl,
   the `#[allow]`, the store, the comment), rebuilt, and A/B'd against a binary
   built from the unmodified file: **150/150 asm outputs byte-identical** across
   `tests/benchmark/programs` (51) + `kernel_corpus` (24) x {-O2, -O2
   -march=x86-64-v3}, and **0 warnings** (so the `allow` was covering nothing but
   the store itself).

The audit offered a fork: *"Either the comment is false, or the compiler is
order-dependent — which would be a far bigger bug."* **Resolved: the first.** No
order-dependence exists. The generalisable rule, now written at the site: a
codegen delta attributed to a provably dead store is a **measurement error**, and
the correct response is to re-measure, not to pin the dead code.

This is the substantive self-criticism of the session. I had a comment asserting
a measurement I had not reproduced, on code whose behaviour I never observed
differing, and I used it to justify keeping dead code. The audit caught it by
reading the code and reasoning about the language; that is exactly the check I
should have run on myself before committing.

### What I did not do

I did **not** silently delete it. The comment is replaced with a record of the
removal, why the claim was unsound, and the measurement — because the measurement
was presumably real for whoever took it (most likely confounded by an unrelated
edit), and an unexplained deletion invites someone to restore it.

## F3 (Medium) — CONFIRMED, my bug, fixed

Verified against base rather than assumed: for each of the four names, base
`f9bef39b` has **1** registration and this branch had **2** — so this patch
introduced them, as the audit said. Root cause: when two sides of the rebase each
wired the same gate, I deduplicated the *comment* text and left both
registrations, and separately re-added `loop-preheader` and
`linker-oracle-verdict` while editing unrelated regions.

Fixed: **122 registrations / 118 distinct -> 118 / 118**, exactly the -4 the
audit predicted.

**I kept the second occurrence, not the first** (the audit said "keep the first of
each"). That instruction would have orphaned the rationale: the first
`volatile-pointer-subscript` / `volatile-access-semantics` pair sits alone, while
the *second* pair sits directly under the block explaining why those two scripts
were wired to NOTHING for months; likewise the first `loop-preheader` is bare
while the second carries the LOOP-PREHEADER-1 note, and the first
`linker-oracle-verdict` is stranded in the middle of the codegen-oracle group
while the second is in the linker block with the paragraph describing exactly
what it pins. The surviving copies are behaviourally identical, so this only
decides where the comments live — and they belong with their registration. All
four rationales are preserved, with the hoist-gate rationale merged into a single
coherent block instead of two overlapping ones.

## F4 (Medium) — CONFIRMED the diagnosis, REJECTED the prescribed fix

The diagnosis is right and I verified it: `check_ci_gate_parity.py:470` did
`set(COMMAND.findall(...)) - sourced_libraries()`, and a set erases the
multiplicity a duplicate is made of. That is exactly why my own parity gate
passed while four duplicates sat in the same file it was validating.

**The audit's prescribed fix is wrong, and I only know that because I
implemented it and measured.** It says to "compare `Counter` instead of `set()`".
I did that: it **fails the clean tree**, with four false positives —

```
ci_local.sh invokes the same gate command more than once:
  scripts/asmdiff.py: 2x
  scripts/check_volatile_destructuring.py: 3x
  scripts/fuzz_diff.py: 2x
  scripts/peephole_trace_bisect.py: 2x
```

All four are legitimate. `asmdiff.py` and `fuzz_diff.py` run once per mode, and
`test_ci_gate_parity.py` *requires* the i686 and x64 invocations to be distinct
("an i686 gate cannot stand in for the missing x64 corpus").
`check_volatile_destructuring.py` runs as `--self-test` and then for real — the
deliberate pattern that a broken parser cannot pass by matching nothing.
`COMMAND.findall` also matches paths named in **prose**, so a script mentioned in
a comment counts as an invocation. Command-path multiplicity is therefore not a
sound invariant for this file.

What I shipped instead asserts uniqueness on **gate names**, which is the
property that actually broke, and which fails the clean tree only if the tree is
genuinely broken:

- `check_gate_name_uniqueness()` in `check_ci_gate_parity.py`, imported `Counter`.
- PASS on the real tree (105 commands, unchanged output).
- Mutation-verified: duplicating `hot-loop-metric` fails with
  `"registered 2x at line(s) 401, 403"` and exit 1.
- 3 permanent tests in `test_ci_gate_parity.py` (suite went **21 -> 24**),
  including one that pins the design decision by asserting that the *same script
  under different arguments* is not a duplicate — so the tempting-but-wrong
  `Counter`-over-paths implementation cannot be reintroduced without a failure.
- The rejected alternative and its four counterexamples are documented in a
  comment at the decision site, so no one re-derives it.

## F4c (Medium) — CONFIRMED. My comment overclaimed; the gate's own header was honest.

The gate states plainly that no shape tried makes reduction detection fire on a
multi-entry loop, so these cases **do not currently reach the buggy branch**, and
that it is "a *behavioural smoke test and a compile contract*, not a reproduction
of the miscompile". My `ci.yml` comment credited it with catching the miscompile
"whether the cause is this pass declining wrongly or anything downstream". The
gate is right and I was wrong. Reworded to match the gate, including the
correction that the fix's correctness rests on the dominance proof at
`find_loop_preheader`, not on this gate overclaiming. The `ci_local.sh` comment
never made the claim (it says only "pinned against the host compiler at five
optimisation levels"), so it is unchanged.

## F5 (Low) — CONFIRMED, fixed; and I checked the audit had not under-reported

All four bindings were real and dead: `_debug`/`_changes` at both FMA helpers
(each pair immediately shadowed by the live `debug`/`changes` — a rename-sweep
leftover that also ran `env::var` twice per invocation for nothing), plus
`_original_block_count` and `_rem_iv_unused`. **6 lines removed**, 27156 -> 27150.

The same `FileLiveness` idiom appears in four *other* functions
(`eliminate_move_relays`, `fold_lea_into_load`, `retarget_producer_into_copy`,
`coalesce_copy_into_rmw`), so I checked each rather than assuming the audit had
found them all. All four **genuinely read `lv`** (4/3/2/1 real reads in
non-comment code, passed as `&lv` into `provably_dead_lv`). The audit did not
under-report; `fold_copy_into_lea_base` was the only write-only instance.

## F6 (Low) — CONFIRMED and independently verified

The comment claimed "CI runs clippy advisory (`continue-on-error: true`)".
Verified: `continue-on-error` occurs **0** times in `.github/workflows/ci.yml`,
and the job runs `cargo clippy --all-targets --profile fastbuild --locked -j 2 --
-D warnings`. Strict, not advisory. Rewritten to say so, and to state the
consequence that makes the list load-bearing: removing a category before its
sites are fixed turns hosted CI red, which is the intended failure mode.

## Optional item 8 — agreed with, and improved on

The audit asked to "narrow the two function-level `#[allow(unused_variables)]`".
Narrowing is the wrong target: a function-wide allow on
`try_emit_phase9_indexed_{store,load}` would mask a genuinely dead binding added
to the live prologue above the `return false`, which is the exact hazard the
crate-wide allow removal was about.

I removed both allows entirely instead. First I tested whether they were needed —
and they **were**: removing them produced six warnings (`unused variable: val`,
`ptr`, `ty`, `dest`, ...). That is worth recording, because it shows rustc
excludes the unreachable body from liveness, so the parameters really are unused.
The lint then names its own fix, and it is strictly narrower than an allow:
underscore the parameters. Applied to all six parameters; **0 warnings**; both
`#[allow]` attributes gone. The dead body still type-checks, so the
decomposition cannot silently rot while Phase 9 is disabled.

## Where I disagreed with the audit's own method

- The report says base `f9bef39`; the commit is `f9bef39b`. Trivial, but the
  audit asks for claims a reviewer can re-derive, so: `git rev-parse gh/main`.
- *"your local checkout is at `f9bef39` with the patch uncommitted, while the PR
  is `ba9cc96`"* — already false when written. My tree was committed
  (`8032ecbe`, clean `git status`) with the patch regenerated against
  `f9bef39b`, and the patch had been independently verified to apply in a fresh
  `gh/main` worktree.
- Its three "agreements" held up when I re-derived them independently:
  `Terminator` has exactly **6** variants (`Return`, `Branch`, `CondBranch`,
  `IndirectBranch`, `Switch`, `Unreachable`), exactly **4** carry block targets,
  and `find_loop_preheader` matches all four explicitly with a fail-closed
  `_ => false` plus the "several entries: not a single preheader" guard;
  `strict_external_value_available` takes **5** parameters at all **4** call
  sites; and the `licm.rs` delta is genuinely comment-only (25+/8-, zero
  non-comment added lines). I had claimed the LICM arm was "restructured"; it
  was not, and the audit is right to insist the delta be described as rationale.

## One opportunity found and deliberately NOT shipped

`fold_copy_into_lea_base` built a fresh `FileLiveness::new(store, infos)` on every
successful fold, where every other function in the file keeps one object and
calls `lv.refresh_at(store, infos, idx)`. Adopting the idiom would be more
consistent and would avoid a full liveness construction per iteration.

I did not do it, and the reason is the lesson already paid for once this session:

- `FileLiveness::new` analyses **every** `.cfi_startproc` region in the store;
  `refresh_at` re-analyses **one** function, via the bounds recorded in `func_of`.
- `refresh_at` has an **early return** (`liveness.rs:503`, when the backward scan
  meets `.cfi_endproc` before `.cfi_startproc`) that leaves the *previously
  computed* liveness in place — a stale-data path `new` does not have.
- The fold's rollback branch rewrites lines after the query, so which of those
  two behaviours you observe is exactly the case that is hardest to construct.

Changing a fail-closed proof path like this cannot be validated by a
byte-identical corpus A/B — the corpus demonstrably never exercises the rejection
branch, which is *how the dropped-guard bug survived a whole session*. So it goes
to the backlog behind the item that already exists for exactly this reason: the
IR-level test that makes the rejection path actually fire. Ship it after that
test exists, not before.

## Status

- `cargo fmt --check` green; build 0 errors / 0 warnings; `ci.yml` parses as YAML;
  `doc-link` gate green; parity checker green (105 commands) *and* now able to
  fail on the duplicate it could not previously see.
- **150/150 asm outputs byte-identical** to the pre-change tree, so every change
  in this session is behaviour-preserving.
- `ci_local.sh --fast` re-run on this tree; outcome recorded in the handoff.
- No claim in this document is asserted without a command that was actually run
  on this tree.

## Audit observations I checked and found NOT to be defects

The audit's closing notes listed three items it did not raise as findings. Two
are worth adjudicating explicitly, because "true but harmless" and "true and
serious" look identical in a list.

**"3 changed scripts lack `set -e` (pre-existing)."** True, and for the gate that
matters it is **not** a defect. `check_loop_preheader.sh` — the one this branch
extends — omits `set -e` but is fail-closed five ways over:

1. Every check routes through `fail()`, which sets `rc=1`; the script ends
   `exit $rc` (`rc=0` initialised at line 56, `exit $rc` at line 189), so no
   check can be skipped by accumulation.
2. Both compile steps are guarded explicitly:
   `|| { fail "cannot emit asm (on)"; exit 1; }`.
3. `pipefail` is set, so a failure inside a pipeline is not swallowed.
4. `loop_region()` — which extracts the loop body that every effect contract
   measures — fails closed twice: `if (!last) exit 1` when no backward branch is
   found, and `if (!start) exit 1` when the target label is missing.
5. **The negative control closes the remaining hole.** If (4) fails for *both*
   builds, both in-loop counts are 0, and contract 2's `on_in -eq 0` test would
   pass vacuously — which is why the very next check requires `off_in -ge 1` and
   fails with "the pass-off build also has no in-loop load, **so contract 2
   proves nothing**". The author of that gate understood the vacuity trap and
   built the guard for it.

I also verified it empirically rather than by reading: `LCCC_BIN=/bin/false bash
tests/regression/check_loop_preheader.sh` exits **1** and prints both
`FAIL: compile failed with the pass on` and `... off`. Combined with the recorded
mutation result (`enabled()` forced to `false` fails 4 contracts, including 2b),
this gate is demonstrably capable of failing.

`lib_loop_bounds.sh` is a sourced library and correctly does not set `set -e` on
its caller's behalf; `ci_local.sh` is the harness and checks the exit status of
every gate it runs.

**"The preheader `debug_assert!` is release-off (acceptable — the gate is the real
guard)."** Agreed, and that is the correct allocation: `debug_assert!` compiles
out of the fastbuild/release profiles, so a `debug_assert!` on a
catastrophically-reparenting path would be exactly backwards. The runtime guard
is the `strict_external_value_available && strict_cfg_dominates` conjunct, and
contract 2b is what pins its magnitude. This is the same lesson as the earlier F8
finding (a `debug_assert!` guarding a catastrophic reparent), applied.
