#!/usr/bin/env python3
"""TEMPORARY diagnostic for PR #817 (delete before merge).

Replays a range of steps from the `test` job of .github/workflows/ci.yml
without stopping at the first failure, emitting one check annotation per
failing step so the verdicts are readable through the GitHub API (runner
logs are not readable from the audit sandbox).

Usage: audit_diag_replay.py FROM TO   (1-based step numbers, inclusive)
Steps with `uses:` are skipped (caches/uploads); the provisioning `run`
steps they precede are self-sufficient and will provision for real.
"""
from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parent.parent
WORKFLOW = ROOT / ".github" / "workflows" / "ci.yml"


def main() -> int:
    lo, hi = int(sys.argv[1]), int(sys.argv[2])
    doc = yaml.safe_load(WORKFLOW.read_text())
    steps = doc["jobs"]["test"]["steps"]
    failures = 0
    for idx, step in enumerate(steps, 1):
        if not (lo <= idx <= hi):
            continue
        name = step.get("name", f"step-{idx}")
        run = step.get("run")
        if not run:
            print(f"SKIP (no run) {idx} {name}", flush=True)
            continue
        env = dict(os.environ)
        for key, value in (step.get("env") or {}).items():
            value = str(value)
            if "${{" in value:
                print(f"SKIP (unexpanded env) {idx} {name}: {key}={value}", flush=True)
                break
            env[key] = value
        else:
            print(f"::group::STEP {idx} {name}", flush=True)
            log = Path(f"/tmp/diag-step-{idx}.log")
            with log.open("w") as fh:
                proc = subprocess.run(
                    ["bash", "--noprofile", "--norc", "-eo", "pipefail", "-c", run],
                    cwd=ROOT, env=env, stdout=fh, stderr=subprocess.STDOUT,
                    timeout=1800,
                )
            print(f"STEP {idx} {name}: rc={proc.returncode}", flush=True)
            if proc.returncode != 0:
                failures += 1
                tail = log.read_text(errors="replace")[-1500:]
                tail = " ".join(tail.split())[:900]
                print(f"::error title=DIAG-STEP-FAIL::step {idx} {name} rc={proc.returncode} tail: {tail}", flush=True)
            print("::endgroup::", flush=True)
            continue
    print(f"::notice title=DIAG2-SUMMARY::steps {lo}-{hi} failures={failures}", flush=True)
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
