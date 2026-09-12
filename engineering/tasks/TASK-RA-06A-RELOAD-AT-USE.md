# TASK-RA-06A — reload-at-use splitting + arithmetic-chain copy webs

IDs: RA-06, RA-06a, RA-06B, PF-05 · Priority: **P0** · Base: origin/main @ f657de55
· The single largest measured codegen gap (3–5× spill traffic vs LLVM).

## Objective

Teach the linear scan to split live ranges at uses instead of demoting the
whole remaining lifetime: when a value crosses a high-pressure region
(calls first), insert a reload immediately before the use. In parallel,
extend copy webs through the arithmetic chain so ONE range carries a
recurrence (adler's `s1`/`s2` partial sums are separate scan values today;
promoting only the copy leaders measured +28 % runtime — the web must be
one range).

## Files

`src/backend/live_range.rs` (scan, `enable_splitting` stub gate),
`src/backend/regalloc.rs` (sweep-eviction traffic model — reuse it),
`src/backend/split_ranges.rs` (IR-level split foundation), emit-side
reload placement.

## Acceptance

- Adler-32 kernel ≤ 1.15× GCC; adler stack refs 78 → <30.
- gzip `longest_match` stack-mem must not rise (hard veto).
- xmltok/inflate TU stack-mem ratios drop below 4× (re-measure; RA-05a
  addressed the interference half already).
- `CCC_VERIFY_REGALLOC=1` clean over the whole correctness corpus.
- Kill switch: extend the `enable_splitting` gate (`CCC_NO_LIFETIME_SPLIT`
  if a new switch is needed) — bisectable, default-on after validation.

## RA-06B — SUPERSEDED: the demotion's root cause was elsewhere, and is fixed

The acceptance target originally written here assumed that merging the two-phase
temps lengthens a loop-carried range across the back edge, that the allocator
then loses it, and that a profitable back-edge split (RA-06 location pieces) was
therefore the prerequisite for making `CCC_PHI_ACYCLIC_ORDER` the default. **That
diagnosis was wrong and the prerequisite does not exist.** Do not implement a
back-edge split on the strength of the old text.

What was actually happening: `live_range::mark_loop_spanning` derives
`span_has_in_loop_use` web-wide by summing `uses_in_extents` over a leader and its
coalesce members — and `uses_in_extents` was populated by a pass over `ranges`. A
coalesced member is merged into its leader's interval and owns no `LiveRange`, so
it never entered that pass, every member lookup missed, and the sum that the code
documents as carrying "the web-wide in-loop-use flag" was a silent no-op that
degraded to *does the leader have an in-extent use*. For a phi web led by a cold
preheader definition — the shape every loop-carried recurrence has — that is
always false, so the in-loop-USELESS-span admission rule demoted the hottest
values in the loop. On `sha256_transform`: `leader=v166 members=[166,389]`
(`Load state[0]`) and `leader=v182 members=[182,392]` (`Load state[4]`), each
reloaded 7× per iteration — 14 of the round loop's 15 memory operations
concentrated on two slots. The two-phase temps were not performing a beneficial
back-edge split; they were *hiding* this bug by keeping those webs apart.

Landed as the web-wide in-loop-use supply in `mark_loop_spanning`, killing switch
`CCC_NO_WEB_INLOOP_USE` (`RaConfig::no_web_inloop_use`), gate
`tests/regression/check_ra_web_inloop_use.sh`. Worth **+3.63 % / +4.33 %** runtime
on `sha256_transform` in two amplified paired replicates (p=0.0000, median and min
agreeing in both), 210→198 function instructions, and the round loop's
most-reloaded slot 17→10 accesses. Span flags only: it deliberately does not touch
`LiveRange::uses` or `priority`, because inflating a coalesce web's priority in the
main scan waves is a measured negative (expat −30 %, adler32 −23 %, arith_loop
−12 %, sha256_transform −56 %).

It did **not** make the resolver default-on. Re-measured after the fix on base
`25ed36de`, the resolver on top of the supply is **−1.99 % / −0.71 %**: the penalty
shrank from ~5 % but did not vanish, and the pair (+1.88 % / +1.98 %) is worse than
the allocator fix alone. `CCC_PHI_ACYCLIC_ORDER` therefore stays opt-in. What is
left is a *concentration* problem, not a range-length one — the resolver cuts the
round loop's stack memops 58 → 25 but onto 9 distinct slots with one touched 10×,
while gcc runs the same loop in 47 instructions with **zero** stack references.

### Revised acceptance for RA-06 (location pieces)

- `sha256_transform` reaches gcc's shape: whole-function frame-relative stack
  references ≤ 8 (LCCC ships 62, base 62, gcc 8) and `mov` count ≤ 65 (LCCC 98,
  gcc 61). Runtime within a few percent of `gcc -O2`; today gcc is **44.10 %
  faster** (31-round amplified paired A/B, median 1.4410 / min 1.4361, p=0.0000).
  Loop structure is *not* the gap — both compilers emit 2 loops here — so do not
  spend this task on fusion or splitting. The gap is 54 extra frame-relative
  references and ~40 extra moves.
- `CCC_PHI_ACYCLIC_ORDER=1` becomes runtime-neutral-or-better *on top of* the
  web-wide supply, so the resolver can be default-enabled without giving back the
  allocator fix's +4 %.
- The `rot()` resolver arm returns to fully register-allocated. Note the historic
  "55 insns / 0 stack refs" target is no longer reachable at production settings
  for an unrelated reason: upstream's `evict_short_k` cost-ratio escape (`d6e2a7f5`)
  costs that arm +2 stack refs (56/2 at `CCC_EVICT_SHORT_K=0` → 61/4 at the default
  16) while *helping* the legacy arm (71/33 → 71/27). The full isolation matrix is
  in the evidence directory below; `check_phi_acyclic_order.sh` now pins the
  resolver's contribution at both settings instead of an absolute zero that belongs
  to a different component.

Evidence:
[`../evidence/ra-web-inloop-use-2026-09-11/`](../evidence/ra-web-inloop-use-2026-09-11/)
(raw factorial JSON, isolation matrices, reproduction commands) and
[`../FOLLOWUP-2026-09-11-phi-acyclic-copy-order.md`](../FOLLOWUP-2026-09-11-phi-acyclic-copy-order.md)
(falsified alternatives: store-to-load width mismatch, `CCC_EVICT_MODE`,
`CCC_NO_TIER2_GRAPH`, `CCC_NO_LEAF_CALLER_HOME`, forcing `MachInst` onto the loop).

## Validation battery

`cargo test --lib` · full `run_regression.py` · 300/300 O2/O3 differential
fuzz · 600 phi-CFG + 540 alias · gzip 1.14 30/30 + roundtrip · zlib-ng
ctest · expat ctest · kernel corpus 15/15 output-identical · adler/gzip A/B
interleaved best-of-3 with checksums.

## Do not

- Do not promote copy leaders without the web extension (measured +28 %
  runtime, reverted — DECISIONS.md "Register allocation").
- Do not key segment decisions on raw per-value segments (need the merged
  coalesce-leader union; `sqlite_yy_shift` miscompiled once).
- Do not model evicted occupancy as closed windows (half-open `[start, cut)`
  — verifier encodes both).
- Do not attempt a clean-slate allocator rewrite (SGSA rejected: RA-05a→06a
  incremental path is strictly safer).
- RA-06B: do not retry the store-to-load width mismatch as the cause of the
  `sha256` regression — rewriting the round loop's `movq` stores/`movl` loads
  to 32-bit moves runtime by 1.8% (noise) and leaves the gap at 5.5%.
- RA-06B: do not retry `CCC_EVICT_MODE=6` or `CCC_NO_TIER2_GRAPH=1`; neither
  changes the victim set. Victim choice is made in scan/coalescing.
- RA-06B: do not judge the fix by instruction count. The opt-in arm has *fewer*
  instructions and stack refs and is slower; the metric that moved is
  reads-per-slot on a demoted recurrence value.

## 2026-09-11 — oracle-derived target for the sha256 gap (rebased on `d03ca818`)

Compiler Explorer on `sha256_transform` at `-O2`
(evidence: `engineering/evidence/godbolt-sha256-2026-09-11/`):

| compiler | insns | frame-slot refs in the loop body |
|---|---|---|
| clang 23.1.0 | **126** | **0** |
| gcc 16.2 | 154 | 1 |
| lccc (shipping) | 198 | **52 refs / 14 distinct slots** |

Hottest slot `360(%rsp)` = the first parameter `u32 *state`: stored once at entry,
reloaded at **every** use in the write-back epilogue. Note the allocator's decision to
spill it is *correct* — it is used in two clusters with the 64-round loop between them.
The gap is codegen, and it is **not** reload-CSE:

- lccc materializes each address (`movq 360(%rsp), %rax; leaq 4(%rax), %rax`) instead of
  using `4(%base)`, and the `lea` overwrites the base, forcing the next reload.
- clang holds the base in one register and emits `movl 4(%rbase), %eax` — zero reloads.

**Acceptance target for this task, from the oracle:** drive in-loop frame-slot refs on
`sha256_transform` from 52 toward **0** and the instruction count from 198 toward
**126**, by (1) preferring `disp(base)` / SIB addressing over materializing `base + k`,
and (2) admitting a spilled value that is reloaded ≥ N times within one basic block with
no intervening call to a callee-saved register for that block. Judged by runtime on the
amplified benchmark, not by instruction count alone (see "Do not" below).

### Do not (added 2026-09-11)

- **Do not retry reload-CSE / slot-load dedup as the fix.** It was implemented in
  `global_store_forwarding` (recording `slot → reg` on loads, the sibling of the
  existing store→load direction, gated `CCC_NO_SLOT_LOAD_DEDUP`), proven sound
  (2394 lib tests + 6 new barrier tests; the 5 fragile affected regression TUs —
  `i128_pair_store_load`, `switch_i128_high_half`, `i128_stack_argument`,
  `vararg_indirect_va_list`, `va_arg_wide_struct` — execute byte-identically correct
  ON vs OFF), and **removed 2 instructions out of 8414 (0.024 %)** across 51 benchmark
  programs; sha256 gained exactly 1. Each epilogue group contains a genuine indirect
  write through the reloaded pointer (`movl %eax, (%r8)`), which must invalidate every
  mapping, so dedup cannot reach the pattern. Reverted.
  Full record: `engineering/FOLLOWUP-2026-09-11-x86-slot-load-dedup.md`.
- **Do not relax `has_indirect_mem → invalidate_all_mappings` for read-only indirect
  operands** (`addl (%rcx), %eax`) to unlock the above: it widens the risk surface of
  every slot-tracking pass for a measured payoff of nil.
