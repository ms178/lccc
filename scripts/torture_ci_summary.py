#!/usr/bin/env python3
"""Publish a compact, validated GCC torture inventory as a GitHub annotation.

The original reports and failure logs remain the detailed artifacts. This
summary separates candidate and oracle outcomes and is useful even when an
artifact download is inaccessible. It is not a baseline or a regression gate.
Incomplete, malformed and legacy reports cannot become completion claims.
Long inventories explicitly count omitted annotation entries; the full report
remains the artifact and must be consulted for the complete list.
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
    else:
        if data.get("arch") not in ("x86_64", "i686"):
            errors.append("x86_64 or i686 architecture required")
        corpus = data.get("gcc_checkout_head")
        if not isinstance(corpus, str) or not schema.SHA1.fullmatch(corpus):
            errors.append("full GCC corpus commit required")
        if not isinstance(data.get("counts"), dict):
            errors.append("counts required")
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


def annotation_payload(result: dict) -> str:
    """Bound check-run annotation size and report any omitted identities."""
    nonpass = result["nonpass"]

    def encode(count: int) -> str:
        bounded = {**result, "nonpass": nonpass[:count],
                   "nonpass_omitted": len(nonpass) - count}
        return (json.dumps(bounded, separators=(",", ":"), ensure_ascii=True)
                .replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A"))

    if len(encode(0).encode("utf-8")) > 3800:
        raise ValueError("inventory annotation metadata exceeds safe limit")
    low, high = 0, len(nonpass)
    while low < high:
        middle = (low + high + 1) // 2
        if len(encode(middle).encode("utf-8")) <= 3800:
            low = middle
        else:
            high = middle - 1
    return encode(low)


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
        try:
            payload = annotation_payload(result)
        except ValueError as exc:
            print(f"Cannot publish inventory {path}: {exc}", file=sys.stderr)
            return 2
        print(f"::notice title=GCC torture inventory::{payload}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
