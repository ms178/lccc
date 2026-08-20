# Current Register Allocator Analysis (August 2026)

**Supersedes** the March 2026 write-up of a 574-line three-phase greedy
allocator. That file described a snapshot that no longer exists.

## Overview

Production entry: `src/backend/regalloc.rs::allocate_registers`.

The **scan kernel** is `LinearScanAllocator` in `src/backend/live_range.rs`
(Poletto & Sarkar 1999 lineage, LLVM `RegAllocGreedy` cascade idea, Braun–Hack
MIN tie-break). The **policy** (who may enter which pool, coalescing, ABI
hazards) is entirely in `regalloc.rs`.

`graph_coloring.rs` colors **stack slots**, not GPRs.

## Key files

| File | ~Lines | Purpose |
|------|--------|---------|
| `src/backend/regalloc.rs` | 3612 | Policy + waves + XMM + i686 + phi |
| `src/backend/live_range.rs` | 1261 | LiveRange, eviction, run/run_with_seed |
| `src/backend/liveness.rs` | 2163 | Intervals, segments, loop depth, call points |
| `src/backend/split_ranges.rs` | 1377 | Call / loop-transparent IR splits |
| `src/backend/generation.rs` | — | `run_regalloc_and_merge_clobbers` |
| `src/backend/stack_layout/*` | — | Homes for spilled values |

## Result / config (actual structs)

```rust
pub struct PhysReg(pub u8);

pub struct RegAllocResult {
    pub assignments: FxHashMap<u32, PhysReg>,
    pub used_regs: Vec<PhysReg>,
    pub caller_save_spans: FxHashMap<u8, Vec<(u32, u32)>>,
    pub liveness: Option<LivenessResult>,
}

pub struct RegAllocConfig {
    pub available_regs: Vec<PhysReg>,       // callee-saved
    pub caller_saved_regs: Vec<PhysReg>,
    pub call_arg_regs: Vec<PhysReg>,        // SysV arg regs (empty i686/RISC-V)
    pub indirect_target_regs: Vec<PhysReg>, // e.g. r10 fptr
    pub allow_inline_asm_regalloc: bool,
    pub xmm_regs: Vec<PhysReg>,             // x86 XMM or AArch64 v-regs (id 40)
    pub never_materialized: FxHashSet<u32>,
    pub folded_index_uses: FxHashMap<u32, Vec<u32>>, // AArch64 only
}
```

March docs omitted `call_arg_regs`, `indirect_target_regs`, `xmm_regs`,
`never_materialized`, `folded_index_uses`, `caller_save_spans`.

## Liveness

`LiveInterval { start, end, value_id }` is still the fat envelope.

**New:** `LivenessResult.segments` — hole-aware pieces. Call-spanning uses
segments: a call in a diamond *gap* does **not** force callee-saved. A call
at a segment start (loop re-entry format string) **does** (`seg.start <= cp < seg.end`,
`cp > def`).

Also: GEP-base extension, f128 source-pointer gen, setjmp live-across,
`gep_base_values` for priority bump.

Program points: one per instruction then terminator, block order. Shared
with `build_live_ranges` / PGO point weights.

## Eligibility (GPR)

Whitelist (non-float, non-i128, non-i64-on-32): BinOp, UnaryOp, Cmp, Cast,
Load, GEP, GlobalAddr, LabelAddr, Copy (if dest not non-GPR), Call result,
Select, AtomicLoad/Rmw/Cmpxchg, ParamRef.

Then `remove_ineligible_operands` strips memcpy/va/atomic pointers,
CallIndirect fptrs, inline asm (unless allowed), stackrestore, etc.
`never_materialized` stripped. Optional `exclude_every_third_mul_temp` on
x86 FP pool (pressure hack).

**Still no general FP GPR path** — floats go to XMM/NEON class when
`xmm_regs` is non-empty.

## Coalescing

1. **Copy groups** (`build_coalesce_groups`): union-find over `Copy dest, Value(src)`
   iff pairwise interval disjoint. Scan allocates the **leader**; members inherit.
   Group priority = sum of members' loop-weighted uses (else ParamRef leaders
   look single-use and lose Phase 2c).
2. **Phi latch** (`detect_phi_coalesce_groups`): loop-carried dest + backedge
   src; window-use and cross-block sqlite `deleteTable` shapes rejected.
   Backedge src removed from eligible so the pair shares via later policy.
3. **Die-at-birth hints** in the scan (`follow_value`): Copy, ALU LHS, FP LHS,
   Neg/Not/Bswap, Sqrt/Fabs. **Not** applied on the rotation path (sqlite
   `or %r9,%r9` miscompile).

## Allocation waves

### Phase 1 — callee-saved, call-spanning

GlobalAddr/LabelAddr **are** eligible (no remat path; excluding them spilled
nbody `bodies`, +274 B).

### Phase 2 — caller-saved, non-spanning

- **i686:** per-reg `%ecx`/`%edx` scratch hazards; inclusive overlap.
- **x86-64/AArch64:** four waves, most-constrained first, `run_with_seed`
  so later waves cannot alias earlier homes:
  1. indirect-call args index ≥ 1 — no arg regs, no r10
  2. indirect-call arg 0 — no r10
  3. direct-call args index ≥ 1 — no arg regs
  4. rest — full caller-saved pool
- ParamRef (and groups containing them) excluded from caller-saved except
  x86 single-block leaf param copies.

### Phase 2c — leftover callee-saved

Only **unused** callee-saved regs. Group + GEP-base priority bumps.

### Phase 2d — i686 load-hazard refine

Loads whose pointer is already in a register (or folded global) do not
clobber `%ecx`; those corridors reopen.

### Phase 2e — i686 `%eax` homes

Hazards everywhere except Phi / Branch / Unreachable. Last-use hazard OK
only if **every** use is accumulator-first (BinOp/Cmp LHS, Store val, …).
RHS-in-eax was a fuzz miscompile (`xorl %eax,%eax`).

### AArch64 loop-pin

Steal a colder callee-saved for a hot phi dest (`CCC_LOOP_PIN`, default 2).
Explicitly **not** on x86 (gzip regression).

### XMM / NEON

Second linear scan over `xmm_regs`. Scalar FP + fail-closed 128-bit alloca
vecreg whitelist (compute intrinsics only; FMA/horiz/raw readers poison).

## Spill model (important)

The scan **demotes the whole remaining lifetime**. There is **no**
reload-at-next-use inside `live_range.rs`. `enable_splitting` is reserved
and unused. Real splitting is the IR pre-pass `split_ranges.rs` (local
same-block post-call; unique-exit loop transparent). That is **not**
Wimmer/Mössenböck second-chance allocation.

Stack slots are packed later; `allocate_spill_slot` 8-byte placeholders
are **not** the frame.

## Cost model

- `priority = max(1, Σ pgo_weight(use)) * 10^min(depth,4)`
- `spill_weight = priority / (end-start+1)`
- Eviction mode 3: incoming.priority > victim, victim next_use > incoming.end,
  steal-safe, cascade
- Mode 5: future-use exchange — **measured worse as default**
- PGO factor default 1 (4× double-counted loop depth, gzip +4.7%)

## Remaining structural weaknesses

See [WEAKNESSES_AND_BACKLOG.md](WEAKNESSES_AND_BACKLOG.md). Headline:

- Whole-interval demotion vs split/reload
- Fat intervals (segments used only for call-span, not for the scan graph)
- No interference graph / Briggs optimistic coloring
- No remat of lea/const/GlobalAddr
- Eligibility still conservative for many pointer/atomic shapes
- Multi-wave linear scans cannot globally recolor
- XMM vecreg whitelist vs real SIMD IR
- Cost model is use-count, not Raptor Lake port/latency
