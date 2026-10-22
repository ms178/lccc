# FOLLOWUP 2026-10-22 — the corpus's worst kernels, round 1

Working down `tests/benchmark/run_benchmarks.py --compilers lccc,gcc` from the
bottom. Three defects are fixed and verified here; the rest of this document
records what each remaining kernel actually costs and what it needs, so the
next round starts from measurements instead of from a ranking.

Host for every number below: 1 physical Xeon core @ 2.60 GHz (two SMT
threads), gcc 12.2, `-O2 -std=gnu17`, minimum of N interleaved samples.
`run_benchmarks.py` reports paired medians; the `min` figures quoted here come
from `scripts/ab_interleaved.py` on the same binaries.

## Fixed in this round

### `double_reduction` — 268 ms → 119 ms, and now correct

`[VEC-RED] I32 dot product needs SSE4.1 pmulld; ISA denies it`. The refusal was
correct for the code that existed at the time (`VecMulI32x4` *was* `pmulld`,
SSE4.1), but wrong as a permanent policy: SSE2 has `pmuludq`, and a C `int32`
product keeps only the low 32 bits, which are the same bits whether the
operands are read as signed or unsigned. `x86::codegen::intrinsics::
emit_int_mul_i32x4` now lowers the 4-lane product as

    lo = pmuludq(x, y)      ; [x0*y0, hi, x2*y2, hi]
    hi = pmuludq(x>>32, y>>32)
    lo = pshufd(lo, 0x08)   ; [x0*y0, x2*y2, x0*y0, x2*y2]
    hi = pshufd(hi, 0x08)
    r  = punpckldq(hi, lo)  ; [x0*y0, x1*y1, x2*y2, x3*y3]

`punpckldq` (not `por` after a shift) is load-bearing: `pmuludq` writes a full
64-bit product per lane, so the odd dword of each lane holds the *high* half of
an even-lane product. OR-ing that in is correct only for `|product| < 2^31`,
and it is what the first version of this sequence did — `double_reduction`
returned 191509376 instead of 201415680, and `sha256_transform` (whose data
fill also vectorizes through this intrinsic) printed the wrong digest. Both are
clean now; `-msse4.1` / `-march=x86-64-v3` (`pmulld`) and the SSE2 pair agree
on every reproducer from 16 to 4096 elements with negative operands.

The vectorizer's `IrType::I32` reduction row no longer vetoes dot products on
the baseline ISA. `%xmm2` is withheld from the vector register pool only when
the SSE2 lowering is reachable *and* the function contains `VecMulI32x4`, so
SSE4.1+ targets keep all 14 vector registers (`nbody` regression guard).

### `hot_web_loop` — 2.45x → 1.30x

if-conversion had a single arm budget for every diamond: `mispredict_penalty /
2` (8), the break-even arm length for a branch that is 50/50 **and reached once
per call**. A diamond inside a loop pays the speculation on every iteration and
the branch only on the iterations it mispredicts; loop-resident branches are
also the most predictable branches in integer code (guards, monotone
loop-carried state, rare bail-outs). Here the rare arm is `(h & 255) == 0` — 1
iteration in 256 — and its 6 effective instructions were hoisted into a
40-million-trip loop: 4 `cmoveq`, 6 stack reloads, 45 instructions per
iteration against gcc's 16.

`cpu_model::if_convert_arm_budget_in_loop` = `penalty / 5` clamped to [3, 4]
(3 on every tuning row) is the loop-resident prior. 3 is the floor because that
is the conditional-reduction diamond (`if (a[i] > 0) s += a[i]`: Load, Cast,
Add), whose conversion is what lets the vectorizer recover a masked reduction.
`LCCC_IFCONV_ARM_BUDGET=<n|off>` reproduces either behaviour for A/B.

Residual (not fixed): the loop still carries 4 phi copies + one `jmp` per
iteration because the join phi and the in-loop values did not get the same
home, and gcc additionally keeps the four accumulators in caller-saved
registers across `helper()` via `-fipa-ra` (it knows a local function's
clobber set). Closing the rest needs either cross-block copy coalescing for
multi-phi webs or interprocedural register allocation, not a tuning knob.

### Phase-1 register allocation — measured, then withdrawn

A wave of call-spanning ranges that outnumbers the callee-saved pool hands one
register to each winner and slots everyone else, so the only decision left is
*who* wins. The linear scan answers it with source order, and six loop-invariant
cold-path constants defined before the loop beat four accumulators read every
iteration. Ranking that wave by the cost model is worth 5 % on `hot_web_loop`
(108.7 -> 102.9 ms) — and it is not shippable, because it breaks the scan's
ordering invariant. The scan consumes ranges in start order and consults the
ACTIVE set at the incoming range's start; a worklist sorted by cost admits a
range that starts later first, which parks a register on an interval that has
not begun. The earlier range then finds the pool occupied by a range it does
not overlap, `select_evict_victim` sees no legal victim, and it takes a slot.
`tests/regression/check_hot_loop_register_homes.sh` catches exactly that:
loop-spanning values with in-loop reads demoted in a six-register loop.

Restricting the ranking to waves whose ranges all start at the same point is
sound, and it fires on **zero** of the corpus's 67 programs, so the mechanism
was removed rather than left in as a switch that never flips. The intent -- the
cost model decides who keeps the last register -- has to be implemented in the
victim selection, which is order-independent; that is the next step here.

## Corpus after this round

`run_benchmarks.py --compilers lccc,gcc --cflag=-std=gnu17 --reps 7 --warmup 1`
(49/49 correct, geometric mean 0.7620, arithmetic 0.9474). Worst first, with
the median *wall* and *child CPU* time of the lccc binary:

| kernel | ratio | lccc wall | lccc CPU | note |
| --- | ---: | ---: | ---: | --- |
| `hot_web_loop` | 1.324 | 103.9 ms | 102.3 ms | 2.454 before this round; residual is phi copies + `-fipa-ra`, see above |
| `byte_load_reassembly` | 1.284 | 5.9 ms | 5.6 ms | unchanged by this round (A/B: 1.002 vs the pre-round binary); sub-20 ms, unresolved |
| `hash_table` | 1.249 | 12.27 s | 12.27 s | unresolved, see below |
| `expat_xml_scan` | 1.246 | 48.6 ms | 48.2 ms | unchanged by this round (A/B: 1.018); the ratio moved because gcc's own time halved between runs |
| `linux_find_bit_scaled` | 1.235 | 886 ms | 886 ms | two taken branches per scan iteration, see below |
| `zstd_count` | 1.196 | 13.8 ms | 13.4 ms | not analysed |
| `linux_rbtree` | 1.191 | 19.2 ms | 18.9 ms | not analysed |
| `linux_find_bit` | 1.186 | 16.1 ms | 15.7 ms | same code as the scaled arm |
| `arith_loop` | 1.152 | 117 ms | 117 ms | **noise**: the generated `.s` is byte-identical across every arm below (the loop has no branch at all), and repeated A/B of the same binaries spreads 7 % |
| `glibc_memcmp` | 1.148 | 10.1 ms | 9.9 ms | sub-20 ms floor |
| `spectral_norm` | 1.131 | 230 ms | 229 ms | see below |
| `fannkuch` | 1.085 | 2.69 s | 2.69 s | not analysed |

`double_reduction` is 1.061 (118.8 ms vs gcc 111.1 ms) and 1.415 before the
fix — and the pre-fix build of the vectorized loop was *wrong*, so the 1.415
number measured the scalar fallback on the old binary. gcc's own median for
this kernel moved between 111 ms and 210 ms across sessions on this host, so
the ratio is not the useful number here; the lccc time is: 268 ms scalar
(before) → 119 ms vectorized (after). `sha256_transform` is 0.984 and correct (it
was a hard FAIL).

### Block placement priced by branches — measured, and blocked

The dual-layout selection compares the chain order against the pre-chain order
by counting emitted instruction lines, which is blind to the one thing block
placement controls: a terminator whose target is the physically next block emits
no branch at all, and a conditional whose preferred edge is next emits a
NOT-taken `jcc`. `layout_branch_cost` prices the transitions a layout leaves
taken, with the chain builder's own edge weights and its `10^depth` frequency
model, and the two layouts compare on `lines + branch cost`.

It works, and it is not shippable as it stands:

    hash_table           13031 -> 10162 ms   0.780
    byte_load_reassembly     6.01 -> 5.64     0.939
    zstd_count               13.53 -> 13.48   0.996   (0.87 with no size guard)
    geomean over ten kernels                  0.970

The CI codegen gate rejected it on four golden workloads
(`glibc_memcmp` insns +3.5 % and stackmem +17.6 %, `sqlite_varint` stackmem
+9.1 %, `expat_xml_scan` moves +7.4 %, `portable_bit_counts` insns +2.2 %).
The cause is structural: the winner is measured on an **exploratory**
emission of the function, then the function is emitted a second time for
shipping, and the two are not the same code. `glibc_memcmp_bytes` is the
minimal witness — the metric scores both layouts at 23 lines and the branch
term breaks the tie, and the shipped chain emission is then ~29 lines with
three more stack operands. Every win above comes from exactly that kind of
tie, so no tolerance or tie-break rule can separate the good ties from the
bad ones.

The fix is to make the exploratory emission faithful (or to ship the
explored text instead of re-emitting it, which collides with the
`EXPLORATORY_EMISSION` trace-counting contract), and only then to score
layouts. Until then the historical rule stands and `hash_table` keeps its
25 % gap.

### `linux_find_bit` — the fold is worth 3.9 %, the layout is not the cost

The scan loop's two-load-per-word body is one instruction longer than gcc's
because `a[i] & ~b[i]` never folds the load into the `and`:

    lccc   movq (%rdi,%r11,8),%r10 ; movq (%rsi,%r11,8),%r14 ; notq %r14 ; andq %r14,%r10
    gcc    movq (%rsi,%r11,8),%r15 ; notq %r15 ; andq (%rdi,%r11,8),%r15

Hand-folding exactly that in the emitted assembly is worth 3.9 % of the kernel
(874 -> 840 ms at `PASSES=65536`; gcc 714 ms). The fold needs a purpose-built
peephole—not the existing `try_fuse_commuted_adjacent_load`, because the `not`
sits between the load and the consumer—and it is *not* implemented here rather
than risk the 3 903-line liveness-aware pass for 3.9 % on one kernel.

Two other leads were measured and **rejected**, and are recorded so they are
not chased again:

* The 7 % that `CCC_NO_STATIC_CHAIN=1` bought on this kernel is not the scan
  loop's block order. With the new metric the scan loop *is* the fall-through
  body (verified in the `.s`) and the time does not move (878.7 vs 880.5 ms).
* A synthetic microbenchmark said a carried `(index+1)*64` bound counter costs
  25 % against recomputing it from the index. In situ the same rewrite is 4.9 %
  *slower* (916.6 vs 874.1 ms). The synthetic effect is a harness artifact;
  strength reduction is not the problem.

## Diagnosed, not yet fixed

| kernel | ratio | what it actually costs |
| --- | ---: | --- |
| `linux_find_bit_scaled` | 1.24 | Superseded by the measurement above: the block order is now the fall-through one and the time did not move, so the two-branch-per-word scan loop was **not** the cost. What is left is the un-folded load (3.9 %) and, beyond that, an unexplained gap that a hand-spliced build localises: with gcc's `linux_find_next_andnot_bit` inside lccc's `main` the program runs at 683 ms, i.e. faster than gcc's own 705 ms, so the caller and the whole program structure are fine and everything left is inside that one function. lccc still wins the tail: `bsfq` vs gcc's 12-instruction `__ffs` decision tree. |
| `spectral_norm` | 1.13 | gcc carries `i+j` as an induction variable and derives `(i+j)*(i+j+1)` from it, so an iteration is 14 instructions; lccc recomputes `i+j` and `i+j+1` from `j`, 16. The loop is issue-bound (divsd + addsd chain), so 2 instructions ≈ the measured 13 %. Needs cross-iteration redundancy elimination over affine expressions (a loop-PRE for `affine(IV)+k`), which `backedge_pre` does not do today. |
| `hash_table` | 1.25 (12.3 s, the most reliable number in the corpus) | 22 % of it is block placement (`0.780` with the branch-priced metric, see above) but that metric is blocked on exploratory-emission fidelity. The rest is unresolved. The generated code is instruction-for-instruction equivalent to gcc's in `insert`, `lookup` and all three `main` loops, the checksums agree, and the work is identical, yet lccc is 25 % slower on both the insert and the lookup phase (a phase-instrumented copy shows both). The lccc `.s` assembled and linked by gcc shows the same gap, so it is not the linker. Needs a real PMU (absent on this host: no `perf`), or `rdtsc`-based phase counters, before any change is justified. |
| `percpu_ptr_walk`, `tls_seg_access`, `histogram`, `binary_search`, `ring_fifo`, `tce_sum`, `ascii_case_fold` | 0.95–1.05 | Not code at all. In this run they land between 0.95 and 1.05; the first run of the same binaries reported 1.68, 1.32 and 1.25 for three of them. Their medians are 2–16 ms of *wall* while the *child CPU* time is 2.0–2.3 ms for both compilers — the ratio is launch-time quantisation. `percpu_ptr_walk`'s inner loop is also instruction-for-instruction identical to gcc's (`addq %gs:8(%r8),%rdx` … 8 instructions), and its header's documented purpose (the segment-qualified ALU fold) is met. A harness fix — scale `PASSES` so each timed region is ≥ 0.5 s, as `linux_find_bit_scaled` already does — is worth more than any compiler change here. |
| `fannkuch`, `binary_trees`, `strlen_bench`, `mandelbrot`, `loop_patterns`, `zstd_count`, `linux_rbtree` | 1.05–1.20 | Not yet analysed. All are long enough to rank; each needs the same asm-diff plus phase-measurement treatment. |

## Method notes

* This host shares **one** physical core between both CPUs
  (`thread_siblings_list` = 0-1), so any concurrent compile or test halves the
  throughput of a running benchmark. Every number quoted here was taken with
  the box otherwise idle; the corpus runs that overlapped a `cargo build`
  produced 17–33 % coefficients of variation and were discarded.
* `min` of interleaved samples is the statistic used for A/B decisions;
  `run_benchmarks.py`'s paired median is used for the corpus table.
