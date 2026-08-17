#!/usr/bin/env python3
"""Generate the `many_syms` microbenchmark used for linker symbol-path work.

This is the workload behind the only synthetic benchmark where lccc has
historically lost to wild and mold, so it is the reference for every
symbol-table optimisation. Keeping the generator in-tree (rather than
recreating it ad hoc each session) means the numbers quoted in
docs/linker/FOLLOWUP*.md are reproducible.

The link is deliberately *freestanding*: `_start` is defined locally and no
libc is referenced, so a run exercises object parsing, symbol registration and
symtab emission without dragging in dynamic-symbol resolution. Lesson learned
the hard way: a seed that needs libc makes every symbol resolve through the
shared-library path and hides the cost being measured.

Usage:
    tests/linker/gen_many_syms.py [--count N] [--outdir DIR]
    valgrind --tool=callgrind <linker> -o out bmain.o syms.o
"""
import argparse
import os
import subprocess
import sys


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--count", type=int, default=20000,
                    help="number of global functions to emit (default 20000)")
    ap.add_argument("--outdir", default="/tmp/ms")
    ap.add_argument("--cc", default=os.environ.get("CC", "gcc"))
    args = ap.parse_args()

    os.makedirs(args.outdir, exist_ok=True)
    syms_c = os.path.join(args.outdir, "syms.c")
    main_c = os.path.join(args.outdir, "bmain.c")

    with open(syms_c, "w") as f:
        for i in range(args.count):
            f.write("int g_sym_%d(void){return %d;}\n" % (i, i))

    # Freestanding entry point: exit(0) via raw syscall, no libc.
    with open(main_c, "w") as f:
        f.write(
            "void _start(void){\n"
            '    __asm__ volatile("mov $60,%eax\\n\\t"\n'
            '                     "xor %edi,%edi\\n\\t"\n'
            '                     "syscall");\n'
            "}\n"
        )

    for src, obj, extra in ((syms_c, "syms.o", []),
                            (main_c, "bmain.o", ["-ffreestanding"])):
        out = os.path.join(args.outdir, obj)
        cmd = [args.cc, "-c", "-O1"] + extra + [src, "-o", out]
        r = subprocess.run(cmd, capture_output=True)
        if r.returncode != 0:
            sys.stderr.write(r.stderr.decode())
            return 1
        print("%s  %d bytes" % (out, os.path.getsize(out)))

    print("\nlink with:\n  <linker> -o %s/out %s/bmain.o %s/syms.o"
          % (args.outdir, args.outdir, args.outdir))
    return 0


if __name__ == "__main__":
    sys.exit(main())
