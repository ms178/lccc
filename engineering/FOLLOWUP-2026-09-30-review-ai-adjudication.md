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
