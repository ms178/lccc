# Current compiler state

SHA at last doc refresh: **`35a6b88`** (`ms178/lccc` main, PR #154). Re-verify line numbers before editing. The 150-item catalog is [`agent/BACKLOG.md`](agent/BACKLOG.md) (P0-01…MS-09).

## What is production

- **C frontend** → SSA IR → `-O0` skip / `-O1` light / `-O2` full / `-O3` +unroll / `-Os`/`-Oz` size (`src/passes/README.md`).
- **Linear-scan RA** in `src/backend/live_range.rs` + policy in `regalloc.rs` (waves, coalescing, XMM/NEON, i686). Not a 3-phase greedy allocator. No `linear_scan.rs`.
- **Liveness** `src/backend/liveness.rs` — worklist backward dataflow (no `MAX_ITERATIONS` cap). Produces both fat `intervals` and hole-aware `segments`. `segments` is consumed by `regalloc.rs` for call-spanning detection and interval extension (lines 571, 785); the linear scan itself still runs on fat `intervals`.
- **SROA** `aggregate_sroa.rs` load-forward + chain collapse **on**. Copy-out **off** (`CCC_SROA_COPYOUT` hangs tests).
- **Alias** `alias.rs` (128 LOC) — `LoopFrames`, `resolve_in_frame`, `forms_disjoint` (SCEV-lite). Consumed by `redundant_loads` only. LICM does **not** yet use it for GEP loads (`licm.rs:750` TODO).
- **FMA** scalar `vfmadd231sd` and vector `vfmadd231pd` **emitters exist**. Auto-vectorize of non-reduction loops and FMA-in-vector-body are the remaining gaps.
- **YMM memcpy** exists in `memory.rs`. By-value struct ABI still shuttles `double` through `%rax` (`struct_copy` 21× vs GCC).
- **MachInst** ISel/emit path exists; **disabled** when loop insts > 32 (`CCC_MI_MAX_LOOP_INSTS`) because the local scheduler **regressed gzip ~3%**. The `machinst_regalloc.rs` module (635 LOC) is **dead code** (zero callers, soundness bug in `rewrite_machinsts` RAX-clobber) — delete only this file (P0-01).
- **PGO** generate/use; layout must not reorder hot loops (expat 131→248 ms). `vectorize_gate` is `return true` (`unroll_pgo.rs:122`).
- **`enable_splitting`** in the scan is a stub (`false`, never read) — keep as the gate for RA-06 (reload-at-next-use), do not delete.
- **`outline_switch`** min cases = **40** (was 999999; fixed).
- UnaryOp already emits `lzcnt`/`tzcnt`/`popcnt`. C if-trees (`__ffs`, hand-rolled popcount) do not become those insns until recognized.
- **`__builtin_cpu_supports`** folds against an exact Raptor Lake allowlist (`expr_builtins.rs:436`, `PRESENT` const). FIXED — the old "return 1 for everything" SIGILL bug is gone. Still compile-time, not runtime CPUID.
- **`usual_arithmetic_conversion`** else-arm is correct C11 6.3.1.8 (`types.rs:1586`: `signed_ty.size() > unsigned_ty.size() ? signed : signed.to_unsigned_version()`). FIXED.
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

## Dead code — nuanced assessment

Not all dead code should be deleted. Some is sound infrastructure blocked by a hard blocker, or a stub for an unimplemented feature.

| Item | LOC | Verdict | Rationale |
|---|---|---|---|
| `machinst_regalloc.rs` | 635 | **DELETE (P0-01a)** | Zero callers; `rewrite_machinsts` routes all spills through RAX (clobbers if both src+dst spilled — soundness bug); two-RAs-fighting design — MachInst path piggy-backs on main RA via `resolve_stack_vregs` (`emit.rs:2998`) |
| `graph_coloring.rs` | 131 | **KEEP — wire after RA-23 (P0-01b)** | Sound interval-graph-coloring algorithm; blocked by `immediately_consumed` accumulator cache; real win for switch-heavy code (sqlite VdbeExec: O(N)→O(1) slots); upgrade to `liveness.segments` for hole-aware coloring |
| `reg_hint: Option<PhysReg>` | field | **WIRE UP as RA-26** | `follow_value` can't cover ABI-mandated registers (ParamRef→%rdi, sret→%rdi, return→%rax); field + read path (`find_free_register:295`) + `register_compatible` all exist — just needs populator in `build_live_ranges` |
| `enable_splitting: bool` | field | **KEEP (RA-06 stub)** | Not dead code — unimplemented feature stub; becomes the gate when reload-at-next-use (RA-06) lands |
| `handled: Vec<ActiveInterval>` | field | **WIRE INTO RA-13** | Eviction-chain verification; write O(1) per expiry/eviction, read only in debug; `verify_no_overlap` should check `handled` for co-holder overlaps |

## Hard blockers for RA improvements

1. **`immediately_consumed`** (`stack_layout/copy_coalescing.rs:1328` `compute_immediately_consumed` + `is_safe_sole_consumer`) hard-codes the accumulator codegen's operand load order. Any RA change placing a "would-be immediately consumed" value in a non-accumulator register will miscompile. Must be refactored to RA-owned accumulator hint before RA can own placement (RA-23).
2. **`SlotAddr::Indirect(StackSlot(0))` dummy** (`state.rs:844`) for register-homed values. Every Indirect codepath must check `reg_assignments` first — convention, not enforced. New codepaths that forget silently read offset 0 (return address / saved RBP). Add `SlotAddr::Reg(PhysReg)` variant (RA-24).
3. **Lifetime demotion** (`live_range.rs:26-27`): "There is no reload-at-next-use in this module." 3-5× spill traffic vs LLVM. `segments` infrastructure is built but the scan doesn't use it for splitting (RA-05/RA-06).
4. **`verify_no_overlap` is `eprintln!`-only** (`regalloc.rs:1757`) — non-aborting, O(n²), uses fat `intervals` not `segments`. Promote to `debug_assert!` (RA-13).

## Dual pipelines (do not "just enable")

Text ISel + string peephole **and** MachInst ISel/emit (the MachInst RA module is dead code — delete it; keep the ISel/emit path). Homes are many HashSets, not a `ValueLocation` enum. PhysReg **(11) = `%r10`**, (10) = `%r11`.
