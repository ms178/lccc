#!/usr/bin/env python3
"""Batch Compiler Explorer code-generation oracle for LCCC.

This is the research loop's "compare codegen between all listed compilers"
tool.  It reuses :mod:`godbolt` rather than duplicating HTTP/cache behavior,
adds batch function selection, deterministic JSON/Markdown reports, static
instruction/load/store/spill/branch statistics, and a non-zero exit when any
requested local or remote compiler fails.

Examples
--------
Compare one function against all competition compilers::

    scripts/codegen_oracle.py hot.c --function crc32_update \
        --local target/fastbuild/lccc --flags '-O3 -march=x86-64-v3'

Survey a benchmark file and write review artifacts::

    scripts/codegen_oracle.py tests/benchmark/programs/gzip_crc32.c \
        --local target/fastbuild/lccc \
        --local-flags '-O3 -march=x86-64-v3 -I/usr/lib/gcc/x86_64-linux-gnu/14/include' \
        --flags '-O3 -march=x86-64-v3' \
        --artifact-dir results/gzip-crc --json results/gzip-crc/manifest.json

The x86 defaults intentionally include GCC, Clang, ICC and ICX because ICX is
a moving channel. AArch64 defaults to ARM64 GCC 16.1, GCC trunk, and Clang 22.1
(with automatic `--target=aarch64-linux-gnu`). Every manifest records the
architecture and resolved compiler id/name/version.
"""
from __future__ import annotations

import argparse
import concurrent.futures
import hashlib
import json
import os
import re
import shlex
import statistics
import subprocess
import sys
import tempfile
import time
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Any, Iterable

sys.path.insert(0, str(Path(__file__).resolve().parent))
import godbolt  # noqa: E402

DEFAULT_FLAGS = "-O3 -march=x86-64-v3"
DEFAULT_ORACLES = ("gcc16.2", "clang", "icc", "icx")
DEFAULT_AARCH64_ORACLES = ("carm64g1610", "carm64gtrunk", "cclang2210")
DEFAULT_RISCV64_ORACLES = ("crv64g1520", "crv64g1610", "crv64gtrunk")

_DIRECTIVE = re.compile(r"^\s*(?:\.|#|//|cfi_)")
_LABEL = re.compile(r'^\s*"?([.\w$]+)"?:')
_BRANCH = re.compile(r"^(j\w+|callq?|retq?|loop\w*)$")
_STACK_MEM = re.compile(r"(?:[-+]?\d+)?\(%(?:r(?:sp|bp)|e(?:sp|bp))(?:,|\))")


@dataclass(frozen=True)
class Request:
    source: Path
    function: str | None
    flags: str
    local_flags: str | None
    arch: str


@dataclass
class AsmStats:
    instructions: int = 0
    loads: int = 0
    stores: int = 0
    spills: int = 0
    branches: int = 0
    vectors: int = 0


_OPTIMIZED_FUNCTION_SUFFIX = re.compile(
    r"^(?:constprop|isra|part|cold|llvm\.[A-Za-z0-9_.-]+)(?:\.\d+)*$"
)


def _label_is_function(wanted: str, actual: str) -> bool:
    """Match a requested function and GCC/LLVM's local clone suffixes."""
    if actual == wanted:
        return True
    prefix = wanted + "."
    return actual.startswith(prefix) and bool(
        _OPTIMIZED_FUNCTION_SUFFIX.fullmatch(actual[len(prefix):])
    )


def _function_body(lines: list[str], wanted: str | None) -> list[str] | None:
    if not wanted:
        return [line for line in lines if not _DIRECTIVE.match(line)]
    label = re.compile(r'^\s*"?([^"\s:]+)"?:\s*(?:[#;].*)?$')
    out: list[str] = []
    active = False
    for line in lines:
        text = line.strip()
        match = label.match(line)
        if match and not match.group(1).startswith("."):
            if active:
                break
            active = _label_is_function(wanted, match.group(1))
        if active and text.startswith(".size "):
            break
        if active and text.startswith(".cfi_endproc"):
            continue
        if active:
            out.append(line)
    return out if active else None


def _stats(lines: Iterable[str], arch: str) -> AsmStats:
    stats = AsmStats()
    for raw in lines:
        text = raw.strip()
        if not text or _DIRECTIVE.match(text) or text.endswith(":"):
            continue
        if _LABEL.match(text):
            continue
        parts = text.split(None, 1)
        if not parts:
            continue
        mnemonic = parts[0].lower()
        operands = parts[1] if len(parts) > 1 else ""
        stats.instructions += 1

        if arch == "aarch64":
            if (mnemonic in {"b", "bl", "blr", "br", "ret", "cbz", "cbnz", "tbz", "tbnz"}
                    or mnemonic.startswith("b.")):
                stats.branches += 1
            if mnemonic.startswith(("ld", "prfm")) and "[" in operands:
                stats.loads += 1
            if mnemonic.startswith("st") and "[" in operands:
                stats.stores += 1
            if "[sp" in operands or "[x29" in operands:
                stats.spills += 1
            if (re.search(r"\b[vsdq][0-9]+(?:\.|\b)", operands)
                    or mnemonic in {"addv", "smaxv", "fmaxv", "sadalp"}):
                stats.vectors += 1
            continue
        if arch == "riscv64":
            # Plain RISC-V (V/N ext aside): branches are the explicit
            # b* comparison set plus the jump/call/return family. Loads and
            # stores are width-mnemonic based ("li" is an immediate, not a
            # load); spills are any sp-relative memory operand.
            if mnemonic in {
                "beq", "bne", "blt", "bge", "bltu", "bgeu",
                "beqz", "bnez", "blez", "bgez", "bltz", "bgtz",
                "j", "jal", "jr", "jalr", "call", "tail", "ret",
            }:
                stats.branches += 1
            if mnemonic.startswith(("lb", "lh", "lw", "ld", "fl")):
                stats.loads += 1
            if mnemonic.startswith(("sb", "sh", "sw", "sd", "fs")):
                stats.stores += 1
            if "(sp)" in operands:
                stats.spills += 1
            if mnemonic.startswith("v") and len(mnemonic) > 1:
                stats.vectors += 1
            continue

        if _BRANCH.match(mnemonic):
            stats.branches += 1
        if mnemonic.startswith("v") and not re.search(r"(ss|sd|si)(q|l)?$", mnemonic):
            stats.vectors += 1
        if "(" in operands:
            split_operands = operands.rsplit(",", 1)
            source = split_operands[0]
            destination = split_operands[1] if len(split_operands) > 1 else ""
            if "(" in source:
                stats.loads += 1
            if "(" in destination:
                stats.stores += 1
            if _STACK_MEM.search(operands):
                stats.spills += 1
    return stats


def _local_compile(executable: str, source: Path, flags: str) -> list[str]:
    digest = hashlib.sha256((str(source.resolve()) + "\0" + flags).encode()).hexdigest()[:16]
    fd, output_name = tempfile.mkstemp(
        prefix=f"codegen-oracle-{digest}-",
        suffix=".s",
        dir=os.environ.get("TMPDIR", "/tmp"),
    )
    os.close(fd)
    output = Path(output_name)
    command = [executable, *shlex.split(flags), "-S", str(source), "-o", str(output)]
    result = subprocess.run(command, capture_output=True, text=True)
    if result.returncode != 0:
        raise godbolt.GodboltError(
            f"local compile failed: {' '.join(command)}\n{result.stderr.strip()}"
        )
    try:
        return output.read_text(errors="replace").splitlines()
    finally:
        output.unlink(missing_ok=True)


def _compile_remote(name: str, source: str, flags: str) -> list[str]:
    cid = godbolt.resolve_compiler(name)
    data = godbolt.compile_on_godbolt(cid, source, flags, intel=False)
    if data is None:
        raise godbolt.GodboltError(f"remote compile failed: {name} ({cid})")
    return godbolt.assembly_lines(data)


def _record_local(req: Request, executable: str) -> dict[str, Any]:
    started = time.perf_counter()
    lines = _local_compile(executable, req.source, req.local_flags or req.flags)
    elapsed = time.perf_counter() - started
    body = _function_body(lines, req.function)
    if body is None:
        raise godbolt.GodboltError(
            f"function '{req.function}' was not emitted by local LCCC "
            "(likely inlined or removed)"
        )
    return {
        "key": "lccc",
        "id": str(Path(executable).resolve()),
        "name": "local LCCC",
        "flags": req.local_flags or req.flags,
        "elapsed_s": elapsed,
        **asdict(_stats(body, req.arch)),
        "assembly": body,
    }


def _record_remote(name: str, source: str, req: Request, compilers: list[dict[str, Any]]) -> dict[str, Any]:
    metadata = godbolt.compiler_metadata(name, compilers=compilers)
    flags = req.flags
    compiler_text = f"{metadata.get('id', '')} {metadata.get('name', '')}".lower()
    # CE's native ARM GCC channels already target AArch64, whereas the newest
    # Clang channel is hosted as x86-64 and needs an explicit backend target.
    if req.arch == "aarch64" and "clang" in compiler_text and "--target=" not in flags:
        flags = f"{flags} --target=aarch64-linux-gnu"
    started = time.perf_counter()
    lines = _compile_remote(name, source, flags)
    elapsed = time.perf_counter() - started
    body = _function_body(lines, req.function)
    if body is None:
        raise godbolt.GodboltError(
            f"function '{req.function}' was not emitted by {name} "
            "(likely inlined or removed)"
        )
    return {
        "key": name,
        **metadata,
        "flags": flags,
        "elapsed_s": elapsed,
        **asdict(_stats(body, req.arch)),
        "assembly": body,
    }


def _measure(req: Request, executable: str | None, oracles: list[str]) -> dict[str, Any]:
    source = req.source.read_text()
    compilers = godbolt.list_compilers("c")
    records: list[dict[str, Any]] = []
    errors: list[dict[str, str]] = []
    if executable:
        try:
            records.append(_record_local(req, executable))
        except Exception as exc:  # noqa: BLE001 -- report and return failure
            errors.append({"compiler": "lccc", "error": f"{type(exc).__name__}: {exc}"})
    for name in oracles:
        try:
            records.append(_record_remote(name, source, req, compilers))
        except Exception as exc:  # noqa: BLE001 -- one compiler must not kill the batch
            cid = godbolt.resolve_compiler(name)
            errors.append({"compiler": name, "id": cid, "error": f"{type(exc).__name__}: {exc}"})
    for record in records:
        counts = [r["instructions"] for r in records if r.get("instructions", 0) > 0]
        record["ratio_vs_best"] = (
            record["instructions"] / min(counts) if counts and record.get("instructions") else None
        )
    return {
        "source": str(req.source),
        "function": req.function,
        "arch": req.arch,
        "records": records,
        "errors": errors,
    }


def _write_artifacts(result: dict[str, Any], artifact_dir: Path) -> None:
    stem = Path(result["source"]).stem
    function = result.get("function") or "all"
    safe_function = re.sub(r"[^A-Za-z0-9_.-]+", "_", function).strip("_") or "all"
    target_dir = artifact_dir / f"{stem}-{safe_function}"
    target_dir.mkdir(parents=True, exist_ok=True)
    for record in result["records"]:
        key = re.sub(r"[^A-Za-z0-9_.-]+", "_", record["key"])
        (target_dir / f"{key}.s").write_text("\n".join(record["assembly"]) + "\n")
    manifest = {k: v for k, v in result.items() if k != "records"}
    manifest["records"] = [{k: v for k, v in r.items() if k != "assembly"} for r in result["records"]]
    (target_dir / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")


def _print_table(result: dict[str, Any]) -> None:
    print(f"\n{result['source']} :: {result.get('function') or '<all functions>'}")
    print(f"{'compiler':<12} {'insns':>7} {'loads':>6} {'stores':>7} {'spills':>7} {'branch':>7} {'best-x':>8}  name")
    print("-" * 92)
    for record in result["records"]:
        ratio = record.get("ratio_vs_best")
        ratio_text = f"{ratio:.2f}x" if ratio is not None else "—"
        print(
            f"{record['key']:<12} {record.get('instructions', 0):>7} "
            f"{record.get('loads', 0):>6} {record.get('stores', 0):>7} "
            f"{record.get('spills', 0):>7} {record.get('branches', 0):>7} "
            f"{ratio_text:>8}  {record.get('name', '')}"
        )
    for error in result["errors"]:
        print(f"ERROR       {error['compiler']}: {error['error']}", file=sys.stderr)


def _markdown(results: list[dict[str, Any]], path: Path) -> None:
    lines = [
        "# Codegen oracle report",
        "",
        "Static code-size/structure statistics from local LCCC and Compiler Explorer.",
        "These are screening metrics, not PMU evidence; verify wins with controlled",
        "runtime and hardware counters on the intended target before making claims.",
        "",
        "| Source | Function | LCCC | Best | Best compiler | LCCC/best | Loads | Stores | Spills | Branches |",
        "|---|---:|---:|---:|---|---:|---:|---:|---:|---:|",
    ]
    for result in results:
        records = result["records"]
        positive = [r for r in records if r.get("instructions", 0) > 0]
        if positive:
            best = min(positive, key=lambda r: r["instructions"])
            lccc = next((r for r in records if r["key"] == "lccc"), None)
            lccc_count = lccc["instructions"] if lccc else None
            ratio = (lccc_count / best["instructions"]) if lccc_count else None
            ratio_text = f"{ratio:.2f}x" if ratio is not None else "—"
            lines.append(
                f"| `{result['source']}` | `{result.get('function') or ''}` | "
                f"{lccc_count if lccc_count is not None else 'ERROR'} | {best['instructions']} | "
                f"{best['key']} | {ratio_text} | "
                f"{lccc.get('loads', 0) if lccc else 0} | {lccc.get('stores', 0) if lccc else 0} | "
                f"{lccc.get('spills', 0) if lccc else 0} | {lccc.get('branches', 0) if lccc else 0} |"
            )
        else:
            lines.append(f"| `{result['source']}` | `{result.get('function') or ''}` | ERROR | — | — | — | — | — | — | — |")
    path.write_text("\n".join(lines) + "\n")


def _print_totals(results: list[dict[str, Any]]) -> None:
    """Cross-source aggregate: sum static stats per compiler and count
    per-source bests.  Gives a one-glance answer to 'which compiler is
    smallest over this whole corpus' that the per-file tables cannot.

    These are screening metrics, not PMU evidence (see _markdown header).
    """
    import collections

    totals: dict[str, dict[str, float]] = collections.defaultdict(
        lambda: {"instructions": 0, "loads": 0, "stores": 0,
                 "spills": 0, "branches": 0, "vectors": 0, "sources": 0}
    )
    wins: dict[str, int] = collections.defaultdict(int)
    for result in results:
        records = {r["key"]: r for r in result["records"]}
        comparable = [r for r in result["records"] if r.get("instructions", 0) > 0]
        if comparable:
            best = min(comparable, key=lambda r: r["instructions"])
            wins[best["key"]] += 1
        for key, record in records.items():
            agg = totals[key]
            for field in ("instructions", "loads", "stores", "spills", "branches", "vectors"):
                agg[field] += record.get(field, 0)
            agg["sources"] += 1
    print(f"\nTOTALS across {len(results)} source(s) per compiler:"
          " instruction/load/store/spill/branch/vector sums, per-source bests")
    print(f"{'compiler':<12} {'insns':>7} {'loads':>6} {'stores':>7} "
          f"{'spills':>7} {'branch':>7} {'vector':>6} {'best':>5}")
    print("-" * 72)
    for key, agg in sorted(totals.items(), key=lambda kv: kv[1]["instructions"]):
        print(f"{key:<12} {agg['instructions']:>7.0f} {agg['loads']:>6.0f} "
              f"{agg['stores']:>7.0f} {agg['spills']:>7.0f} {agg['branches']:>7.0f} "
              f"{agg['vectors']:>6.0f} {wins.get(key, 0):>5}")


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    argv = list(sys.argv[1:] if argv is None else argv)
    # Dash-leading flag values ("--local-flags -O3", "--flags -O3 ...") are
    # rejected by argparse as options; join them exactly like godbolt.py.
    argv = godbolt.join_dash_values(argv)
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("sources", nargs="+", type=Path)
    parser.add_argument("--function", action="append", default=[], help="function to isolate; repeatable")
    parser.add_argument("--all-functions", action="store_true", help="report whole translation unit(s)")
    parser.add_argument("--local", default="target/fastbuild/lccc")
    parser.add_argument("--no-local", action="store_true")
    parser.add_argument("--flags", default=DEFAULT_FLAGS)
    parser.add_argument("--local-flags")
    parser.add_argument(
        "--arch",
        choices=("x86", "aarch64", "riscv64"),
        default="x86",
        help="assembly syntax used for structural statistics (default: x86)",
    )
    parser.add_argument(
        "--oracles",
        help="comma-separated CE compilers (defaults are architecture-specific)",
    )
    parser.add_argument("--artifact-dir", type=Path)
    parser.add_argument("--json", type=Path)
    parser.add_argument("--markdown", type=Path)
    parser.add_argument("--totals", action="store_true",
                        help="also print a cross-source aggregate per compiler "
                             "(summed static stats + per-source bests)")
    parser.add_argument("-j", "--jobs", type=int, default=4)
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    functions: list[str | None] = list(args.function) if args.function else []
    if args.all_functions or not functions:
        functions.append(None)
    default_oracles = (
        DEFAULT_AARCH64_ORACLES
        if args.arch == "aarch64"
        else DEFAULT_RISCV64_ORACLES
        if args.arch == "riscv64"
        else DEFAULT_ORACLES
    )
    oracle_text = args.oracles or ",".join(default_oracles)
    oracles = [item.strip() for item in oracle_text.split(",") if item.strip()]
    requests = [Request(source, function, args.flags, args.local_flags, args.arch)
                for source in args.sources for function in functions]
    executable = None if args.no_local else str(Path(args.local).expanduser().resolve())
    max_workers = max(1, min(args.jobs, len(requests)))
    results: list[dict[str, Any]] = []
    with concurrent.futures.ThreadPoolExecutor(max_workers=max_workers) as pool:
        futures = [pool.submit(_measure, req, executable, oracles) for req in requests]
        for future in concurrent.futures.as_completed(futures):
            result = future.result()
            results.append(result)
            _print_table(result)
            if args.artifact_dir:
                _write_artifacts(result, args.artifact_dir)
    order = {req: idx for idx, req in enumerate(requests)}
    results.sort(key=lambda item: order[Request(
        Path(item["source"]), item.get("function"), args.flags, args.local_flags, args.arch
    )])
    if args.json:
        args.json.parent.mkdir(parents=True, exist_ok=True)
        tmp = args.json.with_suffix(args.json.suffix + f".tmp.{os.getpid()}")
        tmp.write_text(json.dumps(results, indent=2) + "\n")
        tmp.replace(args.json)
    if args.markdown:
        _markdown(results, args.markdown)
    if args.totals:
        _print_totals(results)
    return 1 if any(result["errors"] for result in results) else 0


if __name__ == "__main__":
    raise SystemExit(main())
