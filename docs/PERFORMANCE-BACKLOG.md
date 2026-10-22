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

## Block placement and the LLVM missed-optimization campaign, batch 2

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

## 2026-10-22: workload timing-floor calibration

The new match-search/list-sort workloads previously ran in approximately
1.6 ms (`gzip_longest_match`), 7.1 ms (`linux_list_sort`), and 12–16 ms
(`zstd_double_fast`) with the local GCC 16.2/Clang 23.1.3 builds. Those are
process-wall measurements, not isolated kernel timings. Default `PASSES` is
now 8192, 128, and 512 respectively; bounded Callgrind sizes still explicitly
select `PASSES=1` and are unchanged.

A pinned, randomized-order, 15-round run of the repository benchmark runner
(with two excluded warmups per arm) produced:

| workload | LCCC median | GCC 16.2 median | Clang 23.1.3 median |
|---|---:|---:|---:|
| gzip_longest_match | 103.91 ms | 87.88 ms | 79.98 ms |
| linux_list_sort | 268.63 ms | 180.17 ms | 180.62 ms |
| zstd_double_fast | 235.94 ms | 239.23 ms | 231.25 ms |

All nine compiler/workload results agreed. The original startup-floor problem
is removed on this host: even the fastest arm now exceeds 50 ms. **These are
not performance-acceptance results:** compiler builds and CI shared this
2-vCPU host, and per-arm coefficients of variation remained 11.6–25.6%.
Retain every sample and rerun on a quiet host before making speedup claims.
GCC ASan/UBSan runs also completed with the longer default workloads.

Reproduce (no ICX substitution is implied when that compiler is unavailable):

```sh
python3 tests/benchmark/run_benchmarks.py \
  --only gzip_longest_match,linux_list_sort,zstd_double_fast \
  --compilers lccc,gcc,clang --gcc gcc-16 --clang clang-23 \
  --reps 15 --strict --no-pmu-probe --json /tmp/workload-timings.json
```
