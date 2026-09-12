# AUDIT-2026-09-12-S17-redteam.md

Red-team audit of the S17 session, and adjudication of the claims made by the
merged S15/S16 work (upstream as `dd10184e`). Base: `ms178/lccc` @ `a0e03144`.

The most important content here is §3: three claims this session made early and
then disproved with its own instrumentation, and one claim shipped in the
previous delivery that is flatly false.

---

## 1. Adjudication of the merged S15/S16 work

| claim (shipped) | verdict | evidence |
|---|---|---|
| Phase A+B reload-reuse precision is sound and wins on x86-64 | **Agree.** | Re-verified on the new base: 805-TU corpus byte-identical under the change set in this patch, gate green, 2484 lib tests pass. |
| `parse_frame_slot` must reject segment overrides explicitly rather than rely on a parse failure | **Agree, and it is now load-bearing.** | The explicit `mem.contains(':')` rejection is the only thing standing between `%fs:360(%rsp)` and a bogus slot identity. Keeping it. |
| Instruction count is not runtime ground truth | **Agree — re-vindicated.** | The i686 table in `FOLLOWUP-2026-09-12-i686-x87-gp-pair-staging.md` §2 contains a 0.008× "win" that is constant folding, not codegen. Static ratios and runtime ratios disagree in direction on 4 of 29 benchmarks. |
| **Evidence README §4: the i686 extension is a 2-line change to `parse_frame_slot`** | **DISAGREE — the claim is false.** | `lccc-i686` does not use `src/backend/x86/codegen/peephole/` at all; it uses a separate 13,491-line `src/backend/i686/codegen/peephole.rs`. The 2-line change was applied, built, and measured: 792 TUs, **byte-identical output**. The section is corrected in place (§4 of this patch). |
| The i686 extension was declined because the sandbox cannot execute i686 binaries | **DISAGREE — the premise was never tested.** | `sudo -n true` succeeds; `apt-get install libc6-dev-i386 gcc-multilib` takes 8 seconds; i386 ELF runs natively on the x86-64 kernel with no QEMU. Corpus coverage went 292 → **792** TUs. A refusal based on an unprobed assumption cost the project the whole i686 evidence base. |
| Slot-load dedup / reuse work is the main remaining peephole lever | **DISAGREE for i686.** | Measured opportunity: 19 redundant loads (3 deletable) in 271,585 instructions. The i686 peephole already participates `%esp` slots and fences every `%esp` mover. The real lever is §3 of the FOLLOWUP: x87 GP-pair staging, 1,317 sites, ≈5,268 instructions, 30% of `nbody`. |

---

## 2. New findings this session

### 2.1 `global_store_forwarding` (i686) was dead, buggy and superseded — deleted

It shipped disabled behind `#[expect(dead_code)]` with:

```
TODO: Disabled - causes 21 regressions in FP computation tests (matrix/FP
operations produce wrong numerical results). Needs investigation into FP
load/store forwarding patterns.
```

The investigation is done and **the diagnosis in that TODO is wrong**. It has
nothing to do with forwarding FP values: `parse_store_to_ebp` /
`parse_load_from_ebp` only ever accepted `movl`/`movw`/`movb`, so an x87 or SSE
value was never forwarded. The defect was the invalidation model:

```asm
movl %eax, -4(%ebp)     # tracked: slot -4 holds %eax
fstpl  -8(%ebp)         # writes 8 bytes, -8..-1; cleared ONLY the entry at -8
movl -4(%ebp), %ecx     # forwarded to `movl %eax, %ecx`  -- STALE
```

`%ecx` received a stale integer instead of the high half of the stored double:
exactly "matrix/FP operations produce wrong numerical results". Three further
defects: the write check was gated on `s.contains("(%ebp)")` so `%esp`-slot
writes were invisible; the map was indexed `-off - 1` under `off < 0` while
`%esp` offsets are biased positive by `ESP_SLOT_BIAS = 1<<24`, silently
rejecting all 100,442 `%esp` operands; and it never fenced `%esp` movement.

Deleted rather than fixed, because `forward_slot_loads` already implements the
same transformation, is enabled, and is sound in exactly those four places.
Keeping it invited a future "fix and enable" that would duplicate a sound pass
and reintroduce four miscompiles. The adjudication is recorded in-file where
the pass used to be.

> **ADJUDICATED IN S19 (2026-09-12, commit `8d4ad8fc`) — PARTLY REVERSED.**
>
> The four defects and their diagnosis stand; each is reproduced and closed by
> construction in the new design. Two conclusions do not stand.
>
> 1. **"Deleted rather than fixed, because `forward_slot_loads` already
>    implements the same transformation" — DISAGREE, and this was the costly
>    error.** `forward_slot_loads` is *windowed*: it proves "this register still
>    holds the stored value" across at most 48 lines of straight-line text. The
>    deleted pass was the only thing able to forward across a basic-block
>    boundary, a loop back edge, or a call. Deleting it did not remove a
>    duplicate, it removed a **capability**, and neither the windowed pass nor
>    the S18 x87 folds substitutes for it. S19 restores it as
>    `forward_slot_loads_cfg` — see
>    `FOLLOWUP-2026-09-12-i686-cfg-slot-forwarding-and-narrow-store-miscompile.md`:
>    −247 instructions / −405 slot references over 794 i686 TUs, 108 TUs
>    strictly smaller, 0 larger, and −260 / −415 with upstream's Location
>    Allocation Phase 1 (`CCC_RA_GLOBAL_LOCATION=1`) enabled in both arms, i.e.
>    complementary to it rather than obsoleted by it. The "invited a future
>    fix-and-enable" argument was right about the risk and wrong about the
>    premise: the surviving pass was neither equivalent nor sound.
> 2. **§2.2's "The live pass is sound — now proven, not assumed" — DISAGREE,
>    falsified by measurement.** Those five tests pin the *invalidation* model
>    (wide x87/SSE stores, `%esp` movement). Not one of them compares the
>    *width of a store* against the *width of its consumer*. A differential
>    fuzzer found a live silent miscompile in that same pass inside its first 96
>    cases: `movw %ax, 8(%esp) / addl 8(%esp), %esi` was rewritten to
>    `addl %eax, %esi`, substituting a 16-bit value for a 32-bit slot read
>    (S19 FOLLOWUP §4; wrong at `-O1`/`-Os`/`-O2`/`-O3`; i686 only; in C,
>    `u.h[0] = acc & 0xffff; acc += u.w;`). "Proven, not assumed" was itself an
>    assumption — five hand-written tests over the one mechanism already
>    suspected are not a proof over the pass. The general lesson, and the reason
>    `tools/slot_fwd_fuzz.py` now exists: a soundness claim about a peephole
>    needs a differential oracle over *generated* programs, because the defects
>    that survive an audit are the ones in the mechanisms nobody thought to
>    enumerate.

### 2.2 The live pass is sound — now proven, not assumed

When `global_store_forwarding` was disabled, its tests were deleted too
("store forwarding tests removed"), so the failure mode had **no coverage at
all**. Five tests restore it against the live pass:

| test | pins |
|---|---|
| `slot_forwarding_positive_control` | forwarding still fires (guards the four below against passing vacuously) |
| `wide_x87_store_kills_overlapping_integer_slot_forwarding` | the exact `fstpl`/`movl` mechanism from §2.1 |
| `wide_sse_store_kills_overlapping_integer_slot_forwarding` | the 16-byte window-kill constant against silent reduction |
| `esp_adjustment_fences_esp_slot_forwarding` | `subl $4, %esp` renumbers `%esp` slots |
| `push_pop_fence_esp_and_ebp_slots` | implicit SP movement fences; and pins the deliberate conservatism of also fencing `%ebp` |

All five pass. 875 i686 peephole tests, 2484 lib tests, `ci_local.sh --fast`
25/0/3 green, clippy clean, and the change is **byte-identical** on 791 i686 +
804 x86-64 corpus TUs (it deletes dead code and adds tests).

### 2.3 Residual risk assessed and closed

`forward_slot_loads` assumes 16 bytes for an unrecognised frame-offset write.
Stores wider than that exist (`fxsave` 512B, `fnsave` 108B, `fldenv` 28B) and
would defeat it. Measured: **zero occurrences in the 792-TU i686 corpus**, and
they can only arrive through inline asm, which `LineKind::InlineAsm` already
makes a barrier. No change made; the reasoning is recorded in the in-file
adjudication so the next reader does not have to redo it.

---

## 3. Self-corrections — claims this session made and then disproved

Listed because they are the audit's real content: each was caught by
instrumentation, not by review.

1. **"i686 has 6,034 mid-body push/pop clusters → the implicit-SP hazard is a
   guaranteed miscompile."** Wrong twice. The detector flagged an *epilogue*
   `pop` whenever more than two instructions followed the prologue pushes, so
   "mid-body" measured function length. And `reuse_redundant_loads` breaks on
   `LineKind::Push | LineKind::Pop` (dead_writes.rs:288), so it never scans
   across an SP move at all. Corrected whole-corpus answer: **0** reachable
   instances on x86-64 (805 TUs) and **0** on i686 (792 TUs).

2. **Three separate bugs in the hazard detector itself**, each of which
   produced a confident wrong number:
   * `SLOT = re.compile(r"^-?\d+\(%esp\)")` used with `re.search` — the `^`
     anchors mid-line operands out, reporting **0 frame-slot refs across
     271,585 instructions**. Implausible on its face; the real figure is
     102,701.
   * load/store direction ignored — `movq %r8, 48(%rsp)` is a *store*, matched
     as a load, reporting 12 affected TUs.
   * the predicate never actually required a push/pop **between** the pair, so
     it counted ordinary same-slot reloads (what the peephole is *supposed* to
     delete) and reported 6 TUs. The correct answer is 0.

   Lesson recorded: an instrument that reports a clean zero on a corpus with
   102,701 instances of the thing it counts is broken, not reassuring.

3. **"Extending `%esp`/`%ebp` to `parse_frame_slot` + width-agnostic push/pop
   classification is the i686 fix."** Both were implemented and built. The
   x86-64 corpus contains 0 `N(%esp)`, 0 `N(%ebp)`, 0 `pushl`/`popl`/`pushw`,
   and the i686 backend does not use that file. Measured effect: **zero, on
   both architectures.** Both reverted instead of shipped as defensive
   decoration — dead code that looks like a safety fix is worse than no code,
   because it gets trusted.

4. **A test expectation of mine was wrong, not the compiler.** I asserted that
   forwarding across `pushl` should still work for `%ebp` slots (sound in
   principle: the push area is below the frame). It failed. The live pass fences
   both bases unconditionally, and proving the narrower rule needs the frame
   layout, which a text peephole does not have — for 2,085 operands out of
   102,527. The test now pins the conservative behaviour and says why, rather
   than being weakened to pass.

---

7. **"The store side is 542 sites / ≈2,168 instructions."** Retracted in S18 by
   re-measurement: the strict, strictly-adjacent, `%esp`-slot, 4-apart
   destination pattern occurs **247** times in the baseline arm; the loose prefix
   the original figure appears to have counted occurs 408 times, on a binary that
   also predates the S17 `global_store_forwarding` deletion. What the store side
   actually realises is 72 sites and −288 instructions — 13% of the retracted
   claim. A census that is not tied to a foldable shape is not an opportunity
   estimate.

---

## 4. What did not ship, and why

* **The x87 GP-pair staging fold** — specified in S17, **landed in S18** and
  audited in §6 below. The S17 objection ("half-validated") was answered rather
  than waived: the validation plan in
  `FOLLOWUP-2026-09-12-i686-x87-gp-pair-staging.md` §4.4 was executed in full —
  792-TU corpus A/B, byte-identity scan, 3-arm execution differential, 250-program
  differential fuzz with 6 adversary generators, 12 in-tree unit tests, runtime
  A/B — and the load side ships behind a kill switch. The **store side (542
  sites, ≈2,168 instructions) is still not implemented**; see §6.3.
* **x87 stack allocation and invariant hoisting** (causes 2 and 3 in the
  FOLLOWUP). This is P0-A for i686 — a backend project, not a peephole.
* **The `fstp`/`fld` pair collapse.** 346 sites, none in any benchmark. Stopped
  per the project's own bar (a P0 needs a realistic benchmark win).

---

## 5. Environment hazard, and the fix

Between turns the sandbox wipes `~/.rustup`, `lccc/.git`, `target/`, script
exec bits and the swap file, while `/home/user` persists. Three separate
recoveries were needed this session, each costing minutes and one nearly
costing the git history.

`tools/recover_env.sh` is now idempotent and restores all of it from persisted
state (rustup installer, `artifacts/lccc.bundle`, apt, `fallocate`), including
the upstream fetch. It should be run at the start of any session that finds the
toolchain missing, before concluding anything about the workspace state.

The 8G swap file is re-established by it and by `scripts/build_lccc_fast.sh`;
both were verified active (`swapon --show` → `/swapfile 8G`) before the builds
in this session, and every build used the `fastbuild` profile.

---

## 6. S18 self-audit: the landed x87 GP-pair staging fold

Red-team of my own change, in the same terms as §1–§3: what I agree with, what
I disagree with, and what evidence would change my mind.

**Claim under audit.** A bounded peephole removes 948 instructions (−0.3491%)
from the 792-TU i686 corpus across 38 TUs with 0 regressions, improves `nbody`
runtime by 7.5–8.0%, and changes no observable behaviour anywhere.

### 6.1 Decisions I agree with, and why

1. **The CFG post-condition instead of a linear scan.** The linear version
   folded 0 of nbody's 37 sites because a hot loop rewrites the slot only in the
   *next* iteration, and a linear scan stops at the back edge. Following the CFG
   is what turns the pass from a curiosity into −128 instructions and −8%
   runtime on the worst outlier. This is the highest-value decision in the
   change, and it was driven by a measurement, not by taste.
2. **Kill switch (`CCC_NO_X87_PAIR_FOLD`).** It makes every number a same-binary
   A/B — the only variable is the pass — and it makes the pass deletable in one
   line once the root fix exists. It also produced the strongest single piece of
   evidence in §4.4: with the switch on, the corpus reproduces the pre-fold
   baseline *exactly* (271,585 / 102,701), which proves inertness rather than
   asserting it.
3. **Deleting the staging, not just rewriting the load.** The rewrite-only
   variant measured exactly **0** corpus change, because the slot offsets are
   reused for different values and `eliminate_never_read_stores` cannot clean up
   afterwards. The negative result is recorded (FOLLOWUP §5.7) instead of being
   argued away.
4. **One kill switch per half** (`CCC_NO_X87_PAIR_FOLD`,
   `CCC_NO_X87_PAIR_UNSTAGE`). This is what made the store side attributable at
   all: the three-arm corpus run shows the load-only arm reproducing the
   previously recorded numbers *exactly*, so the two passes are proven not to
   interfere instead of being assumed not to. It also means either half can be
   reverted alone if a defect ever surfaces in it.
5. **Adversaries before happy paths.** 6 of the 11 fuzz generators exist only to
   *fail*; the two `xchg` unit tests exist only to document a guard that already
   held; every negative unit test carries a positive control. That last habit is
   what found the SIB bug (§6.3.5).

### 6.2 Evidence quality, graded honestly

| claim | strength | why |
|---|---|---|
| no behaviour change | **strong** | off==on on 35 executable changed TUs, 340/340 fuzz programs (134 firing), 0 assembly failures, 2,508 unit tests, 25 CI gates |
| −1,236 instructions, 0 regressions | **strong** | same binary, two independent kill switches, three arms, authoritative counter, per-TU table; each arm reproduces the previously recorded numbers exactly |
| store side −288 instructions | **strong** statically, **none** at runtime | 15 TUs, 0 worse; no timed benchmark moves outside noise (see 6.3.2) |
| `nbody` −7.5…−8.0% | **strong** | two independent runs (N=9, N=7) on 7-second workloads; the effect is ~15× the run-to-run spread of a min-of-N measurement |
| `matmul` −1.5% | **none** | inside the noise floor; reported as uninformative, not as a win |
| `vector_remainder` | **none** | 1.3 ms total; §22/§23 says an arm that cannot resolve the effect is not evidence |
| closes the i686 gap | **false, and not claimed** | `nbody` remains 14.49× gcc, `matmul` 13.66× |

### 6.3 Where I disagree with my own work

1. **The win is small relative to the surface area, and I would have rejected
   this patch on the project's own bar if it stood alone.** −0.3491%
   corpus-wide, 18% of the opportunity I myself measured, one benchmark with a
   real runtime win. "No bare peepholes" is the rule; what makes this admissible
   is the causal chain to `memory.rs:470`, the documented removal path, and the
   fact that it *instruments* the root fix (it proves, per site, how much the
   location model would recover). I would not accept a second peephole in this
   family on the same reasoning.
2. **The store side is landed, and it is a static win only — I will not cite it
   as a performance result.** 247 strict candidate sites, 72 folded, −288
   instructions (−0.1064%), −216 slot refs, 15 TUs, 0 worse. Runtime: nothing
   measurable (`nbody` 0.925 combined vs 0.920/0.925 for the load side alone,
   `matmul` 0.988, `vector_remainder` 1.013). Under the project's own bar — "a
   P0 is done only if a realistic benchmark wins" — the store side does not
   qualify as a win and is landed as a 0-regression completion of the
   transformation family, explicitly labelled as such in FOLLOWUP §4.6. `matmul`
   still does not move, and the reason is now measured rather than guessed: it
   gained 8 instructions, not the 248 the S17 census implied.
   Two honest sub-findings:
   * **The refusal profile is not what I predicted.** I had written the
     frame-address taint off as the coarse condition costing coverage; measured
     over the 247 candidates it costs **zero** (0 refusals), the `%ebp`
     destination rule costs 31, and the entire residual — 144 of 216 — is the
     dead-store post-condition and register liveness. The post-condition gives up
     at `Call`/`InlineAsm`/`JmpIndirect`, and that is the largest remaining
     lever. Relaxing it looks sound under condition (6) (no register holds a
     frame address, so no argument can point into the frame, and a callee's own
     frame is below `%esp`), but "looks sound" is not the bar: it needs an
     adversary that gets a frame pointer to a callee by a route the taint
     fixpoint cannot see. Filed, not done.
   * **My own fuzzer could not reach the new pass.** The first 340-program run
     after the store side landed reported *exactly* the pre-change census (103
     fired), which is the only reason I noticed that none of the 11 generators
     produces the store-side shape. A new pass validated by a fuzzer that cannot
     generate its input is not validated. Six generators were added (three
     producing the shape, three adversaries); the census went to 134 with
     `g_store_indexed_stride` 20/20 and `g_store_struct_out` 20/20, all store
     adversaries 0/20, 0 hard failures. The lesson is procedural: after adding a
     transformation, check that the harness *fires* on it before reading its
     verdict.

3. **The frame-address taint refusal is coarser than it needs to be — and the
   measurement says it currently costs nothing.** It is function-wide: one
   `leal 4(%esp), %ecx` anywhere kills every site in the function. A per-site
   version — taint only registers live across the window, or discharge the
   specific slot bytes against the tainted value — would recover sites in
   functions that form a frame address for an unrelated varargs/alloca path. I
   did not do it because it needs a value-range argument I could not discharge
   cheaply. Measured cost on the store side: **0 of 247 candidates** refused by
   it (§4.6 of the FOLLOWUP), so refining it would buy nothing today; the
   criticism stands as a design coarseness, not as a live loss, and the lever
   that actually matters is the post-condition's call conservatism (6.3.2).
4. **Coverage is at the mercy of the register allocator, which I documented but
   did not fix.** Condition (6) means a site folds only when the GP pair happens
   to be dead afterwards: 237 of 775 load-side sites. Some of the 538 refusals
   are allocator artifacts rather than semantic necessities — which is precisely
   the argument for the location model, and against extending the peephole
   further.
5. **Two defects were invisible to every corpus-level gate.**
   * The SIB operand split (`movl (%ebx,%edi,8), %eax` parsed on the first
     comma) rejected *every* indexed staging site. 792 TUs measured identical
     either way, because the corpus sites that survive the other conditions are
     base-only. A unit test with a control found it in one run.
   * The unsuffixed `xchg` blind spot in `line_reg_use_def` (`uses=0x18,
     defs=0x000`). 25 CI gates, 2,497 unit tests, 250 fuzz programs and the
     whole corpus passed with it in place, because lccc's emitter always
     suffixes and the bare form can only arrive via inline asm.
   The common lesson, and the reason both are now pinned by tests: **a corpus
   cannot find defects the corpus does not contain.** Measurement finds
   regressions; only adversarial construction finds refusals and blind spots.
6. **My own harness had a defect that would have manufactured a regression.**
   An ad-hoc instruction counter reported 271,598 for the kill-switch arm where
   the authoritative `i686_corpus.py` regex (`^\s+[a-z][a-z0-9]*\b`) reports
   271,585 — 13 instructions across 4 inline-asm-macro TUs, because the ad-hoc
   version accepted uppercase-leading verbatim asm lines. The error was in the
   *baseline* arm, so a neutral change would have looked like a +13 regression.
   Caught only because I cross-checked the absolute totals against the recorded
   baseline instead of trusting the delta. Rule adopted: never hand-roll a
   counter that a harness already provides.

### 6.4 Residual risk, and falsifiers

* **Unsoundness in a shape no gate covers** is the live risk. Against it: the
  fold never rewrites a line it does not fully parse; every refusal condition
  has a test; condition (4) refuses an entire function on any stack-pointer
  taint, which is what caught `xchg %esp, %ebx` before the oracle was fixed;
  and inline asm was adversarially compiled and run in three shapes (slot read
  by C afterwards, pair live inside the asm, backward branch straddling the
  staging) with byte-identical output between arms and agreement with gcc.
* **Falsifier for landing it:** one reproducible `off != on`. After ~600
  executions across three independent harnesses there is none.
* **Falsifier for keeping it:** the x87 location model landing at
  `memory.rs:470`. At that point the two arms should converge and the fold
  becomes dead weight — measurable with one command, removable with one line.

### 6.5 Bottom line

Both halves landed: −1,236 instructions (−0.4551%) and −927 frame-slot
references over the 792-TU corpus, 309 sites folded, 0 regressions, 0
behaviour differences across ~700 executions of three independent harnesses.
`nbody` 16.07× → 14.49× gcc (−7.5…−8.0% runtime, reproduced); `matmul`
13.86× → 13.66× (noise). The structural
gap is untouched: lccc still generates no x87 memory operands from its location
model, hoists no invariants out of the FP inner loop, and schedules without
loop awareness. §2–§3 of the FOLLOWUP still describe the real defect. This is a
down payment with a receipt, not the fix.
