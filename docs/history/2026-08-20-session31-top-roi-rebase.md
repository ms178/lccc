# 2026-08-20 session 31 — nuanced resurrection + top-ROI implementation

**Base:** latest `ms178/lccc/main` at
`50e7ac8310e49dc0251c3eba811b636238be0a9b` (PR #155).

## Dead-code audit correction

GLM's audit was checked against the code and accepted where evidence supports
it. Only the private zero-caller MachInst allocator was deleted. The live
MachInst ISel/emitter remains on the main RA.

The other components were restored and used:

- `graph_coloring.rs`: rewritten as exact-size, hole-aware segment coloring
  with conservative closed stack-lifetime boundaries and protected/missing
  value fallback. It is called from Tier 2 under
  `CCC_ENABLE_TIER2_GRAPH`.
- `reg_hint`: populated with conservative ABI physical hints. Only scalar,
  non-sret signatures whose GP slot numbering is unambiguous qualify; mixed
  FP/aggregate/i128 signatures refuse. `follow_value` always has priority.
- `enable_splitting`: retained as the RA-06 feature gate.
- `handled`: now records physical register plus the actual occupancy end;
  eviction cuts occupancy at the incoming range's start. Verification consumes
  expired and evicted history instead of discarding it.

### Graph-coloring blocker measured, not guessed

Enabling Tier-2 coloring before RA-23 caused `huft_build_crash` and
`sqlite_vdbe_peephole` crashes plus 166/450 phi-CFG mismatches. Closed segment
boundaries alone cannot model hidden accumulator materialization. Therefore
the perfected colorer remains research-wired/default-off until RA-23. Shipping
it enabled would knowingly miscompile.

## Ten highest-ROI items addressed

1. **P0-01a:** deleted only unsound/dead `machinst_regalloc.rs` (635 LOC).
2. **P0-01b:** retained, rewrote and wired segment graph coloring; blocker
   quantified and fail-closed.
3. **P0-02/RA-05 partial:** retained the validated i686 residual segment fill:
   boot C text -1,573 B and stack references -13.1%, no eviction/new saves.
4. **P0-03/MS-07:** remeasured pinned gzip 1.14 on this base: 30/30 upstream
   tests; `longest_match` LCCC 303 instructions/119 stack-memory references,
   GCC 114/0. Treatment and kill-switch control are identical for this gate.
5. **P0-04/OP-01:** shared `LinForm`/`forms_disjoint` is now used by LICM.
   Calls, atomics, memcpy, inline asm, intrinsic writes and unresolved stores
   fail closed. Checked Shl scaling was added to the shared resolver.
6. **RA-04:** `CCC_RA_EXPLAIN=fn` emits deterministic one-line spill evidence
   (range, segment count, weighted uses, reason class).
7. **RA-13/P0-01e:** hard O(n log n) final segment verifier plus `handled`
   occupancy-history verification through eviction chains.
8. **RA-26/P0-01c:** ABI physical register hints. A 180-function scalar sweep
   reduced `.text` 14,037 -> 14,005 bytes; best function eliminated three
   entry shuffles (-9 B). Runtime/structural regression included.
9. **IS-01/RA-17:** safe byte-mask hidden indexes retain a home through
   Cast/Shl chains, enabling scale-4 SIB CRC table loads. Broad hidden homes
   were rejected after phi fuzz; only canonical `x & 255` I32/U32 table
   indexes qualify until RA-23.
10. **shared alias infrastructure:** `loop_memory_promote` linear forms now
    resolve checked `Shl(value,const)` exactly like multiply-by-power-of-two,
    benefiting LICM and every existing alias consumer.

## CRC breakthrough

For `gzip_crc32` at `-O2 -march=x86-64-v3`:

| metric | control | treatment | delta |
|---|---:|---:|---:|
| `.text` | 452 B | **440 B** | **-12 B** |
| instructions | 132 | **125** | **-7** |
| paired VM median | 239.80 ms | **179.30 ms** | **1.34x faster** |

The generated hot loop now uses `movl (base,index,4),reg` instead of
Cast+Shl+LEA+load. Against local GCC 14, LCCC is now 1.066x slower, a major
closure of the prior ~1.49x gap. Compiler Explorer (`main`, same flags) reports
LCCC 120 instructions, GCC 16.2 50, Clang 22.1 82, ICC 60, ICX-latest 100;
the remaining gap is explicit and retained in the artifact manifest. No
`crc32` instruction was introduced; this matches the oracle strategy.

## LICM alias result

The regression loop loads `a[0]` and stores `a[i+1]`. Shared linear forms prove
constant/marching separation and hoist exactly one load before the loop.
Unresolved or potentially aliasing stores remain in-loop. The complete alias
and phi differential batteries pass.

## Validation

- fastbuild `-O1 -j2`, active 8 GiB swap;
- 962 unit pass, 6 ignored;
- 50/50 correctness;
- 357 regression pass + same six documented GCC-oracle mismatches;
- alias differential 720/720 (`O0/O2/Os`);
- phi-CFG differential 600/600 after narrowing hidden index homes;
- pinned gzip 1.14: 30/30 tests per compiler, bit-identical streams;
- graph-coloring enable experiment retained as negative evidence, default off;
- ARM/RISC-V indexed/ABI paths compile on standalone cross-target kernels;
- canonical patch mirror + SHA manifest + latest-base apply check.

## Still highest priority

RA-23 explicit accumulator locations and RA-24 `SlotAddr::Reg` are prerequisites
for default Tier-2 graph coloring and broad cross-backend segment allocation.
Next generated-code gaps remain gzip rematerialization/next-use, SysV aggregate
SSE classes, 64-byte YMM copies, and ICX-style non-reduction FMA vectorization.
