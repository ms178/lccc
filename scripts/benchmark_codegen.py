#!/usr/bin/env python3
"""Registry-aware static codegen survey; errors fail, counts are NOT runtime.

Unlike a programs/*.c glob (or a list of source_for() paths alone), each
registered workload retains its own macro flags. In particular,
linux_find_bit_scaled and lz4_match_extend share sources with other workloads
but are different compilation requests. Never deduplicate them by filename.

Example (offline local-only inventory):
  python3 scripts/benchmark_codegen.py --gcc gcc --oracles '' \
      --local target/fastbuild/lccc --out results/codegen

Omit --oracles to request the codegen oracle's configured competition set.
Compiler Explorer identities/errors are retained by that tool. A failed or
unavailable oracle is a failure here, not a silently successful rank survey.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import shlex
import subprocess
import sys

import codegen_oracle
import oracle_profile

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tests.benchmark import run_benchmarks as bench
from tools.corpus import publication


def compilation_flags(item, common: str, include: str) -> tuple[str, str]:
    """The workload macros go to BOTH compilers; host includes go local only."""
    flags = [*shlex.split(common), *item.extra_flags]
    return shlex.join(flags), shlex.join([*flags, "-I" + include])


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--local", default=str(bench.DEFAULT_LCCC))
    parser.add_argument("--gcc", default="gcc-16", help="local builtin-header provider; identity recorded")
    parser.add_argument("--oracles", help="CE compiler list; empty string explicitly selects local-only")
    parser.add_argument("--verify-haswell-profile", action="store_true",
                        help="require a four-feature-family admission canary before the survey")
    parser.add_argument("--flags", default="-O2 -march=x86-64-v3 -fwrapv")
    parser.add_argument("--only", help="comma-separated registry IDs, not source stems")
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args(argv)
    known = {item.name for item in bench.BENCHMARKS}
    selected = set(args.only.split(",")) if args.only else known
    if not selected or selected - known:
        parser.error("empty/unknown corpus selection")
    include = subprocess.check_output([args.gcc, "-print-file-name=include"], text=True).strip()
    if not Path(include).is_dir():
        parser.error("GCC did not identify an existing builtin-header directory")
    version = subprocess.check_output([args.gcc, "--version"], text=True)
    args.out.mkdir(parents=True, exist_ok=True)
    index = {"complete": False, "metric": "whole-TU static instructions; NOT runtime or spill causality",
             "selected": sorted(selected), "header_provider": {"command": args.gcc, "version": version,
             "include": include}, "results": {}, "failures": []}

    def save():
        publication.atomic_bytes(args.out / "index.json", (json.dumps(index, indent=2) + "\n").encode())

    with publication.lock(args.out / ".codegen.lock"):
        save()
        if args.verify_haswell_profile:
            names = [s.strip() for s in (args.oracles if args.oracles is not None else ",".join(codegen_oracle.DEFAULT_ORACLES)).split(",") if s.strip()]
            profile = oracle_profile.verify(names, args.flags)
            profile_path = args.out / "profile.json"
            publication.atomic_bytes(profile_path, (json.dumps(profile, indent=2) + "\n").encode())
            index["target_profile"] = {"report": str(profile_path), "complete": profile["complete"]}
            from oracle_survey_diagnostics import emit
            for name, row in profile["compilers"].items():
                emit({"profile_compiler": name, "id": row["id"],
                      "requested_flags": args.flags, "effective_flags": row.get("effective_flags"),
                      "features": row.get("features"), "ok": row["ok"],
                      "error": str(row.get("error", ""))[:500],
                      "compile_only": True, "target": profile["target"]})
            if not profile["complete"]:
                index["failures"].append({"name": "target-profile", "error": "admission failed; see profile.json and notices"})
                save()
                return 1
            save()
        for item in bench.BENCHMARKS:
            if item.name not in selected:
                continue
            source = bench.source_for(item)
            directory = args.out / item.name
            directory.mkdir(exist_ok=True)
            flags, local_flags = compilation_flags(item, args.flags, include)
            command = [str(source), "--local", args.local, "--flags", flags,
                       "--local-flags", local_flags, "--all-functions", "--jobs", "1",
                       "--json", str(directory / "result.json"),
                       "--artifact-dir", str(directory / "artifacts")]
            if args.oracles is not None:
                command += ["--oracles", args.oracles]
            row = {"source": str(source), "source_sha256": hashlib.sha256(source.read_bytes()).hexdigest(),
                   "tags": item.tags, "extra_flags": item.extra_flags, "arguments": command,
                   "report": str(directory / "result.json")}
            print(item.name, flush=True)
            try:
                report = directory / "result.json"
                report.unlink(missing_ok=True)
                status = codegen_oracle.main(command)
                row["exit_code"] = status
                if status:
                    index["failures"].append({"name": item.name, "exit_code": status})
                else:
                    result = json.loads(report.read_text())
                    if len(result) != 1 or result[0].get("errors") or not result[0].get("records"):
                        raise ValueError("successful oracle returned empty/invalid evidence")
                    if Path(result[0]["source"]).resolve() != source.resolve():
                        raise ValueError("oracle report names another source")
                    local = [record for record in result[0]["records"] if record["key"] == "lccc"]
                    if len(local) != 1 or shlex.split(local[0]["flags"]) != shlex.split(local_flags):
                        raise ValueError("oracle report lost local workload flags")
            except (OSError, ValueError) as error:
                row["error"] = str(error)
                index["failures"].append({"name": item.name, "error": str(error)})
            index["results"][item.name] = row
            save()
        index["complete"] = True
        save()
    return int(bool(index["failures"]))


if __name__ == "__main__":
    raise SystemExit(main())
