#!/usr/bin/env python3
"""Time symbol registration on links whose overriding definitions live in
COMDAT members -- the path that used to rescan every earlier object's groups
(quadratic in the object count; see linker_common::comdat::ObjectSet).

Object 0 defines weak w_1..w_N; object i (1..N) defines a strong w_i inside
its own COMDAT group plus G more groups, half of their signatures shared
across objects. Every registration of object i therefore asks whether its
definition's group was claimed earlier.

Usage: scripts/comdat_registration_bench.py [--lccc PATH] [--as PATH]
           [--groups G] [N ...]            (default N: 500 1000 2000 4000)
Prints best-of-3 link wall time per N; linear growth is the pass criterion.
"""
import argparse
import os
import subprocess
import sys
import tempfile
import time


def write_objects(d: str, n: int, g: int) -> None:
    with open(f"{d}/o0.s", "w") as f:
        f.write(".text\n.globl _start\n_start:\n  movl $60, %eax\n"
                "  xorl %edi, %edi\n  syscall\n")
        for i in range(1, n + 1):
            f.write(f".weak w_{i}\nw_{i}:\n  ret\n")
    for i in range(1, n + 1):
        with open(f"{d}/o{i}.s", "w") as f:
            for j in range(g):
                sig = f"shared_{j}" if j % 2 == 0 else f"u_{i}_{j}"
                f.write(f'.section .text.{sig},"axG",@progbits,{sig},comdat\n'
                        f".globl {sig}\n.weak {sig}\n{sig}:\n  ret\n")
            f.write(f'.section .text.w_{i},"axG",@progbits,gw_{i},comdat\n'
                    f".globl w_{i}\nw_{i}:\n  ret\n")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("sizes", nargs="*", type=int, default=[500, 1000, 2000, 4000])
    ap.add_argument("--lccc", default="target/fastbuild/lccc")
    ap.add_argument("--as", dest="gas", default="as")
    ap.add_argument("--groups", type=int, default=10)
    args = ap.parse_args()
    for n in args.sizes:
        with tempfile.TemporaryDirectory(prefix="comdat-bench-") as d:
            write_objects(d, n, args.groups)
            objs = []
            for i in range(n + 1):
                subprocess.run([args.gas, "--64", f"{d}/o{i}.s", "-o", f"{d}/o{i}.o"],
                               check=True)
                objs.append(f"{d}/o{i}.o")
            best = float("inf")
            for _ in range(3):
                t0 = time.perf_counter()
                subprocess.run([args.lccc, "-nostdlib", "-static", *objs,
                                "-o", os.path.join(d, "out")], check=True)
                best = min(best, time.perf_counter() - t0)
            print(f"N={n:5d} groups={args.groups}: best-of-3 {best:.3f} s", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
