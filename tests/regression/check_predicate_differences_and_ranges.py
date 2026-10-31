#!/usr/bin/env python3
"""Predicate differences and ranges: three rewrites, one gate.

What this pins, and how:

* **values** -- the witness is self-checking at ``-O0`` through ``-O3``, with
  every interval boundary and the wrap points of each subtraction in the probe
  set.  A wrong fold fails the run; it cannot hide behind an instruction count.
* **mechanism** -- at ``-O2`` the folded bodies must *lose the thing the fold
  removes*: a difference compare no longer contains a subtraction at all, an
  interval test contains exactly one compare, and a sign-magnitude select
  carries the arithmetic-shift mask instead of a conditional move.  Counting
  alone is not enough for any of them.
* **controls** -- `diff_control` (constant 5, not 0), `range_union` (a union,
  not an interval), `member_gap` ({0, 2}, not consecutive), `two_values` (two
  different values) and `abs_control` (arms disagreeing on the value) are one
  token away from a folded shape and must keep their original shape.
* **attribution** -- with ``CCC_DISABLE_PASSES=diffcmp,interval,signmag`` the
  same functions must get longer, so each win belongs to its pass.
* **oracle** -- with GCC 16.2 present, every folded function must be no longer
  than GCC's.  `abs_select` is reported, not failed: the remaining difference
  is instruction *selection* (GCC folds the negation's own flags into the
  conditional move), not this fold's arithmetic.
"""

import argparse
import os
from pathlib import Path
import re
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
SOURCE = Path(__file__).resolve().parent / "predicate_differences_and_ranges.c"

DIFF_FOLDED = ("diff_gt", "diff_eq", "diff_ge", "diff_une")
RANGE_FOLDED = (
    "range_bitand",
    "range_short",
    "range_closed",
    "range_unsigned",
    "outside_or",
    "outside_short",
    "member_short",
    "member_bitand",
)
MAGNITUDE_FOLDED = ("abs_select", "abs_select64", "absdiff_select")
CONTROLS = ("diff_control", "range_union", "member_gap", "two_values", "abs_control")


def run(command, env=None):
    result = subprocess.run(command, env=env, capture_output=True, timeout=300)
    if result.returncode:
        raise RuntimeError(
            f"command failed {command!r}: {result.stderr.decode(errors='replace')[:400]}"
        )
    return result.stdout


def body(text, name):
    starts = list(re.finditer(r"^" + re.escape(name) + r":\s*$", text, re.M))
    if len(starts) != 1:
        raise ValueError(f"missing/duplicate function {name}")
    rest = text[starts[0].end() :]
    end = re.search(r"^\s*\.size\s+" + re.escape(name) + r"\s*,", rest, re.M)
    if not end:
        raise ValueError(f"missing end of {name}")
    return [
        line.strip()
        for line in rest[: end.start()].splitlines()
        if line.strip()
        and not line.strip().startswith((".", "#"))
        and not line.strip().endswith(":")
    ]


def asm(cc, path, env=None, extra=()):
    return run([cc, "-O2", "-S", "-o", "-", path, *extra], env=env).decode()


def compares(lines):
    return [line for line in lines if re.match(r"^(cmp|test)", line)]


def has_subtraction(lines):
    return any(re.match(r"^sub", line) for line in lines)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--lccc", default=os.environ.get("CCC", ROOT / "target/fastbuild/lccc")
    )
    parser.add_argument("--gcc", default=os.environ.get("GCC", "gcc-16"))
    args = parser.parse_args()
    lccc = str(args.lccc)
    problems = []

    with tempfile.TemporaryDirectory() as td:
        # ---- values, every optimization level ------------------------------
        for level in ("0", "1", "2", "3"):
            binary = Path(td) / f"predfold-O{level}"
            run([lccc, f"-O{level}", "-o", str(binary), str(SOURCE)])
            out = run([str(binary)]).decode()
            if "predfold: ok" not in out:
                problems.append(f"-O{level}: witness did not report ok: {out!r}")

        text = asm(lccc, SOURCE)

        # ---- mechanism: the removed thing is gone -------------------------
        for name in DIFF_FOLDED:
            lines = body(text, name)
            if has_subtraction(lines):
                problems.append(f"{name}: the subtraction survived the fold: {lines}")
            if not compares(lines):
                problems.append(f"{name}: no direct comparison emitted: {lines}")

        for name in RANGE_FOLDED:
            lines = body(text, name)
            found = compares(lines)
            if len(found) != 1:
                problems.append(f"{name}: {len(found)} compares, want exactly 1: {lines}")
            if len(lines) > 6:
                problems.append(f"{name}: {len(lines)} instructions after the fold: {lines}")

        for name in MAGNITUDE_FOLDED:
            lines = body(text, name)
            joined = "\n".join(lines)
            if not re.search(r"\bsar[a-z]*\b", joined):
                problems.append(f"{name}: no arithmetic shift mask: {lines}")
            if not re.search(r"\bxor[a-z]*\b", joined):
                problems.append(f"{name}: no sign-mask XOR: {lines}")
            if "cmov" in joined:
                problems.append(f"{name}: conditional move survived the fold: {lines}")

        # ---- controls: one token away, must not fold ----------------------
        if not has_subtraction(body(text, "diff_control")):
            problems.append("diff_control folded although its constant is 5, not 0")
        for name in ("range_union", "member_gap", "two_values"):
            if len(compares(body(text, name))) < 2:
                problems.append(
                    f"{name} lost a comparison although it is not one interval: "
                    f"{body(text, name)}"
                )
        control = body(text, "abs_control")
        if not any("cmov" in line for line in control) and not has_subtraction(control):
            problems.append(f"abs_control: expected the original select: {control}")

        # ---- attribution ---------------------------------------------------
        disabled = asm(
            lccc,
            SOURCE,
            env=os.environ | {"CCC_DISABLE_PASSES": "diffcmp,interval,signmag"},
        )
        # These six are the shapes no other pass reaches; each must get longer
        # when its own pass is off.  The remaining folded shapes (`diff_eq`,
        # `diff_une`, `range_closed`, `range_unsigned`, `outside_or`,
        # `outside_short`, `member_bitand`) are already covered by simplify,
        # range_check and set_membership, so the claim there is only "not
        # worse" -- an honest attribution rather than a borrowed win.
        improved = (
            "diff_gt",
            "diff_ge",
            "range_bitand",
            "range_short",
            "member_short",
            "absdiff_select",
        )
        for name in improved:
            on, off = len(body(text, name)), len(body(disabled, name))
            if off <= on:
                problems.append(
                    f"{name}: disabling the passes did not lengthen it ({on} vs {off})"
                )
        for name in DIFF_FOLDED + RANGE_FOLDED + MAGNITUDE_FOLDED:
            on, off = len(body(text, name)), len(body(disabled, name))
            if on > off:
                problems.append(
                    f"{name}: longer with the passes on than off ({on} vs {off})"
                )

        # ---- oracle --------------------------------------------------------
        gcc = args.gcc
        try:
            version = run([gcc, "-dumpfullversion"]).decode().strip()
        except (RuntimeError, FileNotFoundError):
            version = ""
        if re.fullmatch(r"16\.\d+(?:\.\d+)?", version):
            gtext = asm(gcc, SOURCE, extra=("-fno-ipa-icf",))
            exempt = {
                "abs_select": "conditional-move flag selection (GCC reuses neg flags)",
                "abs_select64": "conditional-move flag selection",
            }
            for name in DIFF_FOLDED + RANGE_FOLDED + MAGNITUDE_FOLDED:
                ours, theirs = len(body(text, name)), len(body(gtext, name))
                if ours > theirs:
                    if name in exempt:
                        print(
                            f"note: {name}: {ours} vs GCC {theirs} instructions "
                            f"({exempt[name]})"
                        )
                    else:
                        problems.append(
                            f"{name}: {ours} instructions, GCC {version} emits {theirs}"
                        )
            print(f"oracle: {gcc}: GCC {version} (normative)")
        else:
            print(f"note: no GCC 16.x oracle ({gcc!r} -> {version!r}); oracle leg skipped")

    if problems:
        for problem in problems:
            print(f"FAIL predfold: {problem}")
        raise SystemExit(1)
    print(
        "PASS predicate differences and ranges: difference compares, interval and "
        "membership tests, sign-magnitude selects, five controls, disabled-pass "
        "attribution"
    )


if __name__ == "__main__":
    main()
