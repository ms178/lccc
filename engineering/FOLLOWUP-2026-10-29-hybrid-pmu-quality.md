# Hybrid PMU quality follow-up

## Evidence and fixes

The user's i7-14700KF report had 63/63 correctness cases per core class and
756/756 successful wrapper perf observations, all four required events at
100% reported running. Its generic metadata probe nonetheless said PMU
unavailable: generic `cycles,instructions` expanded to `cpu_core/.../u` and
`cpu_atom/.../u`, but the parser accepted bare labels only. The E-section
probe also ran unpinned and actually observed the P PMU. The report's `VM
screening` label was a consequence of that bug, not evidence of a VM.

* The generic corpus probe now uses the selected affinity and explicit
  matching sysfs PMU; unknown/incomplete/overlapping topology and counters that
  are missing, zero, non-finite, duplicated, uncounted or below 95% running
  fail closed. Real report-shaped mocked CSV tests cover both PMUs, generic
  systems and negative cases. Unknown PMU no longer implies a detected VM.
* The wrapper checks the true selected PMU before running a lengthy corpus;
  measured pairs are summarized separately per core, with raw rows preserved,
  no incomplete-pair ratios, an explicit small-sample warning, and bootstrap
  interval only with at least seven pairs. Defaults are nine PMU pairs.
* Timed corpus aggregates disclose test-mix/specialization sensitivity and a
  secondary >=20 ms-per-compiler view, retaining all original measurements.
  This filter is descriptive, not a representative workload replacement.
* Evidence includes the bounded tracked Git diff and SHA-256 when available;
  untracked contents and absence of Git are explicitly identified. Perf child
  output and CSV reads are bounded and overflow fails closed.

Focused tests include actual-report-shaped mocked hybrid events, topology and
permission/error cases, synthetic perf transport through the real corpus,
paired-summary failures, output overflow, and a real temporary Git worktree.
Mocks are not hardware validation. Upstream LCCC base for this follow-up is
`2d95c4e394d94664c216d2fef7e20a383d9b5b06`; the prior package patch has been
merged and is not to be applied a second time.

## Open work and evidence boundaries

The earlier local fast CI passed on a different commit, not this one. Run the
current full fast gate and hosted CI; test the new package with real Arch
makepkg and test the patched scripts on the user's P/E hardware. Preserve all
JSON, disassemblies, workload sources, patch and untracked files. Nine paired
rounds and bootstrap intervals do not establish independence or whole-program
performance. Longer inputs, repeated sessions, thermal/SMT isolation, and
assembly audits remain needed for the short and extreme reported cases. The
mixed-GCC include-next defect is not addressed here. The VM provides no usable
hardware PMU, so no performance claim can be validated locally.

The two-compiler timing and perf schedules now choose a random first order and
reverse it in the next round, repeated in two-round blocks. This balances
first/second position to within one sample while retaining random block starts;
three-or-more compiler comparisons remain independently shuffled. Individual
round orders are retained. A failure that removes an active compiler cannot
reintroduce it. This mitigates order bias but does not establish independence
between nearby rounds or across sessions. Direct invocation of the benchmark
harness tests previously skipped the option-parsing test class because
`unittest.main()` preceded it; moving that call to the end makes direct and
module discovery agree (20 tests including the new balanced-order contract).
