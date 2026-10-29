#!/usr/bin/env python3
"""Publish auditable same-host native/portable research receipts, not a speed gate.

Checks annotations remain accessible when artifact-storage downloads are blocked.
Raw samples, binaries and disassembly remain in the uploaded research artifact.
Never infer a before/after gain from a vendor comparison on another CPU.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re


# Bounded excerpts of the actual timed binaries, not a separate compiler run.
# GCC may give a specialized clone a suffix such as .constprop.0. The complete
# disassemblies and the exact binaries remain in the research artifact.
PRIORITY_SYMBOLS = {
    "zlib_ng_slide_hash": ("slide_hash_c_chain", "zng_slide_hash_c", "zng_insert_string_batch"),
    "fp64_product_map": ("fp_product_map", "main"),
    "fp32_product_map": ("fp_product_map", "main"),
    "byte_load_reassembly": ("main",),
    "linux_list_sort": ("merge_and_count", "merge", "list_sort"),
    "sqlite_vdbe_serial": ("vdbe_serial_get", "scan_all_records"),
    "zlib_ng_compare256": ("zlib_compare256_64",),
    "arith_loop": ("arith_loop",),
}


def symbol_excerpt(disassembly: str, names: tuple[str, ...], limit: int = 7000) -> str:
    """Return named objdump functions, including compiler-suffixed clones.

    A missing symbol is reported rather than replaced by an unrelated routine;
    it may have been inlined. Excerpts explicitly say when they were truncated.
    """
    header = re.compile(r"^[0-9a-fA-F]+ <([^>]+)>:\s*$")
    selected = []
    keep = False
    for line in disassembly.splitlines():
        match = header.match(line)
        if match:
            symbol = match.group(1)
            keep = any(symbol == name or symbol.startswith(name + ".") for name in names)
        elif line.startswith("Disassembly of section "):
            keep = False
        if keep:
            selected.append(line)
    if not selected:
        return "No named function found (it may have been inlined)."
    text = "\n".join(selected)
    if len(text) > limit:
        text = text[:limit] + "\n[excerpt truncated; full disassembly is in the artifact]"
    return text


def summaries(report: dict, profile: str) -> list[dict]:
    settings = report["settings"]
    flags = settings["common_cflags"]
    if settings["opt"] != "-O2" or settings["reps"] < 9 or settings["warmup"] < 3:
        raise ValueError("insufficient or foreign timing settings")
    if ("-march=native" in flags) != (profile == "native"):
        raise ValueError("profile label does not match compiler flags")
    if not report["metadata"]["pinning"]["applied"]:
        raise ValueError("CPU affinity was not applied")
    identities = {c["key"]: c["binary_sha256"] for c in report["compilers"]}
    if not {"lccc", "before", "gcc", "clang"} <= identities.keys():
        raise ValueError("missing before/current/vendor compiler")
    rows = []
    names = set()
    for b in report["benchmarks"]:
        if b["name"] in names:
            raise ValueError("duplicate workload")
        names.add(b["name"])
        for arm in identities:
            c = b["correctness"][arm]
            if not all(c[k] for k in ("all_runs_successful", "matches_baseline", "stable_within_compiler")):
                raise ValueError(f"incorrect workload: {b['name']}/{arm}")
            if b["runtime"][arm]["wall"]["n"] != settings["reps"]:
                raise ValueError("incomplete paired rounds")
        ratios = {}
        for ref in ("before", "gcc", "clang", "icx", "rolled"):
            if ref in identities:
                r = b["ratios_from_lccc"][ref]
                ratios[ref] = {"median": r["median"], "ci95": r["median_ci95_bootstrap"]}
        row = {
            "profile": profile, "name": b["name"], "ratios_after_over": ratios,
            "whole_executable_identical": b["compile"]["lccc"]["binary_sha256"] == b["compile"]["before"]["binary_sha256"],
            "source_sha256": b["source_sha256"],
        }
        if "rolled" in identities:
            row["whole_executable_identical_rolled"] = (
                b["compile"]["lccc"]["binary_sha256"] == b["compile"]["rolled"]["binary_sha256"]
            )
        rows.append(row)
    if not rows:
        raise ValueError("empty report")
    return rows


def annotate(title: str, text: str) -> None:
    escaped = text.replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")
    print(f"::notice title={title}::{escaped}")


def publish_disassembly(root: Path, profile: str, cases: list[str]) -> None:
    """Publish one bounded notice per workload, with all arms labeled.

    GitHub can cap annotations for the whole job, not just each step. Grouping
    by workload leaves room for timing, startup-excluded and Callgrind results.
    """
    for name in cases:
        sections = []
        if name == "nbody":
            for arm in ("lccc", "before", "gcc", "clang"):
                path = root / f"{profile}-artifacts" / name / arm / "assembly.txt"
                if not path.exists():
                    continue
                lines = path.read_text().splitlines()
                selected = set()
                for pos, line in enumerate(lines):
                    if "sqrtss" in line or "sqrtsd" in line:
                        selected.update(range(max(0, pos - 24), min(len(lines), pos + 13)))
                text = "\n".join(lines[i] for i in sorted(selected))[:3000]
                sections.append(f"=== {arm} ===\n{text or 'No scalar sqrt found.'}")
        else:
            symbols = PRIORITY_SYMBOLS[name]
            if name in ("fp32_product_map", "fp64_product_map"):
                arms = ("lccc", "gcc", "icx", "rolled")
            elif name == "zlib_ng_slide_hash":
                arms = ("lccc", "gcc", "icx")
            else:
                arms = ("lccc", "gcc")
            for arm in arms:
                path = root / f"{profile}-artifacts" / name / arm / "assembly.txt"
                if path.exists():
                    disassembly = path.read_text()
                    for symbol in symbols:
                        # Map callers can dominate tiny kernels; retain their
                        # hot loops for the two primary compilers, not just the
                        # callee. Bound each grouped notice below API limits.
                        if name in ("fp32_product_map", "fp64_product_map") and symbol == "main" and arm not in ("lccc", "gcc"):
                            continue
                        long_main = symbol == "main" and name in (
                            "byte_load_reassembly", "fp32_product_map", "fp64_product_map")
                        limit = 7000 if long_main else 3000
                        text = symbol_excerpt(disassembly, (symbol,), limit=limit)
                        sections.append(f"=== {arm}/{symbol} ===\n{text}")
        if sections:
            annotate(f"CPU disassembly {profile}/{name}", "\n\n".join(sections))


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("root", type=Path)
    parser.add_argument("--assembly-profile", choices=("portable", "native"))
    parser.add_argument("--assembly-cases", help="comma-separated named workloads")
    args = parser.parse_args()
    root = args.root
    if args.assembly_profile or args.assembly_cases:
        if not args.assembly_profile or not args.assembly_cases:
            parser.error("assembly publication requires both profile and cases")
        cases = args.assembly_cases.split(",")
        if any(case not in PRIORITY_SYMBOLS and case != "nbody" for case in cases):
            parser.error("unknown assembly workload")
        publish_disassembly(root, args.assembly_profile, cases)
        return 0
    failed = False
    reports = {}
    for profile in ("portable", "native"):
        path = root / f"{profile}.json"
        if not path.exists():
            annotate("CPU runtime report missing", profile)
            failed = True
            continue
        try:
            report = json.loads(path.read_text())
            rows = summaries(report, profile)
            reports[profile] = report
            metadata = report["metadata"]
            identity = {"profile": profile, "cpu": metadata["cpu_models"],
                        "settings": report["settings"], "pinning": metadata["pinning"],
                        "compiler_versions": metadata["compiler_versions"],
                        "report_sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
            annotate("CPU runtime identity", json.dumps(identity, separators=(",", ":")))
            for start in range(0, len(rows), 5):
                text = json.dumps(rows[start:start + 5], separators=(",", ":"))
                if len(text.encode()) > 3900:
                    raise ValueError("annotation budget exceeded")
                annotate("CPU paired runtime", text)
        except (ValueError, KeyError) as exc:
            annotate("CPU runtime report rejected", f"{profile}: {exc}")
            failed = True
    if len(reports) == 2:
        a, b = reports["portable"], reports["native"]
        for key in ("cpu_models", "pinning"):
            if a["metadata"][key] != b["metadata"][key]:
                raise ValueError("profiles did not use the same CPU/affinity")
        if [(x["name"], x["source_sha256"]) for x in a["benchmarks"]] != [(x["name"], x["source_sha256"]) for x in b["benchmarks"]]:
            raise ValueError("profile workloads/sources changed")
    return int(failed)


if __name__ == "__main__":
    raise SystemExit(main())
