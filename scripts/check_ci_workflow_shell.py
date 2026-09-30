#!/usr/bin/env python3
"""Structural check: no GitHub-Actions step scaffolding may hide inside a
`run:` block.

Why this exists
---------------
A `run: |` block is a *shell script*, not a YAML mapping. Pasting step
scaffolding into one does not make the workflow invalid YAML -- it makes it
valid YAML that means something else. On 2026-09-30 that shipped in
`.github/workflows/ci.yml` (`- name: Redundant flags compare` /
`run: bash ...` inside the `Verify remaining fast local contracts` block).
The hosted runner executed the block, bash reached the line, tried to run a
command named `-`, and the step died with **exit code 127** -- taking the
gate itself and every gate below it with it. Thirty-six gates never ran, and
because the step that *reported* the failure was the one that died, nothing
in the log named the offending line.

`bash -n` does not catch this: `- name: X` is a syntactically valid command
line (a command named `-` with the argument `name: X`). So the check is
explicit rather than delegated to the shell.

What it checks
--------------
For every workflow file, every `run:` scalar is extracted and scanned for
lines that look like Actions step keys. A match is a hard failure. The scan
runs against the *parsed* document, so it sees the real script text rather
than raw file lines, and it cannot be confused by indentation that happens to
line up.

Exit codes: 0 all clean, 1 violations found, 2 could not run the check.
"""
from __future__ import annotations

import pathlib
import re
import sys

# Keys that only mean something to the Actions runner, never to a shell.
# `env:`/`with:` are included because a step body is the easiest place for a
# scaffold to reappear during a merge.
STEP_KEY = re.compile(
    r"""^\s*(?:-\s+)?    # optional YAML sequence dash
        (?:name|run|uses|with|if|env|shell|steps|jobs|working-directory|
           continue-on-error|timeout-minutes|id)
        \s*:""",
    re.VERBOSE,
)


def iter_run_scripts(node, path: str):
    """Yield (yaml_path, script_text) for every `run:` scalar in the tree."""
    if isinstance(node, dict):
        for k, v in node.items():
            if k == "run" and isinstance(v, str):
                yield path, v
            else:
                yield from iter_run_scripts(v, f"{path}.{k}")
    elif isinstance(node, list):
        for i, v in enumerate(node):
            yield from iter_run_scripts(v, f"{path}[{i}]")


def main() -> int:
    try:
        import yaml
    except ImportError:
        print("check_ci_workflow_shell: PyYAML unavailable; cannot verify", file=sys.stderr)
        return 2

    root = pathlib.Path(__file__).resolve().parent.parent
    wf_dir = root / ".github" / "workflows"
    if not wf_dir.is_dir():
        print(f"check_ci_workflow_shell: no {wf_dir}", file=sys.stderr)
        return 2

    violations: list[str] = []
    files = sorted(wf_dir.glob("*.yml")) + sorted(wf_dir.glob("*.yaml"))
    if not files:
        print("check_ci_workflow_shell: no workflow files found", file=sys.stderr)
        return 2

    for wf in files:
        try:
            doc = yaml.safe_load(wf.read_text())
        except yaml.YAMLError as e:
            violations.append(f"{wf.name}: not parseable as YAML: {e}")
            continue
        for loc, script in iter_run_scripts(doc, wf.name):
            for n, line in enumerate(script.splitlines(), 1):
                if STEP_KEY.match(line):
                    violations.append(
                        f"{wf.name} {loc} line {n}: Actions step key inside a "
                        f"`run:` block -- bash will try to execute it and die "
                        f"with exit 127:\n      {line.rstrip()}"
                    )

    if violations:
        print("check_ci_workflow_shell: FAIL", file=sys.stderr)
        for v in violations:
            print(f"  {v}", file=sys.stderr)
        print(
            "\n  A `run: |` block is shell, not YAML. Use a bare command:\n"
            "      bash tests/regression/check_foo.sh\n"
            "  If a real script line looks like a step key, quote it or add\n"
            "  an explicit '# nocheck: yaml-step-key' comment on that line.",
            file=sys.stderr,
        )
        return 1

    print(f"check_ci_workflow_shell: ok ({len(files)} workflow file(s))")
    return 0


if __name__ == "__main__":
    sys.exit(main())
