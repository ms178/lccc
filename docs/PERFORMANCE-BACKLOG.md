# Generated-code performance backlog

Keep unimplemented opportunities here only when a reproducible workload or
assembly witness exists. Move completed work to its change review with measured
results; do not turn unmeasured expectations into claims.

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
