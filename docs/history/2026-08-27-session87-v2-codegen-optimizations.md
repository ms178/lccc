# Session 87 — v2 optimization round on rebased main: pass-gating bug, branchy range folds, i64 fast paths, hot-web steal

Date: 2026-08-27 (evening continuation)
Upstream base at session start: `3db0bbe2` (merge of PR #267 — the user
merged the session-86 codegen patch upstream; local tree was content-identical,
so the rebase was a pure fast-forward).
Session commits: `bc577150` (six optimizations), `6ef51495` (sext+zext fold)
Compiler build: `scripts/build_lccc_fast.sh` (fastbuild, Rust 1.98.0 -O1, -j2)
Kernel tree: linux-6.18.46 (unchanged from session 86; `.lccc-prepared`)

## Objective (user task)

Continue the codegen optimizations; find and fix inefficiencies, code bloat
and performance problems; deliver **v2 ms178-1.patch** with all new
optimizations, rebased on the new main; keep the deliverable autosaved,
no garbage/debug leftovers.

## Headline results

* **Setup corpus `.text` 24,716 → 24,240 (−476 bytes)** across seven
  validated changes. cargo 1205/0/6, regression suite 467/0/11 AB-diff 0,
  i64 runtime torture 5.4M checks green (all opt levels, fast paths on/off).
* **A latent pass-gating bug is fixed that had silently disabled the primary
  inliner for every -m16 -Os/-Oz compile** (substring matching in the
  disable list: "postinline" also disabled "inline"). This alone is −195 B
  on the corpus and re-opens inlining as a size lever for the boot code.
* **The 32 KiB gate is now fully understood as an alignment cliff** (below),
  not a smooth overflow: `_end` is dominated by `.pecompat`'s 4096-byte
  alignment (header.S `.p2align 12` — PE-header semantics). The gate passes
  iff `bstext+…+text32` ends ≤ 24,576; current end is 25,436 (−860 to go).
  Session 82's PASS and session 86's FAIL both sit on this cliff — this is
  the first time the mechanism is written down.

## The changes (all validated individually)

1. **`pass_disabled()` token-exact matching** (passes/mod.rs). The m16 size
   policy disables `postinline,ifconv,gaddrcse,licm`; every gate tested the
   comma list by SUBSTRING, so "postinline" also matched the "inline" gate —
   the primary size-aware inliner never ran on -m16 -Os/-Oz. Also "univsr"
   disabled "ivsr". Re-measured each disabled pass individually via the new
   `CCC_M16_KEEP` hook: all four still grow the corpus (ifconv +410, licm
   +367, gaddrcse +355, postinline +56) — the policy itself is correct and
   stays. Corpus −195.
2. **`range_check`: `resolve_bool_cmp()`** — phi/select arms carry the
   frontend's boolean-widening cast `(i32)(u8)cmp`; the Select fold looked
   up the arm value in `cmp_defs` and failed on the cast — **the Select-form
   range fold had never fired on standard lowering**. Now resolves through
   the widening cast. Verified on -m32 -O2 (is_my_digit emits GCC's
   `subl $48; cmpl $9; setbe` shape).
3. **`range_check`: `fold_phi_diamonds()`** — the branchy short-circuit form
   that survives when if_convert is off (the m16 profile): `Bcond: Cmp;
   CondBranch → Bcheck: Cmp+Cast; Branch → Bmerge: Phi([0,Bcond],[w,Bcheck])`
   folds to the unsigned-bias `sub+cmp` in Bcond; Bcheck dies. Fail-closed
   structural checks: exactly two phi arms, single-pred Bcheck, no other
   Bmerge preds, second cmp/cast not used outside the diamond. Corpus −175.
4. **i686 `emit_copy_value`: register→register copies** — one direct
   `movl %s,%d` instead of the eax relay (`movl %s,%eax; movl %eax,%d`);
   same-register (phi-coalesced) copies elide. Corpus −35.
5. **i686 `try_emit_i64_binop_fast/try_emit_i64_cmp_fast`** (i128_ops.rs) —
   64-bit And/Or/Xor/Add/Sub/Mul and all compares apply immediates and
   memory operands directly instead of the i128 stack staging
   (`pushl pair; op (%esp); addl $8`); identity folding for 0/−1 immediates;
   Eq/Ne collapse to the branchless xor-normalize-or-test. kstrtoull's
   overflow check was paying ~25 extra bytes per 64-bit ALU site.
   `CCC_NO_I64_FAST=1` disables. Corpus −21 net (string.c −138; small
   offsets elsewhere). Runtime torture: 20,000 random rounds × 27 checks ×
   all levels, against 32-bit-half reference implementations.
6. **regalloc Phase 2g: i686 hot-web steal** — phi-dest loop webs ranked by
   whole-web loop-weighted use count may displace colder holders (params)
   whose combined count is strictly lower; global cost accounting (the
   AArch64 loop-pin steal contract), plus the reg_hint guard. cmdline's
   `state` value now holds a register through the parse loop.
   `CCC_NO_HOT_WEB_STEAL` disables, `CCC_HOT_WEB_STEAL=k` tunes (default 3).
   Corpus −36.
7. **i686 peephole `fold_sext_zext_pairs`** — `movsbl SRC,%R32; movzbl
   %R8,%R32` → `movzbl SRC,%R32` (the `(u8)(i8)*p` shape; strncmp
   198 → 184). Corpus −14.

## The 32 KiB gate — the alignment cliff (important for planning)

`header.S` places `.pecompat` with `.p2align 12` (4096) — the EFI
mixed-mode PE header requirement. setup.ld links `.pecompat` right after
`.text32`, so the linker rounds the pre-pecompat content end up to the next
4096 boundary. Everything after (.rodata 1,394 + .videocards 84 + .data 140
+ .signature 4 + .bss 4,960 ≈ 6,582 B) then lands at that boundary and
`_end = boundary + ~6,608`.

* Content end ≤ 24,576 → `_end` ≈ 31,184 → **PASS** (session 82: content
  end 24,261, headroom 1,552 — exactly the recorded numbers).
* Content end 25,436 (now) → boundary 28,672 → `_end` 35,280 → **FAIL by
  2,512** — the same number session 86 measured, now decomposed:
  860 bytes of real code excess + 1,652 bytes of alignment waste inside the
  cliff window. GNU ld links the identical objects to the identical layout
  (verified) — this is kernel semantics, not an lccc-ld bug.

**Gate math: `.text` ≤ 23,380 passes.** Current `.text` = 24,240 (−860).

## GCC reference deltas after this session (per-file .text)

string +1,883 (kstrtoull 996 vs 428, simple_strtoull 597 vs 279,
__div_u64_rem 186 vs 0), printf +1,620 (vsprintf 1,889 vs 1,121, number
1,290 vs 600), video +1,407, cpucheck +976, cmdline +790, video-vga +708,
edd +680, video-vesa +613, video-mode +612, cpuflags +568, main +347.

## What did NOT work / was rejected

* Re-enabling ifconv/licm/gaddrcse/postinline for m16: individually
  measured, all four grow the corpus (numbers above). Policy stays.
* Inlining `__div_u64_rem` (186 B standalone, GCC folds it away): the
  inliner's thresholds reject it (34 insts/3 blocks); forcing it is not
  obviously profitable under the current slot-heavy wide-value codegen —
  revisit after the RA work below.
* Deeper RA work in `number()`'s digit loop: ~7 live intermediates + the
  div's eax/edx hazard shape keep spilling; needs the hazard model to
  allow values BORN at a hazard point to claim the hazard's output
  register. Deferred (high value, high risk).

## Next-session priorities

1. **P0 — close the 860-byte gap to the gate cliff** (`.text` ≤ 23,380):
   number()/vsprintf loop RA (see below), the branch-form (control-flow)
   range fold — worth ~2-4 B × ~40 sites, the store-reload/scratch-slot
   elimination in number() (71 candidate pairs in one function).
2. **P0 — hazard-model refinement**: values defined BY a div (rem in %edx)
   should be able to claim %edx as home (born-at-hazard). Unlocks ecx/edx
   for the number() loop body.
3. **P1 — wire build_kernel_boot.sh into run_regression_suite.sh** (still
   not done; the cliff analysis makes the gate metric precise:
   pre-pecompat end ≤ 24,576).
4. **P1 — value-width map for Copy/phi dests** (byte slots, movb immediates)
   — cmdline's char state machine and myisspace sites.
5. **P2 — kstrtoull/simple_strtoull 64-bit multiply-accumulate chains**
   (res*base+val via mull/add/adc pairs like GCC).

## Snapshot ledger

`S03-s87-v2-codegen` — base `3db0bbe2`, head `6ef51495`, deliverable
ms178-1.patch regenerated (v2). All mirrors refreshed (workspace + PolarFS).
