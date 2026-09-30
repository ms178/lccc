# Adjudication of the Review-AI audit of PR #689

**Verdict: request changes accepted. Two of the three blockers are real, one is
a symptom of a worse bug, and there is a fourth the audit did not find which
produces wrong answers.** All are fixed. The audit was right to block, right
about both of its technical findings, and wrong to stop where it did.

One correction to the audit's own premises, which changes how its findings
should be read: it audited `e4dc5285`, whose base is `ac003e4` (PR #687), while
the tree this round was validated on is based on `ec08e6a6` and the current
`7b3958f6`. The fold's logic is the same in both. The audit's complaint that the
pass is gated by `CCC_PEEPHOLE_SKIP=load_alu_fuse` was already fixed before the
PR was opened — `zero_ext_cmp` has its own key — so that remark is stale rather
than wrong. Everything else in the audit applies verbatim to the code as
shipped.

---

## Finding 1 — mirrored arm changes the second load's address · **AGREE, but the fix is structural, not a guard**

The audit is correct that the shipped guard was one-directional. But the guard
was treating a symptom, and the deeper problem is that the mirrored arm should
not exist at all.

The mirror arm deletes the **first** load and keeps the second. There is no AT&T
spelling of that which preserves the source's subtraction:

```
cmpl %r8d, %r9d          computes  r9 - r8
  keep r9, fold r8:  cmpb %r9b, (%rcx)  computes  (%rcx) - r9  =  r8 - r9   ✗ INVERTED
  keep r8, fold r9:  cmpb %r8b, (%rbx)  computes  (%rbx) - r8  =  r9 - r8   ✓
```

The shipped code took the first spelling. **That is not an address hazard; it
is a wrong answer** — see finding 4. So adding the one-directional guard would
have silenced the symptom and left the miscompile.

The fix removes the case instead of policing it. The pass now deletes the
**second** load in both arms, always keeps the **first** load's register, and
uses the first load's memory nowhere:

```rust
if src_fam == fam_b && dst_fam == fam_a {
    (fam_a, mem_b, true)     // cmp mem_S, D    -> D - mem_S
} else if src_fam == fam_a && dst_fam == fam_b {
    (fam_a, mem_b, false)    // cmp S, mem_D    -> mem_D - S
} else { … }
```

Now the first load keeps both its address register and its value register, so
no address can change; and the only read that moves is the second one, which
moves *down* to the compare, so source order is preserved. **Both of the
audit's findings 1 and 2 are eliminated by construction rather than by a guard**,
which is strictly better: there is no remaining case to audit, and no way for a
future edit to reintroduce the hazard without also reintroducing the first-load
delete.

Consequence worth stating plainly: the audit's recommendation #1 ("reject the
mirrored case") is the right instinct, but the better form of it is *fold the
mirrored case correctly* rather than *refuse it* — the shape is real, it
appears in the corpus, and refusing it costs a fold that is provably safe.

## Finding 2 — reordering observable memory accesses · **AGREE, and it is now structurally impossible**

Correct, and the same fix closes it. The pass no longer deletes the first load,
so no read is ever moved up past another read. The `pin_volatile_stack_slots`
point is right that a stack-slot mechanism does not cover volatile globals or
pointer loads, and the audit is right that this pass lacks the metadata to prove
non-volatility in general — but the question no longer arises, because the pass
does not reorder accesses at all. It only slides one read *later*, within a
window that contains no other memory operation.

I would not want to over-claim here: "we never reorder" is a stronger property
than "we check for volatility", and it is now a property of the construction.
`the_deleted_load_is_always_the_second_one` pins it.

## Finding 3 — partial flag writers end the walk · **AGREE, this is the best finding in the audit**

`stc` writes CF and nothing else; `sahf` writes SF/ZF/AF/PF/CF and explicitly
leaves OF alone. Treating either as a total clobber ends the walk early and
hides whatever is behind it. The audit's example is exactly right and it
reproduces.

Fixed generally rather than locally, because the coarse model is wrong for any
future rewrite that preserves some flags. `flags_written_mask` returns the flags
a writer can clobber, narrow **only** where the SDM is explicit:

| writer | mask | basis |
|---|---|---|
| `stc` `clc` `cmc` | `CF` | SDM: define CF alone |
| `sahf` | `SF ZF PF AF CF` | SDM: "LAHF/SAHF … OF is not affected" |
| `cld` `std` | none | DF is not one of the six arithmetic flags |
| everything else | all six | fail-closed |

`walk_flag_consumers` takes a `preserved` mask: a writer that clobbers nothing
outside `preserved` is stepped over rather than treated as terminal. The four
existing call sites pass `0`, which reproduces the old behaviour exactly, so no
other pass changes behaviour. A new entry point
`flag_consumers_are_zf_cf_only_preserving_zf_cf` passes `F_ZF | F_CF` and is
used by the fold, which proves ZF and CF identical.

The fix is strictly more precise, not merely more conservative: it also
*unblocks* folds the old model refused for no reason, since a `clc` in the
middle of a `jb` path no longer terminates the walk. That direction is tested
too (`a_cf_only_flag_setter_does_not_veto_a_genuine_zf_cf_block`), because a
fix that only ever refuses is not a fix.

## Finding 4 — the one the audit did not find: **the mirrored arm inverts CF**

This is a P0 that produces wrong answers, and it is why the other findings had
to be reframed. The audit reasoned about what the mirror arm does to an
*address*; the more basic problem is that its output is not the same comparison.

I proved it by assembling the pass's own output for
`movzbl (%rdi),%r8d ; movzbl (%rsi),%r9d ; cmpl %r8d,%r9d ; setb %al` and
running it: **input returns 1, the pass's output returns 0** for a=200, b=100.
For an unsigned consumer the subtraction direction is CF, so `jb` silently
becomes `ja` and `jbe` becomes `jae`. `setb` reads only CF, so the ZF/CF
consumer law admitted it, and the code's own comment claiming "AT&T preserves
the subtraction order" was simply wrong.

Every existing test passed with this live. The flag law was satisfied, the
instruction count went *down*, the code assembled and linked, and the answer was
inverted. That is the most important thing to record about this episode: **a
peephole gate built on instruction counts, mnemonics and even a C-level
differential against gcc can be unanimous and wrong**, because the defect lives
in operand position, and no amount of shape matching looks there.

## The fix that would have caught all of it, and did not exist

The audit noted the regression coverage "misses both first-load hazards". The
deeper gap is that *every* test here compares text. Text cannot see operand
position.

`the_emitted_assembly_computes_the_same_predicate_as_its_input` now runs the
pass, takes the assembly it actually produced, assembles **that**, links it
against a C driver, executes it over four discriminating argument pairs, and
compares against executing the assembly it replaced. Correct total is 2;
the buggy spelling returns 0.

**Non-vacuity is verified, not asserted:** with the bug deliberately
reintroduced the test reports `input returned 2, folded returned 0`.

It is worth recording how that test nearly shipped broken, because the failure
mode is the same one the audit identified in my previous gate. Its first version
silently `return`ed when the harness could not build, so it was **green with the
bug live** — a vacuous pass. It now propagates `Result` and `expect`s, so a
broken harness is a failure. A later iteration was still vacuous for a different
reason (a hand-written `main` in `.s` segfaulted against libc startup, and the
`None` path swallowed it). Both times the symptom was the same: a test that
cannot fail looks exactly like a test that passes. I only caught it by
deliberately reintroducing the bug and watching for red.

## Where I disagree with the audit

**Scores.** "Correctness 2/10" was fair for the code as submitted and I will
not argue it. But the audit's *overall* 3/10 weights implementation structure
and documentation against a defect that is one arm-selection expression, and
the documentation is the part that let a reviewer find the seam in the first
place. The reasoning that produced the comment claiming AT&T preserves the
subtraction order is the defect; it is a single reasoning error, not a
low-quality implementation.

**Recommendation #1's remedy.** Rejecting the mirrored case, as above, throws
away a safe fold to work around a bug. The shape is in the corpus; the fix is
to emit it correctly.

**Recommendation #4's framing.** "Add coverage for sequenced volatile
global/pointer reads" presumes the pass reorders reads. After the fix it does
not, so the coverage to add is a property test on delete position — which is
what I added — not a volatility model the pass has no use for.

**"Performance evidence 4/10, reported change too small to establish a
speedup."** Agreed, and stronger than the audit put it: measured against its own
switch the fold is worth **−0.96% (lz4) and −2.80% (zstd)**, both inside a
10–23% noise floor, one pointing the wrong way. The audit's "1.903 → 1.889" was
a single unpaired run. I have already relabelled it in the code, the commit
message and the follow-up document as codegen fidelity, not speed. The audit is
right, and it should be said more plainly than "too small": **it is not a
speedup at all.**

## The meta-lesson, which is the part I will actually carry

The Review AI found two of four defects statically, with no ability to run
anything. I found the remaining two by executing the pass's own output. The
binding constraint was never analysis — it was that **nothing in the feedback
loop could observe operand position**: the flag law was satisfied, the
instruction count improved, the C differential passed, the shape tests passed,
and the code was still wrong.

Every gate I have added this session has been about closing exactly that kind
of blind spot — the non-vacuity check on the zero-ext gate, the kill-switch
isolation that stopped one pass's win being credited to another, and now an
executed-output test. They are the same lesson three times: *a check that has
never been observed to fail is a check of unknown value*, and a check that only
inspects a representation will agree with any bug that preserves it.
