#!/usr/bin/env python3
"""Replay the tracked historical PR #629 isolated inputs against two assemblers.

GAS 2.47 is the oracle; compare acceptance, emitted .text bytes, and ELF
relocations. Diagnostics are captured for rejected inputs, but wording is not
an assertion: independently documented historical wording differences exist.
Unlike a path-only evidence claim, the input file and this runner are portable.
"""
from __future__ import annotations

import argparse
import concurrent.futures
import hashlib
import json
import subprocess
import sys
import tempfile
from collections import Counter
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
from asmdiff import read_elf  # noqa: E402  (the repository's ELF32/ELF64 parser)


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def version(path: Path) -> str:
    p = subprocess.run([str(path), "--version"], capture_output=True, text=True, timeout=10)
    if p.returncode:
        raise RuntimeError(f"{path}: --version failed: {p.stderr}")
    return p.stdout.splitlines()[0]


def assemble(executable: Path, is_lccc: bool, source: str, directory: Path) -> dict:
    directory.mkdir()
    src = directory / "probe.s"
    obj = directory / "probe.o"
    src.write_text(".text\n" + source + "\n")
    cmd = [str(executable), *(["-c"] if is_lccc else []), str(src), "-o", str(obj)]
    proc = subprocess.run(cmd, capture_output=True, text=True, timeout=30)
    if proc.returncode or not obj.is_file():
        # Paths are intentionally excluded from tracked results. A single
        # terminal diagnostic lets a human inspect unexpected rejections.
        lines = (proc.stderr or proc.stdout).strip().splitlines()
        return {"ok": False, "diagnostic": lines[-1].split(": ", 1)[-1] if lines else f"exit {proc.returncode}"}
    elf = read_elf(obj)
    return {"ok": True, "text": elf.content.get(".text", b"").hex(" "),
            "relocs": elf.relocs.get(".text", [])}


def compare(case: dict, tools: dict[str, tuple[Path, Path]], tmp: Path, index: int) -> dict:
    gas, lccc = tools[case["target"]]
    folder = tmp / f"{index:04d}"
    folder.mkdir()
    expected = assemble(gas, False, case["source"], folder / "gas")
    actual = assemble(lccc, True, case["source"], folder / "lccc")
    match = expected["ok"] == actual["ok"]
    if expected["ok"] and actual["ok"]:
        match = (expected["text"], expected["relocs"]) == (actual["text"], actual["relocs"])
    return {"id": case["id"], "series": case["series"], "target": case["target"],
            "source": case["source"], "match": match, "gas": expected, "lccc": actual}


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--as-x64", required=True, type=Path)
    p.add_argument("--as-i686", required=True, type=Path)
    p.add_argument("--lccc-x64", default=ROOT / "target/fastbuild/lccc-x86", type=Path)
    p.add_argument("--lccc-i686", default=ROOT / "target/fastbuild/lccc-i686", type=Path)
    p.add_argument("--jobs", default=2, type=int)
    p.add_argument("--inputs", default=HERE / "pr629-probe-inputs.json", type=Path)
    p.add_argument("--output", default=HERE / "followup-isolated-probes.json", type=Path)
    p.add_argument("--allow-mismatches", action="store_true",
                   help="baseline runs only: record divergences but return success")
    args = p.parse_args()
    if args.jobs < 1:
        p.error("--jobs must be positive")
    tools = {"x86_64": (args.as_x64.resolve(), args.lccc_x64.resolve()),
             "i686": (args.as_i686.resolve(), args.lccc_i686.resolve())}
    for target, (gas, lccc) in tools.items():
        if "2.47" not in version(gas):
            p.error(f"{target}: expected GNU as 2.47, got {version(gas)}")
        if not lccc.is_file():
            p.error(f"missing {target} assembler: {lccc}")
    inputs = args.inputs.resolve()
    cases = json.loads(inputs.read_text())["cases"]
    with tempfile.TemporaryDirectory(prefix="pr630-isolated-") as temp:
        with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as pool:
            futures = [pool.submit(compare, c, tools, Path(temp), i)
                       for i, c in enumerate(cases)]
            rows = [future.result() for future in futures]
    summary = {
        "repository_head": subprocess.check_output(["git", "-C", str(ROOT), "rev-parse", "HEAD"], text=True).strip(),
        "note": "Compiled binaries (not merely HEAD); identify them by SHA-256 below. Diagnostic wording is informational.",
        "inputs": str(inputs.relative_to(ROOT)),
        "inputs_sha256": sha256(inputs),
        "oracle": {target: {"version": version(gas), "sha256": sha256(gas)}
                   for target, (gas, _lccc) in tools.items()},
        "compiler_sha256": {target: sha256(lccc) for target, (_gas, lccc) in tools.items()},
        "per_series": {},
        "mismatches": [r for r in rows if not r["match"]],
    }
    for series in sorted({r["series"] for r in rows}):
        series_rows = [r for r in rows if r["series"] == series]
        summary["per_series"][series] = {
            "total": len(series_rows), "accepted_by_gas": sum(r["gas"]["ok"] for r in series_rows),
            "accepted_by_lccc": sum(r["lccc"]["ok"] for r in series_rows),
            "matches": sum(r["match"] for r in series_rows),
            "mismatch_reasons": dict(Counter("acceptance" if r["gas"]["ok"] != r["lccc"]["ok"]
                                             else "bytes_or_relocs" for r in series_rows if not r["match"])),
        }
    args.output.write_text(json.dumps(summary, indent=2) + "\n")
    for series, totals in summary["per_series"].items():
        print(f"{series}: {totals['matches']}/{totals['total']} accept/byte/reloc matches; "
              f"GAS accepted {totals['accepted_by_gas']}, LCCC accepted {totals['accepted_by_lccc']}")
    print(f"Saved {args.output} ({len(summary['mismatches'])} mismatches)")
    return int(bool(summary["mismatches"]) and not args.allow_mismatches)


if __name__ == "__main__":
    sys.exit(main())
