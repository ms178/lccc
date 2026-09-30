#!/usr/bin/env python3
"""check_pipefail_sigpipe.py — a gate must not confuse SIGPIPE with a real failure.

THE DEFECT THIS PREVENTS
------------------------
Under `set -o pipefail`, a pipeline's status is the last non-zero status of ANY
of its components — including the producer's.  When the consumer exits before
reading all of its input (`head`, `grep -q`, `grep -m<N>`, `grep -l`), the
producer is killed by SIGPIPE and reports 128+13 = 141.  The pipeline is then
non-zero *even though the check succeeded*, so

    if echo "$body" | grep -Eq "$pat"; then ...   # "pattern not found" !?

reports failure while grep itself returned 0.  The gate is then flaky in a way
that looks exactly like a compiler regression, and the harder the machine is
pressed (parallel CI jobs, a loaded dev box) the more often it fires.

MEASURED, on this repo's own fixtures, 20000 iterations per idiom:

    echo "$b" | grep -Eq P          false failures:  27 / 20000
    printf '%s\\n' "$b" | grep -Eq P  false failures: 153 / 20000
    grep -Eq P <<<"$b"              false failures:   0 / 20000
    [[ $b =~ P ]]                   false failures:   0 / 20000

and end to end: `check_volatile_access_semantics.sh` failed about one run in
three, on a compiler that produced a byte-identical .s on 400 consecutive
compiles of its fixture.  Every failure message blamed a different assertion
(loop back edge, pointer-param load, DCE-surviving load), which is the signature
of a harness race rather than a codegen bug: `PIPESTATUS` showed `echo=141
grep=0`, i.e. the assertion had PASSED.

Notice that `printf` is WORSE than `echo`: both are shell builtins whose stdio
buffer is flushed at exit, so a consumer that exits first turns a successful
write into EPIPE with a probability that depends on scheduling.

THE RULE
--------
Never let a pipeline's consumer stop reading early when the status matters.
Use a consumer that reads to end of input, or feed the consumer without a pipe:

    head -N        ->  sed -n '1,Np'      (identical output, reads to EOF)
    grep -q P      ->  grep -c P >/dev/null   (identical status, reads to EOF)
                   ->  grep -q P <<<"$var"    (no pipe at all)
    grep -m1 P     ->  grep P | sed -n '1p'

All three forms keep the PRODUCER's real exit status visible, which the naive
`|| true` band-aid does not: it hides a compiler crash along with the SIGPIPE.

SCOPE
-----
Strict over the scripts CI actually executes (derived from `.github/workflows`
and `scripts/ci_local.sh` by the same regexes `check_ci_gate_parity.py` uses, so
a newly wired gate is covered automatically): a false red there is a red CI.
The remaining repo-wide sites are a census, not a failure, because most are
developer-facing helper scripts whose output is not a pass/fail signal.  That
census is printed so it can be burned down deliberately.
"""

from __future__ import annotations

import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
from check_ci_gate_parity import COMMAND, LOCAL, WORKFLOWS  # noqa: E402

# A consumer that stops before end-of-input, so the producer can take SIGPIPE.
# The consumer token must be followed by something that is not part of a longer
# word -- including end-of-line, `)`, `;` and a trailing backslash, all of which
# appear in real gate scripts (`x=$(cmd | head -5)` is the shape that a
# space-or-EOL boundary misses).
EARLY_EXIT = re.compile(
    r"\|[ ]*(?:head[ ]+-n?[ ]*[0-9]+|grep[ ]+-[a-zA-Z]*q[a-zA-Z]*|"
    r"grep[ ]+-[a-zA-Z]*m[0-9]+|grep[ ]+-[a-zA-Z]*l)(?![A-Za-z0-9_-])"
)
PIPEFAIL = re.compile(r"set[ ]+-[a-zA-Z]*o[ ]+pipefail|set[ ]+-[a-zA-Z]*[a-z]+[ ]*$|pipefail")


def wired_scripts() -> set[str]:
    """Repo-relative paths of scripts that CI runs, from the parity gate's own source of truth."""
    text = ""
    for wf in sorted(WORKFLOWS.glob("*.yml")):
        text += wf.read_text(encoding="utf-8")
    if LOCAL.exists():
        text += LOCAL.read_text(encoding="utf-8")
    return {m.group(0) for m in COMMAND.finditer(text)}


def violations(path: Path) -> list[tuple[int, str]]:
    """Line numbers and text of early-exit consumers in a script that enables pipefail."""
    try:
        text = path.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return []
    if "pipefail" not in text:
        return []
    out = []
    for n, line in enumerate(text.split("\n"), 1):
        stripped = line.strip()
        if stripped.startswith("#"):
            continue
        if "|| true" in line:  # the status is discarded anyway; cannot produce a false red
            continue
        if EARLY_EXIT.search(line):
            out.append((n, stripped))
    return out


def census(paths: list[Path]) -> dict[str, list[tuple[int, str]]]:
    return {str(p.relative_to(ROOT)): v for p, v in ((p, violations(p)) for p in paths) if v}


SELF_TEST_CASES = [
    # (name, body, expect_flagged)
    ("head after a pipe", "set -euo pipefail\nx=$(cmd | head -5)\n", True),
    ("grep -q after a pipe", "set -euo pipefail\nif a | grep -q p; then :; fi\n", True),
    ("grep -m1 after a pipe", "set -euo pipefail\na | grep -m1 p\n", True),
    ("grep -l after a pipe", "set -euo pipefail\na | grep -l p\n", True),
    ("chained head mid-pipeline", "set -euo pipefail\nx=$(a | head -1 | cut -d: -f1)\n", True),
    # the fixed spellings must be accepted
    ("sed instead of head", "set -euo pipefail\nx=$(cmd | sed -n '1,5p')\n", False),
    ("grep -c with discarded count", "set -euo pipefail\nif a | grep -c p >/dev/null; then :; fi\n", False),
    ("here-string", "set -euo pipefail\nif grep -qE p <<<\"$v\"; then :; fi\n", False),
    ("grep -q on a file", "set -euo pipefail\nif grep -q p file; then :; fi\n", False),
    ("process substitution", "set -euo pipefail\nif grep -q p <(cmd); then :; fi\n", False),
    ("no pipefail at all", "set -euo pipefail\n".replace("pipefail", "e") + "x=$(cmd | head -5)\n", False),
    ("comment only", "set -euo pipefail\n# cmd | head -5 is fine to mention\n", False),
    ("status discarded with || true", "set -euo pipefail\ncmd | head -5 || true\n", False),
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


def main() -> int:
    args = sys.argv[1:]
    if "--self-test" in args:
        return self_test()

    wired = sorted(wired_scripts())
    # Shell scripts only: `set -o pipefail` is a shell feature, so the class
    # cannot occur in a Python gate -- and scanning Python would flag this
    # file's own self-test fixtures, which quote the bad spellings on purpose.
    targets = [ROOT / w for w in wired if (ROOT / w).is_file() and w.endswith(".sh")]
    bad = census(targets)
    total = sum(len(v) for v in bad.values())

    if "--census" in args:
        print(f"  repo-wide census (CI set + helper scripts):")
        all_sh = sorted(set(ROOT.glob("tests/**/*.sh")) | set(ROOT.glob("scripts/*.sh")) | set(ROOT.glob("tools/**/*.sh")))
        c = census(all_sh)
        for f, v in sorted(c.items(), key=lambda kv: -len(kv[1])):
            print(f"    {f}: {len(v)} site(s)")
        print(f"  total: {sum(len(v) for v in c.values())}")
        return 0

    if not bad:
        print(f"  ok: no early-exit consumer in a pipeline across {len(targets)} CI-run scripts")
        print("      (a false red here would be a red CI: the producer's SIGPIPE under")
        print("       `set -o pipefail` would be reported as the check failing)")
        return 0

    print("  a pipeline consumer that stops reading early, under `set -o pipefail`:", file=sys.stderr)
    for f, v in sorted(bad.items()):
        for n, line in v:
            print(f"    {f}:{n}: {line}", file=sys.stderr)
    print(f"  {total} site(s) in {len(bad)} CI-run script(s)", file=sys.stderr)
    print("  fix: `head -N` -> `sed -n '1,Np'`;  `grep -q P` -> `grep -c P >/dev/null`", file=sys.stderr)
    print("       or `grep -q P <<<\"$var\"`.  Do not use `|| true`: it also hides a real", file=sys.stderr)
    print("       producer failure (a compiler crash) along with the SIGPIPE.", file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(main())
