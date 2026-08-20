# Register Allocator Integration Points (August 2026)

Paths are `src/backend/...` (the crate is no longer `ccc/src`).

## Call flow

```
arch codegen calculate_stack_space / prologue
  → generation::run_regalloc_and_merge_clobbers(...)
       → regalloc::allocate_registers(func, &RegAllocConfig)
            → liveness::compute_live_intervals
            → live_range::build_live_ranges + LinearScanAllocator::run[_with_seed]
       → merge inline-asm clobbers into used_regs
  → stack_layout::calculate_stack_space_common(..., &reg_assigned, cached_liveness)
  → emit_prologue uses used_callee_saved
  → emit uses state.reg_assignments
```

IR pre-pass (earlier in the pipeline, gated by `max_splits`):

```
split_ranges::split_call_spanning_ranges
split_ranges::split_loop_transparent_ranges
split_ranges::place_edge_copy_blocks
```

## Config fields the March docs missed

Call sites **must** fill:

- `call_arg_regs` — SysV AMD64 rdi/rsi/rdx/r8/r9 (not rcx? check backend);
  AArch64 x4–x7 + x8 indirect result; empty i686/RISC-V
- `indirect_target_regs` — AMD64 r10
- `xmm_regs` — XMM ids (~20+) or AArch64 starting at PhysReg(40)
- `never_materialized`
- `folded_index_uses` — **AArch64 only**; x86 must stay empty (session-23
  `pointer_mix` regression)

## PhysReg map (x86-64)

| id | reg | notes |
|----|-----|--------|
| 0 | rax | accumulator; i686 Phase 2e uses PhysReg(6) as eax on that backend |
| 1 | rbx | callee-saved |
| 2–5 | rcx rdx rsi rdi | |
| 6–7 | r8 r9 | |
| 8–9 | r10 r11 | r10 = indirect target |
| 10–13 | r12–r15 | callee-saved |

i686: smaller file; PhysReg(4/5) = ecx/edx scratch hazards; PhysReg(6) = eax home wave.

AArch64: x0–x30; NEON pool advertised as xmm_regs with first id 40.

## Interface is **not** frozen

March claimed “no struct changes”. Reality: `RegAllocResult` gained
`caller_save_spans`; `RegAllocConfig` grew six fields. New features
(remat, split children, hint physregs from ABI) **will** extend these
structs. Keep call sites compiling; do not treat the March signature as
sacred.

## Stack layout coupling

Unassigned values get slots via `stack_layout` three-tier packing +
`graph_coloring.rs` for 8-byte multi-block values. Copy coalescing there
is **independent** of RA copy groups — two systems can disagree. Audit
before changing either.

`regalloc_helpers.rs` filters callee-saved lists per function (i128,
atomics, indirect calls). That filtering is **before** `allocate_registers`;
the allocator must not assume a full architectural file.

## Codegen coupling (miscompile surface)

Homes are **whole-interval**. Codegen assumes a value in `%r12` is in
`%r12` at every use. Introducing in-scan split **requires** reload/spill
insertion or SSA rename — cannot be local to `live_range.rs`.

Die-at-birth sharing requires the **emitter** to read the dying operand
before writing the dest. Hints are restricted to those emitters.

Folded GEP: AArch64 emits `[base, index, lsl #N]` with **no** IR use of
index at the load. RA must extend index liveness via `folded_index_uses`.

## Testing hooks

- `CCC_DEBUG_RA` — final assignment dump
- `CCC_DEBUG_RA_INTERVALS`
- `CCC_TRACE_ALLOC` — overlapping-assignment asserts
- `verify_no_overlap` — fat-interval overlap print (does not understand
  die-at-birth; expect false positives at `end == start`)
