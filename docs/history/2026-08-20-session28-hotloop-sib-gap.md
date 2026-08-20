# 2026-08-20 (session 28) — RA hot-loop homes, x86-64 SIB, and the slot-traffic gap analysis

**Base:** `origin/main` @ `ef6511eb` (PR #148 = session-27 SIB i686 merged).
**Commits:** `7d2ca463` (hot-loop homes) + `5f821254` (x86-64 SIB).
**Deliverable:** `/home/user/ms178-1.patch` (auto-saved), artifact S28, git bundle.

## Item 1 — RA hot-loop home promotion (−167 B boot corpus)

**Root cause found by instrumented census (LCCC_DBG_RA):** in
`__cmdline_find_option` the loop-carried state values (uses=71/81
loop-weighted) were ALL slotted while span-1 temps took registers. Phase 1
allocated callee-saved homes only to call-spanning values; boot-code
functions have no calls, so 4 of 6 registers sat idle against 12
simultaneously-live values. Phase 2 (edx/ecx) excludes any interval
overlapping a scratch-hazard point — with hazards scattered through every
loop body, ALL long-lived values are excluded; only gap-fitting short
intervals get edx/ecx.

**Fix:** `hot_loop_home` candidates join Phase 1: loop-depth ≥ 1 at their
start, use pressure ≥ 12 (coalesce-group TOTAL, since phi-web leaders show
only their own incoming-edge count), span ≥ 10. Threshold sweep:
uc≥12/span≥10 = **31 105** vs uc≥20/span≥16 = 31 176, uc≥10/span≥8 = 31 366,
baseline 31 272. Gate: `CCC_NO_HOT_LOOP=1`.

Per-file: printf −88, video −122, video-bios −36, cmdline −25, edd −16;
string.c +91 (simple_strtoull frame +36 B: promotions evict other values —
a pressure trade-off, documented, net strongly positive).

## Item 2 — x86-64 SIB indexed addressing (session-27 item 3, DONE)

Mirrors the i686 session-27 machinery on x86-64:
- `emit_{load,store}_indexed` + `emit_{load,store}_indexed_sym`
  (`off(%base,%idx,scale)`, `sym(,%idx,scale)`, non-PIC only); FP loads/
  stores stay in the SSE domain (`movsd off(%base,%idx,scale),%xmm`);
  immediate-direct stores single-instruction.
- RA wiring upgraded to `collect_folded_gep_links_all` (base AND index
  liveness).
- Gate: `CCC_NO_X64_SIB=1`.

**Bugs found & fixed while landing it (all caught by the hard gates /
differentials, none shipped broken):**
1. **I64/U64 missing from the scalar type list** — the emitter refused I64
   loads while the dead-offset-producer skip had already removed the offset
   chain → rematerialisation read an uninitialized index register
   (`leaq (%r12,%r10),%rsi` with %r10 never set). Fixed by including
   I64/U64 (everyday scalars on x86-64); skip⇔emit agreement restored.
2. **`movl …, %rax` acc-target** — 32-bit loads into the accumulator must
   target `%eax`; the fallback now picks the width-correct acc name
   (bitops_builtins assemble error).
3. Debug-gate naming trap found while triaging: the x86-64 peephole escape
   hatch is `LCCC_NO_PEEPHOLE`, NOT `CCC_NO_PEEPHOLE` (that one is the
   i686 gate). Documented here so nobody loses hours to it again.

**Validation:** sib64 differential 3/3 (O0/O2/Os) · bitops_builtins MATCH ·
unit 959/959 · correctness 50/50 · regression 352+known-6 · sqlite -O2 +
big harness bit-exact · zlib-ng non-compat -O2/-O3/-Os roundtrip.

## The remaining gap — analysis (session 29 agenda)

Boot corpus on base 73ea7910 (incl. PR #152 .code16 branch relaxation):
folds-off 31 606 → **all session-28 levers on: 30 194 (−1 412 B)**; gcc
14 717 (2.05×).  Precise gate math from the diagnostic link: .text+.text32
end at 0x7613; fixed overheads (header/pecompat/rodata/videocards/data/
signature/bss) ≈ 7 130 B ⇒ text budget ≈ 25 638 B ⇒ **remaining gate gap
≈ 3 476 B of text** (the earlier ~8 KB estimate predates the relaxations).
The dominant remaining cost:

1. **Register budget usage**: gcc keeps 6-7 loop values resident INCLUDING
   eax/edx/ecx (it co-designs instruction selection with register usage:
   `%dl` for the char, `testb %dl`, immediate shifts that don't touch %cl).
   lccc treats eax/edx/ecx as fixed scratch; the hazard-point model then
   bars long-lived values from them entirely. Unlocking them needs
   hazard-aware split allocation (spill/reload around hazard points) or
   register-aware instruction selection — both are session-29-scale work.
2. **SSA-web slot proliferation**: identical values stored to multiple
   slots (`movl %eax,152(%esp); movl %eax,144(%esp)`), double reloads of
   the same slot in one straight-line region. Levers: slot coalescing for
   provably-equal values, stronger slot-forwarding peepholes.
3. Per-file deep dives: printf/video/string remain the biggest absolute
   deltas; each deserves a targeted gcc-vs-lccc asm diff session.

## Procedure notes (discipline kept)

- Auto-save after EVERY validated fix: S27 (hot-loop) and S28 (SIB) each
  committed + `ms178-1.patch` regenerated + artifact + bundle immediately.
- Every suspected regression triaged with gates first
  (CCC_NO_HOT_LOOP / CCC_NO_X64_SIB / CCC_NO_GEP_FOLD / LCCC_DBG_RA /
  LCCC_DBG_IDX), then root-caused in the asm, then fixed — no guesswork
  shipped.
