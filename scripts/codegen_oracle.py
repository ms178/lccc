#!/usr/bin/env python3
"""Batch Compiler Explorer code-generation oracle for LCCC.

This is the research loop's "compare codegen between all listed compilers"
tool.  It reuses :mod:`godbolt` rather than duplicating HTTP/cache behavior,
adds batch function selection, deterministic JSON/Markdown reports, static
instruction/load/store/spill/branch statistics, and a non-zero exit when any
requested local or remote compiler fails.

``--rank`` (survey mode, absorbed from the deleted ``codegen_scoreboard.py``)
answers the question that comes *before* single-function comparison: across a
whole corpus, WHICH functions are furthest behind the best oracle, and by what
margin -- so effort goes where the gap is largest instead of where it is
easiest to look.  Both report modes share ONE measurement pipeline and ONE
metric implementation (:func:`_stats`); the scoreboard's subtly divergent
duplicate extraction (its SIMD classifier counted the non-VEX packed SSE
family the oracle missed) was the defect this consolidation fixed.  Rank mode
defaults to the scoreboard's historical survey flags ``-O2 -march=x86-64-v3``,
ranks by instruction-count gap to the best oracle (worst first), and exits 0
even when compilers fail -- the scoreboard contract: a survey reports errors
on stderr, it is not a gate.  ``--oracles ''`` runs local-only, which leaves
per-function LCCC statistics in ``--json`` with no gap rows (the scoreboard's
controlled same-binary A/B usage).

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

Rank a whole benchmark suite, worst gaps first (scoreboard replacement)::

    scripts/codegen_oracle.py --rank tests/benchmark/programs/*.c

One file, every function, full detail (including tied/ahead rows)::

    scripts/codegen_oracle.py --rank tests/benchmark/programs/matmul.c -v

Re-check after a change, comparing to a saved baseline (the rank ``--json``
schema keeps the scoreboard's historical ``gaps`` keys, so old baselines stay
readable)::

    scripts/codegen_oracle.py --rank --baseline before.json \
        --json after.json tests/benchmark/programs/*.c

The x86 defaults intentionally include GCC, Clang, ICC and ICX because ICX is
a moving channel. AArch64 defaults to ARM64 GCC 16.1, GCC trunk, and Clang 22.1
(with automatic `--target=aarch64-linux-gnu`). Every manifest records the
architecture and resolved compiler id/name/version.  Pass
``--oracles gcc,clang,icc,icx`` to keep the scoreboard's short compiler keys
in rank-mode JSON detail (both spellings resolve to the same CE ids).
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
import threading
import time
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Any, Iterable

sys.path.insert(0, str(Path(__file__).resolve().parent))
import godbolt  # noqa: E402

DEFAULT_FLAGS = "-O3 -march=x86-64-v3"
# Rank mode keeps the scoreboard's historical survey default so old
# --baseline comparisons remain meaningful; compare mode keeps -O3.
RANK_DEFAULT_FLAGS = "-O2 -march=x86-64-v3"
DEFAULT_ORACLES = ("gcc16.2", "clang", "icc", "icx")
DEFAULT_AARCH64_ORACLES = ("carm64g1610", "carm64gtrunk", "cclang2210")
DEFAULT_RISCV64_ORACLES = ("crv64g1520", "crv64g1610", "crv64gtrunk")

_DIRECTIVE = re.compile(r"^\s*(?:\.|#|//|cfi_)")
_LABEL = re.compile(r'^\s*"?([.\w$]+)"?:')
_BRANCH = re.compile(r"^(j\w+|callq?|retq?|loop\w*)$")
_STACK_MEM = re.compile(r"(?:[-+]?\d+)?\(%(?:r(?:sp|bp)|e(?:sp|bp))(?:,|\))")

_ROOT = Path(__file__).resolve().parent.parent
_REPO_FASTBUILD = _ROOT / "target/fastbuild/lccc"
_REPO_RELEASE = _ROOT / "target/release/lccc"


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


def _is_simd_instruction(mnemonic: str) -> bool:
    """Conservative x86 SIMD classifier (do not count VEX scalar FP or popcnt).

    Absorbed verbatim from codegen_scoreboard.py so that single-file compare
    and ``--rank`` share ONE vector metric.  The oracle's former x86 arm
    counted only VEX-encoded ``v*`` instructions and silently undercounted
    the non-VEX packed integer/FP (SSE) family the scoreboard did count
    (``paddb``, ``punpck*``, ``packssdw``, ...), so the two tools could
    disagree about the same assembly listing.
    """
    m = mnemonic.lower()
    if m.startswith("v"):
        # VEX scalar instructions still begin with v; suffix ss/sd/si marks
        # scalar arithmetic/conversion. Keep packed ps/pd and vector integer.
        return not re.search(r"(?:ss|sd|si|2ss|2sd|2si)(?:q|l)?$", m)
    if m.startswith("p") and not m.startswith(("popcnt", "pause", "prefetch")):
        return m.startswith((
            "padd", "psub", "pmul", "pmadd", "pand", "por", "pxor",
            "pcmp", "pshuf", "psll", "psrl", "psra", "pack", "punpck",
            "pblend", "pmax", "pmin", "pabs", "psad", "palign",
        ))
    return False


def _function_body(lines: list[str], wanted: str | None) -> list[str] | None:
    if not wanted:
        # Whole-translation-unit mode: drop directives but keep label
        # definitions.  Local labels (`.LBB1:`, `.L2:`) start with a dot like
        # directives, so a bare `_DIRECTIVE` filter strips them and leaves
        # behind jumps to nowhere — the saved artifact then misleads every
        # manual CFG/diff read (and cannot be re-assembled).  Statistics are
        # unaffected either way: `_stats` skips colon-terminated lines and
        # `_split_function_bodies` only splits on non-dot labels.
        return [line for line in lines
                if not _DIRECTIVE.match(line) or _LABEL.match(line)]
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
        if _is_simd_instruction(mnemonic):
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


def _split_function_bodies(lines: Iterable[str]) -> dict[str, list[str]]:
    """Split a whole assembly listing into per-function bodies.

    Boundary rules absorbed from codegen_scoreboard.py's ``parse_functions``:
    a non-dot-prefixed label starts a new function (GCC's CE output may quote
    externally-visible labels, ``"foo":``), directives and local ``.L*``
    labels stay inside the current function, and lines before the first
    function are dropped.  Per-function metrics are then produced by the same
    :func:`_stats` used in single-function compare mode, so both report modes
    measure identically.
    """
    bodies: dict[str, list[str]] = {}
    name: str | None = None
    body: list[str] = []
    for raw in lines:
        line = raw.rstrip()
        if not line:
            continue
        match = _LABEL.match(line)
        if match and not match.group(1).startswith("."):
            if name is not None:
                bodies[name] = body
            name, body = match.group(1), []
            continue
        if name is not None:
            body.append(line)
    if name is not None:
        bodies[name] = body
    return bodies


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
    # att-v2 cache absorbed from codegen_scoreboard.py so batch compare and
    # --rank share one remote-compile path.  The digest pins syntax/parser
    # ABI (a pre-v2 cache stored Intel syntax, which made AT&T load/store
    # metrics silently read as zero) and the network is hit once per
    # (compiler, flags, source) tuple instead of once per report mode.
    key = hashlib.sha256(f"att-v2\0{cid}\0{flags}\0{source}".encode()).hexdigest()[:32]
    hit = godbolt.CACHE / f"{key}.s"
    if hit.exists():
        return hit.read_text(errors="replace").splitlines()
    data = godbolt.compile_on_godbolt(cid, source, flags, intel=False)
    if data is None:
        raise godbolt.GodboltError(f"remote compile failed: {name} ({cid})")
    lines = godbolt.assembly_lines(data)
    godbolt.CACHE.mkdir(parents=True, exist_ok=True)
    tmp = hit.with_suffix(f".tmp.{os.getpid()}.{threading.get_ident()}")
    tmp.write_text("\n".join(lines))
    tmp.replace(hit)
    return lines


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


def _measure(req: Request, executable: str | None, oracles: list[str],
             compilers: list[dict[str, Any]]) -> dict[str, Any]:
    source = req.source.read_text()
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
    parser.add_argument("--local", "--lccc", dest="local", default=None,
                        help="local LCCC executable; --lccc is the historical "
                             "scoreboard spelling (default: $LCCC, else "
                             "target/fastbuild/lccc, else target/release/lccc, "
                             "resolved relative to the repository root)")
    parser.add_argument("--no-local", action="store_true")
    parser.add_argument("--flags", default=None,
                        help="compiler flags for the oracle compilers (default: "
                             "-O3 -march=x86-64-v3, or the scoreboard's "
                             "-O2 -march=x86-64-v3 in --rank mode)")
    parser.add_argument("--local-flags")
    parser.add_argument(
        "--arch",
        choices=("x86", "aarch64", "riscv64"),
        default="x86",
        help="assembly syntax used for structural statistics (default: x86)",
    )
    parser.add_argument(
        "--oracles",
        help="comma-separated CE compilers (defaults are architecture-specific); "
             "an empty value runs local-only, the scoreboard's controlled "
             "same-binary A/B mode",
    )
    parser.add_argument("--artifact-dir", type=Path)
    parser.add_argument("--json", type=Path,
                        help="compare mode: manifest JSON; --rank mode: the "
                             "scoreboard's historical gaps/detail schema")
    parser.add_argument("--markdown", type=Path)
    parser.add_argument("--totals", action="store_true",
                        help="also print a cross-source aggregate per compiler "
                             "(summed static stats + per-source bests)")
    parser.add_argument("-j", "--jobs", type=int, default=4)
    parser.add_argument("--rank", action="store_true",
                        help="survey mode absorbed from codegen_scoreboard.py: "
                             "rank every function across all sources by "
                             "instruction-count gap to the best oracle, worst "
                             "first; exits 0 even when compilers fail (errors "
                             "go to stderr) -- the historical scoreboard "
                             "contract")
    parser.add_argument("--min-insns", type=int, default=6,
                        help="rank mode: ignore functions smaller than this many "
                             "instructions (default: %(default)s)")
    parser.add_argument("--baseline", type=Path,
                        help="rank mode: previous --rank --json output; report "
                             "per-function gap deltas versus it")
    parser.add_argument("-v", "--verbose", action="store_true",
                        help="rank mode: also list functions that are tied or "
                             "ahead of the best oracle")
    args = parser.parse_args(argv)
    if args.baseline and not args.rank:
        parser.error("--baseline is a rank-mode option; add --rank")
    return args


def _fetch_compiler_list(oracles: list[str]) -> list[dict[str, Any]]:
    """Fetch the CE compiler catalogue once per run (shared by all workers).

    Fetching it inside every :func:`_measure` worker raced the cache file's
    pid-suffixed temporary name (one process, many threads) and cost one
    network round-trip per request.  A failed fetch degrades to unknown
    metadata instead of killing the batch: each oracle then reports its own
    compile error, and compare mode still exits non-zero on it.
    """
    if not oracles:
        return []
    try:
        return godbolt.list_compilers("c")
    except Exception as exc:  # noqa: BLE001 -- degrade to per-oracle failures
        print(f"godbolt: compiler catalogue unavailable ({exc}); "
              "oracle metadata will be unknown", file=sys.stderr)
        return []


def _resolve_executable(args: argparse.Namespace) -> str | None:
    """Resolve the local LCCC path the scoreboard way (repo-anchored)."""
    if args.no_local:
        return None
    local = args.local
    if local is None:
        local = (os.environ.get("LCCC")
                 or (str(_REPO_FASTBUILD) if _REPO_FASTBUILD.exists()
                     else str(_REPO_RELEASE)))
    return str(Path(local).expanduser().resolve())


def _scoreboard_stat_dict(stats: AsmStats) -> dict[str, int]:
    """Historical codegen_scoreboard.py JSON field names (insns/…/branch/vector)."""
    return {
        "insns": stats.instructions,
        "loads": stats.loads,
        "stores": stats.stores,
        "spills": stats.spills,
        "branch": stats.branches,
        "vector": stats.vectors,
    }


RankRow = tuple[int, str, str, AsmStats, str, dict[str, AsmStats]]


def _rank_data(results: list[dict[str, Any]], oracles: list[str],
               min_insns: int) -> tuple[list[RankRow], list[dict[str, Any]]]:
    """Turn whole-TU measurements into per-file stats and gap-ranked rows.

    Mirrors codegen_scoreboard.py's main-loop ranking: rows come from the
    LOCAL compiler's functions (an oracle-only function cannot be behind),
    each function is compared against the best-scoring oracle that emitted
    it, and rows are ordered worst gap first.
    """
    rows: list[RankRow] = []
    per_file: list[dict[str, Any]] = []
    for result in results:
        stats_by_compiler: dict[str, dict[str, AsmStats]] = {}
        for record in result["records"]:
            stats_by_compiler[record["key"]] = {
                fname: _stats(body, result["arch"])
                for fname, body in _split_function_bodies(record["assembly"]).items()
            }
        errors = {error["compiler"]: error["error"] for error in result["errors"]}
        per_file.append({
            "file": result["source"],
            "compilers": stats_by_compiler,
            "errors": errors,
        })
        local_stats = stats_by_compiler.get("lccc", {})
        for fname, local in local_stats.items():
            if local.instructions < min_insns:
                continue
            best_name, best_count = None, None
            per: dict[str, AsmStats] = {}
            for oracle in oracles:
                candidate = stats_by_compiler.get(oracle, {}).get(fname)
                if candidate is None:
                    continue
                per[oracle] = candidate
                if best_count is None or candidate.instructions < best_count:
                    best_name, best_count = oracle, candidate.instructions
            if best_count is None:
                continue
            rows.append((local.instructions - best_count, Path(result["source"]).stem,
                         fname, local, best_name, per))
    rows.sort(key=lambda row: -row[0])
    return rows, per_file


def _print_rank(rows: list[RankRow], verbose: bool, baseline: dict[str, Any]) -> None:
    """The scoreboard's ranking table: worst gaps first, positive gaps only
    unless verbose, with the total gap and the behind/tied/ahead split."""
    print(f"{'gap':>5} {'benchmark':<22} {'function':<22} "
          f"{'insns':>6} {'loads':>6} {'store':>6} {'spill':>6} "
          f"{'brnch':>6} {'vec':>6}   best")
    print("-" * 108)
    total_gap = 0
    for gap, bench, fname, local, best_name, per in rows:
        if gap <= 0 and not verbose:
            continue
        total_gap += max(0, gap)
        best = per[best_name]
        delta = ""
        if baseline:
            old = baseline.get("gaps", {}).get(f"{bench}:{fname}")
            if old is not None:
                change = gap - old
                delta = f"  ({change:+d} vs baseline)" if change else "  (unchanged)"
        print(f"{gap:>5} {bench:<22} {fname:<22} "
              f"{local.instructions:6d} {local.loads:6d} {local.stores:6d} "
              f"{local.spills:6d} {local.branches:6d} {local.vectors:6d}   "
              f"{best_name}={best.instructions}{delta}")
    print("-" * 108)
    print(f"total instruction gap vs best-of-oracles: {total_gap}")
    behind = sum(1 for row in rows if row[0] > 0)
    ahead = sum(1 for row in rows if row[0] < 0)
    tied = sum(1 for row in rows if row[0] == 0)
    print(f"functions: {behind} behind, {tied} tied, {ahead} ahead "
          f"({len(rows)} compared)")


def _rank_markdown(rows: list[RankRow], verbose: bool, path: Path, flags: str) -> None:
    """Markdown rendering of the rank table (compare mode keeps its own)."""
    lines = [
        "# Codegen rank report — gap to best oracle",
        "",
        "Static per-function instruction-gap ranking, worst first, from local LCCC",
        "and the Compiler Explorer oracles. These are screening metrics, not PMU",
        "evidence; verify wins with controlled runtime and hardware counters on the",
        "intended target before making claims.",
        "",
        f"- flags: `{flags}`",
        "",
        "| Gap | Benchmark | Function | LCCC insns | Best insns | Best compiler | LCCC loads | stores | spills | branches | vectors |",
        "|---:|---|---|---:|---:|---|---:|---:|---:|---:|---:|",
    ]
    for gap, bench, fname, local, best_name, per in rows:
        if gap <= 0 and not verbose:
            continue
        best = per[best_name]
        lines.append(
            f"| {gap} | `{bench}` | `{fname}` | {local.instructions} | "
            f"{best.instructions} | {best_name} | {local.loads} | {local.stores} | "
            f"{local.spills} | {local.branches} | {local.vectors} |"
        )
    path.write_text("\n".join(lines) + "\n")


def _rank_totals(per_file: list[dict[str, Any]]) -> None:
    """Cross-compiler totals over the per-function rank statistics."""
    view: list[dict[str, Any]] = []
    for info in per_file:
        records = []
        for compiler, funcs in info["compilers"].items():
            aggregate = AsmStats()
            for stats in funcs.values():
                for field in ("instructions", "loads", "stores",
                              "spills", "branches", "vectors"):
                    setattr(aggregate, field,
                            getattr(aggregate, field) + getattr(stats, field))
            records.append({"key": compiler, **asdict(aggregate)})
        view.append({"records": records})
    _print_totals(view)


def _rank_main(args: argparse.Namespace, flags: str, oracles: list[str],
               executable: str | None) -> int:
    """Whole-corpus gap-to-best-oracle ranking (absorbed codegen_scoreboard.py).

    Exit code follows the scoreboard contract: always 0.  Compiler failures
    are reported on stderr as ``!`` lines; a rank run is a survey, not a gate
    (use compare mode without --rank when a failing compiler must fail the
    run).
    """
    functions: list[str | None] = list(args.function) if args.function else []
    if args.all_functions or not functions:
        functions.append(None)
    requests = [Request(source, function, flags, args.local_flags, args.arch)
                for source in args.sources for function in functions]
    compilers = _fetch_compiler_list(oracles)
    max_workers = max(1, min(args.jobs, len(requests)))
    results: list[dict[str, Any]] = []
    with concurrent.futures.ThreadPoolExecutor(max_workers=max_workers) as pool:
        futures = [pool.submit(_measure, req, executable, oracles, compilers)
                   for req in requests]
        for future in concurrent.futures.as_completed(futures):
            result = future.result()
            results.append(result)
            if args.artifact_dir:
                _write_artifacts(result, args.artifact_dir)
    order = {req: idx for idx, req in enumerate(requests)}
    results.sort(key=lambda item: order[Request(
        Path(item["source"]), item.get("function"), flags, args.local_flags, args.arch
    )])

    rows, per_file = _rank_data(results, oracles, args.min_insns)

    baseline: dict[str, Any] = {}
    if args.baseline and args.baseline.exists():
        baseline = json.loads(args.baseline.read_text())

    _print_rank(rows, args.verbose, baseline)

    for info in per_file:
        for who, error in info["errors"].items():
            print(f"  ! {Path(info['file']).stem}: {who}: {error}", file=sys.stderr)

    if args.json:
        payload = {
            "flags": flags,
            "gaps": {f"{bench}:{fname}": gap for gap, bench, fname, *_ in rows},
            "detail": [
                {
                    "file": info["file"],
                    "compilers": {
                        compiler: {fname: _scoreboard_stat_dict(stats)
                                   for fname, stats in funcs.items()}
                        for compiler, funcs in info["compilers"].items()
                    },
                    "errors": info["errors"],
                }
                for info in per_file
            ],
        }
        args.json.parent.mkdir(parents=True, exist_ok=True)
        tmp = args.json.with_suffix(args.json.suffix + f".tmp.{os.getpid()}")
        tmp.write_text(json.dumps(payload, indent=1) + "\n")
        tmp.replace(args.json)
        print(f"wrote {args.json}")
    if args.markdown:
        _rank_markdown(rows, args.verbose, args.markdown, flags)
    if args.totals:
        _rank_totals(per_file)
    return 0


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    flags = (args.flags if args.flags is not None
             else (RANK_DEFAULT_FLAGS if args.rank else DEFAULT_FLAGS))
    default_oracles = (
        DEFAULT_AARCH64_ORACLES
        if args.arch == "aarch64"
        else DEFAULT_RISCV64_ORACLES
        if args.arch == "riscv64"
        else DEFAULT_ORACLES
    )
    oracle_text = args.oracles if args.oracles is not None else ",".join(default_oracles)
    oracles = [item.strip() for item in oracle_text.split(",") if item.strip()]
    executable = _resolve_executable(args)
    if args.rank:
        return _rank_main(args, flags, oracles, executable)
    functions: list[str | None] = list(args.function) if args.function else []
    if args.all_functions or not functions:
        functions.append(None)
    requests = [Request(source, function, flags, args.local_flags, args.arch)
                for source in args.sources for function in functions]
    compilers = _fetch_compiler_list(oracles)
    max_workers = max(1, min(args.jobs, len(requests)))
    results: list[dict[str, Any]] = []
    with concurrent.futures.ThreadPoolExecutor(max_workers=max_workers) as pool:
        futures = [pool.submit(_measure, req, executable, oracles, compilers) for req in requests]
        for future in concurrent.futures.as_completed(futures):
            result = future.result()
            results.append(result)
            _print_table(result)
            if args.artifact_dir:
                _write_artifacts(result, args.artifact_dir)
    order = {req: idx for idx, req in enumerate(requests)}
    results.sort(key=lambda item: order[Request(
        Path(item["source"]), item.get("function"), flags, args.local_flags, args.arch
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
