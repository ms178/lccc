#!/usr/bin/env python3
"""Publish a compact, validated GCC torture inventory as a GitHub annotation.

The original reports and failure logs remain the detailed artifacts. This
summary separates candidate and oracle outcomes and is useful even when an
artifact download is inaccessible. It is not a baseline or a regression gate.
Incomplete, malformed and legacy reports cannot become completion claims.
"""
from __future__ import annotations

import json
from pathlib import Path
import sys

from torture_evidence import CURRENT_SCHEMA, validate
from tools.corpus import schema


def inventory(path: Path) -> dict:
    data = schema.loads(path.read_text(encoding="utf-8"))
    errors = validate(data)
    if not isinstance(data, dict) or data.get("schema") != CURRENT_SCHEMA:
        errors.append("current schema required")
    if errors:
        raise ValueError("; ".join(errors))
    return {
        "arch": data.get("arch"), "mode": data["mode"],
        "corpus": data.get("gcc_checkout_head"), "flags": data["flags"],
        "case_set_sha256": data["case_set_sha256"],
        "counts": data.get("counts"),
        "nonpass": sorted([row["test"], row["flags"], row["status"]]
                          for row in data["results"] if row["status"] != "pass"),
    }


def main(argv: list[str]) -> int:
    if not argv:
        print("usage: torture_ci_summary.py REPORT...", file=sys.stderr)
        return 2
    for name in argv:
        path = Path(name)
        if not path.is_file():
            print(f"No completed report: {path}", file=sys.stderr)
            return 2
        try:
            result = inventory(path)
        except (OSError, ValueError, schema.SchemaError) as exc:
            print(f"Invalid report {path}: {exc}", file=sys.stderr)
            return 2
        payload = json.dumps(result, separators=(",", ":"), ensure_ascii=True)
        # Escape workflow-command metacharacters rather than letting a case
        # name or future metadata inject another annotation or break a line.
        escaped = payload.replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")
        print(f"::notice title=GCC torture inventory::{escaped}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
