#!/usr/bin/env python3
"""Rank a `run_benchmarks.py` evidence JSON by LCCC-vs-reference regression.

`run_benchmarks.py` produces the raw evidence; its Markdown report lists the
workloads in corpus order.  That ordering hides exactly what a code-generation
investigation needs: *which* workloads are furthest behind, whether the gap is
statistically real, and whether a headline geometric mean is being carried by a
single spectacular win while a dozen workloads quietly regress.

This tool therefore:

  * sorts every correct benchmark pair by median ratio (worst first);
  * marks each gap as SIGNIFICANT / NOISE using the runner's own paired
    bootstrap CI on the ratio (a CI excluding 1.0 is a real difference);
  * reports the geometric mean **and** the top-improvement / top-regression /
    unchanged split, so no aggregate can conceal a catastrophic regression
    (project policy: "a global geomean must not hide a regression");
  * flags pairs whose samples are too short or too noisy to support any claim.

It reads only the JSON the runner already wrote, so it adds no measurement and
cannot perturb a run in progress.

Examples
--------
    # The ten worst workloads of a recorded run
    scripts/bench_rank.py results/baseline.json --worst 10

    # Every workload, with code-size columns, as Markdown for a follow-up doc
    scripts/bench_rank.py results/baseline.json --all --markdown

    # Diff two runs: what did this session's change move?
    scripts/bench_rank.py after.json --compare before.json
"""
from __future__ import annotations

import argparse
import json
import math
import sys
from pathlib import Path
from typing import Any


def _median(stats: dict[str, Any] | None) -> float | None:
    if not stats:
        return None
    value = stats.get("median")
    return float(value) if isinstance(value, (int, float)) else None


def load_rows(path: Path) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    """Flatten one evidence JSON into per-workload comparison rows."""
    document = json.loads(path.read_text(encoding="utf-8"))
    rows: list[dict[str, Any]] = []
    for bench in document.get("benchmarks", []):
        runtime = bench.get("runtime", {})
        reference = bench.get("baseline_compiler") or "gcc"
        lccc_wall = _median((runtime.get("lccc") or {}).get("wall"))
        ref_wall = _median((runtime.get(reference) or {}).get("wall"))

        ratios = (bench.get("ratios_to_" + reference) or {}).get("lccc") or {}
        ci = ratios.get("median_ci95_bootstrap")
        ratio = _median(ratios)
        if ratio is None and lccc_wall and ref_wall:
            ratio = lccc_wall / ref_wall

        significant = None
        if isinstance(ci, (list, tuple)) and len(ci) == 2:
            # A paired bootstrap CI that excludes 1.0 means the median ratio is
            # distinguishable from parity at 95%; one that straddles 1.0 does
            # not, and must never be reported as a win or a regression.
            significant = not (float(ci[0]) <= 1.0 <= float(ci[1]))

        compile_info = bench.get("compile", {})
        correctness = bench.get("correctness", {})
        warnings = sorted({
            warning
            for per_compiler in runtime.values()
            if isinstance(per_compiler, dict)
            for warning in per_compiler.get("quality_warnings", [])
        })
        rows.append({
            "name": bench.get("name", "?"),
            "description": bench.get("description", ""),
            "ratio": ratio,
            "significant": significant,
            "ci": ci,
            "cv": ratios.get("cv"),
            "lccc_s": lccc_wall,
            "ref_s": ref_wall,
            "reference": reference,
            "lccc_text": (compile_info.get("lccc") or {}).get("text_bytes"),
            "ref_text": (compile_info.get(reference) or {}).get("text_bytes"),
            "correct": all(
                (correctness.get(key) or {}).get("matches_baseline", False)
                for key in ("lccc", reference)
                if key in correctness
            ) if correctness else False,
            "lccc_ok": (compile_info.get("lccc") or {}).get("ok", False),
            "ref_ok": (compile_info.get(reference) or {}).get("ok", False),
            "warnings": warnings,
        })
    return rows, document


def geometric_mean(values: list[float]) -> float | None:
    positive = [value for value in values if value and value > 0]
    if not positive:
        return None
    return math.exp(sum(math.log(value) for value in positive) / len(positive))


def classify(rows: list[dict[str, Any]], threshold: float) -> dict[str, list[dict[str, Any]]]:
    """Split into regressed / improved / unchanged using the policy threshold.

    Only statistically significant pairs may land in regressed/improved: the
    project's regression policy (>1% investigate, >2% strong investigation) is
    meaningless when the measurement noise is larger than the effect.
    """
    regressed, improved, unchanged, unknown = [], [], [], []
    for row in rows:
        ratio = row.get("ratio")
        if not ratio or not row.get("correct"):
            unknown.append(row)
            continue
        if row.get("significant") is not True:
            unchanged.append(row)
        elif ratio > 1.0 + threshold:
            regressed.append(row)
        elif ratio < 1.0 - threshold:
            improved.append(row)
        else:
            unchanged.append(row)
    return {
        "regressed": regressed,
        "improved": improved,
        "unchanged": unchanged,
        "unknown": unknown,
    }


def format_ms(seconds: float | None) -> str:
    if seconds is None:
        return "—"
    if seconds >= 1.0:
        return f"{seconds * 1000:.1f}ms"
    return f"{seconds * 1000:.2f}ms"


def render_table(rows: list[dict[str, Any]], *, show_size: bool, reference: str) -> str:
    header = (f"{'#':>3} {'workload':<24} {'lccc':>10} {reference:>10} "
              f"{'ratio':>7} {'sig':>4} {'95% CI':>16}")
    if show_size:
        header += f" {'text L':>8} {'text R':>8}"
    lines = [header, "-" * len(header)]
    for index, row in enumerate(rows, 1):
        ratio = row.get("ratio")
        ci = row.get("ci")
        ci_text = (f"[{ci[0]:.3f},{ci[1]:.3f}]"
                   if isinstance(ci, (list, tuple)) and len(ci) == 2 else "—")
        sig = {True: "YES", False: "no", None: "?"}[row.get("significant")]
        line = (f"{index:>3} {row['name']:<24} {format_ms(row.get('lccc_s')):>10} "
                f"{format_ms(row.get('ref_s')):>10} "
                f"{ratio:>7.3f} {sig:>4} {ci_text:>16}" if ratio else
                f"{index:>3} {row['name']:<24} {'—':>10} {'—':>10} {'—':>7} {sig:>4} {ci_text:>16}")
        if show_size:
            line += (f" {row.get('lccc_text') or 0:>8} {row.get('ref_text') or 0:>8}")
        if not row.get("correct"):
            line += "   <-- INCORRECT/FAILED"
        lines.append(line)
    return "\n".join(lines)


def render_markdown(rows: list[dict[str, Any]], document: dict[str, Any],
                    groups: dict[str, list[dict[str, Any]]], threshold: float) -> str:
    aggregate = document.get("aggregate_lccc_vs_gcc") or {}
    out = ["# LCCC benchmark ranking (worst first)", ""]
    out.append(f"- workloads: **{len(rows)}**")
    if aggregate:
        out.append(f"- geometric mean ratio: **{aggregate.get('geometric_mean', float('nan')):.4f}**")
        out.append(f"- arithmetic mean ratio: **{aggregate.get('arithmetic_mean', float('nan')):.4f}**")
    out.append(f"- policy threshold: ±{threshold * 100:.0f}% (significant pairs only)")
    for key in ("regressed", "improved", "unchanged", "unknown"):
        out.append(f"- {key}: **{len(groups[key])}**")
    out += ["", "| # | workload | lccc | ref | ratio | significant | 95% CI |",
            "|---|----------|-----:|----:|------:|:-----------:|--------|"]
    for index, row in enumerate(rows, 1):
        ci = row.get("ci")
        ci_text = (f"[{ci[0]:.3f}, {ci[1]:.3f}]"
                   if isinstance(ci, (list, tuple)) and len(ci) == 2 else "—")
        ratio = row.get("ratio")
        sig = {True: "yes", False: "no", None: "?"}[row.get("significant")]
        out.append(f"| {index} | `{row['name']}` | {format_ms(row.get('lccc_s'))} | "
                   f"{format_ms(row.get('ref_s'))} | "
                   f"{ratio:.3f} | {sig} | {ci_text} |" if ratio else
                   f"| {index} | `{row['name']}` | — | — | — | {sig} | {ci_text} |")
    return "\n".join(out) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("evidence", type=Path, help="run_benchmarks.py --json output")
    parser.add_argument("--worst", type=int, default=10,
                        help="how many worst workloads to print (default: %(default)s)")
    parser.add_argument("--best", type=int, default=5,
                        help="how many best workloads to print (default: %(default)s)")
    parser.add_argument("--all", action="store_true", help="print every workload")
    parser.add_argument("--threshold", type=float, default=0.01,
                        help="fractional change counted as a regression (default: %(default)s)")
    parser.add_argument("--size", action="store_true", help="include .text size columns")
    parser.add_argument("--markdown", action="store_true", help="emit Markdown instead of text")
    parser.add_argument("--json", type=Path, help="write the ranked rows as JSON")
    parser.add_argument("--compare", type=Path,
                        help="second evidence JSON; report the per-workload delta")
    args = parser.parse_args()

    rows, document = load_rows(args.evidence)
    reference = rows[0]["reference"] if rows else "gcc"

    # Worst first: a missing/failed ratio sorts last rather than masquerading
    # as an infinite regression.
    rows.sort(key=lambda row: (row.get("ratio") is None, -(row.get("ratio") or 0.0)))
    groups = classify(rows, args.threshold)

    if args.json:
        args.json.parent.mkdir(parents=True, exist_ok=True)
        args.json.write_text(json.dumps(rows, indent=1), encoding="utf-8")

    if args.compare:
        before, _ = load_rows(args.compare)
        previous = {row["name"]: row.get("ratio") for row in before}
        print(f"{'workload':<24} {'before':>8} {'after':>8} {'delta':>9}")
        print("-" * 54)
        deltas = []
        for row in rows:
            old, new = previous.get(row["name"]), row.get("ratio")
            if old and new:
                deltas.append((new / old, row["name"], old, new))
        for delta, name, old, new in sorted(deltas, reverse=True):
            marker = "  REGRESSED" if delta > 1.02 else ("  improved" if delta < 0.98 else "")
            print(f"{name:<24} {old:>8.3f} {new:>8.3f} {delta:>8.3f}x{marker}")
        return 0

    if args.markdown:
        sys.stdout.write(render_markdown(rows, document, groups, args.threshold))
        return 0

    selected = rows if args.all else rows[: args.worst]
    print(f"WORST {len(selected)} of {len(rows)} (ratio = lccc/{reference}; >1 is slower)")
    print(render_table(selected, show_size=args.size, reference=reference))
    if args.best and not args.all:
        print()
        print(f"BEST {args.best}")
        print(render_table(rows[-args.best:][::-1], show_size=args.size, reference=reference))

    ratios = [row["ratio"] for row in rows if row.get("ratio") and row.get("correct")]
    geo = geometric_mean(ratios)
    print()
    print(f"n={len(ratios)}  geometric mean={geo:.4f}" if geo else "no comparable pairs")
    if ratios:
        print(f"      arithmetic mean={sum(ratios) / len(ratios):.4f}  "
              f"best={min(ratios):.3f}  worst={max(ratios):.3f}")
    for key in ("regressed", "improved", "unchanged", "unknown"):
        names = ", ".join(row["name"] for row in groups[key][:12])
        suffix = " …" if len(groups[key]) > 12 else ""
        print(f"  {key:>9}: {len(groups[key]):>2}  {names}{suffix}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
