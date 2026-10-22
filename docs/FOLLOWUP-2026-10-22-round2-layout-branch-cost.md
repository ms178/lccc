# FOLLOWUP 2026-10-22 — block placement priced by branches, round 2

Round 1 (`FOLLOWUP-2026-10-22-corpus-worst15.md`) measured `hash_table` at 1.25
and localised 22 % of it to block placement, then withdrew the metric it had
built because the metric and the shipped text disagreed. This round removes the
reason for that disagreement instead of the metric.

Host for every number below: 1 physical Xeon core @ 2.60 GHz (two SMT
threads), gcc 12.2, `-O2 -std=gnu17`, minimum of interleaved samples from
`scripts/ab_interleaved.py`. The box was otherwise idle; no measurement below
overlaps a build.

## What the metric scores, and why it is now sound

`lccc` emits a function twice when the chain builder has a candidate order: once
for exploration and once for shipping. Round 1's probe scored the *exploratory
text*, and the exploratory emission is not the shipped one — `glibc_memcmp_bytes`
scored 23 lines for both layouts and shipped ~29 with three more stack operands.
That made every win a coin flip, and the metric was withdrawn.

`layout_branch_cost` (in `src/passes/block_layout.rs`) scores the **CFG**, not
the text:

```rust
pub(crate) fn layout_branch_cost(func: &IrFunction) -> u64
```

It walks the block order and charges one `10^depth`-scaled slot for every block
transition the layout leaves without a fall-through, using the chain builder's
own edge weights — 9 for an in-loop edge, 8 for a deeper edge, 5 for a single
successor, 4 for a straight-line branch, 1 for leaving a loop — and skips
`Return`, `Unreachable` and `Switch` (a switch has no fall-through to price).
The `IrFunction` is **identical** for both layouts; only the permutation
differs. So the metric cannot be measuring a different program than the one
shipped, which is exactly the defect that killed the round-1 probe. It prices
the placement decision and nothing else.

Adoption rule (`src/backend/generation.rs`):

    adopt the chain order  iff  spill_ok && chain_cost < orig_cost
    spill_ok               =    stack(chain) * 10 <= stack(orig) * 11

## The spill band is load-bearing, and it is calibrated, not guessed

Static size is a bad proxy for what a layout costs, and the two kernels that
disagree about size bracket the band:

| kernel | chain order | pre-chain order | size | stack operands | measured |
| --- | --- | --- | --- | --- | ---: |
| `linux_rbtree` | 250 ins + 2701 br | 239 ins + 3900 br | **+11 ins** | 30 vs 28 (**+7.1 %**) | 16.41 vs 19.00 ms → **0.864** |
| `zstd_count` | 131 ins + 2231 br | 130 ins + 2332 br | +1 ins | 11 vs 8 (**+37.5 %**) | 14.28 vs 13.08 ms → **1.092** |

`linux_rbtree`'s chain order is *longer* and 13 % faster; `zstd_count`'s is one
instruction longer and 9 % slower, because the moved block pushes a loop counter
into a slot that is reloaded every iteration. Nothing about instruction counts
separates them. Spill traffic does: a memory operand relative to the frame or
stack pointer is a value a register could have held, and `count_stack_operands`
counts them per architecture (`%rsp`/`%rbp`/`%esp`/`%ebp`, `[sp,`/`[x29,`,
`(sp)`/`(s0)`), ignoring directives, labels, comments and cfI markers. +10 % sits
between the two with margin on both sides and matches the shape of the codegen
gate's own tolerance bands. A false positive can only keep the pre-chain order;
it can never change what the program computes.

With the band, `zstd_count` takes the pre-chain order and is unchanged
(0.993), while `linux_rbtree` takes the chain order and keeps its 13.6 %.

## Corpus-wide A/B

`CCC_NO_LAYOUT_BRANCH_COST=1` (old line-count rule) against the new rule, all 67
`tests/benchmark/programs`, interleaved, min of 3:

| program | off | on | ratio |
| --- | ---: | ---: | ---: |
| `hash_table` | 11953.63 ms | 9799.94 ms | **0.8198** |
| `linux_rbtree` | 19.88 | 16.68 | **0.8389** |
| `binary_trees` | 1471.34 | 1372.21 | **0.9326** |
| `arith_loop` | 120.86 | 113.90 | **0.9424** |
| `llvm_scalar_facts` | 53.12 | 51.35 | 0.9666 |
| `sieve` | 36.71 | 35.65 | 0.9710 |
| `byte_load_reassembly`, `glibc_memcmp`, `linux_find_bit`, `expat_xml_scan`, `zstd_count`, `hot_web_loop`, `fannkuch`, `nbody`, `spectral_norm`, `glibc_strstr`, `chacha20_block`, `sha256_transform`, `popcount_loop_density`, `reduction_vecreg`, `i686_alu_chains`, `switch_dispatch`, `tls_seg_access`, `struct_copy`, `sqlite_varint`, `qsort`, `sieve` … | — | — | 0.97–1.03 |
| **geomean, 67 programs** | | | **0.9947** |

`hash_table` is the headline: 11.95 s → 9.80 s, **−18.0 %**, and the arithmetic
is where round 1 predicted it (`0.780` was the size-priced variant's score on a
12.9 s baseline; the shipped rule gets 0.82 on a 11.95 s baseline, i.e. most of
the headroom that survived the withdrawal).

The only apparent regressions in the sweep were `vecreg_new_ops` 1.1428,
`base64_enc` 1.0471 and `histogram` 1.0388. All three are 2 ms programs where
the harness timer quantises at ~1 ms, and `vecreg_new_ops` takes **no layout
decision at all** (`CCC_DEBUG_CHAIN` prints nothing for it). Re-measured with 11
interleaved samples they read 0.9827, 1.0252 and 0.9836. They are noise, and the
`min`-of-3 corpus sweep is not the instrument for a 2 ms workload — that is a
harness defect recorded in round 1 and still open.

## Verification

* `cargo test --profile fastbuild --lib` — **5707 passed, 0 failed** (5704 + 3
  new: `branch_cost_prefers_the_fall_through_layout`,
  `branch_cost_of_a_straight_line_chain_is_zero`,
  `stack_operands_are_counted_across_backends`).
* `ci-codegen-gate.py --lccc target/fastbuild/lccc` — **all golden workloads
  within tolerance** with the baseline unchanged from `#916`. The metric no
  longer moves `portable_bit_counts` (it did before the spill band, stackmem
  6→7).
* `check_hot_loop_register_homes.sh` — ok, 6 callee-saved registers, no
  recurrence demoted.
* Differential correctness: all 67 corpus programs built both ways, output and
  exit status compared byte for byte — **67/67 identical** (`i686_alu_chains`
  prints microbenchmark latencies, which differ as expected; its exit status and
  structure are identical).
* `cargo fmt --check` clean, `cargo clippy --lib` clean, zero warnings under
  `-D warnings`.

## LLVM missed-optimization campaign — batch 2, scoped with witnesses

Batch 1 (bit demand, guarded folds, integer observations, zero choice,
predicate relations, scalar facts, scalar logic, word counts) worked on scalar
simplification and demand observation. Batch 2 turns to **memory shape and
control flow**, because that is where the remaining corpus gaps actually are.
Each item below has a reproducible asm witness produced by this round's build
against gcc 12.2 at `-O2`; none is claimed as a measured win yet, and per
`PERFORMANCE-BACKLOG.md` an item only becomes a change when it has a timing.

1. **Load-operand commutation in binary ALU selection.** `linux_find_next_andnot_bit`
   selects `mov (%rdi,%r11,8),%r10 / mov (%rsi,%r11,8),%rbx / not %rbx /
   and %rbx,%r10` — four instructions per word where gcc emits three
   (`mov / not / and (%rdi,…),%rdx`). The fold exists (`load_op_fuse.rs`) but
   only fires when the loaded value is the *source*; here it is the destination
   because selection assigned the destination first. Fix: when selecting a
   commutative ALU op with one register and one single-use load operand, prefer
   the register as destination so the load can fold. Witness:
   `tests/benchmark/programs/linux_find_bit.c`, round-1 measured headroom 3.9 %.
2. **Switch lowering for constant results.** `switch(x)` returning a constant per
   case compiles to a jump table plus a frame (`pushq %rbx`, `movq %rdi,%rbx`,
   `.Ljt_0`) in lccc and to a plain value table in gcc (`xorl %eax,%eax / cmpl
   $5 / ja / leaq CSWTCH(%rip) / mov (%rax,%rdi,4),%eax`). The indirect branch is
   both harder to predict and needs callee-saves. Witness: the `sw` probe in
   this round, plus `tests/benchmark/programs/switch_dispatch.c` (currently
   1.0000 — the win is code shape, and must be measured as latency, not
   throughput, before it is claimed).
3. **Affine loop-PRE for `IV + k`.** `spectral_norm` recomputes `i+j` and
   `i+j+1` from `j` (16 instructions/iteration) where gcc carries `i+j` as an
   induction variable and derives `(i+j)*(i+j+1)` (14). `backedge_pre` does not
   eliminate a redundancy whose two occurrences are `affine(IV)+k` and
   `affine(IV)+k+1`. Round-1 measured 13 % for this kernel.
4. **Signed constant division by immediates.** `sdiv37` costs lccc
   `movslq / movl $3714566311,%eax / imulq / sarq $37 / movq / shrq $63 / addq`
   against gcc's `movslq / imulq $-580400985,%rax,%rax / shrq / addl / sarl /
   sarl / subl`. Same count, but lccc's `imulq %rax,%rsi` cannot use the
   immediate form and adds a serialising `movl`. Latency probe
   (`i686_alu_chains`) shows `sdr7 4.39 / 4.43` ns against gcc's — **not**
   currently a defect; recorded so the batch does not re-litigate it.
5. **Byte-load reassembly is done; vectorisation is not.** The round-1 note
   that `byte_load_reassembly`'s hot loop still loads bytes is **wrong for this
   build**: the hot loop already emits single `movl 8/16/24(%r8,%rdi)` loads.
   The residual ~20 % is gcc's SSE2 auto-vectorisation of the 32-byte block (4×
   scalar load/xor/add per iteration in lccc). Witness: `-S` diff of
   `byte_load_reassembly.c`.
6. **`spectral_norm`/`mandelbrot`/`fannkuch` need a scalar loop unroller before
   any of the above matters.** Recorded, not started; they are the corpus's
   longest kernels after `hash_table`.

The batch-2 rule: an item ships only with a `min`-of-N timing on the host core,
a corpus sweep for collateral, and a gate run. Items 1 and 2 are the ones with
both a witness and a known-positive prior.
