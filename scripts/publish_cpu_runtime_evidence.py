#!/usr/bin/env python3
"""Publish auditable same-host native/portable research receipts, not a speed gate.

Checks annotations remain accessible when artifact-storage downloads are blocked.
Raw samples, binaries and disassembly remain in the uploaded research artifact.
Never infer a before/after gain from a vendor comparison on another CPU.
"""
from __future__ import annotations

import hashlib
import json
from pathlib import Path
import sys


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
        for ref in ("before", "gcc", "clang"):
            r = b["ratios_from_lccc"][ref]
            ratios[ref] = {"median": r["median"], "ci95": r["median_ci95_bootstrap"]}
        rows.append({
            "profile": profile, "name": b["name"], "ratios_after_over": ratios,
            "whole_executable_identical": b["compile"]["lccc"]["binary_sha256"] == b["compile"]["before"]["binary_sha256"],
            "source_sha256": b["source_sha256"],
        })
    if not rows:
        raise ValueError("empty report")
    return rows


def annotate(title: str, text: str) -> None:
    escaped = text.replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")
    print(f"::notice title={title}::{escaped}")


def main() -> int:
    root = Path(sys.argv[1])
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
    # Preserve the exact measured disassembly even without artifact-host access.
    for profile in ("portable", "native"):
        for name in ("nbody",):
            for arm in ("lccc", "before", "gcc", "clang"):
                path = root / f"{profile}-artifacts" / name / arm / "assembly.txt"
                if path.exists():
                    lines = path.read_text().splitlines()
                    selected = set()
                    for pos, line in enumerate(lines):
                        if "sqrtss" in line or "sqrtsd" in line:
                            selected.update(range(max(0, pos - 24), min(len(lines), pos + 13)))
                    text = "\n".join(lines[i] for i in sorted(selected))[:9000]
                    for i in range(0, len(text), 3500):
                        annotate(f"CPU disassembly {profile}/{name}/{arm}/{i // 3500}", text[i:i + 3500])
    return int(failed)


if __name__ == "__main__":
    raise SystemExit(main())
