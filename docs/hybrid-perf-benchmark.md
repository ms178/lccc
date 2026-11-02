# Local corpus + hybrid-PMU benchmark

`scripts/benchmark_hybrid_perf.py` combines the canonical full registered
`tests/benchmark/run_benchmarks.py` corpus with **separate**, per-task `perf
stat` runs of the exact same retained binaries. It is intentionally opt-in:
profiling is expensive, measures the machine doing the build, and can fail
because of correctness errors or insufficient PMU permissions. This is not a
replacement for whole-application benchmarking.

```sh
python3 scripts/benchmark_hybrid_perf.py \
  --lccc "$PWD/target/fastbuild/lccc" --gcc "$(command -v gcc)" \
  --output "$PWD/report.txt" --artifacts "$PWD/benchmark-evidence"
```

The evidence directory must not already exist. The text report is atomically
updated with an explicit incomplete marker; raw per-round timing and compiler
commands, source hashes, binaries, disassembly, and compile errors are in the
adjacent evidence directory. Preserve the **whole directory and the txt**.
The runner compiles with its default `-O2` and each workload's required flags,
not `-march=native`; it uses GCC's own include directory for LCCC. The profile
used to *build the compiler* does not change those code-generation flags.

## P-cores, E-cores, and perf

On Linux with hybrid PMUs, `cpu_core/cpus` and `cpu_atom/cpus` in
`/sys/bus/event_source/devices` define the P and E CPU sets. Each is
intersected with the current process affinity, and the first allowed logical
CPU in each set is used, or supply `--p-cpu N --e-cpu N`. Invalid overrides,
missing PMU topology, permissions failures, failed differential correctness,
not-counted counters, running percentages below 95%, or unavailable core
classes are reported and return nonzero. Never infer core type from CPU number
or CPU model. On non-hybrid kernels a single generic class is explicitly
reported as **P/E unknown**. A VM is not evidence about Raptor Lake hardware.

The wrapper pins `perf` and its child with `taskset`, then collects per-task,
user-only `cycles` and `instructions` as a group, plus `branches` and
`branch-misses` using the corresponding `cpu_core/` or `cpu_atom/` PMU.
No `-a`/`-C`: those would change the collection to system-wide. Raw
semicolon-delimited perf output, run percentages, return codes, command and
stdout hashes are retained for every randomized paired run. A correctness
mismatch prevents counter comparison. The preflight validates all four events;
missing PMU access is a failure, never a zero count or fabricated result.
The Linux [`perf-stat(1)`](https://man.archlinux.org/man/perf-stat.1.en)
documents both the hybrid naming and CSV fields.

`perf_event_paranoid` and kernel/capability policy govern access. Run as the
ordinary build user with suitable administrator-approved permissions; do not
run `makepkg` as root or change system policy silently. Install Arch's `perf`,
`python`, `gcc`, and `util-linux` packages. Counter sampling is distinct from
the paired wall-clock run and includes process startup. Small sub-200 ms
programs are particularly noisy; compare raw samples and consider the existing
batched runner before drawing conclusions. Pin/record SMT siblings, governor,
turbo, background load, thermal state and microcode when making decisions.
`--reps`, `--perf-reps`, and `--warmup` control sample counts. `--only` is for
surgical debugging, not a full-corpus claim. Each report is per class; no P/E
samples are pooled. Wall-clock aggregate is not a proof of full-workload or
hardware-counter speedup; inspect worst regressions as well as the geomean.

## Evidence quality and hybrid probe

The corpus metadata probe is pinned to the requested CPU and uses its matching
`cpu_core` or `cpu_atom` PMU. An unidentified or overlapping hybrid topology
is not guessed. In particular, `cpu_core/cycles/u` is not an unrecognized
bare `cycles` row, and a probe on a P-core is not evidence for an E-core.
Counter access is checked **before** the expensive corpus. Failures leave an
explicit `FAILED/INCOMPLETE` report; do not weaken `perf_event_paranoid` or run
makepkg as root to hide a failure.

The report includes per-workload PMU paired medians and all raw ratios. The
wall and PMU samples are independent runs. With at least seven PMU pairs, a
paired bootstrap interval is included; fewer pairs have an explicit warning
and no interval. This is a small-sample descriptive interval, not proof that a
difference generalizes across sessions. The default is nine timing pairs and
nine PMU pairs. The workload aggregate is sensitive to short and specialized
cases; the supplementary >=20 ms view is a **sensitivity analysis**, not an
unbiased replacement score. No short tests or outliers are silently discarded.

Where a Git worktree is available, `evidence/tracked-source-diff.patch` saves
the bounded tracked delta against the reported HEAD, with its SHA-256 in the
report. Untracked filenames are disclosed but their contents are **not**
archived; preserve those files and the original source tree separately. When
no Git worktree exists the report explicitly says so. Binary hashes, workload
source hashes, flags, disassembly, topology, and raw measurements remain
necessary; a diff and a hash alone do not reproduce an experiment.

For a two-compiler comparison, execution order is randomized at the start of
each two-round block and reversed in the second round. Nine repetitions thus
balance first/second position within one sample. This limits order imbalance;
it does not eliminate thermal drift or make adjacent rounds independent.
