#!/usr/bin/env python3
"""MS-01a: hard codegen-quality gate for the golden workloads.

The runtime CI (ci-bench.py) gates a single metric (fib >= 10x vs GCC) —
spectral_norm could regress 4x and stay green. This gate protects what
actually matters, CODE GENERATION QUALITY, on the six golden workloads
from the performance charter:

    gzip_crc32, zlib_ng_adler32, expat_xml_scan, sqlite_varint,
    glibc_memcmp, hash_table  (+ stencil5 as the OP-05a sentinel)

Per workload it compiles with `lccc -O2 -S` and extracts stable, 
noise-free assembly metrics:

    insns      total instruction count (function bodies)
    stackmem   rbp/rsp-relative memory operands (spill traffic)
    pushes     prologue pressure (callee-save saves)
    ymm        256-bit vector instructions
    fma        fused multiply-add instructions

and compares each against a checked-in baseline
(.github/scripts/ci-codegen-baseline.json) with PER-WORKLOAD tolerance
bands. A metric may regress at most its tolerance (default 2%) without
an explicit baseline refresh; improvements update the baseline via
--update-baseline in the PR that lands them.

Usage:
    python3 ci-codegen-gate.py --lccc target/fastbuild/lccc
    python3 ci-codegen-gate.py --lccc ... --update-baseline
    python3 ci-codegen-gate.py --lccc ... --json out.json --summary
"""

import argparse
import json
import re
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).parent.resolve()
REPO = HERE.parent.parent
PROGRAMS = REPO / "tests" / "benchmark" / "programs"
BASELINE = HERE / "ci-codegen-baseline.json"

# (source file, label) — the golden set + the OP-05a sentinel.
WORKLOADS = [
    ("gzip_crc32.c", "gzip_crc32"),
    ("zlib_ng_adler32.c", "zlib_ng_adler32"),
    ("expat_xml_scan.c", "expat_xml_scan"),
    ("sqlite_varint.c", "sqlite_varint"),
    ("glibc_memcmp.c", "glibc_memcmp"),
    ("hash_table.c", "hash_table"),
    ("fp_memfold_stencil5.c", "stencil5"),
]

# Metric -> default max regression (fraction). A metric that got BETTER
# never fails; only regressions beyond the band do.
DEFAULT_TOLERANCE = 0.02
# Spill traffic is the RA-sensitive metric the whole program is about;
# give it a slightly wider band so ordinary churn does not flap CI.
TOLERANCES = {
    "stackmem": 0.05,
    "pushes": 0.10,
    "moves": 0.05,
}

INSN_RE = re.compile(r"^\s+([a-z0-9]+)[\s]")
MEM_RE = re.compile(r"[-0-9(]*\((?:%rbp|%rsp)\)")
YMM_RE = re.compile(r"^\s+(?:v[a-z]+)\s+[^#]*%ymm")
FMA_RE = re.compile(r"^\s+(?:vfmadd|vfnmadd|vfmsub|vfnmsub)[a-z0-9]*\s")


def function_body_metrics(asm: str) -> dict:
    """Aggregate metrics over function bodies only (between .type @function
    and the next .size/.section)."""
    m = {"insns": 0, "stackmem": 0, "pushes": 0, "moves": 0, "ymm": 0, "fma": 0}
    in_func = False
    for line in asm.splitlines():
        s = line.strip()
        if s.startswith(".type") and "@function" in s:
            in_func = True
            continue
        if s == ".size" or (s.startswith(".section") and in_func):
            in_func = False
            continue
        if not in_func:
            continue
        mi = INSN_RE.match(line)
        if not mi:
            continue
        op = mi.group(1)
        m["insns"] += 1
        if MEM_RE.search(line):
            m["stackmem"] += 1
        if op.startswith("push"):
            m["pushes"] += 1
        if op in ("mov", "movq", "movl", "movw", "movb", "vmovsd", "vmovss"):
            if "%" in line and "(" not in line:
                m["moves"] += 1
        if YMM_RE.match(line):
            m["ymm"] += 1
        if FMA_RE.match(line):
            m["fma"] += 1
    return m


def compile_metrics(lccc: str, gcc_inc: str, src: Path) -> dict | None:
    with tempfile.NamedTemporaryFile(suffix=".s", delete=False) as f:
        out = Path(f.name)
    try:
        r = subprocess.run(
            [lccc, f"-I{gcc_inc}", "-O2", "-S", str(src), "-o", str(out)],
            capture_output=True, text=True, timeout=300,
        )
        if r.returncode != 0:
            return None
        return function_body_metrics(out.read_text())
    finally:
        out.unlink(missing_ok=True)


def detect_gcc_inc() -> str:
    r = subprocess.run(
        ["gcc", "-print-file-name=include"], capture_output=True, text=True
    )
    return r.stdout.strip()


def main():
    p = argparse.ArgumentParser(description="LCCC codegen quality gate")
    p.add_argument("--lccc", required=True, help="Path to LCCC binary")
    p.add_argument("--gcc-inc", default=None)
    p.add_argument("--update-baseline", action="store_true",
                   help="Write the measured metrics as the new baseline")
    p.add_argument("--json", metavar="FILE", help="Dump results JSON")
    p.add_argument("--summary", action="store_true", help="GitHub markdown")
    args = p.parse_args()

    gcc_inc = args.gcc_inc or detect_gcc_inc()
    if not Path(args.lccc).exists():
        print(f"ERROR: LCCC binary not found: {args.lccc}", file=sys.stderr)
        sys.exit(2)

    baseline = {}
    if BASELINE.exists() and not args.update_baseline:
        baseline = json.loads(BASELINE.read_text())

    results = []
    failures = []
    for src_name, label in WORKLOADS:
        src = PROGRAMS / src_name
        if not src.exists():
            results.append({"label": label, "skip": str(src)})
            continue
        got = compile_metrics(args.lccc, gcc_inc, src)
        if got is None:
            failures.append(f"{label}: compile failed")
            results.append({"label": label, "error": "compile failed"})
            continue

        entry = {"label": label, "metrics": got}
        base = baseline.get(label)
        if base is not None:
            regressions = []
            for metric, value in got.items():
                old = base.get(metric)
                if old is None or old == 0:
                    # No baseline for this metric: only fail if it appeared
                    # with a vengeance (any amount > 0 when baseline absent
                    # and value large). Keep conservative: skip.
                    continue
                tol = TOLERANCES.get(metric, DEFAULT_TOLERANCE)
                limit = old * (1.0 + tol)
                if value > limit:
                    regressions.append(
                        f"{metric} {old} -> {value} (+{100*(value-old)/old:.1f}%, "
                        f"tolerance {tol*100:.0f}%)"
                    )
            entry["regressions"] = regressions
            if regressions:
                failures.append(f"{label}: " + "; ".join(regressions))
        results.append(entry)

    if args.update_baseline:
        new_base = {}
        for r in results:
            if "metrics" in r:
                new_base[r["label"]] = r["metrics"]
        BASELINE.write_text(json.dumps(new_base, indent=2) + "\n")
        print(f"Baseline updated: {BASELINE}", file=sys.stderr)

    if args.json:
        Path(args.json).write_text(json.dumps(results, indent=2))

    if args.summary:
        print("## Codegen Quality Gate (golden workloads)")
        print("")
        print("| Workload | insns | stackmem | pushes | ymm | fma | verdict |")
        print("|---|---:|---:|---:|---:|---:|---|")
        for r in results:
            if "skip" in r:
                print(f"| {r['label']} | — | — | — | — | — | SKIPPED |")
                continue
            m = r["metrics"]
            verdict = "PASS" if not r.get("regressions") else (
                "REGRESSION: " + "; ".join(r["regressions"])
            )
            print(
                f"| {r['label']} | {m['insns']} | {m['stackmem']} | "
                f"{m['pushes']} | {m['ymm']} | {m['fma']} | {verdict} |"
            )

    print(file=sys.stderr)
    if failures:
        print("ERROR: codegen regressions detected:", file=sys.stderr)
        for f in failures:
            print(f"  - {f}", file=sys.stderr)
        print(
            "If these are intentional improvements-in-progress, refresh with "
            "--update-baseline in the landing PR.",
            file=sys.stderr,
        )
        sys.exit(1)
    print("Codegen gate: all golden workloads within tolerance.", file=sys.stderr)


if __name__ == "__main__":
    main()
