#!/usr/bin/env python3
"""Pointer type rules of conversions, conditionals and comparisons against GCC 16.2.

`qualifier_conversions.c` covers the qualifier rules of implicit pointer
conversions (C23 6.5.16.1p1), the pointer arms of `?:` (6.5.15p3) and pointer
comparisons (6.5.8p2, 6.5.9p2). It marks every line GCC diagnoses with an
`/* expect: <severity> <option> */` comment (`none` for a warning without an
option); every other line must stay silent. The gate checks:

  * LCCC against the comments (line, severity, option), always;
  * GCC against the comments, and LCCC against GCC byte for byte (line,
    column, severity, text, option), when the GCC oracle is available;
  * `-Wno-<option>` silences exactly that option's lines, for every option
    the comments name, and `-Werror=discarded-qualifiers` makes those lines
    errors.
"""

from __future__ import annotations

import argparse
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
# The corpus is ill-formed by design, so it lives outside the
# tests/regression/*.c glob that run_regression.py compiles and links.
SOURCE = HERE / "fixtures" / "qualifier_conversions" / "qualifier_conversions.c"
# GCC's C-locale quoting (see check_warning_compat.py).
ENV = {**os.environ, "LC_ALL": "C"}
EXPECT = re.compile(r"/\* expect: (warning|error) ([a-z-]+) \*/")
DIAG = re.compile(
    r"^(?P<file>[^:\s]+):(?P<line>\d+):(?P<col>\d+): (?P<sev>warning|error): "
    r"(?P<msg>.*?)(?: \[-W(?:error=)?(?P<opt>[a-z-]+)\])?$"
)


def expectations() -> dict[int, tuple[str, str]]:
    out = {}
    for number, text in enumerate(SOURCE.read_text().splitlines(), 1):
        match = EXPECT.search(text)
        if match:
            out[number] = (match.group(1), match.group(2))
    return out


def diagnose(compiler: str, *flags: str) -> list[dict[str, str]]:
    proc = subprocess.run(
        [compiler, "-std=gnu17", "-fsyntax-only", *flags, SOURCE.name],
        cwd=SOURCE.parent,
        env=ENV,
        capture_output=True,
        text=True,
        timeout=120,
    )
    diags = []
    for line in proc.stderr.splitlines():
        match = DIAG.match(line)
        if match and match.group("file") == SOURCE.name:
            diags.append(match.groupdict())
    if proc.returncode not in (0, 1) or (proc.returncode == 1 and not diags):
        raise SystemExit(f"{compiler} failed unexpectedly:\n{proc.stderr}")
    return diags


def classified(diags: list[dict[str, str]]) -> dict[int, tuple[str, str]]:
    out: dict[int, tuple[str, str]] = {}
    for d in diags:
        line = int(d["line"])
        if line in out:
            raise SystemExit(f"two diagnostics on line {line}: {out[line]} and {d}")
        out[line] = (d["sev"], d["opt"] or "none")
    return out


def compare(name: str, got: dict, want: dict) -> list[str]:
    problems = []
    for line in sorted(set(got) | set(want)):
        if got.get(line) != want.get(line):
            problems.append(f"{name}: line {line}: got {got.get(line)}, expected {want.get(line)}")
    return problems


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--lccc", default="target/fastbuild/lccc")
    parser.add_argument("--gcc", default="gcc-16")
    args = parser.parse_args()
    lccc = str(Path(args.lccc).resolve())
    want = expectations()
    problems: list[str] = []
    checks = 0

    lccc_diags = diagnose(lccc)
    problems += compare("lccc vs expect", classified(lccc_diags), want)
    checks += 1

    for option in sorted({opt for _, opt in want.values()} - {"none"}):
        quiet = {k: v for k, v in want.items() if v[1] != option}
        problems += compare(
            f"lccc -Wno-{option}", classified(diagnose(lccc, f"-Wno-{option}")), quiet
        )
        checks += 1
    promoted = {k: (("error", v[1]) if v[1] == "discarded-qualifiers" else v) for k, v in want.items()}
    problems += compare(
        "lccc -Werror=discarded-qualifiers",
        classified(diagnose(lccc, "-Werror=discarded-qualifiers")),
        promoted,
    )
    checks += 1

    gcc = shutil.which(args.gcc)
    if gcc:
        gcc_diags = diagnose(gcc)
        problems += compare("gcc vs expect", classified(gcc_diags), want)
        checks += 1
        render = lambda d: "{line}:{col}: {sev}: {msg} [{opt}]".format(**d)  # noqa: E731
        got, ref = [render(d) for d in lccc_diags], [render(d) for d in gcc_diags]
        for text in sorted(set(got) ^ set(ref), key=lambda t: int(t.split(":")[0])):
            problems.append(f"{'lccc only' if text in got else 'gcc only '}: {text}")
        checks += 1
    else:
        print(f"note: {args.gcc} not found; checked LCCC against the expect comments only")

    for p in problems:
        print(f"FAIL {p}")
    print(f"qualifier-conversions: {len(want)} expected diagnostics, {checks} checks, "
          f"{'FAIL' if problems else 'PASS'}")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
