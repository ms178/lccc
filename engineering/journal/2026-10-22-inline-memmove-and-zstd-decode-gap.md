# Constant-size memmove inlining; where zstd's decode gap actually lives

## Problem

Two independent defects, found while investigating a user report that zstd
"does not work" when built with lccc:

1. `inline_memcpy_len` expanded a constant-size `memcpy` only up to 32 bytes
   and never expanded `memmove` at all. Every `memmove(d, s, 16)` in a hot
   path therefore became a PLT call, lost its alignment knowledge, and could
   not be folded by the peephole. The 32-byte cap was an artefact of the
   caller's constant-size table, not a policy: the very same lowering in
   `x86/codegen/memory.rs` already had straight-line expansions to 64 bytes,
   including a two-instruction `vmovdqu %ymm` pair at exactly 64.
2. A lccc-built zstd decodes Huffman/FSE-compressed data at roughly half the
   speed of a GCC-built zstd (numbers below). That is a *codegen* gap in one
   translation unit, not a correctness defect — every round trip, cross
   decompression, dictionary and fuzz leg is identical to GCC.

## Change and boundaries

`INLINE_COPY_MAX` 32 -> 64 for `memcpy`; new `INLINE_MEMMOVE_MAX = 32` and a
`memmove` path. `inline_memcpy_len` now admits `memmove`/`__memmove_chk`
alongside `memcpy`/`__memcpy_chk`, with the same destlen check, and the
inline site dispatches on the libcall name.

`memmove` needs overlap-safety, which a forward `movdqu` sequence does not
have. Sizes that fit the published scratch set are handled by *buffering*:
every chunk is loaded into a scratch register before the first store, so the
copy is correct for any overlap, for `dst == src`, and without a branch or
any `std`/`cld` traffic. The scratch set is exactly the one the rest of the
backend already assumes for an inline libcall (`%xmm0`/`%xmm1`, or
`%rax`/`%rcx` under `-mno-sse`) — the surface `regalloc.rs`'s parameter-home
and ASpill exemptions, the `prologue.rs` frame scan and the `emit.rs`
MachInst gate are all written against. Chunking never rounds up, so no load
or store leaves `[d, d+n)`. `-mno-sse` sizes above 16 bytes cannot stay in
that set (two 8-byte pieces do not reach 32) and take a `rep movsb` fallback.

That fallback was wrong, and untested, because nothing could reach it: it
moved only the count into `%rcx` before the backward copy, so `dst > src`
copied from the wrong addresses, and its backward start pointed one byte
past the source end. Both pointers now move onto the last byte of their range
with `DF=1`, and the `-mno-sse` fixture pins it (44 `std` sites in the
generated assembly of that fixture).

The return value is threaded through: `emit_inline_memmove_call` takes the
result value like the memcpy path and leaves `dest` in `%rdx`, so
`p = memmove(...)` is exact.

## Measurements

Pinned (`taskset -c 0`), in-process, interleaved round-robin, max of N
(zstd's own `-b -d`, which is the only decode number in this tree that is not
dominated by process start-up). Host: 2 vCPU Xeon 2.6 GHz, AVX2 + BMI2.

| file        | lccc before | lccc after | GCC 16.2 |
|-------------|------------:|-----------:|---------:|
| access.log  |       809.8 |      829.5 |   1568.2 |
| data.json   |       793.5 |      794.1 |   1493.3 |
| table.csv   |       478.8 |      476.6 |    752.8 |
| source_c.txt|       732.6 |      735.7 |   1129.6 |

The memmove/memcpy widening is worth a few percent on literal-heavy decode
and is dwarfed by the gap that remains. Attribution of that gap, using an
`LD_PRELOAD` `SIGPROF` sampler (no `perf` in this image):

* Replace ONE object — `lib/decompress/zstd_decompress_block.o` — with the
  GCC-built one and relink with lccc everywhere else: access.log
  807 -> 1448 MB/s, data.json 786 -> 1333, table.csv 473 -> 718 (1.8x).
  No other single object moves any of these by more than 3 %. So the entire
  gap is in that one TU, and it is not a whole-binary layout effect.
* 87.5 % of samples land in `ZSTD_decompressSequences_default` (the plain,
  not the `Long`, variant) with lccc; the same function inlined into
  `ZSTD_decompressBlock_internal` accounts for 78.7 % with the GCC object.
  The hot cluster is the literal-copy block around `ZSTD_wildcopy`.
* `-O2` for that TU is as slow as `-O3` (795 vs 796 MB/s on access.log), and
  NO single pass or allocator kill switch recovers anything: `NO_LOOP_ROTATE`
  803, `NO_INVAR_HOIST` 798, `NO_PEEPHOLE_RERUN` 792, `NO_REDUNDANT_EXT` 785,
  `NO_CFG_COPY_COALESCE` 798, `NO_ENTRY_COPY_COALESCE` 792, `NO_SPLIT_RANGES`
  779, `RA_GLOBAL_LOCATION=0` 779; `CCC_PRESSURE_BUDGET` 14 and 20 are within
  noise of the default.
  `CCC_DEBUG_SPLIT` reports ZERO pressure/call/loop splits in that TU, so the
  earlier "pressure-splitter cliff" hypothesis is falsified.
* It is NOT code layout: relinking the identical objects in four different
  link orders (reversed, and three rotations) moves access.log by < 1 %.
* It is NOT the huf FSE table machinery: swapping `huf_decompress.o`,
  `huf_decompress_amd64.o`, `entropy_common.o` or `fse_decompress.o` for GCC
  builds leaves table.csv at 470-480 MB/s (base 472).

What the disassembly shows instead: in the literal-copy loop lccc reloads the
source and destination pointers from stack slots on every iteration
(`mov 0xf8(%rsp),%rax` / `mov 0xf0(%rsp),%r9` / `add $0x10` / store back) and
copies 16 bytes at a time, where GCC keeps both pointers in registers and
issues 32-byte `vmovdqu` pairs. Sample density concentrates on the
load/store instructions of that loop (29 % of all samples on one 16-byte
copy store), i.e. the loop is latency/bandwidth bound on lccc's side because of
per-iteration pointer reloads. That is a register-allocation problem in a
loop whose live set is far above the GPR budget — the same shape the
`ZSTD_decompressSequences` loop has always had — and it is left open here.

## Limits

The widening is validated semantically, not by proof: every size 0..32 for
`memmove` (runtime shift -48..48, all four overlap relations, plus a
compile-time-shift matrix), every chunk-ladder boundary for `memcpy` up to
64 bytes, and page-boundary probes at every size 0..64 where source and
destination each end at a `PROT_NONE` page, so a single byte of over-read or
over-store faults. `tests/regression/inline_memmove_const_sizes{,_nosse}.c`;
GCC and Clang are the differential oracles.

## Resolved: the aggregate copy was the gap

The lead below was correct and is now fixed. `aggregate_sroa` gained a fifth
form (`src/passes/aggregate_sroa/byval.rs`) that coalesces a whole-object copy
whose source dies at the copy: `tmp = s` becomes `tmp` *being* `s`.

The two shapes in the IR are complementary, and neither existing form could
express either:

* the copy's destination is the inlined callee's by-value parameter, so it
  escapes to a call — form 1 refuses an escaping destination, and form 4 must
  refuse any object whose address reaches a call;
* the copy's source is named by a `Memcpy`, which form 4 must refuse outright,
  because a block copy is a byte-range operation that independent fields cannot
  express.

The proof that lets the copy disappear is a liveness statement about the
*source*: `s` is a private, exact-size alloca, and no access to it can execute
between the copy and any use of the destination. That has to be answered per
program point, not per block, because the copy sits in a loop:
`[body writes] -> copy -> use` is fine even though the body blocks are reachable
from the copy through the back edge, since that path re-executes the copy first.
`find_interference` does this with a forward walk from the copy and a backward
walk from each use that stops at the copy's block.

Once the copy is gone the object is named by no `Memcpy` and escapes to no call,
so form 4 splits it and mem2reg promotes the fields; a by-value call still needs
a contiguous image, which is rebuilt at the call site from the (now scalar)
fields, gap bytes included, so the callee sees the same padding as before.

Diagnostics for the form: `CCC_AGG_BYVAL_DEBUG=1` reports every candidate it
declines, with the copy's block/instruction position and the reason, plus the
field count of each plan it applies.  It never changes codegen.

Two defects found while landing it, both kept as regression tests:

* Renaming onto a *parameter home* is not sound with this IR's storage model.
  A by-value aggregate parameter's home is the incoming argument area, filled by
  backend ABI lowering rather than by an IR-visible store, and aliasing a local
  onto it made `tests/regression/agg_copy_fwd_partial_rewrite.c` read the struct
  at an 8-byte-wrong frame offset. Both sides now refuse parameter homes, which
  is the same rule form 4 already applies (`func.param_alloca_values`).
* A terminator naming the source object is an observer this pass cannot place
  (`scan`'s escape set only walks instructions), so it refuses outright.

### Measured

`zstd -b2 -d`, same corpus, three binaries, best of two runs. `base` is the
compiler without this change, `new` is with it:

| input | gcc | base | new | base/gcc | new/gcc |
|-------|-----|------|-----|----------|---------|
| 98 MB log | 1801 | 649 | 1486 | 0.360 | **0.825** |
| 5.4 MB json | 2044 | 1283 | 1398 | 0.628 | **0.684** |
| 4.7 MB csv | 583 | 493 | 600 | 0.845 | **1.029** |
| source text | 1540 | 1013 | 1309 | 0.657 | **0.850** |

Wall-clock decode of the same files with the CLI is noisier on this host but
shows the same direction (log 0.51 -> 0.83 of GCC). `ZSTD_decompressSequences_default`
drops from 128 to 19 stack references in the repro shape and keeps all three
sequence fields in registers; the remaining stack traffic in that function is
`seqState_t` (a 112-byte object reached through variable-offset `GEP`s, which is
a different problem) and twelve 16-byte `Loaddqu`/`Storedqu` vector temporaries
that `vector_temp_promotion` leaves in place.

The json input remains at 0.68 of GCC; swapping only
`lib/decompress/zstd_decompress_block.o` for GCC's recovers ~10 % of that, so the
remaining gap is still in that TU, split between `seqState_t` scalar replacement
and the unresolved vector-temporary forwarding above.

## Verification sweep: three IR-contract violations

Adding `verify_after_pass` checkpoints after `inline`, `sret_copy_elision` and
`aggregate_sroa` and running `CCC_VERIFY_IR=abort` over every zstd 1.5.7
translation unit turned up three passes that hand the next pass malformed IR.
All three are pre-existing (none is caused by the by-value copy work above);
the sweep is what made them visible, and all three are fixed.

1. **`sret_copy_elision`: intra-block use-before-def.** The inliner
   materialises a callee-local aggregate at the call site, so an sret call can
   be *followed* by the alloca of the object it will be copied into:

   ```text
   call  ZSTD_decodeSequence(%seq)      # sret, fills %seq
   %d    = alloca 24                    # materialised at the call site
   memcpy %d <- %seq, 24
   load  %d[8]
   ```

   The retarget rewrote the call's hidden pointer to `%d`, defining `%d`
   after its only use (`ZSTD_decompressSequencesSplitLitBuffer_default`
   block 106: `v5308` defined at #3, used at #2). The pass now requires the
   destination alloca to dominate the call and declines otherwise, leaving the
   copy to the read-forwarding phase, which removes it anyway.

   The same audit found the retarget unsound when an instruction between the
   call and the copy touches the destination: the retarget moves the call's
   write *earlier*, so an intervening reader observes the callee's bytes (it
   used to see the object's previous value) and an intervening writer's bytes
   survive the deleted copy (they used to be overwritten). Candidates with such
   an instruction — or with a second retargeted call inside the window — are
   declined.

2. **`vector_temp_promotion::hoist_block_allocas` moved allocas above phis.**
   The hoist exists to keep an alloca above the producer that a promotion
   retargets at it, but it hoisted to index 0 unconditionally, pushing the
   block's phi prefix below an `Alloca`. Allocas now hoist to the end of the
   phi prefix, which still places them above every non-phi instruction.

3. **The inliner under-reserved label space.** `merge_block_id` was derived
   from `callee.max_block_id`, and `caller.next_label` advanced only to
   `merge_block_id + 1`. When nested inlining had pushed the callee's live
   labels past that hint, cloned blocks landed above the merge block and above
   the caller's counter, so the next pass minting a label from `next_label`
   reused a live one (`next_label=3947494193`, live max `3947503941` in
   `ZSTD_compressSequencesAndLiterals`). Both are now reserved past the maximum
   label actually present — the discipline the value-id path already documents
   ("always reserve past the actual maximum present in the IR").

Two doc-comment pseudo-code blocks in the x86 backend that made
`cargo test --doc` fail to compile its own examples were fenced as `text`.

### Measured after the fixes

`zstd -b1 -d` (one second of in-process decoding per run), best of nine
interleaved runs, so scheduling noise biases every binary the same way:

| input | gcc 16.2 | base | new | base/gcc | new/gcc |
|-------|----------|------|-----|----------|---------|
| 98 MB log | 1709 | 711 | 1567 | 0.416 | **0.917** |
| 5.4 MB json | 2218 | 1445 | 1863 | 0.651 | **0.840** |
| 4.7 MB csv | 717 | 516 | 613 | 0.719 | **0.855** |
| source text | 1626 | 1024 | 1342 | 0.630 | **0.825** |

The by-value coalescing plus these fixes are worth 1.41x–2.20x over the
baseline compiler and close 55–95 % of each file's gap to GCC. What is left is
not one shape: `ZSTD_decompressSequencesLong_default` (2517 instructions, 871
frame-relative memory operands, against GCC's 1786/401) is the level-19
hot path and its frame traffic is `seqState_t` plus the sequence ring, not the
sequence object; `ZSTD_safecopy` still keeps its marching pointer in memory
(211/58 against 152/0).
