#!/usr/bin/env python3
"""Pointer type rules of conversions, conditionals and comparisons against GCC 16.2.

`qualifier_conversions.c` covers the qualifier rules of implicit pointer
conversions (C23 6.5.16.1p1), the pointer arms of `?:` (6.5.15p3) and pointer
comparisons (6.5.8p2, 6.5.9p2). It marks every line GCC diagnoses with an
`/* expect: <severity> <option> */` comment (`none` for a warning without an
option); every other line must stay silent. The gate checks:

  * LCCC against the comments (line, severity, option), always;
  * GCC against the comments, and LCCC against GCC byte for byte (line,
    column, severity, text, option), with the required GCC oracle (optional only by explicit local opt-out);
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
from dataclasses import dataclass

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


@dataclass(frozen=True)
class DiagnosticResult:
    returncode: int
    diagnostics: list[dict[str, str]]


def parse_result(proc: subprocess.CompletedProcess, source: Path) -> DiagnosticResult:
    """Fail closed on unexpected failures, not merely on missing expected rows."""
    diags = []
    unexpected = [f"unexpected stdout: {proc.stdout!r}"] if proc.stdout.strip() else []
    summaries = []
    gcc_summaries = 0
    promoted_error = False
    source_lines = re.sub(r"/\*.*?\*/", lambda m: re.sub(r"[^\n]", " ", m[0]),
                          source.read_text() if source.exists() else "", flags=re.S).splitlines()
    excerpt = None
    for line in proc.stderr.splitlines():
        match = DIAG.fullmatch(line)
        if match:
            if match.group("file") != source.name:
                unexpected.append(line)
            else:
                diags.append(match.groupdict())
                promoted_error |= match.group("sev") == "error" and "[-Werror=" in line
                number = int(match.group("line"))
                excerpt = (" ".join(source_lines[number - 1].split())
                           if 0 < number <= len(source_lines) else None)
            continue
        if line in ("cc1: some warnings being treated as errors",
                    "cc1: all warnings being treated as errors"):
            gcc_summaries += 1
            continue
        summary = re.fullmatch(
            r"ccc: error: (\d+) error\(s\) (?:during semantic analysis|"
            r"\(warnings promoted by -Werror\))", line)
        if summary:
            summaries.append(int(summary[1]))
            continue
        if excerpt is not None and line.startswith(" ") and " ".join(line.split()) == excerpt:
            excerpt = None
            continue
        # The compiler's source excerpts, carets, context and secondary notes
        # are not primary diagnostics. Do not silently discard driver errors,
        # panics, ICEs, diagnostics without columns, or arbitrary stderr.
        if (not line.strip()
                or re.match(r"^\s*(?:[0-9]+\s*)?\|", line)
                or re.match(r"^\s*[~^]+\s*$", line)
                or re.match(r"^[^:]+:\d+:\d+: note: ", line)
                or re.match(r"^[^:]+: (?:In function |At top level:)", line)):
            continue
        unexpected.append(line)
    errors = sum(d["sev"] == "error" for d in diags)
    if gcc_summaries > 1 or (gcc_summaries and not promoted_error):
        unexpected.append("GCC promotion summary without a promoted primary error")
    if len(summaries) > 1 or any(count != errors for count in summaries):
        unexpected.append(f"inconsistent error summaries: {summaries}, primary errors: {errors}")
    expected_rc = int(errors != 0)
    if proc.returncode != expected_rc or unexpected:
        raise SystemExit(
            f"unexpected compiler result: exit {proc.returncode}, expected {expected_rc}; "
            f"unparsed output: {unexpected!r}\n{proc.stderr}"
        )
    return DiagnosticResult(proc.returncode, diags)


def diagnose(compiler: str, *flags: str, std: str = "gnu17",
             source: Path = SOURCE) -> list[dict[str, str]]:
    proc = subprocess.run(
        [compiler, "-std=" + std, "-fsyntax-only", *flags, source.name],
        cwd=source.parent, env=ENV, capture_output=True, text=True, timeout=120,
    )
    return parse_result(proc, source).diagnostics


def oracle_path(name: str, allow_missing: bool) -> str | None:
    gcc = shutil.which(name)
    if gcc is None:
        if allow_missing:
            return None
        raise SystemExit(f"required GCC 16.2.0 oracle not found: {name}")
    version = subprocess.run([gcc, "-dumpfullversion"], env=ENV,
                             capture_output=True, text=True, timeout=20)
    if version.returncode != 0 or version.stdout.strip() != "16.2.0" or version.stderr:
        raise SystemExit(f"expected GCC 16.2.0, got: {version.stdout!r} {version.stderr!r}")
    return gcc


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
    parser.add_argument("--allow-missing-oracle", action="store_true",
                        help="explicit local expectation-only mode; never used by CI")
    args = parser.parse_args()
    lccc = str(Path(args.lccc).resolve())
    gcc = oracle_path(args.gcc, args.allow_missing_oracle)
    want = expectations()
    if not want:
        raise SystemExit("empty diagnostic expectation corpus")
    problems: list[str] = []
    checks = 0

    variants = [("baseline", (), want)]
    for option in sorted({opt for _, opt in want.values()} - {"none"}):
        variants.append((f"-Wno-{option}", (f"-Wno-{option}",),
                         {k: v for k, v in want.items() if v[1] != option}))
    promoted = {k: (("error", v[1]) if v[1] == "discarded-qualifiers" else v)
                for k, v in want.items()}
    variants.append(("-Werror=discarded-qualifiers", ("-Werror=discarded-qualifiers",), promoted))
    for name, flags, expected in variants:
        actual = diagnose(lccc, *flags)
        problems += compare(f"lccc {name}", classified(actual), expected)
        checks += 1
        if gcc:
            reference = diagnose(gcc, *flags)
            problems += compare(f"gcc {name}", classified(reference), expected)
            render = lambda d: "{line}:{col}: {sev}: {msg} [{opt}]".format(**d)
            got, ref = list(map(render, actual)), list(map(render, reference))
            if got != ref:
                # Compare ordered diagnostics, not sets that hide reordering.
                import difflib
                problems += [f"{name}: {line}" for line in
                             difflib.unified_diff(ref, got, fromfile="gcc", tofile="lccc", lineterm="")]
            checks += 2
    if not gcc:
        print(f"explicit expectation-only mode: {args.gcc} is unavailable")

    for p in problems:
        print(f"FAIL {p}")
    print(f"qualifier-conversions: {len(want)} expected diagnostics, {checks} checks, "
          f"{'FAIL' if problems else 'PASS'}")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
