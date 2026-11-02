#!/usr/bin/env python3
"""Native call-inclusive FMA recurrence A/B; never a correctness/perf CI gate.

Inputs are LCCC assembly snapshots of vector_ext/matrix.c, not executables.
The runner extracts fma_chain, checks equal outputs, alternates sample order,
pins one allowed CPU, and emits every sample plus robust summaries as JSON.
This measures this host and this dependency chain, not universal SIMD speed.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import statistics
import subprocess
import tempfile

DRIVER = r'''
#define _POSIX_C_SOURCE 200809L
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
typedef float V __attribute__((vector_size(32)));
extern V original(V,V,V,V);
extern V optimized(V,V,V,V);
static uint64_t ns(void) {
    struct timespec t;
    if (clock_gettime(CLOCK_MONOTONIC, &t)) exit(2);
    return (uint64_t)t.tv_sec * 1000000000u + (uint64_t)t.tv_nsec;
}
int main(int argc, char **argv) {
    if (argc != 3) return 2;
    unsigned n = (unsigned)strtoul(argv[2], 0, 10);
    V a={1,2,3,4,5,6,7,8}, b={-.5,-.5,-.5,-.5,-.5,-.5,-.5,-.5};
    V c={.125,.25,.375,.5,.625,.75,.875,1}, d={.5,.5,.5,.5,.5,.5,.5,.5};
    uint64_t start=ns();
    if (argv[1][0]=='o') for(unsigned i=0;i<n;i++) a=original(a,b,c,d);
    else for(unsigned i=0;i<n;i++) a=optimized(a,b,c,d);
    uint64_t elapsed=ns()-start;
    uint32_t bits[8]; memcpy(bits,&a,sizeof a);
    printf("%llu",(unsigned long long)elapsed);
    for(unsigned i=0;i<8;i++) printf(" %08x",bits[i]);
    puts("");
    return 0;
}
'''


def extract(path, name):
    lines = path.read_text().splitlines()
    start = lines.index("fma_chain:")
    end = next(i for i in range(start + 1, len(lines))
               if lines[i].strip().startswith(".size fma_chain,"))
    text = "\n".join(lines[start:end + 1])
    return f".globl {name}\n" + text.replace("fma_chain", name) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--before", type=Path, required=True)
    parser.add_argument("--after", type=Path, required=True)
    parser.add_argument("--cc", default="gcc")
    parser.add_argument("--samples", type=int, default=11)
    parser.add_argument("--iterations", type=int, default=5_000_000)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    if args.samples < 3 or not 1 <= args.iterations <= 1_000_000_000:
        parser.error("need >=3 samples and 1..1000000000 iterations")
    cpu_text = Path("/proc/cpuinfo").read_text()
    flags = next(line.split(":", 1)[1].split() for line in cpu_text.splitlines()
                 if line.startswith("flags"))
    if not {"avx2", "fma"} <= set(flags):
        parser.error("native measurement requires AVX2 and FMA")
    cpu = min(os.sched_getaffinity(0))
    os.sched_setaffinity(0, {cpu})
    rows = []
    with tempfile.TemporaryDirectory(prefix="packed-fma-ab-") as tmp:
        root = Path(tmp)
        (root / "driver.c").write_text(DRIVER)
        (root / "kernels.s").write_text(
            ".text\n" + extract(args.before, "original")
            + extract(args.after, "optimized")
            + '.section .note.GNU-stack,"",@progbits\n')
        subprocess.run([args.cc, "-O2", "-mavx2", "-mfma", "-Wall", "-Wextra",
                        "-Werror", str(root / "driver.c"), str(root / "kernels.s"),
                        "-o", str(root / "bench")], check=True, timeout=120)
        expected = None
        for sample in range(args.samples + 1):  # first pair is warmup
            row = {}
            for mode in (["original", "new"] if sample % 2 == 0 else ["new", "original"]):
                output = subprocess.check_output(
                    [str(root / "bench"), mode, str(args.iterations)],
                    text=True, timeout=120).split()
                checksum = output[1:]
                if expected is None:
                    expected = checksum
                if checksum != expected:
                    raise RuntimeError("A/B result mismatch")
                row[mode] = int(output[0])
            if sample:
                rows.append(row)
    ratios = [row["original"] / row["new"] for row in rows]
    median = statistics.median(ratios)
    report = {
        "metric": "call-inclusive dependent f32x8 FMA chain; not general throughput",
        "cpu_model": next(line.split(":", 1)[1].strip() for line in cpu_text.splitlines()
                          if line.startswith("model name")),
        "cpu": cpu, "iterations_per_sample": args.iterations,
        "before_sha256": hashlib.sha256(args.before.read_bytes()).hexdigest(),
        "after_sha256": hashlib.sha256(args.after.read_bytes()).hexdigest(),
        "before_kernel_sha256": hashlib.sha256(extract(args.before, "fma_chain").encode()).hexdigest(),
        "after_kernel_sha256": hashlib.sha256(extract(args.after, "fma_chain").encode()).hexdigest(),
        "driver_flags": ["-O2", "-mavx2", "-mfma", "-Wall", "-Wextra", "-Werror"],
        "compiler": subprocess.check_output([args.cc, "--version"], text=True).splitlines()[0],
        "samples_ns": rows, "median_before_over_after": median,
        "ratio_median_absolute_deviation": statistics.median(abs(r - median) for r in ratios),
        "min_ratio": min(ratios), "max_ratio": max(ratios),
        "caveat": "Virtualized single-host timing, no PMU evidence; no universal speed claim.",
    }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
