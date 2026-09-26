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
import shlex
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

REGRESSION_DIR = ROOT / "tests" / "regression"
ALLOWLIST = ROOT / "scripts" / "ci_gate_allowlist.txt"


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


def allowlist_entries() -> set[str]:
    """Paths named in the orphan allowlist (comments and blanks stripped).

    Each entry is a gate script that is KNOWN to be unwired; the list is a
    reviewed, shrinking record of coverage debt, never a place to park a new
    gate (wiring it into ci_local.sh or a workflow deletes its entry).
    """
    if not ALLOWLIST.exists():
        return set()
    entries = set()
    for line in ALLOWLIST.read_text().splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        # A trailing "  # reason" annotates the entry; only the path
        # matters for matching (a reason can never contain whitespace-free
        # `tests/...sh` shape that would alias another path).
        path = line.split("#", 1)[0].strip()
        if path:
            entries.add(path)
    return entries


def check_orphaned_gates(local_text: str, hosted: str) -> int:
    """Fail on any tests/regression/check_*.sh executed by nothing.

    The parity check above only sees scripts that ci_local.sh already
    references; a gate script wired into NEITHER mirror is invisible to it.
    That blind spot let a whole batch of codegen-contract gates rot
    unexecuted. Every gate script must therefore be referenced by
    ci_local.sh, by a hosted workflow, or be explicitly recorded in the
    allowlist -- and the allowlist must not carry entries that are no
    longer orphans, so it can only shrink.
    """
    if not REGRESSION_DIR.is_dir():
        return 0
    allow = allowlist_entries()
    orphans = []
    for path in sorted(REGRESSION_DIR.glob("check_*.sh")):
        rel = str(path.relative_to(ROOT))
        if rel in local_text or rel in hosted:
            continue
        if rel not in allow:
            orphans.append(rel)
    stale = sorted(p for p in allow if p in local_text or p in hosted)
    ok = True
    if orphans:
        ok = False
        print("gate scripts are wired into nothing and not allowlisted:", file=sys.stderr)
        for p in orphans:
            print(f"  {p}", file=sys.stderr)
        print(
            "  wire the gate into scripts/ci_local.sh and/or a workflow",
            file=sys.stderr,
        )
    if stale:
        ok = False
        print("allowlist entries that are no longer orphans (delete them):", file=sys.stderr)
        for p in stale:
            print(f"  {p}", file=sys.stderr)
    if not ok:
        return 1
    return 0


def direct_asmdiff_commands(script: str) -> list[list[str]]:
    """Parse directly executed asm-diff commands, preserving mode and corpus.

    Path-only parity conflates x86-64 and i686 invocations of asmdiff.py.
    Only a real `python3 scripts/asmdiff.py ...` shell line qualifies: prose,
    comments, and `echo python3 ...` are not executable differential gates.
    Consume `\\` continuation lines from that invocation, not the enclosing
    ci_local `gate ... \\` line.
    """
    lines = script.splitlines()
    commands = []
    i = 0
    while i < len(lines):
        line = lines[i].strip()
        i += 1
        if not line.startswith("python3 scripts/asmdiff.py "):
            continue
        while line.endswith("\\") and i < len(lines):
            line = line[:-1] + " " + lines[i].strip()
            i += 1
        try:
            tokens = shlex.split(line, comments=True)
        except ValueError:
            continue
        if tokens[:2] == ["python3", "scripts/asmdiff.py"]:
            commands.append(tokens[2:])
    return commands


def asm_option(tokens: list[str], name: str) -> str | None:
    for i, token in enumerate(tokens):
        if token == name and i + 1 < len(tokens):
            return tokens[i + 1]
        if token.startswith(name + "="):
            return token[len(name) + 1 :]
    return None


def check_asmdiff_gate_parity(local_text: str, hosted: str) -> int:
    """Require the *specific mode, compiler and corpus*, not just the path."""
    specs = (
        ("merged-pr629-x86-asm-diff", False, "target/fastbuild/lccc-x86",
         ("tests/asm-diff/merged-pr629-followup.casefile",)),
        ("i686-asm-diff", True, "target/fastbuild/lccc-i686", ()),
    )
    missing = []
    for where, text in (("local", local_text), ("hosted", hosted)):
        commands = direct_asmdiff_commands(text)
        for gate, mode32, compiler, casefiles in specs:
            if not any(
                ("--32" in cmd) == mode32
                and asm_option(cmd, "--jobs") == "2"
                and asm_option(cmd, "--lccc") == compiler
                and set(casefiles) == {t for t in cmd if t.endswith(".casefile")}
                and (where != "hosted" or mode32 or
                     "gas-2.47-x86_64-linux-gnu/bin/as" in (asm_option(cmd, "--as") or ""))
                for cmd in commands
            ):
                missing.append(f"{where}: {gate} (mode/corpus/compiler/jobs/oracle)")
    if "bash scripts/ensure_gas_247.sh x86_64-linux-gnu" not in hosted:
        missing.append("hosted: install GNU as 2.47 x86-64 oracle")
    if missing:
        print("missing mode/corpus-specific assembly gates:", file=sys.stderr)
        for item in missing:
            print(f"  {item}", file=sys.stderr)
        return 1
    return 0


def main() -> int:
    local_text = LOCAL.read_text()
    local_paths = set(COMMAND.findall(local_text))
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
    rc = check_orphaned_gates(local_text, hosted)
    if rc != 0:
        return rc
    if check_asmdiff_gate_parity(local_text, hosted) != 0:
        return 1
    print(
        f"CI/local standalone gate parity: PASS ({len(local_paths)} commands, "
        "2 mode/corpus-specific asm-diff gates)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
