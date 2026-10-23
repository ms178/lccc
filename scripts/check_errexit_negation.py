#!/usr/bin/env python3
"""check_errexit_negation.py -- a negative assertion must be able to fail.

THE DEFECT THIS PREVENTS
------------------------
bash's errexit (`set -e`) deliberately ignores the status of a pipeline
inverted with `!` (POSIX 2.8.1, bash manual "The Set Builtin"):

    set -euo pipefail
    ! grep -q 'andnq' "$tmp/base.s"   # never fails the script

The line evaluates `! grep`, gets status 1 when the forbidden text IS
present, and the shell carries on.  The assertion reads as a check and
never fires.  When this gate landed it found 43 such lines across 16
gate scripts.  Two of them hid wrong test premises: the "baseline x86-64
must stay SIGILL-safe" legs of check_andn_fusion.sh and
check_copy64_ymm.sh compiled without `-march`.  lccc's default baseline
is x86-64-v3, so `andnq` and `%ymm` were present and the gates had been
"passing" with the forbidden output in hand.

THE FIX SPELLING
----------------
Make the inverted status reach errexit through an OR list with an explicit
exit:

    ! grep -q P file || { echo "FAIL: ..." >&2; exit 1; }

`if grep -q P file; then ...; exit 1; fi` is equally fine.  A trailing
`|| true`, `&& ...`, or use as an `if`/`while` condition is not flagged.
The status reaches a decision point there, by construction.

SCOPE
-----
Every shell script under tests/, scripts/ and tools/ that enables errexit,
not only the CI-wired ones.  The allowlisted orphan gates in
ci_gate_allowlist.txt are tagged "passes locally", and that claim is only
true if their assertions can fail.  The scan is textual and costs well
under a second.
"""
from __future__ import annotations

import re
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

ERREXIT = re.compile(r"^\s*set\s+-[a-zA-Z]*e|^\s*set\s+-o\s+errexit", re.M)
QUOTED = re.compile(r"'[^']*'|\"(?:\\.|[^\"\\])*\"")
NEGATED = re.compile(r"^!\s")
# A status that reaches a decision point: OR/AND lists, or an if/while/until
# condition (`! cmd; then` on the same logical line).
HEREDOC = re.compile(r"(?<!<)<<-?(?!<)\s*['\"]?([A-Za-z_][A-Za-z0-9_]*)['\"]?")
DECIDED = re.compile(r"\|\||&&|;\s*(?:then|do)\b")


def logical_lines(text: str):
    """(first_line_number, joined_text) with backslash continuations folded."""
    lines = text.split("\n")
    i = 0
    while i < len(lines):
        start = i
        buf = lines[i]
        while buf.rstrip().endswith("\\") and i + 1 < len(lines):
            i += 1
            buf = buf.rstrip()[:-1] + " " + lines[i].lstrip()
        yield start + 1, buf
        i += 1


def violations_text(text: str) -> list[tuple[int, str]]:
    if not ERREXIT.search(text):
        return []
    out = []
    in_heredoc: str | None = None
    for n, line in logical_lines(text):
        stripped = line.strip()
        if in_heredoc is not None:
            if stripped == in_heredoc:
                in_heredoc = None
            continue
        m = HEREDOC.search(line)
        if m:
            in_heredoc = m.group(1)
        if not NEGATED.match(stripped):
            continue
        masked = QUOTED.sub('""', stripped)
        masked = masked.split(" #", 1)[0]
        if DECIDED.search(masked):
            continue
        out.append((n, stripped))
    return out


def violations(path: Path) -> list[tuple[int, str]]:
    try:
        return violations_text(path.read_text(encoding="utf-8", errors="replace"))
    except OSError:
        return []


SELF_TEST_CASES = [
    # (name, body, expect_flagged)
    ("bare negated grep", "set -euo pipefail\n! grep -q p f\n", True),
    ("negated pipeline", "set -e\n! sed -n 1p f | grep -E p >/dev/null\n", True),
    ("negated compiler run", "set -eu\n! \"$CCC\" -fsyntax-only bad.c 2>e\n", True),
    ("indented in a function", "set -e\nf() {\n    ! grep -q p f\n}\n", True),
    ("trailing comment", "set -e\n! grep -q p f  # must not appear\n", True),
    ("continued line", "set -e\n! grep -q p \\\n    f\n", True),
    ("errexit via -o", "set -o errexit\n! grep -q p f\n", True),
    # spellings whose status reaches a decision
    ("OR exit", "set -e\n! grep -q p f || exit 1\n", False),
    ("OR block", "set -e\n! grep -q p f || { echo FAIL >&2; exit 1; }\n", False),
    ("AND list", "set -e\n! grep -q p f && echo absent\n", False),
    ("if condition", "set -e\nif ! grep -q p f; then exit 1; fi\n", False),
    ("while condition", "set -e\nwhile ! test -e f; do sleep 1; done\n", False),
    ("continued OR", "set -e\n! grep -q p f \\\n    || exit 1\n", False),
    ("|| inside quotes is not an OR", "set -e\n! grep -q 'a||b' f\n", True),
    ("no errexit", "set -u\n! grep -q p f\n", False),
    ("comment", "set -e\n# ! grep -q p f\n", False),
    ("heredoc body", "set -e\ncat <<'EOF'\n! not shell\nEOF\n", False),
    ("after a heredoc", "set -e\ncat >t.c <<'C'\nint x;\nC\n! grep -q p f\n", True),
    ("here-string is not a heredoc", "set -e\ngrep -q x <<<\"$v\"\n! grep -q p f\n", True),
    ("test negation is not a pipeline", "set -e\n[ ! -e f ]\n", False),
]


def self_test() -> int:
    failures = 0
    with tempfile.TemporaryDirectory() as td:
        for name, body, expect in SELF_TEST_CASES:
            p = Path(td) / "case.sh"
            p.write_text(body, encoding="utf-8")
            got = bool(violations(p))
            if got != expect:
                print(f"  FAIL {name}: flagged={got} want={expect}", file=sys.stderr)
                failures += 1
    total = len(SELF_TEST_CASES)
    if failures:
        print(f"  {total - failures}/{total} self-test cases pass", file=sys.stderr)
        return 1
    print(f"  self-test: {total}/{total} cases pass")
    return 0


def targets() -> list[Path]:
    found = set(ROOT.glob("tests/**/*.sh")) | set(ROOT.glob("scripts/**/*.sh")) | set(ROOT.glob("tools/**/*.sh"))
    return sorted(p for p in found if p.is_file())


def main() -> int:
    if "--self-test" in sys.argv[1:]:
        return self_test()
    paths = targets()
    bad = {str(p.relative_to(ROOT)): v for p in paths if (v := violations(p))}
    if not bad:
        print(f"  ok: every negated assertion can fail, across {len(paths)} shell scripts")
        return 0
    print("  `! cmd` as a whole command under errexit never fails the script:", file=sys.stderr)
    for f, v in sorted(bad.items()):
        for n, line in v:
            print(f"    {f}:{n}: {line}", file=sys.stderr)
    print(f"  {sum(map(len, bad.values()))} site(s) in {len(bad)} script(s)", file=sys.stderr)
    print("  fix: `! cmd || { echo \"FAIL: ...\" >&2; exit 1; }`  or  `if cmd; then ...; exit 1; fi`", file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(main())
