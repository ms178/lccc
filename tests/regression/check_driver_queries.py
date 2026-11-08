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
import os
import re
import subprocess
import sys
import tempfile
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

    def refused_options(line: str) -> list[str]:
        """The options of a `-print-multi-lib` line (`dir;@opt@opt`) that
        lccc refuses to compile with, proven by asking lccc each time."""
        opts = [o for o in line.partition(";")[2].split("@") if o]
        return [o for o in opts if run(lccc, f"-{o}", "-E", "-x", "c", "/dev/null").returncode != 0]

    def multilibs_lccc_builds(out: str) -> str:
        """GCC's multilib list minus the variants lccc cannot build. A GCC
        configured with x32 (`x32;@mx32`, e.g. the Compiler Explorer build)
        lists it; lccc rejects -mx32, and advertising it would send
        multilib-probing build systems into a guaranteed failure."""
        kept = [ln for ln in out.splitlines(keepends=True) if not refused_options(ln.rstrip("\n"))]
        return "".join(kept)

    def exact(label: str, argv: list[str]) -> None:
        g = run(gcc, *argv)
        lc = run(lccc, *argv)
        if "-print-multi-lib" in argv and g.returncode == 0:
            g = subprocess.CompletedProcess(g.args, 0, multilibs_lccc_builds(g.stdout), g.stderr)
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

    # --- selected target's real linker runtime, including multilib ----------
    # No host files are modified. Different synthetic roots distinguish all
    # four architectures and make an x86-64 path returned for i686 fail.
    # The marker files are intentionally not linked; this checks discovery,
    # not archive contents or cross-compiler runtime ABI.
    with tempfile.TemporaryDirectory(prefix="lccc-query-sysroot-") as tmp:
        root = Path(tmp)
        bases = {
            "x86-64": "usr/lib/gcc/x86_64-linux-gnu/99.0",
            "i686": "usr/lib/gcc-cross/i686-linux-gnu/99.0",
            "aarch64": "usr/lib/gcc-cross/aarch64-linux-gnu/99.0",
            "riscv64": "usr/lib/gcc-cross/riscv64-linux-gnu/99.0",
        }
        for subdir in bases.values():
            directory = root / subdir
            directory.mkdir(parents=True, exist_ok=True)
            (directory / "crtbegin.o").write_bytes(b"marker")
            (directory / "libgcc.a").write_bytes(b"marker")
        env = {**os.environ, "LCCC_SYSROOT": str(root)}
        binary_dir = Path(args.lccc).resolve().parent
        cases = [
            ("x86-64", [args.lccc]),
            ("i686", [args.lccc, "-m32"]),
            ("i686", [str(binary_dir / "lccc-i686")]),
            ("aarch64", [str(binary_dir / "lccc-arm")]),
            ("riscv64", [str(binary_dir / "lccc-riscv")]),
        ]
        for target, command in cases:
            expected = str(root / bases[target] / "libgcc.a") + "\n"
            for flag in ("-print-libgcc-file-name", "-print-file-name=libgcc.a"):
                p = subprocess.run([*command, flag], cwd=ROOT, env=env,
                                   text=True, capture_output=True, timeout=30)
                if p.returncode or p.stdout != expected or p.stderr:
                    failures.append(f"{target} {flag}: expected {expected!r}, "
                                    f"got rc={p.returncode} {p.stdout!r} {p.stderr!r}")
                else:
                    print(f"driver queries {target} {flag}: PASS (target sysroot)")
        # A shared-only installation does not make the static archive exist.
        (root / bases["x86-64"] / "libgcc.a").unlink()
        (root / bases["x86-64"] / "libgcc_s.so").write_bytes(b"marker")
        for flag in ("-print-libgcc-file-name", "-print-file-name=libgcc.a"):
            p = subprocess.run([args.lccc, flag], cwd=ROOT, env=env,
                               text=True, capture_output=True, timeout=30)
            if p.returncode or p.stdout != "libgcc.a\n" or p.stderr:
                failures.append(f"missing static archive {flag}: rc={p.returncode} "
                                f"stdout={p.stdout!r} stderr={p.stderr!r}")
            else:
                print(f"driver queries {flag}: PASS (missing static archive)")

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
