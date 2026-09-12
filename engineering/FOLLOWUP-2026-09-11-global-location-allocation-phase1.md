# Global Location Allocation (P0-A), Phase 1 — root-cause report & decision record

**Date:** 2026-09-11
**Code:** `src/backend/location_alloc.rs` (new), `src/backend/split_ranges.rs`
(Fix #3 lazy entry-alloca + `pub(crate)` planning helpers),
`src/driver/pipeline.rs` (gate), `src/backend/mod.rs` (module).
**Gate:** `CCC_RA_GLOBAL_LOCATION=1` (default OFF; `=0`/`false`/`off` also OFF).
**Rebased onto:** main `25ed36d` (PR #499 merged in the same session).

---

## 1. What the module is

The production register allocator's only spill model is whole-range lifetime
demotion. RA-06 added *intra-block* Belady-MIN splitting and proved that
partial intra-block spill-then-color is a dead end for the loop-carried φ
webs that dominate real pressure peaks — those need cross-block SSA repair.
GLA is that cross-block substrate:

- a `Location { Register, Stack, Rematerialize }` / `LocationPiece`
  vocabulary the planner reasons in;
- **rematerialization** of source-less cheap defs (`GlobalAddr`,
  `Copy <const>`): clones at each use cluster, no stack traffic;
- **cross-block spill gaps**: one immutable capture store after the unique
  def, reload clusters at next uses, dominance-free joins realized in the
  slot (the SplitKit observation), with critical-edge trampolines for
  fan-out φ edges;
- a post-rewrite structural verifier (unique defs, intra-block
  store-before-reload, cross-block dominating store, id bounds, φ prefix).

## 2. Soundness fixes folded in this work

1. **Def-block closure barrier** — the memory-region BFS must not enter a
   block that (re-)defines the value; liveness's φ-edge-copy
   over-approximation marks even an in-block-defined value live-in, and a
   reload propagated in before the capture anchor reads a stale slot.
   Test: `closure_does_not_reenter_def_block` (asserts the precondition
   `is_live_in(3,3)` first, then probes `memory_closure` directly).
2. **Folded-read range guard** — `gp < gs || gp > ge` before mapping a
   global folded read into local coordinates (a saturating subtraction
   fabricated a phantom point 0 inside the φ prefix).
   Test: `folded_reads_never_land_in_phi_prefix`.
3. **Lazy entry alloca in `apply_local_call_split`** — the entry alloca was
   minted *before* wrapping call sites; when the only site was in block 0
   the insertion shifted every site index, guards skipped all sites, and
   the function returned `Some(0)` with a leaked alloca whose id was never
   synced to `next_value_id`; GLA then minted colliding ids (crash repro
   `artifacts/repros/crash_gen_slot_stress_seed20260930_-O2.c`: duplicate
   definition of v2437). The alloca is now minted at the first confirmed
   wrap with corrected raw/inserted indices; `wrapped == 0 → None` with
   zero mutation. Tests in `split_ranges`:
   `local_call_split_in_entry_block_does_not_leak_alloca`,
   `local_call_split_without_site_is_clean`.
4. Defense in depth: the materializer floors its next-value counter at the
   largest existing dest id + 1 before minting anything.

All four are regression-tested; fixes #1–#3 were negative-validated
(reverting each makes its test fail).

## 3. Profitability — the measured Phase-1 policy

The first planner fired on the pressure proxy alone ("any block over
budget, edit something"). On the 78-file benchmark corpus at -O2 it was a
large **regression**: +610 instructions (+4.8%) and **+520 stack
references (+51%)**, with the driver up +418 stkref. Root-causing the
regression produced four independent, fail-closed filters; each is a
gate whose loosening can only ADD edits:

| Filter | Root cause | Evidence |
|---|---|---|
| φ-web color classes in pressure counts (`ColorClasses`) | Pressure counted each SSA name; the colorer coalesces φ result+operands for free. Loop-carried φ webs therefore inflated every loop peak. Unioning webs can only under-count (fail-closed). | Halved gap damage; nbody/adler discrimination. |
| Reach band 6 (`CCC_GLA_REACH`), with a **block-level gate** | A block beyond `budget + 6` (the ~6 callee-saved registers the colorer can still buy, plus folding) cannot be made colorable by a few edits; editing it only perturbs the colorer's own folding/callee-save plan. Credit is block-gated: a block with a hopeless peak credits **nothing**, even at points that are individually within the band — found via negative testing of an earlier per-point version that still credited relief at the reachable buildup points of a hopeless block. | nbody inner loop peaks 38–45 classes (FP loop); its v94 format-string global covers ONLY hopeless blocks (`covers_reachable=false`) — every edit was futile/cascading; adler32 peaks 14–16 vs budget 12 — one remat tips it. |
| Remat segment cap 1 (`CCC_GLA_REMAT_MAX_SEGMENTS`) | A global with 2+ hole-aware segments clones across two use regions and reorders the global register assignment for +28 insns / +88 stkref; a single-segment hoisted base frees a register cleanly. Now defense-in-depth for nbody (the reach band rejects first), but still independently load-bearing: with `CCC_GLA_REACH=999` the candidate becomes reachable and the cap must reject it; both knobs together re-apply it. | nbody v94 `weighted_uses=2 segments=2` vs adler v2 `segments=1`. Anchored end-to-end in `tests/regression/check_gla_remat_policy.sh`, which pins the full 2×2 gate matrix on the real program. |
| Spill gaps OFF (`CCC_GLA_SPILL_GAPS=1` to re-enable) | Pre-alloc gaps fire on residency the colorer resolves for free by folding memory operands; the explicit store+reload pair cannot be folded and mostly replaced once-per-call callee-save pushes with per-iteration reload traffic. Even at reach=2 gaps remained +31 insns/+10 stkref on the new allocator. Gap machinery is retained as the P0-B/post-alloc-feedback substrate, fully fuzzed and unit-tested, but never plans in shipped policy. | A/B sweeps over reach ∈ {2,4,6,10,64}, min-benefit, ratio, and intra-block gates. |

Intra-block gaps stay separately rejected (RA-06's measured result).

The master gate now parses truthy values properly (`CCC_RA_GLOBAL_LOCATION=0`
means off), and the max-edit env is parsed once in the module.

## 4. Hard data (post-rebase allocator, PR #499)

### 4.1 Static census (78 files; `ra_quality_census.py` same-binary A/B)

x86-64, **-O2** (feature ON vs default):

| metric | A | B | Δ |
|---|---:|---:|---:|
| instructions | 12606 | 12610 | +4 (+0.03%) |
| stack references | 1003 | 991 | **−12 (−1.2%)** |
| stores (oracle tooling, adler only changed function) | 6 | 4 | **−2** |
| spills (oracle tooling) | 19 | 7 | **−12 (−63%)** |
| pushes | 345 | 345 | 0 |

Exactly one function changes: `zlib_ng_adler32.c::main`. The +4 insns are
cold (modulo-reduction prologue block + one post-loop unconditional jump;
frame 56→40 bytes). The −12 stack references are hot: the unrolled DO8
inner loop goes **8 → 0** stack references, and the 2M-iteration LCG fill
loop loses its single stack reference; the residual store/reload pair
sits in the NMAX outer block (once per 5552-byte block, ~694× less often
than the inner loop).

x86-64, **-O0**: −12 instructions / −8 reg-reg moves corpus-wide
(binary_trees −6 insns/−3 stkref; fannkuch kernel −6 insns/−7 moves,
frame −16 B, +3 stkref in the memory-resident debug tier).

**i686** (budget 6 GPRs), -O2: +3 insns / **−5 stkref**, non-main kernels
−1 insn / −2 stkref; the only +insn sites are cold table/prologue setup
(i686_alu_chains function-pointer GOT table, matmul scalar main) with
frames 16 bytes smaller; the hot kernels (reduction_vecreg,
fp_memfold_stencil5, tls_pass, conv_u8_3x3) all improve. -O0: large wins
(~−48 insns, ~−48 stkref).

### 4.2 Godbolt oracle (`codegen_oracle.py`, `-O2 -march=x86-64-v3`)

adler32 `main` (all callees inlined into main by the inlining compilers):
LCCC 245→249 insns while spills 19→7; GCC 16.2 = 156 insns / 2 spills,
Clang 23.1 = 148 / 0, ICX (latest) = 147 / 3. ICC 2021.10 (73 insns) does
not inline the callee and is not comparable. The DO8 hot inner loop is
~30 insns/8 bytes with zero stack traffic after GLA vs GCC's ~28 — the
remaining gap is production-colorer/scheduler work, not pre-allocation.
The decision is invariant across `-O2/-O3`, base/v3/native march strings.

Full-corpus rank diff (off vs on): one changed function in 229 compared.

## 5. Validation matrix (post-rebase)

- `cargo test --lib`: **2400 pass / 0 fail** (23 GLA tests incl. the five
  policy tests; 15 split_ranges tests).
- `check_benchmark_outputs.sh` with GLA + `CCC_VERIFY_REGALLOC=1`,
  -O0…-O3: **204/204 PASS**.
- `run_regression.py` with GLA + verifier: **745 pass / 0 fail**.
- Differential fuzz vs gcc, -O0/-O2, GLA on: synthetic, phi_cfg,
  differential, intcmp_thread (60 each) all PASS; stress_suite failure set
  **identical** to baseline (31 pre-existing FP-stress failures, zero
  new). i686 fuzz (m32, alu_torture, slot_rmw, regparm, alias_m32, 60
  each): zero failures. Gap substrate (spill gaps enabled + verifier
  forced): synthetic/phi_cfg/differential 100 each, zero failures.
- i686 benchmark gate: 86/102 pass both ways; the same 16 pre-existing
  oracle failures with and without GLA (symmetric diff empty).
- New CI gate `gla-remat-policy`; shell regression pinned to the real
  nbody program (default policy rejects its 2-segment candidate and is
  output-identical; raised cap applies it and stays output-identical).

## 6. Negative results recorded (do not re-litigate cheaply)

- Profitability cannot be tuned through benefit/ratio knobs alone: the
  proxy never sees folded operands or callee-save buys; min-benefit up to
  100000 still regressed. Only structural filters (coalesced classes,
  reach band, segment cap) discriminate.
- Remat on multi-segment webs: measured loss (nbody), capped at 1.
- Spill gaps pre-allocation: net-negative through the full sweep; keep
  the machinery for P0-B cyclic φ webs and for a post-alloc feedback
  driver that plans from *actual* colorer spills (the correct future
  input to the gap planner).
- Trial-accept against a standalone `allocate_registers` at the early
  IR stage would use a different allocator state (MachInst-window tier,
  folding happens later) and was not attempted; post-alloc feedback is
  the sound version.

## 7. Next (P0-B and beyond)

P0-B sha256 cyclic φ rotation (ring header 9 dests ← 8 latch values) is
the first planned consumer of the gap/trampoline substrate, driven by
post-allocation spill evidence rather than a proxy. P0-C SIB/IV
allocation, then the scheduler/XMM work in P1, are prerequisites for
moving the adler-class functions the rest of the way to the oracle.
