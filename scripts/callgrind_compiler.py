#!/usr/bin/env python3
"""Paired compiler-work profiles; simulated Ir is NOT compilation wall time.

Both compiler executables are frozen into equal-length paths. Each source must
produce the same assembly bytes and stdout across both arms, all rounds, and
native/Callgrind execution. Shared event parsing, cache geometry and atomic
publication come from the existing research tools. Once an invocation owns
its output lock, an interrupted/failed sweep leaves complete=false. Use a
separate output directory to measure a compiler frozen by an earlier sweep.
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
import sys
import tempfile

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
    args.out.mkdir(parents=True, exist_ok=True)
    out = args.out.resolve()
    directories = {"mine": out / "aa", "ref": out / "bb"}
    report = {"complete": False, "metric": "simulated compiler work; NOT wall-clock speedup",
              "flags": None, "geometry": None, "geometry_overrides": None,
              "valgrind": None, "binaries": {}, "results": [], "error": None}

    def save():
        publication.atomic_bytes(out / "summary.json", (json.dumps(report, indent=2) + "\n").encode())

    with publication.lock(out / ".compiler-work.lock"):
        # Invalidate any previous success before preflight, not just before
        # compilation. A missing input or tool must not leave a stale verdict.
        save()
        try:
            if args.rounds < 1 or args.timeout < 1:
                raise ValueError("rounds and timeout must be positive")
            flags = shlex.split(args.flags)
            report["flags"] = flags
            if any(f in ("-o", "-c", "-S", "-E") or f.startswith("-o") for f in flags):
                raise ValueError("output/mode flags are owned by the harness")
            sources = [s.resolve(strict=True) for s in args.source]
            if len(set(sources)) != len(sources):
                raise ValueError("duplicate source selection")
            binaries = {"mine": args.mine.resolve(strict=True), "ref": args.ref.resolve(strict=True)}
            for directory in directories.values():
                if directory.is_symlink():
                    raise ValueError(f"compiler-work output directory must not be a symlink: {directory}")
                for path in (*sources, *binaries.values()):
                    if path.is_relative_to(directory):
                        raise ValueError(f"input is in a reserved compiler-work output directory: {path}")
            geometry, overrides = cg.geometry()
            report.update(geometry=geometry, geometry_overrides=overrides, valgrind=cg.tool_identity("valgrind"))
            for name, directory in directories.items():
                directory.mkdir(exist_ok=True)
                frozen = directory / "compiler"
                # A previous slot can be linked to an input. Replacing the
                # directory entry must not truncate and rewrite that inode.
                with tempfile.TemporaryDirectory(prefix=".freeze-", dir=directory) as stage:
                    staged = Path(stage) / "compiler"
                    shutil.copy2(binaries[name], staged)
                    staged.replace(frozen)
                report["binaries"][name] = {"original": str(binaries[name]),
                                            "sha256": cg.schema.file_hash(frozen)}
                binaries[name] = frozen
            save()
            for index, source in enumerate(sources):
                row = {"source": str(source), "source_sha256": cg.schema.file_hash(source), "rounds": []}
                report["results"].append(row)
                baseline = None
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
                        # Matching each arm against its own instrumented run
                        # is insufficient: two stable but distinct outputs
                        # would yield a bogus identity-preserving Ir delta.
                        if baseline is None:
                            baseline = expected, native.stdout
                        else:
                            where = f"{source}: round {repetition + 1}, {name}"
                            if expected != baseline[0]:
                                raise ValueError(f"{where}: native assembly differs from round 1 mine")
                            if native.stdout != baseline[1]:
                                raise ValueError(f"{where}: native stdout differs from round 1 mine")
                        assembly.unlink()
                        profile = directory / f"case-{index}-round-{repetition}.cg"
                        stderr_path = profile.with_suffix(".stderr")
                        profile.unlink(missing_ok=True)
                        stderr_path.unlink(missing_ok=True)
                        instrument = ["valgrind", "--tool=callgrind", "--cache-sim=yes", "--branch-sim=yes", "--quiet",
                                      *[f"--{key}={value}" for key, value in geometry.items()],
                                      f"--callgrind-out-file={profile}", *command]
                        result = subprocess.run(instrument, capture_output=True, timeout=args.timeout)
                        stderr_path.write_bytes(result.stderr)
                        if result.returncode:
                            raise ValueError(f"instrumented compiler rc={result.returncode}: {result.stderr.decode(errors='replace')[-1500:]}")
                        if assembly.read_bytes() != expected or result.stdout != native.stdout:
                            raise ValueError("native/instrumented compiler output mismatch")
                        pair["arms"][name] = {"command": instrument, "events": cg.parse_summary(profile),
                                              "profile_sha256": cg.schema.file_hash(profile),
                                              "assembly_sha256": hashlib.sha256(expected).hexdigest(),
                                              "stdout_sha256": hashlib.sha256(native.stdout).hexdigest(),
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
    if report["error"] is not None:
        print(f"compiler-work measurement failed: {report['error']}", file=sys.stderr)
    return int(not report["complete"])


if __name__ == "__main__":
    raise SystemExit(main())
