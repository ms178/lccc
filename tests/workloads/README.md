# Pinned end-to-end workloads

These workloads build and execute complete upstream projects selected from the
local `archpkgbuilds` recipes. They complement the small standalone kernels in
`tests/benchmark/programs`; they do not replace focused regressions or claim
that VM wall time is target-hardware evidence.

Each workload must pin source identity and license, use deterministic inputs,
run upstream correctness tests where available, compare complete outputs,
retain raw individual and aggregate measurements, and label evidence honestly.

- [`gzip-1.14/`](gzip-1.14/README.md) — full GNU gzip build, 30-test suite,
  deterministic compression/decompression corpora, local disassembly, and
  randomized paired LCCC/GCC VM screening.
