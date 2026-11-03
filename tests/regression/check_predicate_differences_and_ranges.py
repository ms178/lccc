#!/usr/bin/env python3
"""Predicate differences and ranges: three rewrites, one gate.

What this pins, and how:

* **values** -- the witness is self-checking at ``-O0`` through ``-O3``, with
  every interval boundary and the wrap points of each subtraction in the probe
  set.  A wrong fold fails the run; it cannot hide behind an instruction count.
* **mechanism** -- at ``-O2`` the folded bodies must *lose the thing the fold
  removes*: a difference compare no longer contains a subtraction at all, an
  interval test contains exactly one compare, and a sign-magnitude select
  contains no compare: the x86-64 peephole `abs_cmov` spells the pass's
  arithmetic-shift identity as GCC's `mov; neg; cmovs` (the `cmovs` reads the
  negation's own flags), and with ``CCC_PEEPHOLE_SKIP=abs_cmov`` the identity
  itself -- the shift mask and its XOR, no conditional move -- must be there.
  Counting alone is not enough for any of them.
* **controls** -- `diff_control` (constant 5, not 0), `member_gap` ({0, 2},
  not consecutive), `two_values` (two different values) and `abs_control` (arms
  disagreeing on the value) are one token away from a folded shape and must
  keep their original shape.  `range_crossing_control`, `(x > -8) | (x < 8)`,
  was listed with them as "an outside test", but its two halves cover every
  int: it is the constant 1, which GCC 16.2 and Clang 23.1.4 return and the
  comparison-dominance fold now returns too.  It must therefore contain no
  comparison (or be gone, once interprocedural propagation has used the
  constant), and `CCC_NO_COMPARE_DOMINANCE=1` must bring both comparisons back.
  `range_union` (`(x > 3) | (x > 10)`) is pinned in two pieces instead, because
  two rules meet on it: it is NOT one interval, so `interval` must not merge it
  into a window, but it IS a disjunction whose weaker disjunct decides it, so
  the comparison-dominance fold reduces it to the bare `x > 3`.  With
  `CCC_NO_COMPARE_DOMINANCE=1` the second comparison must come back.
* **attribution** -- with ``CCC_DISABLE_PASSES=diffcmp,interval,signmag`` the
  same functions must get longer, so each win belongs to its pass.
* **oracle** -- with GCC 16.2 present, every folded function must be no longer
  than GCC's.  `abs_select` and `abs_select64` used to be reported instead of
  failed (GCC folds the negation's flags into the conditional move); since
  `abs_cmov` does the same they are held to the oracle like the rest.
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
    "range_crossing",
    "range_crossing_bitand",
)
MAGNITUDE_FOLDED = (
    "abs_select",
    "abs_select64",
    "absdiff_select",
    # Narrow operands compared at their own width (GVN must wake the select
    # up once it has unified the two sign extensions; see passes/mod.rs).
    "abs_short",
    "abs_short_ret",
    "abs_schar",
    "abs_int_wide",
    "absdiff_short",
)
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


def body_if_present(text, name):
    """`body`, or None when `name` was removed (a static function whose
    constant result interprocedural propagation has already used)."""
    if not re.search(r"^" + re.escape(name) + r":\s*$", text, re.M):
        return None
    return body(text, name)


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

        identity_text = asm(
            lccc, SOURCE, env={**os.environ, "CCC_PEEPHOLE_SKIP": "abs_cmov"}
        )
        for name in MAGNITUDE_FOLDED:
            lines = body(text, name)
            if compares(lines):
                problems.append(f"{name}: the select's compare survived the fold: {lines}")
            # The final spelling: `neg` and the `cmovs` reading its flags,
            # adjacent, and no other conditional move.
            negs = [i for i, line in enumerate(lines) if re.match(r"^neg[lq]\b", line)]
            cmovs = [i for i, line in enumerate(lines) if line.startswith("cmov")]
            if len(negs) != 1 or cmovs != [negs[0] + 1] or not lines[cmovs[0]].startswith("cmovs "):
                problems.append(f"{name}: want `neg` then the `cmovs` reading its flags: {lines}")
            identity = body(identity_text, name)
            joined = "\n".join(identity)
            if not re.search(r"\bsar[a-z]*\b", joined):
                problems.append(f"{name}: no arithmetic shift mask under abs_cmov skip: {identity}")
            if not re.search(r"\bxor[a-z]*\b", joined):
                problems.append(f"{name}: no sign-mask XOR under abs_cmov skip: {identity}")
            if "cmov" in joined:
                problems.append(f"{name}: conditional move survived the fold: {identity}")

        # ---- controls: one token away, must not fold ----------------------
        if not has_subtraction(body(text, "diff_control")):
            problems.append("diff_control folded although its constant is 5, not 0")
        for name in ("member_gap", "two_values"):
            if len(compares(body(text, name))) < 2:
                problems.append(
                    f"{name} lost a comparison although it is not one interval: "
                    f"{body(text, name)}"
                )
        # `range_union`: not one interval for `interval`, but a disjunction the
        # dominance fold owns.  Both halves are pinned so neither rule can take
        # the shape by accident and neither can regress behind the other.
        union = body(text, "range_union")
        if len(compares(union)) != 1:
            problems.append(
                f"range_union: {len(compares(union))} compares, want the "
                f"dominance fold's single `x > 3`: {union}"
            )
        union_off = body(
            asm(lccc, SOURCE, env={**os.environ, "CCC_NO_COMPARE_DOMINANCE": "1"}),
            "range_union",
        )
        if len(compares(union_off)) < 2:
            problems.append(
                f"range_union: CCC_NO_COMPARE_DOMINANCE=1 does not restore the "
                f"second comparison: {union_off}"
            )

        # ---- the tautology: two halves that cover every int ---------------
        cover = body_if_present(text, "range_crossing_control")
        if cover is not None and compares(cover):
            problems.append(
                f"range_crossing_control: still compares although (x > -8) | "
                f"(x < 8) holds for every int: {cover}"
            )
        cover_off = body(
            asm(lccc, SOURCE, env={**os.environ, "CCC_NO_COMPARE_DOMINANCE": "1"}),
            "range_crossing_control",
        )
        if len(compares(cover_off)) < 2:
            problems.append(
                f"range_crossing_control: CCC_NO_COMPARE_DOMINANCE=1 does not "
                f"restore both comparisons: {cover_off}"
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
            "range_crossing",
            "range_crossing_bitand",
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
            # Measured gaps, each pinned to lccc's exact count so it cannot
            # silently grow.  `abs_short_ret`: lccc keeps narrow values
            # sign-extended in registers, so the 4-instruction int core sits
            # between the parameter's `movswl` and the return conversion's
            # `movswl`; GCC and Clang compute at 16 bits (`negw; cmovs`).
            # `abs_int_wide`: GCC and Clang compute |x| at 32 bits and let the
            # 32-bit `neg`/`cmovs` zero the upper half (|INT_MIN| = 2^31 fits
            # in 32 unsigned bits); lccc negates the `movslq`-widened value.
            exempt = {
                "abs_short_ret": (6, "narrow values are kept sign-extended"),
                "abs_int_wide": (5, "64-bit negate of the sign-extended operand"),
            }
            for name in DIFF_FOLDED + RANGE_FOLDED + MAGNITUDE_FOLDED:
                ours, theirs = len(body(text, name)), len(body(gtext, name))
                if name in exempt and ours > exempt[name][0]:
                    problems.append(
                        f"{name}: {ours} instructions, more than the pinned "
                        f"{exempt[name][0]} ({exempt[name][1]})"
                    )
                elif ours > theirs:
                    if name in exempt:
                        print(
                            f"note: {name}: {ours} vs GCC {theirs} instructions "
                            f"({exempt[name][1]})"
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
        "membership tests, sign-magnitude selects, four controls, one tautology, "
        "disabled-pass attribution"
    )


if __name__ == "__main__":
    main()
