#!/usr/bin/env python3
"""Paired compiler-work profiles; simulated Ir is NOT compilation wall time.

Both compiler executables are frozen into equal-length paths. Every arm must
produce identical assembly natively and under Callgrind. Shared event parsing,
cache geometry and atomic publication come from the existing research tools.
An interrupted/failed sweep leaves complete=false, never a successful subset.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import shlex
import shutil
import statistics
import subprocess

import callgrind_ab as cg
from tools.corpus import publication


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--mine", type=Path, required=True)
    parser.add_argument("--ref", type=Path, required=True)
    parser.add_argument("--source", type=Path, action="append", required=True)
    parser.add_argument("--flags", default="-O2 -march=x86-64-v3 -fwrapv")
    parser.add_argument("--rounds", type=int, default=3)
    parser.add_argument("--timeout", type=int, default=180)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args(argv)
    if args.rounds < 1 or args.timeout < 1:
        parser.error("rounds and timeout must be positive")
    flags = shlex.split(args.flags)
    if any(f in ("-o", "-c", "-S", "-E") or f.startswith("-o") for f in flags):
        parser.error("output/mode flags are owned by the harness")
    sources = [s.resolve(strict=True) for s in args.source]
    if len(set(sources)) != len(sources):
        parser.error("duplicate source selection")
    binaries = {"mine": args.mine.resolve(strict=True), "ref": args.ref.resolve(strict=True)}
    args.out.mkdir(parents=True, exist_ok=True)
    out = args.out.resolve()
    geometry, overrides = cg.geometry()
    report = {"complete": False, "metric": "simulated compiler work; NOT wall-clock speedup",
              "flags": flags, "geometry": geometry, "geometry_overrides": overrides,
              "valgrind": cg.tool_identity("valgrind"), "binaries": {}, "results": [], "error": None}

    def save():
        publication.atomic_bytes(out / "summary.json", (json.dumps(report, indent=2) + "\n").encode())

    with publication.lock(out / ".compiler-work.lock"):
        save()
        try:
            for name, short in (("mine", "aa"), ("ref", "bb")):
                directory = out / short
                directory.mkdir(exist_ok=True)
                frozen = directory / "compiler"
                shutil.copy2(binaries[name], frozen)
                report["binaries"][name] = {"original": str(binaries[name]),
                                            "sha256": cg.schema.file_hash(frozen)}
                binaries[name] = frozen
            save()
            for index, source in enumerate(sources):
                row = {"source": str(source), "source_sha256": cg.schema.file_hash(source), "rounds": []}
                report["results"].append(row)
                for repetition in range(args.rounds):
                    order = ["mine", "ref"] if repetition % 2 == 0 else ["ref", "mine"]
                    pair = {"order": order, "arms": {}}
                    row["rounds"].append(pair)
                    for name in order:
                        directory = binaries[name].parent
                        assembly = directory / f"case-{index}.s"
                        command = [str(binaries[name]), *flags, "-S", str(source), "-o", str(assembly)]
                        assembly.unlink(missing_ok=True)
                        native = subprocess.run(command, capture_output=True, timeout=args.timeout, check=True)
                        expected = assembly.read_bytes()
                        assembly.unlink()
                        profile = directory / f"case-{index}-round-{repetition}.cg"
                        profile.unlink(missing_ok=True)
                        instrument = ["valgrind", "--tool=callgrind", "--cache-sim=yes", "--branch-sim=yes", "--quiet",
                                      *[f"--{key}={value}" for key, value in geometry.items()],
                                      f"--callgrind-out-file={profile}", *command]
                        result = subprocess.run(instrument, capture_output=True, timeout=args.timeout)
                        profile.with_suffix(".stderr").write_bytes(result.stderr)
                        if result.returncode:
                            raise ValueError(f"instrumented compiler rc={result.returncode}: {result.stderr.decode(errors='replace')[-1500:]}")
                        if assembly.read_bytes() != expected or result.stdout != native.stdout:
                            raise ValueError("native/instrumented compiler output mismatch")
                        pair["arms"][name] = {"command": instrument, "events": cg.parse_summary(profile),
                                              "profile_sha256": cg.schema.file_hash(profile),
                                              "assembly_sha256": hashlib.sha256(expected).hexdigest(),
                                              "stderr": result.stderr.decode(errors="replace"),
                                              "native_stderr": native.stderr.decode(errors="replace")}
                        save()
                    pair["ir_ratio"] = pair["arms"]["mine"]["events"]["Ir"] / pair["arms"]["ref"]["events"]["Ir"]
                    save()
                    print(source.name, repetition, pair["ir_ratio"], flush=True)
                row["median_ir_ratio"] = statistics.median(pair["ir_ratio"] for pair in row["rounds"])
            report["complete"] = True
        except (OSError, ValueError, subprocess.SubprocessError) as error:
            report["error"] = str(error)
        save()
    return int(not report["complete"])


if __name__ == "__main__":
    raise SystemExit(main())
