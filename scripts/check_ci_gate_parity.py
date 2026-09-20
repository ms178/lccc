#!/usr/bin/env python3
"""Fail when a standalone ci_local gate is absent from hosted workflows.

The local mirror and GitHub workflows intentionally use different orchestration,
but every directly invoked regression shell/Python program in ci_local.sh must
also occur in at least one workflow. This catches silent coverage drift without
requiring duplicate gate display names or running the full local driver in CI.
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
LOCAL = ROOT / "scripts" / "ci_local.sh"
WORKFLOWS = ROOT / ".github" / "workflows"

COMMAND = re.compile(
    r"(?:tests/regression/[A-Za-z0-9_./-]+\.sh|"
    r"scripts/[A-Za-z0-9_./-]+\.py|"
    r"\.github/scripts/[A-Za-z0-9_./-]+\.py)"
)


def main() -> int:
    local_paths = set(COMMAND.findall(LOCAL.read_text()))
    hosted = "\n".join(path.read_text() for path in sorted(WORKFLOWS.glob("*.yml")))
    missing = sorted(path for path in local_paths if path not in hosted)
    if missing:
        print("hosted CI is missing standalone ci_local gates:", file=sys.stderr)
        for path in missing:
            print(f"  {path}", file=sys.stderr)
        return 1
    print(f"CI/local standalone gate parity: PASS ({len(local_paths)} commands)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
