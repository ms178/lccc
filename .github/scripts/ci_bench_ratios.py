#!/usr/bin/env python3
"""Publish a benchmark shard's paired LCCC/GCC ratios as one annotation.

Why this exists
---------------
The benchmark lane's numbers are produced by
``tests/benchmark/run_benchmarks.py`` and land in a JSON report and a Markdown
summary. Both are *artifacts*, and an artifact is not readable through the
checks API: `gh api .../actions/jobs/<id>/logs` redirects to a blob host, and a
reusable-workflow job's ``$GITHUB_STEP_SUMMARY`` does **not** surface in
``output.summary`` (the same defect ``ci-codegen-gate.py`` documents for the
static metrics). A reviewer -- or an agent doing a performance change -- can
therefore see that the lane passed but not what it measured, which makes every
performance claim unfalsifiable from the outside.

One annotation fixes that: ``/repos/{o}/{r}/check-runs/{id}/annotations``
returns the warning and error levels, so a compact table is API-visible. It is
publishing evidence, not gating: the exit status is always 0, because
``ci-bench.py --strict`` already owns correctness and the merge job owns
report integrity.

Opt-in
------
The table is always printed to stdout (it belongs in the log). The annotation
is emitted only when ``LCCC_BENCH_RATIOS`` is set, exactly like
``LCCC_CODEGEN_ASM`` for the codegen gate: a normal run carries no annotations.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path

SCHEMA = "lccc-bench-v2"


def gha_escape(text: str) -> str:
    """Escape a workflow-command payload (property values are `%`/`,`/`:`)."""
    return (
        text.replace("%", "%25")
        .replace("\r", "%0D")
        .replace("\n", "%0A")
        .replace(",", "%2C")
        .replace(":", "%3A")
    )


def collect(report: dict) -> tuple[list[tuple[float, str, int, list[str]]], dict]:
    """Return [(ratio, name, rounds, quality_warnings)] and the aggregate block.

    Only benchmarks whose paired ratio exists are included: the runner skips
    the ratio when either compiler's output failed to match the baseline, so a
    row's presence already means "both sides were correct".
    """
    rows: list[tuple[float, str, int, list[str]]] = []
    for entry in report.get("benchmarks", []):
        stats = (entry.get("ratios_to_gcc") or {}).get("lccc")
        if not stats or not stats.get("median"):
            continue
        warnings = list(entry.get("warnings") or [])
        warnings.extend((entry.get("runtime", {}).get("lccc", {}) or {}).get("quality_warnings") or [])
        rows.append((float(stats["median"]), entry.get("name", "?"), int(stats.get("n") or 0), warnings))
    aggregate = report.get("aggregate_lccc_vs_gcc") or {}
    return rows, aggregate


def render(rows: list[tuple[float, str, int, list[str]]], aggregate: dict, top: int) -> str:
    lines = ["LCCC/GCC paired median wall-clock ratios (<1 = LCCC faster)"]
    if not rows:
        lines.append("no benchmark carries a paired ratio (both sides must be correct to have one)")
        return "\n".join(lines)
    gm = aggregate.get("geometric_mean_ratio")
    if gm is not None:
        worst = aggregate.get("worst") or {}
        best = aggregate.get("best") or {}
        lines.append(
            f"geomean {gm:.3f} over {aggregate.get('n_correct_benchmarks', len(rows))} kernels"
            f" | arithmetic {aggregate.get('arithmetic_mean_ratio', float('nan')):.3f}"
            f" | worst {worst.get('benchmark', '?')} {worst.get('ratio', float('nan')):.3f}"
            f" | best {best.get('benchmark', '?')} {best.get('ratio', float('nan')):.3f}"
        )
    else:
        lines.append(f"geomean unavailable; {len(rows)} kernels with a paired ratio")
    # Worst first: that is the order in which a performance change should look.
    ordered = sorted(rows, key=lambda r: (-r[0], r[1]))
    for ratio, name, rounds, warnings in ordered[:top]:
        suffix = f" [{len(warnings)} quality warning(s)]" if warnings else ""
        lines.append(f"{name} {ratio:.3f} n={rounds}{suffix}")
    if len(ordered) > top:
        lines.append(f"... {len(ordered) - top} more kernel(s) in the shard report")
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--report", required=True, help="shard report JSON written by ci-bench.py")
    parser.add_argument("--top", type=int, default=24, help="rows to publish (worst first)")
    parser.add_argument("--annotate", action="store_true", help="emit the workflow annotation")
    args = parser.parse_args()

    annotate = args.annotate or bool(os.environ.get("LCCC_BENCH_RATIOS"))

    path = Path(args.report)
    if not path.is_file():
        message = (f"no benchmark report at {path}: the timed corpus did not run, "
                   f"so there are no ratios to publish")
        print(message)
        if annotate:
            print(f"::warning title=bench-ratios::{gha_escape(message)}")
        return 0

    try:
        report = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError) as error:
        message = f"unreadable benchmark report {path}: {error}"
        print(message)
        if annotate:
            print(f"::warning title=bench-ratios::{gha_escape(message)}")
        return 0

    if report.get("schema") != SCHEMA:
        message = (f"{path}: schema {report.get('schema')!r}, expected {SCHEMA!r} "
                   f"(the report format changed; update this instrument)")
        print(message)
        if annotate:
            print(f"::warning title=bench-ratios::{gha_escape(message)}")
        return 0

    rows, aggregate = collect(report)
    table = render(rows, aggregate, args.top)
    print(table)
    if annotate:
        print(f"::warning title=bench-ratios::{gha_escape(table)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
