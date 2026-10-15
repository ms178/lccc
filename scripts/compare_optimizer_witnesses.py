#!/usr/bin/env python3
"""Bounded native differential witnesses; any failed arm makes the gate fail.

Signed-overflow witnesses explicitly use -fwrapv. FP trap ordering is tested
natively, not under Valgrind. A compiler's treatment of FENV_ACCESS must be
considered before calling its trap-order result an oracle for another compiler.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def run(command, environment, timeout=20):
    try:
        result = subprocess.run(command, env=environment, text=True,
                                capture_output=True, timeout=timeout)
        return dict(status=result.returncode, stdout=result.stdout, stderr=result.stderr)
    except subprocess.TimeoutExpired:
        return dict(status="timeout", stdout="", stderr="bounded execution timed out")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=ROOT)
    parser.add_argument("--extra-root", type=Path, help="optional donor checkout for the PR829 fixtures")
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--compiler", action="append", required=True, help="label=executable; repeat for each arm")
    args = parser.parse_args()
    args.extra_root = args.extra_root or args.root
    args.out.mkdir(parents=True, exist_ok=True)
    report = dict(complete=False, rows=[], compilers={}, metric="native correctness; not performance")

    def save():
        temporary = args.out / "index.tmp"
        temporary.write_text(json.dumps(report, indent=2) + "\n")
        temporary.replace(args.out / "index.json")

    def execute(name, cc, source, flags, parameters, expected, environment, **description):
        binary = (args.out / "witness").resolve()
        binary.unlink(missing_ok=True)
        # Libraries must follow the source/object for --as-needed linkers.
        libraries = [flag for flag in flags if flag.startswith("-l")]
        frontend = [flag for flag in flags if not flag.startswith("-l")]
        command = [cc, *frontend, str(source), *libraries, "-o", str(binary)]
        build = run(command, environment)
        row = dict(compiler=name, source=source.name,
                   source_sha256=hashlib.sha256(source.read_bytes()).hexdigest(),
                   command=command, args=parameters, build=build, expected=expected,
                   environment={k: v for k, v in environment.items() if k.startswith("CCC_")},
                   **description)
        if build["status"] == 0:
            row["run"] = run([str(binary), *parameters], environment, timeout=2)
        row["pass"] = (build["status"] == 0 and row["run"]["status"] == 0
                       and (expected is None or row["run"]["stdout"] == expected))
        report["rows"].append(row)
        save()
        binary.unlink(missing_ok=True)

    save()
    for entry in args.compiler:
        name, requested = entry.split("=", 1)
        cc = shutil.which(requested)
        if not cc or name in report["compilers"]:
            parser.error("missing compiler or repeated compiler label: " + entry)
        report["compilers"][name] = dict(path=cc, sha256=hashlib.sha256(Path(cc).read_bytes()).hexdigest())
        for abi in ("64", "32"):
            for opt in ("-O1", "-O2", "-O3", "-Os"):
                for rotation in (False, True):
                    environment = {k: v for k, v in os.environ.items() if not k.startswith("CCC_")}
                    environment["CCC_VALIDATE_SSA"] = "1"
                    if rotation:
                        environment["CCC_LOOP_ROTATE"] = "1"
                    flags = [opt, "-m" + abi, "-msse2", "-fwrapv"]
                    execute(name, cc, args.root / "tests/regression/affine_control_proof.c",
                            flags, [], "affine-control-proof 5 97\n", environment,
                            abi=abi, opt=opt, rotation=rotation)
                    for case, expected in enumerate(("28", "-1073665362", "1", "10")):
                        execute(name, cc, args.extra_root / "tests/regression/wrapv_domains.c",
                                flags, [str(case)], expected + "\n", environment,
                                abi=abi, opt=opt, rotation=rotation)
        for opt in ("-O0", "-O1", "-O2", "-O3", "-Os", "-Oz"):
            environment = {k: v for k, v in os.environ.items() if not k.startswith("CCC_")}
            execute(name, cc, args.extra_root / "tests/regression/strict_recip_traps.c",
                    [opt, "-march=x86-64-v3", "-ffp-contract=off", "-lm"], [], None,
                    environment, opt=opt)
        # The first-trap witness alone missed a backedge-PRE rounding bug.
        # Keep the dynamic-rounding/FTZ/DAZ/sticky-flag digest in the portable
        # comparison too, not only in the larger local CI shell gate.
        for arch in ("x86-64", "x86-64-v3"):
            for opt in ("-O0", "-O1", "-O2", "-O3"):
                environment = {k: v for k, v in os.environ.items() if not k.startswith("CCC_")}
                environment["CCC_VALIDATE_SSA"] = "1"
                execute(name, cc, args.root / "tests/regression/fp_recurrence/cases.c",
                        [opt, "-march=" + arch, "-ffp-contract=off", "-frounding-math", "-lm"], [],
                        "fp recurrence: PASS e778c29691d2c901\n", environment, opt=opt, arch=arch)
    report["complete"] = True
    report["summary"] = {
        name: {"pass": sum(row["pass"] for row in report["rows"] if row["compiler"] == name),
               "fail": sum(not row["pass"] for row in report["rows"] if row["compiler"] == name)}
        for name in report["compilers"]
    }
    save()
    print(json.dumps(report["summary"], indent=2))
    return int(any(row["fail"] for row in report["summary"].values()))


if __name__ == "__main__":
    raise SystemExit(main())
