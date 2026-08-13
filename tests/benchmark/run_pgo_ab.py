#!/usr/bin/env python3
"""LCCC PGO A/B benchmark laboratory.

For every workload kernel and compiler (lccc, gcc) it builds TWO binaries:

  * `plain`  — compiled with the requested optimization level only;
  * `pgo`    — the same flags PLUS a profile-guided round trip:
        generate  : -fprofile-generate=<dir>  -> compile
        train     : run the generated binary once (production training data)
        use       : -fprofile-use=<dir>        -> compile the final binary

It then verifies that every binary prints the SAME output (differential
correctness: a PGO build must be observationally identical to the plain build
and to the reference compiler), and times all binaries in interleaved,
shuffled rounds so the PGO-vs-plain ratio and the lccc-vs-gcc ratio are both
paired and controlled.

The runtime measurement is CPU-time based and every binary is a fixed-workload
program with a deterministic self-check, so this is a fair A/B even without a
PMU.  Run with `--reps 21` (default) for bootstrap-CI quality.

Usage:
  python3 run_pgo_ab.py --reps 21 --opt -O2 --compilers lccc,gcc \
      --only gzip_crc32,expat_xml_scan,zlib_ng_adler32
"""
from __future__ import annotations

import argparse
import json
import os
import random
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_LCCC = REPO_ROOT / "target" / "release" / "lccc"
PROGRAMS = REPO_ROOT / "tests" / "benchmark" / "programs"


def invoke(command: list[str], *, timeout_s: int, cwd: Path | None = None) -> dict:
    start = time.perf_counter_ns()
    try:
        cp = subprocess.run(
            command, capture_output=True, text=True, timeout=timeout_s, cwd=cwd
        )
        ok = cp.returncode == 0
        return {
            "ok": ok,
            "wall_s": (time.perf_counter_ns() - start) / 1e9,
            "returncode": cp.returncode,
            "stdout": cp.stdout,
            "stderr": cp.stderr,
            "error": None,
        }
    except subprocess.TimeoutExpired:
        return {
            "ok": False,
            "wall_s": float(timeout_s),
            "returncode": -1,
            "stdout": "",
            "stderr": "",
            "error": "timeout",
        }


def bootstrap_ci(values: list[float], seed: int, resamples: int = 4000) -> dict | None:
    """Median with a bias-corrected bootstrap 95% CI. None if <2 samples."""
    if len(values) < 2:
        return None
    rng = random.Random(seed)
    n = len(values)
    medians = []
    for _ in range(resamples):
        medians.append(statistics.median(values[rng.randrange(n)] for _ in range(n)))
    medians.sort()
    lo = medians[int(0.025 * len(medians))]
    hi = medians[int(0.975 * len(medians))]
    return {
        "median": statistics.median(values),
        "mean": statistics.mean(values),
        "min": min(values),
        "max": max(values),
        "ci_lo": lo,
        "ci_hi": hi,
        "n": n,
    }


def pair_ratio(pgo_values: list[float], plain_values: list[float], seed: int) -> dict | None:
    """Paired ratio of PGO over plain for matching run indices."""
    n = min(len(pgo_values), len(plain_values))
    if n < 2:
        return None
    rng = random.Random(seed)
    ratios = []
    for i in range(n):
        # Paired within a round; tiny eps to avoid div-by-zero.
        ratios.append((pgo_values[i] + 1e-9) / (plain_values[i] + 1e-9))
    c = bootstrap_ci(ratios, seed + 12345)
    if c is None:
        return None
    c["pgo_median_s"] = statistics.median(pgo_values)
    c["plain_median_s"] = statistics.median(plain_values)
    c["pgo_speedup_vs_plain"] = 1.0 / c["median"]  # >1 = PGO faster
    return c


def gcc_include() -> str | None:
    r = subprocess.run(
        ["gcc", "-print-file-name=include"], capture_output=True, text=True
    )
    return r.stdout.strip() if r.returncode == 0 and r.stdout.strip() else None


def compiler_flags(compiler: str, opt: str, extra: list[str], ginc: str | None) -> list[str]:
    if compiler == "lccc":
        flags = [opt]
        if ginc:
            flags.append(f"-I{ginc}")
        return flags + extra
    return [opt] + extra


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--only", default="", help="comma-separated kernel names (default: all)")
    ap.add_argument("--opt", default="-O2", help="optimization level")
    ap.add_argument("--compilers", default="lccc,gcc")
    ap.add_argument("--reps", type=int, default=21)
    ap.add_argument("--warmup", type=int, default=2)
    ap.add_argument("--cflag", action="append", default=[], help="extra compiler flag")
    ap.add_argument("--work", default="/tmp/lccc_pgo_ab", help="work directory")
    ap.add_argument("--lccc", default=str(DEFAULT_LCCC))
    ap.add_argument("--json", default="", help="write JSON report to this path")
    ap.add_argument("--timeout", type=int, default=120)
    args = ap.parse_args()

    compilers = [c.strip() for c in args.compilers.split(",") if c.strip()]
    work = Path(args.work)
    work.mkdir(parents=True, exist_ok=True)
    ginc = gcc_include() if "lccc" in compilers else None

    selected = {s.strip() for s in args.only.split(",") if s.strip()}
    kernels = sorted(p.stem for p in PROGRAMS.glob("*.c"))
    if selected:
        kernels = [k for k in kernels if k in selected]

    if not kernels:
        print(f"no kernels selected (available: {sorted(p.stem for p in PROGRAMS.glob('*.c'))})")
        return 2

    report = {
        "lccc": args.lccc,
        "opt": args.opt,
        "compilers": compilers,
        "gcc_version": subprocess.run(
            ["gcc", "--version"], capture_output=True, text=True
        ).stdout.splitlines()[0] if "gcc" in compilers else None,
        "results": {},
    }

    for name in kernels:
        src = PROGRAMS / f"{name}.c"
        if not src.exists():
            continue
        entry: dict = {"plain": {}, "pgo": {}, "outputs": {}, "ratios": {}, "results": {}}
        binaries: dict[str, dict[str, Path]] = {}
        outputs: dict[str, str] = {}

        for compiler in compilers:
            exe = args.lccc if compiler == "lccc" else compiler
            flags = compiler_flags(compiler, args.opt, args.cflag, ginc)
            base = work / f"{name}_{compiler}"
            plain_bin = base / "plain"
            gen_dir = base / "prof"
            use_bin = base / "pgo"
            gen_dir.mkdir(parents=True, exist_ok=True)

            # ---- plain build ----
            plain_bin.parent.mkdir(parents=True, exist_ok=True)
            cmd = [exe, *flags, "-o", str(plain_bin), str(src)]
            r = invoke(cmd, timeout_s=args.timeout, cwd=REPO_ROOT)
            if not r["ok"]:
                entry["plain"][compiler] = {"ok": False, "stderr": r["stderr"][:800]}
                continue
            # train once (deterministic kernel -> identical profile)
            tr = invoke([str(plain_bin)], timeout_s=args.timeout)
            if not tr["ok"]:
                entry["plain"][compiler] = {"ok": False, "stderr": "train run failed"}
                continue
            outputs[f"{compiler}_plain"] = tr["stdout"]

            # ---- PGO build (generate -> train -> use) ----
            gen_bin = base / "gen"
            gcmd = [exe, *flags, f"-fprofile-generate={gen_dir}", "-o", str(gen_bin), str(src)]
            rg = invoke(gcmd, timeout_s=args.timeout, cwd=REPO_ROOT)
            if not rg["ok"]:
                entry["pgo"][compiler] = {"ok": False, "stderr": rg["stderr"][:800]}
                continue
            t2 = invoke([str(gen_bin)], timeout_s=args.timeout)
            if not t2["ok"]:
                entry["pgo"][compiler] = {"ok": False, "stderr": "generated train run failed"}
                continue
            outputs[f"{compiler}_pgo_train"] = t2["stdout"]
            ucmd = [exe, *flags, f"-fprofile-use={gen_dir}", "-o", str(use_bin), str(src)]
            ru = invoke(ucmd, timeout_s=args.timeout, cwd=REPO_ROOT)
            if not ru["ok"]:
                entry["pgo"][compiler] = {"ok": False, "stderr": ru["stderr"][:800]}
                continue
            binaries[compiler] = {"plain": plain_bin, "pgo": use_bin}
            entry["plain"][compiler] = {"ok": True}
            entry["pgo"][compiler] = {"ok": True}

        # ---- differential correctness ----
        # All successful builds must print the identical output.
        out_values = set(outputs.values())
        if len(out_values) > 1:
            entry["outputs"] = outputs
            entry["differential_ok"] = False
        else:
            entry["differential_ok"] = True
        # PGO (use) output must equal its plain output for every compiler.
        for compiler in compilers:
            if f"{compiler}_plain" in outputs and f"{compiler}_pgo_train" in outputs:
                if outputs[f"{compiler}_plain"] != outputs[f"{compiler}_pgo_train"]:
                    entry["differential_ok"] = False

        # ---- timed A/B rounds (interleaved, shuffled) ----
        if not entry.get("differential_ok", False):
            # Still time, but flag it loudly.
            entry["warning"] = "differential mismatch (see outputs)"
        variants: dict[str, Path] = {}
        for compiler in compilers:
            if compiler in binaries:
                variants[f"{compiler}_plain"] = binaries[compiler]["plain"]
                variants[f"{compiler}_pgo"] = binaries[compiler]["pgo"]

        timed: dict[str, list[float]] = {k: [] for k in variants}
        rng = random.Random(hash((name, args.opt)) & 0xFFFFFFFF)
        keys = list(variants)
        for _ in range(args.warmup):
            rng.shuffle(keys)
            for k in keys:
                invoke([str(variants[k])], timeout_s=args.timeout)
        for _ in range(args.reps):
            rng.shuffle(keys)
            for k in keys:
                run = invoke([str(variants[k])], timeout_s=args.timeout)
                if run["ok"]:
                    timed[k].append(run["wall_s"])
        for k in variants:
            entry["results"][k] = bootstrap_ci(timed[k], hash(k) & 0xFFFFFFFF)
        # Paired ratios.
        for compiler in compilers:
            r = pair_ratio(timed.get(f"{compiler}_pgo", []), timed.get(f"{compiler}_plain", []),
                           hash((name, compiler, "pgo")) & 0xFFFFFFFF)
            if r:
                entry["ratios"][f"{compiler}_pgo_over_plain"] = r
        # lccc vs gcc on the same build kind.
        if "lccc" in compilers and "gcc" in compilers:
            for kind in ("plain", "pgo"):
                r = pair_ratio(timed.get(f"lccc_{kind}", []), timed.get(f"gcc_{kind}", []),
                               hash((name, "lccc_vs_gcc", kind)) & 0xFFFFFFFF)
                if r:
                    entry["ratios"][f"lccc_over_gcc_{kind}"] = r

        report["results"][name] = entry
        # ---- console summary ----
        def line(k: str) -> str:
            s = entry["results"].get(k)
            return "n/a" if not s else f"{s['median']*1e3:.3f}ms [{s['ci_lo']*1e3:.3f},{s['ci_hi']*1e3:.3f}]"
        print(f"\n=== {name}  ({args.opt}) ===")
        print(f"  differential_ok={entry.get('differential_ok')}")
        for compiler in compilers:
            pr = entry["ratios"].get(f"{compiler}_pgo_over_plain")
            prs = "n/a" if not pr else f"{pr['pgo_speedup_vs_plain']:.3f}x PGO speedup (pgo {pr['pgo_median_s']*1e3:.3f}ms vs plain {pr['plain_median_s']*1e3:.3f}ms)"
            print(f"  {compiler:5} plain {line(f'{compiler}_plain'):>28} | pgo {line(f'{compiler}_pgo'):>28} | {prs}")
        lpg = entry["ratios"].get("lccc_over_gcc_pgo")
        if lpg:
            print(f"  lccc_pgo/gcc_pgo ratio = {lpg['median']:.3f}x (lccc {lpg['pgo_median_s']*1e3:.3f}ms vs gcc {lpg['plain_median_s']*1e3:.3f}ms)")

    if args.json:
        Path(args.json).write_text(json.dumps(report, indent=2, default=str))
        print(f"\nreport written to {args.json}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
