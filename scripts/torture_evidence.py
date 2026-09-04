#!/usr/bin/env python3
"""Summarise gcc.c-torture JSON reports into the session evidence table.

Input:  one or more ``--json`` reports written by ``scripts/x86_gcc_torture.py``
        or ``scripts/x86_gcc_torture_i686.py`` (schema 1).
Output: ``engineering/evidence/session/evidence.json`` (consumed by the
        harness web console via ``scripts/lccc-harness-snapshot.sh``) and a
        Markdown table on stdout for follow-up documents.

Reference skips (``reference-compile-skip`` / ``reference-run-skip``) are tests
the host GCC itself cannot build or run; they are excluded from the
denominator because they say nothing about LCCC.  Everything else is either
``pass`` or an LCCC failure, so ``pass + fail == total``.

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
SKIPS = {"reference-compile-skip", "reference-run-skip"}


def summarise(path: Path, target: str) -> tuple[list[dict], list[str]]:
    data = json.loads(path.read_text())
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
            suite="gcc.c-torture/execute", target=target, flags=flag,
            pass_=c["pass"], fail=fail, total=c["pass"] + fail, skipped=skipped,
            note=f"lccc {str(data.get('lccc_head') or '')[:12]}",
        ))
    notes = [f"{target}: {len(failing)} distinct failing sources: " +
             ", ".join(f"{t} [{' '.join(v)}]" for t, v in sorted(failing.items()))
             if failing else f"{target}: zero LCCC failures across {', '.join(data['flags'])}"]
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
        r, n = summarise(Path(path), label or Path(path).stem)
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
    print("| Target | Flags | Pass | Fail | Rate |")
    print("|---|---|---:|---:|---:|")
    for row in payload["rows"]:
        rate = 100.0 * row["pass"] / row["total"] if row["total"] else 0.0
        print(f"| {row['target']} | `{row['flags']}` | {row['pass']}/{row['total']} | {row['fail']} | {rate:.2f}% |")
    for n in notes:
        print(f"\n{n}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
