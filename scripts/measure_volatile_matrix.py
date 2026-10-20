#!/usr/bin/env python3
"""Diagnostic short/long-trip and register-pressure matrix; no timing CI gates.

Shared objects provide an opaque ABI boundary (no cross-compiler inlining).
ADDRESS=1 uses Initial-Exec TLS, ADDRESS=2 uses the dynamic TLS model.
The driver validates every call and times only repeated kernel execution.
"""

import argparse
import itertools
import json
import os
from pathlib import Path
import tempfile
from measure_volatile_reductions import (
    ROOT,
    checked_run,
    positive,
    provenance,
    sha256,
    summary,
)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--candidate-revision", default="unspecified")
    parser.add_argument("--gcc", type=Path, required=True)
    parser.add_argument(
        "--previous",
        type=Path,
        help="optional previous candidate for paired measurements",
    )
    parser.add_argument("--clang", type=Path)
    parser.add_argument("--icx", type=Path)
    parser.add_argument("--samples", type=positive, default=5)
    parser.add_argument("--budget", type=positive, default=20000000)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument(
        "--quick",
        action="store_true",
        help="representative cases rather than full matrix",
    )
    args = parser.parse_args()
    cpu = min(os.sched_getaffinity(0))
    os.sched_setaffinity(0, {cpu})
    env = os.environ.copy()
    env.pop("CCC_DISABLE_PASSES", None)
    tools = {"candidate": args.candidate, "gcc": args.gcc}
    for name in ("previous", "clang", "icx"):
        if getattr(args, name):
            tools[name] = getattr(args, name)
    report = {
        "provenance": provenance(tools, env),
        "cpu": cpu,
        "samples": args.samples,
        "budget": args.budget,
        "warmups": 2,
        "cases": [],
        "sources": {},
        "status": "in_progress",
        "quick": args.quick,
        "candidate_revision": args.candidate_revision,
    }
    kernel = ROOT / "tests/benchmark/programs/volatile_reduce_matrix.c"
    driver_source = ROOT / "tests/benchmark/programs/volatile_reduce_driver.c"
    for source in (kernel, driver_source):
        report["sources"][source.name] = sha256(source)
    cases = list(itertools.product((32, 64), range(5), (0, 1), (0,)))
    cases += [
        (64, kind, address, pressure)
        for kind in (0, 1)
        for address, pressure in ((0, 1), (2, 0))
    ]
    if args.quick:
        cases = [
            (64, 0, 0, 0),
            (64, 1, 0, 0),
            (32, 4, 1, 0),
            (64, 1, 0, 1),
            (64, 1, 2, 0),
        ]
    counts = list(range(17)) + [32, 64, 256, 1024, 65536]
    report["planned_cases"] = len(cases)
    report["trip_counts"] = counts
    with tempfile.TemporaryDirectory(prefix="volatile-matrix-") as directory:
        work = Path(directory)
        driver = work / "driver"
        command = [
            str(args.gcc.resolve()),
            "-O3",
            "-Wall",
            "-Wextra",
            "-Werror",
            str(driver_source),
            "-ldl",
            "-o",
            str(driver),
        ]
        checked_run(command, env=env, timeout=120)
        report["driver"] = {"command": command, "sha256": sha256(driver)}
        for width, kind, address, pressure in cases:
            builds = {}
            for name, compiler in {
                "rolled": args.candidate,
                **tools,
                "two_chain": args.candidate,
            }.items():
                binary = work / f"{name}.so"
                command = [
                    str(compiler.resolve()),
                    "-O3",
                    "-Wall",
                    "-Wextra",
                    "-Werror",
                    "-fPIC",
                    "-shared",
                    f"-DWIDTH={width}",
                    f"-DKIND={kind}",
                    f"-DADDRESS={int(address != 0)}",
                    f"-DPRESSURE={pressure}",
                    f"-DMANUAL_LANES={2 if name == 'two_chain' else 0}",
                ]
                if address == 1:
                    command += ["-ftls-model=initial-exec"]
                command += [str(kernel), "-o", str(binary)]
                build_env = env | (
                    {"CCC_DISABLE_PASSES": "volatile_reduce"}
                    if name == "rolled"
                    else {}
                )
                checked_run(command, env=build_env, timeout=120)
                builds[name] = {
                    "path": binary,
                    "command": command,
                    "sha256": sha256(binary),
                    "disabled_passes": build_env.get("CCC_DISABLE_PASSES", ""),
                }
            case = {
                "width": width,
                "kind": kind,
                "address": address,
                "pressure": pressure,
                "builds": {
                    name: {k: v for k, v in build.items() if k != "path"}
                    for name, build in builds.items()
                },
                "counts": [],
            }
            for n in counts:
                calls = max(32, args.budget // max(1, n))
                samples = {name: [] for name in builds}
                names = list(builds)
                expected = None
                for round_index in range(args.samples + 2):
                    offset = round_index % len(names)
                    for name in names[offset:] + names[:offset]:
                        command = [
                            str(driver),
                            str(builds[name]["path"]),
                            str(n),
                            str(calls),
                            str(kind),
                            str(width),
                            str(pressure),
                            str(address),
                        ]
                        output = (
                            checked_run(command, env=env, timeout=60)
                            .stdout.decode()
                            .split()
                        )
                        if len(output) != 3:
                            raise ValueError(f"malformed driver output: {output}")
                        checksum, cpu_ns, wall_ns = output
                        if expected is None:
                            expected = checksum
                        if checksum != expected:
                            raise ValueError("cross-compiler checksum mismatch")
                        if round_index >= 2:
                            samples[name].append(
                                {
                                    "cpu": int(cpu_ns) / calls,
                                    "wall": int(wall_ns) / calls,
                                }
                            )
                result = {
                    name: {
                        metric: summary([s[metric] for s in values])
                        for metric in ("cpu", "wall")
                    }
                    for name, values in samples.items()
                }
                case["counts"].append(
                    {
                        "n": n,
                        "calls": calls,
                        "samples_ns_per_call": samples,
                        "summary_ns_per_call": result,
                    }
                )
            report["cases"].append(case)
            args.output.parent.mkdir(parents=True, exist_ok=True)
            temporary = args.output.with_suffix(args.output.suffix + ".tmp")
            temporary.write_text(json.dumps(report, indent=2) + "\n")
            temporary.replace(args.output)
            print(
                f"completed width={width} kind={kind} address={address} pressure={pressure}",
                flush=True,
            )

    report["status"] = "complete"
    temporary = args.output.with_suffix(args.output.suffix + ".tmp")
    temporary.write_text(json.dumps(report, indent=2) + "\n")
    temporary.replace(args.output)


if __name__ == "__main__":
    main()
