# Generated-code performance backlog

Keep unimplemented opportunities here only when a reproducible workload or
assembly witness exists. Move completed work to its change review with measured
results; do not turn unmeasured expectations into claims.

## Split-latch diamond coalescing: shape implemented, reachability open

`phi_update_region` admits the load-before-if/latch-copy diamond
`source -> {middle, copy}` with exact predecessor and successor counts. Full
diamonds and arbitrary multi-arm regions remain rejected; this is not
permission for broader coalescing or IV replacement.

**Reachability is measured, not assumed.** The latch copy is a rename of an
already-available value, so the current pipeline schedules it into the same
block as its source definition. Every C shape probed (store arm, call arm,
two-way merge) produced `source block == copy block` and took the pre-existing
same-block path. The diamond is therefore covered by unit tests that build the
CFG directly and by no execution fixture, and **no runtime gain is claimed for
it in this tree**.

The originating review reports S81/S80 **0.9774543992** (CI95
[0.9719785956, 0.9790335418]) and S81/GCC **0.8503659416** (CI95
[0.8418807346, 0.8571916937]) over 11 output-checked batched rounds, with one
move removed per hash insert. Those numbers come from a pipeline that emitted
the shape and are **not reproduced here**; treat them as the source review's
evidence, not as a property of this commit.

## Unsigned saturating subtract: scalar work closed, extensions still proof-gated

The exact late scalar matcher, x86-64 CMOV lowering, baseline-i686
SBB/NOT/AND lowering, and secondary-cache correctness fix are implemented.
The [S62–S70 review](../engineering/FOLLOWUP-2026-10-29-saturating-sub.md)
contains the legality argument, negative controls, and complete receipts.
On the final rebased compiler, same-compiler `satsub` on/off measurement of
`zlib_ng_slide_hash` gives median ratios **0.9010041577** for scalar x86-64
(31 paired process rounds) and **0.6766923942** for i686 (21 rounds), with
stdout checked in every run. Callgrind Ir ratios are 0.8953274073 and
0.8941186746 respectively; these simulated counts are not PMU or timing data.

This is not an open request to broaden the pattern indiscriminately:

- New compare/subtract/select forms still need exact operand identity,
  single-use rules, and the signed/narrow nonnegative-domain proof.
- SSE2 and AVX2 objects for this workload are byte-identical with the late
  scalar pass on/off. Do not manufacture a timing result from identical
  binaries; a new vector-tail workload is required for further work there.
- Packed saturation remains x86-64/U16. Do not open i686 packed lowering by
  merely widening a gate, and do not restore the slower x86-64 mask chain.
- Other generic RCX-staging emitters require individual cache-lifetime audits
  and direct regressions, not a mass edit based on textual similarity.

The historical whole-candidate S66 table and the explanation of which packed
parts are now upstream are kept in the review rather than reused as claims
about only the remaining scalar patch.

## Remaining gaps after the fresh worst-ten investigation

[The 56-workload screen and 31-round follow-up](../engineering/evidence/worst-ten-runtime-2026-10-25/README.md)
measure real nbody/hot-loop gains, but do not close the other eight gaps. Retain
canonical sources, sizes and flags; preserve a before compiler; use randomized
paired timings, output checks and bounded Callgrind. Never attribute a timing
fluctuation in a byte-identical executable to optimization.

- **gzip_longest_match / byte_load_reassembly:** final LCCC/Clang ratios
  1.3225 / 1.2588. Baseline bounded Ir ratios are 1.5459 / 2.1439. Investigate
  redundant address/extract/extension work and allocator traffic using emitted
  loop assembly. gzip's helper is already inlined; byte-load induction already
  widened. Acceptance: a generic sound lowering change, negative width/alias
  witnesses, native paired gain and no full-corpus codegen regression.
- **expat_xml_scan / chacha20_block:** 1.2569 / 1.1947 versus Clang. Separate
  loop instructions, register pressure and scheduling; chacha's baseline
  simulated Ir is already lower than Clang, so count reduction is not enough.
  No fast-math/undefined-behavior assumptions or benchmark-name special cases.
- **switch_dispatch / sqlite_varint / llvm_scalar_facts / binary_trees:**
  1.1664 / 1.1167 / 1.0946 / 1.0806 versus GCC. Start from retained profiles;
  establish a specific legal IR/assembly witness before changing global
  heuristics. Preserve switch and pointer/provenance correctness boundaries.
- **nbody / hot_web_loop:** remaining Clang gaps 1.0484 / 1.1013. The scalar
  SQRT and placement fixes are implemented, not open tasks. Further LICM needs
  must-execute/zero-trip/alias proofs. A wider stack-veto relaxation needs
  stronger spill evidence and broad corpus protection. Return-copy/widened-IV
  variants were measured without a convincing win; do not replay them blindly.

## Population-count and CRC follow-through

The population-count loop recognizer and range-aware CRC index lowering are
implemented. Their legality contracts and regression commands are recorded in
[the optimization review](POPCOUNT-CRC-REVIEW.md). Remaining performance work
must distinguish code shape from measured runtime:

- Measure sparse, dense, and zero population-count workloads separately, with
  explicit target features and tuning. A native instruction removes the
  per-set-bit loop, but does not by itself establish an end-to-end speedup.
- Compare CRC witnesses using identical inputs and flags; retain the emitted
  assembly, compiler identities, text sizes, and timing distributions. Relevant
  workloads are `tests/benchmark/programs/gzip_crc32.c` and
  `tests/benchmark/programs/crc32_nibble_hot.c`.
- Keep the established exploratory block-layout policy at `-O1` unless a
  separately measured compile-time/code-size tradeoff justifies changing it.

## Block placement priced from the CFG (round 2)

`layout_branch_cost` now prices the block permutation from the CFG and is gated
by a spill-traffic band; measured in
[round 2](FOLLOWUP-2026-10-22-round2-layout-branch-cost.md) at −18.0 % on
`hash_table`, −16.1 % on `linux_rbtree` and geomean 0.9947 over 67 programs.
The remaining corpus work is inventoried there with per-item asm witnesses:

1. Load-operand commutation in commutative ALU selection (`linux_find_bit`,
   round-1 headroom 3.9 %). `load_op_fuse.rs` folds a load into an ALU only when
   the load feeds the *source*; `a[i] & ~b[i]` selects `a[i]` as the destination,
   so the fold never fires and the loop carries one extra `mov`. Swapping the
   operands does not help — the destination register is already `a[i]`'s home, so
   the fix has to be a destination hint or a `not`-into-destination pattern.
2. Switch lowering for constant results: value table (gcc) instead of a jump
   table plus callee-saves (`pushq %rbx`). `switch_dispatch.c` is 1.0000 today,
   so this must be measured as *latency*, not throughput, before it is claimed.
3. Affine loop-PRE for `IV + k` / `IV + k + 1` (`spectral_norm`, 13 %).
4. Scalar loop unrolling before `spectral_norm`/`mandelbrot`/`fannkuch` are
   worth touching at all.

Two items are recorded as **closed**, so they are not re-litigated:
`sdiv`-by-constant already emits a 64-bit magic multiply with a `sar` and is
within noise of gcc in `i686_alu_chains` (`sdr7` 4.39 vs 4.43 ns); and the
`byte_load_reassembly` hot loop *does* fold its byte loads — the residual 20 %
is gcc's SSE2 vectorisation of the 32-byte block, not load reassembly.

## LLVM missed-optimization campaign: what batch 2 fixed, and what it exposed

Batch 2 (forwarding-phi copy coalescing, 32-bit accumulator round trips) is
recorded in [its follow-up](FOLLOWUP-2026-10-22-llvm-batch2.md). Measured with
a kill switch over the 67-program corpus: geomean 0.9757, with `linux_rbtree`
0.839, `arith_loop` 0.880 and `hot_web_loop` 0.953.

New items, each with a witness, on top of the list above:

1. **`expat_xml_scan` +5.9 % (46.5 → 49.3 ms)** caused by the phi fold changing
   the rotation the scan loop gets; it carries one extra bounds compare. This
   is the first thing to fix, not a tolerable cost.
2. **Loop-carried copy coalescing in the register allocator.** After the IR hop
   is gone, `hot_web_loop`'s arms still copy results into the phi homes instead
   of computing into them (`movq %r9,%rbp` after a `leaq`), because the source
   and destination ranges of a loop-carried copy are allocated separately. That
   is 4 of the 24 instructions left against gcc's 17.
3. **Scaled-LEA composition**: `leaq 0(,%r14,8),%r9` + `leaq (%r8,%r9,1),%rdi`
   → `leaq (%r8,%r14,8),%rdi`. One corpus site, inside `hot_web_loop`'s hot
   loop.
4. **`andl $255, %r10` self-mask** where gcc emits `testb %al,%al`
   (`fold_copy_and_mask_into_movz` handles the `movl`+`andl` pair, not the
   in-place mask).

Status 2026-10-25 (read from source; no runtime claim):

* Item 3 is **closed**.  `fold_scaled_index_temp_into_mem`
  (`src/backend/x86/codegen/peephole/passes/relay_and_lea.rs`) composes a
  scaled-index temporary into a `lea` consumer as one `(%B,%I,s)` address —
  the `leaq 0(,%r14,8),%r9` + `leaq (%r8,%r9,1),%rdi` shape — through the same
  `compose_sib` every LEA fold uses, and declines when the composition would
  need a displacement (the 3-component slow LEA).  The unit
  `lea_chain_with_index_producer_composes_not_concatenates` pins that it never
  concatenates into the unencodable two-index form.  Do not re-open it; the
  remaining `hot_web_loop` work is the loop-carried copies below.
* The measured levers that are actually left are the allocator-side ones in
  [`FOLLOWUP-2026-10-23-remaining-codegen-gap.md`](FOLLOWUP-2026-10-23-remaining-codegen-gap.md):
  cross-web zero-extending copy coalescing (76 % of `zstd_count`'s instruction
  gap), the loop-carried spill, and re-homing the `%cl` shift count into `%rcx`
  instead of copying into it.  All three are interference/coalescing work in
  `src/backend/regalloc.rs`; none is a peephole, and the document's "what this
  rules out" section explains why more peephole copies cannot substitute.

## Callgrind re-baseline after the sibling-call conversion (TODO-9 + TODO-3)

`scripts/callgrind_ab.py`, simulated Ir (not wall-clock), correctness-gated on
identical stdout, 2026-10-23, fastbuild binaries on this host. Both arms of
the sibling-call A/B used the SAME compiler binary, differing only by
`CCC_NO_SIBCALL` (the conversion on vs off):

* **Sibling-call conversion (on/off): neutral-to-positive, never slower.**
  Geomean 0.9999985 over the fast corpus; every arm at or below 1.00000;
  `glibc_memcmp` 0.99995. Output byte-identical on every program. The
  conversion's benefit class (wrapper/forwarder code) is thin in the fast
  corpus; the §5.3 shape evidence (dmin2: call/ret pair + return-address
  traffic removed, gcc-16.2 parity on the emitted shape) carries the claim.
* **lccc vs gcc-16.2 standing (same run, second A/B):** instruction parity on
  `gzip_crc32` (1.00000) and `qsort` (1.00032); ahead on `sha256_transform`
  (0.95), `tce_sum` (1.00), `struct_copy`/`bitops`/`double_reduction`
  (0.73–0.75), `chacha20_block` (0.62); behind on `ring_fifo` (2.49),
  `binary_search` (1.40), `switch_dispatch` (1.28), `linux_rbtree` (1.27),
  `loop_patterns` (1.32), `zstd_count` (1.17), `linux_find_bit` (1.18).
  The two 0.00013 rows (`ackermann`, `constant_recursion`) are the IPA
  devirt/constprop machinery eliminating recursion GCC keeps: 1.0e9 vs
  1.3e5 Ir. Those are real effects of the invariant-callback pass, not
  measurement artifacts — the same binaries' stdout gates the comparison.
  Remaining-gap ownership: regalloc/callocation quality (the §5.1
  post-TODO-2 residuals), not call shape.

## x86 SIMD: measured gaps

Context and contracts: `x86-isa-permissions-and-simd-abi.md`. Ir figures
are Callgrind counts (modeled, not hardware).

* **i686 GNU vector arithmetic is scalarized.** `driver/pipeline.rs` sets
  the vector-lowering width to 0 for non-x86-64, and the i686 backend
  implements about 12 of the roughly 202 `Vec*` families. Add-instruction
  counts at `-O2 -mavx2`, m32 vs m64: char32B 173/5, short32B 93/5,
  int32B 53/5, ll32B 65/5, int16B 33/4, ll16B 37/4, float32B 81/5,
  double32B 45/5. GCC needs 3 at m32. The fix is porting the families,
  ideally by sharing the x86-64 emitters. Opening the gate first would
  miscompile.
* **i686 vector arguments** are copied through memory more than once, and
  the i686 sret path copies with `rep movsb`. Both create store-forwarding
  stalls on wide values.
* **MMX on x86-64** (`_mm_add_pi16` and friends) is lowered through scalar
  code instead of one SSE2 instruction on the low half of an XMM register.
  The F2 `movq` round trip and the redundant `movl %edi,%edi` zero-extension
  are part of the same work.
* **MMX API gaps:** the `_m_*` aliases, `_mm_s*_pi*` shifts,
  `_mm_setr_pi8`, the SSE2 `_si64` forms and the SSSE3 `_pi` forms are not
  provided yet.
* **Ir gaps to GCC 16.2:** reduction_vecreg 3278 vs 1721, popcount 5801
  vs 4728, nbody 3385 vs 2965, switch_dispatch 978 vs 766, zstd
  `double_fast` 2823.5 vs 2686.2.
* **i686 aggregate/32-byte stack ABI (oracle table, 2026-10-30).** The
  vector-extension m32 differential (`check_vector_ext_codegen.sh`, first
  -m32 run ever) exposed the exact position rule GCC 16.2 `-m32` uses for
  stack structs whose TYPE carries a vector-member alignment, measured from
  callee `%ebp` reads with the sret pointer at `ebp+8`:
  `struct { double d[2]; } __attribute__((aligned(16)))` -> `ebp+12`
  (attribute alignment stays 4-granular: compat mode), `struct { v16i;
  int; }` -> `ebp+24` (natural 16 honoured across the sret pointer),
  `struct { v32f; }` -> `ebp+40` (natural 32 honoured), raw `v32f` argument
  -> `ebp+8` with caller-side 32-byte alignment (already correct: the
  `[Memory, NoClass]` marker path).  LCCC packs every struct 4-granular, so
  `sadd`, `s4i_add`, `cf_wrap_phi` (and, through other classes,
  `sh_vs_shr_ul`/`st_v32l`) mismatch the GCC-built driver — the gate names
  them in the closed `LCCC_M32_KNOWN_ABI_GAPS` list in `vector_ext/driver.c`
  (printed as `KNOWN-ABI`, any other divergence still fails; the list must
  shrink to empty when the fix lands).  The fix direction: honour
  `align_ctx` on i686 stack struct arguments exactly when the type
  `contains_gnu_vector && !has_user_align` (both predicates already exist on
  `CType` in `common/types.rs`), keeping the 4-granular cap for
  attribute-aligned and ordinary records — one placement rule shared by the
  outgoing (`I686Codegen::stack_arg_align`) and incoming (prologue param
  capture) paths, oracle-checked against the table above before landing.
* **SSE2-baseline vector value reuse (hot-instruction evidence,
  2026-10-30).** `hotdiff --bench double_reduction` attributes the 1.104 Ir
  ratio to the legacy (non-VEX) path re-reading images from slots per use:
  `movdqu +167,772,160`, `pshufd +134,217,728` versus GCC 16.2 (which folds
  the multi-use loads into `pmuludq`-mem and selects odd lanes with
  `psrlq`+`punpckldq`).  `zlib_ng_adler32` (1.036) attributes to the byte
  loop's structure: `lea +39,852,227`, `mov +12,776,255`, `sub +12,582,921`
  — its 8x-unrolled body computes the a/b chains through `lea`+register
  moves where GCC uses straight `addl` chains with one branch per 8 bytes.
  Both are register/callocation quality (the §5.1 ownership note), measured
  with `scripts/hotdiff.py`; fix candidates: legacy-path value caching for
  the SSE2 emitter and lea→add/mov fusion in the unrolled byte loop.
* **Unsigned-exit reductions** (`for (size_t i...) s += x[i]`) stay scalar
  under the Slt/Sgt restriction of the reduction recognizer. A census of 74
  benchmark programs at `-O2` and `-O3 -march=x86-64-v3` found no
  occurrence, so the restriction stays.
