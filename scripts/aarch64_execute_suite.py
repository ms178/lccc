#!/usr/bin/env python3
"""Differential AArch64 execution harness for external C conformance suites.

The primary corpus is GCC's ``gcc.c-torture/execute`` directory.  LCCC emits
assembly, the requested GNU assembler validates/encodes it, and the resulting
binary is linked with the cross GCC driver.  Reference binaries are built by
AArch64 GCC and, when requested, Clang.  All binaries execute under qemu-user;
stdout, stderr, and exit status must agree.

This is a correctness and coverage harness. QEMU wall time is deliberately not
reported as target-performance evidence.
"""
from __future__ import annotations

import argparse
import concurrent.futures
import dataclasses
import json
import os
import re
import shlex
import subprocess
import tempfile
import time
from pathlib import Path
from typing import Sequence


@dataclasses.dataclass(frozen=True)
class Toolchain:
    corpus_root: Path
    lccc: Path
    assembler: Path
    cross_gcc: str
    qemu: str
    clang: str | None
    sysroot: Path
    cflags: tuple[str, ...]
    timeout: float


@dataclasses.dataclass
class Result:
    source: str
    status: str
    stage: str
    elapsed_s: float
    detail: str = ""


def run(command: Sequence[str], timeout: float, *, env: dict[str, str] | None = None) -> subprocess.CompletedProcess[bytes]:
    return subprocess.run(
        command,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=timeout,
        env=env,
    )


def text(data: bytes, limit: int = 1200) -> str:
    return data.decode("utf-8", "replace")[:limit].strip()


def tool_version(command: Sequence[str]) -> str:
    try:
        result = run(command, 10)
        output = result.stdout or result.stderr
        return output.decode("utf-8", "replace").splitlines()[0]
    except (OSError, subprocess.SubprocessError, IndexError):
        return "unavailable"


def execute(binary: Path, tools: Toolchain) -> tuple[int, bytes, bytes]:
    result = run(
        [tools.qemu, "-L", str(tools.sysroot), str(binary)],
        tools.timeout,
    )
    return result.returncode, result.stdout, result.stderr


def dejagnu_flags(source: Path) -> tuple[str, ...]:
    """Extract unconditional dg-options/additional-options from one test.

    Target-qualified directives are intentionally ignored: evaluating DejaGNU
    target expressions incorrectly is worse than leaving the test at the suite
    default. The common unconditional language/overflow flags are sufficient
    to avoid false oracle disagreements in gcc.c-torture.
    """
    contents = source.read_text(errors="replace")
    flags: list[str] = []
    pattern = re.compile(r"\{\s*dg-(?:additional-)?options\s+\"([^\"]*)\"\s*\}")
    for match in pattern.finditer(contents):
        flags.extend(shlex.split(match.group(1)))
    return tuple(flags)


def compile_reference(
    compiler: str,
    source: Path,
    output: Path,
    tools: Toolchain,
    test_flags: tuple[str, ...],
    *,
    clang: bool,
) -> subprocess.CompletedProcess[bytes]:
    command = [compiler]
    if clang:
        command += ["--target=aarch64-linux-gnu", f"--sysroot={tools.sysroot}"]
    command += [
        *tools.cflags,
        *test_flags,
        f"-I{source.parent}",
        str(source),
        "-static",
        "-lm",
        "-o",
        str(output),
    ]
    return run(command, tools.timeout)


def test_one(source: Path, tools: Toolchain, work_root: Path) -> Result:
    started = time.monotonic()
    safe = re.sub(r"[^A-Za-z0-9_.-]+", "_", str(source.relative_to(tools.corpus_root)))
    work = work_root / safe
    work.mkdir(parents=True, exist_ok=True)
    assembly = work / "lccc.s"
    obj = work / "lccc.o"
    binary = work / "lccc"
    gcc_binary = work / "gcc"

    try:
        test_flags = dejagnu_flags(source)
        gcc_compile = compile_reference(
            tools.cross_gcc, source, gcc_binary, tools, test_flags, clang=False
        )
        if gcc_compile.returncode != 0:
            return Result(str(source), "SKIP", "gcc-compile", time.monotonic() - started, text(gcc_compile.stderr))

        lccc_compile = run(
            [
                str(tools.lccc),
                *tools.cflags,
                *test_flags,
                f"-I{source.parent}",
                "-S",
                str(source),
                "-o",
                str(assembly),
            ],
            tools.timeout,
        )
        if lccc_compile.returncode != 0:
            return Result(str(source), "FAIL", "lccc-compile", time.monotonic() - started, text(lccc_compile.stderr))

        assembled = run([str(tools.assembler), str(assembly), "-o", str(obj)], tools.timeout)
        if assembled.returncode != 0:
            return Result(str(source), "FAIL", "gas-2.47", time.monotonic() - started, text(assembled.stderr))

        linked = run(
            [tools.cross_gcc, str(obj), "-static", "-lm", "-o", str(binary)],
            tools.timeout,
        )
        if linked.returncode != 0:
            return Result(str(source), "FAIL", "lccc-link", time.monotonic() - started, text(linked.stderr))

        gcc_result = execute(gcc_binary, tools)
        lccc_result = execute(binary, tools)
        if lccc_result != gcc_result:
            detail = (
                f"lccc rc={lccc_result[0]} out={text(lccc_result[1], 400)!r} err={text(lccc_result[2], 400)!r}; "
                f"gcc rc={gcc_result[0]} out={text(gcc_result[1], 400)!r} err={text(gcc_result[2], 400)!r}"
            )
            return Result(str(source), "FAIL", "execute-vs-gcc", time.monotonic() - started, detail)

        if tools.clang:
            clang_binary = work / "clang"
            clang_compile = compile_reference(
                tools.clang, source, clang_binary, tools, test_flags, clang=True
            )
            if clang_compile.returncode == 0:
                clang_result = execute(clang_binary, tools)
                if clang_result != gcc_result:
                    return Result(
                        str(source),
                        "ORACLE-DISAGREE",
                        "clang-vs-gcc",
                        time.monotonic() - started,
                        f"clang rc={clang_result[0]} vs gcc rc={gcc_result[0]}",
                    )

        return Result(str(source), "PASS", "execute", time.monotonic() - started)
    except subprocess.TimeoutExpired as error:
        return Result(str(source), "FAIL", "timeout", time.monotonic() - started, " ".join(error.cmd))
    except OSError as error:
        return Result(str(source), "HARNESS-ERROR", "spawn", time.monotonic() - started, str(error))


def discover(root: Path, recursive: bool) -> list[Path]:
    iterator = root.rglob("*.c") if recursive else root.glob("*.c")
    return sorted(path.resolve() for path in iterator if path.is_file())


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("corpus", type=Path)
    parser.add_argument("--lccc", type=Path, default=Path("target/fastbuild/lccc-arm"))
    parser.add_argument("--assembler", type=Path, required=True)
    parser.add_argument("--cross-gcc", default="aarch64-linux-gnu-gcc")
    parser.add_argument("--clang", help="optional Clang executable")
    parser.add_argument("--qemu", default="qemu-aarch64")
    parser.add_argument("--sysroot", type=Path, default=Path("/usr/aarch64-linux-gnu"))
    parser.add_argument("--flags", default="-O2")
    parser.add_argument("--jobs", type=int, default=2)
    parser.add_argument("--timeout", type=float, default=15.0)
    parser.add_argument("--recursive", action="store_true")
    parser.add_argument("--limit", type=int)
    parser.add_argument("--match", help="regular expression applied to the source path")
    parser.add_argument("--work-dir", type=Path)
    parser.add_argument("--json", type=Path)
    parser.add_argument("--keep", action="store_true")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    assembler_version = tool_version([str(args.assembler), "--version"])
    if "2.47" not in assembler_version:
        raise SystemExit(f"error: --assembler must be GNU as 2.47, got: {assembler_version}")

    tools = Toolchain(
        corpus_root=args.corpus.resolve(),
        lccc=args.lccc.resolve(),
        assembler=args.assembler.resolve(),
        cross_gcc=args.cross_gcc,
        qemu=args.qemu,
        clang=args.clang,
        sysroot=args.sysroot.resolve(),
        cflags=tuple(shlex.split(args.flags)),
        timeout=args.timeout,
    )
    sources = discover(args.corpus.resolve(), args.recursive)
    if args.match:
        pattern = re.compile(args.match)
        sources = [source for source in sources if pattern.search(str(source))]
    if args.limit is not None:
        sources = sources[: args.limit]
    if not sources:
        raise SystemExit("error: no C tests selected")

    temporary = None
    if args.work_dir:
        work_root = args.work_dir.resolve()
        work_root.mkdir(parents=True, exist_ok=True)
    else:
        scratch_parent = Path(os.environ.get("LCCC_SCRATCH", "/home/user/.cache"))
        scratch_parent.mkdir(parents=True, exist_ok=True)
        if args.keep:
            work_root = Path(
                tempfile.mkdtemp(prefix="lccc-aarch64-suite-", dir=scratch_parent)
            )
        else:
            temporary = tempfile.TemporaryDirectory(
                prefix="lccc-aarch64-suite-", dir=scratch_parent
            )
            work_root = Path(temporary.name)

    results: list[Result] = []
    with concurrent.futures.ThreadPoolExecutor(max_workers=max(1, args.jobs)) as executor:
        futures = {executor.submit(test_one, source, tools, work_root): source for source in sources}
        for future in concurrent.futures.as_completed(futures):
            result = future.result()
            results.append(result)
            if result.status != "PASS":
                print(f"{result.status:16} {result.stage:16} {Path(result.source).name}: {result.detail}")

    results.sort(key=lambda result: result.source)
    counts: dict[str, int] = {}
    for result in results:
        counts[result.status] = counts.get(result.status, 0) + 1
    metadata = {
        "corpus": str(args.corpus.resolve()),
        "flags": list(tools.cflags),
        "tools": {
            "lccc": tool_version([str(tools.lccc), "--version"]),
            "assembler": assembler_version,
            "gcc": tool_version([tools.cross_gcc, "--version"]),
            "clang": tool_version([tools.clang, "--version"]) if tools.clang else None,
            "qemu": tool_version([tools.qemu, "--version"]),
        },
        "counts": counts,
        "results": [dataclasses.asdict(result) for result in results],
    }
    print("SUMMARY " + " ".join(f"{key}={value}" for key, value in sorted(counts.items())))
    if args.json:
        args.json.parent.mkdir(parents=True, exist_ok=True)
        temporary_json = args.json.with_suffix(args.json.suffix + f".tmp.{os.getpid()}")
        temporary_json.write_text(json.dumps(metadata, indent=2) + "\n")
        temporary_json.replace(args.json)
    if args.keep:
        print(f"WORKDIR {work_root}")
    return 1 if any(result.status in {"FAIL", "HARNESS-ERROR"} for result in results) else 0


if __name__ == "__main__":
    raise SystemExit(main())
