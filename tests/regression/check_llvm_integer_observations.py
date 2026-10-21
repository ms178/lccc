#!/usr/bin/env python3
"""Demand-observation batch: runtime, per-rule mechanism and exact policy controls."""

import argparse
import json
import os
from pathlib import Path
import re
import tempfile
from check_llvm_missed_zero_choice import run, body

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "tests/regression/llvm_integer_observations.c"
CASES = json.loads(SOURCE.with_suffix(".json").read_text())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--compiler", default=os.environ.get("CCC", str(ROOT / "target/fastbuild/lccc"))
    )
    parser.add_argument("--gcc", default=os.environ.get("GCC", "gcc-16"))
    parser.add_argument("--clang", help="optional exact Clang 23.1.3 comparison")
    parser.add_argument(
        "--baseline",
        help="optional saved pre-batch compiler, never inferred from disabled output",
    )
    args = parser.parse_args()
    if len(CASES) != 45 or len({c["name"] for c in CASES}) != 45:
        raise ValueError("missing or duplicate witness contracts")
    if not re.search(r"\b16\.2(?:\.0)?\b", run([args.gcc, "--version"]).decode()):
        raise ValueError("requires exact GCC 16.2")
    if args.clang and not re.search(
        r"\b23\.1\.3\b", run([args.clang, "--version"]).decode()
    ):
        raise ValueError("requires exact Clang 23.1.3")
    env = os.environ | {"CCC_VERIFY_IR": "abort"}
    with tempfile.TemporaryDirectory() as directory:
        p = Path(directory)
        run(
            [
                args.gcc,
                "-O3",
                "-Wall",
                "-Wextra",
                "-Werror",
                str(SOURCE),
                "-o",
                str(p / "gcc"),
            ]
        )
        expected = run([str(p / "gcc")])
        if args.clang:
            run(
                [
                    args.clang,
                    "-O3",
                    "-Wall",
                    "-Wextra",
                    "-Werror",
                    str(SOURCE),
                    "-o",
                    str(p / "clang"),
                ]
            )
            if run([str(p / "clang")]) != expected:
                raise ValueError("Clang/GCC runtime disagreement")
        for flags in [[x] for x in ("-O0", "-O1", "-O2", "-O3", "-Os")] + [
            ["-O3", "-g"]
        ]:
            run(
                [
                    args.compiler,
                    *flags,
                    "-Wall",
                    "-Wextra",
                    "-Werror",
                    str(SOURCE),
                    "-o",
                    str(p / "lccc"),
                ],
                env=env,
            )
            if run([str(p / "lccc")]) != expected:
                raise ValueError(f"native mismatch {flags}")
        for opt in ("-O2", "-O3"):
            run([args.compiler, opt, "-S", str(SOURCE), "-o", str(p / "on.s")], env=env)
            run(
                [args.compiler, opt, "-S", str(SOURCE), "-o", str(p / "off.s")],
                env=env | {"CCC_DISABLE_PASSES": "bit_idioms"},
            )
            on, off = (p / "on.s").read_text(), (p / "off.s").read_text()
            old = None
            if args.baseline:
                run([args.baseline, opt, "-S", str(SOURCE), "-o", str(p / "old.s")])
                old = (p / "old.s").read_text()
            for case in CASES:
                name = case["name"]
                yes, no = body(on, name), body(off, name)
                # High-unsigned threshold currently materializes an immediate:
                # same instruction count, but removes the dependent shift.
                if name == "lshr_full":
                    if len(yes) > len(no) or any(re.match(r"shr", x) for x in yes):
                        raise ValueError(f"{name}: threshold fold absent or regressed")
                elif len(yes) >= len(no):
                    raise ValueError(
                        f"{name}: missing per-rule policy witness {yes} vs {no}"
                    )
                if (
                    old is not None
                    and name != "lshr_full"
                    and len(yes) >= len(body(old, name))
                ):
                    raise ValueError(f"{name}: no measured pre-batch improvement")
                print(
                    f"{opt} {name}: {len(no)} -> {len(yes)} instructions"
                    + (f"; pre-batch {len(body(old,name))}" if old else "")
                )
            if not any(
                re.match(r"mov[lq]\s+.*\(%rsi\)", x) for x in body(on, "observed_batch")
            ):
                raise ValueError("volatile condition evaluation lost")
            if not any(re.match(r"call.*effect", x) for x in body(on, "effect_batch")):
                raise ValueError("side-effecting condition call lost")
        print(
            "PASS 45 individually guarded witnesses; runtime, IR, effects, shared uses and policy"
        )


if __name__ == "__main__":
    main()
