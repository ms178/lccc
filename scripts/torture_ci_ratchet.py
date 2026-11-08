#!/usr/bin/env python3
"""Fail closed on changed outcomes in an identity-matched torture campaign.

Baseline inventories must be copied from complete validated hosted reports,
with every non-pass identity (including skips) and no omitted rows. Any status
change, including a fix, requires explicit baseline review. Reference failures
are never allowed. Identity drift is an error, not permission to fall back to an
advisory success. This is NOT hermetic: host system headers, binutils, CRT,
shared libraries and kernel are not fully pinned by the comparison metadata.
"""
from __future__ import annotations

from collections import Counter
import json
import os
from pathlib import Path
import sys

from torture_ci_provenance import outcome_evidence, provenance
from torture_ci_summary import validated_report
from torture_evidence import STATUSES
from tools.corpus import schema


def compare(data: dict, baseline: object) -> list[str]:
    if not isinstance(baseline, dict) or type(baseline.get("schema")) is not int or baseline["schema"] != 1:
        raise ValueError("unsupported baseline schema")
    if set(baseline) != {"schema", "source_run", "source_head", "comparison", "counts", "outcomes"}:
        raise ValueError("missing or unknown baseline fields")
    if type(baseline["source_run"]) is not int or baseline["source_run"] <= 0:
        raise ValueError("baseline source run required")
    if not isinstance(baseline["source_head"], str) or not schema.SHA1.fullmatch(baseline["source_head"]):
        raise ValueError("baseline source head required")
    comparison = baseline["comparison"]
    current = provenance(data)["comparison"]
    if not isinstance(comparison, dict):
        raise ValueError("baseline comparison identity required")
    if comparison != current:
        changed = sorted(key for key in set(comparison) | set(current)
                         if comparison.get(key) != current.get(key) or
                         (key in comparison) != (key in current))
        return ["identity mismatch: " + ", ".join(changed)]
    rows = baseline["outcomes"]
    counts = baseline["counts"]
    if not isinstance(rows, list) or not isinstance(counts, dict):
        raise ValueError("baseline outcome rows and counts required")
    expected: dict[tuple[str, str], tuple[str, str, int, str]] = {}
    for row in rows:
        if not (isinstance(row, list) and len(row) == 6 and
                all(isinstance(value, str) and value for value in row[:4]) and
                type(row[4]) is int and isinstance(row[5], str) and schema.SHA256.fullmatch(row[5])):
            raise ValueError("invalid baseline outcome row")
        test, flags, status, phase, returncode, detail_sha256 = row
        if flags not in current["flags"] or status not in STATUSES - {"pass", "reference-fail"}:
            raise ValueError("invalid baseline flags or status (reference failures are never allowed)")
        if (test, flags) in expected:
            raise ValueError("duplicate baseline case")
        expected[test, flags] = (status, phase, returncode, detail_sha256)
    if any(not isinstance(key, str) or key not in STATUSES or type(value) is not int or value < 0
           for key, value in counts.items()):
        raise ValueError("invalid baseline counts")
    observed_counts = Counter(value[0] for value in expected.values())
    observed_counts["pass"] = current["expected_cases"] - len(expected)
    if observed_counts["pass"] <= 0 or any(counts.get(k, 0) != observed_counts.get(k, 0)
                                           for k in STATUSES):
        raise ValueError("baseline counts and outcome rows disagree")
    all_keys = {(row["test"], row["flags"]) for row in data["results"]}
    actual = {(row[0], row[1]): tuple(row[2:]) for row in outcome_evidence(data)}
    problems = []
    for key in sorted(set(expected) | set(actual)):
        before, after = expected.get(key), actual.get(key)
        if before != after:
            before_label = before[0] if before else "pass"
            after_label = after[0] if after else ("pass" if key in all_keys else "missing")
            suffix = " (phase, exit code, or diagnostic changed)" if before and after and before[0] == after[0] else ""
            problems.append(f"{json.dumps(key, ensure_ascii=True)}: {before_label} -> {after_label}{suffix}")
    return problems


def error_annotation(problems: list[str]) -> str:
    """Compact, escaped failure summary; full detail remains in job logs."""
    def encode(count: int) -> str:
        payload = json.dumps({"problems": problems[:count], "omitted": len(problems) - count},
                             ensure_ascii=True, separators=(",", ":"))
        return payload.replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")
    low, high = 0, len(problems)
    while low < high:
        middle = (low + high + 1) // 2
        if len(encode(middle).encode("utf-8")) <= 3800:
            low = middle
        else:
            high = middle - 1
    return encode(low)


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print("usage: torture_ci_ratchet.py REPORT BASELINE", file=sys.stderr)
        return 2
    try:
        data = validated_report(Path(argv[0]))
        baseline = schema.loads(Path(argv[1]).read_text(encoding="utf-8"))
        problems = compare(data, baseline)
    except (OSError, ValueError, schema.SchemaError) as exc:
        message = f"invalid torture ratchet input: {exc}"
        print(message, file=sys.stderr)
        if os.environ.get("GITHUB_ACTIONS"):
            print(f"::error title=GCC torture ratchet::{error_annotation([message])}")
        return 2
    if problems:
        if os.environ.get("GITHUB_ACTIONS"):
            print(f"::error title=GCC torture ratchet::{error_annotation(problems)}")
        print("torture ratchet: comparison refused or outcomes changed:", file=sys.stderr)
        for problem in problems:
            print("  " + problem, file=sys.stderr)
        return 1
    print(f"torture ratchet: {data['arch']}/{data['mode']} matches reviewed per-case baseline")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
