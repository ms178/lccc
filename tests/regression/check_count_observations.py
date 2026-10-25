#!/usr/bin/env python3
"""Count observations: independent native oracle, ablation and IR mechanisms."""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
from check_llvm_missed_zero_choice import body, run

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "tests/regression/count_observations.c"


def ir_functions(text):
    parts = re.split(r"^=== IR (\w+) ===$", text, flags=re.M)
    return dict(zip(parts[1::2], parts[2::2]))


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--compiler", default=os.environ.get("CCC", str(ROOT / "target/fastbuild/lccc")))
    ap.add_argument("--gcc", default=os.environ.get("GCC", "gcc-16"))
    ap.add_argument("--clang")
    ap.add_argument("--baseline")
    ap.add_argument("--report", type=Path)
    args = ap.parse_args()
    versions = {}
    for compiler, version in [(args.gcc, r"\b16\.2(?:\.0)?\b")] + ([(args.clang, r"\b23\.1\.3\b")] if args.clang else []):
        versions[compiler] = run([compiler, "--version"]).decode().splitlines()[0]
        if not re.search(version, versions[compiler]):
            raise ValueError(f"unexpected oracle version: {versions[compiler]}")
    env = os.environ | {"CCC_VERIFY_IR": "abort", "CCC_VERIFY_REGALLOC": "1", "CCC_VALIDATE_SSA": "1"}
    off = env | {"CCC_DISABLE_PASSES": "bit_idioms"}
    report = {"oracle_versions": versions, "instruction_counts": {}, "modes": []}
    with tempfile.TemporaryDirectory() as tmp:
        p = Path(tmp)
        expected = None
        for compiler in versions:
            run([compiler, "-O2", "-Wall", "-Wextra", "-Werror", str(SOURCE), "-o", str(p / "oracle")])
            got = run([str(p / "oracle")])
            if expected is not None and got != expected:
                raise ValueError("vendor disagreement")
            expected = got
        report["checksum_and_volatile_reads"] = expected.decode().strip()
        modes = [[o] for o in ("-O0", "-O1", "-O2", "-O3", "-Os")] + [["-O2", "-g"], ["-O2", "-mpopcnt"]]
        for flags, settings in [(f, env) for f in modes] + [(["-O2"], off)]:
            run([args.compiler, *flags, "-Wall", "-Wextra", "-Werror", str(SOURCE), "-o", str(p / "native")], env=settings)
            if run([str(p / "native")]) != expected:
                raise ValueError(f"native mismatch: {flags}, disabled={settings is off}")
            report["modes"].append({"flags": flags, "disabled": settings is off})
        asm = {}
        ir = {}
        for label, compiler, settings in [("on", args.compiler, env), ("off", args.compiler, off)] + ([("before", args.baseline, env)] if args.baseline else []):
            result = subprocess.run([compiler, "-O2", "-S", str(SOURCE), "-o", str(p / f"{label}.s")], env=settings | {"LCCC_DUMP_IR": "1"}, capture_output=True, timeout=60)
            if result.returncode or re.search(rb"\bwarning:", result.stderr, re.I):
                raise RuntimeError(result.stderr.decode())
            asm[label] = (p / f"{label}.s").read_text()
            ir[label] = ir_functions(result.stderr.decode())
        for width in (32, 64):
            for stem in ("cap", "inverse", "swapped", "leading", "trailing"):
                name = f"co_{stem}{width}"
                current = body(asm["on"], name)
                if current != ["movl $1, %eax", "ret"]:
                    raise ValueError(f"constant count observation survived: {name}: {current}")
                report["instruction_counts"][name] = {label: len(body(text, name)) for label, text in asm.items()}
                for label in asm.keys() - {"on"}:
                    if len(body(asm[label], name)) <= len(current):
                        raise ValueError(f"missing ablation/baseline mechanism: {name}: {label}")
            for stem in ("div", "rem", "nonzero"):
                name = f"co_{stem}{width}"
                current_ir = ir["on"][name]
                if "op: Popcount" not in current_ir:
                    raise ValueError(f"count predicate lost: {name}")
                if stem == "nonzero":
                    zero_cmp = r"Cmp .*rhs: Const\([IU](?:32|64)\(0\)\)"
                    if re.search(zero_cmp, current_ir):
                        raise ValueError(f"redundant source-zero comparison: {name}")
                    if not all(re.search(zero_cmp, ir[label][name]) for label in ir.keys() - {"on"}):
                        raise ValueError(f"missing source-zero ablation: {name}")
                else:
                    if current_ir.count("term: CondBranch") != 1:
                        raise ValueError(f"redundant zero-divisor branch: {name}")
                    if not all(ir[label][name].count("term: CondBranch") == 2 for label in ir.keys() - {"on"}):
                        raise ValueError(f"missing zero-divisor ablation: {name}")
            for stem in ("zero", "full", "total_leading", "total_trailing"):
                name = f"co_{stem}{width}"
                if re.search(r"op: (?:Popcount|Clz|Ctz)\b", ir["on"][name]):
                    raise ValueError(f"exact count observation survived: {name}")
            for stem in ("near_cap", "near_signed", "near_other", "near_leading", "near_nonzero"):
                name = f"co_{stem}{width}"
                if body(asm["on"], name) == ["movl $1, %eax", "ret"]:
                    raise ValueError(f"refusal control became constant: {name}")
            name = f"co_shared{width}"
            if "op: Popcount" not in ir["on"][name]:
                raise ValueError(f"shared count producer lost: {name}")
        names = re.findall(r"^co_\w+(?=:)" , asm["on"], re.M)
        for name in names:
            report["instruction_counts"][name] = {label: len(body(text, name)) for label, text in asm.items()}
        report["native_forms"] = len(names)
        # Native volatile counters and the independent custom-zero sentinel
        # oracle also guard effects and the explicitly deferred split-scan case.
    if args.report:
        args.report.write_text(json.dumps(report, indent=2) + "\n")
    print("PASS count observations: " + report["checksum_and_volatile_reads"])


if __name__ == "__main__":
    main()
