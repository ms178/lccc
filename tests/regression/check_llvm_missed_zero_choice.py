#!/usr/bin/env python3
"""zero-choice: native oracle + mechanism + disabled-policy + volatile condition."""

import argparse
import os
from pathlib import Path
import re
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]

def gcc_oracle_identity(version, allow_snapshot=False):
    """Only the Debian exact-version gate supplies normative GCC evidence.

    `version` is a -dumpfullversion token.  Exact 16.2(.0) is normative;
    with allow_snapshot any other GCC 16 token is accepted as a
    supplemental oracle (hosted CI provisions the rolling-major package,
    scripts/ci_install_gcc16.sh).  Returns True when normative.
    """
    normative = bool(re.fullmatch(r"16\.2(?:\.0)?", version))
    if not normative and not (allow_snapshot and version.startswith("16.")):
        raise ValueError(f"GCC 16.2 oracle required, found {version!r}")
    return normative


def report_gcc_oracle(name, version, normative):
    kind = "normative" if normative else "supplemental snapshot"
    print(f"oracle: {name}: GCC {version} ({kind})")



def run(command, env=None):
    result = subprocess.run(command, env=env, capture_output=True, timeout=60)
    if result.returncode or re.search(rb"\bwarning:", result.stderr, re.I):
        raise RuntimeError(
            f"failed/warning-producing command {command!r}: {result.stderr.decode(errors='replace')}"
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


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--compiler", default=os.environ.get("CCC", str(ROOT / "target/fastbuild/lccc"))
    )
    parser.add_argument("--gcc", default=os.environ.get("GCC", "gcc-16"))
    parser.add_argument("--allow-gcc16-snapshot", action="store_true",
                        help="explicit supplemental snapshot oracle; never normative GCC 16.2 evidence")
    args = parser.parse_args()
    # The oracle only produces the expected program output; every mechanism
    # assertion below compares this compiler against itself (pass on vs
    # off), so a snapshot oracle cannot change any verdict -- but the
    # strict default stays, and CI opts in explicitly.
    version = run([args.gcc, "-dumpfullversion"]).decode().strip()
    report_gcc_oracle(args.gcc, version,
                      gcc_oracle_identity(version, args.allow_gcc16_snapshot))
    source = str(ROOT / "tests/regression/llvm_missed_zero_choice.c")
    with tempfile.TemporaryDirectory() as tmp:
        p = Path(tmp)
        run(
            [
                args.gcc,
                "-O2",
                "-Wall",
                "-Wextra",
                "-Werror",
                source,
                "-o",
                str(p / "reference"),
            ]
        )
        expected = run([str(p / "reference")])
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
                    source,
                    "-o",
                    str(p / "native"),
                ]
            )
            if run([str(p / "native")]) != expected:
                raise ValueError(f"native output mismatch: {flags}")
        for opt in ("-O2", "-O3"):
            run([args.compiler, opt, "-S", source, "-o", str(p / "on.s")])
            run(
                [args.compiler, opt, "-S", source, "-o", str(p / "off.s")],
                env=os.environ | {"CCC_DISABLE_PASSES": "bit_idioms"},
            )
            on, off = (p / "on.s").read_text(), (p / "off.s").read_text()
            for name in ("zero_eq32", "zero_ne32", "zero_eq64", "zero_ne64"):
                yes, no = body(on, name), body(off, name)
                if any(re.match(r"(?:neg|cmov|j\w+)", line) for line in yes):
                    raise ValueError(f"{name}: conditional negation survives: {yes}")
                if len(yes) >= len(no) or not any(
                    re.match(r"neg", line) for line in no
                ):
                    raise ValueError(f"{name}: missing disabled-policy witness")
                print(f"{opt} {name}: {len(no)} -> {len(yes)} instructions")
            observed = body(on, "observed_condition")
            if not any(re.match(r"mov[lq]\s+.*\(%rsi\)", line) for line in observed):
                raise ValueError(f"observable condition read lost: {observed}")
    print("PASS LLVM #233753: runtime, mechanism, policy, observable condition")


if __name__ == "__main__":
    main()
