# PR #683 vs this branch — red-team adjudication

**Date:** 2026-09-29
**Subjects:** `pr683` = `582823799` (one commit, 53 files, +7885/−375) vs this
branch (`mywork`), both on the same base `02d4c906`.
**Method:** every claim below was produced by building both trees and running
the gates, not by reading diffs. Where a number appears it came from a command
in this session.

---

## 0. Verdict

**#683 is better than this branch was, on the one axis that matters most, and
worse on four others. Neither should land alone.**

The single most valuable thing in #683 is a test-design idea I did not have:
**negative controls**. Its `check_volatile_spin_loop.sh` asserts that six
volatile probes *keep* an access per iteration **and** that three non-volatile
controls *are* hoisted to zero. That makes "the gate passed because the
compiler hoisted nothing at all" impossible. My gates asserted only the first
half, so a compiler that stopped hoisting entirely would have gone green.

The single most valuable thing in this branch is a fix #683 does not have: the
root cause of the miscompile is a **crate-wide lint suppression**, and removing
it makes `rustc` reject the exact PR #681 bug at build time.

Both are now in this branch.

---

## 1. The measurement that decides it

The regression both PRs exist to prevent: delete the volatility guard from
LICM's load arm, so `while (!regs[4]) {}` becomes a load-once infinite loop.
I built each tree, deleted that guard, rebuilt, and ran **both** PRs' gates
against **both** binaries. Exit 1 = caught it, exit 0 = shipped a miscompile.

| gate | on **#683**'s mutated build | on **this branch**'s mutated build |
|---|---|---|
| `check_volatile_spin_loop.sh` (theirs) | **1 — caught** | **1 — caught** |
| `check_volatile_licm.sh` (theirs) | **1 — caught** | **1 — caught** |
| `check_volatile_pointer_subscript.sh` (#683's version) | **0 — MISSED** | — |
| `check_volatile_pointer_subscript.sh` (this branch's version) | — | **1 — caught** |
| `check_volatile_access_semantics.sh` | 0 | 0 |
| `scripts/check_volatile_destructuring.py` (theirs) | 0 | 0 |

Restored (correct) builds: all of the above exit 0. So every "caught" is a real
detection and every "0" on a correct build is not a false alarm.

Three things follow.

**1a. #683 beat me on coverage.** Two of its gates catch the regression; I had
none that did. That is not a close call.

**1b. #683's `check_volatile_pointer_subscript.sh` is a false comfort.** It is
wired into both `ci_local.sh` and `ci.yml`, presented as the volatile guard, and
it exits 0 with the miscompile live. The reason is precise and worth recording
because it is a trap worth naming: its `spin` assertion uses
`count_mem_movs spin`, which counts memory operands **in the whole function**,
so a load that has been hoisted *out of the loop but still inside `spin`* still
satisfies `spin_loads -lt 1`. This branch's version of the same gate added
`mem_accesses_in_loop_body`, which spans back-edge target → back-edge, and that
is the only difference that makes it detect the regression.

Notably, #683 *already owns the correct helper* — it wrote
`tests/regression/lib_loop_bounds.sh` for exactly this purpose and uses it in
its two newer gates — but did not retrofit the gate it inherited. I have now
done that.

**1c. `check_volatile_access_semantics.sh` cannot see a hoist, on either tree.**
It checks forwarding/CSE/DCE. It exits 0 on both mutated builds. #683 left it in
`scripts/ci_gate_allowlist.txt:85` with the note `exits 1 after partial pass —
needs triage` and never triaged it; this branch fixed the underlying
`mov`-only pattern defect (it rejected a correct fused
`addl counter(%rip), %esi`), so it is wired and passing.

**This overturns a claim of mine.** I had recorded that the Review AI's HIGH-1
("the subscript gate does not detect the regression") was false, on the grounds
that my gate caught it. Both statements were about different files. The claim
was substantively right about the gate it named, and my counter-evidence came
from a gate #683 does not ship.

---

## 2. The root cause, corrected

The Review AI's audit of #683 asserts that `rustc`'s `unused_variables` does not
fire on a binding introduced by refutable pattern destructuring, and that this
is why the deletion was invisible. **That is false, and I had independently
written the same false claim into this branch's documentation.**

Measured, with a genuinely refutable pattern (two-variant enum) and the lint
forced on:

```
warning: unused variable: `volatile`
  |
3 |     if let Instr::Load { ptr, volatile, .. } = i {
  |                               ^^^^^^^^ help: try ignoring the field: `volatile: _`
```

The operative cause is one line that was in `src/lib.rs`:

```rust
#![allow(dead_code, unused_variables, unused_mut, unused_assignments,
         unused_imports, unreachable_code)]
```

A crate-level `allow` silences the lint at its source. `-D warnings` in the
build script cannot override it — which is why the build finished clean — and,
verified separately, `cargo fix` reports "89 suggestions" while applying none of
them, because machine-applicable suggestions are filtered by the same level.

With the allow removed and the guard deleted, the project's own build script
fails:

```
$ ./scripts/build_lccc_fast.sh
error: unused variable: `volatile`
    --> src/passes/licm.rs:1256:30
     |
1256 |                     ptr, ty, volatile, ..
     |                              ^^^^^^^^ help: try ignoring the field: `volatile: _`
rc=101
```

**The miscompile can no longer ship.** That is a strictly stronger guarantee
than any gate, because it does not depend on anyone remembering to write a
fixture for the next safety flag.

Scope difference, and the reason this is not redundant with #683's ratchet: a
checker written for `volatile` covers `volatile`; the lint covers **every**
destructured safety flag in the crate — aliasing, atomicity, `noalias`,
signedness, anything added later — with span-accurate diagnostics, for free.

Its one blind spot, stated so it is not mistaken for complete coverage:
`if false && *volatile` still *reads* the binding, so the lint stays quiet while
the guard is dead. Deleting the term entirely — the actual PR #681 bug — is
caught. Lint, ratchet and gates are all kept because they fail on different
mistakes.

**Cost:** `cargo clippy` reported **99 `unused_variables` warnings across 49
files** once the allow was gone, and all of them are resolved. The fixes are of
four kinds: prefix a genuinely dead binding with `_` (including the
`field: _field` spelling a struct-shorthand pattern needs, since `_field` alone
is not a valid field name); spell an ignored pattern slot `_`; delete the
binding outright where it was write-only; and, in two cases, annotate the item
with `#[allow(unused_variables)]` plus a comment — the disabled
`try_emit_phase9_indexed_store`/`_load` pair in
`src/backend/x86/codegen/memory.rs`, whose bodies are kept as executable
documentation of an intended SIB-addressing decomposition and therefore
reference parameters that are genuinely never read. `unused_variables` is the
only lint removed from the allow. The crate now builds with **0 errors and 0
warnings** under the project's `-D warnings`, in both the lib and the
`lib test` targets.

---

## 3. #683's dead oracle version probe

`tools/oracle/godbolt_oracle.py::_compiler_version` reads
`c.get("version") or c.get("fullVersion")`. Measured against the live API
(`GET https://godbolt.org/api/compilers/c`):

```
across all 1049 C compilers: version non-null=0  fullVersion non-null=0  semver non-null=1019

  cg162        version=None fullVersion=None semver='16.2'
  cclang2310   version=None fullVersion=None semver='23.1.0'
  cicc2021100  version=None fullVersion=None semver='2021.10.0'
  cicxlatest   version=None fullVersion=None semver='(latest)'
```

and executed directly in #683's own tree:

```
_compiler_version('cg162')      -> None
_compiler_version('cclang2310') -> None
_compiler_version('cicxlatest') -> None
```

So the probe always returns `None`, and #683's cache-invalidation rule — which
compares a recorded version against a probed one — can never fire. It is a
correctly-written, correctly-documented, entirely dead feature, and it fails
*open*: a record produced by an older compiler is trusted forever.

This branch's equivalent, `live_semvers()`, reads `semver` from one request:

```
live_semvers() -> {'cclang2310': '23.1.0', 'cg162': '16.2',
                   'cicc2021100': '2021.10.0', 'cicxlatest': '(latest)'}
```

Note `(latest)` for `cicxlatest`: the ICX channel is a moving target and can
never be pinned, so provenance for it is a timestamp, not a version. That is a
property of the service, not a defect.

---

## 4. What was adopted from #683

| item | why |
|---|---|
| `tests/regression/lib_loop_bounds.sh` | One definition of "inside the loop body", shared by the volatile gates and the preheader gate. I had duplicated it. |
| `tests/regression/check_volatile_spin_loop.sh` + `check_volatile_licm.sh` + `volatile_licm.c` | The negative-control design (§1a). Both are mutation-verified on this tree: exit 1 mutated, exit 0 restored. |
| `scripts/check_volatile_destructuring.py` (+ `--self-test`) | A 674-LOC static ratchet with a fail-closed parser. It found a **real** unused `volatile` in this branch's `src/backend/generation.rs:5634` on its first run here. |
| `src/backend/generation.rs` explicit `volatile: _` discard | Fixes the finding above, with the reasoning written down. |
| `LateMinmaxOnlyScope` RAII guard | Replaces this branch's `set(true)/set(false)` pair. Structurally safe against an early return, `?` or unwind in the window. |
| `licm_debug` / `CCC_DEBUG_LICM` diagnostics | Six independent rejection causes were previously indistinguishable from the emitted asm. |

## 5. What was fixed on the way in

* **#683's minmax fixtures have no runtime coverage.** They live in
  `tests/regression/minmax_shapes/`, which satisfies the corpus glob by *not*
  matching it, but there are no include-wrappers, so nothing in the corpus ever
  executes them. This branch ships `minmax_reduction.c` and `minmax_refused.c`,
  which build, run and exit 0.
* **The preheader gate was merged, not chosen.** #683's version has the A/B
  kill switch (`CCC_DISABLE_PASSES=loop_preheader`) and a soundness contract
  (`guarded_sum` must never be hoisted) that mine lacked; mine had the exact
  insertion count, the measured 2→1 in-loop-memory-operand delta and the
  hoisted-load-after-NULL-guard ordering that theirs lacked. The merged
  `check_loop_preheader.sh` asserts **7 contracts over 2 fixtures** and passes.
* **The fixture basename collision was removed**
  (`loop_preheader/loop_preheader_insertion_shapes.c`).
* **`check_ci_gate_parity.py` now understands sourced libraries.** It treated
  `lib_loop_bounds.sh` as an orphan gate and demanded a workflow step that
  executes it directly — a no-op, since it is a library. The fix derives the set
  from `# shellcheck source=` directives rather than a `lib_` naming convention,
  so it keeps working for a helper that is not called `lib_*`.
  Result: `PASS (103 commands)`, up from 100.
* **The `loop_preheader.rs` module doc was wrong** (LOOP-PREHEADER-4). It
  motivated the pass with `for (i = 0; i < n; i++) t += c[0];` — the guard-at-top
  spelling, which is precisely the shape the pass *refuses*. It now leads with
  the do-while and keeps the counted `for` as an explicit counter-example, and
  both are pinned by contracts 1 and 2.

---

## 6. A regression I introduced, and what it cost

Recorded because the failure mode is more useful than the fix.

Applying the 99 lint fixes, I deleted nine write-only flags with a script that
matched **by line content across the whole file**. One of the strings,
`last = j;`, occurs twice in `narrow_copy_fold.rs`: once in
`fold_induction_copyback` (genuinely dead — that is the one `rustc` flagged) and
once in `fold_register_copies`, where `last` **is read** at
`lv.live_after(last, dfam)` and `lv.refresh_span(store, infos, i, last)`. The
script deleted both.

The compiler stayed silent, correctly: the surviving `last` in
`fold_register_copies` is still assigned and read, so no lint applies. The
behaviour changed anyway — with the update gone, `last` stays at its initial
`i`, so `live_after` was queried at the wrong index and
`refresh_span(i, last)` covered an empty range.

Measured effect: `sqlite_varint` 256 → 270 instructions (+5.5 %), `gzip_crc32`
65 → 69 (+6.2 %), 12 → 14 moves, and `check_indexed_fold_scratch_index` failing.
Caught by the codegen quality gate, which is exactly what a checked-in
instruction-count baseline is for.

Bisected by directory: `src/passes/` clean → `src/backend/` reproduces →
`x86/codegen/peephole/` reproduces → per-file → `narrow_copy_fold.rs` →
per-string. Fixed by deleting only lines 880 and 906. Post-fix: codegen gate
`all golden workloads within tolerance`, indexed-fold `OK`.

**The rule this leaves behind:** a lint diagnostic gives a *span*, not a name.
Any cleanup driven by one must be applied at the span, or at an explicit line
number inside a verified enclosing item — never by searching for the text.

---

## 7. Still open

* **#683's `semver` bug is fixed here but not upstreamed to them.** If #683
  lands in any form, `_compiler_version` must read `semver`.
* **DO-WHILE-BRANCH-1** — bottom-tested loops emit
  `setl / movzbl / testb / jne` where one `jCC` belongs; 6 of 8 measured shapes
  affected, 9 of the first 60 corpus `.c` files emit at least one `setCC`.
* **LOOP-PREHEADER-3** — the pass hoists the bound but not the accumulator
  (2→1 in-loop memory operands, not 1→0). Relaxing LICM's must-execute rule from
  "is the header" to "is dominated by the header" is sound; the gate's contract
  5 is the one-token change that would record it.
* **WR-COND-RETEST** — a second conditional store re-tests a live condition.
* **MINMAX-4** — re-opening the late rerun to non-min/max reductions. The
  steady-state gain is real; the prologue and accumulator traffic are what cost
  more than it saves.
