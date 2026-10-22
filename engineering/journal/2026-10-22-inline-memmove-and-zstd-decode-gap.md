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

## Open lead (start here next time)

`CCC_DEBUG_SROA=1` on that TU splits three 24-byte, two-field allocas in
`ZSTD_decompressSequencesLong_body` / `_Long_default` and **nothing** in
`ZSTD_decompressSequences_default`, yet the latter's assembly contains a
24-byte stack-to-stack aggregate copy per sequence
(`lea 0x128(%rsp),%rdi; lea 0x140(%rsp),%rsi; movdqu ... ; mov ...`) followed
by immediate reloads of the copied fields. That is the `seq_t`/`ZSTD_SequenceState`
pair surviving in memory after the inliner replaced
`ZSTD_decodeSequence(&seq, ...)`: the alloca's address escaped to a call at the
time the pre-inline SROA ran, and the post-inline SROA at
`src/passes/mod.rs:1902` then declined it. GCC promotes the fields to SSA and
keeps them in registers. Re-running the aggregate split (or teaching it to
re-scan allocas whose only escaping uses disappeared with inlining) is the
first thing to try; it is the concrete difference behind the field reloads in
the hot loop, and it is not attempted here.

The decode gap is diagnosed, not fixed. Do not re-run the knob sweep or the
link-order experiment; both are negative above.
