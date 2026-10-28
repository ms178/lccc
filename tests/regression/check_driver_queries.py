#!/usr/bin/env python3
"""GP-21 driver query parity against GCC 16.2.

Every early-exit driver query the sentinel lists must answer the same way
GCC does: `-dumpfullversion`, `-print-multiarch`, `-print-multi-lib`,
`-print-sysroot` (plus the neighbouring `-print-multi-directory` and
`-print-prog-name=` probes).

Two rule classes keep the comparison honest:

* ``exact``    — stdout bytes must equal GCC's, including multi-line and
                  zero-byte answers (`gcc-16 -print-sysroot` really is
                  0 bytes, measured with `od -c`).
* ``identity`` — LCCC's documented GCC 14.2.0 identity is exempt from
                  matching GCC's own version *number* but must agree with
                  LCCC's `--version` inside the same binary, and both
                  compilers must return a dotted triple.

The sysroot *refusal* (`--sysroot=` rejected while GCC would answer the
path) is a documented driver contract, pinned here as such.
"""
from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def run(compiler: list[str], *args: str) -> subprocess.CompletedProcess:
    return subprocess.run(
        [*compiler, *args],
        cwd=ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=30,
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--lccc", default="target/fastbuild/lccc")
    parser.add_argument("--gcc", default="gcc-16")
    args = parser.parse_args()
    lccc = [args.lccc]
    gcc = [args.gcc]

    failures: list[str] = []

    def exact(label: str, argv: list[str]) -> None:
        g = run(gcc, *argv)
        lc = run(lccc, *argv)
        if g.returncode or lc.returncode:
            failures.append(
                f"{label}: nonzero rc gcc={g.returncode} lccc={lc.returncode} "
                f"(stderr {g.stderr.strip() or lc.stderr.strip()})"
            )
            return
        if lc.stdout != g.stdout:
            failures.append(
                f"{label}: stdout differs\ngcc : {g.stdout!r}\nlccc: {lc.stdout!r}"
            )
            return
        print(f"driver queries {label}: PASS")

    def empty_both(label: str, argv: list[str]) -> None:
        g = run(gcc, *argv)
        lc = run(lccc, *argv)
        if g.returncode or lc.returncode:
            failures.append(f"{label}: nonzero rc gcc={g.returncode} lccc={lc.returncode}")
            return
        if g.stdout != "":
            failures.append(f"{label}: GCC answer is no longer empty: {g.stdout!r}")
            return
        if lc.stdout != "":
            failures.append(f"{label}: LCCC must print nothing, got {lc.stdout!r}")
            return
        print(f"driver queries {label}: PASS")

    # --- identity rule: -dumpfullversion -----------------------------------
    ver = run(lccc, "--version")
    lccc_version = None
    if ver.returncode == 0:
        m = re.search(r"(\d+\.\d+\.\d+)", ver.stdout.splitlines()[0] if ver.stdout else "")
        if m:
            lccc_version = m.group(1)
    full = run(lccc, "-dumpfullversion")
    gfull = run(gcc, "-dumpfullversion")
    if lccc_version is None:
        failures.append(f"dumpfullversion: cannot parse LCCC --version: {ver.stdout!r}")
    elif full.returncode or not re.fullmatch(r"\d+\.\d+\.\d+\n", full.stdout or ""):
        failures.append(f"dumpfullversion: bad LCCC answer rc={full.returncode} {full.stdout!r}")
    elif full.stdout.strip() != lccc_version:
        failures.append(
            f"dumpfullversion: {full.stdout.strip()} disagrees with --version {lccc_version}"
        )
    elif not re.fullmatch(r"\d+\.\d+\.\d+\n", gfull.stdout or ""):
        failures.append(f"dumpfullversion: GCC answer changed: {gfull.stdout!r}")
    else:
        print("driver queries dumpfullversion: PASS (identity, GCC dotted)")

    # --- exact rules --------------------------------------------------------
    exact("print-multiarch", ["-print-multiarch"])
    exact("print-multiarch -m32", ["-m32", "-print-multiarch"])
    exact("print-multi-lib", ["-print-multi-lib"])
    exact("print-multi-lib -m32", ["-m32", "-print-multi-lib"])
    exact("print-multi-directory", ["-print-multi-directory"])
    exact("print-multi-directory -m32", ["-m32", "-print-multi-directory"])
    exact("print-prog-name=c++", ["-print-prog-name=c++"])
    empty_both("print-sysroot (no --sysroot)", ["-print-sysroot"])

    # --- documented refusal contract ----------------------------------------
    g = run(gcc, "--sysroot=/tmp/lccc-gp21-sysroot", "-print-sysroot")
    lc = run(lccc, "--sysroot=/tmp/lccc-gp21-sysroot", "-print-sysroot")
    if g.returncode != 0 or g.stdout.strip() != "/tmp/lccc-gp21-sysroot":
        failures.append(
            f"sysroot contract: GCC behaviour changed rc={g.returncode} {g.stdout!r}"
        )
    elif lc.returncode == 0:
        failures.append("sysroot contract: LCCC must keep refusing --sysroot")
    elif "sysroot is not supported" not in lc.stderr:
        failures.append(f"sysroot contract: refusal wording changed: {lc.stderr!r}")
    else:
        print("driver queries sysroot refusal (documented): PASS")

    # --- documented known divergence ----------------------------------------
    gdm = run(gcc, "-dumpmachine").stdout.strip()
    ldm = run(lccc, "-dumpmachine").stdout.strip()
    print(
        "driver queries -dumpmachine: DOCUMENTED "
        f"(gcc configured {gdm!r} vs lccc active-canonical {ldm!r}; not a sentinel query)"
    )

    if failures:
        for line in failures:
            print(f"ERROR: {line}", file=sys.stderr)
        return 1
    print("driver queries: PASS (GCC 16.2.0)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
