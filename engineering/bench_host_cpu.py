#!/usr/bin/env python3
"""Host-CPU differential correctness + runtime benchmark for the PR tree.

Research tool (NOT a CI gate): it needs the pinned GCC 16.2 / Clang 23.1.5
oracles from scripts/restore_agent_toolchain_cache.py, which hosted CI only
installs in its Debian lanes.  It compares FOUR legs:

  * gcc    -- GCC 16.2.0, -O2
  * clang  -- Clang 23.1.5, -O2
  * lccc_old -- pristine origin/main (the A/B baseline leg), -O2
  * lccc_new -- the PR tree, -O2

Correctness is a hard gate: every leg must print the SAME checksum for each
kernel.  Only then are the timings reported.  Three kernels exercise distinct
hardware paths: FP multiply-add throughput (matmul), integer dependency chains
(LCG), and a branchy byte workload (classify).

ICX 2026.2 is intentionally absent: see engineering/ICX-LOCAL-FEASIBILITY.md
for the exhaustive, observed provisioning failures in this sandbox.  Hosted
CI's authenticated icx-oracle lane is the only available third oracle, and no
local number here is attributed to it.

Usage: python3 engineering/bench_host_cpu.py \
    [--old <fastbuild-dir-or-binary>] [--new <fastbuild-dir-or-binary>] \
    [--out report.json]

The two LCCC legs are explicit (or LCCC_AB_OLD / LCCC_AB_NEW) and fail closed:
a stale baseline path is never silently substituted for the one measured.
"""
from __future__ import annotations

import argparse
import json
import pathlib
import statistics
import subprocess
import sys
import tempfile
import time

from compiler_legs import add_leg_arguments, resolve

CACHE = pathlib.Path("/home/user/.cache/lccc/agent-toolchains/bin")
COMPILERS = {
    "gcc": str(CACHE / "gcc-16"),
    "clang": str(CACHE / "clang-23"),
    "lccc_old": "",   # bound in main() from --old, fail-closed
    "lccc_new": "",   # bound in main() from --new, fail-closed
}
OPT = "-O2"

KERNELS = {
    "matmul": r"""
#include <stdio.h>
#include <stdlib.h>
#define N 64
static double A[N][N], B[N][N], C[N][N];
int main(int argc, char **argv) {
  int iters = argc > 1 ? atoi(argv[1]) : 200;
  for (int i = 0; i < N; i++) for (int j = 0; j < N; j++) {
    A[i][j] = (double)((i * 7 + j * 13) % 97) / 8.0;
    B[i][j] = (double)((i * 3 + j * 11) % 89) / 4.0;
  }
  double sum = 0.0;
  for (int it = 0; it < iters; it++) {
    for (int i = 0; i < N; i++) for (int j = 0; j < N; j++) {
      double s = 0.0;
      for (int k = 0; k < N; k++) s += A[i][k] * B[k][j];
      C[i][j] = s + (double)(it & 1);
    }
    sum += C[it % N][(it * 3) % N];
  }
  printf("%.6f\n", sum);
  return 0;
}
""",
    "lcg": r"""
#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
int main(int argc, char **argv) {
  int n = argc > 1 ? atoi(argv[1]) : 2000;
  uint64_t s = 0x123456789abcdefULL;
  uint64_t h = 0;
  for (int i = 0; i < n; i++) {
    s = s * 6364136223846793005ULL + 1442695040888963407ULL;
    h ^= s + (h << 6) + (h >> 2);
  }
  printf("%llu %llu\n", (unsigned long long)s, (unsigned long long)h);
  return 0;
}
""",
    "classify": r"""
#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#define M 4096
static unsigned char buf[M];
int main(int argc, char **argv) {
  int n = argc > 1 ? atoi(argv[1]) : 4000;
  uint32_t x = 0x9e3779b9u;
  for (int i = 0; i < M; i++) {
    x = x * 1664525u + 1013904223u;
    buf[i] = (unsigned char)(x >> 24);
  }
  uint64_t a = 0, b = 0;
  for (int it = 0; it < n; it++) {
    for (int i = 0; i < M; i++) {
      unsigned char c = buf[i] ^ (unsigned char)(it + i);
      if (c & 1) a += c * 3u; else if (c & 2) b += c * 5u; else a ^= c;
    }
  }
  printf("%llu %llu\n", (unsigned long long)a, (unsigned long long)b);
  return 0;
}
""",
}
ITERS = {"matmul": 200, "lcg": 200000, "classify": 4000}


def main() -> int:
    ap = argparse.ArgumentParser(description="host-CPU four-leg differential bench")
    add_leg_arguments(ap)
    ap.add_argument("--out", type=pathlib.Path,
                    default=pathlib.Path("/home/user/artifacts/host_cpu_final.json"))
    a = ap.parse_args()
    COMPILERS["lccc_old"], COMPILERS["lccc_new"] = resolve(a.old, a.new)
    missing = [k for k, v in COMPILERS.items()
               if not v or not pathlib.Path(v).exists()]
    if missing:
        print(f"MISSING compilers: {missing}; restore the pinned toolchain cache",
              file=sys.stderr)
        return 2
    out: dict = {"opt": OPT, "kernels": {}, "iters": ITERS,
                 "legs": {k: COMPILERS[k] for k in ("lccc_old", "lccc_new")}}
    with tempfile.TemporaryDirectory(prefix="lccc-host-") as td:
        d = pathlib.Path(td)
        for kernel, src in KERNELS.items():
            cfile = d / f"{kernel}.c"
            cfile.write_text(src)
            bins = {}
            for name, cc in COMPILERS.items():
                exe = d / f"{kernel}-{name}"
                r = subprocess.run([cc, OPT, str(cfile), "-o", str(exe)],
                                   capture_output=True)
                if r.returncode != 0:
                    print(f"COMPILE FAIL {name} {kernel}: "
                          f"{r.stderr.decode('utf-8', 'replace')[-300:]}",
                          file=sys.stderr)
                    return 1
                bins[name] = exe
            # Correctness: every leg must agree, including old vs new.
            checks = {}
            for name, exe in bins.items():
                p = subprocess.run([str(exe), str(ITERS[kernel])],
                                   capture_output=True, timeout=120)
                checks[name] = p.stdout.strip().decode("utf-8", "replace")
            reference = checks["gcc"]
            mismatches = [k for k, v in checks.items() if v != reference]
            if mismatches:
                print(f"CORRECTNESS FAIL {kernel}: {mismatches} != gcc"
                      f" ({reference!r})", file=sys.stderr)
                print(checks, file=sys.stderr)
                return 1
            times = {}
            for name, exe in bins.items():
                samples = []
                for _ in range(5):
                    t = time.monotonic()
                    p = subprocess.run([str(exe), str(ITERS[kernel])],
                                       capture_output=True, timeout=120)
                    samples.append(time.monotonic() - t)
                    assert p.returncode == 0
                times[name] = statistics.median(samples)
            out["kernels"][kernel] = {
                "checksum": reference,
                "median_seconds": times,
                "lccc_new_vs_gcc": round(times["lccc_new"] / times["gcc"], 3),
                "lccc_new_vs_clang": round(times["lccc_new"] / times["clang"], 3),
                "lccc_new_vs_old": round(times["lccc_new"] / times["lccc_old"], 3),
            }
            print(f"{kernel:9s} checksum={reference!r} "
                  + " ".join(f"{k}={v:.6f}s" for k, v in times.items()),
                  flush=True)
    path = a.out
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(out, indent=2) + "\n")
    print(f"HOST_CPU_OK -> {path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
