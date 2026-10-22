#!/usr/bin/env python3
"""Demand-observation batch: runtime, per-rule mechanism and exact policy controls."""

import argparse
import json
import os
from pathlib import Path
import re
import tempfile
from check_llvm_missed_zero_choice import (body, gcc_oracle_identity,
                                           report_gcc_oracle, run)

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "tests/regression/llvm_integer_observations.c"
CASES = json.loads(SOURCE.with_suffix(".json").read_text())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--compiler", default=os.environ.get("CCC", str(ROOT / "target/fastbuild/lccc"))
    )
    parser.add_argument("--gcc", default=os.environ.get("GCC", "gcc-16"))
    parser.add_argument("--allow-gcc16-snapshot", action="store_true",
                        help="explicit supplemental snapshot oracle; never normative GCC 16.2 evidence")
    parser.add_argument("--clang", help="optional exact Clang 23.1.3 comparison")
    parser.add_argument(
        "--baseline",
        help="optional saved pre-batch compiler, never inferred from disabled output",
    )
    args = parser.parse_args()
    if len(CASES) != 45 or len({c["name"] for c in CASES}) != 45:
        raise ValueError("missing or duplicate witness contracts")
    # The oracle only produces the expected program output; every mechanism
    # assertion below compares this compiler against itself (pass on vs
    # off), so a snapshot oracle cannot change any verdict -- but the
    # strict default stays, and CI opts in explicitly.
    gcc_version = run([args.gcc, "-dumpfullversion"]).decode().strip()
    report_gcc_oracle(args.gcc, gcc_version,
                      gcc_oracle_identity(gcc_version, args.allow_gcc16_snapshot))
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
                elif name == "not_mask":
                    # Current main also shortened the disabled Boolean return.
                    # A tie still proves complement elimination, but only with
                    # the exact mask/equality shape and a NOT in the control.
                    mask = r"(?:and|cmp)[lq]\s+\$23130,"
                    if (len(yes) > len(no)
                        or any(re.match(r"not[lq]\b", x) for x in yes)
                        or not any(re.match(r"not[lq]\b", x) for x in no)
                        or sum(bool(re.match(mask, x)) for x in yes) != 2):
                        raise ValueError(f"{name}: complement/equality mechanism absent")
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
