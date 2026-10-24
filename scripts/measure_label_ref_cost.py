#!/usr/bin/env python3
"""Measure compiler cost on deterministic branch-heavy TUs, not program speed.

Uses randomized paired order, CPU affinity, excluded warmups, exact assembly
hashes and optional Callgrind instruction counts. No timing threshold is a CI
gate. Example:
  python3 scripts/measure_label_ref_cost.py --before /path/to/baseline \
      --after target/fastbuild/lccc --out /tmp/label-cost --callgrind
"""
from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import random
import subprocess
import time


def source(functions: int) -> str:
    text = "extern unsigned opaque(unsigned);\n"
    for function in range(functions):
        text += f"unsigned branch_{function}(unsigned x, unsigned y) {{\n"
        for bit in range(12):
            text += (f"if(x & {1 << bit}u) y=opaque(y+{bit + 1}u); "
                     f"else y=opaque(x^{bit + 17}u);\n")
        text += "return x+y; }\n"
    return text


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--before", type=Path, required=True)
    parser.add_argument("--after", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--functions", type=int, nargs="+", default=[40, 120])
    parser.add_argument("--reps", type=int, default=15)
    parser.add_argument("--warmup", type=int, default=2)
    parser.add_argument("--seed", type=int, default=20261024)
    parser.add_argument("--timeout", type=float, default=180)
    parser.add_argument("--callgrind", action="store_true")
    args = parser.parse_args()
    if (args.reps < 1 or args.warmup < 0 or not math.isfinite(args.timeout) or args.timeout <= 0
            or any(count < 1 for count in args.functions)):
        parser.error("positive repetitions, function counts and timeout are required")
    compilers = {name: getattr(args, name).resolve(strict=True)
                 for name in ("before", "after")}
    args.out.mkdir(parents=True, exist_ok=True)
    cpu = min(os.sched_getaffinity(0))
    env = {key: value for key, value in os.environ.items()
           if not key.startswith(("CCC_", "LCCC_"))}
    report = {"scope": "compiler cost, not generated-code runtime", "cpu": cpu,
              "seed": args.seed, "repetitions": args.reps, "warmup": args.warmup,
              "flags": ["-O1", "-S"], "rows": {},
              "binary_sha256": {name: digest(path) for name, path in compilers.items()}}

    def save() -> None:
        temporary = args.out / "report.json.tmp"
        temporary.write_text(json.dumps(report, indent=2) + "\n")
        temporary.replace(args.out / "report.json")

    for count in args.functions:
        input_file = args.out / f"branch-{count}.c"
        input_file.write_text(source(count))
        row = {"source_sha256": digest(input_file),
               "samples_s": {name: [] for name in compilers}, "assembly_sha256": {}}
        rng = random.Random(args.seed)
        for round_number in range(args.warmup + args.reps):
            order = list(compilers)
            rng.shuffle(order)
            for name in order:
                output = args.out / f"branch-{count}-{name}.s"
                command = ["taskset", "-c", str(cpu), str(compilers[name]),
                           "-O1", "-S", str(input_file), "-o", str(output)]
                start = time.perf_counter()
                run = subprocess.run(command, env=env, capture_output=True,
                                     check=True, timeout=args.timeout)
                elapsed = time.perf_counter() - start
                if run.stderr:
                    raise RuntimeError(run.stderr.decode(errors="replace"))
                sha = digest(output)
                previous = row["assembly_sha256"].setdefault(name, sha)
                if sha != previous:
                    raise RuntimeError(f"nondeterministic assembly: {name}")
                if round_number >= args.warmup:
                    row["samples_s"][name].append(elapsed)
            if len(set(row["assembly_sha256"].values())) != 1:
                raise RuntimeError(f"assembly changed for {count} functions")
            report["rows"][str(count)] = row
            save()
    if args.callgrind:
        count = args.functions[0]
        for name, compiler in compilers.items():
            profile = args.out / f"callgrind-{name}.out"
            with (args.out / f"callgrind-{name}.log").open("w") as log:
                subprocess.run(["taskset", "-c", str(cpu), "valgrind",
                                "--tool=callgrind", f"--callgrind-out-file={profile}",
                                str(compiler), "-O1", "-S",
                                str(args.out / f"branch-{count}.c"), "-o",
                                str(args.out / f"profile-{name}.s")], env=env,
                               stdout=subprocess.DEVNULL, stderr=log,
                               check=True, timeout=args.timeout * 4)
            total = next(line.split(":", 1)[1] for line in profile.read_text().splitlines()
                         if line.startswith("summary:"))
            report.setdefault("callgrind_Ir", {})[name] = int(total)
            save()
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
