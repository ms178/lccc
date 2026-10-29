#!/usr/bin/env python3
"""Compare two full-link builds of the corpus by disassembled instruction count.

Excludes crt/libc scaffolding (identical in both arms) so the delta is the
program's own codegen.
"""
import re
import subprocess
import sys
from pathlib import Path

REPO = Path("/home/user/lccc")
PROGS = REPO / "tests/benchmark/programs"
LCCC = REPO / "target/fastbuild/lccc"

SKIP = {"_start", "_init", "_fini", "deregister_tm_clones", "register_tm_clones",
        "__do_global_dtors_aux", "frame_dummy", "__libc_csu_init",
        "__libc_csu_fini", "_dl_relocate_static_pie", "abort", "printf",
        "puts", "putchar", "__stack_chk_fail"}


def build(src, extra, out):
    r = subprocess.run([str(LCCC), "-O2", *extra, str(src), "-o", str(out)],
                       capture_output=True, text=True)
    return r.returncode == 0


def count(binary):
    d = subprocess.run(["objdump", "-d", str(binary)], capture_output=True, text=True)
    if d.returncode != 0:
        return None
    total = 0
    cur = None
    per = {}
    for line in d.stdout.splitlines():
        m = re.match(r"^[0-9a-f]+ <([^>]+)>:$", line)
        if m:
            cur = m.group(1)
            continue
        if not re.match(r"^\s+[0-9a-f]+:", line):
            continue
        if cur and (cur in SKIP or cur.startswith("__cxa") or "@plt" in cur):
            continue
        total += 1
        per[cur] = per.get(cur, 0) + 1
    return total, per


def main():
    rows = []
    for src in sorted(PROGS.glob("*.c")):
        a = Path("/tmp/cc_a")
        b = Path("/tmp/cc_b")
        if not build(src, [], a):
            continue
        if not build(src, ["-fpie"], b):
            continue
        ra, rb = count(a), count(b)
        if not ra or not rb:
            continue
        rows.append((src.stem, ra[0], rb[0]))
    ta = sum(r[1] for r in rows)
    tb = sum(r[2] for r in rows)
    d = [(r[1] - r[2], r[0], r[1], r[2]) for r in rows]
    d.sort()
    print(f"{'':26} {'new':>6} {'pie':>6} {'delta':>6}")
    for delta, name, x, y in d[:6]:
        print(f"{name:26} {x:6d} {y:6d} {delta:+6d}")
    print("  ...")
    for delta, name, x, y in d[-3:]:
        print(f"{name:26} {x:6d} {y:6d} {delta:+6d}")
    print(f"\nprograms: {len(rows)}   new={ta}  pie={tb}  delta={ta-tb:+d} "
          f"({100*(ta-tb)/max(tb,1):+.2f}%)")
    print(f"programs smaller with the new default: {sum(1 for r in d if r[0] < 0)}")
    print(f"programs larger:  {sum(1 for r in d if r[0] > 0)}")


if __name__ == "__main__":
    main()
