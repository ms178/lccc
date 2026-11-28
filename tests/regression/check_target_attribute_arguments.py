#!/usr/bin/env python3
"""The arguments of `target("...")` against GCC 16.2.

Two fixtures in `fixtures/target_attribute/`:

  * `target_attribute_arguments.c`: one function definition per target
    string;
  * `target_pragma_and_prototypes.c`: the same strings on prototypes, on
    functions declared through a typedef, and in `#pragma GCC target`, with
    the option stack (`push_options`, `pop_options`, `reset_options`).

A trailing `/* expect: ... */` comment gives the errors GCC 16.2 reports on
that line, in order (` | ` between them). In front of a message, `@name`
says GCC places it at the declared name and `@cc1` that GCC gives it no
location at all, which is how it refuses a pragma's string. The gate checks:

  * lccc against the comments, always: the same errors, in the same order,
    on the same lines, and at the declared name where the comment says so
    (a `@cc1` error is expected on the line of its pragma);
  * GCC against the comments, when a GCC 16.2 is available, so the comments
    cannot drift from the compiler they quote;
  * that lccc refuses each file (a diagnosed function is not compiled) and
    accepts the file made of the lines without a comment, without a
    diagnostic of any kind.

Until this gate, lccc read every target string in silence: `target("foo")`,
`target("AVX2")` and `target("avx2, sse4.2")` compiled, the misspelt part
ignored, where GCC stops the build with "attribute 'target' argument '...'
is unknown". `#pragma GCC target` was dropped whole, so the functions of a
region compiled without the extensions the region had asked for -- and were
refused as soon as they used one.
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
FIXTURES = HERE / "fixtures" / "target_attribute"
# (file, the declared name on a line, least refused lines, least accepted names)
SOURCES = [
    (FIXTURES / "target_attribute_arguments.c", re.compile(r"\bint (t\d+)\(void\)"), 40, 15),
    (FIXTURES / "target_pragma_and_prototypes.c", re.compile(r"\b(p\d+)\b"), 18, 10),
]
# GCC's C-locale quoting (plain apostrophes), as the comments have it.
ENV = {**os.environ, "LC_ALL": "C"}
EXPECT = re.compile(r"/\* expect: (.*) \*/\s*$")
DIAG = re.compile(r"^(?P<file>[^:\s]+):(?P<line>\d+):(?P<col>\d+): (?P<sev>warning|error): (?P<msg>.*?)(?: \[-W[^\]]*\])?$")
NO_LOCATION = re.compile(r"^cc1: error: (?P<msg>.*)$")
NAME, CC1, PLAIN = "name", "cc1", "plain"


def expectations(text: str) -> dict[int, list[tuple[str, str]]]:
    """line -> [(kind, message)] for the lines that expect errors."""
    out = {}
    for number, line in enumerate(text.splitlines(), 1):
        match = EXPECT.search(line)
        if not match:
            continue
        parts = []
        for part in match.group(1).split(" | "):
            if part.startswith("@name "):
                parts.append((NAME, part.removeprefix("@name ")))
            elif part.startswith("@cc1 "):
                parts.append((CC1, part.removeprefix("@cc1 ")))
            else:
                parts.append((PLAIN, part))
        out[number] = parts
    return out


def name_columns(text: str, name: re.Pattern) -> dict[int, int]:
    """line -> 1-based column of the declared name."""
    out = {}
    for number, line in enumerate(text.splitlines(), 1):
        match = name.search(EXPECT.sub("", line))
        if match and not line.lstrip().startswith(("/*", "*", "#")):
            out[number] = match.start(1) + 1
    return out


def diagnostics(compiler: str, source: Path, workdir: Path):
    """(exit status, line -> [(column, message)] of the located errors,
    the errors without a location, the other diagnostics)."""
    proc = subprocess.run(
        [compiler, "-c", str(source), "-o", str(workdir / "out.o")],
        capture_output=True, text=True, env=ENV, timeout=120, check=False,
    )
    errors: dict[int, list[tuple[int, str]]] = {}
    unlocated = []
    other = []
    for line in proc.stderr.splitlines():
        match = DIAG.match(line)
        if not match:
            bare = NO_LOCATION.match(line)
            if bare:
                unlocated.append(bare.group("msg"))
            continue
        if match.group("sev") == "error":
            errors.setdefault(int(match.group("line")), []).append(
                (int(match.group("col")), match.group("msg")))
        else:
            other.append(line)
    return proc.returncode, errors, unlocated, other


def compare(label: str, expected, columns, errors, unlocated, located_kinds) -> list[str]:
    """`located_kinds`: the kinds this compiler reports with a line. The
    others it reports without one, in the order of the file."""
    problems = []
    for line in sorted(set(expected) | set(errors)):
        want = [(kind, message) for kind, message in expected.get(line, []) if kind in located_kinds]
        got = errors.get(line, [])
        if [message for _, message in want] != [message for _, message in got]:
            problems.append(
                f"{label}: line {line}: expected {[m for _, m in want]}, got {[m for _, m in got]}")
            continue
        for (kind, message), (column, _) in zip(want, got):
            if kind == NAME and column != columns.get(line):
                problems.append(
                    f"{label}: line {line}: '{message}' at column {column}, "
                    f"the declared name is at {columns.get(line)}")
    want_unlocated = [message for line in sorted(expected) for kind, message in expected[line]
                      if kind not in located_kinds]
    if want_unlocated != unlocated:
        problems.append(
            f"{label}: errors without a location: expected {want_unlocated}, got {unlocated}")
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


def check_source(args, gcc, source: Path, name: re.Pattern, least_refused: int,
                 least_accepted: int, workdir: Path) -> tuple[list[str], int, int]:
    text = source.read_text()
    expected = expectations(text)
    columns = name_columns(text, name)
    accepted_names = len([line for line in columns if line not in expected])
    if len(expected) < least_refused or accepted_names < least_accepted:
        return ([f"{source.name}: the fixture lost its cases ({len(expected)} refused lines, "
                 f"{accepted_names} accepted names)"], 0, 0)
    problems = []
    status, errors, unlocated, other = diagnostics(args.lccc, source, workdir)
    problems += compare("lccc", expected, columns, errors, unlocated, {NAME, CC1, PLAIN})
    if status == 0:
        problems.append("lccc: the fixture compiled although declarations were diagnosed")
    problems += [f"lccc: unexpected diagnostic: {line}" for line in other]

    # The lines without an expectation alone: no diagnostic of any kind, and
    # an object.
    accepted = workdir / f"accepted-{source.name}"
    accepted.write_text("\n".join(
        line for number, line in enumerate(text.splitlines(), 1) if number not in expected) + "\n")
    status, errors, unlocated, other = diagnostics(args.lccc, accepted, workdir)
    if status != 0 or errors or unlocated or other:
        problems.append(f"lccc: the accepted lines are not accepted: {errors} {unlocated} {other}")

    if gcc is not None:
        status, errors, unlocated, _ = diagnostics(gcc, source, workdir)
        problems += compare("gcc", expected, columns, errors, unlocated, {NAME, PLAIN})
        status, errors, unlocated, _ = diagnostics(gcc, accepted, workdir)
        if status != 0 or errors or unlocated:
            problems.append(f"gcc: refuses a line the fixture calls accepted: {errors} {unlocated}")
    problems = [f"{source.name}: {problem}" for problem in problems]
    return problems, len(columns), sum(len(v) for v in expected.values())


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--lccc", default=str(ROOT / "target/fastbuild/lccc"))
    parser.add_argument("--gcc", default=None)
    parser.add_argument("--allow-missing-oracle", action="store_true",
                        help="pass without GCC 16.2 (lccc is still checked against the comments)")
    args = parser.parse_args()

    gcc = find_gcc(args.gcc)
    if gcc is None and not args.allow_missing_oracle:
        print("FAIL: GCC 16.2 not found (pass --gcc, or --allow-missing-oracle)")
        return 1
    oracle = ("comments re-measured against GCC 16.2" if gcc
              else "no GCC 16.2: comments not re-measured")
    problems = []
    names = refused = 0
    with tempfile.TemporaryDirectory(prefix="lccc-target-attr-") as tmp:
        for source, name, least_refused, least_accepted in SOURCES:
            found, declared, errors = check_source(
                args, gcc, source, name, least_refused, least_accepted, Path(tmp))
            problems += found
            names += declared
            refused += errors

    for problem in problems:
        print(f"FAIL: {problem}")
    verdict = "FAIL" if problems else "PASS"
    print(f"target-attribute-arguments: {names} declarations, {refused} expected errors, "
          f"{oracle}, {verdict}")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
