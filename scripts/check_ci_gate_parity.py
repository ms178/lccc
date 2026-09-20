#!/usr/bin/env python3
"""Fail when a standalone ci_local gate is absent from hosted workflows.

The local mirror and GitHub workflows intentionally use different orchestration,
but every directly invoked regression shell/Python program in ci_local.sh must
also be EXECUTED by at least one hosted workflow step.

Execution semantics: a command counts only when it appears inside a `run:`
script body of a workflow (single-line or block scalar). Mentioning a gate in
a comment, a step name, or any non-run YAML field does not run anything and
must not satisfy parity -- a workflow that referenced
`tests/regression/check_x.sh` in prose while the step itself was removed would
otherwise pass this check while silently losing coverage.
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
LOCAL = ROOT / "scripts" / "ci_local.sh"
WORKFLOWS = ROOT / ".github" / "workflows"

COMMAND = re.compile(
    r"(?:tests/regression/[A-Za-z0-9_./-]+\.sh|"
    r"scripts/[A-Za-z0-9_./-]+\.py|"
    r"\.github/scripts/[A-Za-z0-9_./-]+\.py)"
)


def run_script_bodies(path: Path) -> str:
    """All `run:` script text of one workflow, comment-stripped.

    Handles the single-line (`run: make check`) and block scalar
    (`run: |`/`run: >`) forms through the YAML parser, so quoting,
    folding, and comment placement cannot fake an execution. Shell
    comment lines and trailing comments inside the script body are
    removed before matching: a gate commented out of its own run block
    executes nothing and must fail parity.
    """
    try:
        import yaml
    except ImportError:
        print(
            "error: PyYAML is required to verify gate parity with execution "
            "semantics (python3 -m pip install pyyaml)",
            file=sys.stderr,
        )
        raise

    doc = yaml.safe_load(path.read_text())
    if not isinstance(doc, dict):
        return ""
    pieces: list[str] = []
    jobs = doc.get("jobs") or {}
    if isinstance(jobs, dict):
        for job in jobs.values():
            if not isinstance(job, dict):
                continue
            # `steps` is the normal location; composite action files would
            # use `runs.steps`, accepted for future-proofing.
            containers = []
            if isinstance(job.get("steps"), list):
                containers.append(job["steps"])
            runs = job.get("runs") or {}
            if isinstance(runs, dict) and isinstance(runs.get("steps"), list):
                containers.append(runs["steps"])
            for steps in containers:
                for step in steps:
                    if isinstance(step, dict) and isinstance(step.get("run"), str):
                        for line in step["run"].splitlines():
                            stripped = line.lstrip()
                            if stripped.startswith("#"):
                                continue
                            # Cut a trailing comment; `#` can never be part
                            # of a matched path (not in the command charset).
                            cut = re.search(r"\s#", line)
                            pieces.append(line[: cut.start()] if cut else line)
    # `uses:` composite actions are out of scope: this repository's workflows
    # inline every gate command.
    return "\n".join(pieces)


def main() -> int:
    local_paths = set(COMMAND.findall(LOCAL.read_text()))
    bodies = []
    for path in sorted(WORKFLOWS.glob("*.yml")):
        bodies.append(run_script_bodies(path))
    hosted = "\n".join(bodies)
    missing = sorted(path for path in local_paths if path not in hosted)
    if missing:
        print("hosted CI is missing standalone ci_local gates:", file=sys.stderr)
        for path in missing:
            print(f"  {path}", file=sys.stderr)
        return 1
    print(f"CI/local standalone gate parity: PASS ({len(local_paths)} commands)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
