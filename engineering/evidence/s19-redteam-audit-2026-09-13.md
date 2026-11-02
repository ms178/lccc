# S19 Red-Team Audit — 2026-09-13

Auditor: the delivering agent itself (second-pass, adversarial mindset), per the
user's standing order: "carefully and thoroughly red-team audit your own work.
Do you agree with their choices? Why or why not?"

Scope: (A) the S38 milestone commit `faa34734` (my own in-flight work, saved and
validated this session), (B) the upstream arena chain `a0e03144..a361f26b`
(PRs #512–#518, merged by the user as "other work"), (C) the S19 fixes
`9b927b4d` + the marker-hygiene reset, (D) process and measurement gaps.

Method: full-diff reads, live miscompile reproduction (both P0s reproduced as
SIGSEGVs before fixing), fresh Compiler-Explorer rank survey
(`engineering/evidence/godbolt/s19-rank/`), knob sweeps with corpus-wide A/B.

---

## A. The S38 milestone (faa34734) — my own work

**Verdict: the performance work was sound, but two of its load-bearing safety
claims were FALSE, and the validation methodology had a hole exactly where the
claims were load-bearing.**

1. **The r10 chain-call marker protocol (fix #3) was incomplete in two ways,
   both found by live reproduction:**
   - *Emission hole*: `try_lower_call_typed` (the MachInst call fast path)
     emits calls without passing through `emit_call_instruction_impl`, which is
     the only publisher of `# LCCC_CHAIN_CALL`. A SetStaticChain-armed call on
     the typed path produced NO marker; the liveness oracles were then entitled
     to retire the chain staging; the nested callee dereferenced a garbage
     chain (`nested_chain.c`: SIGSEGV, parent build PASS, so the regression was
     introduced by S38).
   - *Consumption hole*: `family_private_to` (the peephole's second liveness
     oracle) never consulted the marker at all — with the marker present the
     relay pass still deleted the staging (unit test proved it). The S38 doc
     invariant "an invisible r10 read requires a textual r10 writer, which the
     scan treats as non-owned" is CIRCULAR for exactly the line being deleted:
     the transform's `owned` set contains the staging itself.
   - *Root architectural cause*: TWO parallel liveness oracles
     (FileLiveness and family_private_to) with DIFFERENT knowledge of the same
     ABI facts. Divergent oracles are miscompiles-in-waiting. The S38 work
     updated one oracle and documented the invariant in terms of the other.
   **Fixes**: typed path rejects armed chain calls (mature emitter owns the
   marker); family_private_to mirrors FileLiveness's exact marker/thunk
   protocol for family 10. Both oracles now agree by construction.

2. **The inline-retpoline claim was wrong**: "The inline-thunk and `call *%r10`
   forms mention %r10 in their own text (`mentioned` covers them)" — FALSE for
   the inline thunk: the mention lives in the call TARGET block
   (`.Lrpl_set_2: movq %r10, (%rsp)`), and FileLiveness gave `Call` lines
   fallthrough-only successor edges, making the thunk blocks UNREACHABLE in the
   CFG. The pre-S38 every-call-reads-%r10 model had been papering over the
   missing CFG edges; removing it exposed `retpoline_thunk_inline` to a
   SIGSEGV (parent PASS). **Fix**: intra-function call targets get the target
   edge (a `resolve()` hit proves locality — labels are collected from this
   function's range only) and drop the conservative CALLER_SAVED clobber for
   them; real callees keep the ABI model. The `.Lrpl_*` retpoline pair is the
   only intra-function call shape this backend emits (PLT/i128 calls resolve
   as foreign symbols).

3. **The corpus A/B (190 functions, 24 better / 1 worse) missed both P0s**
   because the A/B measures instruction counts on code shapes the corpus
   already contains: no corpus program combines a typed-path call with an
   armed chain, and the retpoline corpus test's crash was masked... actually
   it was NOT masked — it was a NEW upstream test the S38 session never ran
   (the corpus grew 738→766 with the user's i686/retpoline merges; S38's A/B
   used the old set). **Lesson recorded**: after ANY re-base onto new main,
   the full corpus (not the A/B subset) must run before the milestone is
   called validated. This is now the session protocol.

4. The other three S38 fixes (GVN GEP-CSE base normalization, dead in-place
   extension elimination, direct-source multiply) re-validated clean: corpus
   747/0, benchmark oracle 204/204 bit-identical, differential green.

**Do I agree with S38's choices?** The optimization DESIGN yes (all five fixes
target measured rank gaps and the A/B deltas were real); the MARKER PROTOCOL
design yes (one authority per ABI fact is the right shape); the EXECUTION no —
it changed the consumption model in one oracle while the other oracle and one
emission path still ran the old model. That inconsistency is precisely what
the two-oracle principle forbids, and it cost two P0s.

## B. Upstream arena chain a0e03144..a361f26b (the user's merged work)

- **f145c094 (flag-flow audit: OF as well as CF)** — AGREE. Reproductions
  before fixes, the f10 flag sweep instead of mnemonic tables, SF-only
  rewrites gated on `flags_reach_an_sf_consumer`, truncated i686 scan refused.
  The stated theme ("a structural assumption standing in for a verified
  effect") is exactly the S38 failure class; this commit models the right
  discipline.
- **5e80f196 (i686 CFG slot forwarder fixpoint)** — AGREE, strongly. Ten pins
  written against the unmodified code BEFORE fixing (six red); budget
  exhaustion abandons the function (fail-closed — a truncated iteration is
  not a fixpoint); analysis and application share ONE transfer function
  ("two copies of a transfer this size drift, and a drift between what was
  proved and what was rewritten is a miscompile"). That last sentence is the
  same principle I enforced for the two-oracle fix. Constant call arguments
  straight to stack slots at the lowering site (not a peephole, which would
  have to prove the materializing register dead): correct call.
- **7a8803a1 (RORX, default-on loop idiom, no-remainder vectorization)** —
  MIXED. The CONTENT is well-guarded (const_trip_covers_exactly with i32
  overflow guard, zero-start requirement for reductions, CCC_NO_RORX kill
  switch, memmove-vs-memcpy aliasing split, bails on trapping div/rem), and it
  ships 10 self-checking regression tests + ~54 unit tests. **DISAGREE with
  the process**: "No local build was run for this sync" — default-ON behavior
  changes landing on remote-CI-only verification is a risk concentration the
  repo's own history (gzip 1.14 miscompiles from default-on peepholes) argues
  against. Debt status: PAID — this session ran the full local battery on the
  result (fast 31/31, corpus 747/0 including the new tests, benchmark oracle
  204/204, cargo-test 2705/0, clippy, rustfmt).
- **d4a2d598 (tight loops sized by real encoded bytes; GLA phase 1 split,
  fail-closed)** — AGREE. Sizing loops by real encoded bytes is the honest
  metric; "split and fail-closed" is the right GLA shape (no silent
  approximation of a missing phase).
- **bcfeeefe / 5e3e16b3 (i686 store->load forwarding, x87 GP-pair staging)** —
  AGREE; the narrow-store miscompile fix carries its own repro class, and
  retiring dead store forwarding removes the exact hazard class that bit S38.

**Net on upstream: agree with 5 of 6 commits' engineering content; the one
disagreement (merge process for 7a8803a1) is now moot locally but should stay
on record: default-on transforms land only after a full LOCAL battery.**

## C. The S19 fixes themselves (9b927b4d + hygiene reset)

Self-audit residuals checked:
- Typed-path reject: `state.chain_call` armed ⟹ mature path. Argument staging
  between arming and the call cannot itself emit a call (stack/reg arg
  emission has no call sites); the IR splice guarantees SetStaticChain/Call
  adjacency; nested-call-in-argument evaluation is handled at IR lowering
  (backward scan). No residual found.
- family_private_to fam-10: mirrors FileLiveness exactly (marker within 2
  non-nop lines after a Call; every `call __x86_indirect_thunk_*` reads the
  r10-staged target). The external-thunk name carries no `%r10` operand so
  `reg_refs` misses it — that was the latent third miscompile, closed.
- CFG local-call edges: `resolve()` only knows this function's labels, so a
  hit proves locality; PLT/@PLT/foreign symbols never resolve. The
  infinite-loop spec block converges (self-edge, reads=∅). `fall` empty at
  function end is handled (edges=[target]).
- `dead_in_block_after` was already call-barrier-safe (is_barrier includes
  Call), and FileLiveness's marker consumption was already correct — the
  three deadness proofs are now individually sound for chain staging.
- Stale-test updates: each carries its soundness argument in-test (SysV:
  caller-saved values are undefined after calls; every call rewrites %rax).
  One of my own new tests was itself a bad contract (copy to a never-read
  %ebx IS dead) — deleted after analysis rather than defended.
- Marker hygiene: `reset_for_function` now clears `chain_call` and
  `call_is_variadic`. The stale-flag leak is conservative-only, but the
  protocol's contract is "marker ⟺ ABI-invisible read" and a false marker is
  a false premise for every future consumer. Closed by construction.

## D. Process / measurement gaps (open, honest)

- Fresh CE rank baseline saved (`s19-rank/after.json`): total gap 3156 vs
  best-of-oracles over 102 functions (78 behind / 4 tied / 20 ahead).
  Top gaps and their root causes, with today's data:
  - linux_rbtree main +492 / csv_field_sum +422: loop state (IV, cursor,
    seed) in slots while callee-saved regs hold len-1 temps. Root cause is
    STRUCTURAL register pressure (Select-of-GEP keeps both link arms live;
    per-field GEP materialization; cast chains), NOT the spill ranking:
    `CCC_EVICT_MODE=0` and `CCC_LEAF_STRICT_CALL_FREE=1` both lose
    corpus-wide (9/12 and 6-of-6 measured worse). The fix program is
    select-arm branching (GCC's if-conversion-reversal shape) + IR-level
    load-store pair coalescing into Memcpy (feeding the existing tuned
    inline-memcpy ladder) + the recurrence-aware spill cost. Not rushed here.
  - struct_copy +80: clang fully SLP-vectorizes the loop (76 insns, 0
    spills); LCCC 156. This is a vectorizer epic, not a peephole.
  - glibc_memcmp +32: 4×u64 slot→slot copy as 8 scalar movq through %rax
    incl. reloading 3 just-stored constants; arrays are address-taken (SROA
    blocked by the memcmp pointer args). Fix E lands as: immediate-store
    forwarding in the store_forwarding pass + consecutive-slot copy
    coalescing (needs machine-feature plumbing into the peephole config and
    an XMM-dead register proof — scoped, not rushed).
- The `git worktree` parent build at /tmp is disposable; the parent binary
  comparison served its purpose (regression attribution) and is not needed
  for reproduction (the unit tests pin both P0s).
- i686 corpus execution on this sandbox required rootless restoration
  (libc6-{i386,dev-i386} extracted into LCCC_SYSROOT; qemu-user extracted;
  `LCCC_I686_RUNNER` set). The harness's own runner-probe design made this a
  configuration fix, not a code change — good harness design, validated by
  747/0.

## Verdict

The deliverable (latest-main re-based patch) is validated end-to-end locally:
fast battery 31/31, cargo-test 2705/0, regression corpus 747/0 (122s, incl.
i686 under qemu and the retpoline/nested fixes), benchmark-output-oracle
204/204 (385s), clippy, rustfmt. Two P0 miscompiles introduced by the S38
milestone were found by this audit, root-caused, fixed, and pinned by tests;
the marker protocol is now closed by construction across BOTH oracles and
ALL THREE call emission paths. The remaining corpus gap to the best oracle is
structural and scoped with fresh data for the next session's Fix D/E program.
