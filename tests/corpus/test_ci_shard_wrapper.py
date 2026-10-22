"""The fast-contract shards must fail when a gate fails.

Why this exists
---------------
Both shard steps in ``.github/workflows/ci.yml`` run 60+ gates through one
shell function and pipe its transcript through ``tee`` so a failing gate can
attach its own diagnostics.  The original shape was::

    if ! fast_contracts 2>&1 | tee "$RUNNER_TEMP/fast-contracts-a.log"; then
      python3 scripts/ci_annotate_log.py ... "$RUNNER_TEMP/fast-contracts-a.log"
      exit 1
    fi

That shape *cannot fail*.  POSIX says ``-e`` is ignored for the entire body of
a function invoked as part of an ``if``/``while``/``until`` condition or of a
``||``/``&&`` list, and bash additionally stops running the ERR trap there.
The function therefore returns the status of its LAST command: a shard whose
final gate passed reported green with any number of earlier gates red.  It
took a gate that fails on a from-source build while CI was green to expose it.

This test pins both halves of the fix, because either one alone regresses:

1. STRUCTURAL -- the workflow must not invoke a trap/errexit gate function
   from a conditional context (the rule lives in
   ``scripts/check_ci_workflow_shell.py`` so ``ci_local.sh`` enforces it too).
2. BEHAVIOURAL -- the *actual* wrapper tail from the workflow, executed with a
   synthetic gate body that fails on its first command, must exit non-zero and
   must not reach the line after the pipeline; with a passing body it must
   exit zero.  This runs the real trap text, the real ``tee`` plumbing and the
   real annotator, so a refactor that reintroduces the masking shape fails
   here even if it satisfies the structural rule by accident.
"""

from __future__ import annotations

import importlib.util
import os
import subprocess
import tempfile
import unittest
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github" / "workflows" / "ci.yml"


def _shell_checker():
    spec = importlib.util.spec_from_file_location(
        "check_ci_workflow_shell", ROOT / "scripts" / "check_ci_workflow_shell.py"
    )
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


def _gate_steps():
    """(job id, step name, run body) for every step that runs a shell function."""
    document = yaml.safe_load(WORKFLOW.read_text())
    found = []
    for job, spec in (document.get("jobs") or {}).items():
        for step in spec.get("steps") or []:
            body = step.get("run")
            if not isinstance(body, str) or "() {" not in body:
                continue
            found.append((job, step.get("name") or "", body))
    return found


def _wrapper_tail(body: str, function: str) -> str:
    """The lines after the gate function closes: trap, annotate, invocation."""
    lines = body.splitlines()
    start = next(
        index for index, line in enumerate(lines)
        if line.strip().startswith(f"{function}()")
    )
    end = next(index for index in range(start + 1, len(lines)) if lines[index].rstrip() == "}")
    return "\n".join(lines[end + 1:])


class ShardWrapperTest(unittest.TestCase):
    def setUp(self) -> None:
        self.checker = _shell_checker()

    def test_gate_steps_exist(self) -> None:
        steps = [s for s in _gate_steps() if "fast-contracts" in s[2]]
        self.assertEqual(
            {name for _job, name, _body in steps},
            {
                "Verify remaining fast local contracts (shard a of 2)",
                "Verify remaining fast local contracts (shard b of 2)",
            },
        )

    def test_no_gate_function_is_invoked_conditionally(self) -> None:
        for job, name, body in _gate_steps():
            with self.subTest(job=job, step=name):
                self.assertEqual(
                    self.checker.check_conditional_gate_functions(body), [],
                    f"{job}/{name} invokes a trap/errexit gate function from a "
                    f"conditional context: bash disables both there, so the "
                    f"shard cannot fail",
                )

    def test_wrapper_tail_propagates_the_first_failure(self) -> None:
        for job, name, body in _gate_steps():
            if "fast-contracts" not in body:
                continue
            function = next(
                candidate for candidate in self.checker.function_definitions(body)
            )
            tail = _wrapper_tail(body, function)
            self.assertIn("ci_annotate_log.py", tail, f"{job}/{name} lost its annotator")
            for gate_status, expected in ((1, "failed"), (0, "passed")):
                with self.subTest(job=job, step=name, gate_status=gate_status):
                    script = (
                        "set -Eeuo pipefail\n"
                        f"{function}() {{\n"
                        "  trap 'rc=$?; printf \"::error title=Fast contract failed::%s\\n\" "
                        "\"$BASH_COMMAND\"; exit \"$rc\"' ERR\n"
                        f"  bash -c 'exit {gate_status}'\n"
                        "  echo SHOULD-NOT-RUN\n"
                        "}\n"
                        f"{tail}\n"
                        "echo REACHED-END\n"
                    )
                    with tempfile.TemporaryDirectory() as tmp:
                        env = dict(os.environ)
                        env["RUNNER_TEMP"] = tmp
                        result = subprocess.run(
                            ["bash", "-c", script], cwd=ROOT, env=env,
                            capture_output=True, text=True, timeout=120,
                        )
                    if expected == "failed":
                        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
                        self.assertNotIn("SHOULD-NOT-RUN", result.stdout)
                        self.assertNotIn("REACHED-END", result.stdout)
                    else:
                        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                        self.assertIn("REACHED-END", result.stdout)


if __name__ == "__main__":
    unittest.main()
