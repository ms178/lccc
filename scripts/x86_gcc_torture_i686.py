#!/usr/bin/env python3
"""Run GCC's native i686 (32-bit x86) C torture/execute corpus with LCCC.

This is the 32-bit counterpart of ``x86_gcc_torture.py``.  LCCC's 32-bit
backend is the ``lccc-i686`` binary; because a 32-bit object must be linked
with 32-bit CRT/C-runtime libs, we drive compilation **and** linking through
``lccc-i686`` directly (its driver invokes the standalone ``lccc-ld`` with the
``elf_i386`` code model and multilib CRT paths).  A local ``gcc -m32``
compile+run establishes native-host eligibility; GCC output is not compared
(gcc.c-torture/execute's contract is the exit status).

Sandbox / cross-host support:
  * ``LCCC_SYSROOT`` (read by the compiler itself): when set, lccc's CRT and
    multilib discovery probes the absolute candidates beneath that prefix
    first, e.g. an unpacked ``libc6-dev-i386`` tree at ``$LCCC_SYSROOT``.
  * ``--runner`` / ``GCC_I686_RUNNER``: argv prefix used to *execute* both
    the gcc reference and the LCCC-produced binaries (e.g.
    ``/path/qemu-i386``). Empty by default. Needed whenever the host cannot
    execute ELF32 binaries natively (pure amd64 seccomp hosts).
  * ``--append``: extra arguments appended to every compile+link invocation
    of BOTH compilers (e.g. ``-static`` so produced binaries need no i386
    dynamic loader). Applied after per-test dg-options so they win.

Examples:
  scripts/x86_gcc_torture_i686.py --flags=-O2 -j2
  scripts/x86_gcc_torture_i686.py pr110817-1.c pr23135.c --flags=-O0,-O2
  LCCC_SYSROOT=$HOME/i686-root \
      scripts/x86_gcc_torture_i686.py --runner $HOME/qemu-root/usr/bin/qemu-i386 \
          --append=-static --flags=-O0,-O1,-O2 -j4
"""
from __future__ import annotations

import argparse
import concurrent.futures
import dataclasses
import json
import os
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
import time
from collections import Counter
from pathlib import Path
from typing import Sequence

REPO = Path(__file__).resolve().parent.parent
DEFAULT_LCCC = REPO / "target" / "fastbuild" / "lccc-i686"
DEFAULT_SUITE = Path(os.environ.get(
    "GCC_TORTURE", "/home/user/src/gcc/gcc/testsuite/gcc.c-torture/execute"
))
DEFAULT_FLAGS = ("-O0", "-O1", "-O2", "-O3", "-Os")
_DIRECTIVE = re.compile(r"\{\s*dg-(?:additional-)?options\s+\"([^\"]*)\"([^}]*)\}")

# GCC multilib include dirs required for 32-bit system headers (stdarg.h etc.).
#
# The compiler-private include directory is derived from the host GCC rather
# than hard-coded to one major version: research hosts have shipped GCC 12,
# 14 and 16, and a stale `/usr/lib/gcc/<triple>/<N>/include` silently turns
# every test that needs <stddef.h>/<stdarg.h> into a bogus compile-fail.
def _gcc_private_include(gcc: str) -> list[str]:
    try:
        proc = subprocess.run([gcc, "-m32", "-print-file-name=include"],
                              capture_output=True, text=True, timeout=10, check=False)
        path = proc.stdout.strip()
        if proc.returncode == 0 and path and os.path.isdir(path):
            return ["-isystem", path]
    except (OSError, subprocess.SubprocessError):
        pass
    import glob as _glob
    candidates = sorted(
        _glob.glob("/usr/lib/gcc/x86_64-linux-gnu/*/include"),
        key=lambda p: int(p.split("/")[-2]) if p.split("/")[-2].isdigit() else -1,
    )
    return ["-isystem", candidates[-1]] if candidates else []


_I686_INCLUDES = [
    *_gcc_private_include(os.environ.get("GCC_BIN", "gcc")),
    "-isystem", "/usr/include/x86_64-linux-gnu",
    "-isystem", "/usr/include/i386-linux-gnu",
    "-isystem", "/usr/include",
]


@dataclasses.dataclass(frozen=True)
class Case:
    source: Path
    opt_flags: str
    directive_flags: tuple[str, ...]

    @property
    def key(self) -> str:
        return f"{self.source.name}[{self.opt_flags}]"


@dataclasses.dataclass
class Result:
    test: str
    flags: str
    directive_flags: list[str]
    status: str
    phase: str
    returncode: int
    seconds: float
    detail: str = ""


def run(command: Sequence[str], timeout: float, *, cwd: Path | None = None) -> subprocess.CompletedProcess[bytes]:
    try:
        return subprocess.run(command, cwd=cwd, capture_output=True, timeout=timeout, check=False)
    except subprocess.TimeoutExpired as exc:
        stdout = exc.stdout or b""
        stderr = (exc.stderr or b"") + f"\nTIMEOUT after {timeout:.0f}s".encode()
        return subprocess.CompletedProcess(command, 124, stdout, stderr)
    except OSError as exc:
        return subprocess.CompletedProcess(command, 125, b"", str(exc).encode())


def detail_of(proc: subprocess.CompletedProcess[bytes], limit: int = 6000) -> str:
    data = proc.stdout + proc.stderr
    return data.decode("utf-8", "replace")[-limit:]


def native_directive_flags(source: Path) -> tuple[str, ...]:
    text = source.read_text(errors="replace")[:8192]
    result: list[str] = []
    for match in _DIRECTIVE.finditer(text):
        trailer = match.group(2)
        if "target" in trailer:
            continue
        result.extend(shlex.split(match.group(1)))
    return tuple(result)


def execute_case(
    case: Case,
    *,
    lccc: Path,
    gcc: str,
    suite: Path,
    compile_timeout: float,
    run_timeout: float,
    runner: Sequence[str] = (),
    append: Sequence[str] = (),
) -> Result:
    started = time.monotonic()
    all_flags = [case.opt_flags, *case.directive_flags]
    common = ["-w", *all_flags, *list(append), *["-I", str(suite)], *_I686_INCLUDES]

    with tempfile.TemporaryDirectory(prefix="lccc-i686-gcc-torture-") as temp_name:
        temp = Path(temp_name)

        # Eligibility / behavioral reference with the host gcc -m32.
        reference = temp / "reference"
        proc = run(
            [gcc, "-m32", *common, str(case.source), "-lm", "-o", str(reference)],
            compile_timeout,
        )
        if proc.returncode:
            return Result(
                case.source.name, case.opt_flags, list(case.directive_flags),
                "reference-compile-skip", "reference-compile", proc.returncode,
                time.monotonic() - started, detail_of(proc),
            )
        proc = run([*runner, str(reference)], run_timeout)
        if proc.returncode:
            return Result(
                case.source.name, case.opt_flags, list(case.directive_flags),
                "reference-run-skip", "reference-run", proc.returncode,
                time.monotonic() - started, detail_of(proc),
            )

        # lccc-i686 drives compile + 32-bit link directly (its driver calls the
        # standalone lccc-ld with the elf_i386 model + multilib CRT).
        binary = temp / "lccc-i686"
        proc = run(
            [str(lccc), *common, str(case.source), "-lm", "-o", str(binary)],
            compile_timeout,
        )
        if proc.returncode:
            return Result(
                case.source.name, case.opt_flags, list(case.directive_flags),
                "compile-fail", "lccc-compile", proc.returncode,
                time.monotonic() - started, detail_of(proc),
            )

        proc = run([*runner, str(binary)], run_timeout)
        if proc.returncode:
            return Result(
                case.source.name, case.opt_flags, list(case.directive_flags),
                "run-fail", "execute", proc.returncode,
                time.monotonic() - started, detail_of(proc),
            )

    return Result(
        case.source.name, case.opt_flags, list(case.directive_flags),
        "pass", "complete", 0, time.monotonic() - started,
    )


def revision(path: Path) -> str | None:
    proc = run(["git", "-C", str(path), "rev-parse", "HEAD"], 10)
    return proc.stdout.decode().strip() if proc.returncode == 0 else None


def atomic_json(path: Path, payload: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + f".tmp.{os.getpid()}")
    temporary.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n")
    temporary.replace(path)


def discover(args: argparse.Namespace) -> list[Path]:
    available = {path.name: path for path in args.suite.glob("*.c")}
    if args.tests:
        selected: list[Path] = []
        missing: list[str] = []
        for item in args.tests:
            name = Path(item).name
            if not name.endswith(".c"):
                name += ".c"
            path = Path(item)
            if path.is_file():
                selected.append(path.resolve())
            elif name in available:
                selected.append(available[name])
            else:
                missing.append(item)
        if missing:
            raise ValueError("tests not found: " + ", ".join(missing))
    else:
        selected = sorted(available.values())
    if args.filter:
        pattern = re.compile(args.filter)
        selected = [path for path in selected if pattern.search(path.name)]
    return selected


def parse_args(argv: Sequence[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("tests", nargs="*", help="exact test paths/names (default: full execute corpus)")
    parser.add_argument("--suite", type=Path, default=DEFAULT_SUITE)
    parser.add_argument("--lccc", type=Path, default=DEFAULT_LCCC)
    parser.add_argument("--gcc", default=os.environ.get("GCC_BIN", "gcc"))
    parser.add_argument(
        "--runner",
        default=os.environ.get("GCC_I686_RUNNER", ""),
        help="argv prefix to execute compiled binaries (e.g. qemu-i386); "
             "default: env GCC_I686_RUNNER or none",
    )
    parser.add_argument(
        "--append",
        default=os.environ.get("GCC_I686_APPEND", ""),
        help="extra args appended to every compile (both compilers), "
             "comma-separated; default: env GCC_I686_APPEND or none",
    )
    parser.add_argument("--flags", default=",".join(DEFAULT_FLAGS),
                        help="comma-separated optimization configurations")
    parser.add_argument("--filter", default="", help="regular expression over basenames")
    parser.add_argument("-j", "--jobs", type=int, default=2)
    parser.add_argument("--compile-timeout", type=float, default=60)
    parser.add_argument("--run-timeout", type=float, default=10)
    parser.add_argument("--json", type=Path)
    parser.add_argument("--failure-log", type=Path)
    parser.add_argument("-v", "--verbose", action="store_true")
    return parser.parse_args(argv)


def main(argv: Sequence[str] | None = None) -> int:
    # Progress must be visible in `nohup ... > log` runs on the constrained
    # research host; default block buffering hides it for tens of minutes.
    sys.stdout.reconfigure(line_buffering=True)
    args = parse_args(argv)
    args.suite = args.suite.expanduser().resolve()
    args.lccc = args.lccc.expanduser().resolve()
    args.gcc = shutil.which(args.gcc) or args.gcc

    for label, path in (("suite", args.suite), ("lccc", args.lccc)):
        if not path.exists():
            print(f"error: {label} not found: {path}", file=sys.stderr)
            return 2
    try:
        sources = discover(args)
    except (ValueError, re.error) as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 2
    flags = [item.strip() for item in args.flags.split(",") if item.strip()]
    if not sources or not flags:
        print("error: no tests/configurations selected", file=sys.stderr)
        return 2
    runner = shlex.split(args.runner) if args.runner else []
    append = [item for item in args.append.split(",") if item]
    if (
        not append
        and os.environ.get("GCC_I686_APPEND") is None
        and os.environ.get("LCCC_TORTURE_STATIC") == "1"
    ):
        append = ["-static"]  # convenience switch for pure-amd64 sandbox hosts

    cases = [
        Case(source, opt, native_directive_flags(source))
        for source in sources
        for opt in flags
    ]
    print(f"suite:   {args.suite} ({len(sources)} sources)")
    print(f"lccc:    {args.lccc}")
    print(f"matrix:  {len(cases)} cases, jobs={max(1, args.jobs)}")

    started = time.monotonic()
    results: list[Result] = []
    counts: Counter[str] = Counter()
    with concurrent.futures.ThreadPoolExecutor(max_workers=max(1, args.jobs)) as pool:
        futures = {
            pool.submit(
                execute_case,
                case,
                lccc=args.lccc,
                gcc=args.gcc,
                suite=args.suite,
                compile_timeout=args.compile_timeout,
                run_timeout=args.run_timeout,
                runner=runner,
                append=append,
            ): case
            for case in cases
        }
        for index, future in enumerate(concurrent.futures.as_completed(futures), 1):
            result = future.result()
            results.append(result)
            counts[result.status] += 1
            if result.status not in {"pass", "reference-compile-skip", "reference-run-skip"}:
                print(f"{result.status.upper():<20} {result.test}[{result.flags}] rc={result.returncode}")
                if result.detail:
                    print("    " + result.detail.strip().splitlines()[-1])
            elif args.verbose:
                print(f"{result.status.upper():<20} {result.test}[{result.flags}]")
            elif index % 250 == 0:
                print(f"progress {index}/{len(cases)}: {dict(counts)}")

    results.sort(key=lambda item: (item.test, flags.index(item.flags)))
    elapsed = time.monotonic() - started
    payload = {
        "schema": 1,
        "arch": "i686",
        "suite": str(args.suite),
        "gcc_checkout_head": revision(args.suite.parents[3]),
        "lccc": str(args.lccc),
        "lccc_head": revision(REPO),
        "gcc": args.gcc,
        "flags": flags,
        "jobs": max(1, args.jobs),
        "elapsed_s": round(elapsed, 3),
        "counts": dict(sorted(counts.items())),
        "results": [dataclasses.asdict(result) for result in results],
    }
    if args.json:
        atomic_json(args.json, payload)
    if args.failure_log:
        args.failure_log.parent.mkdir(parents=True, exist_ok=True)
        with args.failure_log.open("w") as stream:
            for result in results:
                if result.status in {"pass", "reference-compile-skip", "reference-run-skip"}:
                    continue
                stream.write(
                    f"=== {result.status} {result.test}[{result.flags}] "
                    f"phase={result.phase} rc={result.returncode} ===\n"
                    f"{result.detail}\n"
                )

    print(f"\n== {len(cases)} cases in {elapsed:.1f}s == (i686 backend)")
    for status, count in sorted(counts.items()):
        print(f"{status:>24}: {count}")
    failures = sum(count for status, count in counts.items() if status.endswith("-fail"))
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
