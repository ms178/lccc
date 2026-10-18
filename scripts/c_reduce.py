#!/usr/bin/env python3
"""Delta-debugging reducer for C miscompiles and compiler crashes (T4).

Shrinks a C file while an *interestingness* command keeps accepting it.  The
command receives the candidate path as its last argument and must exit 0 iff
the candidate still exhibits the failure.  The reducer itself knows nothing
about C; soundness lives entirely in the predicate, which is why
``scripts/c_reduce_predicate.py`` implements the two-oracle check that stops a
reduction from turning a real bug into undefined behaviour.

Algorithm (classic ddmin, Zeller & Hildebrandt 2002, followed by a
one-at-a-time fixpoint):

1. Split the lines into ``n`` chunks; try each complement.  On success keep the
   smaller input and restart with ``n-1`` chunks, otherwise double ``n``.
2. Repeat single-line deletion until no single line can be removed.  This is
   the 1-minimal guarantee: removing any remaining line makes the predicate
   fail.

Every candidate is memoised by content hash, so the cost is bounded by the
number of *distinct* candidates, not by the number of visits.

Usage::

    scripts/c_reduce.py case.c --out min.c -- \\
        python3 scripts/c_reduce_predicate.py --ccc target/fastbuild/lccc --level=-O2

The predicate's exit status is a verdict only for 0 (interesting) and 1 (not
interesting).  Any other status is a predicate error (usage mistake, crash of
the test harness) and aborts the reduction; treating it as a verdict would
silently turn a broken predicate into "nothing is interesting".

Exit status: 0 if the input was interesting and a reduction was written,
2 if the input itself is not interesting (nothing to reduce), 1 on error.
"""
from __future__ import annotations

import argparse
import hashlib
import os
import subprocess
import sys
import tempfile
from collections.abc import Callable, Sequence
from pathlib import Path


class PredicateError(Exception):
    """The predicate exited with a status that is not a verdict."""


class Budget(Exception):
    """Raised when the candidate budget is exhausted; the best result so far is kept."""


class Reducer:
    def __init__(self, check: Callable[[Path], bool], scratch: Path,
                 max_tests: int, start: Sequence[str]) -> None:
        self._check = check
        self._scratch = scratch
        self._max_tests = max_tests
        self._memo: dict[str, bool] = {}
        self.tests = 0  # predicate invocations (memo misses)
        # Smallest interesting candidate accepted so far, by character count.
        # Kept so a budget stop returns real progress, not the original input.
        self.best: list[str] = list(start)

    def interesting(self, lines: Sequence[str]) -> bool:
        text = "".join(lines)
        key = hashlib.sha256(text.encode("utf-8", "surrogateescape")).hexdigest()
        hit = self._memo.get(key)
        if hit is not None:
            return hit
        if self.tests >= self._max_tests:
            raise Budget()
        self.tests += 1
        path = self._scratch / f"cand-{self.tests % 2}.c"
        path.write_text(text, encoding="utf-8", errors="surrogateescape")
        verdict = self._check(path)
        self._memo[key] = verdict
        if verdict and len(text) < len("".join(self.best)):
            self.best = list(lines)
        return verdict

    def ddmin(self, lines: list[str]) -> list[str]:
        n = 2
        while len(lines) >= 2:
            chunk = max(1, len(lines) // n)
            chunks = [lines[i:i + chunk] for i in range(0, len(lines), chunk)]
            reduced = False
            for i in range(len(chunks)):
                complement = [ln for j, c in enumerate(chunks) if j != i for ln in c]
                if complement and self.interesting(complement):
                    lines = complement
                    n = max(n - 1, 2)
                    reduced = True
                    break
            if not reduced:
                if n >= len(lines):
                    break
                n = min(len(lines), 2 * n)
        return lines

    def one_minimal(self, lines: list[str]) -> list[str]:
        changed = True
        while changed:
            changed = False
            i = 0
            while i < len(lines):
                candidate = lines[:i] + lines[i + 1:]
                if candidate and self.interesting(candidate):
                    lines = candidate
                    changed = True
                else:
                    i += 1
        return lines


def reduce_text(text: str, check: Callable[[Path], bool], max_tests: int,
                scratch: Path) -> tuple[str, int, bool]:
    """Return (reduced text, predicate calls, budget_exhausted).

    The input must already be interesting; the caller checks that and this
    function trusts it.  On budget exhaustion the smallest interesting text
    accepted so far is returned.
    """
    lines = text.splitlines(keepends=True)
    red = Reducer(check, scratch, max_tests, lines)
    exhausted = False
    try:
        red.one_minimal(red.ddmin(lines))
    except Budget:
        exhausted = True
    return "".join(red.best), red.tests, exhausted


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("input", type=Path)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--max-tests", type=int, default=5000)
    ap.add_argument("--timeout", type=float, default=120.0,
                    help="seconds per predicate invocation")
    # Everything after the first '--' is the predicate command. It is split
    # off before argparse runs because argparse.REMAINDER mishandles '--'.
    raw = list(sys.argv[1:] if argv is None else argv)
    pred: list[str] = []
    if "--" in raw:
        cut = raw.index("--")
        raw, pred = raw[:cut], raw[cut + 1:]
    args = ap.parse_args(raw)
    if not pred:
        ap.error("missing predicate command after '--'")

    def check(path: Path) -> bool:
        try:
            proc = subprocess.run([*pred, str(path)], timeout=args.timeout,
                                  stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        except subprocess.TimeoutExpired:
            return False
        if proc.returncode in (0, 1):
            return proc.returncode == 0
        raise PredicateError(
            f"predicate exited {proc.returncode} on {path}; expected 0 or 1")

    text = args.input.read_text(encoding="utf-8", errors="surrogateescape")
    try:
        with tempfile.TemporaryDirectory(prefix="c-reduce-") as tmp:
            scratch = Path(tmp)
            if not check(args.input):
                print(f"c_reduce: input {args.input} is not interesting", file=sys.stderr)
                return 2
            reduced, calls, exhausted = reduce_text(text, check, args.max_tests, scratch)
    except PredicateError as exc:
        print(f"c_reduce: {exc}", file=sys.stderr)
        return 1
    args.out.parent.mkdir(parents=True, exist_ok=True)
    tmp_out = args.out.with_suffix(args.out.suffix + f".tmp{os.getpid()}")
    tmp_out.write_text(reduced, encoding="utf-8", errors="surrogateescape")
    os.replace(tmp_out, args.out)
    status = "budget exhausted (not 1-minimal)" if exhausted else "1-minimal"
    print(f"c_reduce: {len(text.splitlines())} -> {len(reduced.splitlines())} lines, "
          f"{calls} predicate calls, {status}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
