#!/usr/bin/env python3
"""Surface a failed gate's diagnostic lines as GitHub Actions annotations.

A shell gate reports *why* it failed on stdout/stderr; the Checks tab shows
only the step's exit status, and the job log is one more click away (and
unavailable to tooling that can reach ``api.github.com`` but not the log
storage behind it). This filter reads the gate's captured output and emits
``::error`` workflow commands for the lines that carry the verdict, so the
failure text is attached to the check run itself.

Selection: every line matching the verdict pattern (``FAIL``, ``error``,
``Error``, ``mismatch``, ``panicked``, ``Traceback``, ``AssertionError``),
APT refresh/resolver errors (``Err:``, ``E:``, failed-fetch warnings),
plus the following ``--context`` lines (defaults to 3) which usually hold
the ``cat``-ed compiler or program output. If nothing matches, the last
``--tail`` lines are used so a gate that dies without a verdict still
surfaces something. The selection is then packed into at most ``--max``
annotations of at most 4000 characters each (GitHub truncates longer
messages and stops after ten per step), newest material last.

Usage::

    python3 scripts/ci_annotate_log.py --title "ext-vector ABI" gate.log

Exit status is always 0: this is reporting, never a verdict.
"""
from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

VERDICT = re.compile(
    r"\bFAIL\b|\berror\b|\bError\b|mismatch|panicked|Traceback|AssertionError"
    r"|^Err:|^E:|^W:.*[Ff]ailed"
)
# Compiler warnings often quote "-Werror" or "make this an error". They
# must not displace the actual fatal diagnostic in a large parallel build.
WARNING_DIAGNOSTIC = re.compile(r"^(?:.*?:\s*)?warning:")
LIMIT = 4000


def select(lines: list[str], context: int, tail: int) -> list[str]:
    keep = [False] * len(lines)
    for i, line in enumerate(lines):
        if VERDICT.search(line) and not WARNING_DIAGNOSTIC.match(line):
            for j in range(i, min(len(lines), i + 1 + context)):
                keep[j] = True
    chosen = [line for line, k in zip(lines, keep) if k]
    if chosen:
        return chosen
    return lines[-tail:]


def escape(message: str) -> str:
    # Workflow-command data encoding: percent first, then the line breaks.
    return message.replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")


def pack(lines: list[str], limit: int) -> list[str]:
    chunks: list[str] = []
    current: list[str] = []
    size = 0
    for line in lines:
        line = line.rstrip("\n")
        if len(line) > limit:
            line = line[: limit - 3] + "..."
        if current and size + len(line) + 1 > limit:
            chunks.append("\n".join(current))
            current, size = [], 0
        current.append(line)
        size += len(line) + 1
    if current:
        chunks.append("\n".join(current))
    return chunks


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("log", type=Path)
    parser.add_argument("--title", default="Gate failed")
    parser.add_argument("--context", type=int, default=3)
    parser.add_argument("--tail", type=int, default=40)
    parser.add_argument("--max", type=int, default=8)
    args = parser.parse_args()
    try:
        text = args.log.read_text(encoding="utf-8", errors="replace")
    except OSError as exc:
        print(f"::error title={args.title}::log unavailable: {exc}")
        return 0
    lines = text.splitlines()
    if not lines:
        print(f"::error title={args.title}::the gate produced no output")
        return 0
    chunks = pack(select(lines, args.context, args.tail), LIMIT)
    if len(chunks) > args.max:
        # Keep the first cause AND the final summary. Parallel make can
        # print thousands of lines between a compiler error and "Error 2".
        chunks = chunks[:1] if args.max <= 1 else [chunks[0], *chunks[-(args.max - 1):]]
    total = len(chunks)
    for index, chunk in enumerate(chunks, 1):
        title = args.title if total == 1 else f"{args.title} [{index}/{total}]"
        print(f"::error title={escape(title)}::{escape(chunk)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
