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

Second rule: a shell function that installs an ERR trap (or enables
`errexit`) must not be invoked from a CONDITIONAL context.

    if ! fast_contracts 2>&1 | tee log; then ... fi     # WRONG
    fast_contracts 2>&1 | tee log                       # right (step trap)

POSIX/bash *disables* `set -e` for every command inside a function invoked as
part of an `if`/`while`/`until` condition, or of a `||`/`&&` list.  The
function's own `set -e` and its ERR trap are silently inert there, so the call
returns the status of its LAST command.  `if ! fast_contracts | tee; then` in
the fast-contract shards therefore reported green whenever the last gate
passed: 133 gates could fail without failing the lane.  It took a gate that
failed on a from-source build while CI was green to expose it.

`--self-test` runs both rules against in-memory fixtures: a masking wrapper
must be rejected, the fail-closed shape accepted, and a plain conditional
command (no function, no trap) left alone.

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


# ── rule 2: conditional invocation of trap/errexit gate functions ──────────
FUNCTION_DEF = re.compile(r"^(?P<indent>[ \t]*)(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*\(\)\s*\{")
ERREXIT_OR_TRAP = re.compile(r"^\s*(?:set\s+-[A-Za-z]*e\b|trap\s+.*\bERR\b)", re.MULTILINE)


def function_definitions(script: str) -> dict[str, tuple[int, int]]:
    """name -> (first line index, last line index) of every `name() {` body."""
    lines = script.splitlines()
    found: dict[str, tuple[int, int]] = {}
    for index, line in enumerate(lines):
        match = FUNCTION_DEF.match(line)
        if not match:
            continue
        name = match.group("name")
        end = index
        for probe in range(index + 1, len(lines)):
            end = probe
            if lines[probe].rstrip() == "}":
                break
        found[name] = (index, end)
    return found


def conditional_invocations(script: str, name: str) -> list[int]:
    """Line numbers (1-based) where `name` is called from a conditional context."""
    hits: list[int] = []
    pattern = re.compile(
        r"(?:\bif\s+!?\s*|\bwhile\s+!?\s*|\buntil\s+!?\s*|\|\|\s*|&&\s*|!\s*)"
        + re.escape(name)
        + r"\b"
    )
    for number, line in enumerate(script.splitlines(), 1):
        if line.strip().startswith("#"):
            continue
        if pattern.search(line):
            hits.append(number)
    return hits


def errexit_functions(script: str) -> set[str]:
    """Function names whose body enables errexit or installs an ERR trap."""
    result: set[str] = set()
    lines = script.splitlines()
    for name, (start, end) in function_definitions(script).items():
        body = "\n".join(lines[start : end + 1])
        if ERREXIT_OR_TRAP.search(body):
            result.add(name)
    return result


def check_conditional_gate_functions(script: str) -> list[str]:
    """Violations for trap/errexit functions invoked from a conditional context."""
    violations: list[str] = []
    guarded = errexit_functions(script)
    for name in sorted(guarded):
        for line in conditional_invocations(script, name):
            violations.append(
                f"line {line}: `{name}` installs errexit/an ERR trap but is invoked "
                f"from a conditional context, where bash disables both -- its "
                f"failures cannot fail the step"
            )
    return violations



# ── self-test fixtures (no workflow tree needed) ───────────────────────────
MASKING_BODY = """
set -Eeuo pipefail
fast_contracts() {
  trap 'rc=$?; printf "::error::%s\\n" "$BASH_COMMAND"; exit "$rc"' ERR
  bash gate_one.sh
  bash gate_two.sh
}
if ! fast_contracts 2>&1 | tee "$RUNNER_TEMP/shard.log"; then
  exit 1
fi
"""

FAIL_CLOSED_BODY = """
set -Eeuo pipefail
fast_contracts() {
  trap 'rc=$?; printf "::error::%s\\n" "$BASH_COMMAND"; exit "$rc"' ERR
  bash gate_one.sh
  bash gate_two.sh
}
annotate_shard() {
  rc=$?
  exit "$rc"
}
trap annotate_shard ERR
fast_contracts 2>&1 | tee "$RUNNER_TEMP/shard.log"
"""

PLAIN_CONDITIONAL_BODY = """
set -Eeuo pipefail
if ! bash tests/regression/check_probe.sh; then
  echo "probe refused"
fi
"""

UNGUARDED_PROBE_BODY = """
set -Eeuo pipefail
probe() { bash tests/regression/check_probe.sh; }
if ! probe; then
  echo "probe refused"
fi
"""


def self_test() -> int:
    """Rule fixtures: masking bodies must be rejected, safe shapes accepted."""
    cases = [
        ("masking gate function", MASKING_BODY, True),
        ("fail-closed gate function", FAIL_CLOSED_BODY, False),
        ("plain conditional command", PLAIN_CONDITIONAL_BODY, False),
        ("unguarded probe function", UNGUARDED_PROBE_BODY, False),
    ]
    failures = 0
    for label, body, expect_violation in cases:
        found = check_conditional_gate_functions(body)
        if bool(found) != expect_violation:
            failures += 1
            print(f"  FAIL [{label}]: expected violation={expect_violation}, got {found}")
        else:
            print(f"  ok   [{label}]: {'rejected' if found else 'accepted'}")
    if failures:
        print(f"check_ci_workflow_shell: self-test FAILED ({failures} case(s))")
        return 1
    print(f"check_ci_workflow_shell: self-test PASS ({len(cases)} fixtures, 2 rules)")
    return 0



def main() -> int:
    if "--self-test" in sys.argv[1:]:
        return self_test()
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
            for detail in check_conditional_gate_functions(script):
                violations.append(
                    f"{wf.name} {loc}: {detail}"
                )
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
            "  an explicit '# nocheck: yaml-step-key' comment on that line.\n"
            "\n  A gate function that sets errexit or an ERR trap must be called\n"
            "  as a plain command; the step-level ERR trap does the annotation:\n"
            "      trap annotate_shard ERR\n"
            "      fast_contracts 2>&1 | tee \"$RUNNER_TEMP/shard.log\"",
            file=sys.stderr,
        )
        return 1

    print(f"check_ci_workflow_shell: ok ({len(files)} workflow file(s), "
          f"2 rules)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
