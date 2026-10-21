# Dual-layout metric peepholes: parallel evaluation

## Problem

At `-O1`, static-chain dual-layout exploration emits the chain and original
block orders and runs the full x86 text peephole on each before choosing the
smaller instruction count. The candidates are independent fragments, but they
were evaluated serially, so every explored function paid two complete pass
mixes back to back on one core. This is part of MS-20's measured compile-time
hotspot. Replacing the peephole with a raw instruction count is not valid:
`zstd_count` reverses its preferred order only after the peephole, so the
metric must keep seeing post-peephole text. The only available win is to stop
paying for the work twice in series.

## Change and boundaries

`ArchCodegen::peephole_pair_for_metric` keeps a serial default, so every
non-x86 backend is untouched by construction. x86-64 overrides it with
`peephole_optimize_pair_with_config`, which schedules one scoped worker for the
first candidate while the caller evaluates the second: at most one worker per
pair, pairs never nest, results return in argument order, and a worker panic is
resumed on the caller (`resume_unwind`) so the failure is exactly the serial
failure.

The decision is a pure function, `pair_mode_for(override, multi_thread,
combined_bytes)`, separated from the cached environment and host probes so the
whole policy matrix is testable without touching process state — `cargo test`
runs tests concurrently, and an environment-driven policy test would be
order-dependent. `available_parallelism` (a `sysconf` plus cgroup read) and the
knob are each resolved once per process.

Instrumentation, because the split is invisible in its own output:

- `LCCC_PEEPHOLE_PAIR_PARALLEL` — `0`/`off` never spawns, `1`/`on` always
  spawns on a multi-thread host (this is what makes the concurrent branch
  reachable on a single machine), unset/`auto`/unrecognised keeps the
  heuristic. A typo degrades to the default rather than failing a build.
- `[pair] chain=…B orig=…B mode=… parallel=… serial=… deduped=…` under
  `CCC_TIME_PEEPHOLE=1`, plus `pair_counters()` for unit tests. Counters are
  process-global, so tests assert deltas only.
- `CCC_DUMP_ASM` / `CCC_PEEPHOLE_TRACE` suffix their files `-chain`/`-orig`.
  Both candidates previously derived the same filename, so the second
  overwrote the first and — once the pair ran concurrently — the survivor
  became nondeterministic on exactly the functions people bisect miscompiles
  on. The label is thread-local and restored on unwind; no pass reads it.

Identical candidates skip the second pass entirely: the pipeline is a pure
function of text plus policy, so the result is identical by construction, and
the equality check exits at the first differing byte.

## Why it cannot change output

The peephole is text-in/text-out over a temporary per-function fragment. The
pipeline's shared state is `OnceLock` one-shot caches (`redundant_ext`,
`relay_and_lea`, `liveness`) and no `static mut`, no lock, no `UnsafeCell`;
the only `thread_local` in the peephole is `#[cfg(test)]` (`fp_liveness`'s
refresh counter), so production has no output-affecting thread-local state at
all. `RaConfig` is plain data (`Send + Sync`) captured by shared reference and
never mutated during scoring. Whole-file peephole ordering, candidate scoring,
winner selection, `EXPLORATORY_EMISSION` handling, the re-emission of the
winning order, and every non-x86 backend are unchanged.

## Threshold calibration (measured, not assumed)

On this host (`2` effective threads, `rustc 1.97.0`, `fastbuild`):

- scoped spawn+join: **47-59 µs** (10 000-iteration microbenchmark);
- peephole throughput: **≈0.4 bytes/µs**, so a candidate's work exceeds the
  coupling cost from **≈130 bytes** upward;
- corpus distribution at `-O1`: **34 pairs**, 1 972 B - 16 063 B combined,
  median **5 714 B**.

The original 8 KiB floor rejected ≈70 % of those pairs. Measured with
`scripts/measure_peephole_pair.py --arms serial,auto,forced`, the same session
gives `glibc_memcmp` 0.9696 (8 KiB floor) versus 0.8571 (1 KiB floor) and
`expat_siphash24` 0.8645 versus 0.8493: the conservative floor was leaving
11.6 and 1.5 percentage points of wall time on the table. The shipped floor is
**1 KiB**, an 8x safety margin over the measured crossover that keeps
degenerate two-block stubs off the scheduler while admitting every pair the
corpus produces.

## Evidence

`python3 scripts/measure_peephole_pair.py --arms serial,auto,forced --iters 12`
— interleaved A/B/C, three discarded warm-up rounds, median of paired ratios,
seeded percentile-bootstrap 95 % interval, assembly hashed on every run:

| source | serial (median) | auto / serial | ratio [95 % CI] | pairs split |
|---|---|---|---|---|
| expat_siphash24.c | 308.69 ms | 263.51 ms | 0.8493 [0.8113, 0.8715] | 2/2 |
| glibc_memcmp.c | 77.48 ms | 68.06 ms | 0.8571 [0.8143, 0.9410] | 3/3 |
| zstd_count.c | 71.21 ms | 60.08 ms | 0.8500 [0.8301, 0.9221] | 1/1 |
| linux_rbtree.c | 130.17 ms | 111.49 ms | 0.8046 [0.7752, 0.8931] | 1/1 |
| strlen_bench.c | 91.06 ms | 75.69 ms | 0.8403 [0.8285, 0.8785] | 1/1 |
| sqlite_varint.c | 123.52 ms | 103.57 ms | 0.8329 [0.8144, 0.8553] | 2/2 |

Every interval excludes 1.0; the six medians average **0.839** (≈16 % lower
wall time). The forced arm is statistically indistinguishable from `auto`
(the floor costs nothing measurable on this corpus), and every emitted
assembly hash is identical across all three arms — the script fails the run if
that ever stops holding.

Raw records: [`evidence/2026-10-21-dual-layout-pair/`](../evidence/2026-10-21-dual-layout-pair/)
(`corpus-1kib.json`, `threshold.json`, `measurement.json`, and the gate's
`gate.json`). This is a bounded compile-latency result, not a generated-code
runtime claim: the assembly is byte-identical, so the produced code cannot be
faster or slower.

## Tests

Seven unit tests in `peephole::passes::tests` (all run on any host, including
single-CPU runners):

- `pair_mode_matrix_honours_gate_override_and_host` — the full policy matrix,
  both sides of the size gate and the single-thread fallback;
- `run_pair_runs_both_jobs_once_and_keeps_argument_order` — executes the real
  `thread::scope` path for both modes;
- `worker_panic_is_resumed_on_the_caller` — the payload, not just the fact of
  failure, survives the boundary;
- `identical_candidates_are_peepholized_once` — output plus the dedupe counter;
- `dual_layout_pair_is_byte_identical_to_serial_metrics` — end to end through
  the production entry point in both size modes, with observably distinct
  candidates so a swapped or duplicated result fails;
- `metric_candidate_label_is_scoped_and_restored_on_unwind`, and
  `candidate_path_keeps_both_candidates_of_a_pair` — the instrument contract.

`tests/regression/check_peephole_pair_parallel.py` is the hosted/local gate: it
requires identical assembly across `=0`/unset/`=1`, honours the kill switch,
requires the forced arm to stay serial on a single-thread host (mirroring
`available_parallelism` from affinity and cgroup quota, so a quota-limited
runner cannot flake), requires it never to fall back on a multi-thread host,
and fails if no pair is measured at all so the gate cannot pass vacuously.

## Landing on current main

This change was authored against `91bbcda8`; main had already landed the same
reconciliations in derived form, so those hunks were deliberately not
re-applied rather than force-fitted: benchmark cardinalities now derive from
the registry (`a9337cb0`), the integer-batch shape check lives in
`tests/regression/check_llvm_integer_observations.py` with a stricter
structural `not_mask` witness, and `scripts/audit_census_baseline.json` is
already reconciled (230 / 512 / 835 / 279). Re-applying the literals would have
weakened two gates.

## Open items

- No `make -j$(nproc)` throughput measurement: the split adds at most one
  runnable thread per active compiler, so a saturated many-core build is the
  one place it could plausibly cost more than it saves. `LCCC_PEEPHOLE_PAIR_PARALLEL=0`
  is the measured escape hatch.
- The 1 KiB floor is calibrated on this host; a machine with much dearer
  thread creation (a locked-down container) would want a higher floor. The
  knob makes that measurable rather than a code change.
