# Linear Scan — Implemented Design (August 2026)

This is **not** a Week-2 implementation plan. The algorithm is in
`src/backend/live_range.rs`. Deviations from the March spec are called out.

## Reference

- Poletto & Sarkar, *Linear Scan Register Allocation*, TOPLAS 1999
- Wimmer & Mössenböck, *Optimized Interval Splitting*, VEE 2005 — **not implemented in-scan**
- LLVM `RegAllocGreedy` cascade / eviction — cascade field only
- Braun & Hack, *Register Spilling and Live-Range Splitting for SSA-Form Programs*, CC 2009 — MIN next-use tie-break

## Data structures (as coded)

```rust
pub struct LiveRange {
    pub value_id: u32,
    pub start: u32,
    pub end: u32,
    pub uses: Vec<u32>,          // sorted unique
    pub loop_depth: u32,         // max(def block, hottest use)
    pub priority: u64,
    pub reg_hint: Option<PhysReg>, // unused by build_live_ranges
    pub follow_value: Option<u32>, // producer id
    pub spill_weight: f64,
    pub cascade: u32,
}

pub struct LinearScanAllocator {
    pub ranges: Vec<LiveRange>,
    pub active: Vec<ActiveInterval>,
    pub handled: Vec<ActiveInterval>,
    pub assignments: FxHashMap<u32, PhysReg>,
    pub reg_free_until: FxHashMap<PhysReg, u32>,
    pub spill_slots: FxHashMap<u32, i32>, // placeholder, not frame
    pub available_regs: Vec<PhysReg>,
    pub next_spill_slot: i32,
    pub enable_splitting: bool,  // STUB — splitting is split_ranges.rs
    pub next_reg_idx: usize,     // ILP rotation
    pub exchange_eviction: bool, // default mode 5 if CCC_EVICT_MODE unset
}
```

March spec fields that **do not exist:** `ActiveInterval.reg` (register is
looked up in `assignments`), allocator-owned `available_regs` split by
class (class is chosen by the **caller** of `new`).

## Main loop

```text
run_with_seed(seed):
  clear state; init free-until = 0; apply seed occupancy
  sort ranges by (start asc, priority desc, value_id asc)
  for range in ranges:
    expire_old (end < start) via swap_remove
    if find_free_register: commit
    else if select_evict_victim / find_exchange_candidate:
      try_evict (steal-safe) or spill
    else spill whole remaining lifetime
```

### find_free_register

1. `reg_hint` if in pool and `register_compatible` (**includes** die-at-birth)
2. else `follow_value`'s assignment, same test
3. else rotate through pool requiring `free_until <= range.start`
   (**no** die-at-birth on this path — sqlite `or %r9,%r9`)

### conflicts_with

Intersecting live points, **except** `a.end == b.start` and `a`'s last
**recorded** use is at `a.end`. Artificially extended ends (GEP, f128, live-through)
do not share.

### Eviction

| Mode | Behaviour | Default? |
|------|-----------|----------|
| 0 | never evict | no |
| 1 | hotter + strictly deeper loop | no |
| 2 | hotter, any depth | no |
| 3 | hotter + victim next_use > incoming.end | **yes** |
| 5 | min future-uses exchange if victim_future < incoming.uses.len() | Phase 2c / env |

`select_evict_victim` searches **all** legal victims (pick-then-reject v1
spilled hot incoming when cheapest victim failed the window).

Steal is refused if another active co-holder of that physreg conflicts
with incoming (die-at-birth pair).

## What the March spec promised vs reality

| Feature | March plan | Now |
|---------|------------|-----|
| Interval splitting in scan | Yes | **No** — IR pre-pass only |
| Register coalescing | Post-pass copy merge | Copy groups **before** scan + follow hints |
| Spill cost | uses * 10^d / length | same, plus PGO factor (default 1) |
| Dead-code skip | skip empty uses | empty uses still can allocate if interval exists |
| Eligibility 40–50% incl float | planned | GPR whitelist + separate XMM class |
| Spill slots in allocator | sequential 8 B | placeholders; real packing in stack_layout |
| Prefer already-used callee-saved | planned in find_free | **not** in scan; Phase 2c uses leftover unused regs |

## Invariants (must not regress)

1. Rotation path never die-at-birth-shares.
2. Incoming eviction cost ≠ victim future-use cost (unifying them is a bug).
3. `run` is idempotent; `init_registers` clears occupancy.
4. `occupy_register` saturates at `u32::MAX`.
5. Spill slot id stable per value.
6. `uses` sorted unique.

## Tests in-module

`live_range.rs` covers: overlap, die-at-birth, inverted span, spilling,
idempotent run, shared-reg steal refusal, mode-3 vs mode-2 victim, occupy
wrap, spill stability, exchange cost asymmetry.

`regalloc.rs` phi tests: sqlite deleteTable cross-block reject, same-block
latch accept, window-use reject, hottest-latch sort.

## Next design steps (do not “implement Week 2”)

See [WEAKNESSES_AND_BACKLOG.md](WEAKNESSES_AND_BACKLOG.md):

- Second-chance / use-point split **inside** the scan (Wimmer)
- Segment-aware interference (scan on holes, not fat envelopes)
- Remat class for lea/const/GlobalAddr
- Hybrid: linear scan then Briggs on the residual interference graph
