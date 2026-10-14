# CFG threading and asm-goto edge audit

Follow-up on `2bee471007394a81` (PR #818, `00394a81`). That PR merged the
module-wide label census and address-observable fold protection. This note
describes the Review AI audit response (PR #818 review, 8.9/10 score).
It is not a hosted result for the follow-up head, and not a speedup claim.

## Review AI audit response

The Review AI audit identified four findings. Three were fixed; one was
deferred as out-of-scope for this follow-up.

### F1: `delete_redundant_cond_jumps` missing address-observable guard (FIXED)

**Finding:** `delete_redundant_cond_jumps` (line 10787) deletes `jCC X` when
the next line is `X:`, without the `jump_carries_observable_address` guard
that Pattern 3 in `combined_local_pass` (line 2029) has for `Jmp`. Runs in
the same pass sequence as `fold_reg_copy_idioms` (line 13536), which can
remove the flags producer (`testl`/`cmp`) that normally separates an
address-observable label from the `jCC`.

**Fix:** Added the identity guard to the `LineKind::Label` arm only. The
`LineKind::Jmp` arm (`jCC X; jmp X`) is safe because the `jmp` still
occupies an address between any preceding observable label and `X:`. The
guard uses lazily-cached `identity_pins`, exactly like `combined_local_pass`.

**Verification:** A fixture with `static void *p = &&obs;` and a block
reduced to `jCC X` + `X:` keeps the `jCC`, and `&&obs != &&X` on i686 at
`-O0..-Os`.

### F2: Emitter text heuristic fragility (HARDENED)

**Finding:** `fallthrough_would_alias_observable_label` (state.rs:804) uses
a text heuristic that scans the buffer backwards. Three fragilities:
(a) alignment directives between an observable label and its elided
terminator return `false` → possible aliasing if alignment emits 0 bytes;
(b) a label emitted with a trailing comment would return `false` (silent
aliasing); (c) `starts_with(".loc")` also swallows `.local`.

**Hardening:** (1) Fail-closed for directives that *can* emit bytes
(`.p2align`, `.align`, `.balign`, `.space`) — treated as address-occupying.
(2) `.loc` tightened to `.loc ` / `.loc\t` (with space or tab) to exclude
`.local`. The Review AI verified that "no trigger is reachable today" —
all `.p2align`/`.align` emission sites precede a label, and labels are
emitted alone — so this is robustness hardening, not a bug fix.

**Not done:** Structural tracking (`emitted_since_label` flag) was
considered but rejected as too invasive: it requires touching every emit
site across all backends. The text heuristic is now fail-closed and
documented.

### F3: `test_inline_asm_region_is_never_rewritten` missing `#[test]` (FIXED)

**Finding:** The test function at line 18464 had no `#[test]` attribute and
never executed.

**Fix:** Added `#[test]`. The test now runs and passes.

### F4: `thread_trivial_blocks` O(candidates × M) complexity (DEFERRED)

**Finding:** `thread_trivial_blocks` (line 10663) calls `label_defined_once`
×2, `trampoline_pins`, and `retarget_proven_refs` per candidate. All scan
O(M). At ~1k candidates and ~50k lines: ~50M scans per invocation × 4
fixpoint iterations = ~200M scans. `combined_local_pass` shows the pattern:
lazily cached `identity_pins`.

**Decision:** Deferred. The Review AI rates this "Low, unmeasured" — no
compile-time regression was demonstrated. The fix requires significant
refactoring: precomputing label-definition counts, trampoline-pin sets, and
retarget counts once per invocation and passing them into
`thread_resolved_trampoline`. The risk of introducing a correctness bug
outweighs the unmeasured performance gain. This is a compile-time
optimization, not a correctness fix, and belongs in a dedicated performance
PR with before/after measurements.

### F5/F6/F7: Test coverage gaps (FIXED)

**Finding:** (F5) `.quad` dispatch run not tested as dispatch (only as
address pin); (F6) `.word`/`.short` label differences safe via fallback pin
but untested; (F7) harness's dangling-label check validates only the first
operand of `.long A - B`, never checking `B`.

**Fix:** Added two unit tests in `dispatch_and_pin_tests` module:
(1) `quad_dispatch_run_under_proven_base_is_rewritten` — verifies
`jmp *T(,%rax,8)` proves `T` as a base, and `.quad .L1 - T` entries under
`T` are classified as dispatch. (2) `short_label_difference_pins_both_labels`
— verifies `.word .L1 - .L2` without a proven base is not dispatch, so both
labels are pinned. Extended the harness's Python block to check **every**
`.L` token in a data payload, not just the first operand.

## What the review got right

The i686 text threader scanned only `fstart..fend`, and the first window
starts at line 0. Module data is emitted before function bodies
(`emit_data_sections`). A later function's static `&&label` (`.long .LBBn`
in `.data`) is therefore outside the window that considers deleting that
label. The same pass treated every in-window `.long`/`.quad` as a jump
table and rewrote it. Those are different uses: a proven switch entry is
dispatch, a static initializer is an address.

`jecxz` is classified as `CondJmp` because its second character is `e`.
The encoder and ELF writer have no long form (`E3 cb`, rel8). Retargeting
it before layout can assemble only by accident. `jcxz` and `loop` were
accidentally safe because they are not `CondJmp`; that is not a contract.
Final rel8 range is not knowable in this pass: later folds and relaxation
still change sizes. A text-distance guess would be a false proof. The
relay stays. x86-64 already refuses `jcxz`/`jecxz`/`jrcxz` and never
deletes labels.

## What the review overstated

Hosted CI on the merged PR head is no longer pending. A green result there
does not cover this follow-up. The suggested split is rejected: the
label census belongs in the pass this change already hardens.

No corpus-wide non-debug O2 speedup is claimed. Prior four-arm Callgrind
and Os text notes belong to the merged revision; they were not rerun here.
Ir is not cycles. This machine is not an i7-14700KF.

## What changed

Reference classification is module-wide and use-based:

- Relaxable `jmp`/`jcc` may skip a pure trampoline. The label address does
  not change.
- A short-only branch (`jecxz`, `jcxz`, `jrcxz`, `loop`, `loope`, `loopz`,
  `loopne`, `loopnz`) keeps the relay. Ordinary edges may still skip it.
- `.long`/`.quad` is rewritten only when it is a contiguous entry under a
  symbol the module uses as `jmp *TABLE(, %reg, scale)` or `TABLE@GOTOFF`,
  and the payload is `CASE` or `CASE - TABLE`. `static void *p = &&label`
  and `&&a - &&b` do not match. An address pin also blocks dispatch
  retargeting of that same label.
- The label is deleted only when every reference was retargeted and the
  name is defined once. Inline asm stays opaque (`%l[name]` can be
  address-taken).
- Jump-to-next folding still deletes a redundant `jmp` for dispatch-only
  labels. It does not delete the jump that is the first instruction of an
  address-observable label: that would make `&&mid == &&end`.
- The pin scan and the rewrite skip GAS quotes. `.string ".LBB1"` and a
  quoted comment are text, not a symbol use.
- Fall-through elision is not address-preserving. An empty block whose
  address is observable (`LabelAddr`, a static initializer, an asm-goto
  target) still emits the jump when that jump is the only thing separating
  it from the next label. A non-empty block still elides.

A sole-predecessor empty relay that already feeds a PHI is still not
bypassed. That relay is what keeps the arm alive; removing it miscompiled
a vector map kernel. Multi-predecessor relays remain threadable. Empty
asm-goto sources are not fused into their fallthrough.

## What was not copied from LLVM

LLVM's duplicating jump threader (`JumpThreading.cpp`, `ca1dbb8ee2aa`)
evaluates conditions on predecessor edges and copies blocks under a size
threshold. Its history includes quadratic IR growth when zero-cost
instructions bypass that threshold. LCCC already folds constant branches
and threads bool-phi diamonds without duplication. Cloning that pass
without LazyValueInfo, a dominator updater, and a measured cost model
would add compile time and code size without a demonstrated runtime win.
The empty-block threader stays duplication-free. That is a scope decision,
not a claim that no further threading is profitable.

## Oracle

The asm-goto fallthrough oracle is unchanged: `fd6c281bebb77835` at
`-m32`/`-m64`, `-O0` through `-Os`, matching GCC 16.2.0, Clang 23.1.0,
ICX 2024.0.0, and catalog ICC 2021.10.0 at `-O2`/`-Os` on the merged
revision. ICX's provider alias resolved to 2024.0.0, not a proof of
Intel's newest 2026 release. No new vendor timing was collected.

## What was run

`origin/main` was still `6733d68f` (PR #812). No hosted check covers this
follow-up. No O2 speedup was measured.

`tests/regression/check_i686_label_ref_safety.sh` linked and ran with the
fastbuild `-m32` compiler at `-O0`, `-O1`, `-O2`, and `-Os`. All four
returned 0. GCC 14.2.0 `-m32` on the same source returned 0 at the same
four levels. The corpus sidecar is `-m32 -O2`; without it the standalone
corpus compiles the file as x86-64 and the `#error` fires. At `-O0` the two `.long` entries are `.LBB10` and `.LBB11`,
and the compiler emits `.LBB10: jmp .LBB11`. Before that emitter fix the
labels were adjacent and `hop(-1)` returned 14. The backward `jecxz`
fixture enters past the far label; putting the far label at the function
entry returns before the branch, which is a test bug, not a relay bug.

The emitted `-O0` assembly assembles with `as --32`. Pointing `jecxz`,
`jcxz`, and `loop` at the far labels instead of the relays does not:
forward displacement 211 and backward `ffffff2c` are both too large for a
1-byte field. The i686 peephole module tests passed: 341, including the
quoted-text case.
