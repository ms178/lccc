# The CI pipeline as four lanes that each ran in a straight line

**2026-10-22 — lane sharding, duplicate removal, and the record that keeps both honest**

Run `39752334266` (the 14-job pipeline on `main`) took **824 s of wall clock
and 3 895 s of runner time**. The gap between those two numbers is the whole
story: the pipeline had already been split into jobs, so the *critical path*
was the longest single job, and four jobs were long because they ran their
gates strictly one after another:

| lane | wall clock | shape |
|---|---:|---|
| fast local contracts | 703 s | one step, 140 sequential gates |
| codegen and runtime contracts | 537 s | 32 sequential steps |
| assembler and linker contracts | 508 s | 69 sequential steps |
| benchmark corpus | 466 s | 49 kernels, one runner |
| frontend and ABI contracts | 400 s | 54 sequential steps |

Splitting a lane is not a matter of cutting the list in half. The first attempt
did exactly that and produced **234 s / 527 s** from a "balanced" 191/191 plan,
because one gate's recorded cost (0.01 s) was a measurement taken in a
workspace whose working directory had been deleted under it; the real gate cost
337 s. The lesson is not "measure more carefully" — it is that a split must be
*derived from a cost table that the splitter refuses to invent*, and that the
result must be *audited against the workflow that runs*.

## What changed

* `scripts/ci_lane_split.py` divides a lane by longest-processing-time first
  over a measured cost table, and refuses to guess a missing cost. Two shapes,
  because the pipeline has two: `--mode steps` (a gate is a YAML step) and
  `--mode shell` (a gate is one logical command inside a single `run:` body).
  The preamble of every shard — checkout, toolchain cache, provisioning,
  artifact download, `Verify exact compiler build` — is copied byte for byte,
  so a shard cannot quietly lose a cache and pay the apt transaction.
* `.github/ci-lane-plan.json` records the assignment: per lane, the gates, the
  cost each was given, and the predicted seconds. It is the reviewable artifact;
  the workflow is what runs.
* `tests/regression/check_ci_lane_plan.py` keeps the two honest. A gate planned
  twice, planned but absent, present but unplanned, a shard whose preamble
  drifted, a shard that left the 5 % balance band, or a *sharded job the plan
  does not name at all* — each is a failure, and `--self-test` proves the rules
  can fail (eight rules, eleven fixtures, including the near-miss shapes).
* The benchmark corpus became a reusable workflow with two shards and a merge
  job. `run_benchmarks.py --merge` refuses an incomplete shard, a duplicated
  kernel, a foreign schema, mismatched settings or a different compiler
  identity, so the published report has exactly the shape a single job
  produced. `tests/corpus/test_benchmark_merge.py` and
  `test_benchmark_shard_plan.py` pin the refusals and the partition.
* Duplicate gates were removed where the same invocation ran in two jobs, and
  `scripts/check_ci_gate_parity.py` learned to resolve `workflow_call` inputs:
  an input resolves caller value → declared default → absent (only when not
  required), unknown or required-unsupplied inputs are hard errors, and a
  `secrets:` edge stays refused because a fork PR has no secrets and
  "parity" with a step that cannot run is not coverage.

## Two defects found by building the audit, not by running CI

**The plan was keyed by step name.** `write_plan` recorded shell-mode shards
under their *step* names ("Verify remaining fast local contracts (shard a of
2)") while every other lane was keyed by job id, and the pruning pass ("drop
lanes the workflow no longer has") then deleted those entries as stale — as
soon as any *other* lane was re-split. The contract shards had been running
139 gates with no recorded partition at all, and nothing failed, because the
only thing that read the plan was the plan. The record is now keyed by job id
with a per-lane `mode` (a file-level mode cannot hold a step lane beside a
shell lane), and `unplanned_shards()` fails on any sibling job group the plan
does not name. The rule is pinned by a fixture.

**The splitter priced a command-keyed table as if it were step-keyed.** In
`--mode steps` it looked up `table[step_name]`, but `ci_gate_costs.py` writes
command text, so every step weighed 0.0 and the "balanced" split it produced
was balanced by accident. It now reads the step name first (a hand-written
per-step table still wins) and otherwise sums the commands the step runs,
without double-counting a measured command embedded in another. What the plan
prints as `measured_s` is what the shard total is computed from, so the record
cannot disagree with itself.

## Verification shape

`python3 scripts/test_ci_lane_split.py` pins the transform on synthetic
workflows (partition, byte-identical preamble, `needs:` rewrite, wrapper
identity in shell mode, budget-comment replacement, per-lane plan keying,
pruning). `tests/regression/check_ci_lane_plan.py --self-test` pins the audit.
Both run in the pipeline (`gate ci-lane-split-self-test`, `gate ci-lane-plan`,
and their hosted mirrors), so the tools that rewrite CI are themselves gated.

## Ledger

* `scripts/audit_census_baseline.json`: `knob_literals` 513 → 514. The growth
  is **not** from this branch: a pristine export of the base commit (`d31e9ae`)
  measures 514, and `knobs_undocumented` is unchanged at 277, i.e. the added
  literal is documented. The ratchet was re-baselined with this note rather
  than left red on `main`; `fns_over_300` tightened 230 → 229 in the same
  write.
* `panic_sites` had grown 835 → 836 via the `.expect("alloca with a slot was
  checked by the caller")` added with the alloca-truncation fix. That is now
  **removed rather than re-baselined**: the caller already looks the slot up,
  so the slot is passed in and the invariant is structural. The metric is back
  at its recorded 835.
* `.github/scripts/ci-bench.py` no longer injects the corpus defaults
  (`--compilers lccc,gcc --reps 9 --warmup 1 --strict`) into a `--merge`
  invocation, where they describe a run that is not happening.

## Next lever, with numbers

After this change the longest lanes are the assembler/linker lane (508 s,
still sequential) and the merged benchmark lane (~390 s). Splitting the
assembler/linker lane is the obvious next ~100 s, and it is *deliberately not
done here*: its 69 steps include cross-toolchain provisioning and QEMU-driven
gates whose hosted per-step costs are not in this workspace (the run artifact
download fails with EOF from here), and a blind split is exactly the failure
the 234/527 s attempt already demonstrated. The measurement tool this PR adds
(`scripts/ci_gate_costs.py`) is what should produce that table first.
