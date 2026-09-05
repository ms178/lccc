#!/usr/bin/env python3
"""Dynamic A/B of one compiler knob, designed for a noisy shared VM.

This is a *screening* tool, not a substitute for a controlled bare-metal
benchmark or PMU data.  Shared-VM measurements contain scheduling and
co-tenant noise.  We report the minimum of interleaved samples (plus a mean of
the fastest third) because it is relatively resistant to one-sided scheduling
outliers, but retain a noise threshold and make no claim that either estimator
is a proof of a speedup.

Other properties that matter here:

* A and B are interleaved in alternating AB/BA order.  This avoids assigning a
  systematic first-or-second-run effect to one configuration.
* Benchmarks within `--floor-margin` of the measured empty-process floor are
  reported but **excluded from the aggregate**: their ratio is process
  startup, not generated code. (This is the artifact that made `fib` read as
  a 23.8x win when a direct measurement says 149x.)
* Every execution must exit successfully and produce the same stdout as every
  other A/B execution.  A mismatch or non-zero exit is a hard failure, not a
  timing result.
* `--min-delta` suppresses verdicts below a noise threshold so a sub-noise
  wobble is not read as a result.

Usage:
    scripts/perf_ab.py --env CCC_NO_EXPR_SINK=1
    scripts/perf_ab.py --env CCC_FOO=1 --reps 15 --only nbody,expat_xml_scan
"""
from __future__ import annotations

import argparse
import os
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
LCCC = os.environ.get("LCCC", str(REPO / "target" / "fastbuild" / "lccc"))
PROGRAMS = REPO / "tests" / "benchmark" / "programs"


def build(src: Path, out: str, extra_env: dict[str, str], flags: list[str]) -> tuple[bool, str]:
    """Compile one side and retain diagnostics for a useful A/B failure."""
    env = dict(os.environ)
    env.update(extra_env)
    p = subprocess.run(
        [LCCC, *flags, str(src), "-o", out],
        capture_output=True,
        env=env,
        timeout=900,
    )
    diagnostic = (p.stdout + p.stderr).decode(errors="replace").strip()
    return p.returncode == 0, diagnostic


def run_once(path: str) -> tuple[float, int, bytes]:
    """Return elapsed milliseconds, exit status, and stdout for one run."""
    t0 = time.perf_counter()
    p = subprocess.run([path], capture_output=True, timeout=900)
    return (time.perf_counter() - t0) * 1000.0, p.returncode, p.stdout


def low_mean(xs: list[float]) -> float:
    """Mean of the fastest third — a noise-resistant secondary statistic."""
    k = max(1, len(xs) // 3)
    return statistics.fmean(sorted(xs)[:k])


def main() -> int:
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    ap.add_argument(
        "--env", action="append", default=[], help="KEY=VALUE applied to the B side; repeatable"
    )
    ap.add_argument("--opt", default="-O2")
    ap.add_argument("--cflag", action="append", default=[])
    ap.add_argument("--reps", type=int, default=11)
    ap.add_argument("--only", default="")
    ap.add_argument("--floor-margin", type=float, default=1.5)
    ap.add_argument(
        "--min-delta",
        type=float,
        default=1.0,
        help="percent below which a per-benchmark delta is called noise",
    )
    args = ap.parse_args()
    if args.reps < 1:
        ap.error("--reps must be positive")

    env_b: dict[str, str] = {}
    for kv in args.env:
        k, sep, v = kv.partition("=")
        if not sep or not k:
            ap.error(f"invalid --env {kv!r}; expected KEY=VALUE")
        env_b[k] = v
    if not env_b:
        ap.error("--env is required (nothing to A/B)")

    flags = [args.opt, *args.cflag]
    only = {s.strip() for s in args.only.split(",") if s.strip()}
    srcs = sorted(PROGRAMS.glob("*.c"))
    if only:
        srcs = [s for s in srcs if s.stem in only]
    if not srcs:
        ap.error("no benchmark source matched --only")

    with tempfile.TemporaryDirectory(prefix="perfab-") as tmp:
        floor_src = Path(tmp) / "floor.c"
        floor_src.write_text(
            "#include <stdio.h>\nint main(void){volatile long r=0;printf(\"%ld\\n\",r);return 0;}\n"
        )
        floor_bin = f"{tmp}/floor"
        floor_build = subprocess.run(
            ["gcc", "-O2", str(floor_src), "-o", floor_bin], capture_output=True
        )
        if floor_build.returncode:
            print("FAILURES: could not build the process-floor probe", file=sys.stderr)
            return 1
        try:
            floor_samples = [run_once(floor_bin) for _ in range(15)]
        except subprocess.TimeoutExpired:
            print("FAILURES: process-floor probe timed out", file=sys.stderr)
            return 1
        if any(returncode != 0 for _, returncode, _ in floor_samples):
            print("FAILURES: process-floor probe exited non-zero", file=sys.stderr)
            return 1
        floor = min(elapsed for elapsed, _, _ in floor_samples)

        label = " ".join(f"{k}={v}" for k, v in env_b.items())
        print(f"# perf A/B — A: default   B: {label}")
        print(f"# flags: {' '.join(flags)}   reps: {args.reps}   floor: {floor:.2f} ms")
        print(
            f"# primary statistic: MINIMUM of {args.reps} alternating AB/BA rounds "
            "(shared-VM screening metric; confirm material results elsewhere)"
        )
        print()
        print(f"{'benchmark':<26}{'A min':>10}{'B min':>10}{'B/A':>9}{'B/A low3':>10}  note")

        ratios: list[float] = []
        skipped: list[str] = []
        failures: list[str] = []
        movers: list[tuple[float, str]] = []
        for src in srcs:
            a_bin, b_bin = f"{tmp}/{src.stem}.a", f"{tmp}/{src.stem}.b"
            a_ok, a_diagnostic = build(src, a_bin, {}, flags)
            b_ok, b_diagnostic = build(src, b_bin, env_b, flags)
            if not a_ok or not b_ok:
                failed_sides = ", ".join(
                    side for side, ok in (("A", a_ok), ("B", b_ok)) if not ok
                )
                failures.append(f"{src.stem}: {failed_sides} compile")
                for side, diagnostic in (("A", a_diagnostic), ("B", b_diagnostic)):
                    if diagnostic:
                        print(f"# {src.stem} {side} compiler output:\n{diagnostic}")
                continue

            a_t: list[float] = []
            b_t: list[float] = []
            expected_stdout: bytes | None = None
            failure: str | None = None
            try:
                # Alternate first runner, rather than permanently giving A a
                # warm filesystem/cache/thermal position in every pair.
                for round_index in range(args.reps):
                    order = (("A", a_bin), ("B", b_bin))
                    if round_index % 2:
                        order = order[::-1]
                    for side, binary in order:
                        elapsed, returncode, stdout = run_once(binary)
                        if returncode != 0:
                            failure = f"{src.stem}: {side} non-zero exit ({returncode})"
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
                failure = f"{src.stem}: timeout"

            if failure:
                failures.append(failure)
                note = "*** OUTPUT MISMATCH ***" if "MISMATCH" in failure else "*** EXECUTION FAILURE ***"
                print(f"{src.stem:<26}{'':>10}{'':>10}{'':>9}{'':>10}  {note}")
                continue

            amin, bmin = min(a_t), min(b_t)
            r = bmin / amin if amin > 0 else 1.0
            a_low, b_low = low_mean(a_t), low_mean(b_t)
            r_low = b_low / a_low if a_low > 0 else 1.0
            note = ""
            # If either side is too close to startup cost, its ratio does not
            # describe generated code and must not affect the aggregate.
            if min(amin, bmin) - floor < args.floor_margin:
                note = "at floor — excluded"
                skipped.append(src.stem)
            else:
                ratios.append(r)
                if abs(r - 1.0) * 100 >= args.min_delta:
                    movers.append((r, src.stem))
            print(f"{src.stem:<26}{amin:>10.2f}{bmin:>10.2f}{r:>9.3f}{r_low:>10.3f}  {note}")

        print()
        if ratios:
            g = statistics.geometric_mean(ratios)
            print(f"aggregate over {len(ratios)} above-floor benchmarks: geomean B/A = {g:.4f}")
            pct = (g - 1.0) * 100
            if abs(pct) < args.min_delta:
                print(
                    f"VERDICT: NO MEASURABLE DIFFERENCE ({pct:+.2f}%, below the "
                    f"{args.min_delta:.1f}% noise threshold)"
                )
            elif pct > 0:
                print(f"VERDICT: configuration A is {pct:.2f}% FASTER overall")
            else:
                print(f"VERDICT: configuration A is {-pct:.2f}% SLOWER overall")

            # B/A > 1 means B took longer, i.e. A is faster.  Partitioning
            # avoids the old report bug that printed one mover in both lists.
            a_faster = sorted((item for item in movers if item[0] > 1.0), reverse=True)
            a_slower = sorted(item for item in movers if item[0] < 1.0)
            if a_faster:
                print(
                    "  A faster on: "
                    + ", ".join(f"{n} {100 * (r - 1):+.1f}%" for r, n in a_faster[:5])
                )
            if a_slower:
                print(
                    "  A slower on: "
                    + ", ".join(f"{n} {100 * (r - 1):+.1f}%" for r, n in a_slower[:5])
                )
        if skipped:
            print(
                f"excluded (within {args.floor_margin} ms of the {floor:.2f} ms floor): "
                f"{', '.join(skipped)}"
            )
        if failures:
            print("FAILURES: " + "; ".join(failures))
            return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
