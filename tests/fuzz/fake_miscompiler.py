#!/usr/bin/env python3
"""Compiler stand-in for testing c_reduce.py (T4 mutation test).

Usage: fake_miscompiler.py LEVEL SRC -o OUT [-lm]

Compiles SRC with the reference compiler, but first rewrites every ``* 3`` to
``* 4`` when the environment variable FAKE_MISCOMPILE is "1".  This is a
deterministic, known-location miscompile, so the reducer must keep the
``* 3`` that matters and drop everything else.  With FAKE_MISCOMPILE unset it
is a correct compiler, which is the negative control.
"""
import os
import subprocess
import sys
import tempfile

REF = os.environ.get("REF_CC", "gcc")


def main(argv):
    src = argv[2]  # argv[1] is the optimisation level; ignored by this stand-in
    out = argv[argv.index("-o") + 1]
    with open(src, encoding="utf-8", errors="surrogateescape") as f:
        text = f.read()
    if os.environ.get("FAKE_MISCOMPILE") == "1":
        text = text.replace("* 3", "* 4")
    with tempfile.TemporaryDirectory() as tmp:
        cand = os.path.join(tmp, "m.c")
        with open(cand, "w", encoding="utf-8", errors="surrogateescape") as f:
            f.write(text)
        cmd = [REF, "-std=gnu11", "-O0", "-w", cand, "-o", out, "-lm"]
        return subprocess.call(cmd)


if __name__ == "__main__":
    sys.exit(main(sys.argv))
