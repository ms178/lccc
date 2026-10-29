#!/usr/bin/env python3
"""Diagnose native floating-point output mismatches with a no-contraction control.

This runs only after a correctness-gated research comparison has failed. The
original failure is retained. A one-execution control is not a timing result;
its outputs, exact commands, binaries and disassemblies are kept with the
research artifact. The bounded check annotation is useful when artifact storage
is not reachable from a reviewer.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parent.parent
FUSED = re.compile(r"\bv(?:fmadd|fmsub|fnmadd|fnmsub)[a-z0-9]*\b", re.I)


def mismatched_fp_cases(report: dict) -> list[str]:
    """Select floating-point cases with a real, stable differential mismatch."""
    cases = []
    for row in report.get("benchmarks", []):
        if "fp" not in row.get("tags", []):
            continue
        checks = row.get("correctness", {})
        baseline = checks.get(row.get("baseline_compiler"), {})
        if not baseline.get("all_runs_successful") or not baseline.get("stable_within_compiler"):
            continue
        if any(check.get("matches_baseline") is False
               and check.get("all_runs_successful") and check.get("stable_within_compiler")
               for check in checks.values()):
            cases.append(row["name"])
    return cases


def control_command(report: dict, cases: list[str], out: Path, *, lccc: str,
                    gcc: str, clang: str, icx: str, candidates: list[str]) -> list[str]:
    """Match the failed native profile except for explicit FP contraction."""
    command = [sys.executable, str(ROOT / "tests/benchmark/run_benchmarks.py"),
               "--lccc", lccc, "--compilers", "lccc,gcc,clang,icx",
               "--gcc", gcc, "--clang", clang, "--icx", icx,
               "--opt=" + report["settings"]["opt"]]
    for flag in report["settings"]["common_cflags"]:
        if not flag.startswith("-ffp-contract="):
            command.append("--cflag=" + flag)
    command.append("--cflag=-ffp-contract=off")
    for candidate in candidates:
        command.extend(("--candidate", candidate))
    command.extend(("--only", ",".join(cases), "--cpu", "auto", "--skip-perf",
                    "--no-pmu-probe", "--strict", "--artifact-dir", str(out / "artifacts"),
                    "--json", str(out / "control.json")))
    return command


def observations(report: dict, cases: list[str], artifacts: Path) -> dict:
    """Record outputs and actual emitted fused instructions, not inferred flags."""
    rows = {}
    for row in report.get("benchmarks", []):
        name = row.get("name")
        if name not in cases:
            continue
        arms = {}
        for key, check in row.get("correctness", {}).items():
            asm = artifacts / name / key / "assembly.txt"
            lines = asm.read_text(encoding="utf-8", errors="replace").splitlines() if asm.exists() else []
            fused = [line.strip() for line in lines if FUSED.search(line)]
            output = check.get("output")
            arms[key] = {
                "matches_baseline": check.get("matches_baseline"),
                "stable": check.get("stable_within_compiler"),
                "successful": check.get("all_runs_successful"),
                "output": output[:160] if isinstance(output, str) else output,
                "fused_instructions": len(fused) if asm.exists() else None,
                "first_fused_instruction": fused[0][:160] if fused else None,
            }
        rows[name] = {"baseline": row.get("baseline_compiler"), "arms": arms}
    return rows


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--original-artifacts", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--lccc", required=True)
    parser.add_argument("--gcc", required=True)
    parser.add_argument("--clang", required=True)
    parser.add_argument("--icx", required=True)
    parser.add_argument("--candidate", action="append", default=[])
    args = parser.parse_args(argv)
    if not args.report.exists():
        print("No native report was produced; the original research failure remains authoritative.")
        return 0
    report = json.loads(args.report.read_text(encoding="utf-8"))
    cases = mismatched_fp_cases(report)
    if not cases:
        print("No stable floating-point mismatch to diagnose; the original research failure remains authoritative.")
        return 0
    args.out.mkdir(parents=True, exist_ok=True)
    command = control_command(report, cases, args.out, lccc=args.lccc, gcc=args.gcc,
                              clang=args.clang, icx=args.icx, candidates=args.candidate)
    (args.out / "command.json").write_text(json.dumps(command, indent=2) + "\n", encoding="utf-8")
    run = subprocess.run(command, cwd=ROOT, text=True, capture_output=True, check=False)
    (args.out / "control.log").write_text(run.stdout + "\n--- stderr ---\n" + run.stderr, encoding="utf-8")
    path = args.out / "control.json"
    control = json.loads(path.read_text(encoding="utf-8")) if path.exists() else {}
    detail = {"cases": cases, "control_exit": run.returncode,
              "default": observations(report, cases, args.original_artifacts),
              "contract_off": observations(control, cases, args.out / "artifacts")}
    text = json.dumps(detail, separators=(",", ":"), ensure_ascii=True)
    (args.out / "diagnostic.json").write_text(json.dumps(detail, indent=2) + "\n", encoding="utf-8")
    # Keep the verdict and outputs if a large diagnostic must be shortened; the
    # complete instruction excerpts remain in diagnostic.json and artifacts.
    if len(text.encode()) > 3900:
        for mode in ("default", "contract_off"):
            for row in detail[mode].values():
                for arm in row["arms"].values():
                    arm.pop("first_fused_instruction", None)
        text = json.dumps(detail, separators=(",", ":"), ensure_ascii=True)
    if len(text.encode()) > 3900:
        raise ValueError("FP diagnostic exceeds the GitHub annotation budget")
    escaped = text.replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")
    print("::notice title=Native floating-point mismatch control::" + escaped)
    return run.returncode


if __name__ == "__main__":
    raise SystemExit(main())
