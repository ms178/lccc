#!/usr/bin/env python3
"""Per-function instruction counts for the kernel corpus: LCCC vs system GCC.

Fast local proxy for the Compiler Explorer oracle (scripts/godbolt.py): it
compiles every kernel in tests/benchmark/kernel_corpus with both compilers at
-O2, counts the instructions inside the measured function body, and prints the
delta.  Official scoreboard numbers still come from godbolt.py; this harness
exists so a peephole change can be measured in seconds.

Usage:
    scripts/kernel_count.py                # whole corpus
    scripts/kernel_count.py adler crc      # substring filter on file names

Environment:
    LCCC   path to the lccc binary (default: target/fastbuild/lccc)
    GCC    path to the reference gcc  (default: gcc)
    CFLAGS extra flags appended to both compilers (default: none)
"""

import os
import re
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
KDIR = REPO / "tests" / "benchmark" / "kernel_corpus"
LCCC = os.environ.get("LCCC", str(REPO / "target" / "fastbuild" / "lccc"))
GCC = os.environ.get("GCC", "gcc")
EXTRA = os.environ.get("CFLAGS", "").split()

FUNCS = {
    "k01_adler.c": "adler8",
    "k02_sum8.c": "sum8",
    "k03_crc.c": "crc32k",
    "k04_strlen.c": "my_strlen",
    "k05_max.c": "maxv",
    "k06_dot.c": "dot",
    "k07_bswap.c": "bswp32",
    "k08_bcopy.c": "copy64",
    "k09_clz.c": "lz",
    "k10_ffs.c": "ffs1",
    "k11_swp.c": "swapmax",
    "k12_hash.c": "hsh",
    "k13_strcmp.c": "scmp",
    "k14_isort.c": "isort",
    "k15_bytemask.c": "cntz",
}


def body(text, name):
    """Text of the function body between `name:` and .size/.cfi_endproc."""
    pat = rf"(?ms)^{re.escape(name)}:\n(.*?)(?=^\s*\.size\s+{re.escape(name)}\b|^\s*\.cfi_endproc)"
    m = re.search(pat, text)
    return m.group(1) if m else None


def count(body_text):
    """Instructions only: labels, directives and comments do not count."""
    n = 0
    for line in body_text.splitlines():
        t = line.strip()
        if not t or t.endswith(":") or t.startswith(".") or t.startswith("#"):
            continue
        n += 1
    return n


def cc_count(cc, src, fn):
    out = f"/tmp/kc_{Path(src).stem}_{Path(cc).name}.s"
    try:
        r = subprocess.run(
            [cc, "-O2", *EXTRA, src, "-S", "-o", out],
            capture_output=True,
            timeout=120,
        )
    except (OSError, subprocess.TimeoutExpired) as exc:
        return None, str(exc)[:120]
    if r.returncode != 0:
        return None, r.stderr.decode(errors="replace").strip().splitlines()[-1][:120]
    b = body(Path(out).read_text(), fn)
    if b is None:
        return None, f"function {fn} not found in {out}"
    return count(b), ""


def main():
    only = sys.argv[1:] or None
    total_l = total_g = wins = 0
    print(f"{'kernel':<18}{'fn':<11}{'LCCC':>6}{'GCC':>6}{'delta':>7}  verdict")
    print("-" * 60)
    for f, fn in FUNCS.items():
        if only and not any(o in f for o in only):
            continue
        src = str(KDIR / f)
        l, el = cc_count(LCCC, src, fn)
        g, eg = cc_count(GCC, src, fn)
        if l is None or g is None:
            print(f"{f:<18}{fn:<11}  ERR {el or eg}")
            continue
        d = l - g
        verdict = "LCCC WINS" if d < 0 else ("tie" if d == 0 else f"+{d}")
        wins += d < 0
        total_l += l
        total_g += g
        print(f"{f:<18}{fn:<11}{l:>6}{g:>6}{d:>+7}  {verdict}")
    print("-" * 60)
    print(f"TOTAL: LCCC {total_l} vs GCC {total_g} (LCCC wins on {wins})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
