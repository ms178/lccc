#!/usr/bin/env python3
"""Unified GCC C Torture & Execution Test Harness for LCCC (x86-64 and i686).

Runs GCC's native gcc.c-torture test suite with LCCC across multiple
optimization levels and target architectures: the `execute/` leg (default:
compile, link through lccc-ld, run) and the `compile/` leg (`--mode=compile`:
every source must compile and assemble to an object, gcc's compile.exp
contract).

Architectures Supported:
  - x86_64 (64-bit x86, default): LCCC compiler + lccc-ld standalone linker.
  - i686   (32-bit x86): lccc-i686 compiler + lccc-ld standalone linker
    (`-m elf_i386`) + multilib CRT; --link-mode=driver keeps the legacy
    one-step lccc-i686 compile+link through its built-in linker.

Corpus: scripts/ensure_gcc_torture.sh provisions the GCC 16.2 release;
scripts/ensure_gcc_torture_git.py provisions revision-pinned GCC 17 development
sources. Select the latter with --suite; GCC_TORTURE overrides the default.

Features:
  - Multi-threaded execution pool.
  - Reference compiler validation (GCC) to establish host/test eligibility.
  - Sandbox and cross-execution support (--runner, e.g. qemu-i386).
  - Atomic partial JSON checkpoints, binary/source identities, and failure logs.
  - Reference multilib preflight; missing tools/timeouts cannot become skips.

This is a native differential subset, NOT a full DejaGnu implementation.
Only unconditional dg-options and the documented expensive/timeout directives
are interpreted. Reference skips retain diagnostics and are not passes. The
default five -O levels are not GCC's full LTO/loop-flag torture option matrix.
  - Failure-focused re-runs: --from-list FILE restricts the corpus to the
    test names listed in a previous report (one JSON `test` value per line).

Examples:
  scripts/x86_gcc_torture.py --flags=-O2 -j2
  scripts/x86_gcc_torture.py --arch=i686 --flags=-O2 -j2
  scripts/x86_gcc_torture.py 20080604-1.c pr51933.c --flags=-O0,-O2,-Os
  scripts/x86_gcc_torture.py --filter '^(pr12|va-arg)' --json results.json
  scripts/x86_gcc_torture.py --from-list failing.txt --json rerun.json
  scripts/x86_gcc_torture.py --mode=compile --arch=i686 -j2
"""
from __future__ import annotations

import argparse
import concurrent.futures
import dataclasses
import glob
import functools
import hashlib
import math
import platform
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
sys.path.insert(0, str(REPO))
from tools.corpus import process, publication

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


@functools.lru_cache(maxsize=8)
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
    # Share the corpus runner's bounded output/process-group containment. A
    # GCC driver timeout must kill cc1/collect2 too, not leave them consuming
    # the constrained host after their parent has been reaped.
    try:
        result = process.run(command, timeout, cwd=cwd)
        code = result["returncode"]
        stderr = result["stderr"]
        if result["failure"]:
            code = 124 if result["failure"] == "timeout" else 125
            stderr += f"\nHARNESS {result['failure']} (timeout={timeout:g}s)".encode()
        return subprocess.CompletedProcess(command, code, result["stdout"], stderr)
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
        factor = float(match.group(1)) if match else 1.0
        return max(1.0, factor) if math.isfinite(factor) else 1.0
    except ValueError:
        return 1.0


def requires_expensive(source: Path) -> bool:
    return _EXPENSIVE.search(source.read_text(errors="replace")[:8192]) is not None


def _includes(gcc: str, suite: Path, is_32: bool) -> list[str]:
    includes: list[str] = ["-I", str(suite)]
    if is_32:
        includes.extend(_gcc_private_include(gcc, m32=True))
        includes.extend([
            "-isystem", "/usr/include/x86_64-linux-gnu",
            "-isystem", "/usr/include/i386-linux-gnu",
            "-isystem", "/usr/include",
        ])
    return includes


def compile_case(
    case: Case,
    *,
    arch: str,
    lccc: Path,
    gcc: str,
    suite: Path,
    compile_timeout: float,
    append_args: list[str],
    run_expensive: bool = False,
    **_unused: object,
) -> Result:
    """One `compile/` case: gcc.c-torture/compile.exp's contract.

    The source must compile and assemble to an object (the integrated
    assembler runs inside `lccc -c`); there is no link or run.  The
    reference compiler decides eligibility exactly as in the execute leg: a
    source the host gcc (with `-m32` for i686) cannot compile is a
    `reference-compile-skip` (e.g. `__int128` on i686, target-specific asm).
    """
    started = time.monotonic()
    if case.expensive and not run_expensive:
        return Result(
            case.source.name, case.opt_flags, list(case.directive_flags),
            "unsupported", "run_expensive_tests", 0, 0.0,
            "dg-require-effective-target run_expensive_tests "
            "(set GCC_TEST_RUN_EXPENSIVE=1 or pass --expensive)",
        )
    compile_timeout *= case.timeout_factor
    is_32 = arch in ("i686", "i386", "x86_32")
    common = ["-w", *shlex.split(case.opt_flags), *case.directive_flags, *append_args,
              *_includes(gcc, suite, is_32)]
    with tempfile.TemporaryDirectory(prefix="lccc-gcc-torture-") as temp_name:
        temp = Path(temp_name)
        proc = run([gcc, *(["-m32"] if is_32 else []), *common, "-c",
                    str(case.source), "-o", str(temp / "reference.o")], compile_timeout)
        if proc.returncode:
            return Result(
                case.source.name, case.opt_flags, list(case.directive_flags),
                reference_status(proc, "compile"), "reference-compile", proc.returncode,
                time.monotonic() - started, detail_of(proc),
            )
        obj = temp / "test.o"
        proc = run([str(lccc), *common, "-c", str(case.source), "-o", str(obj)],
                   compile_timeout)
        if proc.returncode:
            return Result(
                case.source.name, case.opt_flags, list(case.directive_flags),
                "compile-fail", "lccc-i686" if is_32 else "lccc-compile",
                proc.returncode, time.monotonic() - started, detail_of(proc),
            )
        # Do not accept four ELF-magic bytes (or a 64-bit object in the i686
        # leg) as successful integrated assembly.
        if not valid_object(obj, is_32):
            return Result(
                case.source.name, case.opt_flags, list(case.directive_flags),
                "compile-fail", "invalid-object", proc.returncode,
                time.monotonic() - started, "lccc exited 0 without a target ELF relocatable object",
            )
    return Result(
        case.source.name, case.opt_flags, list(case.directive_flags),
        "pass", "complete", 0, time.monotonic() - started,
    )


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
    all_flags = [*shlex.split(case.opt_flags), *case.directive_flags, *append_args]
    is_32 = arch in ("i686", "i386", "x86_32")
    common = ["-w", *all_flags, *_includes(gcc, suite, is_32)]

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
                reference_status(proc, "compile"), "reference-compile", proc.returncode,
                time.monotonic() - started, detail_of(proc),
            )

        ref_exec_cmd = [*runner, str(reference)]
        proc = run(ref_exec_cmd, run_timeout)
        if proc.returncode:
            return Result(
                case.source.name, case.opt_flags, list(case.directive_flags),
                reference_status(proc, "run"), "reference-run", proc.returncode,
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
        text = (suite.parents[1] / ".lccc-provisioned").read_text()
    except (OSError, IndexError):
        return None
    # First line is the release; the rest is integrity metadata.
    first = text.split("\n", 1)[0].strip()
    return first or None


def revision(path: Path) -> str | None:
    proc = run(["git", "-C", str(path), "rev-parse", "HEAD"], 10)
    return proc.stdout.decode().strip() if proc.returncode == 0 else None


def atomic_json(path: Path, payload: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    publication.atomic_bytes(path, (json.dumps(payload, indent=2, sort_keys=True,
                                               allow_nan=False) + "\n").encode())


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


def valid_object(path: Path, is_32: bool) -> bool:
    """Minimum ELF header contract, not a full ELF verifier."""
    size = 52 if is_32 else 64
    try:
        with path.open("rb") as stream:
            header = stream.read(size)
    except OSError:
        return False
    return (len(header) == size and header[:7] == b"\x7fELF" +
            bytes((1 if is_32 else 2, 1, 1)) and
            int.from_bytes(header[16:18], "little") == 1 and
            int.from_bytes(header[18:20], "little") == (3 if is_32 else 62) and
            int.from_bytes(header[20:24], "little") == 1 and
            int.from_bytes(header[40:42] if is_32 else header[52:54], "little") == size)


def reference_status(proc: subprocess.CompletedProcess[bytes], phase: str) -> str:
    """Host rejection is ineligibility; missing tools/resource failures are not."""
    if (proc.returncode in (124, 125, 126, 127) or
            (phase == "compile" and (proc.returncode < 0 or
             b"internal compiler error" in proc.stderr.lower()))):
        return "reference-fail"
    return f"reference-{phase}-skip"


def file_sha256(path: Path) -> str | None:
    try:
        # hashlib.file_digest is Python 3.11+, but the tooling floor is 3.9.
        # Streaming also avoids loading a large debug compiler into memory.
        digest = hashlib.sha256()
        with path.open("rb") as stream:
            for block in iter(lambda: stream.read(1024 * 1024), b""):
                digest.update(block)
        return digest.hexdigest()
    except OSError:
        return None


def reference_preflight(args: argparse.Namespace, runner: list[str]) -> str | None:
    """Detect unavailable GCC/multilib/runtime before thousands of false skips."""
    is_32 = args.arch in ("i686", "i386")
    with tempfile.TemporaryDirectory(prefix="lccc-torture-preflight-") as td:
        root = Path(td)
        source = root / "probe.c"
        source.write_text(
            "#include <stddef.h>\n#include <math.h>\n"
            "volatile double x;\n"
            f"int main(void) {{ return sizeof(void *) != {4 if is_32 else 8} || sin(x) != 0; }}\n"
        )
        output = root / ("probe.o" if args.mode == "compile" else "probe")
        command = [args.gcc, *(["-m32"] if is_32 else []), "-O0",
                   *(["-c"] if args.mode == "compile" else []), str(source),
                   *([] if args.mode == "compile" else ["-lm"]), "-o", str(output)]
        proc = run(command, args.compile_timeout)
        if proc.returncode or not output.is_file():
            return f"reference {args.arch} compile/link preflight failed: " + detail_of(proc)
        if args.mode == "compile" and not valid_object(output, is_32):
            return "reference preflight did not produce a target ELF relocatable object"
        if args.mode == "execute":
            proc = run([*runner, str(output)], args.run_timeout)
            if proc.returncode:
                return f"reference {args.arch} execution preflight failed: " + detail_of(proc)
    return None


def public_environment() -> dict[str, str]:
    """Feature knobs are evidence; credential-bearing environment is not."""
    return {k: v for k, v in sorted(os.environ.items())
            if k.startswith(("CCC_", "LCCC_")) and not any(
                marker in k.upper() for marker in ("TOKEN", "SECRET", "PASSWORD", "KEY", "AUTH"))}


def run_identity(args: argparse.Namespace, sources: list[Path]) -> dict:
    tools = {}
    for name, command in (("lccc", str(args.lccc)), ("lccc_ld", str(args.lccc_ld)),
                          ("gcc", args.gcc)):
        if name == "lccc_ld" and (args.mode == "compile" or args.link_mode == "driver"):
            continue
        banner = run([command, "--version"], 15)
        tools[name] = dict(path=command, sha256=file_sha256(Path(command)),
                           version=detail_of(banner, 4096).strip(),
                           version_returncode=banner.returncode)
    selected = hashlib.sha256()
    for source in sorted(sources):
        selected.update(source.name.encode() + b"\0" + source.read_bytes() + b"\0")
    stamp = args.suite.parents[1] / ".lccc-provisioned"
    return {
        "arch": args.arch, "mode": args.mode, "suite": str(args.suite),
        "lccc": str(args.lccc), "lccc_ld": str(args.lccc_ld), "gcc": args.gcc,
        "lccc_head": revision(REPO), "link_mode": args.link_mode,
        "runner_sha256": file_sha256(Path(__file__)),
        "gcc_checkout_head": revision(args.suite),
        "gcc_testsuite": _testsuite_version(args.suite),
        "corpus_stamp_sha256": file_sha256(stamp), "tools": tools,
        "selection": {"tests": args.tests, "filter": args.filter,
                      "from_list": str(args.from_list) if args.from_list else None,
                      "sources": len(sources), "selected_sources_sha256": selected.hexdigest()},
        **({"from_list": str(args.from_list)} if args.from_list else {}),
        "append": args.append, "runner": args.runner, "expensive": args.expensive,
        "compile_timeout": args.compile_timeout, "run_timeout": args.run_timeout,
        "jobs": args.jobs, "host": platform.platform(),
        "environment": public_environment(),
        "coverage_contract": "top-level C sources; native reference eligibility; "
                             "not full DejaGnu directive/option coverage",
    }


def parse_args(argv: Sequence[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument("tests", nargs="*", help="exact test paths/names (default: the full corpus of --mode)")
    parser.add_argument(
        "--mode", choices=["execute", "compile"], default="execute",
        help="execute (default): gcc.c-torture/execute, compile + link + run; "
             "compile: gcc.c-torture/compile, compile + assemble only (the "
             "default --suite becomes the sibling compile/ directory)")
    parser.add_argument(
        "--from-list", type=Path, metavar="FILE",
        help="run only the tests named in FILE (newline-separated JSON `test` "
             "field values, with or without the .c suffix; blank lines and # "
             "comments ignored; unknown names warn and are skipped; composes "
             "as an intersection with positional tests and --filter)",
    )
    parser.add_argument("--arch", choices=["x86_64", "i686", "i386"], default="x86_64")
    parser.add_argument("--suite", type=Path, default=None,
                        help=f"corpus directory (default: {DEFAULT_SUITE}, or its "
                             "sibling compile/ with --mode=compile)")
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
    return parser.parse_intermixed_args(argv)


def main(argv: Sequence[str] | None = None) -> int:
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(line_buffering=True)
    args = parse_args(argv)
    if (args.jobs < 1 or any(not math.isfinite(t) or t <= 0
                             for t in (args.compile_timeout, args.run_timeout))):
        print("error: jobs and finite timeouts must be positive", file=sys.stderr)
        return 2
    if args.suite is None:
        args.suite = DEFAULT_SUITE if args.mode == "execute" else DEFAULT_SUITE.parent / "compile"
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
    if args.link_mode == "lccc-ld" and args.mode == "execute" and not args.lccc_ld.exists():
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

    if len({source.name for source in sources}) != len(sources) or len(set(flags)) != len(flags):
        print("error: duplicate test names/configurations make report keys ambiguous", file=sys.stderr)
        return 2
    try:
        for flag in flags:
            shlex.split(flag)
        runner_cmd = shlex.split(args.runner)
        append_args = shlex.split(args.append)
    except ValueError as exc:
        print(f"error: invalid option quoting: {exc}", file=sys.stderr)
        return 2

    cases = [
        Case(source, opt, native_directive_flags(source),
             directive_timeout_factor(source), requires_expensive(source))
        for source in sources
        for opt in flags
    ]

    print(f"suite:   {args.suite} ({len(sources)} sources)")
    if args.from_list:
        print(f"list:    {args.from_list} (partial run, {len(sources)} listed sources kept)")
    print(f"arch:    {args.arch} | runner: {args.runner or 'native'}")
    print(f"lccc:    {args.lccc}")
    if args.mode == "compile":
        print("mode:    compile (compile + assemble; no link, no run)")
    elif args.link_mode == "lccc-ld":
        print(f"lccc-ld: {args.lccc_ld}")
    else:
        print("link:    lccc-i686 built-in linker (--link-mode=driver)")
    print(f"matrix:  {len(cases)} cases, jobs={max(1, args.jobs)}")

    started = time.monotonic()
    results: list[Result] = []
    counts: Counter[str] = Counter()
    # Identity is captured before the matrix, never inferred later from a
    # possibly rebuilt binary or a rebased checkout. Paths alone are not pins.
    identity = run_identity(args, sources)
    fatal = reference_preflight(args, runner_cmd)

    def save(complete: bool) -> None:
        if args.json:
            atomic_json(args.json, {
                "schema": 2, **identity, "complete": complete,
                "expected_cases": len(cases), "completed_cases": len(results),
                "fatal": fatal, "flags": flags,
                "elapsed_s": round(time.monotonic() - started, 3),
                "counts": dict(sorted(counts.items())),
                "results": [dataclasses.asdict(r) for r in sorted(
                    results, key=lambda r: (r.test, flags.index(r.flags)))],
            })

    save(False)
    if fatal:
        print("error: " + fatal, file=sys.stderr)
        return 2
    with concurrent.futures.ThreadPoolExecutor(max_workers=max(1, args.jobs)) as pool:
        futures = {
            pool.submit(
                compile_case if args.mode == "compile" else execute_case,
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
            # At most 24 completed cases can be lost by an external SIGKILL.
            # Partial reports explicitly carry complete=false; no consumer
            # may mistake them for an all-corpus success.
            if index % 25 == 0 or result.status not in QUIET_STATUSES:
                save(False)
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
    if not counts["pass"] and all(status in QUIET_STATUSES for status in counts):
        fatal = "no eligible case passed; an all-skipped run is not validation"
    save(True)
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
    if fatal:
        print("error: " + fatal, file=sys.stderr)
        return 2
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
