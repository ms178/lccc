#!/usr/bin/env python3
"""`-Wreturn-local-addr` against GCC 16.2: diagnostics and code generation.

`fixtures/return_local_addr/return_local_addr.c` marks every line GCC's
front end diagnoses with `/* expect: warning return-local-addr */`; every
other line must stay silent. The gate checks:

  * GCC (`-fsyntax-only`) against the comments, then LCCC against GCC byte
    for byte (file, line, column, severity, text, option) for every
    diagnostic, under `-fsyntax-only` and when compiling at -O0 and -O2 (the
    front-end verdicts must not depend on the optimization level);
  * `-Wno-return-local-addr` silences exactly those lines and
    `-Werror=return-local-addr` makes them errors, in both compilers;
  * run time: a local-variable return yields a null pointer after its side
    effects (GCC's code generation at every level), while a label address
    and a nested function returning its parent's local keep their values,
    natively and with -m32 -- same output from GCC and LCCC.
"""

from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
# Ill-formed by design (it warns), so it stays out of run_regression.py's glob.
SOURCE = HERE / "fixtures" / "return_local_addr" / "return_local_addr.c"
OPTION = "return-local-addr"
ENV = {**os.environ, "LC_ALL": "C"}
EXPECT = re.compile(r"/\* expect: warning return-local-addr \*/")
DIAG = re.compile(
    r"^(?P<file>[^:\s]+):(?P<line>\d+):(?P<col>\d+): (?P<sev>warning|error): "
    r"(?P<msg>.*?)(?: \[-W(?P<werror>error=)?(?P<opt>[a-z-]+)\])?$"
)
# LCCC's own closing line for a run with errors (GCC prints `cc1: ...`).
LCCC_SUMMARY = re.compile(r"l?ccc: error: \d+ error\(s\) during semantic analysis")
SUMMARY = {
    "cc1: some warnings being treated as errors",
    "cc1: all warnings being treated as errors",
}

RUNTIME = r"""
#include <stdio.h>
static int counter;
__attribute__((noinline)) static int *local(void) { int x = 7; return &x; }
__attribute__((noinline)) static int *indexed(int *k) {
    int a[4] = {0}; return &a[(*k)++];
}
__attribute__((noinline)) static char *cast(void) {
    int x = 0; return (char *)&x + 1;
}
__attribute__((noinline)) static int *bump(void) { counter++; return &counter; }
__attribute__((noinline)) static int *chosen(void) {
    int x = 0; return 1 ? &x : bump();
}
__attribute__((noinline)) static void *label(void) { l: return &&l; }
__attribute__((noinline)) static int nested(void) {
    int x = 42;
    int *inner(void) { return &x; }
    int *p = inner();
    return p ? *p : -1;
}
int main(void) {
    int k = 1;
    int *p = indexed(&k);
    printf("local=%s\n", local() ? "live" : "null");
    printf("indexed=%s k=%d\n", p ? "live" : "null", k);
    printf("cast=%s\n", cast() ? "live" : "null");
    printf("chosen=%s counter=%d\n", chosen() ? "live" : "null", counter);
    printf("label=%s\n", label() ? "live" : "null");
    printf("nested=%d\n", nested());
    return 0;
}
"""
RUNTIME_EXPECTED = (
    "local=null\nindexed=null k=2\ncast=null\nchosen=null counter=0\n"
    "label=live\nnested=42\n"
)


def expectations() -> set[int]:
    return {
        number
        for number, text in enumerate(SOURCE.read_text().splitlines(), 1)
        if EXPECT.search(text)
    }


def diagnostics(cmd: list[str], *, want_failure: bool = False) -> list[tuple]:
    proc = subprocess.run(cmd, capture_output=True, text=True, env=ENV, check=False)
    rows, unexpected = [], []
    for line in proc.stderr.splitlines():
        match = DIAG.fullmatch(line)
        if match and match.group("file") == SOURCE.name:
            rows.append(
                (
                    int(match.group("line")),
                    int(match.group("col")),
                    match.group("sev"),
                    match.group("msg"),
                    bool(match.group("werror")),
                    match.group("opt"),
                )
            )
        elif match:
            unexpected.append(line)
        elif line in SUMMARY or LCCC_SUMMARY.fullmatch(line) or not line.strip() or line.startswith(SOURCE.name + ": In function"):
            continue
        elif not line[:1].isspace():
            # Source excerpts and carets (GCC's `  12 | ...` gutter, LCCC's
            # indented line) start with whitespace; anything else is not a
            # diagnostic this gate understands.
            unexpected.append(line)
    if proc.stdout.strip():
        unexpected.append(f"stdout: {proc.stdout.strip()!r}")
    failed = proc.returncode != 0
    if failed != want_failure:
        unexpected.append(f"exit status {proc.returncode} (want {'failure' if want_failure else 'success'})")
    if unexpected:
        raise RuntimeError(f"{' '.join(cmd)}:\n  " + "\n  ".join(unexpected))
    return rows


def compare(label: str, want: list[tuple], got: list[tuple]) -> list[str]:
    if want == got:
        return []
    missing = [row for row in want if row not in got]
    extra = [row for row in got if row not in want]
    out = [f"{label}: LCCC differs from GCC"]
    out += [f"  GCC only:  {row}" for row in missing[:20]]
    out += [f"  LCCC only: {row}" for row in extra[:20]]
    if not missing and not extra:
        out.append("  same rows, different order")
    return out


def run_program(compiler: list[str], source: Path, exe: Path, flags: list[str]) -> str:
    build = subprocess.run(
        [*compiler, *flags, "-w", str(source), "-o", str(exe)],
        capture_output=True, text=True, env=ENV, check=False,
    )
    if build.returncode != 0:
        raise RuntimeError(f"{' '.join(compiler)} {' '.join(flags)}: build failed\n{build.stderr}")
    run = subprocess.run([str(exe)], capture_output=True, text=True, check=False, timeout=30)
    if run.returncode != 0:
        raise RuntimeError(f"{exe} {' '.join(flags)}: exit {run.returncode}")
    return run.stdout


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lccc", default="target/fastbuild/lccc")
    parser.add_argument("--gcc", default="gcc-16")
    parser.add_argument("--m32", action="store_true", help="also run the i686 execution rows")
    args = parser.parse_args()
    # Resolve a path-like compiler before changing directory.
    lccc = [str(Path(args.lccc).resolve()) if os.sep in args.lccc else args.lccc]
    gcc = [str(Path(args.gcc).resolve()) if os.sep in args.gcc else args.gcc]
    expected = expectations()
    failures: list[str] = []
    cases = 0
    with tempfile.TemporaryDirectory() as tmp:
        # Diagnostics quote the file name as given: compile from its
        # directory so both compilers print the bare name. GCC states the
        # rule; the comments must agree with it.
        os.chdir(SOURCE.parent)
        oracle = diagnostics([*gcc, "-fsyntax-only", SOURCE.name])
        flagged = {row[0] for row in oracle if row[5] == OPTION}
        if flagged != expected:
            failures.append(
                f"GCC vs comments: GCC-only lines {sorted(flagged - expected)}, "
                f"comment-only lines {sorted(expected - flagged)}"
            )
        cases += 1
        for mode in (["-fsyntax-only"], ["-O0", "-S", "-o", os.devnull], ["-O2", "-S", "-o", os.devnull]):
            got = diagnostics([*lccc, *mode, SOURCE.name])
            failures += compare(f"lccc {' '.join(mode)}", oracle, got)
            cases += 1
        # Option handling, two-sided.
        silenced = [row for row in oracle if row[5] != OPTION]
        for cc, name in ((gcc, "gcc"), (lccc, "lccc")):
            got = diagnostics([*cc, "-fsyntax-only", f"-Wno-{OPTION}", SOURCE.name])
            failures += compare(f"{name} -Wno-{OPTION}", silenced, got)
            promoted = [
                (line, col, "error", msg, True, opt) if opt == OPTION else (line, col, sev, msg, werr, opt)
                for (line, col, sev, msg, werr, opt) in oracle
            ]
            got = diagnostics([*cc, "-fsyntax-only", f"-Werror={OPTION}", SOURCE.name], want_failure=True)
            failures += compare(f"{name} -Werror={OPTION}", promoted, got)
            cases += 2
        # Code generation.
        program = Path(tmp) / "rla_run.c"
        program.write_text(RUNTIME)
        abis = [[]] + ([["-m32"]] if args.m32 else [])
        for abi in abis:
            for level in ("-O0", "-O2"):
                flags = [*abi, level]
                for cc, name in ((gcc, "gcc"), (lccc, "lccc")):
                    out = run_program(cc, program, Path(tmp) / f"{name}.out", flags)
                    if out != RUNTIME_EXPECTED:
                        failures.append(f"{name} {' '.join(flags)} run:\n{out}")
                    cases += 1
    if failures:
        print("return-local-addr gate: FAIL", file=sys.stderr)
        print("\n".join(failures), file=sys.stderr)
        return 1
    print(
        f"return-local-addr gate: PASS ({len(expected)} diagnosed lines, "
        f"{sum(1 for _ in SOURCE.read_text().splitlines())} fixture lines, {cases} checks)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
