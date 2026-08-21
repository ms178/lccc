# First tickets

Re-oracle and check gzip `longest_match` stack-mem after **each** item.

## Phase 0 — Hygiene & enablers (do first)

| Step | ID | Work |
|------|-----|------|
| 1 | P0-01a/b | **DONE/BLOCKED:** deleted only dead MachInst RA; retained and perfected graph coloring under research gate, pending RA-23; wired `reg_hint` and `handled`; kept splitting stub. |
| 2 | MS-07 / P0-03 | **DONE on `50e7ac83` + patch:** pinned gzip 1.14, 30/30; longest_match LCCC 303 instructions/119 stack refs vs GCC 114/0; kill-switch control identical. |
| 3 | RA-04 | **DONE:** `CCC_RA_EXPLAIN=fn` deterministic one-line-per-spill report. |

## Phase 1 — Measured codec gaps (P0)

| Step | ID | Work |
|------|-----|------|
| 4 | RA-01 + IS-26 | RIP-relative `window`/`prev`/`strstart` (not GOT+stack). CE: gcc `window(%r9,%rcx)`. |
| 5 | RA-02, RA-03 | Next-use: match IVs in GPR; Adler DO8 keep `sum2`/`n`. CE Adler ~0 stack. |
| 6 | IS-01 | **DONE:** safe masked-index homes produce scale-4 SIB CRC loads; -12 B/-7 instructions, 1.34x faster than control, full differential clean. |
| 7 | IS-12, IS-28 | **DONE:** BMI-gated direct-source ANDN (+4% find-bit vs control); canonical SWAR Popcount already lowers to `popcntl`, revalidated. |
| 8 | IS-02, AB-01 | `double` stays in XMM; SysV SSE-class aggregates. |
| 9 | IS-03 | **DONE for isolated assignment:** AVX2 64 B → 2× YMM pairs + vzeroupper, -17 bytes; broader struct SROA remains OP-02. |
| 10 | IS-04, OP-05 | ICX-style YMM FMA on nbody/matmul/spectral/reduction. |
| 11 | IS-06, OP-01 | **OP-01 DONE:** shared alias forms wired to LICM with checked Shl and fail-closed writes; `btq` classify remains. |

## Phase 2 — Structural blockers (prerequisites for all subsequent RA work)

| Step | ID | Work |
|------|-----|------|
| 12 | P0-02 / RA-05 | Switch `live_range.rs` scan from `intervals` to `segments` (infrastructure built, just wire). |
| 13 | RA-06 | Reload-at-next-use + in-place splitting (the #1 perf gap — 3-5× spill traffic). Gate on `enable_splitting`. |
| 14 | RA-23 | Eliminate `immediately_consumed` blocker (RA-owned accumulator hint). Unblocks P0-01b (graph_coloring Tier 2). |
| 15 | RA-24 | Eliminate `SlotAddr::Indirect(StackSlot(0))` dummy (`SlotAddr::Reg(PhysReg)`). |
| 16 | RA-13 | **DONE:** hard segment/final verifier plus handled eviction-history verifier. |
| 17 | RA-26 | **DONE conservatively:** scalar non-sret SysV/AArch64 ParamRef hints; mixed ABI signatures refuse. |
| 18 | P0-04 / OP-01 | **DONE:** shared alias engine in LICM, targeted hoist plus full differential validation. |
| 19 | P0-05 / FE-01 | Sema type-checking (correctness, not UX). |
| 20 | RA-25 / OP-31 | Unify `loop_memory_promote` alias engine with `alias.rs`. |

Veto: gzip match stack-mem. Stop if sqlite/expat miscompile.
