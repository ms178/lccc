#!/usr/bin/env python3
"""Paired separate-TU fixed-shuffle timings; never a universal speed claim."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import random
import statistics
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "tests/regression/shuffle_composition.c"
DRIVER = r'''
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
typedef void (*kernel)(const uint32_t*,const uint32_t*,uint32_t*);
extern void shuffle_0(const uint32_t*,const uint32_t*,uint32_t*);
extern void shuffle_1(const uint32_t*,const uint32_t*,uint32_t*);
extern void shuffle_16(const uint32_t*,const uint32_t*,uint32_t*);
extern void shuffle_17(const uint32_t*,const uint32_t*,uint32_t*);
extern void shuffle_23(const uint32_t*,const uint32_t*,uint32_t*);
int main(int argc,char**argv){
 if(argc!=3)return 2;
 kernel functions[]={shuffle_0,shuffle_1,shuffle_16,shuffle_17,shuffle_23};
 unsigned k=(unsigned)strtoul(argv[1],0,10);if(k>=5)return 3;
 uint64_t count=strtoull(argv[2],0,10),i;
 uint32_t a[4] __attribute__((aligned(16)))={0x12345678,0xffffffff,0x80000000,0x7fffffff};
 uint32_t b[4] __attribute__((aligned(16)))={0x95f165f1,0x73937771,0x2acdeb31,0x13373331};
 kernel fn=functions[k];
 for(i=0;i<count;i++)fn(a,b,a);
 printf("%08x %08x %08x %08x\n",a[0],a[1],a[2],a[3]);return 0;
}
'''


def run(command):
    return subprocess.run([str(x) for x in command], check=True, capture_output=True).stdout


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--baseline", required=True)
    p.add_argument("--compiler", default=str(ROOT / "target/fastbuild/lccc"))
    p.add_argument("--gcc", default="gcc-16")
    p.add_argument("--clang", required=True)
    p.add_argument("--iterations", type=int, default=67108864)
    p.add_argument("--pairs", type=int, default=15)
    p.add_argument("--json", required=True)
    args = p.parse_args()
    if args.iterations < 1 or args.pairs < 3:
        raise ValueError("positive iterations and at least three pairs required")
    affinity = sorted(os.sched_getaffinity(0))
    os.sched_setaffinity(0, {affinity[0]})
    tools = {"baseline": args.baseline, "candidate": args.compiler, "gcc": args.gcc, "clang": args.clang}
    result = {"method": "O2 SSE2, separate kernel/driver TUs, no LTO; out-of-line in-place maps with data dependence; two warmups, alternating paired order; subprocess wall time including startup; pinned CPU",
              "iterations": args.iterations, "pairs": args.pairs, "cpu_affinity": [affinity[0]],
              "platform": platform.platform(), "cpu": next(line.split(':', 1)[1].strip() for line in Path('/proc/cpuinfo').read_text().splitlines() if line.startswith('model name')),
              "source_sha256": digest(SOURCE), "compiler_sha256": {k: digest(v) for k, v in tools.items() if k in ("baseline", "candidate")},
              "tools": {k: run([v, "--version"]).decode().splitlines()[0] for k, v in tools.items()},
              "claims": "Only these kernels on this VM. Call/memory overhead is included; no whole-program or universal performance claim.", "kernels": []}
    with tempfile.TemporaryDirectory() as tmp:
        directory = Path(tmp)
        driver = directory / "driver.c"
        driver.write_text(DRIVER)
        driver_obj = directory / "driver.o"
        run([args.gcc, "-O2", "-msse2", "-fno-lto", "-c", driver, "-o", driver_obj])
        executables = {}
        for label, compiler in tools.items():
            obj = directory / f"{label}.o"
            exe = directory / label
            run([compiler, "-O2", "-msse2", "-DSHUFFLE_KERNELS_ONLY", "-c", SOURCE, "-o", obj])
            run([args.gcc, "-fno-lto", driver_obj, obj, "-o", exe])
            executables[label] = exe
        for index, name in enumerate(["inverse-permutations", "composed-permutations", "private-add", "private-sub", "shared-producer-control"]):
            samples = {k: [] for k in tools}
            expected = None
            def sample(label, record):
                nonlocal expected
                start = time.perf_counter_ns()
                output = run([executables[label], index, args.iterations])
                duration = (time.perf_counter_ns() - start) / 1e9
                if expected is not None and output != expected:
                    raise ValueError(f"runtime checksum mismatch: {name}/{label}")
                expected = output
                if record:
                    samples[label].append(duration)
            for _ in range(2):
                for label in tools:
                    sample(label, False)
            for pair in range(args.pairs):
                order = list(tools) if pair % 2 == 0 else list(reversed(tools))
                for label in order:
                    sample(label, True)
            ratios = [a / b for a, b in zip(samples['baseline'], samples['candidate'])]
            rng = random.Random(0)
            boot = sorted(statistics.median(rng.choices(ratios, k=len(ratios))) for _ in range(10000))
            medians = {k: statistics.median(v) for k, v in samples.items()}
            row = {"name": name, "checksum": expected.decode().strip(), "seconds": samples, "median_seconds": medians,
                   "paired_baseline_over_candidate_median": statistics.median(ratios), "paired_ratio_bootstrap_95pct": [boot[250], boot[9749]],
                   "gcc_over_candidate": medians['gcc']/medians['candidate'], "clang_over_candidate": medians['clang']/medians['candidate']}
            result['kernels'].append(row)
            Path(args.json).write_text(json.dumps(result, indent=2) + '\n')
            print(json.dumps({k: v for k, v in row.items() if k != 'seconds'}), flush=True)


if __name__ == "__main__":
    main()
