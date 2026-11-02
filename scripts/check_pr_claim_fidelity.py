#!/usr/bin/env python3
"""Check that a commit's own words about the tree are true of the tree.

Claims in commit messages, PR bodies and reports go stale in one specific way:
a message says "the ratchet is 151" while the gate still assigns 153, or names
`tests/regression/foo.c` after the file moved into a subdirectory.  Neither is a
code defect, and both are worse than one, because the reader -- a reviewer, a
future maintainer, CI -- cannot tell a stale sentence from a lying gate.  The
fix is not more care in prose; it is a check: claims that *can* be verified
mechanically must be.

Two claim forms are verified, because those are the two that have actually
fooled someone here:

1. ``NAME=153`` / ``NAME 153`` for a NAME that is assigned a numeric literal in
   the tree (a shell ratchet, a python budget, a Rust const).  The claim and the
   assignment must be the same number.
2. Any tree-relative path in the prose (``src/...``, ``tests/...``,
   ``scripts/...``, ``tools/...``, ``docs/...``).  It must exist at the revision
   the message describes -- which is HEAD, unless ``--at`` says otherwise.

Deliberately *not* verified: counts of passing tests, timings, "measured on
-m32" claims.  Those are properties of a run, not of the tree; this gate would be
either a no-op or a lie-generator if it pretended otherwise, and the honest place
for them is the log file the run leaves behind.

Usage:
    scripts/check_pr_claim_fidelity.py [--base REF] [--head HEAD] [--self-test]
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

# A path mention in prose: must start at a known top-level directory and look
# like a file (extension required -- a bare `src/backend` heading is not a
# claim that a file exists).
# The extension is matched as long-as-it-gets and then required to end: a
# `{1,6}`-style cap silently truncated `scripts/kernel-vm.fragment` to
# `scripts/kernel-vm.fragme`, which of course does not exist, and the gate
# reported a true claim as a lie.  A false positive on an honest message is the
# fastest way for a check like this to be ignored.
PATH_CLAIM = re.compile(
    r"(?<![\w./-])"
    r"((?:src|tests|scripts|tools|docs|\.github)(?:/[\w.\-]+)+\.[A-Za-z0-9]{1,12}(?![A-Za-z0-9]))"
)

# `IDENT=12`, `IDENT = 12`, or "IDENT 12" where IDENT is upper-snake and the
# number is not part of a date/version (no dots, not a 4-digit year alone).
NUM_CLAIM = re.compile(r"\b([A-Z][A-Z0-9_]{3,})\s*[=:]\s*(\d{1,6})\b")
NUM_CLAIM_LOOSE = re.compile(r"\b([A-Z][A-Z0-9_]{3,})\s+(\d{2,6})\b")

# Where a number is *assigned* in the tree, per language.
ASSIGNMENTS = (
    re.compile(r"^[ \t]*([A-Z][A-Z0-9_]{3,})=(\d+)[ \t]*$", re.M),  # shell
    re.compile(r"^([A-Z][A-Z0-9_]{3,})\s*=\s*(\d+)\s*$", re.M),  # python
    re.compile(
        r"const\s+([A-Z][A-Z0-9_]{3,})\s*:\s*(?:usize|u32|u64|i64|usize)\s*=\s*(\d+)",
        re.M,
    ),  # rust
)

# Files worth scanning for assignments: the gates and scripts, where a budget or
# ratchet lives.  `src/` is included for Rust consts.
SCAN_GLOBS = ("tests/regression/*.sh", "scripts/*.py", "scripts/*.sh", "src/**/*.rs")


def tree_assignments() -> dict[str, list[tuple[str, int]]]:
    """Every `NAME = number` in the gate/script/source files, with its location."""
    found: dict[str, list[tuple[str, int]]] = {}
    for pattern in SCAN_GLOBS:
        for path in ROOT.glob(pattern):
            if not path.is_file():
                continue
            try:
                text = path.read_text(encoding="utf-8", errors="replace")
            except OSError:  # pragma: no cover - unreadable file is not a claim
                continue
            for rx in ASSIGNMENTS:
                for m in rx.finditer(text):
                    name, value = m.group(1), int(m.group(2))
                    rel = str(path.relative_to(ROOT))
                    found.setdefault(name, []).append((rel, value))
    return found


def rev_paths(rev: str) -> set[str]:
    out = subprocess.run(
        ["git", "-C", str(ROOT), "ls-tree", "-r", "--name-only", rev],
        capture_output=True,
        text=True,
        check=True,
    )
    return set(out.stdout.split())


def check_message(
    subject: str, body: str, assigns: dict[str, list[tuple[str, int]]], paths: set[str]
) -> list[str]:
    findings: list[str] = []
    prose = f"{subject}\n{body}"
    # Strip fenced code blocks and quoted diff lines: a `-` line from a diff is
    # not a claim about the tree, it is history.
    prose = re.sub(r"```.*?```", "", prose, flags=re.S)
    prose = "\n".join(l for l in prose.split("\n") if not l.startswith(("- ", "+ ")))

    for m in PATH_CLAIM.finditer(prose):
        claimed = m.group(1)
        if claimed not in paths:
            findings.append(
                f"path claim {claimed!r} names no file in the tree at this revision "
                "(moved, renamed or never written?)"
            )

    seen: set[tuple[str, int]] = set()
    for rx in (NUM_CLAIM, NUM_CLAIM_LOOSE):
        for m in rx.finditer(prose):
            name, value = m.group(1), int(m.group(2))
            if (name, value) in seen:
                continue
            seen.add((name, value))
            entries = assigns.get(name)
            if not entries:
                continue  # not a tree constant we can judge
            wanted = {v for _, v in entries}
            if value not in wanted:
                where = ", ".join(f"{f}:{n}" for f, n in entries)
                findings.append(
                    f"claim {name}={value} contradicts the tree: {name} is "
                    f"{'/'.join(str(w) for w in sorted(wanted))} in {where}"
                )
    return findings


def commits(base: str | None, head: str, single: bool = False) -> list[tuple[str, str, str]]:
    if single:
        rng_args = ["-1", head]
    elif base:
        rng_args = [f"{base}..{head}"]
    else:
        # `git log HEAD` would be "every message in the repository": an
        # unbounded range is not what `--all` means here, and reading it as one
        # is how this check once accused 5,517 commits it never looked at.
        # `--all` is the reachable-from-head sweep, said out loud.
        rng_args = [head]
    out = subprocess.run(
        ["git", "-C", str(ROOT), "log", "--format=%H\x01%s\x01%b\x02", *rng_args],
        capture_output=True,
        text=True,
        check=True,
    )
    rows = []
    for chunk in out.stdout.split("\x02"):
        chunk = chunk.strip("\n")
        if not chunk:
            continue
        head_sha, rest = chunk.split("\x01", 1)
        subject, body = rest.split("\n", 1) if "\n" in rest else (rest, "")
        rows.append((head_sha[:9], subject.strip(), body))
    return rows


# ---------------------------------------------------------------------------
# self-test: a claim checker that is never shown a lie is not a claim checker.
# ---------------------------------------------------------------------------
WITNESSES = [
    (
        "a budget that moved out from under the message",
        "gates: pin the read budget\n\nThe ratchet is ENV_READ_BUDGET=999 for now.\n",
        True,
    ),
    (
        "the same message, with the number the file actually assigns",
        "gates: pin the read budget\n\nThe ratchet is ENV_READ_BUDGET=151 for now.\n",
        False,
    ),
    (
        "a fixture renamed after the message was written",
        "corpus: add a case\n\nSee tests/regression/i386_regparm_staging_three.c.\n",
        True,
    ),
    (
        "a path claim that matches the tree",
        "corpus: add a case\n\nSee tests/regression/run_regression.py.\n",
        False,
    ),
    (
        "a long extension must not be truncated into a false accusation",
        "kernel: pin the config\n\n`scripts/kernel-vm.fragment` pins CONFIG_GENERIC_CPU=y.\n",
        False,
    ),
]


def self_test() -> int:
    fixture = "ENV_READ_BUDGET=151\n"
    assigns = {"ENV_READ_BUDGET": [("tests/regression/check_env_test_hygiene.sh", 151)]}
    paths = {
        "tests/regression/run_regression.py",
        "tests/regression/check_env_test_hygiene.sh",
        "scripts/kernel-vm.fragment",
    }
    checks = 0
    for label, msg, expect_finding in WITNESSES:
        checks += 1
        subject, _, body = msg.partition("\n")
        got = check_message(subject, body, assigns, paths)
        if bool(got) != expect_finding:
            verb = "missed" if expect_finding else "flagged"
            print(f"FAIL: self-test {verb} {label!r}: {got}", file=sys.stderr)
            return 1
    # The fixture above must also be a real file assignment, or the witness is
    # testing the witness and nothing else.
    checks += 1
    if not re.search(ASSIGNMENTS[0], fixture):
        print("FAIL: self-test fixture no longer parses as a shell assignment", file=sys.stderr)
        return 1
    print(f"claim-fidelity guard: {checks} witness checks passed")
    return 0


def main(argv=None) -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--base", default=None, help="exclusive base revision")
    ap.add_argument("--head", default="HEAD")
    ap.add_argument("--self-test", action="store_true")
    ap.add_argument(
        "--all",
        action="store_true",
        help="also check history: claims are judged against the tree at --head, so "
        "a message that was true when written and is stale now will be reported",
    )
    args = ap.parse_args(argv)
    # The witness run comes first: a check that silently stopped matching anything
    # would report "no commits in range" as a pass otherwise.
    if (rc := self_test()) != 0:
        return rc
    if args.self_test:
        return 0

    base = args.base
    tip_only = False
    if base is None and not args.all:
        # One commit is the unit of accountability: the message being written now.
        # A range is wrong for that default, because on a merge commit `HEAD~1..HEAD`
        # walks the *other* side and reports someone else's history.
        tip_only = True
    assigns = tree_assignments()
    paths = rev_paths(args.head)
    rows = commits(None, args.head, single=tip_only) if tip_only else commits(base, args.head)
    if not rows:
        print("claim-fidelity guard: no commits in range (nothing claimed, nothing to check)")
        return 0
    findings: list[str] = []
    for sha, subject, body in rows:
        for f in check_message(subject, body, assigns, paths):
            findings.append(f"{sha}: {f}")
    if findings:
        print(
            f"FAIL: {len(findings)} claim(s) in {len(rows)} commit message(s) "
            "do not hold of the tree:",
            file=sys.stderr,
        )
        for f in findings:
            print(f"  {f}", file=sys.stderr)
        return 1
    scope = "the tip commit" if tip_only else f"{len(rows)} message(s)"
    print(
        f"PASS: every verifiable claim in {scope} matches the "
        f"tree ({len(assigns)} named budgets, {len(paths)} paths known)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
