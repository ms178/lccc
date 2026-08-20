---
layout: doc
title: Register Allocator
description: Current linear-scan register allocator (August 2026).
prev_page:
  title: Architecture
  url: /docs/architecture
next_page:
  title: Optimization Passes
  url: /docs/optimization-passes
---

**Engineering index:** [`engineering/subsystems/register-allocation.md`](https://github.com/ms178/lccc/blob/main/engineering/subsystems/register-allocation.md) · backlog [`engineering/agent/BACKLOG.md`](https://github.com/ms178/lccc/blob/main/engineering/agent/BACKLOG.md)

# Register Allocator
{:.doc-subtitle}
Production RA is a **policy-heavy linear scan**, not CCC’s 574-line 3-phase greedy allocator and not the March 2026 “Week 2 plan.”

**Canonical write-up:** [`engineering/subsystems/register-allocation.md`](https://github.com/ms178/lccc/blob/main/engineering/subsystems/register-allocation.md). Work items: [`engineering/agent/BACKLOG.md`](https://github.com/ms178/lccc/blob/main/engineering/agent/BACKLOG.md) (150 highest-ROI). Measurements: [`engineering/evidence/workloads/gzip-zlib-expat.md`](https://github.com/ms178/lccc/blob/main/engineering/evidence/workloads/gzip-zlib-expat.md). `lccc-improvements/register-allocation/` is a stub pointer only.

## Modules

| File | Role |
|------|------|
| `src/backend/regalloc.rs` (~3612) | Eligibility, copy/phi coalescing, call-arg waves, i686 hazards, XMM/NEON, loop-pin |
| `src/backend/live_range.rs` (~1261) | `LinearScanAllocator`, eviction modes, hints, PGO weights |
| `src/backend/liveness.rs` (~2163) | Fat intervals **and hole-aware segments** |
| `src/backend/split_ranges.rs` (~1377) | IR pre-pass: call-split, loop-transparent split |
| `src/backend/stack_layout/` | Homes for **unallocated** values (not a Chaitin GPR allocator) |

`graph_coloring.rs` colors **stack slots**.

## Algorithm (short)

1. Optional `split_ranges` IR rewrite (fail-closed volatile allocas).
2. Liveness: envelopes + segments. Call-spanning uses **segments** (a call in a diamond *gap* does not force callee-saved).
3. Copy-group coalescing (pairwise-disjoint) + phi latch coalescing.
4. **Phase 1** linear scan on callee-saved for spanning values.
5. **Phase 2** caller-saved in constraint waves (`call_arg_regs`, indirect target, i686 `%ecx`/`%edx`, then leftover), `run_with_seed` so waves cannot alias.
6. **Phase 2c** leftover callee-saved.
7. i686 2d/2e; AArch64 loop-pin; XMM/NEON second class.

Default eviction is **mode 3** (hotter incoming, victim next-use after incoming end). Mode 5 (exchange) **regressed gzip** as a default.

The scan **demotes the whole remaining lifetime**. `enable_splitting` is a stub. There is no reload-at-next-use.

## Interface (not frozen)

```rust
pub struct RegAllocConfig {
    pub available_regs: Vec<PhysReg>,
    pub caller_saved_regs: Vec<PhysReg>,
    pub call_arg_regs: Vec<PhysReg>,
    pub indirect_target_regs: Vec<PhysReg>,
    pub allow_inline_asm_regalloc: bool,
    pub xmm_regs: Vec<PhysReg>,
    pub never_materialized: FxHashSet<u32>,
    pub folded_index_uses: FxHashMap<u32, Vec<u32>>, // AArch64 only
}
```

## Screening vs GCC (2026-08-20)

Kernels, checksums matched, VM medians: Adler **1.49×**, gzip CRC **1.47×**, Expat scan **1.95×**. gzip `longest_match`: **118 stack-mem vs GCC 0**, 248 B frame, GOT reloads of `window`. zlib-ng `inflate` **15×** stack-mem. CRC is **not** a spill problem.

Next RA work: remat/`%rip` globals, next-use eviction, segment scan. Do not re-implement March Week 2.
