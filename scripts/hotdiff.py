#!/usr/bin/env python3
"""Deterministic hot-instruction differential: LCCC vs a reference compiler.

The hardware-counter report from the 14700KF was WALL-CLOCK-FALLBACK with a
`powersave` governor, perf_event_paranoid=2, and 11 of 63 benchmarks above 5%
coefficient of variation -- so it can rank benchmarks but it cannot say which
instruction is responsible.  This tool answers the second question with data
that has no noise in it at all: Valgrind's callgrind counts executed
instructions exactly, so two builds of the same program can be compared
instruction by instruction and the difference attributed to a mnemonic, a
function, and an address.

For each benchmark it compiles with each compiler (using the corpus' own
per-benchmark flags, and `-no-pie` so the profile's absolute instruction
addresses match `objdump`'s), runs the instrumented binary under callgrind
with `--dump-instr=yes`, and reports:

  * executed-instruction ratio,
  * the mnemonics LCCC executes more of than the reference, ranked by the
    extra instructions they cost,
  * the mnemonics the reference executes and LCCC never does -- a *missing
    lowering*, which is the fingerprint of an unmodelled instruction class,
  * the hottest functions, so the finding points at a compiler pass.

Nothing here needs hardware counters, so the ranking is reproducible on a
shared box with a powersave governor.

Usage:
  scripts/hotdiff.py --bench byte_load_reassembly
  scripts/hotdiff.py --bench zlib_ng_slide_hash,sqlite_varint --top 12
  scripts/hotdiff.py --bench sqlite_varint --json /tmp/hot.json --keep
"""

from __future__ import annotations

import argparse
import collections
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
PROGRAMS = REPO / "tests" / "benchmark" / "programs"

# Per-benchmark extra flags, mirrored from tests/benchmark/run_benchmarks.py so a
# diff here measures the same source build the performance report measured.
EXTRA = {
    "f32_sse1_map": ["-mno-sse2", "-msse", "-ffp-contract=off", "-DSSE1_MAP_REQUIRE_ONLY"],
    "fp32_product_map": ["-ffp-contract=off"],
    "fp64_product_map": ["-DFP_MAP_DOUBLE", "-ffp-contract=off"],
}

FUNCTION_RE = re.compile(r"^fn=(?:\((\d+)\))?\s*(.*)$")
SUMMARY_RE = re.compile(r"^summary:\s*(\d+)", re.M)
# callgrind writes `positions: instr line` + `events: Ir` for this profile, so an
# instruction line is `<addr-spec> <line> <cost>...`.  The address spec is either
# absolute (`0x401530`) or a delta whose sign carries its magnitude -- `+4`, not
# `+ 4`; a sign followed by whitespace is not a thing callgrind emits, and a
# pattern that required one silently dropped most of the profile.
# The source-line field is itself a spec: `*` for "no line", `-1` for "unknown",
# or a number/delta.  A pattern that only accepted `*` and digits fell through
# to reading the line field as the cost, which is where a run of negative
# "costs" came from.
LINE_FIELD = r"\*|[+\-]?\d+"
INSTR_LINE = re.compile(r"^(0x[0-9a-fA-F]+|[+\-*]\d*)(?:\s+(%s))?\s+(.*)$" % LINE_FIELD)


def corpus_flags(name: str) -> list[str]:
    return list(EXTRA.get(name, []))


def sh(cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, **kw)


def addr_map(program: Path) -> tuple[dict[int, str], dict[int, str]]:
    """instruction address -> mnemonic, and -> full instruction text."""
    objdump = shutil.which("objdump") or "objdump"
    r = sh([objdump, "-d", "--no-show-raw-insn", str(program)])
    mnem: dict[int, str] = {}
    text: dict[int, str] = {}
    for line in r.stdout.splitlines():
        m = re.match(r"^\s+([0-9a-f]+):\s+([a-z][a-z0-9.]*)(.*)$", line)
        if m:
            a = int(m.group(1), 16)
            mnem[a] = m.group(2)
            text[a] = (m.group(2) + m.group(3)).strip()
    return mnem, text


def parse_profile(points: Path) -> tuple[dict[int, int], dict[str, int], int]:
    """Return (address -> Ir, function -> Ir, total Ir).

    Handles callgrind's cost-line grammar: the first field addresses the
    instruction (absolute `0x..`, or a `+N`/`-N`/`*` delta from the previous
    one), the second is the source line, and the remaining fields are the
    per-event costs in the `events:` order (Ir only here).
    """
    by_addr: dict[int, int] = collections.defaultdict(int)
    by_fn: dict[str, int] = collections.defaultdict(int)
    total = 0
    events = 1
    cur_fn = "<unknown>"
    cur_addr = 0
    # A cost line immediately following `calls=` is the *inclusive* cost of the
    # call site: it repeats the callee's own cost, which is also recorded in the
    # callee's own `fn=` block.  Counting both double-counts the profile (it
    # reported printf's whole inclusive cost a second time on top of main's
    # loop).  The line must be consumed and discarded.
    skip_next_cost = False
    # Only addresses spelled absolutely belong to the main executable.  A
    # shared library's function reports its first instruction as `+0`, and
    # accumulating those deltas produces small integers that collide with real
    # executable addresses -- which is how an earlier version reported 111 M
    # `push` for a loop containing none.  Attribution therefore requires the
    # address to have come from an absolute `0x...` position; relative-only
    # functions still contribute to the totals and to the per-function view.
    absolute = False
    # A cost line immediately following `calls=` is the *inclusive* cost of the
    # call site: it repeats the callee's own cost, which is also recorded in the
    # callee's own `fn=` block.  Counting both double-counts the profile (it
    # reported printf's whole inclusive cost a second time on top of main's
    # loop).  The line must be consumed and discarded.
    skip_next_cost = False
    for line in points.read_text(errors="replace").splitlines():
        if line.startswith("events:"):
            events = max(1, len(line[7:].split()) - 1)
            continue
        if line.startswith("fn="):
            m = FUNCTION_RE.match(line)
            cur_fn = (m.group(2) or "").strip() or cur_fn
            cur_addr = 0
            absolute = False
            continue
        if line.startswith("calls="):
            skip_next_cost = True
            continue
        if line.startswith(("cfn=", "jump=", "jcnd=", "ob=", "fl=", "fi=",
                            "fe=", "cfl=", "cfi=", "cfe=", "cob=")):
            continue
        if not line or line[0] not in "0123456789+-*":
            continue
        m = INSTR_LINE.match(line)
        if not m:
            continue
        if skip_next_cost:
            skip_next_cost = False
            continue
        spec, _lineno, rest = m.group(1), m.group(2), m.group(3)
        if spec == "*":
            pass
        elif spec.startswith("0x"):
            cur_addr = int(spec, 16)
            absolute = True
        elif spec[0] in "+-" and len(spec) > 1:
            cur_addr += int(spec)
        costs = []
        for tok in rest.split():
            try:
                costs.append(int(tok))
            except ValueError:
                costs = []
                break
        if not costs:
            continue
        cost = costs[0]
        total += cost
        by_fn[cur_fn] += cost
        if cur_addr and absolute:
            by_addr[cur_addr] += cost
    summary = 0
    m = SUMMARY_RE.search(points.read_text(errors="replace"))
    if m:
        summary = int(m.group(1))
    if summary and total != summary:
        raise SystemExit(
            f"hotdiff: profile parser disagrees with callgrind's own summary "
            f"({total:,} vs {summary:,}); refusing to report a ratio")
    return dict(by_addr), dict(by_fn), total


def build(cc: str, src: Path, out: Path, flags: list[str]) -> tuple[bool, str]:
    # `-no-pie` keeps the profile's instruction addresses equal to objdump's;
    # it is an analysis-only build and is applied identically to both compilers.
    r = sh([cc, "-O2", "-no-pie", "-o", str(out), str(src), *flags])
    return r.returncode == 0, (r.stderr or "")[:400]


def analyse(name: str, lccc: str, ref: str, workdir: Path) -> dict:
    src = PROGRAMS / f"{name}.c"
    if not src.exists():
        return {"bench": name, "error": f"no such benchmark: {src}"}
    flags = corpus_flags(name)
    rec: dict = {"bench": name, "flags": flags, "mnemonic": {}, "functions": {}, "total": {}}

    progs: dict[str, Path] = {}
    for tag, cc in (("lccc", lccc), ("ref", ref)):
        out = workdir / f"{name}.{tag}"
        ok, err = build(cc, src, out, flags)
        if not ok:
            rec["error"] = f"{tag} compile failed: {err}"
            return rec
        progs[tag] = out

    for tag, prog in progs.items():
        points = workdir / f"{name}.{tag}.cg"
        # `--toggle-collect=main` drops the dynamic-loader startup, which for a
        # small benchmark is tens of millions of instructions of ld.so symbol
        # lookup and would otherwise dominate both the totals and the call
        # attribution.  Both builds get the same treatment.
        r = sh(["valgrind", "--tool=callgrind", "--dump-instr=yes",
                "--cache-sim=no", "--branch-sim=no", "--quiet",
                "--toggle-collect=main",
                "--callgrind-out-file=" + str(points), str(prog)], cwd=workdir)
        if r.returncode != 0:
            rec["error"] = f"{tag} run failed (rc={r.returncode}): {(r.stderr or '')[:300]}"
            return rec
        by_addr, by_fn, total = parse_profile(points)
        mapping, inst_text = addr_map(prog)
        rec.setdefault("hot", {})[tag] = [
            (a, c) for a, c in sorted(by_addr.items(), key=lambda kv: -kv[1])]
        rec.setdefault("text", {})[tag] = inst_text
        hist: dict[str, int] = collections.defaultdict(int)
        for a, c in by_addr.items():
            mn = mapping.get(a)
            if mn:
                hist[mn] += c
        rec["mnemonic"][tag] = dict(hist)
        rec["functions"][tag] = by_fn
        rec["total"][tag] = total
    return rec


def report(rec: dict, top: int, show_asm: int = 0) -> None:
    name = rec["bench"]
    if "error" in rec:
        print(f"\n=== {name}: {rec['error']}")
        return
    L, R = rec["mnemonic"]["lccc"], rec["mnemonic"]["ref"]
    tl, tr = rec["total"]["lccc"], rec["total"]["ref"]
    print(f"\n=== {name}  flags={' '.join(rec['flags']) or '(none)'} ===")
    print(f"  executed instructions: lccc {tl:,}  ref {tr:,}  ratio {tl / tr:.3f}")
    keys = set(L) | set(R)
    excess = sorted(((L.get(k, 0) - R.get(k, 0), k) for k in keys), reverse=True)
    print(f"  {'mnemonic':<12}{'lccc':>14}{'ref':>14}{'delta':>14}")
    for d, k in excess:
        if d <= 0 or len([1]) > top:
            break
    shown = 0
    for d, k in excess:
        if d <= 0 or shown >= top:
            break
        print(f"  {k:<12}{L.get(k, 0):>14,}{R.get(k, 0):>14,}{d:>+14,}")
        shown += 1
    missing = [(R[k], k) for k in keys if R.get(k, 0) > 0 and L.get(k, 0) == 0]
    missing.sort(reverse=True)
    if missing:
        print("  reference-only (lccc never executes; missing lowering):")
        for c, k in missing[:top]:
            print(f"  {k:<12}{0:>14,}{c:>14,}")
    if show_asm:
        text = rec["text"]["lccc"]
        reftext = rec["text"]["ref"]
        print(f"  hottest instructions (lccc, self Ir) vs ref:")
        for a, c in rec["hot"]["lccc"][:show_asm]:
            print(f"  {c:>13,}  0x{a:x}  {text.get(a, '?'):<40}")
        print(f"  hottest instructions (ref, self Ir):")
        for a, c in rec["hot"]["ref"][:show_asm]:
            print(f"  {c:>13,}  0x{a:x}  {reftext.get(a, '?'):<40}")
    print("  hottest functions (lccc):")
    for fn, c in sorted(rec["functions"]["lccc"].items(), key=lambda kv: -kv[1])[:6]:
        print(f"  {c:>14,}  {fn[:56]}")


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--bench", required=True, help="comma-separated benchmark basenames")
    ap.add_argument("--lccc", default=str(REPO / "target/fastbuild/lccc"))
    ap.add_argument("--ref", default="gcc-16")
    ap.add_argument("--top", type=int, default=10)
    ap.add_argument("--asm", type=int, default=0, metavar="N",
                    help="also list the N hottest instructions per compiler")
    ap.add_argument("--json", type=Path)
    ap.add_argument("--keep", action="store_true")
    args = ap.parse_args(argv)

    ref = shutil.which(args.ref) or args.ref
    if not (os.path.exists(ref) or shutil.which(args.ref)):
        print(f"reference compiler not found: {args.ref}", file=sys.stderr)
        return 2
    if not os.path.exists(args.lccc):
        print(f"lccc not built: {args.lccc}", file=sys.stderr)
        return 2

    results = []
    for bench in [b.strip() for b in args.bench.split(",") if b.strip()]:
        workdir = Path(tempfile.mkdtemp(prefix=f"hotdiff.{bench}."))
        try:
            rec = analyse(bench, args.lccc, args.ref, workdir)
            results.append(rec)
            report(rec, args.top, args.asm)
        finally:
            if not args.keep:
                shutil.rmtree(workdir, ignore_errors=True)
            else:
                print(f"  (kept {workdir})")

    if args.json:
        args.json.write_text(json.dumps(results, indent=1))
        print(f"\nwrote {args.json}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
