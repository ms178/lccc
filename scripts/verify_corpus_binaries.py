#!/usr/bin/env python3
"""Recompile saved Callgrind inputs and require byte-identical executables.

This is deliberately stronger than comparing assembly: program headers, data,
relocations and instruction addresses must match too. It permits attribution
of existing counters to a rebuilt compiler only for these exact inputs/flags;
it makes no claim about other programs. Original profiles are never modified.
"""
import argparse
import hashlib
import json
from pathlib import Path
import sys

import callgrind_ab as cg

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools.corpus import publication


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--compiler", type=Path, required=True)
    parser.add_argument("--matrix", type=Path, action="append", required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args(argv)
    args.out.mkdir(parents=True, exist_ok=True)
    report = {"complete": False, "compiler": str(args.compiler.resolve()),
              "compiler_sha256": digest(args.compiler), "matrices": [],
              "checks": [], "failures": []}
    cache = {}
    def save():
        publication.atomic_bytes(args.out / "equivalence.json",
                                 (json.dumps(report, indent=2) + "\n").encode())
    with publication.lock(args.out / ".equivalence.lock"):
        save()
        for matrix in args.matrix:
            index = json.loads((matrix / "index.json").read_text())
            if (not index.get("complete") or index.get("failures") or
                    not index.get("selected") or
                    set(index["selected"]) != set(index.get("results", {}))):
                raise ValueError(f"incomplete/failed matrix: {matrix}")
            report["matrices"].append(str(matrix))
            for name in index["selected"]:
                manifest = json.loads((matrix / name / "manifest.json").read_text())
                if (manifest.get("failures") or manifest.get("comparable") != 1 or
                        len(manifest.get("results", {})) != 1):
                    raise ValueError(f"incomparable manifest: {matrix / name}")
                stem, row = next(iter(manifest["results"].items()))
                source = ROOT / "tests/benchmark/programs" / (stem + ".c")
                if digest(source) != row["source_sha256"]:
                    raise ValueError(f"source changed: {source}")
                # Do not silently replace an explicitly recorded environment.
                if manifest.get("compiler_environment"):
                    raise ValueError("nonempty original compiler environment: replay explicitly")
                key = (str(source), manifest["opt"], manifest["include"])
                if key not in cache:
                    binary = args.out / str(len(cache)) / stem
                    binary.parent.mkdir(exist_ok=True)
                    error = cg.compile_program(str(args.compiler), manifest["opt"],
                                               source, binary, include=manifest["include"])
                    cache[key] = (None if error else digest(binary), error)
                actual, error = cache[key]
                expected = row["binary_sha256"]["mine"]
                check = {"matrix": str(matrix), "workload": name,
                         "original_compiler_sha256": manifest["tools"]["mine"]["sha256"],
                         "source_sha256": row["source_sha256"],
                         "flags": manifest["opt"], "include": manifest["include"],
                         "expected_binary_sha256": expected, "actual_binary_sha256": actual,
                         "error": error, "pass": error is None and actual == expected}
                report["checks"].append(check)
                if not check["pass"]:
                    report["failures"].append(check)
                save()
        report["complete"] = True
        report["unique_compilations"] = len(cache)
        save()
    print(f"{len(report['checks'])} binary equivalence checks; "
          f"{len(report['failures'])} failures")
    return int(bool(report["failures"]))


if __name__ == "__main__":
    raise SystemExit(main())
