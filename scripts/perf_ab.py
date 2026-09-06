#!/usr/bin/env python3
"""Unified Compiler Performance & A/B Screening Engine for LCCC.

Supports:
1. Dynamic A/B testing of compiler knobs/flags/optimizations (with built-in presets
   or custom environment variables).
2. Paired multi-compiler benchmarking (LCCC vs GCC, Clang, ICX, or custom binaries).
3. Noise-resistant shared-VM metrics (interleaved alternating AB/BA rounds,
   empty-process floor exclusion, geometric mean speedup calculation, and
   statistical bounds).
4. Output verification on every run (ensuring zero miscompilations or crashes).
5. JSON and Markdown report export.

Built-in Presets:
  --preset fp_memfold          Scalar FP memory-source folding
  --preset reduction_vecreg    Vector reduction accumulators
  --preset vecreg_ops          Vector intrinsic / vecreg chains
  --preset vector_remainder    Vector remainder transitions
  --preset expr_sink           Expression sinking optimization
  --preset iv_widen            Induction variable widening
  --preset peephole            Peephole optimizer pass
  --preset gvn                 Global Value Numbering
  --preset mem2reg             Memory-to-register promotion
  --preset tailcall            Tail call elimination
  --preset inlining            Function inlining
  --preset licm                Loop invariant code motion
  --preset dce                 Dead code elimination
  --preset sccp                Sparse conditional constant propagation

Usage Examples:
  scripts/perf_ab.py --preset fp_memfold
  scripts/perf_ab.py --env CCC_NO_EXPR_SINK=1 --reps 11
  scripts/perf_ab.py --vs gcc --opt -O2 --only chacha20_block,sha256_transform,glibc_strstr
  scripts/perf_ab.py --preset vecreg_ops --json results_vecreg.json
"""
from __future__ import annotations

import argparse
import concurrent.futures
import json
import os
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import Any

REPO = Path(__file__).resolve().parent.parent
DEFAULT_LCCC = REPO / "target" / "fastbuild" / "lccc"
PROGRAMS_DIR = REPO / "tests" / "benchmark" / "programs"

PRESETS: dict[str, dict[str, Any]] = {
    "fp_memfold": {
        "desc": "Scalar FP memory-source folding",
        "env": {"CCC_PEEPHOLE_SKIP": "fp_reg_mem_fold"},
        "only": ["fp_memfold_stencil5", "nbody", "spectral_norm"],
    },
    "reduction_vecreg": {
        "desc": "Vector reduction accumulators",
        "env": {"CCC_NO_REDUCTION_VECREG": "1"},
        "only": ["double_reduction", "histogram", "zlib_ng_adler32"],
    },
    "vecreg_ops": {
        "desc": "Vector intrinsic / vecreg chains",
        "env": {"CCC_NO_VECREG": "1"},
        "only": ["chacha20_block", "sha256_transform", "zlib_ng_adler32"],
    },
    "vecreg": {
        "desc": "Vector intrinsic / vecreg chains",
        "env": {"CCC_NO_VECREG": "1"},
        "only": ["chacha20_block", "sha256_transform", "zlib_ng_adler32"],
    },
    "vector_remainder": {
        "desc": "Vector remainder transitions",
        "env": {"CCC_NO_VEC_REMAINDER": "1"},
        "only": ["chacha20_block", "zlib_ng_adler32", "histogram"],
    },
    "expr_sink": {
        "desc": "Expression sinking optimization",
        "env": {"CCC_NO_EXPR_SINK": "1"},
        "only": ["arith_loop", "loop_patterns", "sqlite_varint"],
    },
    "iv_widen": {
        "desc": "Induction variable widening",
        "env": {"CCC_NO_IV_WIDEN": "1"},
        "only": ["sqlite_varint", "glibc_memcmp", "expat_xml_scan"],
    },
    "peephole": {
        "desc": "Peephole optimizer pass",
        "env": {"CCC_NO_PEEPHOLE": "1"},
        "only": [],
    },
    "gvn": {
        "desc": "Global Value Numbering",
        "env": {"CCC_NO_GVN": "1"},
        "only": ["arith_loop", "hash_table", "linux_find_bit"],
    },
    "mem2reg": {
        "desc": "Memory-to-register promotion",
        "env": {"CCC_NO_MEM2REG": "1"},
        "only": [],
    },
    "tailcall": {
        "desc": "Tail call elimination",
        "env": {"CCC_NO_TAILCALL": "1"},
        "only": ["tce_sum", "constant_recursion", "ackermann"],
    },
    "inlining": {
        "desc": "Function inlining",
        "env": {"CCC_NO_INLINE": "1"},
        "only": ["fib", "tce_sum", "binary_trees"],
    },
    "licm": {
        "desc": "Loop invariant code motion",
        "env": {"CCC_NO_LICM": "1"},
        "only": ["matmul", "sieve", "nbody"],
    },
    "dce": {
        "desc": "Dead code elimination",
        "env": {"CCC_NO_DCE": "1"},
        "only": [],
    },
    "sccp": {
        "desc": "Sparse conditional constant propagation",
        "env": {"CCC_NO_SCCP": "1"},
        "only": ["constant_recursion", "switch_dispatch"],
    },
}


def build_binary(
    compiler: str,
    src: Path,
    out: str,
    extra_env: dict[str, str],
    flags: list[str],
) -> tuple[bool, str]:
    """Compile one benchmark binary."""
    env = dict(os.environ)
    env.update(extra_env)
    cmd = [compiler, *flags, str(src), "-o", out]
    if "gcc" in compiler or "clang" in compiler or "icx" in compiler:
        cmd.append("-lm")
    try:
        p = subprocess.run(
            cmd,
            capture_output=True,
            env=env,
            timeout=900,
        )
        diagnostic = (p.stdout + p.stderr).decode(errors="replace").strip()
        return p.returncode == 0, diagnostic
    except subprocess.TimeoutExpired:
        return False, "build timed out"
    except Exception as e:
        return False, str(e)


def run_once(path: str) -> tuple[float, int, bytes]:
    """Return elapsed milliseconds, exit status, and stdout for one run."""
    t0 = time.perf_counter()
    p = subprocess.run([path], capture_output=True, timeout=900)
    return (time.perf_counter() - t0) * 1000.0, p.returncode, p.stdout


def low_mean(xs: list[float]) -> float:
    """Mean of the fastest third — noise-resistant secondary statistic."""
    k = max(1, len(xs) // 3)
    return statistics.fmean(sorted(xs)[:k])


def measure_floor(tmp: str) -> float:
    """Measure the empty-process startup floor cost."""
    floor_src = Path(tmp) / "floor.c"
    floor_src.write_text(
        "#include <stdio.h>\nint main(void){volatile long r=0;printf(\"%ld\\n\",r);return 0;}\n"
    )
    floor_bin = f"{tmp}/floor"
    floor_build = subprocess.run(
        ["gcc", "-O2", str(floor_src), "-o", floor_bin], capture_output=True
    )
    if floor_build.returncode != 0:
        return 0.5
    try:
        floor_samples = [run_once(floor_bin) for _ in range(15)]
        return min(elapsed for elapsed, rc, _ in floor_samples if rc == 0)
    except Exception:
        return 0.5


def parse_args() -> argparse.Namespace:
    ap = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    ap.add_argument(
        "--preset",
        choices=list(PRESETS.keys()),
        help="Named optimization/feature screening preset",
    )
    ap.add_argument(
        "--env",
        action="append",
        default=[],
        help="KEY=VALUE applied to configuration B; repeatable",
    )
    ap.add_argument(
        "--compiler-a",
        default=os.environ.get("LCCC", str(DEFAULT_LCCC)),
        help="Path to compiler binary A (default: lccc fastbuild)",
    )
    ap.add_argument(
        "--vs",
        dest="compiler_b",
        help="Compare compiler A directly against reference compiler (e.g. gcc, clang, icx)",
    )
    ap.add_argument(
        "--compiler-b",
        help="Path or name of compiler binary B (alternative to --vs)",
    )
    ap.add_argument(
        "--opt",
        default="-O2",
        help="Optimization level (default: -O2)",
    )
    ap.add_argument(
        "--cflag",
        action="append",
        default=[],
        help="Additional C compiler flags; repeatable",
    )
    ap.add_argument(
        "--reps",
        type=int,
        default=11,
        help="Number of alternating AB/BA rounds (default: 11)",
    )
    ap.add_argument(
        "--only",
        default="",
        help="Comma-separated subset of benchmarks to run",
    )
    ap.add_argument(
        "--floor-margin",
        type=float,
        default=1.5,
        help="Floor margin in ms for excluding startup-bound benchmarks",
    )
    ap.add_argument(
        "--min-delta",
        type=float,
        default=1.0,
        help="Percent threshold below which speedup is deemed noise (default: 1.0)",
    )
    ap.add_argument(
        "--json",
        dest="json_out",
        help="Path to export benchmark results as JSON",
    )
    ap.add_argument(
        "--markdown",
        dest="md_out",
        help="Path to export benchmark table as Markdown",
    )
    ap.add_argument(
        "--list-presets",
        action="store_true",
        help="List available optimization presets and exit",
    )
    return ap.parse_args()


def main() -> int:
    args = parse_args()

    if args.list_presets:
        print("Available optimization presets:")
        for name, data in PRESETS.items():
            env_str = " ".join(f"{k}={v}" for k, v in data["env"].items())
            print(f"  {name:<18} {data['desc']:<42} ({env_str})")
        return 0

    if args.reps < 1:
        sys.exit("error: --reps must be at least 1")

    compiler_a = args.compiler_a
    compiler_b = args.compiler_b or compiler_a
    is_cross_compiler = compiler_a != compiler_b

    env_b: dict[str, str] = {}
    preset_data = PRESETS.get(args.preset) if args.preset else None
    if preset_data:
        env_b.update(preset_data["env"])

    for kv in args.env:
        k, sep, v = kv.partition("=")
        if not sep or not k:
            sys.exit(f"error: invalid --env {kv!r}; expected KEY=VALUE")
        env_b[k] = v

    if not is_cross_compiler and not env_b:
        sys.exit(
            "error: either --preset, --env KEY=VALUE, or --vs <compiler> is required"
        )

    flags = [args.opt, *args.cflag]
    only_set = {s.strip() for s in args.only.split(",") if s.strip()}
    if not only_set and preset_data and preset_data.get("only"):
        only_set = set(preset_data["only"])

    srcs = sorted(PROGRAMS_DIR.glob("*.c"))
    if only_set:
        srcs = [s for s in srcs if s.stem in only_set]
    if not srcs:
        sys.exit("error: no benchmark sources matched specified filter")

    with tempfile.TemporaryDirectory(prefix="perfab-") as tmp:
        floor = measure_floor(tmp)

        if is_cross_compiler:
            label_a = Path(compiler_a).name
            label_b = Path(compiler_b).name
            header_label = f"A: {label_a} vs B: {label_b}"
        else:
            label_b = " ".join(f"{k}={v}" for k, v in env_b.items())
            header_label = f"A: default vs B: {label_b}"

        print(f"# perf A/B — {header_label}")
        print(f"# flags: {' '.join(flags)}   reps: {args.reps}   floor: {floor:.2f} ms")
        print(
            f"# primary metric: MINIMUM over {args.reps} interleaved AB/BA rounds "
            f"(noise floor: {args.floor_margin:.2f} ms)"
        )
        print()
        print(f"{'benchmark':<26}{'A min (ms)':>12}{'B min (ms)':>12}{'B/A ratio':>11}{'B/A low3':>11}  note")
        print("-" * 75)

        results_data: list[dict[str, Any]] = []
        ratios: list[float] = []
        skipped: list[str] = []
        failures: list[str] = []
        movers: list[tuple[float, str]] = []

        for src in srcs:
            a_bin = f"{tmp}/{src.stem}.a"
            b_bin = f"{tmp}/{src.stem}.b"

            a_ok, a_diag = build_binary(compiler_a, src, a_bin, {}, flags)
            b_ok, b_diag = build_binary(compiler_b, src, b_bin, env_b, flags)

            if not a_ok or not b_ok:
                failed_sides = ", ".join(
                    side for side, ok in (("A", a_ok), ("B", b_ok)) if not ok
                )
                failures.append(f"{src.stem}: {failed_sides} compile failure")
                for side, diag in (("A", a_diag), ("B", b_diag)):
                    if diag:
                        print(f"# {src.stem} {side} compiler diagnostic:\n{diag}")
                continue

            a_t: list[float] = []
            b_t: list[float] = []
            expected_stdout: bytes | None = None
            failure: str | None = None

            try:
                for round_idx in range(args.reps):
                    order = (("A", a_bin), ("B", b_bin))
                    if round_idx % 2 == 1:
                        order = (("B", b_bin), ("A", a_bin))

                    for side, binary in order:
                        elapsed, returncode, stdout = run_once(binary)
                        if returncode != 0:
                            failure = f"{src.stem}: {side} exited non-zero ({returncode})"
                            break
                        if expected_stdout is None:
                            expected_stdout = stdout
                        elif stdout != expected_stdout:
                            failure = f"{src.stem}: OUTPUT MISMATCH (miscompile)"
                            break
                        (a_t if side == "A" else b_t).append(elapsed)
                    if failure:
                        break
            except subprocess.TimeoutExpired:
                failure = f"{src.stem}: execution timed out"

            if failure:
                failures.append(failure)
                note = "*** OUTPUT MISMATCH ***" if "MISMATCH" in failure else "*** FAIL ***"
                print(f"{src.stem:<26}{'':>12}{'':>12}{'':>11}{'':>11}  {note}")
                continue

            amin, bmin = min(a_t), min(b_t)
            r = bmin / amin if amin > 0 else 1.0
            a_low, b_low = low_mean(a_t), low_mean(b_t)
            r_low = b_low / a_low if a_low > 0 else 1.0
            note = ""

            if min(amin, bmin) - floor < args.floor_margin:
                note = "at floor — excluded"
                skipped.append(src.stem)
            else:
                ratios.append(r)
                if abs(r - 1.0) * 100 >= args.min_delta:
                    movers.append((r, src.stem))

            print(f"{src.stem:<26}{amin:>12.2f}{bmin:>12.2f}{r:>11.3f}{r_low:>11.3f}  {note}")

            results_data.append({
                "benchmark": src.stem,
                "a_min_ms": amin,
                "b_min_ms": bmin,
                "ratio_b_over_a": r,
                "ratio_low3": r_low,
                "samples_a": a_t,
                "samples_b": b_t,
                "excluded": bool(note),
            })

        print("-" * 75)
        geomean_val = 1.0
        if ratios:
            geomean_val = statistics.geometric_mean(ratios)
            pct = (geomean_val - 1.0) * 100
            print(f"Aggregate over {len(ratios)} benchmarks: geomean B/A = {geomean_val:.4f}")
            if abs(pct) < args.min_delta:
                print(
                    f"VERDICT: NO MEASURABLE DIFFERENCE ({pct:+.2f}%, below "
                    f"{args.min_delta:.1f}% noise threshold)"
                )
            elif pct > 0:
                print(f"VERDICT: configuration A is {pct:.2f}% FASTER overall (B/A = {geomean_val:.4f})")
            else:
                print(f"VERDICT: configuration A is {-pct:.2f}% SLOWER overall (B/A = {geomean_val:.4f})")

            a_faster = sorted((item for item in movers if item[0] > 1.0), reverse=True)
            a_slower = sorted(item for item in movers if item[0] < 1.0)
            if a_faster:
                print("  A faster on: " + ", ".join(f"{n} ({100 * (r - 1):+.1f}%)" for r, n in a_faster[:5]))
            if a_slower:
                print("  A slower on: " + ", ".join(f"{n} ({100 * (r - 1):+.1f}%)" for r, n in a_slower[:5]))

        if skipped:
            print(f"\nExcluded startup-floor benchmarks ({len(skipped)}): {', '.join(skipped)}")

        if failures:
            print("\nFAILURES:\n  " + "\n  ".join(failures), file=sys.stderr)

        if args.json_out:
            json_payload = {
                "header": header_label,
                "flags": flags,
                "reps": args.reps,
                "floor_ms": floor,
                "geomean_b_over_a": geomean_val,
                "benchmarks": results_data,
                "failures": failures,
            }
            Path(args.json_out).write_text(json.dumps(json_payload, indent=2))
            print(f"\nSaved JSON results to {args.json_out}")

        if args.md_out:
            md_lines = [
                f"# Benchmark Screen: {header_label}",
                f"- **Flags**: `{' '.join(flags)}`",
                f"- **Rounds**: `{args.reps}`",
                f"- **Aggregate B/A Geomean**: `{geomean_val:.4f}`",
                "",
                "| Benchmark | A min (ms) | B min (ms) | B/A Ratio | B/A low3 | Note |",
                "| :--- | :---: | :---: | :---: | :---: | :--- |",
            ]
            for d in results_data:
                ex = "excluded" if d["excluded"] else ""
                md_lines.append(
                    f"| `{d['benchmark']}` | {d['a_min_ms']:.2f} | {d['b_min_ms']:.2f} | "
                    f"{d['ratio_b_over_a']:.3f} | {d['ratio_low3']:.3f} | {ex} |"
                )
            Path(args.md_out).write_text("\n".join(md_lines) + "\n")
            print(f"Saved Markdown report to {args.md_out}")

        return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
