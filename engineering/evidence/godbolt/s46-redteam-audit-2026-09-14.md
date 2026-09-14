# Red-team audit — Session 46 (2026-09-14)

Auditor: the session agent, auditing its own S23/S24 work + the upstream
commits not yet audited (e1d3c154, 9d618278, 11d6bb21, 1caba5e7, f7c53eaf,
46e14738, bcfeeefe, 5e80f196).

## Self-audit of S23 (zext-backed Q→L self-reload nop)

**FOUND AND FIXED (P0-class, latent): the swap/CAS hole.** `xchgq %rax,
(%rcx)` (emitted by atomics.rs for `__atomic_exchange`) writes its FIRST
operand — the last-comma destination parse sees only the memory operand,
so `dest_write_width` reports None and the zext flag for the swapped
family stays at its pre-xchg value. A stale TRUE after `xchgq` would nop
a Q→L self-reload whose upper half now holds swapped-in memory bits:
a genuine miscompile window. The corpus never hits it today (no reg-reg
xchg, and the memory-form xchg is followed by full redefinition in
practice), but the window was real. Fix: xchg/cmpxchg (+ lock forms)
clear the whole zext state (they are rare atomics; no measurable loss).
Regression test added (tracked by pre-transform index — the plain
forward may legitimately rewrite the reload to `movl %eax, %eax`).

Same class: `vzeroall` zeroes ALL 128 bits of every XMM (unlike
`vzeroupper`, which preserves bits 127:0 — exactly what SD/SS mappings
track). The emitter never produces vzeroall today; it is now in both
the zext guard and the XMM-home disable scan, with tests pinning BOTH
directions (vzeroall disables; vzeroupper does NOT disable — the
forward must still fire across it, or the common pre-ret vzeroupper
would silently kill the FP forwarding).

## Self-audit of S24 (XMM store→load forwarding + imm materialization)

* **VEX discipline**: the Q-size forward on a VEX-encoded load now emits
  the 2-operand VEX form `vmovq %xmmS, %xmmD` (was: legacy `movq` —
  identical zero-upper semantics, but a legacy-SSE instruction in a VEX
  context risks the AVX-SSE transition penalty). SD/SS already used the
  3-operand VEX merge form. (L, VEX) still refuses: there is no VEX
  reg-reg vmovd.
* **Merge vs zero-upper matrix** re-verified line by line: movsd/movss
  loads MERGE (nop-safe on self-reload); movq/movd loads ZERO the upper
  (self-reload is NOT a nop); the rewrites preserve each line's own
  merge/zero discipline. `movd` self-reload survival is pinned by test.
* **Liveness-oracle staleness**: every rewrite's added register READS sit
  at lines ≤ the current scan position; every fusion gate queries facts
  at j > i, and liveness-after-j only depends on uses strictly after j —
  so plain rewrites cannot invalidate any later gate (the same argument
  the pre-existing plain GP forward has always relied on). Fusion
  rewrites refresh both oracles at the rewritten line.
* **i686**: XMM0-7 only (parse rejects ≥8), SSE2 forms only where the
  input already used them, FpLiveness's call-reads are i686-correct.
* **Future-proofing**: %xmm16+ (AVX-512) parse rejection makes unknown
  registers invalidate-only.
* Immediate materialization: memory does not change (the store stays),
  the dest register is still written at the same line, GP liveness
  facts are unchanged (loads/immediates read no GP registers).

## Upstream audit (commits since the S20 audit)

* e1d3c154 (W2 sign-extend fold, window-adjacent GEP folds, GSF pair
  fusion): AGREE. The window relaxation's guards (bounded 8-insn
  same-block window, no memory writes/phis/calls/atomics/fences/stack
  ops, no redefinition of the producer's dest or base/offset operands)
  correctly reduce to the soundness requirement "the GEP's inputs are
  unchanged and the load/store stays put"; the load is NOT moved. The
  width_ok restriction of the sign-extend fold to (I32,I64)/(I32,U64)
  with the movl-convention exclusion of (I32,U32) is correctly argued.
  The immediate-backed GSF mappings are the foundation my S24
  materialization extends — shared invalidation paths, shared fate.
* 9d618278 (RA-GLA-03 reach bands): target tuning constants with
  per-target gates; no soundness surface. AGREE.
* 11d6bb21 (block layout): placement-only (dual-layout selection,
  fall-through elision); the static-chain layout touches nested-function
  ABI state the S19 marker protocol already guards. AGREE.
* 1caba5e7 (GLA remat hardening + determinism): fail-closed direction.
  AGREE.
* f7c53eaf (Linker PIE/TLS): surface review only — evidence-backed by
  the 220-line i686 TLS IE-relax regression test it ships. Not fully
  deep-dived (out of session scope); flagged for the next audit pass.
* 46e14738 (GLA-02 default ON): the CCC_GLA_REMAT kill switch retained.
  AGREE.

## Corpus A/B discipline note (self-correction recorded)

The session's first csv_field_sum measurement (claimed −38) was a grep
artifact: LCCC emits conditional jumps at column 0, so `grep -cE
"^\s+[a-z]"` undercounts by the branch count. All A/B counts since use
the oracle's own _stats pipeline or include unindented branches. The
movslq_relay fold fires ZERO times on the benchmark corpus (the csv
pairs are legitimately blocked: %r15 is read before its redefinition);
it is retained as the general form of an rax-only fold (skip key
slq_relay) with a documented no-hit corpus A/B.

## Environment findings

* The i686 sysroot's usr/include/bits → x86_64-linux-gnu/bits symlink
  went dangling mid-session (partial sandbox wipe of the multiarch
  directory): 4 fast-battery gates failed with libc-header-start.h
  missing. Repaired by copying the host's x86_64-linux-gnu/bits. The
  sysroot must be self-contained; recorded here for the next rebuild.
