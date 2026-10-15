#!/usr/bin/env python3
"""Bounded corpus screen via callgrind_ab, not native-runtime rankings.

All compiler arms use the same checked-in problem sizes and -fwrapv, because
arith_loop and loop_patterns exercise overflowing integer arithmetic. The rank
is whole-program simulated Ir (including libraries/startup), NOT speedup or the default-
size corpus's runtime rank. Per-case manifests retain commands, hashes, full
counters, correctness checks and errors; an interrupted sweep publishes only
an explicitly incomplete index. No raw counter data is thrown away.
"""
import argparse
import json
from pathlib import Path
import contextlib
import callgrind_ab as cg
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tests/benchmark"))
import run_benchmarks as bench
sys.path.insert(0, str(ROOT))
from tools.corpus import publication


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--mine", required=True)
    parser.add_argument("--ref", default="gcc")
    parser.add_argument("--gcc-include-command", default="gcc",
                        help="GCC driver supplying matching builtin headers")
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--target-flags", default="-march=x86-64-v3",
                        help="common target flags; use an empty string for compiler defaults")
    parser.add_argument("--only", help="comma-separated corpus ids")
    args = parser.parse_args(argv)
    sizes = json.loads((ROOT / "tests/benchmark/callgrind_sizes.json").read_text())
    known = {b.name for b in bench.BENCHMARKS}
    if set(sizes) != known:
        parser.error("instrumentation sizes must cover the exact corpus")
    selected = set(args.only.split(",")) if args.only else known
    if not selected or selected - known:
        parser.error("empty/unknown corpus selection")
    args.out.mkdir(parents=True, exist_ok=True)
    index = {"complete": False, "metric": "bounded whole-program simulated Ir; not runtime",
             "selected": sorted(selected), "results": {}, "failures": []}
    def save():
        publication.atomic_bytes(args.out / "index.json", (json.dumps(index, indent=2)+"\n").encode())
    with publication.lock(args.out / ".corpus.lock"):
        save()
        for item in bench.BENCHMARKS:
            if item.name not in selected:
                continue
            directory = args.out / item.name
            directory.mkdir(exist_ok=True)
            flags = "-O2 " + args.target_flags + " -fwrapv -Wl,--no-as-needed -lm " + sizes[item.name]
            command = [args.mine, args.ref, "--opt=" + flags, bench.source_for(item).stem,
                       "--out-root", str(directory), "--instrument-timeout", "60",
                       "--gcc-include-command", args.gcc_include_command]
            print(item.name, flush=True)
            try:
                with (directory / "run.log").open("w") as log, contextlib.redirect_stdout(log), contextlib.redirect_stderr(log):
                    result = cg.main(command)
                if result:
                    raise ValueError(f"measurement exit {result}")
                manifest = json.loads((directory / "manifest.json").read_text())
                row = manifest["results"][bench.source_for(item).stem]
                index["results"][item.name] = {"flags": flags, "metrics": row["metrics"],
                                                "manifest": str(directory / "manifest.json")}
            except (OSError, ValueError) as error:
                index["failures"].append({"name": item.name, "error": str(error)})
            save()
        index["complete"] = True
        index["worst_first"] = sorted(index["results"], key=lambda name:
                                       index["results"][name]["metrics"]["ir_ratio"], reverse=True)
        save()
    return int(bool(index["failures"]))


if __name__ == "__main__":
    raise SystemExit(main())
