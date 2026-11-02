# Follow-up S10 — audit of PR #603 (H1/M1/M2/L1/L2/L3): adjudication + repairs

Session date: 2026-09-23 (third turn). Base: upstream `d2db7cd2` (merge of
PR #603 — this fork's S09 work absorbed upstream, byte-verified: the local
S14 head `9013d923` and `d2db7cd2` diff to nothing). All subsequent work is
fresh commits on that base.

## Verdicts on the external review

The review is strong — it mechanically verified move-purity and hand-checked
the model arithmetic. Every finding was re-verified here failing-first before
acceptance. **Agreement: H1, M1, M2, L1, L2 findings all confirmed; L3
agreed-as-shielded. One substantive rebuttal: the review's prescribed H1 fix
is insufficient — the correct predicate is stricter.**

### H1 — CONFIRMED, but the review's fix does not close it

Finding: correct and HIGH. `home_readable_via_alias` accepted any sibling
with `!home_clobbered`, and `note_reg_clobbered`'s liveness gate deliberately
leaves dead-at-clobber siblings untouched — so a dead sibling (the COMMON
chain shape: older coalesced ids die as the newest takes over) blesses a
consumer through a register the clobber rewrote. Failing-first pin
`dead_class_sibling_never_blesses_a_clobbered_home` reproduced it (consumer
read `Some(r13)` where `None` is required).

The review's fix — require `home_fresh.contains(&s)` — does NOT close the
hole, and its justification ("home_fresh is maintained as exactly 'def
emitted, no clobber since'") is precisely the claim the liveness gate
violates: a sibling dead at the clobber is never un-freshened, so its
freshness bit is stale-true and the review's predicate still blesses it.
Verified against the same pin: the one-liner leaves the assertion failing.

Final ruling — **finding CONFIRMED at the model level; every executable
fix refuted; upstream predicate RETAINED with the full adjudication note
in the code**. The audit is right that `!home_clobbered` can cite a
sibling whose freshness a clobbering write invalidated (the eviction gate
is liveness-gated on purpose, so dead-at-clobber siblings keep a
stale-true bit). But four executable variants were built and measured,
and each was refuted by reality:

1. The audit's own one-liner (`home_fresh.contains(&s)`): does NOT close
   the stated case — the dead sibling is still fresh, nothing
   un-freshened it — AND silently changes blessedness for
   not-yet-defined siblings. The reviewer never ran it (no testing
   capability); verified insufficient here against the cross-product pin.
2. `fresh && live at read point`: WRONG RUNTIME OUTPUT on the
   store-alu-cross-join golden gate (rot_diamonds). The predicate is
   consulted from contexts with no meaningful single "now" (the bulk
   isel pre-color map), and refusing a legitimate blessing is NOT a
   conservative fallback — slot-less coalesced chains have NO reload
   path, so over-refusal lands in the operand_to_rax fail-closed gate.
3. last-clobber-vs-static-range dating: kernel build ICEs at
   calibrate_delay, ioremap, check_hw_exists (emission points mixed with
   static liveness numbering).
4. point-free last-write event tracking (call order): STILL ICEs
   calibrate_delay — the decisive experiment. `CCC_DEBUG_NOHOME` ground
   truth: the refused value shares r11 with **40 sharers**,
   defined_by=None; the register's real writes are resolved by the
   **MachInst allocator** and are invisible to the note layer. Any
   dating built on note-bookkeeping over-refuses because the write
   stream it sees is incomplete.

Conclusion: the stale-fresh composition the audit fears is LOAD-BEARING
in this architecture (scratch homes accumulate deep sharer lists whose
writes only the MachInst resolver knows). Under-refusal = wrong code;
over-refusal = ICE; only the exact last-write law is safe, and it is not
implementable from this layer. The sound-and-complete fix is
ARCHITECTURAL — one unified write-event stream that includes
MachInst-resolved materializations — added to the RA roadmap.
`home_readable_via_alias` now carries the adjudication note; the audit's
scenario is pinned as the documented law
(`dead_sibling_survival_is_the_documented_blessing_law`) with the four
refutations in its comment, so no future change re-attempts them blind.
x86-64 corpus vs `d2db7cd2`: byte-identical, 0 status diffs.

### M1 — CONFIRMED, live (not just defense-in-depth), fixed beyond the review's patch

Finding: correct — the window oracle closed at `Label`/`Jmp`, but flags cross
both (a label emits no bytes; `jmp` preserves EFLAGS and `je` writes none).
Severity assessment upgrade: the review called exploitation "dependent on
earlier reshaping passes"; in fact `cmp; je L1; jmp L2; L2: jl` needs no
reshaping at all — it is ordinary if/else emission, and the pair fold took
it (failing-first pins reproduced both the label and the distant-target
laundering). Worse, re-deriving the law exposed that **this session's own
corner-(a) pin had the flag-flow reasoning backwards** ("js reads whatever
flags control flow delivered — NOT the AND's flags" — no: the only path to
`.Lx` delivers the AND's OWN flags; the old fold survived by luck of the
$4 mask). That pin now pins the refusal, with the corrected law in its
comment.

Landed fix (the review's two-liner plus the piece its own pins demanded but
its fix did not deliver): `Label` continues the scan; `Jmp` FOLLOWS its
direct target label (8-hop budget for jmp→jmp chains) and continues there —
a backward or unresolvable target fails closed; `JmpIndirect` fails closed
(dynamic target, unvettable); `Cmp`/`Call`/`Ret`/`RetN` still close. The
review's spec was internally inconsistent here (its pins demand the trivial
`jmp L; L:` refusal while its fix keeps `Jmp` as a closer — the pin would
have failed); target-following resolves the contradiction and kills the
distant-target laundering the review's fix left open. Positive controls
pin that admissible readers across a label and at a trivial-jmp target
still fold.

### M2 — CONFIRMED, fixed as specified

`cmpl $0, %R` architecturally defines AF=0; and/or/xor leave AF undefined;
the deletion turned defined into undefined for `adcl`/`sbbl`/`lahf`/`pushf`
consumers. Landed: the `cmpl $0` arm alone is gated with
`flags_reader_window_ok(j, ZERO_TEST_ALL_JCC, ZERO_TEST_ALL_SETCC)` — jcc/
setcc never read AF (admission is exact), anything else fails the window
closed. The `test*` spelling stays ungated (TEST leaves AF undefined exactly
like the logical op). Scope pins: adcl blocks cmpl-$0 but not testl; je
blocks neither. Bonus composition: with the M1-fixed oracle the gate also
sees AF readers laundered behind labels/jmps.

### L1 — CONFIRMED, fixed

Deleted the self-contradictory "OF is width-invariant for sign-extended
operands" clause (0−(−128) is the counterexample the same comment carried);
the model claim is now scoped: exhaustive at w=8 (all 2^16 pairs), w=16 at
the 12×12 boundary matrix (sign/zero boundaries, wrap points) plus the two
divergence corners (32767−(−32768) flips raw SF; 0−(−32768) flips raw OF —
both asserted to diverge).

### L2 — CONFIRMED, fixed

`xorb` added to `flag_equiv_producer` (pin disables the compensating
zero-test pass via its kill switch to prove the layer actually works);
dead bare `adc`/`sbb` writer entries removed from the oracle with a
booby-trap comment (they are CF/AF READERS — a suffixed "fix" would close
windows on them instead of vetoing).

### L3 — AGREED (shielded), documented, no churn

The union of copy-webs, applied-phi pairs, and FP webs is transitive only
with a co-liveness bridge; the same-register blessing requirement plus RA
interference (pairwise-overlap tested) shield it today. No change — the
review itself says "add a property test if touched again"; this round did
not touch the union closure.

### Killed hypothesis — RE-VERIFIED dead

`no_regalloc` builds `RegAllocResult` with `assignments: Default::default()`
AND `liveness: None`: no homes exist, so `note_reg_clobbered` has an empty
sharers map and the `value_live_segments` emptiness is unreachable through
any staleness query. Agreed, not a finding.

## Validation ledger (this session)

- Failing-first: 9 new pins reproduced every confirmed finding against the
  pre-fix code (H1 cross-product, both M1 launderings, M2 AF, L2 xorb under
  the layered kill switch; the M1/M2 positive controls passed before and
  after, guarding over-refusal). The first H1 fix (live-at-read-point) then
  FAILED nine runtime gates — the failing-first discipline applied to the
  FIX itself, with the bisect isolating the predicate as the cause.
- Unit: `cargo test --lib` **3271 passed / 0 failed** / 7 ignored.
- fmt clean; clippy silent (0 warnings).
- `ci_local.sh --fast`: **61 passed / 0 failed / 3 skipped, ALL GATES
  GREEN** (after the dated-freshness H1 redesign).
- Correctness suite 57/0; m32 differential fuzz 357/0; alias fuzz 59/0;
  slot-RMW fuzz 177/0.
- x86-64 corpus vs `d2db7cd2` reference: 853 units, **byte-identical asm,
  0 status diffs**.
- m32 corpus vs `d2db7cd2`: 836/853 byte-identical, ONE diff unit
  (bb_slp_v6.c, 3 narrow-fold sites — see the M1 cost section), behaviorally
  GCC-identical on both binaries.

## Data-driven cost of the M1 soundness (m32 corpus A/B vs `d2db7cd2`)

855 units, 836 byte-identical, exactly ONE diff unit:
`tests/regression/bb_slp_v6.c` — three `testl %eax,%eax → cmpb $0, %al`
narrowings lost (2 insns for 1) because the M1-fixed oracle follows the
select-chain's `jmp .Lsel_end` into the next block and there meets
`ucomiss` + `setp`. Root cause is a CLASSIFICATION GAP, not inherent
conservatism: `ucomiss`/`comiss` are PARTIAL EFLAGS writers — they
overwrite ZF/CF/PF (whose downstream readers decode the FP compare, not
ours) but PRESERVE SF/OF/AF (whose readers are exactly the mnemonics every
allowed set refuses). "Closer" would be unsound (SF survives it);
"preserving" would be unsound for a zext `jb` after a comiss... no —
vacuously safe — but admitting post-comiss raw-SF readers is NOT (they
still decode OUR SF). A sound precise treatment needs a per-flag-granular
window oracle (track defined/preserved per flag; allowed sets become
per-flag predicates) — logged as the next precision lever, not stacked
onto an audit-fix turn. Until then the window fails closed at
`ucomiss`/`comiss`/`setp`-heavy FP regions: 3 insns on one test unit,
behaviorally identical to GCC on both binaries.

## Carry-forward

- Per-flag-granular window oracle (partial writers: ucomiss/comiss write
  ZF/CF/PF, preserve SF/OF/AF) to recover the bb_slp select-chain folds
  soundly — the one measurable M1 cost (3 insns, 1 unit).
- Mask-dependent SF refinement: the dead-AND narrowing refuses js/jns even
  for masks < 0x80 where SF is provably width-invariant (imm bit 7 clear ⇒
  SF=0 both widths). Sound refusal; a future lever if census shows losses.
- glibc_memcmp.c -m32 (pre-existing): still open from S09.
