#!/usr/bin/env python3
"""Unified GCC C Torture & Execution Test Harness for LCCC (x86-64 and i686).

Runs GCC's native gcc.c-torture/execute test suite with LCCC across multiple
optimization levels and target architectures.

Architectures Supported:
  - x86_64 (64-bit x86, default): LCCC compiler + lccc-ld standalone linker.
  - i686   (32-bit x86): lccc-i686 compiler + lccc-ld standalone linker
    (`-m elf_i386`) + multilib CRT; --link-mode=driver keeps the legacy
    one-step lccc-i686 compile+link through its built-in linker.

Corpus: scripts/ensure_gcc_torture.sh provisions gcc.c-torture (GCC 16.2.0 by
default) into the snapshot-excluded ~/.cache zone; GCC_TORTURE overrides.

Features:
  - Multi-threaded execution pool.
  - Reference compiler validation (GCC) to establish host/test eligibility.
  - Sandbox and cross-execution support (--runner, e.g. qemu-i386).
  - JSON reporting and automated failure triage logs.
  - Failure-focused re-runs: --from-list FILE restricts the corpus to the
    test names listed in a previous report (one JSON `test` value per line).

Examples:
  scripts/x86_gcc_torture.py --flags=-O2 -j2
  scripts/x86_gcc_torture.py --arch=i686 --flags=-O2 -j2
  scripts/x86_gcc_torture.py 20080604-1.c pr51933.c --flags=-O0,-O2,-Os
  scripts/x86_gcc_torture.py --filter '^(pr12|va-arg)' --json results.json
  scripts/x86_gcc_torture.py --from-list failing.txt --json rerun.json
"""
from __future__ import annotations

import argparse
import concurrent.futures
import dataclasses
import glob
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
DEFAULT_LCCC_X86_64 = REPO / "target" / "fastbuild" / "lccc"
DEFAULT_LCCC_I686 = REPO / "target" / "fastbuild" / "lccc-i686"
DEFAULT_LD = REPO / "target" / "fastbuild" / "lccc-ld"
def _default_suite() -> Path:
    """GCC_TORTURE, else the scripts/ensure_gcc_torture.sh location.

    The provisioning script extracts into the snapshot-excluded ``.cache``
    zone (the ~22k-file corpus would blow the workspace snapshot's file cap);
    the pre-2026-09-27 persisted-zone location is still honoured when it is
    the only one present.
    """
    env = os.environ.get("GCC_TORTURE")
    if env:
        return Path(env)
    candidates = (
        Path(os.environ.get("GCC_TESTSUITE_ROOT", "/home/user/.cache/lccc-gcc-testsuite"))
        / "gcc.c-torture" / "execute",
        Path("/home/user/src/gcc/gcc/testsuite/gcc.c-torture/execute"),
    )
    for candidate in candidates:
        if candidate.is_dir():
            return candidate
    return candidates[0]


DEFAULT_SUITE = _default_suite()
DEFAULT_FLAGS = ("-O0", "-O1", "-O2", "-O3", "-Os")
_DIRECTIVE = re.compile(r"\{\s*dg-(?:additional-)?options\s+\"([^\"]*)\"([^}]*)\}")
# DejaGnu `{ dg-timeout-factor N }`: gcc's harness scales every timeout of
# the test by N (the memcpy-ax/memclr expansions ask for 4-8x).
_TIMEOUT_FACTOR = re.compile(r"\{\s*dg-timeout-factor\s+([0-9.]+)\s*\}")
# `{ dg-require-effective-target run_expensive_tests }`: gcc runs the test
# only when GCC_TEST_RUN_EXPENSIVE is set and reports UNSUPPORTED otherwise
# (target-supports.exp: check_effective_target_run_expensive_tests).
_EXPENSIVE = re.compile(r"\{\s*dg-require-effective-target\s+run_expensive_tests\b")
# Statuses that are neither a pass nor an lccc failure.
QUIET_STATUSES = frozenset({"pass", "reference-compile-skip", "reference-run-skip", "unsupported"})


def _gcc_private_include(gcc: str, m32: bool = False) -> list[str]:
    cmd = [gcc]
    if m32:
        cmd.append("-m32")
    cmd.append("-print-file-name=include")
    try:
        proc = subprocess.run(cmd, capture_output=True, text=True, timeout=10, check=False)
        path = proc.stdout.strip()
        if proc.returncode == 0 and path and os.path.isdir(path):
            return ["-isystem", path]
    except (OSError, subprocess.SubprocessError):
        pass
    candidates = sorted(
        glob.glob("/usr/lib/gcc/x86_64-linux-gnu/*/include"),
        key=lambda p: int(p.split("/")[-2]) if p.split("/")[-2].isdigit() else -1,
    )
    return ["-isystem", candidates[-1]] if candidates else []


@dataclasses.dataclass(frozen=True)
class Case:
    source: Path
    opt_flags: str
    directive_flags: tuple[str, ...]
    timeout_factor: float = 1.0
    expensive: bool = False

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


def directive_timeout_factor(source: Path) -> float:
    match = _TIMEOUT_FACTOR.search(source.read_text(errors="replace")[:8192])
    try:
        return max(1.0, float(match.group(1))) if match else 1.0
    except ValueError:
        return 1.0


def requires_expensive(source: Path) -> bool:
    return _EXPENSIVE.search(source.read_text(errors="replace")[:8192]) is not None


def execute_case(
    case: Case,
    *,
    arch: str,
    lccc: Path,
    lccc_ld: Path,
    gcc: str,
    suite: Path,
    compile_timeout: float,
    run_timeout: float,
    runner: list[str],
    append_args: list[str],
    link_mode: str = "lccc-ld",
    run_expensive: bool = False,
) -> Result:
    started = time.monotonic()
    if case.expensive and not run_expensive:
        return Result(
            case.source.name, case.opt_flags, list(case.directive_flags),
            "unsupported", "run_expensive_tests", 0, 0.0,
            "dg-require-effective-target run_expensive_tests "
            "(set GCC_TEST_RUN_EXPENSIVE=1 or pass --expensive)",
        )
    compile_timeout *= case.timeout_factor
    run_timeout *= case.timeout_factor
    all_flags = [case.opt_flags, *case.directive_flags, *append_args]
    is_32 = arch in ("i686", "i386", "x86_32")

    includes: list[str] = ["-I", str(suite)]
    if is_32:
        includes.extend(_gcc_private_include(gcc, m32=True))
        includes.extend([
            "-isystem", "/usr/include/x86_64-linux-gnu",
            "-isystem", "/usr/include/i386-linux-gnu",
            "-isystem", "/usr/include",
        ])

    common = ["-w", *all_flags, *includes]

    with tempfile.TemporaryDirectory(prefix="lccc-gcc-torture-") as temp_name:
        temp = Path(temp_name)

        # 1. GCC Reference Build and Execution
        reference = temp / "reference"
        gcc_cmd = [gcc]
        if is_32:
            gcc_cmd.append("-m32")
        gcc_cmd.extend([*common, str(case.source), "-lm", "-o", str(reference)])

        proc = run(gcc_cmd, compile_timeout)
        if proc.returncode:
            return Result(
                case.source.name, case.opt_flags, list(case.directive_flags),
                "reference-compile-skip", "reference-compile", proc.returncode,
                time.monotonic() - started, detail_of(proc),
            )

        ref_exec_cmd = [*runner, str(reference)]
        proc = run(ref_exec_cmd, run_timeout)
        if proc.returncode:
            return Result(
                case.source.name, case.opt_flags, list(case.directive_flags),
                "reference-run-skip", "reference-run", proc.returncode,
                time.monotonic() - started, detail_of(proc),
            )

        # 2. LCCC Compilation and Link
        binary = temp / "lccc_bin"

        if is_32 and link_mode == "driver":
            # Legacy i686 leg: lccc-i686 compiles AND links through its
            # built-in i686 linker in one invocation.
            lccc_cmd = [str(lccc), *common, str(case.source), "-lm", "-o", str(binary)]
            proc = run(lccc_cmd, compile_timeout)
            if proc.returncode:
                return Result(
                    case.source.name, case.opt_flags, list(case.directive_flags),
                    "compile-fail", "lccc-i686", proc.returncode,
                    time.monotonic() - started, detail_of(proc),
                )
        else:
            # Default for both arches: compile with lccc, then link through
            # GCC's driver with the standalone lccc-ld substituted for `ld`
            # (`-B<shim>`), so the linker sees exactly the argv GNU ld would
            # (`-m elf_x86_64` / `-m elf_i386`, positional CRT objects,
            # --push-state/--as-needed -lgcc_s, --dynamic-linker, ...).
            # -no-pie: the reference build is PIE on Debian-style hosts, but
            # the lccc objects are non-PIC and lccc-ld emits ET_EXEC for i386.
            obj = temp / "test.o"
            proc = run(
                [str(lccc), *common, "-c", str(case.source), "-o", str(obj)],
                compile_timeout,
            )
            if proc.returncode:
                return Result(
                    case.source.name, case.opt_flags, list(case.directive_flags),
                    "compile-fail", "lccc-i686" if is_32 else "lccc-compile",
                    proc.returncode, time.monotonic() - started, detail_of(proc),
                )

            shim = temp / "ld-shim"
            shim.mkdir()
            (shim / "ld").symlink_to(lccc_ld)
            link_cmd = [gcc, *(["-m32"] if is_32 else []), "-no-pie", f"-B{shim}",
                        str(obj), "-lm", "-o", str(binary)]
            proc = run(link_cmd, compile_timeout)
            if proc.returncode:
                return Result(
                    case.source.name, case.opt_flags, list(case.directive_flags),
                    "link-fail", "lccc-ld", proc.returncode,
                    time.monotonic() - started, detail_of(proc),
                )

        # 3. LCCC Execution
        lccc_exec_cmd = [*runner, str(binary)]
        proc = run(lccc_exec_cmd, run_timeout)
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


def _testsuite_version(suite: Path) -> str | None:
    """Release stamp written by scripts/ensure_gcc_torture.sh (e.g. gcc-16.2.0)."""
    try:
        return (suite.parents[1] / ".lccc-provisioned").read_text().strip() or None
    except (OSError, IndexError):
        return None


def revision(path: Path) -> str | None:
    proc = run(["git", "-C", str(path), "rev-parse", "HEAD"], 10)
    return proc.stdout.decode().strip() if proc.returncode == 0 else None


def atomic_json(path: Path, payload: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + f".tmp.{os.getpid()}")
    temporary.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n")
    temporary.replace(path)


def read_test_list(path: Path) -> list[str]:
    """Names from a --from-list file: one test name per line.

    Blank lines and ``#`` comments are ignored; surrounding whitespace is
    stripped.  Names are plain ``test``-field values (e.g. ``20180112-1`` or
    ``20180112-1.c``) -- no per-test flag syntax is interpreted.
    """
    try:
        text = path.read_text()
    except OSError as exc:
        raise ValueError(f"cannot read --from-list file: {exc}") from exc
    names: list[str] = []
    for line in text.splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        names.append(line)
    return names


def resolve_test_name(item: str, available: dict[str, Path]) -> Path | None:
    """Resolve one --from-list entry to a suite source (or None if unknown).

    Accepts the JSON ``test`` value with or without the ``.c`` suffix, and
    (like the positional ``tests`` arguments) an explicit file path.
    """
    name = Path(item).name
    if not name.endswith(".c"):
        name += ".c"
    path = Path(item)
    if path.is_file():
        return path.resolve()
    if name in available:
        return available[name]
    return None


def discover(args: argparse.Namespace) -> list[Path]:
    available = {path.name: path for path in args.suite.glob("*.c")}
    selected: list[Path] | None = None
    if args.tests:
        selected = []
        missing: list[str] = []
        for item in args.tests:
            path = resolve_test_name(item, available)
            if path is not None:
                selected.append(path)
            else:
                missing.append(item)
        if missing:
            raise ValueError("tests not found: " + ", ".join(missing))
    if args.from_list:
        picked: list[Path] = []
        unknown: list[str] = []
        seen: set[Path] = set()
        for item in read_test_list(args.from_list):
            path = resolve_test_name(item, available)
            if path is None:
                unknown.append(item)  # lists go stale: warn and skip, not an error
            elif path not in seen:
                seen.add(path)
                picked.append(path)
        for item in unknown:
            print(f"warning: --from-list test not found, skipping: {item}", file=sys.stderr)
        if selected is None:
            selected = picked
        else:
            # --from-list composes with positional tests as an intersection.
            keep = set(picked)
            selected = [path for path in selected if path in keep]
    if selected is None:
        selected = sorted(available.values())
    if args.filter:
        pattern = re.compile(args.filter)
        selected = [path for path in selected if pattern.search(path.name)]
    return selected


def parse_args(argv: Sequence[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument("tests", nargs="*", help="exact test paths/names (default: full execute corpus)")
    parser.add_argument(
        "--from-list", type=Path, metavar="FILE",
        help="run only the tests named in FILE (newline-separated JSON `test` "
             "field values, with or without the .c suffix; blank lines and # "
             "comments ignored; unknown names warn and are skipped; composes "
             "as an intersection with positional tests and --filter)",
    )
    parser.add_argument("--arch", choices=["x86_64", "i686", "i386"], default="x86_64")
    parser.add_argument("--suite", type=Path, default=DEFAULT_SUITE)
    parser.add_argument("--lccc", type=Path)
    parser.add_argument("--lccc-ld", type=Path, default=DEFAULT_LD)
    parser.add_argument("--gcc", default=os.environ.get("GCC_BIN", "gcc"))
    parser.add_argument(
        "--link-mode", choices=["lccc-ld", "driver"], default="lccc-ld",
        help="lccc-ld (default, both arches): compile with -c and link via "
             "`gcc -B<shim>` with the standalone lccc-ld as `ld`; driver "
             "(i686 only): let lccc-i686 compile and link in one step "
             "through its built-in linker (the pre-2026-09-27 i686 leg)")
    parser.add_argument("--runner", default=os.environ.get("GCC_RUNNER", ""),
                        help="runner command prefix (e.g. qemu-i386)")
    parser.add_argument("--append", default="", help="extra flags appended to all compile commands")
    parser.add_argument("--flags", default=",".join(DEFAULT_FLAGS),
                        help="comma-separated optimization configurations")
    parser.add_argument("--filter", default="", help="regular expression over basenames")
    parser.add_argument("-j", "--jobs", type=int, default=2)
    parser.add_argument(
        "--expensive", action="store_true",
        default=bool(os.environ.get("GCC_TEST_RUN_EXPENSIVE")),
        help="also run tests that require the run_expensive_tests effective "
             "target (default: only when GCC_TEST_RUN_EXPENSIVE is set, like "
             "gcc's own harness; otherwise they report `unsupported`)")
    parser.add_argument("--compile-timeout", type=float, default=60,
                        help="seconds; scaled by a test's dg-timeout-factor")
    parser.add_argument("--run-timeout", type=float, default=10)
    parser.add_argument("--json", type=Path)
    parser.add_argument("--failure-log", type=Path)
    parser.add_argument("-v", "--verbose", action="store_true")
    return parser.parse_args(argv)


def main(argv: Sequence[str] | None = None) -> int:
    sys.stdout.reconfigure(line_buffering=True)
    args = parse_args(argv)
    args.suite = args.suite.expanduser().resolve()

    if not args.lccc:
        args.lccc = DEFAULT_LCCC_I686 if args.arch in ("i686", "i386") else DEFAULT_LCCC_X86_64

    args.lccc = args.lccc.expanduser().resolve()
    args.lccc_ld = args.lccc_ld.expanduser().resolve()
    args.gcc = shutil.which(args.gcc) or args.gcc

    for label, path in (("suite", args.suite), ("lccc", args.lccc)):
        if not path.exists():
            print(f"error: {label} not found: {path}", file=sys.stderr)
            return 2

    if args.link_mode == "driver" and args.arch == "x86_64":
        print("error: --link-mode=driver is the i686 legacy leg only", file=sys.stderr)
        return 2
    if args.link_mode == "lccc-ld" and not args.lccc_ld.exists():
        print(f"error: lccc-ld not found: {args.lccc_ld}", file=sys.stderr)
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

    cases = [
        Case(source, opt, native_directive_flags(source),
             directive_timeout_factor(source), requires_expensive(source))
        for source in sources
        for opt in flags
    ]
    runner_cmd = shlex.split(args.runner) if args.runner else []
    append_args = shlex.split(args.append) if args.append else []

    print(f"suite:   {args.suite} ({len(sources)} sources)")
    if args.from_list:
        print(f"list:    {args.from_list} (partial run, {len(sources)} listed sources kept)")
    print(f"arch:    {args.arch} | runner: {args.runner or 'native'}")
    print(f"lccc:    {args.lccc}")
    if args.link_mode == "lccc-ld":
        print(f"lccc-ld: {args.lccc_ld}")
    else:
        print("link:    lccc-i686 built-in linker (--link-mode=driver)")
    print(f"matrix:  {len(cases)} cases, jobs={max(1, args.jobs)}")

    started = time.monotonic()
    results: list[Result] = []
    counts: Counter[str] = Counter()
    with concurrent.futures.ThreadPoolExecutor(max_workers=max(1, args.jobs)) as pool:
        futures = {
            pool.submit(
                execute_case,
                case,
                arch=args.arch,
                lccc=args.lccc,
                lccc_ld=args.lccc_ld,
                gcc=args.gcc,
                suite=args.suite,
                compile_timeout=args.compile_timeout,
                run_timeout=args.run_timeout,
                runner=runner_cmd,
                append_args=append_args,
                link_mode=args.link_mode,
                run_expensive=args.expensive,
            ): case
            for case in cases
        }
        for index, future in enumerate(concurrent.futures.as_completed(futures), 1):
            result = future.result()
            results.append(result)
            counts[result.status] += 1
            if result.status not in QUIET_STATUSES:
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
        "arch": args.arch,
        "suite": str(args.suite),
        # Present only for --from-list partial runs so downstream evidence
        # tooling (torture_evidence.py & friends) can see the report covers a
        # filtered subset rather than the full corpus; full-run reports keep
        # the exact pre-`--from-list` schema (no extra key).
        **({"from_list": str(args.from_list)} if args.from_list else {}),
        "gcc_checkout_head": revision(args.suite.parents[3]) if len(args.suite.parents) > 3 else None,
        "lccc": str(args.lccc),
        "lccc_ld": str(args.lccc_ld),
        "link_mode": args.link_mode,
        "gcc_testsuite": _testsuite_version(args.suite),
        "lccc_head": revision(REPO),
        "gcc": args.gcc,
        "flags": flags,
        "expensive": args.expensive,
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
                if result.status in QUIET_STATUSES:
                    continue
                stream.write(
                    f"=== {result.status} {result.test}[{result.flags}] "
                    f"phase={result.phase} rc={result.returncode} ===\n"
                    f"{result.detail}\n"
                )

    print(f"\n== {len(cases)} cases in {elapsed:.1f}s ==")
    for status, count in sorted(counts.items()):
        print(f"{status:>24}: {count}")
    failures = sum(count for status, count in counts.items() if status.endswith("-fail"))
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
