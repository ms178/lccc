#!/usr/bin/env python3
"""Signed-mask range differential: 32 Memcheck arms, eight kernels per arm.

GCC/Clang and LCCC enabled/disabled each run both ABIs at O1/O2/O3/Os.
The unsigned-reference fixture checks negative/sign-bit masks and overflow
boundaries. Missing tools, compilation failures, timeouts and Memcheck errors
fail closed. The atomic report retains partial failures and exact identities.
This is an opt-in bounded research experiment, not a hardware timing test.
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
EXPECTED = "ivw-signed-mask 770224292\n"


def matrix(lccc, gcc, clang):
    for label, compiler, disabled in (("gcc", gcc, False), ("clang", clang, False),
                                     ("lccc", lccc, False), ("lccc-no-ivwiden", lccc, True)):
        for abi in ("-m64", "-m32"):
            for opt in ("-O1", "-O2", "-O3", "-Os"):
                yield label, compiler, disabled, abi, opt


def accepted(build, execution):
    return (type(build.get("status")) is int and build["status"] == 0
            and type(execution.get("status")) is int and execution["status"] == 0
            and execution.get("stdout") == EXPECTED)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--lccc", required=True)
    parser.add_argument("--gcc", required=True)
    parser.add_argument("--clang", required=True)
    parser.add_argument("--work-dir", type=Path, required=True)
    parser.add_argument("--json", type=Path, required=True)
    args = parser.parse_args(argv)
    args.work_dir.mkdir(parents=True, exist_ok=True)
    args.json.parent.mkdir(parents=True, exist_ok=True)
    source = ROOT / "tests/regression/ivw_signed_mask.c"
    report = {"complete": False, "source_sha256": hashlib.sha256(source.read_bytes()).hexdigest(),
              "expected_stdout": EXPECTED, "compilers": {}, "rows": []}

    def save():
        temporary = args.json.with_suffix(".tmp")
        temporary.write_text(json.dumps(report, indent=2) + "\n")
        temporary.replace(args.json)

    def run(command, environment):
        try:
            result = subprocess.run(command, capture_output=True, text=True,
                                    env=environment, timeout=30)
            return {"command": command, "status": result.returncode,
                    "stdout": result.stdout, "stderr": result.stderr}
        except subprocess.TimeoutExpired:
            return {"command": command, "status": "timeout", "stdout": "",
                    "stderr": "bounded execution timed out"}

    try:
        valgrind = shutil.which("valgrind")
        if not valgrind:
            raise ValueError("Valgrind is required, not a skipped arm")
        report["valgrind"] = subprocess.check_output([valgrind, "--version"], text=True, timeout=30)
        for label, requested, disabled, abi, opt in matrix(args.lccc, args.gcc, args.clang):
            compiler = shutil.which(requested)
            if not compiler:
                raise ValueError("missing requested compiler: " + requested)
            if label not in report["compilers"]:
                report["compilers"][label] = {
                    "path": compiler, "sha256": hashlib.sha256(Path(compiler).read_bytes()).hexdigest(),
                    "version": subprocess.check_output([compiler, "--version"], text=True, timeout=30)}
            environment = {k: v for k, v in os.environ.items() if not k.startswith("CCC_")}
            environment["CCC_VALIDATE_SSA"] = "1"
            if disabled:
                environment["CCC_NO_IVWIDEN"] = "1"
            binary = (args.work_dir / "signed-mask").resolve()
            binary.unlink(missing_ok=True)
            row = {"compiler": label, "abi": abi, "opt": opt, "pass": False,
                   "environment": {k: v for k, v in environment.items() if k.startswith("CCC_")}}
            report["rows"].append(row)
            row["build"] = run([compiler, abi, opt, "-fwrapv", str(source), "-o", str(binary)], environment)
            save()
            if row["build"]["status"] == 0:
                row["binary_sha256"] = hashlib.sha256(binary.read_bytes()).hexdigest()
                row["run"] = run([valgrind, "--error-exitcode=97", "--leak-check=full", str(binary)], environment)
            row["pass"] = accepted(row["build"], row.get("run", {}))
            save()
            if not row["pass"]:
                raise ValueError(f"failed arm: {label}/{abi}/{opt}; see {args.json}")
        report["complete"] = True
        print("PASS: 32 signed-mask compiler/ABI/optimization/disable Memcheck configurations")
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        report["error"] = str(error)
        print("FAIL:", error)
    finally:
        save()
    return 0 if report["complete"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
