#!/usr/bin/env python3
"""Own-object Ir from a Callgrind profile (dynamic instruction counts).

`callgrind_annotate` reports costs for every object in the process; for small
oracle kernels the dynamic loader and libc are 30-50% of the run, which both
dilutes and hides real codegen differences.  This wrapper keeps only the
functions that belong to one object (by default the profiled binary, which
Callgrind emits as an unnamed `ob=(1)` and `callgrind_annotate` resolves from
the `cmd:` header) and sums their exclusive Ir.

Usage:
  callgrind_own_ir.py cg.out [--object SUBSTR] [--top N] [--json]
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys

ROW = re.compile(r"^\s*([\d,]+(?:\s+[\d,]+)*)\s+(.*?)\s+\[([^\]]+)\]\s*$")


def annotate(path: str, show: str = "Ir") -> list[tuple[list[int], str, str]]:
    """Rows of (event counts, file:function, object) from the annotate table."""
    out = subprocess.run(
        ["callgrind_annotate", "--threshold=100", "--show-percs=no",
         f"--show={show}", path],
        capture_output=True, text=True, check=False,
    )
    if out.returncode != 0:
        sys.stderr.write(out.stderr)
        raise SystemExit(f"callgrind_annotate failed on {path}")
    rows: list[tuple[int, str, str]] = []
    in_table = False
    for line in out.stdout.splitlines():
        if "file:function" in line:
            in_table = True
            continue
        if not in_table:
            continue
        if set(line.strip()) == {"-"} or not line.strip():
            continue  # table separator; keep reading until auto-annotation
        if "Auto-annotated source" in line:
            break
        m = ROW.match(line)
        if m:
            counts = [int(x.replace(",", "")) for x in m.group(1).split()]
            rows.append((counts, m.group(2), m.group(3)))
    return rows


def pick_object(rows: list[tuple[list[int], str, str]], want: str | None) -> str:
    objs: dict[str, int] = {}
    for counts, _, obj in rows:
        objs[obj] = objs.get(obj, 0) + counts[0]
    if want:
        for obj in objs:
            if obj == want or obj.endswith(want) or want in obj:
                return obj
        sys.stderr.write(f"object {want!r} not found; have:\n")
        for obj, ir in sorted(objs.items(), key=lambda kv: -kv[1]):
            sys.stderr.write(f"  {ir:>12}  {obj}\n")
        raise SystemExit(1)
    syslib = ("/lib/", "ld-linux", "vgpreload", "/usr/lib/")
    own = [o for o in objs if not any(s in o for s in syslib)]
    if not own:
        raise SystemExit("no program object in profile: " + ", ".join(objs))
    # The profiled binary is whichever non-library object carries the most Ir.
    return max(own, key=lambda o: objs[o])


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("callgrind_out")
    ap.add_argument("--object", default=None, help="object path/substring")
    ap.add_argument("--top", type=int, default=12)
    ap.add_argument("--show", default="Ir",
                    help="events to show, e.g. Ir,D1mr,DLmr (requires "
                         "--cache-sim=yes in the run)")
    ap.add_argument("--json", action="store_true")
    args = ap.parse_args()

    rows = annotate(args.callgrind_out, args.show)
    if not rows:
        raise SystemExit(f"no function table in {args.callgrind_out}")
    events = [e.strip() for e in args.show.split(",") if e.strip()]
    obj = pick_object(rows, args.object)
    fns = [(counts, fn) for counts, fn, o in rows if o == obj]
    fns.sort(key=lambda kv: -kv[0][0])
    totals = [sum(c[i] if i < len(c) else 0 for c, _ in fns)
              for i in range(len(events))]
    if args.json:
        print(json.dumps({"object": obj, "totals": dict(zip(events, totals)),
                          "functions": [{"name": fn,
                                          **dict(zip(events, c))} for c, fn in fns[: args.top]]}))
    else:
        print(f"object: {obj}")
        print("own " + "  ".join(f"{e}: {t}" for e, t in zip(events, totals)))
        for c, fn in fns[: args.top]:
            pct = 100.0 * c[0] / totals[0] if totals[0] else 0.0
            print(f"  {c[0]:>12}  {pct:5.2f}%  {fn}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
