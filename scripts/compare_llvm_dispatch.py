#!/usr/bin/env python3
"""Paired dispatch experiment with explicit callee alignment and link-layout controls.

Not a CI speed threshold. All variants use the SAME GCC-built driver object.
Timing includes process startup. CPU affinity and raw samples are recorded.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import random
import resource
import statistics
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]


def run(args, env=None):
    p = subprocess.run(args, capture_output=True, text=True, check=True, env=env)
    if "warning:" in p.stderr.lower():
        raise RuntimeError(p.stderr)
    return p.stdout


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compiler", required=True)
    parser.add_argument("--before", required=True)
    parser.add_argument(
        "--reference", help="optional older compiler at default 16-byte alignment"
    )
    parser.add_argument(
        "--ablate-return-bool",
        action="store_true",
        help="also compile with only the new return_bool peephole disabled",
    )
    parser.add_argument("--gcc", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--suite", choices=("integer-observations", "bit-demand"), default="integer-observations")
    parser.add_argument("--rounds", type=int, default=8)
    parser.add_argument("--iterations", type=int, default=1500000)
    args = parser.parse_args()
    if args.rounds < 4 or args.iterations < 1:
        raise ValueError(
            "at least four paired rounds and positive iteration count required"
        )
    version = run([args.gcc, "--version"])
    if "16.2" not in version.splitlines()[0]:
        raise ValueError("exact GCC 16.2 required")
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=True)
    stem = (
        "llvm_integer_observations" if args.suite == "integer-observations" else "llvm_bit_demand"
    )
    source_path = ROOT / "tests/regression" / (stem + ".c")
    cases = json.loads(source_path.with_suffix(".json").read_text())
    names = [c["name"] for c in cases]
    guarded = {
        c["name"] for c in cases if c.get("nonzero_input", c.get("nonzero", False))
    }
    signature = (
        "uint32_t,uint32_t,uint32_t" if args.suite == "integer-observations" else "uint32_t,uint32_t"
    )
    third = ",i&3" if args.suite == "integer-observations" else ""
    declarations = "#include <stdint.h>\n#include <stdio.h>\n" + "".join(
        f"extern uint32_t {n}({signature});\n" for n in names
    )
    objects = {}
    variants = [
        (f"{label}{align}", compiler, align)
        for label, compiler in [("before", args.before), ("after", args.compiler)]
        for align in (16, 32, 64)
    ]
    if args.reference:
        variants.append(("reference16", args.reference, 16))
    if args.ablate_return_bool:
        variants.append(("no-return-bool16", args.compiler, 16))
    for name, compiler, align in variants:
        obj = out / (name + ".o")
        run(
            [
                compiler,
                "-O3",
                "-march=x86-64-v3",
                f"-falign-functions={align}",
                "-Dmain=fixtureMain",
                "-c",
                str(source_path),
                "-o",
                str(obj),
            ],
            env=os.environ
            | (
                {"CCC_PEEPHOLE_SKIP": "return_bool"}
                if name == "no-return-bool16"
                else {}
            ),
        )
        objects[name] = obj
    bins = {}
    hashes = {}
    for mode in ("direct", "indirect"):
        source = declarations
        if mode == "indirect":
            source += (
                f"typedef uint32_t (*fn)({signature});\nstatic fn const functions[]={{"
                + ",".join(names)
                + "};\n"
            )
        source += "int main(void){uint32_t x=1,y=3;uint64_t sum=0;\n"
        source += f"for(unsigned i=0;i<{args.iterations}u;++i){{x=x*1664525u+1013904223u;y=y*22695477u+1;\n"
        if mode == "indirect":
            guard = (
                "||".join(f"j=={i}" for i, n in enumerate(names) if n in guarded) or "0"
            )
            source += f"for(unsigned j=0;j<{len(names)};++j){{uint32_t input=(({guard})&&x==0)?1:x;sum+=functions[j](input,y{third});}}\n"
        else:
            for n in names:
                x = "(x?x:1)" if n in guarded else "x"
                source += f"sum+={n}({x},y{third});\n"
        source += '}printf("%016llx\\n",(unsigned long long)sum);return 0;}\n'
        path = out / (mode + ".c")
        path.write_text(source)
        hashes[mode] = hashlib.sha256(source.encode()).hexdigest()
        obj = out / (mode + ".o")
        run(
            [
                args.gcc,
                "-O3",
                "-march=x86-64-v3",
                "-Wall",
                "-Wextra",
                "-Werror",
                "-c",
                str(path),
                "-o",
                str(obj),
            ]
        )
        for pad in (0, 16, 32, 48):
            asm = out / f"pad{pad}.s"
            asm.write_text(
                f'.text\n.fill {pad},1,0x90\n.section .note.GNU-stack,"",@progbits\n'
            )
            pad_obj = asm.with_suffix(".o")
            run([args.gcc, "-c", str(asm), "-o", str(pad_obj)])
            for name, callee in objects.items():
                key = f"{mode}/{pad}/{name}"
                binary = out / f"{mode}-{pad}-{name}"
                run(
                    [
                        args.gcc,
                        "-no-pie",
                        str(pad_obj),
                        str(obj),
                        str(callee),
                        "-o",
                        str(binary),
                    ]
                )
                bins[key] = binary
    cpu = min(os.sched_getaffinity(0))
    samples = {k: [] for k in bins}
    cpu_samples = {k: [] for k in bins}
    groups = {}
    for key in bins:
        groups.setdefault(key.rsplit("/", 1)[0], []).append(key)
    blocks = list(groups)
    rng = random.Random(178)
    expected = None
    for repeat in range(args.rounds + 2):
        rng.shuffle(blocks)
        order = []
        for block in blocks:
            choices = groups[block].copy()
            rng.shuffle(choices)
            order.extend(choices)
        for key in order:
            usage = resource.getrusage(resource.RUSAGE_CHILDREN)
            cpu_start = usage.ru_utime + usage.ru_stime
            start = time.perf_counter_ns()
            result = subprocess.check_output(
                [str(bins[key])], preexec_fn=lambda: os.sched_setaffinity(0, {cpu})
            )
            elapsed = (time.perf_counter_ns() - start) / 1e9
            usage = resource.getrusage(resource.RUSAGE_CHILDREN)
            cpu_elapsed = usage.ru_utime + usage.ru_stime - cpu_start
            if expected is None:
                expected = result
            if result != expected:
                raise ValueError("checksum disagreement: " + key)
            if repeat >= 2:
                samples[key].append(elapsed)
                cpu_samples[key].append(cpu_elapsed)
    report = {
        "suite": args.suite,
        "reference": args.reference,
        "ablate_return_bool": args.ablate_return_bool,
        "callee_source_sha256": hashlib.sha256(source_path.read_bytes()).hexdigest(),
        "compiler_sha256": {
            label: hashlib.sha256(Path(path).read_bytes()).hexdigest()
            for label, path in [("before", args.before), ("after", args.compiler)]
            + ([("reference", args.reference)] if args.reference else [])
        },
        "cpu_info": Path("/proc/cpuinfo").read_text().split("\n\n")[0],
        "iterations": args.iterations,
        "rounds": args.rounds,
        "cpu_affinity": cpu,
        "gcc_version": version.splitlines()[0],
        "checksum": expected.decode().strip(),
        "driver_sha256": hashes,
        "seconds": samples,
        "cpu_seconds": cpu_samples,
        "median_cpu_seconds": {k: statistics.median(v) for k, v in cpu_samples.items()},
        "protocol": "seed-178 paired layout blocks; child user+system CPU time and external wall time; two warmups",
        "median_seconds": {k: statistics.median(v) for k, v in samples.items()},
        "limitations": "Shared host; startup included; no universal speed guarantee. Explicit alignment variants are experiments, not compiler-default changes.",
    }
    (out / "results.json").write_text(json.dumps(report, indent=2) + "\n")
    for k, median in report["median_seconds"].items():
        print(k, f"{median:.6f}")


if __name__ == "__main__":
    main()
