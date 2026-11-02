# Oracle and benchmark gate policy

Live Compiler Explorer and oracle-delta surveys were retired from PR merge
gates at the owner's request. Network availability is not a merge prerequisite.
Do not reintroduce them as hidden dependencies or call offline tests equivalent
GCC/Clang/ICX execution coverage.

- `scripts/oracle_delta_gate.py` with `--self-test` and
  `scripts/test_oracle_delta_gate.py` test controller policy, not external codegen.
- `scripts/check_benchmark_outputs.sh` and
  `tests/correctness/run_correctness.py` check local outputs and correctness.
- `scripts/codegen_oracle.py` remains an explicitly invoked research tool.

The mandatory benchmark lane remains unchanged. Its strict mode catches
compile/run/output failures, selected failure warnings and unavailable requested
compilers; it does **not** fail on a measured speed-ratio threshold. Mandatory
execution still adds latency and infrastructure/timeout risk.

## Observe before changing the gate

After 20 merged PRs following PR834, run:

```
python3 scripts/collect_ci_followup.py --after-pr 834 --out ci-observations.json
```

Use a read-only `GITHUB_TOKEN` if public rate limits are insufficient. The
collector records the first 20 merged-PR cohort, API head SHA, all CI attempts,
job durations, conclusions and URLs. Exit 2 and `complete: false` mean the
cohort or its run observations are incomplete; they are not a 20-run dataset.
An API head SHA is not automatically the checkout's tested synthetic merge SHA;
check artifact source receipts and workflow versions before interpreting trends.

A failed attempt followed by a successful rerun is only a recovery signal.
Diagnose logs and root causes (runner/network/tool provisioning, timeout,
correctness failure, input/environment differences) before calling it a flake
or changing the gate. Report queue-inclusive and job execution durations
separately; workflow updated-at intervals are not pure compiler time. This
collector does not weaken gates, install a timing threshold, or manufacture
observations that have not occurred.
