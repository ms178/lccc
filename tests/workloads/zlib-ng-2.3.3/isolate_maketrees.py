#!/usr/bin/env python3
"""Isolate maketrees TU/optimization failures in an already configured zlib-ng tree.

No downloads, builds of LCCC, or cleanup of caller directories. Reuses Ninja's
actual compile options, recompiles only the three maketrees TUs, and links every
arm with the same reference compiler. --output must not already exist. A mismatch
exits 1 and is an audit observation, not an accepted regression test baseline.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import shlex
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[3]
OPT_LEVELS = {"-O0", "-O1", "-O2", "-O3", "-Os", "-Oz", "-Og", "-Ofast"}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--build", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--lccc", default=str(ROOT / "target/fastbuild/lccc"))
    parser.add_argument("--reference", default="gcc")
    args = parser.parse_args()
    source, build = args.source.resolve(), args.build.resolve()
    args.output = args.output.resolve()
    expected = (source / "trees_tbl.h").read_bytes()
    args.output.mkdir(parents=True, exist_ok=False)
    rows = []
    # Keep an incomplete receipt even if any subprocess raises or times out.
    report = {"complete": False, "records": rows, "expected_sha256": hashlib.sha256(expected).hexdigest()}

    def run(command, *, cwd=None, env=None):
        p = subprocess.run(command, cwd=cwd, env=env, capture_output=True, timeout=60)
        rows.append({"command": command, "cwd": str(cwd) if cwd else None,
                     "rc": p.returncode, "stderr": p.stderr.decode(errors="replace")})
        if p.returncode:
            raise RuntimeError(f"command failed: {shlex.join(command)}\n{rows[-1]['stderr']}")
        return p

    try:
        lines = run(["ninja", "-C", str(build), "-t", "commands", "maketrees"]).stdout.decode().splitlines()
        units = []
        for line in lines:
            argv = shlex.split(line)
            if "-c" not in argv:
                continue
            filename = Path(argv[argv.index("-c") + 1])
            filename = filename if filename.is_absolute() else build / filename
            flags, i = [], 1
            while i < len(argv):
                arg = argv[i]
                if arg in {"-c", "-o", "-MF", "-MT", "-MQ"}:
                    i += 2
                    continue
                if arg not in OPT_LEVELS | {"-MD", "-MMD", "-MP"}:
                    flags.append(arg)
                i += 1
            units.append((filename.resolve(), flags))
        if sorted(f.name for f, _ in units) != ["maketrees.c", "trees.c", "zutil.c"]:
            raise ValueError("unexpected maketrees build graph; inspect rather than guessing")
        report["sources"] = {str(f): hashlib.sha256(f.read_bytes()).hexdigest() for f, _ in units}
        arms = [("reference-O2", args.reference, "-O2", {})]
        arms += [(f"lccc-O{i}", args.lccc, f"-O{i}", {}) for i in range(4)]
        arms += [("lccc-no-iv-widen", args.lccc, "-O2", {"CCC_NO_IV_WIDEN": "1"})]
        binaries = {}
        with tempfile.TemporaryDirectory(prefix="maketrees-isolate-", dir=args.output) as temp:
            work = Path(temp)
            objects = {}
            for label, cc, opt, extra in arms:
                objects[label] = []
                for f, flags in units:
                    obj = work / f"{label}-{f.stem}.o"
                    run([cc, *flags, opt, "-c", str(f), "-o", str(obj)], cwd=build,
                        env=os.environ | extra)
                    objects[label].append(str(obj))
                exe = work / label
                run([args.reference, *objects[label], "-o", str(exe)])
                binaries[label] = exe
            for index, (f, _) in enumerate(units):
                mixed = objects["reference-O2"].copy()
                mixed[index] = objects["lccc-O2"][index]
                label = "mixed-" + f.stem
                exe = work / label
                run([args.reference, *mixed, "-o", str(exe)])
                binaries[label] = exe
            comparisons = []
            for label, exe in binaries.items():
                result = run([str(exe)])
                (args.output / (label + ".txt")).write_bytes(result.stdout)
                comparisons.append({"arm": label, "matches": result.stdout == expected,
                                    "sha256": hashlib.sha256(result.stdout).hexdigest()})
            report.update(complete=True, comparisons=comparisons,
                          arms=[{"name": label, "cc": cc, "opt": opt, "env": extra}
                                for label, cc, opt, extra in arms])
        for row in comparisons:
            print(row["arm"], "MATCH" if row["matches"] else "MISMATCH", row["sha256"])
        return int(any(not row["matches"] for row in comparisons))
    finally:
        (args.output / "isolation.json").write_text(json.dumps(report, indent=2) + "\n")


if __name__ == "__main__":
    raise SystemExit(main())
