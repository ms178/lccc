#!/usr/bin/env python3
"""Two-oracle interestingness test for ``c_reduce.py`` (T4).

Exit 0 means "the candidate still shows the bug"; anything else means "reject".

A candidate is only ever judged against the compiler under test *after* it
passes the reference oracle, because the reducer happily produces undefined
programs (deleting an initialiser, an index guard, a ``return``).  A miscompile
of a program with undefined behaviour is not a compiler bug, and a
nondeterministic program cannot be compared at all.  The reference oracle
therefore requires that the candidate:

1. compiles with the reference compiler (GCC, ``-O0``, UBSan and ASan with
   ``-fno-sanitize-recover=all``), so no undefined behaviour is executed;
2. runs to completion with exit status 0 under those sanitizers;
3. produces identical stdout on two consecutive runs (determinism).

Only then:

* ``--kind miscompile``: the compiler under test must compile the candidate,
  the binary must exit 0 and its stdout must equal the reference stdout;
* ``--kind crash``: the compiler under test must die on the candidate (a
  signal, exit status >= 128, or an internal-error marker on stderr).

Usage::

    c_reduce_predicate.py --ccc target/fastbuild/lccc --level=-O2 cand.c
"""
from __future__ import annotations

import argparse
import os
import subprocess
import sys
import tempfile
from pathlib import Path

REF_FLAGS = ["-std=gnu11", "-O0", "-w", "-fsanitize=undefined,address",
             "-fno-sanitize-recover=all", "-fno-omit-frame-pointer"]
# Messages a compiler prints when it hits an internal error.  A shell's
# "Segmentation fault" is deliberately absent: the shell prints it, not the
# compiler, and it is not evidence of a compiler bug on its own.
ICE_MARKERS = (b"panicked at", b"internal compiler error", b"assertion failed")


class ToolMissing(Exception):
    """A required tool cannot be executed: a usage or setup error, not a verdict."""


def run(cmd: list[str], timeout: float, env: dict[str, str] | None = None) -> subprocess.CompletedProcess[bytes] | None:
    """Run `cmd`.  None means the run timed out.  A tool that cannot be
    executed raises ToolMissing: treating it as a timeout would turn a wrong
    `--ccc` path into "every candidate is a crash" and reduce to nothing."""
    try:
        return subprocess.run(cmd, stdin=subprocess.DEVNULL, capture_output=True,
                              timeout=timeout, check=False, env=env)
    except subprocess.TimeoutExpired:
        return None
    except OSError as exc:
        raise ToolMissing(f"cannot execute {cmd[0]}: {exc}") from exc


def reference_output(ref: str, src: Path, work: Path, timeout: float) -> bytes | None:
    exe = work / "ref.exe"
    built = run([ref, *REF_FLAGS, str(src), "-o", str(exe), "-lm"], timeout)
    if built is None or built.returncode != 0:
        return None
    # LeakSanitizer is part of -fsanitize=address and reports ANY allocation
    # still live at exit as an error (exit status 1).  Leaking is not
    # undefined behaviour -- C11 makes no requirement to free -- so a leaking
    # but otherwise defined candidate must stay reducible: without
    # detect_leaks=0 the oracle rejects every unfreed-allocation candidate
    # and the reducer silently cannot minimise a leak-containing miscompile.
    # (Demonstrated: a main() that mallocs 4 KiB and returns 0 exits 1 under
    # the default environment, 0 with detect_leaks=0.)  ASan interceptors and
    # UBSan traps are unaffected by the knob; only the at-exit leak report
    # changes.
    run_env = {**os.environ, "ASAN_OPTIONS": "detect_leaks=0"}
    first = run([str(exe)], timeout, env=run_env)
    second = run([str(exe)], timeout, env=run_env)
    if first is None or second is None:
        return None
    if first.returncode != 0 or second.returncode != 0:
        return None
    if first.stdout != second.stdout:
        return None  # nondeterministic: uninitialised read or similar
    return first.stdout


def judge(args: argparse.Namespace, src: Path, work: Path) -> bool:
    ref_out = reference_output(args.ref, src, work, args.timeout)
    if ref_out is None:
        return False  # the reference oracle rejects it: not a compiler bug

    exe = work / "ccc.exe"
    comp = run([args.ccc, args.level, str(src), "-o", str(exe), "-lm"], args.timeout)
    if comp is None:
        return args.kind == "crash"  # a hang in the compiler is a crash-class bug
    if args.kind == "crash":
        if comp.returncode < 0 or comp.returncode >= 128:
            return True
        return any(marker in comp.stderr for marker in ICE_MARKERS)
    if comp.returncode != 0:
        return False  # rejected code is not a miscompile
    ran = run([str(exe)], args.timeout)
    if ran is None:
        return True  # the miscompiled program hangs while the reference does not
    return ran.returncode != 0 or ran.stdout != ref_out


def main(argv: list[str] | None = None) -> int:
    try:
        return _main(argv)
    except ToolMissing as exc:
        print(f"c_reduce_predicate: {exc}", file=sys.stderr)
        return 2  # not 0/1: the reducer aborts on it instead of reading a verdict


def _main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--ccc", required=True, help="compiler under test")
    ap.add_argument("--ref", default=os.environ.get("REF_CC", "gcc"),
                    help="reference C compiler (default: $REF_CC or gcc)")
    ap.add_argument("--level", default="-O2")
    ap.add_argument("--kind", choices=("miscompile", "crash"), default="miscompile")
    ap.add_argument("--timeout", type=float, default=20.0)
    ap.add_argument("candidate", type=Path)
    args = ap.parse_args(argv)
    with tempfile.TemporaryDirectory(prefix="c-reduce-pred-") as tmp:
        return 0 if judge(args, args.candidate, Path(tmp)) else 1


if __name__ == "__main__":
    sys.exit(main())
