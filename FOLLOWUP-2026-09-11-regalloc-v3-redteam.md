# Follow-up: red-team audit of Astra regalloc report + Agent A v1, and v3 next batch

Date: 2026-09-11 · Base: `ms178/lccc` main `@ 70974b2` (PR #486, re-verified
at session end) · Deliverable: `/home/user/ms178-1.patch` (v3, squashed)

## 1. Verdicts up front

| Input | Verdict |
|---|---|
| Astra V1 findings (overlap, union-find, webs, eligibility, div/rem, phi path, synthetic liveness) | **Correct on every P0 mechanism.** Two proposals were wrong/over-broad (zero-length skip, destructive-SIMD replacement) — Astra itself withdrew both in V2. |
| Astra V2 (corrected kernel, deferrals) | **The trustworthy core.** Overlap kernel + flatten + eligibility + seeds + FP fixes are necessary and sufficient as specified. The `used_regs`/`accumulator` deferrals were over-cautious (see §3). |
| Astra V3 batch (interval envelope, div/rem stamps, phi-exemption delete, dirty-state walk, vecreg poison, CFG liveness, wide-ops, saturation, determinism, test hardening) | **Agree with 9 of 11 items.** Disagree on (a) deleting the phi-web exemption (hole-aware check is the correct proof, not deletion) and (b) abort-on-extra-src-def (refresh is sound and required for adler32-class latches). Both disagreements are proven below with IR + measurements. |
| Agent A v1 (`ms178-1-AgentA.patch.txt`, 2709 lines) | **Adopt as v3 base with 7 amendments.** All core mechanisms validated (54→55 tests green, kernels 266/264 unchanged, gate green, differential 57/57, fuzz 4/4). Found and fixed: 1 dead test, 1 missed steal phase, 1 P0 soundness hole (ignored hidden reads — fixed with a novel proof, not a revert), 1 robustness issue (repair `assert!`), 1 over-broad exemption (repair unions), 1 ARM hole (promoted-FP call spanning). |

v3 test totals: **58 regalloc unit tests** (31 baseline + 24 AgentA + 1 revived +
2 new), full `--fast` CI green (see §5).

## 2. Adjudication, item by item

### 2.1 P0: `find_overlapping_classes` misses pairs (Astra V1§2, V2§2–3) — AGREE

Traced on current main: the previous-only scan reports `A↔B, A↔C` but never
`B↔C` for `A[0,100) B[1,20) C[2,30)` on one register. Evicting `A` leaves a
live `B/C` overlap. Astra's V2 `overlapping_class_spans` (active-list sweep,
existing `intervals_overlap` predicate, zero-length preserved) is the correct
kernel. AgentA's port is faithful; v3 keeps it and adds the exhaustive
triple-oracle test. **No better algorithm needed: output-sensitive sweep is
optimal for this query.**

### 2.2 P0: repair/verify treat direct parents as roots (V1§2, V2§3) — AGREE

`class_weight` and the eviction loop compared `parent[v] == rep` without path
compression, so transitive members were neither weighed nor evicted.
`flatten_allocation_classes` + `allocation_class_root` fix it. AgentA port is
faithful. v3 additionally unifies the verifier onto the same kernel (AgentA)
and iterates repair to a fixpoint (§4.4).

### 2.3 P0: `collect_gpr_scan_intervals` resurrects ineligible leaders (V1§3, V2§4) — AGREE

The `merged_of` branch skipped the `eligible` check. Both Astra's replacement
and AgentA's port (plus deterministic sort) are correct. Covered by
`collect_gpr_requires_eligibility_and_sorts`.

### 2.4 P0: member restrictions not propagated to leaders (V1§3, V2§5) — AGREE

`propagate_member_restrictions` for `scratch_denied`/`later_arg_values`/
`indirect_arg_values`/`riscv_entry_guard` + all-acc-first leader rule.
Necessary; AgentA port faithful. Note it is still per-set, not a unified
legality interface (remains the strategic debt Astra V1§15 names).

### 2.5 P0: wave seeds use leader-only bounds (V1§4, V2§5) — AGREE

`allocation_owner_bounds` (merged-or-fat) at all three seed sites. Kept the
`+1` seed encoding (external contract unverified — correct call by Astra).

### 2.6 P0/P1: segment coverage drops unsegmented members (V1§5–6, V2§6) — AGREE

`owned_live_segments` (fat fallback per value) + `collect_call_spanning_owners`
(segment-authoritative, fat fallback). Both correct; the call-spanning fallback
closes a fail-open ("no segments ⇒ not spanning") into fail-closed.

### 2.7 P0: FP-web leader duplication + wrong member map (V1§7, V2§6) — AGREE

Removing the original leader interval before inserting the merged one, leader
election from scan-admitted members only, and `attach_scan_segments` with
`fp_web_member_of` are all correct. AgentA port faithful. v3 additionally
unions `fp_web_member_of` in repair (AgentA) so legitimate FP sharing is not
evicted as a conflict.

### 2.8 P0: div/rem pairing ignores width + incarnation (V1§8, V2§10, V3§2) — AGREE

Old code paired on (signedness, operands, opposite flavor) only. The prefix
`v1=10; v2=UDiv(v1,3); v1=20; v3=URem(v1,3)` fuses two different numerators.
Astra V3 stamps (exact type, operand-def stamps, barrier stamp, single-def
dests) are the right proof; AgentA's port is faithful including the
`InlineAsm`/`NonlocalGotoSave`/`returns_twice` barrier set. Validated by 4 unit
tests + a runtime differential probe (`artifacts/divrem_probe.*`, 8/8 match
GCC) + fusion-intact check (exactly one `divl`/`idivl` per pair). The V1
whole-function single-def rule was correctly withdrawn (over-strict).

### 2.9 P0: destructive SIMD allocator (V1§9, V2§9) — AGREE WITH V2 (no replacement)

Astra V1's `compute_live_intervals` rewrite did not establish the 9 proof
obligations (producer/consumer capability, clobbers, XMM/YMM aliasing, …) and
was rightly withdrawn. AgentA correctly left the allocator alone (only the
`args.get(1)` hardening + comment trim). **Next batch, not this one:** needs
the producer/consumer/clobber matrix first (§6, item N1).

### 2.10 P1: `synthetic_vec_intervals` `hit` + layout heuristic (V1§10.1, V3§6) — AGREE

The `hit` flag demonstrably drops sibling operands' blocks. AgentA's CFG
backwards-dataflow rewrite (uses+defs+terminators, live-in/live-out bound
extension) is the correct model; the only-defs-are-Intrinsic asymmetry is
fail-closed (over-approximates). Validated by all 16 `simd_*` regression tests
passing + `simd_sse2_arith`/`simd_avx2_256`/`simd_crc_adler` differential vs
GCC. The layout-adjacent loop-region heuristic is gone.

### 2.11 P1: `x86_body_has_wide_ops` fixpoint (V1, V3§7) — AGREE

Existence queries don't need propagation; the old code also missed typed wide
`Cmp` and the wide return type. Astra's single-pass replacement (AgentA port
faithful) is correct and faster.

### 2.12 P1: `used_regs` built pre-repair (V1§11, V2-deferred) — AGREE WITH AGENTA (implement)

Astra V2 deferred this ("prove emitters don't depend on historical
reservations"). I proved it: `used_regs` feeds only `used_callee_saved` →
prologue save-set (GPR `available_regs` only); ARM FP callee-saved (32..38)
and the 48..55 promotion pool bypass `used_regs` by construction
(`regalloc_helpers.rs` builds disjoint pools); inline-asm clobbers merge
afterwards; `caller_save_spans.retain` only drops regs with no surviving home
(fewer saves, never missing saves). Rebuilding from final assignments is safe
and strictly reduces prologue traffic. AgentA's implementation kept.

### 2.13 P1: accumulator `Vec::new()` on non-divrem targets (V1§10.2, V2-deferred) — HOLD

Astra is right that "no fusion ≠ no candidates", but enabling a previously
dead target path needs that target's accumulator contract first (notably
RISC-V/AArch64 `stack_layout` interplay). AgentA held; v3 holds. Next batch
item N4 with per-target differential proof.

### 2.14 P1: target recognition via reg IDs / `!is_32bit` (V1§12.9) — AGREE, PARTIAL

The `collect_call_arg_values` x86-64 gate now tests `EM_X86_64` (AgentA).
Remaining reg-ID-as-target checks (`r.0 == 11` RISC-V signatures etc.) are
narrowed but not eliminated; they are pools-construction facts, not legality
proofs. Follow-up item N5: central `target_is_*` predicates at config
construction.

### 2.15 P0: phi-web exemption in general coalescing (V3§3) — DISAGREE WITH ASTRA, AGREE WITH AGENTA

Astra would delete the `phi_web_class` exemption entirely. That is fail-closed
but imprecise: phi dest/incomings on mutually exclusive CFG arms (diamonds)
*should* share a home; their linearized fat intervals overlap while their
segments don't. The correct proof is hole-aware:
`same phi-web AND NOT sorted_coverage_overlaps(segments(a), segments(b))`
with fat-envelope fallback for missing segments (AgentA's
`coverage_of_value`). Segment data is the allocator's own hole model — using
it here is consistent, and `phi_web_rejects_overlapping_segments` pins the
simultaneously-live case. Full small-to-large interference-checked rewrite
(Astra V3§3 code) is rejected for this batch: higher risk, no measured need
once the hole-aware rule lands.

### 2.16 P0: source-path skips copy-block redefs (V1§12.2, V3§4) — AGREE, WITH TWO REFINEMENTS

The instruction-level dirty-state walk is required (block-level was unsound).
On the two sub-questions:

* **Extra `src` defs: refresh, not abort (AGREE WITH AGENTA).** The walk is
  path-sensitive: a `src` redefinition writes the shared home with the current
  source value along exactly the paths that execute it. Aborting rejects
  in-place `buf += 8` latches (adler32 stackmem 11→12, measured). Note the
  detector additionally requires single-def sources for *candidates*, so
  refresh only matters for apply-time revalidation shapes.
* **Folded reads: consult, with a dest-rooted exemption (NEITHER — v3 proves
  more).** AgentA ignores `folded_read_points` (P0 hole: a folded SIB read of
  `src` in a dirty window is a genuine use of a clobbered home). Astra vetoes
  all of them (sound but imprecise: blocks adler32 `v727=v703`, stackmem
  11→12, gate red — measured §5). v3 proves the precise rule: a folded read
  vetoes **unless** the folding consumer's address root is syntactically
  `dest` (`Load`/`Store` through `dest`, or one const-displacement
  `GEP`/`Add` link on `dest`). Then the shared home holds `dest`'s current
  runtime value, which is exactly what the access needs however the static
  copy attribution names it; foldability is irrelevant (unfolded links never
  read the home). Implemented as `folded_consumer_reads_dest_home` +
  `const_address_link_bases`, pinned by `source_home_rejects_folded_read_in_dirty_window`
  (veto) and `folded_consumer_rooted_in_dest_does_not_veto` (exemption +
  control). adler32 back to 230 insns / 11 stackmem, gate green.

### 2.17 P0: unknown vecreg writers + width contract (V3§5) — AGREE (AgentA scope)

Poison-on-unknown-`dest_ptr`-writer + `size != 16` rejection are correct and
sufficient for the legacy 128-bit slot family. Astra's full opcode-table
rewrite is deferred: the `Vec*` 256-bit entries never take `dest_ptr` slot
writers in practice (verified by the 16/16 SIMD sweep + differentials), and a
table rewrite without emitter co-proof risks churn. Next batch item N2.

### 2.18 Remaining Astra blockers (V1 §12.3–12.8, V2 §8)

* **Mul-acc SSA stamps (12.3):** open, next batch N3 (same stamp technique as
  div/rem; needs `casts.rs` + emitter contract first).
* **Late i686 hazard vs eviction (12.4) / assignment-dependent codegen (V2§8):**
  open, the deepest architectural issue. Next batch N1 with fixpoint
  revalidation design (conservative-stable clobber model as the fallback).
* **FP-phi rehome call validation (12.5):** fixed by AgentA
  (`spans_any_call` + callee-FP exemption); fat-interval check is the
  conservative direction. Kept.
* **AArch64 promoted-FP tail (12.6):** half-fixed by AgentA (reservation
  plumbing predates). v3 adds the missing call-spanning guard (§4.6).
  Residual: occupancy vs the scan pool is still bypassed — by design
  (reserved IDs), documented.
* **Caller-save-spanning switch (12.7):** still experimental/off by default
  (`caller_save_spanning`); no change. Demands the full legality filter
  before default-on (N6).
* **Compile-time/determinism (V1 §13, V3 §§8–10):** saturation +
  single-phase tie-break + segment ordering landed via AgentA; v3 completes
  the second steal phase (§4.2). `FunctionRaFacts` lite is next batch N7.

## 3. Where Agent A was wrong (all fixed in v3)

1. **Dead test:** `vecreg_rejects_non_16_byte_alloca` lacked `#[test]` — the
   16-byte guard shipped with zero coverage. Revived; passes (55th test).
2. **Second steal phase untouched:** hot-web steal still used wrapping `.sum()`
   and cost-only tie-break (hash-iteration-dependent). Now
   `summed_use_weight` + `(cost, reg_id)` tie-break like phase one.
3. **Hidden reads ignored:** `source_home_survives_dest_redefs` took
   `_liveness` and never consulted `folded_read_points`. Fixed with the
   dest-rooted proof (§2.16), not a revert: adler32 keeps 230/11.
4. **Repair `assert!`:** a hard panic on any future enumeration/victim
   disagreement would crash compilations. Replaced with a bounded fixpoint
   loop (each round evicts ≥1 class; fail-closed all-evict backstop) +
   `debug_assert!`.
5. **Repair still unions non-applied pairs:** `phi_coalesce` is the
   dedup-by-dest *candidate* subset, not the applied set (apply rejects
   more). `apply_*` now returns applied pairs (GPR + FP-phi rehome); repair
   and `verify_no_overlap` union exactly those.
6. **ARM promoted-FP call spanning:** `PhysReg(48+index)` bypassed
   call-spanning classification on caller-saved d24–d31. Now skipped when the
   owner spans a call (segment-aware set); the value falls through to the FP
   scan. (Promotion loops are call-free, but ranges can reach exit/outer
   calls.)

## 4. v3 delta (on top of AgentA v1)

`src/backend/regalloc.rs` only (+ this doc):

1. `#[test]` on the dead vecreg test.
2. Hot-web steal: `summed_use_weight` + `(cost, reg_id)` tie-break.
3. `source_home_survives_dest_redefs` consults `folded_read_points` with
   program-point mapping (`block_starts` + checked offsets, fail-closed),
   keeps refresh semantics, + `folded_consumer_reads_dest_home` /
   `const_address_link_bases` exemption with soundness proof in comments.
4. Post-RA repair is a bounded fixpoint over applied-pair unions;
   `debug_assert!` replaces `assert!`; `apply_*` returns applied pairs;
   FP-phi rehome records its pairs; verifier takes the applied set.
5. ARM `48+index` promotion skips call-spanning owners.
6. Tests: `summed_use_weight_saturates_instead_of_wrapping`,
   `source_home_rejects_folded_read_in_dirty_window`,
   `folded_consumer_rooted_in_dest_does_not_veto`.

## 5. Validation data (all on 70974b2 + v3, fastbuild, `-j2`)

| Oracle | Result |
|---|---|
| `cargo test --lib backend::regalloc` | **58 passed** (31 base + 27) |
| `ci-codegen-gate` (8 golden workloads) | **green**; adler32 `230 insns / 11 stackmem` (baseline 233/11) |
| `ra_quality_census --kernels --no-clang` | **266/264** LCCC/GCC, identical to baseline (5 fewer / 3 equal / 7 more) |
| `run_correctness.py` differential | **57/57** |
| `fuzz_diff synthetic ×200 --seed 20260911` | **200/200** |
| `asmdiff --32` (i686) | **20/20**; i686 + cross-backend atomics **PASS** |
| `simd_*` regression sweep (16 files) | **16/16** run-OK; sse2/axv2/crc-adler differential vs GCC **PASS** |
| div/rem probe (fuse / redef-between / mixed-width / narrow / signed loops) | runtime **8/8 match GCC**; asm shows single `divl`/`idivl` per fusable pair |
| adler32 workload runtime A/B (`-DPASSES=2000`, taskset, min-of-7) | lccc 1.422s vs gcc 14.2 1.380s (**−3.0%**, outputs identical); VM has no PMU — treat as directional, not gospel |
| godbolt oracle `-O3 -march=x86-64-v3` static insns (adler8/dot/isort) | LCCC 65/12/34 vs GCC16.2 129/52/242 vs Clang23.1 75/45/51 — LCCC fewest static insns; GCC/Clang counts include vector multi-versioning (faster dynamic, bigger static). Manifests in `artifacts/godbolt-*` |
| `cargo fmt --check` | clean |
| `scripts/ci_local.sh --fast` | **ALL GATES GREEN** (see §5.1) |

### 5.1 `ci_local.sh --fast` transcript tail

(Recorded at session end; full log in `artifacts/ci-fast-v3.log`.)

```
SUMMARY: 17 passed, 0 failed, 3 skipped
ALL GATES GREEN
```

## 6. Next batch (prioritized P0→P3)

* **N1 (P0): assignment-dependent codegen fixpoint.** Late i686 `%eax/%ecx/%edx`
  refinement + steal/repair invalidation loop (Astra V2§8). Design: record
  lowering commitments at refinement; revalidate to fixpoint after any
  home-changing repair; fallback = conservative-stable clobber model.
  Needs emitter contract audit (`i686/codegen`, `x86/codegen`) first.
* **N2 (P0): vecreg width contract table.** Narrow legacy-128 slot table per
  Astra V3§5, co-proven with emitter (`emit_sse_binary_128` family).
  Reproducer hunt first: try to construct a 256-bit-op-on-16-byte-slot IR
  (likely unconstructible from the frontend — if so, downgrade to P2
  hardening).
* **N3 (P0/P1): mul-acc incarnation stamps.** Same stamp technique as div/rem;
  needs `casts.rs` + emitter fusion contract + `wide_nop_casts`/`def_point`
  last-wins audit.
* **N4 (P1): accumulator on non-divrem targets.** Per-target contract proof
  (RISC-V/AArch64 stack interplay) + differential per target.
* **N5 (P1): target-identity cleanup.** Central predicates at config
  construction; remove reg-ID-as-target checks.
* **N6 (P1): caller-save-spanning legality filter.** Full scratch/staging
  model before default-on; currently experimental.
* **N7 (P2): `FunctionRaFacts` lite.** Single def/use/copy-adjacency index per
  function threading through div/rem, mul-acc, coalescing, phi detection.
  Measure compile-time on sqlite-sized functions (expect −10–20% RA time).
* **N8 (P2): adler32 −3% runtime gap.** Schedule/unroll/codegen analysis of
  the scalar loop vs GCC 14.2 (needs P-core-pinned measurement on the
  14700KF; VM data is directional only).
* **N9 (P3): deep-chain folded exemption.** Extend `folded_consumer_reads_dest_home`
  past one link (root==dest through const chains) — only if a workload shows
  the veto firing there (none observed; detection: debug counter).

## 7. Red-team self-audit of the v3 delta

Attacked each v3 change for soundness reversals:

1. **Dest-rooted folded exemption** — the subtlest claim. Re-attacked three
   ways: (a) emitter folds via `src` alias while base==dest: reads shared
   home = `dest_latest` = runtime-correct base value regardless of belief —
   sound. (b) Emitter reads another home (`v722`-style): that home holds its
   own value by the allocator invariant — sound. (c) Index==src with
   base==dest: excluded (const-offset-only rule) → vetoes — sound.
   Residual risk: consumer types beyond Load/Store with folded points —
   fail-closed (`return false`). Liveness only emits folded points for
   Load/Store consumers (verified in `extend_gep_base_liveness`).
2. **Refresh-on-src-redef** — path-sensitive by construction (`seen` includes
   the dirty bit; every path explored). Single-def detector precondition
   makes multi-def refresh apply-time-only. No path can read a stale value:
   any use after a dest redef on its path vetoes unless a src redef on the
   same path refreshed first.
3. **Fixpoint repair** — termination: classes finite, ≥1 evicted per
   non-empty round (first unskipped pair always yields a loser). Cap is
   unreachable-but-total. `debug_assert` keeps test builds strict.
4. **Applied-pair unions** — strictly narrower than candidate unions; any
   conflict hidden before by a rejected candidate is now repaired (more
   evictions possible, never fewer — sound direction; measured: zero
   codegen delta on kernels + gate).
5. **Steal tie-break/costs** — evaluation order changes only among
   equal-cost candidates; saturation only affects values near `u64::MAX`
   (unrepresentable-hot → hottest, the intended order).
6. **ARM call guard** — strictly narrows promotion; skipped values use the
   normal FP scan. No ARM codegen delta possible except removing unsound
   homes (none observed in corpus; guard is preventive).

No garbage, no debug leftovers, no `eprintln` added outside existing debug
flags, no baseline refreshes, no tolerance edits. Patch is
`src/backend/regalloc.rs` + 2 follow-up docs.
