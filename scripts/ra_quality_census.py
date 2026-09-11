#!/usr/bin/env python3
"""Register-allocation quality census: LCCC vs GCC vs Clang, per function.

`kernel_count.py` reports one number (instructions).  Register-allocator work
needs the *shape* of the loss, so this tool classifies every instruction of
every function in a corpus into RA-relevant buckets and prints per-function
and per-bucket columns (lccc/gcc/clang) sorted by the instruction delta:

    insns    all instructions in the function body
    rrmov    register-to-register `mov*` (coalescing failures / relays)
    stkref   memory operands through %rsp/%rbp/%esp/%ebp (spill traffic)
    push    `push %reg` (callee-saved pressure)
    acc      accumulator-pinned forms (cltq/cwtl/cqto/cltd/...)

Function boundaries come from `.type NAME,@function`, not from every assembly
label: `.L*` basic-block labels are part of their enclosing function.  This is
important for LCCC, whose first `.LBB*` label otherwise used to truncate a
function's count, and for benchmark drivers where hot static helpers inline
into `main`.

Two modes:

* default: cross-compiler census (lccc vs gcc, clang optional).
* `--ab-env KEY=VALUE` (repeatable): same-binary A/B census - A side is the
  default LCCC, B side is LCCC compiled with the variables set.  This is the
  experiment `ra_ab_census.py` used to own (it now forwards here); its gate
  is preserved: exit 1 when HOT code (non-`main` functions) regresses -
  spill traffic (stkref) with instruction count (insns) as the tiebreak -
  unless `--no-gate` is given.

The `--kernels` preset censuses only the curated hot-function map that
`kernel_count.py` used to own (it now forwards here): the 15 kernel_corpus
files, each restricted to its one hot function.

Usage:
    scripts/ra_quality_census.py                  # benchmark programs + kernel corpus
    scripts/ra_quality_census.py FILE.c ...       # explicit files
    scripts/ra_quality_census.py --include-main   # include inlined benchmark drivers
    scripts/ra_quality_census.py --m32            # i686 (needs gcc-multilib)
    scripts/ra_quality_census.py --json out.json  # machine-readable
    scripts/ra_quality_census.py --top 15         # 15 worst functions only
    scripts/ra_quality_census.py --ab-env CCC_EVICT_MODE=6
    scripts/ra_quality_census.py --kernels [--filter adler crc]

Environment:
    LCCC / GCC / CLANG   compiler paths (defaults: target/fastbuild/lccc, gcc, clang)
    CFLAGS               extra flags appended for every compiler

A compiler failure on one file skips that file and is listed at the end; the
census never aborts on oracle-only or target-specific programs.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
DEFAULT_DIRS = [
    REPO / "tests" / "benchmark" / "programs",
    REPO / "tests" / "benchmark" / "kernel_corpus",
]
KERNEL_DIR = REPO / "tests" / "benchmark" / "kernel_corpus"
LCCC = os.environ.get("LCCC", str(REPO / "target" / "fastbuild" / "lccc"))
GCC = os.environ.get("GCC", "gcc")
CLANG = os.environ.get("CLANG", "clang")
EXTRA = os.environ.get("CFLAGS", "").split()
BUCKETS = ("insns", "rrmov", "stkref", "push", "acc")

# Curated hot-function map for the kernel corpus (moved in from
# kernel_count.py, which is now a compatibility wrapper around this census).
# One entry per kernel: the file, and the ONE function whose instruction
# count is the quality signal for that kernel.
KERN_FUNCS = {
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

_SYMBOL = r"[A-Za-z_$][\w.$@]*"
_LABEL = re.compile(rf"^({_SYMBOL}):")
_TYPE_FUNCTION = re.compile(rf"^\s*\.type\s+({_SYMBOL})\s*,\s*@function\b")
_SIZE_FUNCTION = re.compile(rf"^\s*\.size\s+({_SYMBOL})\s*,")
_DIRECTIVE = re.compile(r"^\s*\.")
_REG_REG_MOV = re.compile(r"^\s*mov[lqwb]?\s+%[a-z0-9]+\s*,\s*%[a-z0-9]+\s*$")
_STKREF = re.compile(r"\(%(?:rsp|rbp|esp|ebp)\b")
_PUSH = re.compile(r"^\s*push[lq]?\s+%")
_ACC = re.compile(r"^\s*(cltq|cwtl|cqto|cltd|cbtw|cwtd|cdqe)\b")


def compile_to_asm(cc: str, src: Path, flags: list[str],
                   env_extra: dict[str, str] | None = None) -> str | None:
    env = None
    if env_extra is not None:
        env = dict(os.environ)
        env.update(env_extra)
    fd, out = tempfile.mkstemp(suffix=".s")
    os.close(fd)
    try:
        proc = subprocess.run(
            [cc, *flags, "-S", "-o", out, str(src)],
            capture_output=True,
            text=True,
            timeout=180,
            env=env,
        )
        if proc.returncode != 0:
            return None
        return Path(out).read_text(errors="replace")
    except (subprocess.TimeoutExpired, OSError):
        return None
    finally:
        try:
            os.unlink(out)
        except OSError:
            pass


def is_local_label(name: str) -> bool:
    """Whether `name` is an assembler-local control-flow label."""
    return name.startswith((".L", "$L"))


def census(asm: str) -> dict[str, dict[str, int]]:
    """Count complete function bodies without mistaking basic-block labels for functions."""
    funcs: dict[str, dict[str, int]] = {}
    counts: dict[str, int] | None = None
    current_name: str | None = None
    pending_function: str | None = None
    in_text = True
    for raw in asm.splitlines():
        line = raw.split("#", 1)[0].rstrip()
        s = line.strip()
        if not s:
            continue
        if s.startswith((".section", ".data", ".bss", ".rodata")):
            in_text = ".text" in s
            if not in_text:
                counts = None
                current_name = None
            continue
        if s == ".text":
            in_text = True
            continue

        type_match = _TYPE_FUNCTION.match(line)
        if type_match:
            pending_function = type_match.group(1)
            continue
        size_match = _SIZE_FUNCTION.match(line)
        if size_match and size_match.group(1) == current_name:
            counts = None
            current_name = None
            continue

        label_match = _LABEL.match(line)
        if label_match:
            label = label_match.group(1)
            if in_text and (label == pending_function or (pending_function is None and not is_local_label(label))):
                current_name = label
                counts = funcs.setdefault(label, dict.fromkeys(BUCKETS, 0))
                if label == pending_function:
                    pending_function = None
            # Any other label is an in-function/basic-block/data label.  It
            # must not reset `counts`; doing so silently dropped most bodies.
            continue
        if _DIRECTIVE.match(line) or counts is None or not in_text:
            continue
        counts["insns"] += 1
        counts["rrmov"] += bool(_REG_REG_MOV.match(line))
        counts["stkref"] += bool(_STKREF.search(line))
        counts["push"] += bool(_PUSH.match(line))
        counts["acc"] += bool(_ACC.match(line))
    return funcs


_VALUE_FLAGS = ("--O", "--opt", "--cflag", "--ab-env", "--filter", "--json", "--top")


def _join_dash_values(argv: list[str]) -> list[str]:
    """Join `--flag -O3`-style pairs into `--flag=-O3`.

    argparse mistakes a value that starts with a single `-` (e.g. `-O3`,
    `-ffast-math`) for an option, so the historical `--opt -O3` /
    `--cflag -ffoo` spellings of ra_ab_census.py must be forwarded in the
    `=`-joined form.  Values that are already flags or plain words are left
    untouched.
    """
    out: list[str] = []
    i = 0
    while i < len(argv):
        a = argv[i]
        if a in _VALUE_FLAGS and i + 1 < len(argv):
            nxt = argv[i + 1]
            if nxt.startswith("-") and not nxt.startswith("--") and nxt not in _VALUE_FLAGS:
                out.append(f"{a}={nxt}")
                i += 2
                continue
        out.append(a)
        i += 1
    return out


def _zeros() -> dict[str, int]:
    return dict.fromkeys(BUCKETS, 0)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("files", nargs="*", type=Path)
    ap.add_argument("--m32", action="store_true")
    ap.add_argument("--O", "--opt", dest="opt_level", default="-O2",
                    help="optimization level (default: -O2)")
    ap.add_argument("--json", type=Path)
    ap.add_argument("--top", type=int, default=0)
    ap.add_argument(
        "--include-main",
        action="store_true",
        help="include main; useful when benchmark helpers inline into its hot loop",
    )
    ap.add_argument("--no-clang", action="store_true")
    ap.add_argument("--no-gcc", action="store_true",
                    help="drop the GCC reference column (A/B experiments rarely need it)")
    ap.add_argument(
        "--ab-env",
        dest="ab_env",
        action="append",
        default=[],
        metavar="KEY=VALUE",
        help="same-binary A/B mode (repeatable): A side = default lccc, "
             "B side = lccc compiled with these variables set; the report and "
             "the exit-1 regression gate are the historical ra_ab_census ones "
             "(kernel spill traffic with insns tiebreak; see --no-gate)",
    )
    ap.add_argument(
        "--no-gate",
        action="store_true",
        help="in --ab-env mode, report-only: never exit 1 on regression",
    )
    ap.add_argument(
        "--kernels",
        action="store_true",
        help="preset: census only the curated kernel_corpus hot functions "
             "(the map kernel_count.py used to own); positional files are "
             "not accepted with this preset - use --filter to narrow",
    )
    ap.add_argument(
        "--filter",
        action="append",
        default=[],
        metavar="SUBSTR",
        help="only files whose name contains SUBSTR; repeatable (any match wins)",
    )
    ap.add_argument(
        "--cflag",
        action="append",
        default=[],
        help="additional C compiler flags appended for every compiler; repeatable",
    )
    args = ap.parse_args(_join_dash_values(sys.argv[1:]))

    if args.kernels and args.files:
        ap.error("--kernels selects its own corpus; use --filter to narrow it")
    env_b: dict[str, str] = {}
    for kv in args.ab_env:
        k, sep, v = kv.partition("=")
        if not sep or not k:
            ap.error(f"invalid --ab-env {kv!r}; expected KEY=VALUE")
        env_b[k] = v

    if args.kernels:
        files = [KERNEL_DIR / f for f in KERN_FUNCS]
    else:
        files = list(args.files) or [f for d in DEFAULT_DIRS for f in sorted(d.glob("*.c"))]
    if args.filter:
        files = [f for f in files if any(s in f.name for s in args.filter)]

    flags = [args.opt_level, *EXTRA, *args.cflag] + (["-m32"] if args.m32 else [])

    if env_b:
        return run_ab_census(args, files, flags, env_b)

    compilers = [("lccc", LCCC)]
    if not args.no_gcc:
        compilers.append(("gcc", GCC))
    if not args.no_clang and shutil.which(CLANG):
        compilers.append(("clang", CLANG))
    names = [n for n, _ in compilers]

    table = []
    totals = {n: _zeros() for n in names}
    skipped = []
    for src in files:
        per_cc = {}
        for name, cc in compilers:
            asm = compile_to_asm(cc, src, flags)
            if asm is None:
                skipped.append(f"{src.name}({name})")
                break
            per_cc[name] = census(asm)
        if len(per_cc) != len(compilers):
            continue
        fns = sorted(set(per_cc["lccc"]) & set(per_cc.get("gcc", per_cc["lccc"])))
        if args.kernels:
            hot = KERN_FUNCS.get(src.name)
            if hot is None:
                continue
            if hot not in fns:
                skipped.append(f"{src.name}: hot function {hot} not found in all compilers")
                continue
            fns = [hot]
        for fn in fns:
            if fn == "main" and not args.include_main:
                continue
            row = {n: per_cc[n].get(fn, _zeros()) for n in names}
            table.append((src.name, fn, row))
            for name in names:
                for bucket in BUCKETS:
                    totals[name][bucket] += row[name][bucket]

    if "gcc" in names:
        table.sort(key=lambda item: item[2]["lccc"]["insns"] - item[2]["gcc"]["insns"], reverse=True)
    else:
        table.sort(key=lambda item: item[2]["lccc"]["insns"], reverse=True)
    rows = table[: args.top] if args.top else table
    tag = "/".join(names)
    header = f"{'file':<26} {'function':<24} " + " ".join(
        f"{bucket + ' ' + tag:>20}" for bucket in BUCKETS
    )
    print(header)
    print("-" * len(header))
    for filename, function, row in rows:
        print(
            f"{filename[:26]:<26} {function[:24]:<24} "
            + " ".join(
                f"{'/'.join(str(row[name][bucket]) for name in names):>20}"
                for bucket in BUCKETS
            )
        )
    print("-" * len(header))
    print(
        f"{'TOTAL':<26} {str(len(table)) + ' fns':<24} "
        + " ".join(
            f"{'/'.join(str(totals[name][bucket]) for name in names):>20}"
            for bucket in BUCKETS
        )
    )
    if args.kernels and table and "gcc" in names:
        # kernel_count.py's verdict summary: how many hot kernels LCCC wins.
        wins = sum(1 for _f, _fn, row in table if row["lccc"]["insns"] < row["gcc"]["insns"])
        ties = sum(1 for _f, _fn, row in table if row["lccc"]["insns"] == row["gcc"]["insns"])
        print(
            f"kernels: LCCC fewer insns on {wins}/{len(table)}, "
            f"equal on {ties}, more on {len(table) - wins - ties}"
        )
    if skipped:
        print("skipped:", ", ".join(skipped))
    if args.json:
        args.json.write_text(
            json.dumps(
                {
                    "flags": flags,
                    "compilers": dict(compilers),
                    "include_main": args.include_main,
                    "kernels": args.kernels,
                    "totals": totals,
                    "skipped": skipped,
                    "functions": [
                        {"file": filename, "fn": function, **{name: row[name] for name in names}}
                        for filename, function, row in table
                    ],
                },
                indent=2,
            )
            + "\n"
        )
    return 0


def run_ab_census(args, files: list[Path], flags: list[str], env_b: dict[str, str]) -> int:
    """Same-binary A/B census (absorbs ra_ab_census.py, now a wrapper).

    A = the default LCCC configuration, B = LCCC with the --ab-env variables
    set, both measured with the unified census buckets.  The verdict and the
    exit-1 gate are the historical ra_ab_census semantics exactly: HOT code
    decides it - spill traffic (stkref) in non-`main` CHANGED functions, with
    the instruction count (insns) as tiebreak.  A configuration that only
    improves driver functions has not improved anything that runs.

    GCC/Clang columns stay optional: they are shown unless --no-gcc /
    --no-clang, and never influence the gate.
    """
    label = " ".join(f"{k}={v}" for k, v in env_b.items())
    compilers = [("A", LCCC, {}), ("B", LCCC, env_b)]
    if not args.no_gcc:
        compilers.append(("gcc", GCC, None))
    if not args.no_clang and shutil.which(CLANG):
        compilers.append(("clang", CLANG, None))

    tot_a = _zeros()
    tot_b = _zeros()
    table: list[tuple[str, str, dict[str, int], dict[str, int]]] = []
    skipped: list[str] = []
    for src in files:
        per_cc = {}
        ok = True
        for name, cc, env in compilers:
            asm = compile_to_asm(cc, src, flags, env)
            if asm is None:
                skipped.append(f"{src.name}({name})")
                ok = False
                break
            per_cc[name] = census(asm)
        if not ok:
            continue
        fns = sorted(set(per_cc["A"]) & set(per_cc["B"]))
        if args.kernels:
            hot = KERN_FUNCS.get(src.name)
            if hot is None:
                continue
            if hot not in fns:
                skipped.append(f"{src.name}: hot function {hot} not found in all compilers")
                continue
            fns = [hot]
        for fn in fns:
            a = per_cc["A"].get(fn, _zeros())
            b = per_cc["B"].get(fn, _zeros())
            table.append((src.name, fn, a, b))
            for k in BUCKETS:
                tot_a[k] += a[k]
                tot_b[k] += b[k]

    # Only functions whose counts actually moved are interesting for an A/B.
    rows = [r for r in table if r[2] != r[3]]
    rows.sort(key=lambda r: (r[3]["stkref"] - r[2]["stkref"], r[3]["insns"] - r[2]["insns"]))

    print(f"# RA A/B census — A: default   B: {label}   flags: {' '.join(flags)}")
    print(f"# sources: {len(files)}  changed functions: {len(rows)}"
          + (f"  skipped: {len(skipped)}" if skipped else ""))
    print()
    top = args.top or 15
    if rows:
        print(f"{'file':<28}{'function':<30}" + "".join(f"{k:>10}" for k in BUCKETS))
        head = rows[:top]
        tail = rows[-top:] if len(rows) > top else []
        for group, title in ((head, "best"), (tail, "worst")):
            if not group:
                continue
            print(f"-- {title} --")
            for fname, fn, a, b in group:
                deltas = "".join(f"{b[k] - a[k]:>+10}" for k in BUCKETS)
                print(f"{fname:<28}{fn:<30}{deltas}")
        print()

    # Hot/cold split. A whole-corpus total silently weights a benchmark's
    # cold `main` (setup, timing, printing) the same as its hot kernel, and
    # those two can move in OPPOSITE directions. Report both, and let the
    # KERNEL total decide the verdict.
    ka, kb = _zeros(), _zeros()
    da, db = _zeros(), _zeros()
    for _f, fn, a, b in rows:
        ta, tb = (da, db) if fn == "main" else (ka, kb)
        for k in BUCKETS:
            ta[k] += a[k]
            tb[k] += b[k]
    print(f"{'CHANGED FUNCTIONS ONLY':<58}" + "".join(f"{k:>10}" for k in BUCKETS))
    print(f"{'  driver (main) delta':<58}"
          + "".join(f"{db[k] - da[k]:>+10}" for k in BUCKETS))
    print(f"{'  kernel (non-main) delta':<58}"
          + "".join(f"{kb[k] - ka[k]:>+10}" for k in BUCKETS))
    print()
    print(f"{'TOTAL':<58}" + "".join(f"{k:>10}" for k in BUCKETS))
    print(f"{'  A (default)':<58}" + "".join(f"{tot_a[k]:>10}" for k in BUCKETS))
    print(f"{'  B (' + label + ')':<58}" + "".join(f"{tot_b[k]:>10}" for k in BUCKETS))
    print(f"{'  delta':<58}" + "".join(f"{tot_b[k] - tot_a[k]:>+10}" for k in BUCKETS))
    pct = [0.0 if tot_a[k] == 0 else 100.0 * (tot_b[k] - tot_a[k]) / tot_a[k] for k in BUCKETS]
    print(f"{'  delta %':<58}" + "".join(f"{p:>+10.2f}" for p in pct))

    if args.json:
        args.json.parent.mkdir(parents=True, exist_ok=True)
        args.json.write_text(json.dumps({
            # historical ra_ab_census.py JSON shape (functions = changed only)
            "env": env_b, "flags": flags,
            "total_a": tot_a, "total_b": tot_b,
            "skipped": skipped,
            "functions": [{"file": f, "fn": n, "a": a, "b": b} for f, n, a, b in rows],
            # unified-census additions
            "gate": {
                "enabled": not args.no_gate,
                "kernel_stkref_delta": kb["stkref"] - ka["stkref"],
                "kernel_insns_delta": kb["insns"] - ka["insns"],
            },
            "kernel_totals": {"a": ka, "b": kb},
            "driver_totals": {"a": da, "b": db},
        }, indent=2) + "\n")

    # The verdict is decided by HOT code: spill traffic in non-`main`
    # functions, with instruction count as the tiebreak. A configuration that
    # only improves driver functions has not improved anything that runs.
    k_stk = kb["stkref"] - ka["stkref"]
    k_ins = kb["insns"] - ka["insns"]
    regressed = k_stk > 0 or (k_stk == 0 and k_ins > 0)
    print()
    print(f"VERDICT (kernel functions): "
          f"{'REGRESSION' if regressed else 'no regression'}"
          f"  [stkref {k_stk:+d}, insns {k_ins:+d}]"
          + ("  (gate disabled by --no-gate)" if args.no_gate else ""))
    if regressed and args.no_gate:
        return 0
    return 1 if regressed else 0


if __name__ == "__main__":
    sys.exit(main())
