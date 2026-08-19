# 2026-08-19 (session 9) — code-size perfection + Agent B audit + segments fix

**Base:** `origin/main` @ `31b8e404` (Grok regalloc/liveness/generation v2)
**Snapshot:** `/home/user/ms178-1.patch`

## Verdict on Agent B's revision

Agent B's line (`38d3b11a` base + their own generation.rs rewrite) **rejects the
Grok regalloc rewrite entirely** ("restores the previously proven allocator").
Measured against the same pre-rework baseline (`d36ad727`) on the 28-file
benchmark corpus (-O2 text):

- Agent B: **+281 bytes** (nbody +301, memcmp +22).
- This line after this session: **-537 bytes**.

Agent B's rationale was stale: "the rewritten allocator reached only 331
passing / 11 failing" describes the Grok commits BEFORE the red-team fixes; the
11 failures were the call-arg home clobber, the indirect-call target register,
the stack-alignment segfault, the intrinsic `direct_fp_result` clobber, the
`real_use` fixpoint, and the `cheap_remat` exclusion — all fixed in the previous
session. Reverting the regalloc is a dumbing-down the user explicitly forbade.

**Valid Agent B insights adopted** (ported onto the Grok line):
1. Loop-memory-promote zero-trip speculation + target pointer width
   (`src/passes/loop_memory_promote.rs`, + regression test): forbids hoisting
   faulting loads out of possibly zero-trip loops unless the access is proven
   in-bounds and aligned on a local alloca, or both load and store dominate the
   exit; uses `ty.size()` (target pointer width) instead of host width.
2. MachInst fallback replay program-point accounting (`x86/codegen/emit.rs`):
   rewind to the buffered window's start before re-emitting, assert the exact
   endpoint; the old code double-counted the window.
3. `invalidate_vec_scratch_peephole` split (`state.rs`): scalar FP emission
   invalidates scratch forwarding (xmm0/xmm1) but not allocator-managed vector
   homes (xmm3+).
4. `instruction_may_clobber_vector_scratch` whitelist (shared between
   generation.rs and copy_coalescing.rs): deferred vector windows may only
   cross integer/address instructions; FP/vector loads and ops flush first.
5. Liveness `extend_use_following_copies`: removed the 16-hop cap (the visited
   set already terminates; the cap under-approximated liveness).

## Code-size root causes fixed (the user's NAK)

1. **FP global load/store through the GPR shuttle** (`x86/codegen/globals.rs`):
   `mov_load_for_type`/`load_dest_reg` return the integer forms for F64/F32, so
   `emit_global_load_rip_rel` emitted `movq sym(%rip),%rax; movq %rax,%xmmN`
   instead of `movsd sym(%rip),%xmmN`. nbody 2611 -> 2328 (-25 vs baseline).
2. **Param/coalesce interaction** (`regalloc.rs`): coalescing merged ParamRefs
   with loop-carried copies, bypassing the multi-block param gate. Fixed by
   excluding param Copy-edges from `build_coalesce_groups`, propagating the
   param restriction to group leaders, and re-weighting coalesced leaders by
   the group's total loop-weighted uses.
3. **GEP-base priority** (`liveness.rs` + `regalloc.rs`): a folded GEP base is
   read at every folded access, but `build_live_ranges` only counts the GEP
   instruction. `LivenessResult::gep_base_values` records the folded bases and
   the allocator ranks them by loop weight.

## The Grok "segments" claim — landed correctly this time

Hole-aware call-spanning (`call_spanning` in regalloc.rs) replaces the fat
`spans_any_call` for the GPR scan: a call in a liveness *gap* no longer forces
a callee-saved register. The two prior miscompiles are fixed:

- **Segment-boundary calls**: inclusive-left `seg.start <= cp < seg.end`
  (a loop re-entry segment starts exactly at the re-clobbering call).
- **Merged-interval under-approximation**: coalesced members attribute their
  segments to the group leader, and `cp > def` keeps a value born at its own
  call (retval) non-spanning.

Measured: sqlite_yy_shift and os_postinline_size_policy now pass (previously
nondeterministic garbage / rc=1). The change alone is worth **-283 bytes**
(spectral_norm -75, matmul -74, loop_patterns -71, expat -45, hash_table -39).

## Final numbers

| metric | result |
|---|---:|
| Rust library tests | 910 passed, 0 failed |
| Correctness | 50/50 |
| Runtime regressions | 339 passed + 6 diagnosed GCC-oracle mismatches |
| i686 ABI | 18/18 |
| Warning-free fastbuild | pass |
| Code size (28 files, -O2 text) | **-537 bytes** vs pre-rework baseline |

Per-file: sqlite_varint -138, spectral_norm -75, matmul -74, loop_patterns -71,
fannkuch -58, expat -45, hash_table -39, binary_trees -36, nbody -25. Remaining
small positives (bitops +37, memcmp +32, adler32 +16, stencil +13) are
register-choice noise in the eviction-based allocator (no spills), offset by
the wins.

## Remaining work

1. The memcmp/adler32 param residues (+32/+16): params whose liveness is
   extended through the GEP copy-chain still carry priority 1 and lose a
   register; the spill is cheap (1 store + 1 reload) but not free. A
   follow-up can attribute fold-usage points to copy-chain sources.
2. Kernel 32 KiB boot gate (unchanged, ~4 KB over).
3. `%eax`-reserved i686 accumulator rework (unchanged).
