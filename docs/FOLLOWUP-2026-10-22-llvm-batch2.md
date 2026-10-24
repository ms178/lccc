# FOLLOWUP 2026-10-22 — LLVM missed-optimization campaign, batch 2

Batch 1 worked on scalar simplification and demand observation (bit demand,
guarded folds, integer observations, zero choice, predicate relations, scalar
facts, scalar logic, word counts). Batch 2 turns to **memory shape and control
flow**, where the corpus's remaining gaps actually are. This document records
what was built, what it measured, and what it exposed.

Host for every number below: 1 physical Xeon core @ 2.60 GHz (two SMT threads),
gcc 12.2, `-O2 -std=gnu17`, minimum of interleaved samples from
`scripts/ab_interleaved.py`. The box was otherwise idle.

## Built in this batch

### 1. Forwarding-phi copy coalescing (`src/passes/copy_prop.rs`)

A *conditional update* of a loop-carried C variable produces **two** phi levels
for one variable: a join phi at the merge of the if-arms, whose home `s` exists
only to carry the merged value to the loop header, and the header's own phi,
whose home is `d`.  Phi elimination turns each level into copies, so the loop
pays a hop through `s` on every iteration:

```text
arm A:   s = Copy(v1)          the join's phi, one copy per edge
arm B:   s = Copy(v2)
merge J: d = Copy(s)           the header's phi
```

`coalesce_forwarding_phi_copies` renames the definitions of `s` to `d` and
deletes the copy in J. The hop is not cosmetic: `s` stays live across the
merge, and that extra live range is what pushed all four loop-carried variables
of `hot_web_loop.c` out of registers and onto the stack.

The soundness argument is in the pass header; the load-bearing preconditions
are that `s` has exactly one reader, that every definition of `s` is a `Copy`
in a predecessor of J that falls through to J and nowhere else, that no such
predecessor reads `d` *after* its definition of `s`, and that `d` is not
defined in J except by the folded copy. Two of those were found the hard way:

* the single-successor rule was dropped in an intermediate version, and
  `expat_xml_scan` then looped forever — a predecessor with two exits wrote `d`
  on a path that never reaches J;
* folding several copies from one analysis let one entry rewrite a definition
  that another had already moved or deleted (`s2` being the `d1` of a chained
  entry), which produced the same infinite loop. Each fold now re-derives the
  uses, so a chain folds one link per iteration.

Both are covered by unit tests (`forwarding_phi_copy_is_folded_into_the_merge`,
`forwarding_phi_copy_is_refused_on_a_critical_edge`) and by a 67-program
differential that compares exit status *and* stdout against the kill switch
(`CCC_NO_PHI_COALESCE=1`).

**Measured** (kill switch A/B, min of 9 interleaved samples, 67 programs):

| program | off | on | ratio |
| --- | ---: | ---: | ---: |
| `linux_rbtree` | 19.30 ms | 16.18 ms | **0.839** |
| `arith_loop` | 122.56 ms | 107.79 ms | **0.880** |
| `hot_web_loop` | 99.16 ms | 94.53 ms | **0.953** |
| `zlib_ng_adler32` | 41.42 ms | 39.74 ms | 0.960 |
| **geomean, 67 programs** | | | **0.9757** |

`hot_web_loop`'s loop body drops from 40 instructions to 36 and the loop-carried
values stop crossing the back edge through memory. `linux_rbtree` is the
surprise: its red-black rebalancing reads the parent and grandparent after a
conditional update, so it had the same double-hop shape.

**One measured regression: `expat_xml_scan` 46.5 → 49.3 ms (+5.9 %, stable
across 9 samples).** The fold changes which blocks the rotation and layout
passes see, and the resulting scan loop carries one extra bounds compare. It is
recorded in `PERFORMANCE-BACKLOG.md` as the first item for batch 3 rather than
hidden: the corpus geomean is a 2.4 % win and the regression is one kernel, but
it is a real number and it is not explained yet.

### 2. 32-bit accumulator round trips (`dead_writes.rs`)

`fold_accumulator_roundtrip` matched `movq` only. Every value that has to
transit `%rax` — every call result, every return value — takes the *32-bit*
form, so the whole class was frozen out:

```text
call g
movl %eax, %r11d        <- staging copy
sarl $1, %r11d          <- the operation
movl %r11d, %eax        <- copy back
```

now folds to a single `sarl $1, %eax`, which is exactly what gcc emits.
Two width rules keep it exact: a 32-bit staging copy must be followed by a
32-bit operation (the staging copy zero-extends, so a `sarq` would read a value
the in-place form does not compute from `%rax`'s high half), and the copy-back
must match the staging copy's width. Both are asserted, in both directions, by
`thirty_two_bit_accumulator_roundtrip_is_applied_in_place` and
`thirty_two_bit_staging_copy_never_folds_a_sixty_four_bit_op`.

## Verification

* `cargo test --profile fastbuild --lib` — **5709 passed, 0 failed** (5707 + 4
  new: two for the phi fold, two for the round trip).
* `ci-codegen-gate.py` — all golden workloads within tolerance, baseline
  unchanged.
* `check_vector_copy_elimination.sh` — PASS 47 / FAIL 0.
* `check_hot_loop_register_homes.sh` — ok, 6 callee-saved registers, no
  recurrence demoted.
* Differential correctness: all 67 corpus programs built with and without
  `CCC_NO_PHI_COALESCE`, exit status **and stdout** compared — 67/67 identical.
* `cargo fmt --check` clean, zero warnings under `-D warnings`.

## Inventory of what is left, with witnesses

Each item has an asm or measurement witness produced by this build.

1. **`expat_xml_scan`'s new +5.9 %** (above). The scan loop carries one extra
   bounds compare after the fold; the fix is either to make the rotation
   re-test cheaper or to keep the fold from changing the rotation's block
   choice.
2. **`hot_web_loop`'s remaining 24 instructions** against gcc's 17. After this
   batch the four phi hops are gone, but the arms still copy their results into
   the phi homes (`movq %r9,%rbp` …) instead of computing into them: the
   allocator assigns the *source* range of a loop-carried copy separately from
   its destination. That is copy coalescing in the register allocator, not in
   the IR.
3. **`leaq 0(,%r14,8),%r9` + `leaq (%r8,%r9,1),%rdi`** where gcc emits one
   `leaq (%r8,%r14,8)`. One corpus site (`hot_web_loop`), but it is in the hot
   loop; a scaled-LEA composition pattern.
4. **`andq $255,%r10` + `test` where gcc emits `testb %al,%al`.**
   `fold_copy_and_mask_into_movz` handles `movl`+`andl` to `movzbl`, not the
   self-masked `andl $255, %r10`.
5. **Load-operand commutation** (`linux_find_bit`, round-1 headroom 3.9 %):
   `mov / mov / not / and` where gcc emits `mov / not / and mem`. The fold
   exists but only for a load feeding the *source*; here the loaded value is
   the destination. Swapping operands does not help — the destination register
   is already the load's home, so the fix is a destination hint or a
   `not`-into-destination pattern.
6. **`spectral_norm`'s affine loop-PRE**: `backedge_pre` does not eliminate a
   redundancy whose occurrences are `affine(IV)+k` and `affine(IV)+k+1`.
7. **Switch lowering for constant results**: value table (gcc) instead of a
   jump table plus callee-saves. `switch_dispatch.c` is 1.0000 today, so this
   must be measured as *latency*, not throughput, before it is claimed.

Two items stay closed, so they are not re-litigated: `sdiv` by a constant
already emits a 64-bit magic multiply (`sdr7` 4.39 vs 4.43 ns in
`i686_alu_chains`), and `byte_load_reassembly`'s hot loop *does* fold its byte
loads — the residual 20 % is gcc's SSE2 vectorisation of the 32-byte block.

## Layout-metric ties go to the trace-built order

The forwarding-phi fold shrank both layout candidates of
`expat_utf8_name_length` onto exactly 68 ins + 60 br, and the tie fell back to
the append order — which needs a `jmp` to reach the shared loop latch where the
chain order falls into it, i.e. three taken branches per character instead of
two. Ties now go to the chain layout: on a tie the cost model has nothing left
to say, and the trace-built order is the better-founded default because its
fall-throughs were chosen by a greedy walk over the same structural weights the
metric prices.

`CCC_LAYOUT_TIE_ORIG=1` restores the historical append-order fallback for A/B
bisection. Measured on the canonical output-checked paired runner:
`expat_xml_scan` 1.106x faster, 1.226 against GCC 16.2 (was 1.172). Of the 1205
benchmark and regression programs that compile cleanly at `-O2`, exactly two
emit different assembly, and both produce output byte-identical to GCC.

One refinement landed with the factored rule (`adopt_chain_layout`): on a tie
the stack-traffic comparison is **strict** (`chain_stack <= orig_stack`) rather
than the usual 10 % admission band. The band exists because "a wrongly rejected
layout costs the whole placement win"; on a tie there is no placement win left
to protect — the metric has already declined to separate the orders — so the
only evidence that remains is the stack term, and there the honest comparison
admits no unproven memory traffic: no measured win means no extra spills.
