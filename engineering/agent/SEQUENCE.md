# First ten tickets

Re-oracle and check gzip `longest_match` stack-mem after **each** item.

| Step | ID | Work |
|------|-----|------|
| 1 | P0-01 | **Delete dead code**: `machinst_regalloc.rs` (635 LOC), `graph_coloring.rs` (131 LOC), `reg_hint`/`enable_splitting`/`handled` dead fields. Zero risk, 1 day. |
| 2 | MS-07 / P0-03 | LCCC `-S` of gzip `deflate.c` `longest_match` on **this SHA** (`b6d0a30` or newer). Do not reuse 118-stack-mem without re-measure. |
| 3 | RA-04 | `CCC_RA_EXPLAIN=fn` spill dump. |
| 4 | RA-01 + IS-26 | RIP-relative `window`/`prev`/`strstart` (not GOT+stack). CE: gcc `window(%r9,%rcx)`. |
| 5 | RA-02, RA-03 | Next-use: match IVs in GPR; Adler DO8 keep `sum2`/`n`. CE Adler ~0 stack. |
| 6 | IS-01 | CRC `xor table(,%rcx,4)`. |
| 7 | IS-12, IS-28 | `andn` for find_bit; IR Popcount → `popcntl` on bitops. |
| 8 | IS-02, AB-01 | `double` stays in XMM; SysV SSE-class aggregates. |
| 9 | IS-03 | 64-byte assignment → 2× `vmovdqu %ymm`. |
| 10 | IS-04, OP-05 | ICX-style YMM FMA on nbody/matmul/spectral/reduction. |

**After step 10**, address the structural blockers (these are prerequisites for all subsequent RA work):

| Step | ID | Work |
|------|-----|------|
| 11 | P0-02 / RA-05 | Switch `live_range.rs` scan from `intervals` to `segments` (infrastructure built, just wire). |
| 12 | RA-06 | Reload-at-next-use + in-place splitting (the #1 perf gap — 3-5× spill traffic). |
| 13 | RA-23 | Eliminate `immediately_consumed` blocker (RA-owned accumulator hint). |
| 14 | RA-24 | Eliminate `SlotAddr::Indirect(StackSlot(0))` dummy (`SlotAddr::Reg(PhysReg)`). |
| 15 | RA-13 | `verify_no_overlap` → `debug_assert!`. |
| 16 | P0-04 / OP-01 | Wire `alias.rs` into `licm.rs:750`. |
| 17 | P0-05 / FE-01 | Sema type-checking (correctness, not UX). |
| 18 | RA-25 / OP-31 | Unify `loop_memory_promote` alias engine with `alias.rs`. |

Veto: gzip match stack-mem. Stop if sqlite/expat miscompile.
