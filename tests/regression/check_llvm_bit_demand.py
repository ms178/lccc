#!/usr/bin/env python3
"""bit demand: native differential, IR verification, mechanism and exact disable token."""

import argparse
import json
import os
from pathlib import Path
import re
import tempfile
from check_llvm_missed_zero_choice import body, run

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "tests/regression/llvm_bit_demand.c"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--compiler", default=os.environ.get("CCC", str(ROOT / "target/fastbuild/lccc"))
    )
    parser.add_argument("--gcc", default=os.environ.get("GCC", "gcc-16"))
    parser.add_argument(
        "--baseline", help="optional saved main compiler; independent of disabling"
    )
    parser.add_argument("--clang", help="optional exact Clang 23.1.3 oracle")
    args = parser.parse_args()
    cases = json.loads(SOURCE.with_suffix(".json").read_text())
    if len(cases) != 60 or len({c["name"] for c in cases}) != 60:
        raise ValueError("missing/duplicate contracts")
    gcc_version = run([args.gcc, "--version"]).decode()
    print(f"oracle: {args.gcc}: {gcc_version.splitlines()[0]}")
    # The oracle only produces the expected program output; every mechanism
    # assertion below compares this compiler against itself (pass on vs
    # off), so the oracle's minor version cannot change any verdict.  Ask
    # for CI's documented policy -- rolling-major GCC 16
    # (scripts/ci_install_gcc16.sh) -- not an exact 16.2 a rolling package
    # archive cannot guarantee.
    if not re.search(r"\b16\.\d+", gcc_version):
        raise ValueError(
            f"a GCC 16 oracle is required, found: {gcc_version.splitlines()[0]}"
        )
    if args.clang and not re.search(
        r"\b23\.1\.3\b", run([args.clang, "--version"]).decode()
    ):
        raise ValueError("exact Clang 23.1.3 required")
    for source in (SOURCE, SOURCE.with_name("llvm_bit_demand_wide.c")):
        check_source(args, source, cases)


def check_source(args, source, cases):
    env = os.environ | {"CCC_VERIFY_IR": "abort"}
    with tempfile.TemporaryDirectory() as directory:
        p = Path(directory)
        expected = None
        for compiler in [args.gcc] + ([args.clang] if args.clang else []):
            run(
                [
                    compiler,
                    "-O3",
                    "-Wall",
                    "-Wextra",
                    "-Werror",
                    str(source),
                    "-o",
                    str(p / "oracle"),
                ]
            )
            actual = run([str(p / "oracle")])
            if expected is not None and actual != expected:
                raise ValueError("oracle disagreement")
            expected = actual
        for flags in [[o] for o in ("-O0", "-O1", "-O2", "-O3", "-Os")] + [
            ["-O3", "-g"],
            ["-O3", "-fwrapv"],
        ]:
            run(
                [
                    args.compiler,
                    *flags,
                    "-Wall",
                    "-Wextra",
                    "-Werror",
                    str(source),
                    "-o",
                    str(p / "native"),
                ],
                env=env,
            )
            if run([str(p / "native")]) != expected:
                raise ValueError(f"native mismatch {flags}")
        for opt in ("-O2", "-O3"):
            texts = {}
            for label, policy in [
                ("on", {}),
                ("off", {"CCC_DISABLE_PASSES": "bit_idioms"}),
                ("prefix", {"CCC_DISABLE_PASSES": "bit_idioms_extra"}),
            ]:
                path = p / (label + ".s")
                run(
                    [
                        args.compiler,
                        opt,
                        "-march=x86-64-v3",
                        "-S",
                        str(source),
                        "-o",
                        str(path),
                    ],
                    env=env | policy,
                )
                texts[label] = path.read_text()
            if args.baseline:
                run(
                    [
                        args.baseline,
                        opt,
                        "-march=x86-64-v3",
                        "-S",
                        str(source),
                        "-o",
                        str(p / "old.s"),
                    ]
                )
                texts["old"] = (p / "old.s").read_text()
            counts = {"smaller": 0, "same": 0, "larger": 0}
            for case in cases:
                name = case["name"]
                yes, no = body(texts["on"], name), body(texts["off"], name)
                if yes != body(texts["prefix"], name):
                    raise ValueError(f"prefix incorrectly disables pass: {name}")
                # These masks were already eliminated by backend rules. They
                # exercise demanded-bit reconstruction, not new asm wins.
                if name not in ("d_xor", "d_or"):
                    if name in ("parity_add", "parity_sub", "parity_xor"):
                        if (
                            sum("popcnt" in x for x in yes) != 1
                            or sum("popcnt" in x for x in no) != 2
                        ):
                            raise ValueError(
                                f"missing count fusion: {name}: {yes} / {no}"
                            )
                    elif len(yes) >= len(no):
                        raise ValueError(f"missing mechanism: {name}: {yes} / {no}")
                if "old" in texts:
                    old = body(texts["old"], name)
                    relation = (
                        "smaller"
                        if len(yes) < len(old)
                        else "same" if len(yes) == len(old) else "larger"
                    )
                    counts[relation] += 1
                    if relation == "larger":
                        raise ValueError(f"code growth vs main: {name}: {yes} / {old}")
                print(
                    f"{source.stem} {opt} {name}: disabled {len(no)}, enabled {len(yes)}"
                )
            if "old" in texts:
                print(f"{opt} versus main: {counts}")
    print(
        "PASS bit-demand: 60 witnesses, seven native modes, IR verification, exact policy controls"
    )


if __name__ == "__main__":
    main()
