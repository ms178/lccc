# Session 43 — red-team audit of Agent D's vector-temp-promotion v2 (2026-08-21)

Base: `ms178/lccc` main `aa8bc98` (PR #178). Build: `target/fastbuild`
(`opt-level=1`, no LTO, 2 jobs, 8 GB swap). Host: 2-core VM, no PMU.

## TL;DR

Audited Agent D's v2 of `src/passes/vector_temp_promotion.rs` (SIMD temp
promotion / load fusion / alignment relaxation) and adopted it after fixing
two soundness holes it re-introduced plus two fail-closed gaps. The claimed
A/B numbers are **reproduced** against the session-42 revision. All gates
green; the 34-file benchmark corpus is byte-identical (the pass only fires on
SIMD-intrinsic code).

## v2's wins over session 42 (verified, kept)

| Kernel | s42 insns | v2 insns | s42 frame | v2 frame |
|---|---:|---:|---:|---:|
| `simd_avx2_256` | 743 | **734** | 1016 | **824** |
| `simd_movnt` | 422 | **390** | 440 | **392** |
| `simd_avx2_defer_chain` | 121 | 121 | 88 | 88 |
| `simd_crc_adler` | 1371 | 1371 | 360 | 360 |
| `simd_defer_chain` | 329 | 329 | 568 | 568 |
| `vector_defer_multidef_slot` | 565 | 565 | 1048 | 1048 |

Two mechanisms produce the wins, both verified sound against the backend:

1. **Non-temporal stores relax to 16, not 32.** `movntdq`/`movntpd` require
   exactly 16-byte alignment; `Movnti`/`Movnti64` require none. The x86 slot
   allocator (`prologue.rs` `assign_slot`) gives `align=16` a 16-aligned
   *direct* slot (with the FPO mod-16 shift) and only `align>16` pays the
   runtime `lea/add/and` dance, so `Some(16)` is both sound and cheap.
   Session 42 fail-closed (`safe.remove`) and kept align 32 for any `dest_ptr`
   store; v2's `intrinsic_dest_required_alignment` relaxes `Storeu*/Store*/FMA`
   destinations to 0 (all lowered `vmovdqu`/`movdqu`) and `Movntdq/Movntpd` to 16.
2. **`pointer_vector_arg_width` unifies forwarding + alignment.** A single
   width-typed table classifies each intrinsic argument as vector-valued
   (forwardable, align-0) or scalar/immediate/address (fail closed), with
   mixed-width cases (`Insert128to256`, `Broadcast128to256`, `Cast256to128`)
   handled per-index.

Also kept: non-escaping-alloca alias refinement (`local_alias_facts`), the
`Add`/`Sub` object-root propagation for folded pointer arithmetic (from
session 42), the ptr-only `Cast` root edge (soundness fix over session 42's
unconditional cast propagation), load-load chaining, single-read-site gating
for multi-use AVX values, and the conservative/exact split alias graphs.

## Defects found and fixed in v2

| # | Defect | Severity | Fix |
|---|--------|----------|-----|
| 1 | `invalidate_for_value_write` ignored the **slot** key (`retain(\|_, load\|)`), checking only source-vs-write. `v = load(p); v = setzero(); use(v)` therefore kept the forward and `use(v)` read the stale source `p` (the exact reassignment miscompile fixed in session 42). | **miscompile** | `write_may_clobber(pointer, target, …)` now tests slot-vs-write *and* source-vs-write; a write through the slot itself kills the forward |
| 2 | `VecStoreI64x2` appears in `produces_vector_value` (slot sizing) but is the memory-form store that writes through `args[1]` with `dest_ptr: None` (`emit_vec_store_addr`). The `dest_ptr: None` invalidation arm exempted it, leaving stale forwards across its write. | **miscompile** | the exemption excludes `VecStoreI64x2` (full barrier) |
| 3 | Load removal ignored the load's `dest` SSA result — a future lowering exposing the register result would be broken by deletion. | fail-closed gap | removal also requires the `dest` result to have zero readers |
| 4 | `destination_unobserved_between` early-return was convoluted. | clarity | split into explicit `escaped` / empty-window cases |

## Missed-optimization opportunities implemented

The six 256-bit widening extensions (`Pmovzxbw256 | Pmovzxbd256 | Pmovzxwd256 |
Pmovsxbw256 | Pmovsxbd256 | Pmovsxwd256`, i.e. `_mm256_cvtepu8_epi16` & co.)
read a **128-bit** source through `sse_load_arg` (`movdqu`) and were absent
from the argument table, so their source loads could not be forwarded and
their source allocas could not relax alignment. Added `(index == 0) → 16`.

## Validation

- **1033** `cargo test --lib` (23 in this module, incl. new
  `intrinsic_write_to_slot_invalidates_pending_load`,
  `vec_store_memory_form_is_a_forwarding_barrier`,
  `widening_extension_reads_a_128_bit_source`).
- **389 + 1 skip** regression suite (all `simd_*`, `vector_defer_*`,
  `temp_promotion_window`, `state_leak`, `loop_alloca_scalar` pass), **50/50**
  correctness, **60/60** O2 differential fuzz — all green.
- **Runtime**: a standalone `movntdq` test (32-aligned local array, two
  `_mm_stream_si128` + `_mm_sfence`) runs and matches GCC; the emitted
  `movntdq` has **no** runtime alignment dance (relaxed 32→16).
- **A/B**: 34-file benchmark + SIMD-pattern corpus byte-identical vs session
  42 (no intrinsics there); SIMD regression corpus improves as tabulated.
- Gate kernels unchanged: gzip_crc32 1.07×, zlib_ng_adler32 1.55×,
  expat_xml_scan 1.89×, struct_copy 1.51×, matmul **1.17× faster** (checksums
  byte-identical).

## Pre-existing, out of scope (unchanged from session 42)

`tests/intrinsics/cases/{t128_fp,t256_fp,t512_int}.c` fail in **semantic
analysis** (imm8 argument-order mismatch between the test cases and the
bundled `immintrin.h`), unrelated to any optimization pass.
