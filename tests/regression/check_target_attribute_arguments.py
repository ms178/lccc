#!/usr/bin/env python3
"""The arguments of `target("...")` against GCC 16.2.

`fixtures/target_attribute/target_attribute_arguments.c` gives one function
per target string and, in a trailing `/* expect: ... */` comment, the errors
GCC 16.2 reports for it, in order (` | ` between them; `@name` in front of
one GCC places at the function's name). The gate checks:

  * lccc against the comments, always: the same errors, in the same order,
    on the same lines, and at the function's name where the comment says so;
  * GCC against the comments, when a GCC 16.2 is available, so the comments
    cannot drift from the compiler they quote;
  * that lccc refuses the file (a diagnosed function is not compiled) and
    accepts the file made of the accepted lines alone, without a diagnostic.

Until this gate, lccc read every target string in silence: `target("foo")`,
`target("AVX2")` and `target("avx2, sse4.2")` compiled, the misspelt part
ignored, where GCC stops the build with "attribute 'target' argument '...'
is unknown".
"""

from __future__ import annotations

import argparse
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
SOURCE = HERE / "fixtures" / "target_attribute" / "target_attribute_arguments.c"
# GCC's C-locale quoting (plain apostrophes), as the comments have it.
ENV = {**os.environ, "LC_ALL": "C"}
EXPECT = re.compile(r"/\* expect: (.*) \*/\s*$")
NAME = re.compile(r"\bint (t\d+)\(void\)")
DIAG = re.compile(r"^(?P<file>[^:\s]+):(?P<line>\d+):(?P<col>\d+): (?P<sev>warning|error): (?P<msg>.*?)(?: \[-W[^\]]*\])?$")


def expectations(text: str) -> dict[int, list[tuple[bool, str]]]:
    """line -> [(at_name, message)] for the lines that expect errors."""
    out = {}
    for number, line in enumerate(text.splitlines(), 1):
        match = EXPECT.search(line)
        if match:
            out[number] = [
                (part.startswith("@name "), part.removeprefix("@name "))
                for part in match.group(1).split(" | ")
            ]
    return out


def name_columns(text: str) -> dict[int, int]:
    """line -> 1-based column of the function's name."""
    out = {}
    for number, line in enumerate(text.splitlines(), 1):
        match = NAME.search(line)
        if match:
            out[number] = match.start(1) + 1
    return out


def diagnostics(compiler: str, source: Path, workdir: Path) -> tuple[int, dict[int, list[tuple[int, str]]], list[str]]:
    """(exit status, line -> [(column, message)] of the errors, other diagnostics)."""
    proc = subprocess.run(
        [compiler, "-c", str(source), "-o", str(workdir / "out.o")],
        capture_output=True, text=True, env=ENV, timeout=120, check=False,
    )
    errors: dict[int, list[tuple[int, str]]] = {}
    other = []
    for line in proc.stderr.splitlines():
        match = DIAG.match(line)
        if not match:
            continue
        if match.group("sev") == "error":
            errors.setdefault(int(match.group("line")), []).append(
                (int(match.group("col")), match.group("msg")))
        else:
            other.append(line)
    return proc.returncode, errors, other


def compare(label: str, expected, columns, errors, check_name_column: bool) -> list[str]:
    problems = []
    for line in sorted(set(expected) | set(errors)):
        want = expected.get(line, [])
        got = errors.get(line, [])
        if [message for _, message in want] != [message for _, message in got]:
            problems.append(
                f"{label}: line {line}: expected {[m for _, m in want]}, got {[m for _, m in got]}")
            continue
        if not check_name_column:
            continue
        for (at_name, message), (column, _) in zip(want, got):
            if at_name and column != columns[line]:
                problems.append(
                    f"{label}: line {line}: '{message}' at column {column}, "
                    f"the function's name is at {columns[line]}")
    return problems


def find_gcc(requested: str | None) -> str | None:
    for candidate in filter(None, [requested, os.environ.get("GCC16"), "gcc-16"]):
        path = shutil.which(candidate)
        if not path:
            continue
        version = subprocess.run([path, "-dumpfullversion"], capture_output=True, text=True,
                                 check=False).stdout.strip()
        if version.startswith("16.2"):
            return path
    return None


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--lccc", default=str(ROOT / "target/fastbuild/lccc"))
    parser.add_argument("--gcc", default=None)
    parser.add_argument("--allow-missing-oracle", action="store_true",
                        help="pass without GCC 16.2 (lccc is still checked against the comments)")
    args = parser.parse_args()

    text = SOURCE.read_text()
    expected = expectations(text)
    columns = name_columns(text)
    if len(expected) < 40 or len(columns) - len(expected) < 15:
        print(f"FAIL: the fixture lost its cases ({len(expected)} refused, "
              f"{len(columns) - len(expected)} accepted)")
        return 1
    problems = []
    with tempfile.TemporaryDirectory(prefix="lccc-target-attr-") as tmp:
        workdir = Path(tmp)
        status, errors, other = diagnostics(args.lccc, SOURCE, workdir)
        problems += compare("lccc", expected, columns, errors, check_name_column=True)
        if status == 0:
            problems.append("lccc: the fixture compiled although functions were diagnosed")
        problems += [f"lccc: unexpected diagnostic: {line}" for line in other]

        # The accepted lines alone: no diagnostic of any kind, and an object.
        accepted = workdir / "accepted.c"
        accepted.write_text("\n".join(
            line for number, line in enumerate(text.splitlines(), 1)
            if number in columns and number not in expected) + "\n")
        status, errors, other = diagnostics(args.lccc, accepted, workdir)
        if status != 0 or errors or other:
            problems.append(f"lccc: the accepted target strings are not accepted: {errors} {other}")

        gcc = find_gcc(args.gcc)
        if gcc is None:
            if not args.allow_missing_oracle:
                print("FAIL: GCC 16.2 not found (pass --gcc, or --allow-missing-oracle)")
                return 1
            oracle = "no GCC 16.2: comments not re-measured"
        else:
            status, errors, _ = diagnostics(gcc, SOURCE, workdir)
            problems += compare("gcc", expected, columns, errors, check_name_column=True)
            status, errors, _ = diagnostics(gcc, accepted, workdir)
            if status != 0 or errors:
                problems.append(f"gcc: refuses a line the fixture calls accepted: {errors}")
            oracle = "comments re-measured against GCC 16.2"

    for problem in problems:
        print(f"FAIL: {problem}")
    refused = sum(len(v) for v in expected.values())
    verdict = "FAIL" if problems else "PASS"
    print(f"target-attribute-arguments: {len(columns)} target strings, {refused} expected errors, "
          f"{oracle}, {verdict}")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
