#!/usr/bin/env python3
"""Summarise gcc.c-torture JSON reports into the session evidence table.

Input:  one or more ``--json`` reports written by ``scripts/x86_gcc_torture.py``
        or ``scripts/x86_gcc_torture_i686.py`` (schemas 1 and 2; schema 2
        requires a completed, non-fatal, cardinality-checked report).
Output: ``engineering/evidence/session/evidence.json`` (consumed by the
        harness web console via ``scripts/lccc-harness-snapshot.sh``) and a
        Markdown table on stdout for follow-up documents.

Reference skips (``reference-compile-skip`` / ``reference-run-skip``) are tests
the host GCC itself cannot build or run, and ``unsupported`` tests are the ones
gcc's own harness would not run either (``run_expensive_tests`` without
``GCC_TEST_RUN_EXPENSIVE``); all are excluded from the denominator because
they say nothing about LCCC. Everything else is either ``pass`` or a
validation failure, so ``pass + fail == total``. Reference tool/infrastructure
failures remain failures (never skips), but are separately counted as
``reference_fail``: an oracle ICE is not evidence of an LCCC defect.

Usage:
  scripts/torture_evidence.py x64.json:x86-64 i686.json:i686 [--out evidence.json]
"""
from __future__ import annotations

import argparse
import json
import os
import sys
import time
from collections import Counter, defaultdict
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
DEFAULT_OUT = REPO / "engineering" / "evidence" / "session" / "evidence.json"
SKIPS = {"reference-compile-skip", "reference-run-skip", "unsupported"}


def summarise(path: Path, target: str) -> tuple[list[dict], list[str]]:
    data = json.loads(path.read_text())
    if data.get("schema") == 2:
        if data.get("complete") is not True:
            raise ValueError(f"incomplete torture checkpoint: {path}")
        if data.get("fatal"):
            raise ValueError(f"fatal torture harness error: {data['fatal']}")
        if not (data.get("expected_cases") == data.get("completed_cases") == len(data["results"])):
            raise ValueError(f"torture report case count mismatch: {path}")
    per_flag: dict[str, Counter] = defaultdict(Counter)
    failing: dict[str, list[str]] = defaultdict(list)
    for r in data["results"]:
        per_flag[r["flags"]][r["status"]] += 1
        if r["status"] not in SKIPS and r["status"] != "pass":
            failing[r["test"]].append(f"{r['flags']}:{r['status']}")
    rows = []
    for flag in data["flags"]:
        c = per_flag[flag]
        skipped = sum(v for k, v in c.items() if k in SKIPS)
        fail = sum(v for k, v in c.items() if k not in SKIPS and k != "pass")
        rows.append(dict(
            suite="gcc.c-torture/" + data.get("mode", "execute"), target=target, flags=flag,
            pass_=c["pass"], fail=fail, total=c["pass"] + fail, skipped=skipped,
            reference_fail=c["reference-fail"],
            note=f"lccc {str(data.get('lccc_head') or '')[:12]}; "
                 f"reference tool/infrastructure failures: {c['reference-fail']}",
        ))
    selection = data.get("selection", {})
    partial = bool(data.get("from_list") or selection.get("tests") or selection.get("filter"))
    scope = "filtered selection" if partial else "recorded selection"
    notes = [f"{target} ({scope}): {len(failing)} distinct failing sources: " +
             ", ".join(f"{t} [{' '.join(v)}]" for t, v in sorted(failing.items()))
             if failing else f"{target} ({scope}): zero reported failures across {', '.join(data['flags'])}"]
    return rows, notes


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("reports", nargs="+", help="<report.json>:<target-label>")
    ap.add_argument("--out", type=Path, default=DEFAULT_OUT)
    ap.add_argument("--append-note", action="append", default=[])
    args = ap.parse_args(argv)
    rows, notes = [], []
    for item in args.reports:
        path, _, label = item.partition(":")
        try:
            r, n = summarise(Path(path), label or Path(path).stem)
        except (OSError, ValueError, KeyError) as exc:
            ap.exit(2, f"cannot summarise {path}: {exc}\n")
        rows += r
        notes += n
    notes += args.append_note
    payload = {"generated": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
               "rows": [{("pass" if k == "pass_" else k): v for k, v in row.items()} for row in rows],
               "notes": notes}
    args.out.parent.mkdir(parents=True, exist_ok=True)
    tmp = args.out.with_suffix(".json.tmp")
    tmp.write_text(json.dumps(payload, indent=2) + "\n")
    os.replace(tmp, args.out)
    print("| Target | Flags | Pass | Validation fail | Of which reference fail | Rate |")
    print("|---|---|---:|---:|---:|---:|")
    for row in payload["rows"]:
        rate = 100.0 * row["pass"] / row["total"] if row["total"] else 0.0
        print(f"| {row['target']} | `{row['flags']}` | {row['pass']}/{row['total']} | {row['fail']} | {row['reference_fail']} | {rate:.2f}% |")
    for n in notes:
        print(f"\n{n}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
