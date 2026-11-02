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

---

# Addendum — second pass, 2026-09-30 (evening). The headline finding is now proven at byte level, one finding is fabricated, and the lint story inverts.

Base still `f9bef39b` (verified in sync with `gh/main` immediately before this
section was written). Tree after this pass: the `ci_local.sh --fast` green
revision. This addendum **supersedes the lint argument in F1/Optional-item-8
above** with a stronger one; the earlier *conclusions* are unchanged, the
*reasoning* is now primary-evidence rather than inference.

## A1 — the `fold_induction_copyback` refusal was a real miscompile, now reproduced byte-for-byte

The first pass established the bug statically: `fold_induction_copyback`
precomputes each use's rewritten form, sets a refusal flag, then commits
regardless, so a use it could not rewrite survives next to a deleted copy.
Static analysis could not settle whether the refusal path is *reachable*, and it
is not reachable by guessing — three hand-written shapes were eliminated by
other passes before the fold ever saw them.

Two conditions are needed, and both were found by instrumenting the pass and
reading the markers back out of the built binary (`strings <test-bin> | grep`
first, so that silence could be trusted):

1. the copy destination `%D` must stay **live** (a `movq %rax, %rdx` whose `%rdx`
   is dead is deleted by an earlier pass, and the fold never sees it — this,
   not any guard, is why hand-written shapes kept missing);
2. the producer must be a `leaq`/load (`parse_copyback_producer` accepts nothing
   else), and one use must be **inexpressible**: `register_family_at` places the
   high-byte aliases `%ah`/`%bh`/`%ch`/`%dh` in the `%eax` family, but they have
   no entry in `REG_NAMES` at any width, so `replace_reg` matches nothing and
   `rw == line` triggers the refusal.

With both in place the differential is unambiguous. Same input, same pipeline,
only the code path under test differs:

| code path | emitted |
|---|---|
| `gh/main` | `leaq 8(%rdi), %rdx` / `movzbl %ah, %r8d` / `addq %rax, %r8` |
| fixed | `leaq 8(%rdi), %rax` / `movzbl %ah, %r8d` / `addq %rax, %r8` |

In `gh/main`'s output **`%rax` is written by nothing**: the producer was
retargeted into the copy destination and the copy deleted, so the two surviving
`%rax` reads are undefined. This is a wrong-answer bug, not a missed
optimisation, and it is worse than the audit described: the audit said the uses
read a `%T` "the deleted copy no longer defines"; in fact the copy's
*destination* is written and dead, while `%T` itself is never written at all.

**The fix is not the audit's fix.** The audit proposed reading the flag. The
shipped fix makes non-consumption a *type error*: `plan_copyback_rewrites`
returns `Option<Vec<(usize, String)>>`, and the caller is forced to handle
`None` with `continue`. A `bool` is state a programmer must remember to consume
— which is exactly what failed — whereas an `Option` is state the compiler
forces the caller to consume. The bug class is removed, not the instance.

Two tests pin it, both passing: `a_non_rewritable_use_aborts_the_whole_copyback_fold`
asserts the discriminating invariant (*the register the surviving uses read must
be the register the producer writes*) rather than the literal copy instruction,
because a later legitimate copy-propagation pass deletes the copy either way;
`a_copyback_fold_still_fires_when_every_use_is_rewritable` is the positive
control — without it, a guard that refused everything would satisfy the
regression test while silently disabling the fold.

## A2 — the audit's dead-binding finding is **fabricated**, and the real class is 20× larger

The audit's third finding named four dead bindings: `has_indirect_call`
(`prologue.rs` L580/L617), `has_i32_widening` (L588/L639), `found_any_mul`,
`k_used`, each "0 reads".

None of them exist. Crate-wide grep over `src/`:

| identifier | hits |
|---|---|
| `has_indirect_call` | **0** |
| `has_i32_widening` | **0** |
| `found_any_mul` | **0** |
| `k_used` | 1 — a substring of the test name `a_mask_used_as_a_value_provably_diverges` |

A finding whose identifiers cannot be located in the tree cannot be accepted on
authority, and this one also cannot be *acted on*: there is nothing to delete.
It is rejected as stated. (The honest failure mode to look for is the opposite
one: an audit that names real identifiers but hallucinates their line numbers.)

The *class* is nevertheless real and much larger than four. Rather than argue,
I made rustc enumerate it: `unused_mut` and `unused_assignments` were un-allowed
and the crate re-checked, giving **80 warnings** — 67 `unused_mut` and 13
`unused_assignments`. Both are now enforced crate-wide and the count is **0**.

The 67 unnecessary `mut`s were removed with the compiler's own machine-applicable
suggestions (`cargo fix --lib --tests`), not by hand and not by regex. The 13 dead
assignments each needed judgement, because deleting an assignment whose
right-hand side has side effects would be a bug:

| site | what it was | action |
|---|---|---|
| `elf_writer_common.rs:3577` | `let mut any_change = false;`, reassigned before any read | uninitialised local |
| `elf_writer_common.rs:4336` | `tight_resolved_align = None;`, always overwritten | removed |
| `i686/codegen/peephole.rs:7060` | `k += 1;` immediately before `break` | removed |
| `x86/codegen/emit.rs:8165` | `window_last_writes_fast = None;`, overwritten on every path | uninitialised local |
| `x86/linker/emit_exec.rs:1690` | section-header index `h += 2;`, nothing reads `h` after it | removed |
| `x86/linker/emit_script.rs:3388` | `ph_off += phdr_size;` immediately before `break` | removed |
| `x86/linker/emit_exec.rs:215`, `link.rs:222` | the last lap of the `zone!`/`phase!` timers | **structural**, see A3 |
| `frontend/parser/nested_functions.rs:163` | computed `alignment`, discarded | **removed + documented**, see A4 |
| `frontend/preprocessor/macro_defs.rs:759` | `q += 1;` immediately before `break` | removed |
| `passes/bool_thread.rs:217` | `phi_dests = Vec::new()`, reassigned on every live path | uninitialised local |
| `passes/vectorize.rs:2869` | `primary_loads = Vec::new()`, reassigned on every live path | uninitialised local |
| `x86/codegen/peephole/passes/tail_call.rs:272` | `frame_release`, never read at all | removed, with its only assignment — and with the `let _ = &frame_release;` that existed solely to keep a lint quiet |

Only one of the 13 is a *bug* rather than dead weight (A4). The rest are dead
state, and the interesting part is that two of them were load-bearing *for other
expansion sites* of the same macro, so the "obvious" deletion would have silently
corrupted the diagnostics (A3).

## A3 — two of the thirteen are live for a *later* expansion, so deletion would have been wrong

`x86/linker/emit_exec.rs` defines `zone!` (6 invocations) and `link.rs` defines
`phase!` (9 invocations). Both print a lap and then restart the clock:

```
$name,
t_zone.elapsed().as_secs_f64() * 1e3
);
t_zone = std::time::Instant::now();
```

The reset is read by the *next* expansion; only the final expansion's reset is
dead — which is exactly what rustc reports. Deleting the line would have been a
behavioural regression in the timing diagnostics: every subsequent lap would
measure from the start of the function, cumulatively. This is the trap in the
audit's implicit framing ("dead assignment ⇒ delete it").

Fixed structurally instead: a shared `linker_common::lap_timer::LapTimer` whose
`lap_ms()` reads the previous checkpoint and installs the new one, so *every*
lap exercises the contract and there is no free-floating reset to be dead. The
duplication between the two drivers disappears with it. Runtime-verified against
a real link with `LCCC_LD_TIME=1`: 16 lap lines, correct per-lap deltas
(`emit/headers` reports `0.0 ms`, not a cumulative value), and the linked binary
is byte-identical with and without the flag (`cmp` clean).

## A4 — one dead assignment was a real dropped feature, and the fix was documentation

`nested_functions.rs` parsed `aligned(...)` off a nested function's declarator
and combined `decl_aligned` with `post_aligned` into `alignment` — which nothing
could consume. The reason is precise: alignment reaches codegen through
`IrModule::function_alignments`, keyed by the **plain** function name
(`global_decl.rs` inserts from prototypes; `generation.rs` emits `.p2align` for a
definition out of that map), while a nested function's IR name is **mangled**
`parent.inner` (`nested_functions.rs:879` and `:1258`). A value computed here
could therefore never match the lookup, and the drop is documented as
intentional at the `FunctionDef` construction.

So the fix is not to delete silently and not to wire it up cosmetically: the
computation is removed, the discard is marked at the destructuring sites
(`_decl_aligned`, `_post_aligned`), and both comments now say where function
alignment *actually* comes from and why the definition's own attribute cannot
reach it. Tracked as `ALIGN-1` in `backlog.md`: `aligned()` on a function
*definition* (top-level or nested) is currently ignored, prototypes are the
supported channel, and wiring the definition path needs the mangled key.

## A5 — the lint story inverts: the PR is what makes the bug detectable

The audit's headline was that deleting four lines "killed the only warning".
Verified from the allow lists themselves:

* `gh/main:src/lib.rs` allowed **six** lints, including both `unused_variables`
  *and* `unused_assignments`;
* this PR removes `unused_variables` from that list;
* this pass additionally removes `unused_assignments` and `unused_mut`.

Under `main`'s own allow list, the four deleted lines produced **no warning at
all** — `unused_variables` does not fire on a binding that is assigned, and the
dead store it contained is `unused_assignments`, also allowed. So the deletion
silenced nothing.

The decisive experiment: put `gh/main`'s exact code back (verbatim — the
declaration, the assignment, the `break`, no `let _ =` hedge) and compile it
under the policy this PR establishes:

```
warning: variable `all_rewritable` is assigned to, but never used
warning: value assigned to `all_rewritable` is never read
```

**Two warnings, on `main`'s code, under this PR's lints.** The audit asked why
the evidence was deleted instead of the bug fixed; the measured answer is that
this PR is the change that makes the bug *visible at all*. Deleting the flag
does not hide a warning — it removes the dead state the warning was pointing at,
and the tightened lint set is what would have caught it earlier.

The same logic applies to the comment in this PR's CI hunk that says the
volatile/LICM deletion "linted clean only because `src/lib.rs` allowed
`unused_variables` crate-wide". That comment is **correct**: `unused_variables`
was in `main`'s allow list, and this PR removes it.

## A6 — the commit-message correction, stated exactly

The patch is a flat `git diff` and contains no commit messages (`grep -c
'^Subject:\|^From [0-9a-f]' ms178-1.patch` → 0), so this cannot be fixed from the
deliverable. It is owed on the GitHub commit message / PR description:

* **Drop** the clause implying `312 → 259` instructions is a result of this PR.
  `LateMinMaxOnlyScope` and that measured effect are **already on `main`**; the
  hunk in this PR that touches that area is comment-only, so it cannot have
  caused any measurement.
* The in-tree evidence note in `src/passes/mod.rs` was already careful (it names
  the causal-isolation method, `CCC_DISABLE_PASSES=latevec`, and reports the
  measurement as the shape of the flag). One clause was added so a reviewer
  cannot misread it: the flag and its effect are already on `main`, and this
  commit's changes in this area are documentation only.

## A7 — gates, after this pass

| gate | result |
|---|---|
| `cargo test --lib` | **3880 passed, 0 failed, 7 ignored** |
| `cargo check` (fastbuild) | 0 errors, **0 warnings** with `unused_mut` + `unused_assignments` enforced |
| `cargo clippy --all-targets -- -D warnings` | **green** (the gate's exact command) |
| `cargo fmt --all -- --check` | **green crate-wide** (19 files were dirty; all 19 were files this PR touches) |
| `ci_local.sh --fast` | **117 passed, 0 failed, 5 skipped** |
| end-to-end | `lccc -O2` compiles and runs a real program (`1499500`, correct); `LCCC_LD_TIME=1` laps correct; linked binary byte-identical with the flag on and off |

Two failures at the first attempt have one root cause worth recording: the new
lint enforcement reaches **test-cfg** code, which `cargo fix --lib` does not, so
`clippy` and `cargo-test` — both of which deny warnings — went red on three
`unused_mut` sites in `#[cfg(test)]` code. `cargo fix --lib --tests` cleared
them. Any future tightening of a crate-level lint must be validated through the
gates, not through `cargo check` alone, because `cargo check` only covers the
library target.

## A8 — competing PR #693 (`f1780b27`): what is verified, what is not

Standing red-team scope. **Verified from the tree and the diff** (not from its
description):

| axis | #693 | this PR |
|---|---|---|
| touches `narrow_copy_fold.rs` | **no** | yes — and it is the file the audit's High finding lands in, so #693 leaves that miscompile in place |
| generated `.s` files | **61**, all at repo root, **+21 483 lines ≈ 84 % of the PR**; 60 of 61 referenced by nothing | 0 (the only root-level file added is `backlog.md`) |
| compiler `src` files | 7 (`flag_peepholes.rs` +151, `loop_preheader.rs` +111, `vectorize.rs` +193/−322, `generation.rs` +19, `licm.rs` +9, two `mod.rs`) | 128 files, dominated by the dead-state removal and the fold fix |
| dead-state lints | untouched — `unused_mut`/`unused_assignments` still allowed crate-wide | enforced crate-wide, 80 warnings → 0 |

**Credit where it is due, and this is the part I do not have.** Its new
`fold_redundant_flags_compare` is a *capability* absent from my PR: it drops a
second `cmp`/`test` across a flag-preserving run (`cmov`/`setcc` read EFLAGS
without writing them), which is a real instruction removed from SQLite's varint
decode loop — one dead instruction on the slowest arm of the hottest kernel
shape. Its own documentation is unusually honest: it records that dropping the
operand-write guards miscompiles `bb_slp_i64_to_i32_select` (`gt_big[2]: got 6
want 100000`) and `array_string_init_matrix_O0`, that a hand-written fixture does
*not* reproduce it (allocation gives the two compares different registers), and
that memory operands are refused outright because identical text over memory is
not identical semantics. It keeps the guards. Its `vectorize.rs` dedup is net
−129 lines and **preserves both** `strict_external_value_available &&
strict_cfg_dominates` guards — worth stating, because that is the guard a careless
refactor drops.

**Not verified — stated as such.** I have not built #693, so I make no claim
about whether its pass fires on the corpus, whether `CCC_PEEPHOLE_SKIP=flags_compare`
is a true negative control, or whether the two named tests pass *with* the guards
present. Reading a pass and judging it sound is not the same as running it, and
the flag-with-no-`.s`-artefact decision (61 committed snapshots, 60 unreferenced)
is a review-quality question I have measured but not adjudicated on its merits.

**What I will take from it:** the flag-preservation argument (`cmov`/`setcc` are
flag *readers*, `adc`/`sbb`/`rcl`/`rcr` are the ones that read *and* write) is a
correct insight, and the operand-write guard it almost shipped without is the
same failure mode as A1 — a transform whose commit step is not justified by the
preconditions it checked. If adopted, it belongs behind a corpus run first, and
with the same precompute-then-commit discipline as `plan_copyback_rewrites`.

---

# Addendum B — third pass, 2026-09-30 (night). The volatile-bitfield bug is real, five of six findings land, and one is understated.

Base `f9bef39b` (unchanged; verified in sync with `gh/main` before this section).
Verdict on this audit: **5 of 6 findings confirmed, 1 confirmed-and-worse,
1 correction that was mine to make and is now made, and 2 of its prescribed
fixes improved on with measurements.** Its score of 6.5/10 is defensible on the
numbers it could see; the readability complaint (item 5) is the one I most agree
with and acted on most aggressively.

## B1 (High) — CONFIRMED, and the real defect is 8 sites across 3 functions, not 4 across 2

This is the most important finding either audit has produced, and it is correct.
I did not take the mechanism on faith; I measured it. But the audit's own
description understates it in two ways.

**What is true.** `store_bitfield_split` took `volatile: bool` and never read it:
`gh/main` has the parameter as a live name, my PR renamed it to `_volatile` when
the crate-wide `unused_variables` allow was removed, and all four of its
accesses were built with `volatile: false`. So the parameter was *warning* that
the flag was dead, and the rename silenced the warning instead of using the flag.
That is a fair hit and it is the sharpest thing anyone has said about this PR:
the change that removes a lint allow is exactly the change that must not then
hide what the lint finds.

**Understated, part 1 — the class is wider.** Unused volatility is not confined
to the split path:
* `store_bitfield` (the *non*-split function, which does read its `volatile`
  parameter and uses it correctly at its main store) still hardcoded
  `volatile: false` in its **full-width** branch.
* `extract_bitfield_from_addr` did not take volatility *at all*, so **all three**
  of its loads were non-volatile — including the ordinary single-unit load, not
  just the split pair. Reads were affected, not only writes.

After the fix: **8 hardcoded sites** now carry the flag, and volatility is
resolved **once** at the member-access entry
(`lower_member_access`/`lower_member_access_via_pointer`) and passed down, so the
split and non-split paths cannot disagree. The three load-side call sites
(`expr_access.rs`, `expr_assign.rs` compound assign, `expr_ops.rs` inc/dec) each
supply it from the same `expr_access_is_volatile` helper the plain path uses.

**Understated, part 2 — the effect is a wrong answer, not just a lost access.**
Measured on `-O2 -S`, GCC 14 as the oracle, before the fix:

| probe | source | LCCC before | LCCC after | GCC 14 |
|---|---|---|---|---|
| `vol_write_loop` | `vf.all = x` × N, object-level volatile | **1 store** | **N stores** | N stores |
| `vol_read_const_loop` | `s += vp.small` × 4 unrolled | **1 load** | **4 loads** | (4 loads) |
| `vol_read_loop` | `s += vp.small` × N, unknown trip | **1 load** | **4 loads** | 4 loads |

A loop of N volatile writes collapsed to one store; a volatile read was hoisted
out of its loop. That is C11 5.1.2.3 ("Every access to such an object is done as
a single access, in program order") broken *observably*, in the same class as the
MMIO spin-loop hang the other volatile gates exist for.

**The gate, and it discriminates.** `tests/regression/check_volatile_bitfield_split.sh`
asserts, over three shapes (single-unit, full-width, packed split), that every
source access survives, with non-volatile twins of the same shapes as controls
that MUST be merged. Verified both directions on real builds:

```
pre-fix binary:  3 FAIL   (vol_full 1<2, vol_read_const_loop 1<3, vol_write_const_loop 1<3)
post-fix binary: 0 FAIL
```

The three failures are the three manifestations, not three phrasings of one.

**Where the audit's fix was followed to the letter:** the parameter is named
`volatile` again and the flag is passed to all four accesses. **Where it was
extended:** the audit asked to "check the matching split *load* path"; that path
has no parameter to check, so it gained one, which also fixed the non-split load.

## B2 (Medium) — CONFIRMED. The debug check was paying the cost it documents.

Verified by reading it: the assertion nested `body.iter()...all(|&b| !body.iter().all(...))`,
which is the O(|body|²·depth) walk the header-only shortcut exists to avoid —
in debug builds only, which is why it survived review.

The replacement is the audit's, and the equivalence is worth stating because it
is the whole reason it is safe: *"some non-header block dominates every loop
block"* and *"some non-header block dominates the header"* are the same
statement, because dominating the header is necessary and the header is itself a
loop block. One dominance walk per block instead of per pair. The error message
is unchanged, and it is still a `debug_assert!` for the reason the earlier
comment gives (the *runtime* enforcement is the `strict_*` conjunct).

## B3 (Medium) — CONFIRMED in principle, but the audit's list is the easy half. 44 dead bindings removed, 30 kept with reasons.

The audit named five sites. All five are real and all five are gone. But the
instruction was to sweep `git grep -nE 'let _[a-z]'` and justify or delete each
hit, so that is what I did: **86 hits, classified, 44 deleted, 42 kept.**

Kept, because deleting them changes behaviour — these are RAII guards whose Drop
restores state, and `let _ = guard;` would drop immediately instead of at end of
scope, which is the classic footgun the *name* prevents:

`EnvGuard`/`ScopedFlag` (test_support, passes/mod, loop_align, vec_interleave,
vec_load_sink, location_alloc/policy), `TriStateFlagWindow` and
`RestorePointerSize`, `EnvWindow`, `Target::set`, `LateMinMaxOnlyScope`, plus one
uninitialised declaration (`let _symtab_shidx;`). 30 of the 42 are
`EnvGuard`/`ScopedFlag` windows.

Deleted — and two of these are not "harmless dead weight", which is why the
sweep was worth doing rather than the five named sites:

| site | what it was | why it mattered |
|---|---|---|
| `generation.rs` `_indexed_gep_map` | a whole map built per function, then discarded | pure waste on every function |
| `if_convert.rs` `_resolve`, `verify.rs` `_push` | closures, never called | a dead closure reads as a helper in use |
| `narrow.rs` `_changes = narrow_function(..)` | **call kept**, binding dropped | the call has effects: `narrow_function(&mut func)` |
| `riscv/…/emit_shared.rs` ×12 `_sh_name = add_shstrtab_name(..)` | **calls kept**, bindings dropped | appends to the shstrtab |
| `linker_script.rs` `_common = parse_expr(lx)?` | **call kept** | advances the lexer |
| `dead_code.rs` `_base = fields.next()?` | **call kept** | advances the iterator |
| `i686/linker/reloc.rs` `_out_name = match .. { None => continue }` | rewritten as `if ….is_none() { continue; }` | it was a control-flow guard wearing a binding |
| `vectorize.rs` `_iv_width_const` → `vec_width` parameter | parameter and argument deleted | the constant's only consumer was itself; the parameter became unused, so the lint cascade removed it |

The last row is the audit's own thesis demonstrated: removing dead state exposed
more dead state, and **the enforced lints caught it** (`unused variable: vec_width`)
rather than leaving a silently-unused parameter behind.

Two splice mistakes were made and caught by the compiler during this work — an
index-based deletion swallowed the following statement in `reloc.rs` and a
`vec_width` argument in `vectorize.rs`. `cargo check` named both. Recording them
because the lesson is that a bulk dead-code sweep must be compiled after *every*
batch, not at the end.

## B4 (Low) — CONFIRMED and fixed, plus the docstring's second claim was also stale

Line 25 said `src/lib.rs` carries `#![allow(unused_variables)]`. It does not any
more; the docstring now says so, and explains what the two rules cover given the
lint is back on.

## B5 (Low) — CONFIRMED, and this is the criticism I most agree with

Comments that narrate the diff rather than the code are a real cost, and "408 of
868 added lines in `src/` are comments" is a fair measurement of my own output.
Four sites were rewritten to state the invariant without the history:
`loop_preheader.rs` ×3 and `ci.yml` ×1. "An earlier revision of this comment used
that spelling as the motivating example", "was the first attempt", "both sides of
this rebase" are gone; the technical content they surrounded is kept, because the
*why* of a header-only scan and the *why* of a refusal-over-assertion are things
a future reader needs.

What I did **not** do is delete the measured-negative-space comments (why a
transform is refused, what was measured and came out neutral). Those are the
opposite of narration — they are the evidence that stops the next person
re-adding a regression — and the same audit credits that style elsewhere.

**Scope check, because "narration" is only a defect where I introduced it.**
`grep` over every `src/` file this PR touches finds none of the three named
phrases any more. The same style does appear ~43 times across 35 files on `main`
(``used to be``, "an earlier revision"), i.e. it is the house style rather than
something this PR brought in — so the finding as written is about *volume* of
comment, which is a judgement call I have partly acted on, not about a new bad
habit. Rewriting `main`'s 43 would be a large unrelated diff.

## B6 (Low) — acknowledged, not actioned, and the reason is on the record

~2,560 lines of `engineering/` process documents. They are this repo's
convention, the audit says so, and it also says they are noise for reviewers. Both
are true; the convention wins, because the alternative is that the measurements
behind every "no" in this PR disappear into a chat log. The mitigations that cost
nothing are taken: the docs are additive-only files, and the code comments no
longer point at them as if they were design docs.

## B7 — the commit-message correction, and the one the audit did not make

The audit's own correction ("the `licm.rs` change only touches comments") is
right, and it is now the second time this has been caught: the `312 -> 259`
attribution for the comment-only hunk in my PR description was the first.

The patch is a flat `git diff` and cannot carry commit messages
(`grep -c '^Subject:' ms178-1.patch` → 0), so both corrections are owed on the PR
body and are recorded here as the exact text to use:

* `licm.rs`: **documentation and tests** around an existing `!*volatile` guard.
  Not a restored guard; the check was already on `main`.
* `src/passes/mod.rs`: **comment-only**; `LateMinMaxOnlyScope` and the
  `312 → 259` measurement are already on `main`.

## B8 — the volatile ratchet, extended as asked, and it now catches the bug it missed

`scripts/check_volatile_destructuring.py` inspected `Instruction::Load/Store`
destructuring patterns and nothing else, so a volatility **parameter** was
invisible to it — which is precisely how `_volatile` shipped.

Added a second rule: a parameter whose *name* contains `volatile` must be read in
its own body, and a `_`-prefixed one is a violation outright (the underscore
*asserts* the flag is unused, which for this parameter can only be wrong), with
`let _ = volatile;` as the explicit escape hatch. Six self-test cases were added,
including the old `store_bitfield_split` signature **verbatim**.

Two details that make it a gate rather than a rubber stamp:
* The use test is negative-lookahead (`\bvolatile\b(?!\s*:)`) so that
  `Load { volatile: false, .. }` inside the body — a field name, which is exactly
  the shape of the bug — does not count as a use. My first version got this wrong
  and one of the new self-test cases failed; the case stays because it is the
  non-obvious half.
* It is registered in CI after its `--self-test`, and the self-test now has 21
  cases.

Evidence: on the **pre-fix** file it reports
`expr_assign.rs(pre-fix):487: volatility parameter '_volatile' of 'store_bitfield_split' …`,
and on the fixed tree it is clean over 497 files. So the rule would have caught
the bug before review.

## B9 — ALIGN-1: implemented for the channel that has an oracle, and measured against it

`ALIGN-1` from the backlog: `aligned()` on a function **definition** was parsed
and discarded, so only the prototype channel reached codegen. Implemented and
gated, with the acceptance table now measured on real builds:

| function | attribute on | before | after | GCC 14 |
|---|---|---|---|---|
| `via_def` | definition only | `.p2align 4` | **`.p2align 6`** | `.align 64` |
| `via_proto` | prototype | `.p2align 6` | `.p2align 6` | `.align 64` |
| `plain` | none | `.p2align 4` | `.p2align 4` | `.p2align 4` |
| `proto_then_def` | prototype + definition | `.p2align 6` | `.p2align 6` | `.align 64` |

`check_function_alignment_definition.sh` asserts exactly these, and it was run
against a **stashed-fix build**: `via_def` FAILs (4 ≠ 6) while the other three
pass, which is the predicted single-cell change and proves the control row is
doing work. The gate measures *effective* alignment as the maximum over the
directive run preceding a label — because LCCC emits a redundant trailing
`.p2align 4` after the attribute directive, and "last directive wins" would read
that as an override when it pads nothing.

**Nested functions were tested and then deliberately left alone.** GCC applies no
observable alignment to a nested function definition (measured at `-O0` and
`-O2`, with and without `noinline`); LCCC does not either. So there is no oracle
to match, and honouring the attribute would be a divergence. The wiring that
would have made LCCC honour it was written, **found not to fire**, and removed
rather than shipped as code claiming a behaviour — the same standard this pass
applies to everyone else's dead code.

**Still open, tracked as ALIGN-2:** the redundant trailing `.p2align N` after an
attribute directive is harmless (it pads nothing) but it hides which directive
carries the attribute from anyone reading the asm.

## B10 — #693 follow-up, executed: its claims are now verified, not read

Addendum A8 said the flag-peephole pass was unverified because I had not built
#693. Built it (`f1780b27`, `scripts/build_lccc_fast.sh`, BUILD_EXIT=0) and ran
its own evidence.

**Its gate passes, and the control is real.** `check_redundant_flags_compare.sh`
on its compiler:

```
runtime: matches gcc (1611652143)
shape:   one cmpb feeding the tail-arm cmovs (was 2)
shape:   2 cmovb present, so the fold removed a cmp and not a cmov
control: disabling the pass restores the duplicate (2 -> 1 with it on)
PASS
```

That is the shape assertion *and* the negative control, with a GCC runtime
comparison on top. It fires on the SQLite varint fixture, removes exactly the
duplicate `cmp`, leaves the `cmov`s it feeds, and `CCC_PEEPHOLE_SKIP=flags_compare`
puts the duplicate back. Credit where it is due: this is a capability my PR does
not have, and its evidence is honest.

**The two tests its doc names as miscompiling without the operand-write guards
pass with the guards in place** — `bb_slp_i64_to_i32_select` and all three
`array_string_init_matrix` variants (`PASS=1`, `PASS=1`): so the guards it kept
are load-bearing and it did keep them. (The i686 variant initially failed for an
environment reason on this box — no i386 multilib — which was fixed by installing
`libc6-dev-i386 gcc-multilib g++-multilib`; the test then passed. Recording it
because "test failed" and "test could not build" are different statements and I
initially reported the wrong one to myself.)

**What still stands against it, measured:** it does not touch
`narrow_copy_fold.rs`, so the refusal-path miscompile in A1 is untouched by it;
and the 61 generated `.s` files (60 unreferenced, +21 483 lines ≈ 84 % of the PR)
remain in its diff.

**Net:** #693's flag pass is good work I would take as-is; #692's contribution is
the dead-state lint ratchet plus the volatile and copyback correctness fixes it
exposed. They are not competing for the same defect, and the honest answer to
"is theirs better" is "on flags, yes; on the defects I found, it does not reach
them."

---

# Addendum C — the two defects the gate could not see, and the CI flake the audit could not run

Same base `f9bef39b`. This addendum answers the review as it stands, and records
what a *testing* pass found that a reading pass did not.

## C1 — Item 1 (volatile bitfield): agreed, and it was worse than the review said

The review is right that `store_bitfield_split` took a `volatile` parameter and
never read it, and right that renaming it to `_volatile` turned a warning into
silence. Beyond the review: the class was 8 sites in 3 functions, not 4 in 2.
`store_bitfield`'s **full-width** branch was also hardcoded non-volatile, and
`extract_bitfield_from_addr` took no volatility parameter at all, so **all three
of its loads** were non-volatile — reads, not just writes.

Measured against GCC 14 before the fix: a loop of N volatile full-width stores
collapsed to **one** store, and a volatile bitfield read was hoisted out of its
loop. That is a wrong answer (C11 5.1.2.3), not a lost optimisation. After the
fix all three probes match GCC. `tests/regression/check_volatile_bitfield_split.sh`
fails 3 probes before the fix and 0 after, with non-volatile twins as controls
that must still merge.

## C2 — Item 3 (dead code renamed, not deleted): agreed, and the sweep was worth doing

86 underscore bindings classified; 44 deleted, 42 kept with reasons. The review's
five named sites are all gone. Two of the deletions were not "harmless dead
weight": a whole per-function map built and discarded in `generation.rs`, and two
never-called closures that read as helpers in use. Several were *call* sites
where only the binding was dead (`narrow_function(&mut func)`, 12×
`add_shstrtab_name`, `parse_expr(lx)?`, `fields.next()?`) — those calls were kept,
because they have effects. Removing one dead binding made its parameter unused,
and the enforced lint caught that too, which is the cascade the item is about.

## C3 — Items 2, 4, 6: agreed, fixed or answered

Item 2 (the quadratic debug check) — agreed and fixed with the equivalence
argument recorded: "some non-header block dominates every loop block" and
"…dominates the header" are the same statement. Item 4 (stale docstring) —
agreed, fixed. Item 6 (process docs) — acknowledged; the repo's convention is to
keep the measurements behind every "no", and the mitigations that cost nothing
are taken (additive files, no code comments pointing at them).

## C4 — Item 5 (comment volume and history narration): partly disagreed

Acted on: the three sites the review named ("an earlier revision of this
comment…", "was the first attempt", "both sides of this rebase") are gone, and
this follow-up round added no new prose of that kind to `src/`.

Disagreed with the framing, on a measurement: those phrases appear **~43 times
across 35 files on `main`** — it is the house style, not something this PR
introduced, and rewriting `main`'s 43 would be a large unrelated diff inside a
correctness PR. The review's own summary ("about 408 of 868 added lines are
comments") is a fair statement about *volume* and I accept it as a preference:
this round added comment lines only where a measurement or an invariant would
otherwise be lost (the `aligned(1)` state, the replace-not-merge rule, the
SIGPIPE mechanics), and those are the comments a future reader needs to avoid
reintroducing the bugs.

## C5 — Item 6 wording: done, and the flat patch cannot carry it

The `licm.rs` change is documentation plus tests around an existing `!*volatile`
guard, and `src/passes/mod.rs` is comment-only; the `312 → 259` and
584-byte-frame numbers belong to `main` (#686), not to this PR. Both are stated
in the commit message, and — because `ms178-1.patch` is a flat diff with zero
embedded commit messages (`grep -c '^Subject:'` → 0) — they are also stated in
the patch's own documentation.

## C6 — What testing found that reading could not: my own ALIGN-1 was half right

`ALIGN-1` (definition-channel `aligned(N)`) was implemented, gated and
mutation-proven — and still wrong, in a way its gate could not see, because
every function in the gate's fixture was **parameterless**.

Two measured defects:

* **A parameter list swallowed the attribute.** The alignment pending when a
  parameter list begins belongs to the enclosing declaration; the per-parameter
  attribute capture merged into that slot and took it. So `aligned(64) void
  f(int x) { }` emitted `.p2align 4` — and so did the *prototype* channel
  (`aligned(64) void f(int x);` registered nothing), which had the same take. The
  original ALIGN-1 work thus fixed only the parameterless spelling.
* **An attribute is not a floor, it is a replacement.** GCC with
  `-falign-functions=32` still emits `.align 2` for `aligned(2)`, and emits **no
  directive at all** for `aligned(1)`. LCCC emitted the attribute directive and
  let the default follow it; since `.p2align` only ever advances the location
  counter, the pair meant max(N, 16) — `aligned(2)` was 16-byte aligned and
  `aligned(1)` was 16-byte aligned. Both are now one value in one place
  (`Option<Option<u32>>`: absent / fixed / natural), consumed by every placement
  site, which also collapses the two directives per entry to the single directive
  GCC emits.

Verified in the object file rather than the text: with a one-byte pad at the
start of `.text`, GNU as places the `aligned(2)` function at offset 2, the
`aligned(1)` function at an odd offset, and an unannotated function on a
16-byte boundary — the same addresses GCC's own `.s` produces. The gate grew
from 4 to 14 assertions with two controls, and it has two independent
mutation signatures: pre-fix parser fails exactly the six parameterized and
attribute rows, floor semantics fail exactly the two replace rows.

The honest lesson is in the commit message: the previous gate asserted only
parameterless functions, which is why it passed while the bug was live. A gate
whose fixture undersamples the feature is a claim of correctness without
coverage.

## C7 — A CI flake class, measured, that no amount of reading would have shown

Chasing a gate that failed roughly one run in three, on a compiler that produced
a **byte-identical** `.s` on 400 consecutive compiles of that gate's own fixture
(and 10 more under a loaded machine, and with no wall-clock input to any
decision except `[TIME]` reporting):

```
iter 50:  pipefail caught statuses: echo=141 grep=0
```

`grep -q` exits on the first match, closing the pipe while `echo` is still
writing; the writer takes SIGPIPE (141); `set -o pipefail` — correctly — reports
the pipeline as failed, so the gate printed *"pattern not in <function>"* about a
comparison that **succeeded**. Every failure named a different assertion
(loop back edge, pointer-param load, DCE-surviving load), which is the signature
of a harness race rather than a codegen bug.

Rates measured here, 20 000 iterations each:

| idiom | false failures |
|---|---|
| `echo "$b" \| grep -Eq P` | 27 / 20000 |
| `printf '%s\n' "$b" \| grep -Eq P` | 153 / 20000 |
| `grep -Eq P <<<"$b"` | 0 / 20000 |
| `[[ $b =~ P ]]` | 0 / 20000 |

Note `printf` is *worse* than `echo`: both are builtins flushed at exit, so the
race depends on scheduling. Fixed in the scripts CI runs — 47 pipelines whose
consumer was `head` → `sed -n '1,Np'`, 29 whose consumer was `grep -q` →
`grep -c … >/dev/null` or a here-string (76 sites in total), each preserving
both output and exit status while reading to end of input — and made unrepresentable by
`scripts/check_pipefail_sigpipe.py` (13-case adversarial self-test; strict over
the scripts CI executes; `--census` prints the 82 remaining sites in
developer-facing helper scripts, as a burn-down list rather than a claim).
End to end: the previously flaky gate is **0 failures in 30 consecutive runs**.

`|| true` was deliberately not used as the fix: it discards a real producer
failure (a compiler crash) along with the SIGPIPE.

## C8 — Where this leaves the review's score

Correctness 7/10, engineering 8/10, readability 5/10, overall 6.5/10 — on the
evidence it had. The two items I would score differently, with reasons: the
volatile bitfield was a live **wrong-answer** bug in two dimensions the review
did not reach (full-width stores, all loads), and the alignment feature it did
not examine was wrong for *every parameterized function* in both channels.

What the review got right and I am keeping: the lint ratchet is the best change
in the PR (it caught the bug the PR then hid); the header-only scan is correct
and honestly unbenchmarked; refusing instead of asserting on block 0 is right;
the checker self-test discipline is right; wiring the unrun gates into CI with a
parity check is right. Item 5's preference is a real one and this round acted on
it. Item 6 remains a convention dispute, recorded rather than hidden.

---

# Addendum D — rebased onto main `ec08e6a6`, which now contains the competing PR

Main moved under this branch by two commits: `03b75a34` *"x86: fold redundant
cmov-chain compares; harden volatile, preheader and CI gates"*, merged as **#695**
— that is the competing PR #693 (head `f1780b27`) landing upstream. My branch
and it had independently solved the same problems in 22 files, so this was a
real rebase, not a fast-forward: **12 conflict hunks in 12 files**, resolved one
at a time, each on evidence rather than by picking a side.

## D1 — What was taken from upstream, and why

| File | Resolution | Reason (the deciding measurement or fact) |
|---|---|---|
| `src/passes/vectorize.rs` | **theirs**, + my AUD-3 proof grafted | Upstream's `emit_invariant_vector_bound` already contains the same measured fix mine did (hoist the `UDiv`/`LShr` bound out of the header, scale by byte stride). Two equivalent refactors of one call site; upstream's returns `(Operand, usize)` with the change count computed inside, so it cannot drift from the emission. Mine returned a `bool` flag the caller re-derived the count from. Their interface is better; their file stays, with my five-step proof that the preheader filter is a *redundant* fail-closed guard kept verbatim. |
| `tests/linker/run_linker_tests.py` | **theirs** | Adds the `incapable` class — an oracle that rejects the *fixture* (a script-syntax feature it lacks) has no opinion on the relocation, and must not be counted in the quorum, while `inapplicable` must. My version could not distinguish them. Their `incapable` handling is the more correct model of "missing feature ≠ missing opinion". |
| `tests/linker/test_reloc_oracle_verdict.py` | **theirs** | Covers the empty-oracle-set case ("must FAIL, never PASS vacuously") with three cases; my version's `ZERO = []` constant was **never referenced** — measured: `grep -n ZERO` in my file returned the definition only. Dead constant, and the coverage it was for already exists upstream. |
| `src/passes/loop_preheader.rs` | **mine**, + upstream's honesty note | Both made the body scan header-only. Upstream keeps a *runtime* dominance walk (`∀b ∈ body. dominates(header, b)`) whose answer the natural-loop definition fixes as a constant — O(|body|·depth) per call to re-derive a theorem. Mine asserts the theorem's contrapositive in `debug_assert!` and pays nothing in release, keeping the bounds guard. Same observable behaviour, strictly less work, and the assertion is the stronger instrument: it fails the *debug* build if the must-execute rule is ever weakened. Upstream's "fires on 0 of the 51 benchmarks — a latent scalability fix, not a corpus win" note is kept: it is the honest framing. `struct Census` / `report_census` (upstream's `CCC_DEBUG_LOOP_PREHEADER` instrument) is preserved — it merged cleanly and is a different facility. |
| `src/passes/licm.rs` | **mine** | Upstream's changes to this file are comment-only (measured: 0 non-comment changed lines vs the merge base). My comment names the three gates that fail when the `!*volatile` arm is deleted and records the regression history; upstream's states the invariant without the gate list. Mine is a superset, and only comments were at stake. |
| `scripts/check_volatile_destructuring.py` | **mine** | Mine is the superset: upstream's file is 674 lines with one rule; mine is 844 with **Rule 2** (a volatility flag passed *in as a parameter*, never read in the body) plus `_match_paren`/`_split_top_level`/`_volatile_params` and a 21-case self-test that runs green. Every function upstream added exists in mine. |
| `scripts/godbolt_cache.py`, `tools/oracle/godbolt_oracle.py` | **mine** | Both sides independently built cache-staleness. Upstream keys a `ce_version` probe into every record on every run and evicts automatically; mine records `ce_semver` + `cached_at`, probes **only** under `--revalidate` (one `/api/compilers` request per run), and handles the case upstream cannot: a *moving* channel such as `cicxlatest`, where CE reports `(latest)` and there is no version to compare, so the record is judged by `--max-age-days` instead. Decider: the tree's own `tools/oracle/godbolt_oracle_selftest.py` is written against my symbols (`ce_semver`, `cached_at`, `--revalidate`, `--max-age-days`, `iter_records`) and upstream's `ce_version`/`_VERSION_PROBE`/`_STALE_EVICTED` are referenced nowhere outside my own docs. Adopted from upstream: the probe's *leash* — a dedicated short timeout (20 s, 2 attempts) and the rule that a failed probe is "unknown", never a crash. |
| `.github/workflows/ci.yml`, `scripts/ci_local.sh` | **union, by construction** | Took upstream's files (they carry the `#695` step set) and added only the invocations upstream lacks, computed as a set difference rather than merged by hand: `check_function_alignment_definition.sh`, `check_multi_entry_loop.sh`, `check_volatile_bitfield_split.sh`, and both `check_pipefail_sigpipe.py` lines. Verified afterwards: parity gate PASS at **111 commands**, no gate invoked twice. |
| `engineering/journal/2026-09-W3.md` | **union** | Upstream appended its PR #681 audit-response entry at the same anchor; both are kept. Fixed in passing: one of my sentences began with `#686's …`, which Markdown renders as a heading. |
| `tests/regression/check_volatile_spin_loop.sh` | **theirs (mode only)** | Contents byte-identical (92 lines each); only the mode differed — upstream has `100755`, mine `100644`. Took upstream's. |

## D2 — What the rebase cost: 28 dead-state sites upstream still carries

The fast build failed on the rebased tree with **28 `-D warnings` errors in
`src/passes/vectorize.rs`** — every one of them dead state that this branch had
already cleaned in its lint-ratchet commits and that upstream's lineage kept:

* **Duplicated declarations** at the head of both FMA transforms — measured at
  the merge base: `let debug = …; let mut changes = 0;` twice in a row, the
  second shadowing the first (2 sites).
* **`primary_loads`**: initialised to `Vec::new()` and then unconditionally
  re-initialised in every match arm, so the initialiser's value is never read.
  Fixed as a *deferred initialisation* (`let primary_loads: Vec<Value>;`), not by
  deleting the variable — it **is** read later (`.clone()` inside the
  multi-accumulator legality block). My first attempt deleted it and the
  compiler's `E0425` caught the mistake within a minute; that is the argument for
  this ratchet in one episode.
* **`vec_sum_value`**: an unused *parameter* of the remainder-loop builder. The
  parameter was genuinely dead inside the callee, but deleting it turned the
  callers' stores into dead ones — three further sites, which is exactly the
  cascade the item-3 review finding was about. Parameter and all three stores
  removed.
* Plus: a dead `fresh` closure, a dead `found_any_mul` flag (declared and set,
  never read), `original_block_count` and `rem_iv_unused` computed and dropped,
  a dead `iv_width_const` (whose removal orphaned the `vec_width` parameter of
  `build_map_remainder_loop`, and that one call-site argument with it), four
  closures declared `mut` without mutating, two unused pattern bindings `dest`
  in `Instruction::Cmp` patterns and one `ty`, an unused `Some(p_back)`
  binding, and three unused parameters (`cfg`, `iv_derived`, and two SSE2
  intrinsics) now spelled `_`-prefixed to keep their interfaces readable.

Cost of leaving them: none to codegen — they are all dead — but a build that
fails on any new warning is the instrument that finds the *next* one. This is the
first time this branch's ratchet has been exercised against code it did not
write, and it found 28 sites in a single file.

## D3 — Validation on the rebased tree

| Check | Result |
|---|---|
| `build_lccc_fast.sh` (fail-on-warning) | exit 0, 3 m 52 s, 0 warnings |
| Nine affected gates (`alignment`, `bitfield`, `multi-entry`, `spin-loop`, `licm`, `pointer-subscript`, `access-semantics`, `loop-preheader`, `redundant-flags`) | 9/9 PASS |
| `check_ci_gate_parity.py` | PASS, 111 commands |
| `check_no_conflict_markers.py` / `check_script_imports.py` / `check_ci_workflow_shell.py` | PASS / PASS / PASS |
| `check_pipefail_sigpipe.py` (+ self-test 13/13) | PASS |
| `check_volatile_destructuring.py` (+ self-test 21 cases) | PASS, 497 files |
| `cargo fmt --all --check` | PASS |
| `cargo clippy --all-targets -- -D warnings` | exit 0, 0 warnings |
| `ci_local.sh --fast` | **124 passed, 0 failed, 5 skipped — ALL GATES GREEN.** The runner withheld its pass stamp because a documentation file was appended to the
worktree while it ran (its own digest check caught it); the code under test was frozen, and a stamped re-run on the committed tree follows. |
