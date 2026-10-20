#!/usr/bin/env python3
"""Fail-closed, compiler-independent Local-Exec TLS mechanism oracle."""

import argparse
import re
from pathlib import Path


def accesses(text, symbol="tls_product"):
    start = list(re.finditer(r"^" + re.escape(symbol) + r":\s*(?:#.*)?$", text, re.M))
    if len(start) != 1:
        raise ValueError(f"expected exactly one {symbol} function, found {len(start)}")
    rest = text[start[0].end() :]
    end = re.search(r"^\s*\.size\s+" + re.escape(symbol) + r"\s*,", rest, re.M)
    if end is None:
        raise ValueError(f"missing .size for {symbol}")
    body = rest[: end.start()]
    return len(
        re.findall(
            r"^\s*(?:mov|add|imul|and|or|xor)[ql]\s+"
            r"%fs:volatile_reduction_slots@TPOFF\+8(?![0-9])",
            body,
            re.M,
        )
    )


def check(text, expected):
    actual = accesses(text)
    if actual != expected:
        raise ValueError(f"expected {expected} volatile TLS accesses, found {actual}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("assembly", type=Path)
    parser.add_argument("expected", type=int)
    args = parser.parse_args()
    try:
        check(args.assembly.read_text(), args.expected)
    except (OSError, ValueError) as error:
        parser.exit(1, f"{args.assembly}: {error}\n")


def ir_accesses(text, symbol):
    """Inspect the actual pass output, before TLS/backend lowering changes it."""
    marker = "==== IR after iter=0 volatile_reduce ===="
    if text.count(marker) != 1:
        raise ValueError("missing or duplicate volatile_reduce checkpoint")
    stage = text.split(marker, 1)[1].split("==== IR", 1)[0]
    marker = re.compile(r"^--- function " + re.escape(symbol) + r" \(.*\) ---$", re.M)
    starts = list(marker.finditer(stage))
    if len(starts) != 1:
        raise ValueError(f"missing or duplicate IR function {symbol}")
    body = stage[starts[0].end() :].split("--- function ", 1)[0]
    return len(re.findall(r"^  Load \{[^\n]*volatile: true", body, re.M))


if __name__ == "__main__":
    main()
