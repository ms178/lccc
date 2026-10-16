#!/usr/bin/env python3
"""Bounded acquire/backedge correctness and global-load scope census.

Counts direct symbolic reads in two deliberately simple kernels, not cycles.
Refuse an unfamiliar addressing shape instead of reporting zero missing loads.
The plain control is guarded do-while: its loads really are must-execute, unlike
an ordinary zero-trip for body that this LICM already declines to speculate.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile

from codegen_oracle import _function_body

ROOT = Path(__file__).resolve().parents[1]
FIXTURE = ROOT / "tests/regression/fixtures/licm-ordering"


def census(assembly: str, function: str) -> dict:
    body = _function_body(assembly.splitlines(), function)
    if body is None:
        raise ValueError("missing requested function: " + function)
    lines = [line.split("#", 1)[0].strip() for line in body]
    labels = {line[:-1]: i for i, line in enumerate(lines) if line.endswith(":")}
    loops = []
    for i, line in enumerate(lines):
        jump = re.fullmatch(r"j[a-z]+\s+([.$A-Za-z_][\w.$]*)", line)
        if jump and jump[1] in labels and labels[jump[1]] <= i:
            loops.append((labels[jump[1]], i))
    reads = []
    for i, line in enumerate(lines):
        parts = line.split(None, 1)
        if len(parts) != 2 or parts[0].startswith("lea"):
            continue
        source = parts[1].split(",", 1)[0]
        if re.search(r"\bordering_g[0-3]\(%rip\)", source):
            reads.append((i, line))
    if len(reads) != 4:
        raise ValueError(f"{function}: expected four direct symbolic reads, found {len(reads)}; unsupported census shape")
    inside = [line for i, line in reads if any(start <= i <= end for start, end in loops)]
    return {"direct_global_reads": len(reads), "inside_backedges": len(inside),
            "backedges": len(loops), "inside_instructions": inside}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--lccc", default=os.environ.get("CCC", str(ROOT / "target/fastbuild/lccc")))
    parser.add_argument("--gcc", default=os.environ.get("GCC", "gcc-16"))
    parser.add_argument("--report", type=Path)
    parser.add_argument("--callgrind", action="store_true",
                        help="also record bounded uncontended shape profiles (not timing)")
    args = parser.parse_args(argv)
    report = {"complete": False, "metric": "static direct global reads in backward-branch spans; not runtime cost", "arms": []}

    def run(command, expected=0):
        result = subprocess.run(command, capture_output=True, text=True, timeout=30)
        row = {"command": list(map(str, command)), "status": result.returncode,
               "stdout": result.stdout, "stderr": result.stderr}
        report.setdefault("commands", []).append(row)
        if result.returncode != expected:
            raise ValueError(f"{command}: expected {expected}, got {row}")
        return result.stdout

    try:
        probe = (FIXTURE / "probe.c").read_text()
        report["sources"] = {p.name: hashlib.sha256(p.read_bytes()).hexdigest()
                             for p in (FIXTURE / "probe.c", FIXTURE / "driver.c", FIXTURE / "shape-driver.c")}
        with tempfile.TemporaryDirectory(prefix="licm-ordering-") as temp:
            directory = Path(temp)
            for name, requested in (("gcc", args.gcc), ("lccc", args.lccc)):
                compiler = shutil.which(requested)
                if not compiler:
                    raise ValueError("missing compiler: " + requested)
                obj, binary, assembly = (directory / (name + suffix) for suffix in (".o", ".exe", ".s"))
                run([compiler, "-O2", "-c", str(FIXTURE / "probe.c"), "-o", str(obj)])
                run([args.gcc, "-O2", "-pthread", str(FIXTURE / "driver.c"), str(obj), "-o", str(binary)])
                output = run([str(binary)])
                if output != "ordering handshake: 8 rounds pass\n":
                    raise ValueError("missing handshake completion marker")
                run([compiler, "-O2", "-S", str(FIXTURE / "probe.c"), "-o", str(assembly)])
                text = assembly.read_text()
                shapes = {kind: census(text, "ordering_" + kind + "_shape") for kind in ("plain", "acquire")}
                if shapes["plain"]["inside_backedges"] != 0 or shapes["acquire"]["inside_backedges"] != 4:
                    raise ValueError(f"{name}: unexpected ordering scope: {shapes}")
                report["arms"].append({"name": name, "compiler_sha256": hashlib.sha256(Path(compiler).read_bytes()).hexdigest(),
                                       "assembly": text, "shapes": shapes})
                print(name, "global reads inside loop: plain=0, acquire=4; handshake=8/8")
                if args.callgrind:
                    profiles = {}
                    for kind in ("plain", "acquire"):
                        profile = directory / (name + "-" + kind + ".callgrind")
                        run([args.gcc, "-O2", "-DORDERING_FUNCTION=ordering_" + kind + "_shape",
                             str(FIXTURE / "shape-driver.c"), str(obj), "-o", str(binary)])
                        output = run(["valgrind", "--tool=callgrind", "--error-exitcode=97",
                                      "--cache-sim=yes", "--branch-sim=yes",
                                      "--callgrind-out-file=" + str(profile), str(binary)])
                        if output != "1000000\n":
                            raise ValueError("incorrect cost witness output")
                        raw = profile.read_text()
                        events = next(line.split()[1:] for line in raw.splitlines() if line.startswith("events:"))
                        totals = next(line.split()[1:] for line in raw.splitlines() if line.startswith("summary:"))
                        if len(events) != len(totals):
                            raise ValueError("incomplete Callgrind counters")
                        profiles[kind] = {"counters": dict(zip(events, map(int, totals))),
                                          "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
                                          "raw_profile": raw}
                    report["arms"][-1]["profiles"] = profiles
                    print(name, "uncontended simulated Ir:",
                          {kind: value["counters"]["Ir"] for kind, value in profiles.items()})
                    report["profile_scope"] = ("100000 iterations; whole-program simulated counters; "
                                               "includes acquire-load and optimizer-shape differences, "
                                               "not isolated barrier latency or hardware timing")
            # Deliberately emulate the suggested unsafe preheader motion. The
            # driver must distinguish 20 from 60, not merely reject a link error.
            bad = directory / "hoisted.c"
            mutant = probe.replace("int sum = 0;", "int sum = 0; int snapshot = ordering_g0 + ordering_g1 + ordering_g2 + ordering_g3;", 1)
            mutant = mutant.replace("sum += ordering_g0 + ordering_g1 + ordering_g2 + ordering_g3;", "sum += snapshot;", 1)
            bad.write_text(mutant)
            binary = directory / "negative"
            run([args.gcc, "-O2", "-pthread", str(FIXTURE / "driver.c"), str(bad), "-o", str(binary)])
            output = run([str(binary)], expected=1)
            if "result=20 expected=60" not in output:
                raise ValueError("negative control failed for the wrong reason")
            report["negative_control"] = output.strip()
            report["complete"] = True
    except (OSError, ValueError, subprocess.TimeoutExpired) as exc:
        report["error"] = str(exc)
        print("FAIL:", exc)
    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(json.dumps(report, indent=2) + "\n")
    return 0 if report["complete"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
