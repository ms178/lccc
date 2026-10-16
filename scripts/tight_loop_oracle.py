#!/usr/bin/env python3
"""Tight-loop alignment census vs the Godbolt oracles.

For every benchmark program, extract each backward-branch label (a loop
header) and the strongest UNCONDITIONAL alignment in the directive group
immediately above it, then compare:

  * GCC 16.2 / Clang 23.1 / ICX on Godbolt (`-O2 -march=raptorlake`,
    directives kept); GCC emits the tight-loop group
    `.p2align K` + `.p2align 4,,10` + `.p2align 3` where
    K = ceil_log2(minimum encoded body size).
  * lccc's integrated assembler decision (`CCC_DEBUG_TIGHT`):
    K = ceil_log2(EXACT encoded body span), clamped to 3..=6, with
    bodies over one cache line rejected to the ordinary cascade.

This is a directive histogram, not a matched-loop or performance proof.
Different compilers may remove, clone, or vectorize different loops. Aggregate
bucket counts cannot establish per-loop equivalence or a hardware speedup.
Compiler/source failures make the survey incomplete and return nonzero; partial
diagnostics are retained. Both local modes and vendors receive the same ISA.

Usage: scripts/tight_loop_oracle.py LCCC --opt=-O2 --oracles gcc16.2,clang,icx [bench...]
"""
import argparse
import hashlib
import json
import shlex
import os
import re
import subprocess
import sys
import tempfile
from collections import Counter
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
PROGRAMS = REPO / "tests/benchmark/programs"

sys.path.insert(0, str(REPO / "scripts"))
import godbolt as gb  # noqa: E402

LABEL_RE = re.compile(r"^([.\w]+):\s*$")
JMP_RE = re.compile(r"^\s*(?:jmp|j[a-z]{1,3}|loop[a-z]*)\s+([.\w]+)\b")
P2A_RE = re.compile(r"\.p2align\s+(\d+)(?:\s*,[^,\n#]*(?:,\s*([^#\n]+))?)?")
FUNC_RE = re.compile(r"^([A-Za-z_][\w.$]*):\s*$")
DUMP_RE = re.compile(r"loop_align: func=(\S+) header=L(\d+) tight=yes")
OK_RE = re.compile(r"\[TIGHT\] sec\d+ header=(\.\S+) span=(\d+) log2=(\d+)")
NO_RE = re.compile(r"\[TIGHT\] sec\d+ header=(\.\S+) reject=(\S+)")


def alignment_group(lines, label_index):
    """Parse only contiguous non-byte-emitting directives before this label."""
    best, bounded = None, False
    for line in reversed(lines[:label_index]):
        text = line.strip()
        match = P2A_RE.match(text)
        if match:
            exponent, max_skip = match.groups()
            if max_skip is None:
                best = max(best, int(exponent)) if best is not None else int(exponent)
            else:
                bounded = True
        elif not text or text.startswith(("#", "//", ".cfi_", ".loc ")):
            continue
        else:
            break
    return best if best is not None else ("cascade" if bounded else None)


def oracle_headers(lines):
    """label -> (function, unconditional log2, bounded cascade, or None)."""
    label_line, func_of = {}, {}
    cur = None
    for i, l in enumerate(lines):
        m = FUNC_RE.match(l.strip())
        if m and not l.startswith("."):
            cur = m.group(1)
        m = LABEL_RE.match(l.strip())
        if m:
            label_line[m.group(1)] = i
            func_of[m.group(1)] = cur
    headers = {}
    for i, l in enumerate(lines):
        m = JMP_RE.match(l)
        if m:
            tgt = m.group(1)
            if tgt in label_line and label_line[tgt] < i:
                headers[tgt] = label_line[tgt]
    out = {}
    for lab, li in headers.items():
        bucket = alignment_group(lines, li)
        out[lab] = (func_of.get(lab), bucket)
    return out


def directive_groups(lines):
    """Strongest UNCONDITIONAL .p2align log2 above each backward-branch
    label in lccc -S text ('cascade' for bounded groups only)."""
    label_line, headers = {}, {}
    for i, l in enumerate(lines):
        m = re.match(r"^(\.\w+):", l.strip())
        if m:
            label_line[m.group(1)] = i
    for i, l in enumerate(lines):
        m = JMP_RE.match(l)
        if m and m.group(1) in label_line and label_line[m.group(1)] < i:
            headers[m.group(1)] = label_line[m.group(1)]
    out = {}
    for lab, li in headers.items():
        out[lab] = alignment_group(lines, li)
    return out


def lccc_decisions(lccc, flags, src, include):
    """Histogram of effective strongest log2 for every loop header in
    integrated mode: tight accepts from CCC_DEBUG_TIGHT override the
    portable cascade groups parsed from -S text; plus reject reasons."""
    with tempfile.TemporaryDirectory() as td:
        # 1. portable text: cascade groups per header
        sfile = Path(td) / "p.s"
        r = subprocess.run([str(lccc), "-I" + include, *shlex.split(flags), "-S", str(src), "-o", str(sfile)],
                           capture_output=True, text=True, timeout=60)
        if r.returncode:
            return None, None, r.stderr[-300:]
        groups = directive_groups(sfile.read_text(errors="replace").splitlines())
        # 2. integrated decision: tight accepts/rejects
        env = dict(os.environ)
        env["CCC_DEBUG_TIGHT"] = "1"
        env["CCC_DUMP_ALIGN"] = "1"
        obj = Path(td) / "p.o"
        r2 = subprocess.run([str(lccc), "-I" + include, *shlex.split(flags), "-c", str(src), "-o", str(obj)],
                            capture_output=True, text=True, env=env, timeout=60)
        if r2.returncode:
            return None, None, r2.stderr[-300:]
        accepted, rejected = {}, {}
        for line in r2.stderr.splitlines():
            m = OK_RE.search(line)
            if m:
                accepted[m.group(1)] = (int(m.group(2)), int(m.group(3)))
                rejected.pop(m.group(1), None)
                continue
            m = NO_RE.search(line)
            if m and m.group(1) not in accepted:
                rejected[m.group(1)] = m.group(2)
    effective = {}
    for lab, bucket in groups.items():
        effective[lab] = accepted[lab][1] if lab in accepted else (
            bucket if bucket is not None else "unaligned")
    # Tight labels can exist only in -c debug; ensure every accept is in.
    for lab, (_, k) in accepted.items():
        effective[lab] = k
    return effective, rejected, None


def bucket_name(value):
    return f"2^{value}" if isinstance(value, int) else value


def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument("lccc", type=Path)
    ap.add_argument("--opt", default="-O2")
    ap.add_argument("--oracles", default="gcc16.2,clang,icx")
    ap.add_argument("--march", default="raptorlake")
    ap.add_argument("--gcc", default="gcc-16", help="builtin-header provider")
    ap.add_argument("--json", type=Path, help="coverage, flags and error receipt")
    ap.add_argument("programs", nargs="*")
    args = ap.parse_intermixed_args(argv)
    names = args.programs or sorted(p.stem for p in PROGRAMS.glob("*.c"))
    oracles = [o.strip() for o in args.oracles.split(",") if o.strip()]
    if not names or len(names) != len(set(names)) or not oracles or len(oracles) != len(set(oracles)):
        ap.error("nonempty unique program and oracle selections required")
    include = subprocess.check_output([args.gcc, "-print-file-name=include"], text=True, timeout=15).strip()
    if not include or not Path(include).is_dir():
        ap.error("GCC did not identify an existing builtin-header directory")
    flags = f"{args.opt} -march={args.march}"
    errors = []
    sources = {}

    totals = {o: Counter() for o in oracles}
    lccc_hist = Counter()
    lccc_reject = Counter()
    rows = []
    for name in names:
        src = PROGRAMS / f"{name}.c"
        if not src.is_file():
            errors.append({"program": name, "compiler": "source", "error": "missing source"})
            continue
        source = src.read_text(errors="replace")
        sources[name] = hashlib.sha256(src.read_bytes()).hexdigest()
        ohists = {}
        for o in oracles:
            try:
                res = gb.compile_on_godbolt(o, source, flags, keep_directives=True,
                                            intel=False, raise_on_error=True)
                if res is None:
                    raise gb.GodboltError("missing oracle response")
            except (gb.GodboltError, ValueError) as exc:
                errors.append({"program": name, "compiler": o, "error": str(exc)})
                ohists[o] = Counter()
                continue
            hs = oracle_headers(gb.assembly_lines(res))
            c = Counter(("unaligned" if k is None else k) for (_, k) in hs.values())
            ohists[o] = c
            totals[o].update(c)
        try:
            effective, rejected, err = lccc_decisions(args.lccc, flags, src, include)
        except (OSError, subprocess.TimeoutExpired) as exc:
            effective, rejected, err = None, None, str(exc)
        if effective is None or rejected is None:
            err = err or "local compiler failed without a diagnostic"
            print(f"{name}: lccc compile failed {err.strip()[:80]}")
            errors.append({"program": name, "compiler": "lccc", "error": err})
            continue
        lc = Counter(("cascade" if k == "cascade" else k) for k in effective.values())
        for why in rejected.values():
            lccc_reject[why] += 1
        lccc_hist.update(lc)
        rows.append((name, ohists, lc))
    width = max(len(n) for n, *_ in rows) if rows else 10
    print(f"{'program':{width}s}  " + "  ".join(f"{o:>22s}" for o in oracles)
          + f"  {'lccc-tight':>16s}")
    for name, oh, lc in rows:
        cells = []
        for o in oracles:
            c = oh[o]
            items = sorted(c.items(), key=lambda kv: (isinstance(kv[0], str), kv[0]))
            cells.append(
                " ".join((f"cas:{v}" if k == "cascade" else f"{bucket_name(k)}:{v}") for k, v in items)
                or "-"
            )
        print(f"{name:{width}s}  " + "  ".join(f"{x:>22s}" for x in cells)
              + "  " + f"{(' '.join(f'{bucket_name(k)}:{v}' for k, v in sorted(lc.items(), key=lambda kv: (isinstance(kv[0], str), kv[0]))) or '-'):>16s}")
    print("\n=== corpus totals (loop-header strongest unconditional alignment) ===")
    for o in oracles:
        c = totals[o]
        items = sorted(c.items(), key=lambda kv: (isinstance(kv[0], str), kv[0]))
        print(
            f"  {o:9s}: "
            + " ".join((f"cascade:{v}" if k == "cascade" else f"{bucket_name(k)}:{v}") for k, v in items)
        )
    print("  lccc     : " + " ".join(f"{bucket_name(k)}:{v}" for k, v in sorted(lccc_hist.items(), key=lambda kv: (isinstance(kv[0], str), kv[0])))
          + "   rejects: " + " ".join(f"{k}:{v}" for k, v in sorted(lccc_reject.items())))

    for error in errors:
        print(f"ERROR: {error}", file=sys.stderr)
    if args.json:
        args.json.parent.mkdir(parents=True, exist_ok=True)
        report = {"complete": not errors, "metric": "alignment directives, not performance or same-loop equivalence",
                  "flags": flags, "include": include, "local": str(args.lccc), "oracles": oracles,
                  "source_sha256": sources, "errors": errors,
                  "rows": [{"program": n, "oracle_histograms": oh, "local_histogram": lc} for n, oh, lc in rows]}
        temporary = args.json.with_suffix(".tmp")
        temporary.write_text(json.dumps(report, indent=2) + "\n")
        temporary.replace(args.json)
    return int(bool(errors))


if __name__ == "__main__":
    sys.exit(main())
