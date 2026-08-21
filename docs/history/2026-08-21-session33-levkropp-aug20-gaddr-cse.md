# 2026-08-21 session 33 — levkropp/lccc from 2026-08-19, class-aware GlobalAddr CSE

**Base:** `ms178/lccc` `c00a71831ea63d98e2ba54c24d92e2d91ab7aebd` (PR #157).
**levkropp/lccc HEAD:** `8ef1978f` (2026-08-20 19:54 -0400).
**Merge-base:** `0bce973e`. Trees have diverged: ms178 +336 / lev +53.

## Audit mandate

Treat every levkropp commit as broken until proven innocent. Session 21 already
adapted the 2026-08-19 portable cluster (redundant_loads, DCE companion,
quadratic_sr) and rejected copy-loop vectorization as already present in a
more mature restrict-aware form. This session covers **2026-08-19 00:00 -0400
onward**, including the 2026-08-20 wave.

## Verdict table (20 commits)

| SHA | Subject | Verdict |
|---|---|---|
| `0c5134cc` | AArch64: load stack-homed FP directly into FP regs | **REJECT** — AArch64; session 21 blocked the quartet on a PhysReg-class panic. Do not graft RA-pool changes onto a crashing baseline. |
| `dd0fa7c5` | AArch64: coalesce FP loop-phi backedge copies | **REJECT** — title is AArch64 but the diff also touches `regalloc.rs` / `copy_coalescing.rs` / `alias.rs` on a 336-commit-older RA. Re-fit only after the AArch64 panic is gone. |
| `6e955227` | Vectorize plain-copy loops via map pattern | **REJECT** — already present, more mature (`store_val == load_dest`, `roots_proven_distinct`). Session 21. |
| `3fa030e5` | Late redundant-load elimination | **ALREADY ADAPTED** (session 21) with volatile + AtomicLoad + span-sync hardening. |
| `bbab1d12` | DCE after late RLE | **ALREADY ADAPTED** (session 21). |
| `8a052b9a` | AArch64 register-direct Select + sxtb | **REJECT** — AArch64 peephole. |
| `9f304050` | AArch64: open unused loop-promotion FP regs | **REJECT** — RA pool change, AArch64, blocked baseline. |
| `885f129c` | Second-order strength reduction | **ALREADY ADAPTED** (session 21) with span-sync; i686 stays cold because `div_by_const` is disabled there. |
| `6a42dd1e` | Document AArch64 codegen findings | **REJECT** — docs for a backend we are not grafting this session. |
| `9c48a277` | AArch64: call-spanning F64 → callee-saved FP | **REJECT** — SysV x86-64 has **no** callee-saved XMM. Porting this heuristic to x86 would save/restore XMM around calls and fight the gzip frame gate. |
| `ed36c446` | GlobalAddr CSE + TCE dest_ptr/asm-output rewrite | **ADAPTED WITH HARDENING** (this session). See below. |
| `e3b21b8f` | AArch64 fmsub/fnmsub fusion | **DEFER** — trait hook in `generation.rs` is interesting for x86 FMA (IS-04/OP-05) but the emitter is NEON `fmsub` and is gated off accumulators. Do not enable a second FMA path without an ICX nbody oracle. |
| `62cc4bb1` | Roadmap: fmsub gated not reverted | **REJECT** — docs only. |
| `1b4bac8b` | Fix aggregate temp forwarding hoist; enable full unroll by default | **REJECT** — ms178 `aggregate_copy_forward.rs` is a different, more conservative pass (dest-liveness window, param-home exclusion, span ICE guard). The hoist patch does not apply. Default-on `CCC_FULL_UNROLL` is the opposite of our gzip/expat policy; we already complete-unroll 2/3-block trips 2..=8 with CRC/F32 guards. |
| `aadab341` | AArch64 FP spill-slot store forwarding | **REJECT** — 420-line ARM peephole. x86 already has `store_load_forward` + redundant_loads. |
| `e20cdf6e` | AArch64 escape-analysis DSE | **REJECT** — ARM peephole; our aggregate DSE is IR-level and default-closed. |
| `de451e57` | AArch64 ldp/stp fusion | **REJECT** — no x86 analogue (adjacent movq pair fusion is a different encoding problem). |
| `b712c79c` | AArch64 dedup slot loads | **REJECT** — ARM. |
| `d0a30aa0` | Roadmap dead-ends | **REJECT** — docs. |
| `8ef1978f` | Late vectorize conditional-sum (smax+sadalp) | **REJECT as-is** — AArch64-only `VecSmaxZeroI32x4`; map-path `gep_uses_iv(..., 4)` hard-codes element size 4 (i64 maps would mis-scale); lev reports correctness **45/50** vs our 50/50. Marching-pointer support in the reduction analyzer is the only portable idea — extract it as a follow-up against `loop_patterns`, not as a 300-line vectorize.rs rewrite. |

## What landed

### 1. Class-aware GlobalAddr CSE (`src/passes/global_addr_cse.rs`)

Lev’s pass was substitution-based (good — no pointer Copy chains) but **class-blind**.
On this tree, `never_materialized` / `build_foldable_global_addr_set` turns
GlobalAddrs whose every use is a Load/Store pointer into `sym(%rip)` / SIB.
Merging a RIP-only `window` address into a value that is also passed to a call
would pin `window` in a GPR and undo RA-01.

Hardening versus `ed36c446`:

* Split **foldable** vs **must-materialize**. Never mix.
* A must-materialize use of a *derived* address (GEP of GlobalAddr used as
  Intrinsic `dest_ptr`, call arg, …) poisons the originating GlobalAddr.
* Rewrite via `Instruction::for_each_operand_mut` **and**
  `for_each_value_use_mut` (complete); do not reuse the incomplete TCE helper.
* Source-span lockstep: 1:1 remove, otherwise drop (never index OOB).
* Tests use `IrFunction::new` (lev’s struct literal is missing
  `ret_is_f128_sse` and Store/Load `volatile`).
* Kill switches: `CCC_NO_GADDR_CSE`, `CCC_DISABLE_PASSES=gaddrcse`.
* Pipeline: after SROA / post-structural-inline, before the main loop.

### 2. `replace_values_in_inst` dest_ptr / InlineAsm outputs

Lev found a real hole: substitution missed `Intrinsic::dest_ptr` and
InlineAsm outputs. That helper is also used by `redundant_loads`. Fixed in
both `tail_call_elim.rs` and the parallel copy in `loop_unroll.rs`.

## Explicitly not taken

* Default-on full unroll (`1b4bac8b`).
* Late AArch64 clamp vectorizer (`8ef1978f`).
* Any AArch64 RA/peephole commit.
* Call-spanning F64 → callee-saved (no x86 XMM callee-saved).

## Validation plan (this session)

* `cargo test --lib` (fastbuild, `-D warnings`)
* targeted `global_addr_cse` unit tests
* `tests/regression/global_addr_cse_runtime.c`
* `tests/regression/check_global_addr_cse.sh` (RIP preserved; kill switch)
* correctness / regression subset as wall-clock allows on 2c/2GB

Full gzip/expat/zlib-ng/sqlite/glibc/kernel gauntlet is **not** runnable to
completion in this harness (2 GiB RAM). Those remain the next-session gate
before default trust.

## Follow-up ranking

1. **RA-01 remat** — CSE reduces must-mat pressure; RIP remat of file-scope
   `window`/`prev`/`strstart` as `sym(%rip,idx,1)` is still the gzip
   `longest_match` gap (118 stack-mem vs GCC 0).
2. **Extract marching-pointer `gep_uses_iv`** from `8ef1978f` with a real
   element-size argument (not hardcoded 4) and x86 `pmaxsd`/`phaddd` or
   AVX2 for `sum(x>0?x:0)`. Gate on `loop_patterns` A/B, gzip unregressed.
3. **AArch64 PhysReg-class panic** (session 21 open defect 2) — then re-fit
   the AArch64 quartet + call-spanning F64 against qemu-aarch64 differential.
4. **x86-64 -O0 miscompile family** (zlib-ng `gz_reset`, sqlite `openDatabase`)
   — still the #1 correctness follow-up from session 21.
5. **IS-04 / OP-05** — ICX YMM FMA accumulator, not GCC 16 horiz-per-iter.
   Do not copy lev’s NEON fmsub path.
6. **RA-23 / RA-24** — still the hard blockers for graph coloring.
7. Linker oracles: `tests/linker/setup_oracles.sh` already honours git-HEAD
   mold with `-DMOLD_TARGETS='X86_64;I386'`, wild from git, bfd 2.47. Do not
   use release tarballs. cmake is not in this harness; skip mold this session.

## Files

* `src/passes/global_addr_cse.rs` (new)
* `src/passes/mod.rs` (wire + module)
* `src/passes/tail_call_elim.rs` (dest_ptr / asm outputs)
* `src/passes/loop_unroll.rs` (same)
* `tests/regression/global_addr_cse_runtime.c`
* `tests/regression/check_global_addr_cse.sh`
* `tests/benchmark/programs/global_addr_pressure.c`
* `docs/history/2026-08-21-session33-levkropp-aug20-gaddr-cse.md`
