# Current compiler state

SHA at last doc refresh: **`b6d0a30`** (`ms178/lccc` main, PR #153). Re-verify line numbers before editing. The 150-item catalog is [`agent/BACKLOG.md`](agent/BACKLOG.md) (RA-01…MS-07).

## What is production

- **C frontend** → SSA IR → `-O0` skip / `-O1` light / `-O2` full / `-O3` +unroll / `-Os`/`-Oz` size (`src/passes/README.md`).
- **Linear-scan RA** in `src/backend/live_range.rs` + policy in `regalloc.rs` (waves, coalescing, XMM/NEON, i686). Not a 3-phase greedy allocator. No `linear_scan.rs`.
- **Liveness** `src/backend/liveness.rs` — worklist backward dataflow (no `MAX_ITERATIONS` cap). Produces both fat `intervals` and hole-aware `segments`. `segments` is consumed by `regalloc.rs` for call-spanning detection and interval extension; the linear scan itself still runs on fat `intervals`.
- **SROA** `aggregate_sroa.rs` load-forward + chain collapse **on**. Copy-out **off** (`CCC_SROA_COPYOUT` hangs tests).
- **Alias** `alias.rs` (128 LOC) — `LoopFrames`, `resolve_in_frame`, `forms_disjoint` (SCEV-lite). Consumed by `redundant_loads` only. LICM does **not** yet use it for GEP loads (`licm.rs:750` TODO).
- **FMA** scalar `vfmadd231sd` and vector `vfmadd231pd` **emitters exist**. Auto-vectorize of non-reduction loops and FMA-in-vector-body are the remaining gaps.
- **YMM memcpy** exists in `memory.rs`. By-value struct ABI still shuttles `double` through `%rax` (`struct_copy` 21× vs GCC).
- **MachInst** ISel/emit path exists; **disabled** when loop insts > 32 (`CCC_MI_MAX_LOOP_INSTS`) because the local scheduler **regressed gzip ~3%**. The `machinst_regalloc.rs` module (635 LOC) is **dead code** (zero callers) — deletion candidate (P0-01).
- **PGO** generate/use; layout must not reorder hot loops (expat 131→248 ms). `vectorize_gate` is `return true`.
- **`enable_splitting`** in the scan is dead (`false`, never read). Splitting is the IR pre-pass `split_ranges.rs` (now correct: rewrites uses, phi-safe, span-preserving).
- **`outline_switch`** min cases = **40**.
- UnaryOp already emits `lzcnt`/`tzcnt`/`popcnt`. C if-trees (`__ffs`, hand-rolled popcount) do not become those insns until recognized.
- **`__builtin_cpu_supports`** folds against an exact Raptor Lake allowlist (FIXED — the old "return 1 for everything" SIGILL bug is gone). Still compile-time, not runtime CPUID.
- **`usual_arithmetic_conversion`** else-arm is correct C11 6.3.1.8 (FIXED — `signed_ty.size() > unsigned_ty.size() ? signed : signed.to_unsigned_version()`).
- **`split_ranges.rs`** call-split now actually rewrites uses (FIXED — old version was a no-op that ran mem2reg on a non-volatile alloca). Loop-split now scans all body blocks, inserts after Phis, rewrites terminators.

## What is still losing vs GCC 14 / gcc16.2 / ICX

| Gap | LCCC | Oracle |
|-----|------|--------|
| gzip `longest_match` | 118 stack-mem, GOT, 248 B frame | gcc RIP `window(%r9,%rcx)`, 1 push |
| Adler-32 kernel | 1.49×; `sum2`/`n` on stack in DO8 | CE whole-file ~0–3 stack refs |
| CRC-32 kernel | 1.49×; 2 vs 0 spills | gcc `xorl table(,%reg,4)` — **no `crc32` insn** at `-O2` |
| Expat name scan | 1.95× | gcc `btq` |
| xmltok / inflate TUs | 12× / 15× stack-mem | segment RA (scan still on fat `intervals`) |
| struct_copy | 21.06× | SysV SSE class + no xmm↔rax field copies |
| nbody / spectral / mandelbrot | 3–9× screening | ICX FMA+YMM (copy **ICX**, not gcc16 horiz-per-iter) |
| find_bit | 1.85× screening | gcc `andn`+`cmov` on ffs tree (**not** tzcnt) |
| bitops | — | gcc/clang `popcntl` if IR is Popcount |
| sieve | 1.3× | gcc 45-insn **scalar**; clang ymm explosion — **do not copy clang** |
| fib/TCE geomean | LCCC can beat gcc | **not** a codec metric |

## Dead code (delete first — P0-01)

| File / field | LOC | Status |
|---|---|---|
| `src/backend/x86/codegen/machinst_regalloc.rs` | 635 | Zero callers (verified) |
| `src/backend/stack_layout/graph_coloring.rs` | 131 | Zero callers (verified) |
| `LiveRange::reg_hint: Option<PhysReg>` | field | Never set to `Some` (verified) |
| `LinearScanAllocator::enable_splitting: bool` | field | Never read (verified) |
| `LinearScanAllocator::handled: Vec<ActiveInterval>` | field | Write-only after `run()` (verified) |

## Hard blockers for RA improvements

1. **`immediately_consumed`** (`stack_layout/copy_coalescing.rs:1328` `compute_immediately_consumed` + `is_safe_sole_consumer`) hard-codes the accumulator codegen's operand load order. Any RA change placing a "would-be immediately consumed" value in a non-accumulator register will miscompile. Must be refactored to RA-owned accumulator hint before RA can own placement (RA-23).
2. **`SlotAddr::Indirect(StackSlot(0))` dummy** (`state.rs:844`) for register-homed values. Every Indirect codepath must check `reg_assignments` first — convention, not enforced. New codepaths that forget silently read offset 0 (return address / saved RBP). Add `SlotAddr::Reg(PhysReg)` variant (RA-24).
3. **Lifetime demotion** (`live_range.rs:26-27`): "There is no reload-at-next-use in this module." 3-5× spill traffic vs LLVM. `segments` infrastructure is built but the scan doesn't use it for splitting (RA-05/RA-06).
4. **`verify_no_overlap` is `eprintln!`-only** (`regalloc.rs:1757`) — non-aborting, O(n²), uses fat `intervals` not `segments`. Promote to `debug_assert!` (RA-13).

## Dual pipelines (do not "just enable")

Text ISel + string peephole **and** MachInst ISel/emit (the MachInst RA module is dead code — delete it). Homes are many HashSets, not a `ValueLocation` enum. PhysReg **(11) = `%r10`**, (10) = `%r11`.
