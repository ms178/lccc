#!/usr/bin/env python3
"""Paired, interleaved runtime A/B with uninformative-comparison detection.

Why this exists
---------------
`scripts/perf_ab.py` reports a corpus geomean and a verdict. On a shared,
frequency-scaling VM that verdict is not trustworthy on its own, and it has one
failure mode that is actively misleading: **arms that compile to byte-identical
binaries still produce a ratio**. Measured here — 6 of 8 benchmark arms were
byte-identical between the two configurations, yet the harness reported
per-kernel deltas up to ±4%. Averaging those into a geomean produces a number
that looks like evidence and is pure noise.

This harness refuses to do that. It:

1. Builds (or takes) both arms and **hashes them**. If they are identical the
   comparison is declared UNINFORMATIVE and no verdict is emitted (exit 3),
   unless `--allow-identical` is passed.
2. Checks both arms produce **identical stdout and exit status** on a warm-up
   run. An A/B across a miscompile is meaningless, so it is reported as a
   CORRECTNESS failure (exit 4) rather than a timing result.
3. Times the two arms **interleaved within each round, with the order
   alternated round to round**, so drift, thermal state and neighbour load hit
   both arms equally instead of landing on whichever ran second.
4. Reports `min` alongside `median`/`mean`. On a shared VM `min` is the
   least-contaminated estimator; a conclusion that only holds on the median is
   flagged as such.
5. Runs a **paired sign test** over per-round differences so "consistent" means
   something quantitative.

Usage
-----
Build both arms from one source (preferred — enables the identity check):

    scripts/paired_ab.py --src tests/benchmark/programs/sha256_transform.c \
        --ccc target/fastbuild/lccc \
        --cflags '-O2 -DPASSES=4 -DBLOCK_COUNT=32768' \
        --env-a '' --env-b 'CCC_PHI_ACYCLIC_ORDER=1' \
        --label 'phi acyclic copy order' --rounds 25 \
        --json artifacts/paired_phi_acyclic.json

Or compare two pre-built binaries (identity check still runs):

    scripts/paired_ab.py --bin-a new --bin-b legacy --label rot --rounds 25

Exit codes: 0 = measured, 2 = usage/build error, 3 = arms byte-identical
(uninformative), 4 = arms disagree on stdout/status (correctness, not perf).
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import statistics
import subprocess
import sys
import tempfile
import time


#: Options whose value may legitimately start with a dash (compiler flags).
_DASHY_OPTS = ("--cflags", "--env-a", "--env-b", "--label")


def _normalise_dash_values(argv: list[str]) -> list[str]:
    """Rewrite ``--cflags -O2`` as ``--cflags=-O2``.

    argparse otherwise reads a leading-dash value as the next option and errors
    out, which is exactly what a compiler-flag argument looks like.
    """
    out: list[str] = []
    i = 0
    while i < len(argv):
        a = argv[i]
        if a in _DASHY_OPTS and i + 1 < len(argv) and "=" not in a:
            out.append(f"{a}={argv[i + 1]}")
            i += 2
            continue
        out.append(a)
        i += 1
    return out


def sha256(path: str) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def build(src: str, ccc: str, cflags: list[str], env_extra: str, out: str) -> None:
    """Compile `src` with `ccc`, adding `env_extra` (KEY=VAL, comma-separated)."""
    env = dict(os.environ)
    for kv in filter(None, (e.strip() for e in env_extra.split(","))):
        if "=" not in kv:
            raise SystemExit(f"--env: expected KEY=VAL, got {kv!r}")
        k, v = kv.split("=", 1)
        env[k] = v
    cmd = [ccc, *cflags, src, "-o", out]
    r = subprocess.run(cmd, env=env, capture_output=True, text=True)
    if r.returncode != 0:
        raise SystemExit(
            f"build failed for env={env_extra or '<default>'}\n"
            f"  cmd: {' '.join(cmd)}\n{r.stderr.strip()[:2000]}"
        )


def run_once(binary: str) -> tuple[int, str]:
    r = subprocess.run([binary], capture_output=True, text=True)
    return r.returncode, r.stdout


def time_run(binary: str) -> float:
    t0 = time.perf_counter()
    subprocess.run([binary], capture_output=True)
    return (time.perf_counter() - t0) * 1000.0


def sign_test_p(diffs: list[float]) -> float:
    """Two-sided exact-ish sign test p-value over per-round paired differences."""
    nz = [d for d in diffs if d != 0.0]
    n = len(nz)
    if n == 0:
        return 1.0
    k = sum(1 for d in nz if d > 0)
    # normal approximation with continuity correction; adequate for n >= 12
    z = (abs(k - n / 2.0) - 0.5) / (0.5 * n**0.5)
    if z <= 0:
        return 1.0
    # erfc via math
    import math

    return math.erfc(z / math.sqrt(2.0))


def summarise(name: str, t: list[float]) -> dict:
    return {
        "arm": name,
        "n": len(t),
        "min_ms": round(min(t), 3),
        "median_ms": round(statistics.median(t), 3),
        "mean_ms": round(statistics.mean(t), 3),
        "stdev_ms": round(statistics.pstdev(t), 3) if len(t) > 1 else 0.0,
        "samples_ms": [round(x, 3) for x in t],
    }


def main() -> int:
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    ap.add_argument("--src", help="C source to compile into both arms")
    ap.add_argument("--ccc", default="target/fastbuild/lccc", help="compiler under test")
    ap.add_argument(
        "--cflags",
        default="-O2",
        help="compiler flags (shell-quoted string, split on whitespace)",
    )
    ap.add_argument("--env-a", default="", help="extra env for arm A (KEY=VAL[,KEY=VAL])")
    ap.add_argument("--env-b", default="", help="extra env for arm B")
    ap.add_argument("--bin-a", help="pre-built binary for arm A (skips --src build)")
    ap.add_argument("--bin-b", help="pre-built binary for arm B")
    ap.add_argument("--label", default="A/B", help="what is being compared")
    ap.add_argument("--name-a", default="A", help="display name for arm A")
    ap.add_argument("--name-b", default="B", help="display name for arm B")
    ap.add_argument("--rounds", type=int, default=25, help="interleaved rounds")
    ap.add_argument("--warmup", type=int, default=2, help="discarded warm-up rounds")
    ap.add_argument("--json", help="write the raw samples and verdict here")
    ap.add_argument(
        "--allow-identical",
        action="store_true",
        help="emit a verdict even when both arms hash identically (not advised)",
    )
    args = ap.parse_args(_normalise_dash_values(sys.argv[1:]))

    tmp = tempfile.mkdtemp(prefix="paired_ab_")
    if args.bin_a and args.bin_b:
        bin_a, bin_b = args.bin_a, args.bin_b
    elif args.src:
        bin_a = os.path.join(tmp, "arm_a")
        bin_b = os.path.join(tmp, "arm_b")
        build(args.src, args.ccc, args.cflags.split(), args.env_a, bin_a)
        build(args.src, args.ccc, args.cflags.split(), args.env_b, bin_b)
    else:
        ap.error("need either --bin-a/--bin-b or --src")

    for b in (bin_a, bin_b):
        if not os.access(b, os.X_OK):
            print(f"not executable: {b}", file=sys.stderr)
            return 2

    hash_a, hash_b = sha256(bin_a), sha256(bin_b)
    identical = hash_a == hash_b

    print(f"{args.label}")
    print(f"  arm {args.name_a}: {bin_a}")
    print(f"    sha256 {hash_a}   env={args.env_a or '<default>'}")
    print(f"  arm {args.name_b}: {bin_b}")
    print(f"    sha256 {hash_b}   env={args.env_b or '<default>'}")

    result: dict = {
        "label": args.label,
        "arm_a": {"binary": bin_a, "sha256": hash_a, "env": args.env_a},
        "arm_b": {"binary": bin_b, "sha256": hash_b, "env": args.env_b},
        "identical_binaries": identical,
        "rounds": args.rounds,
        "warmup": args.warmup,
    }

    # ---- correctness screen: an A/B across a miscompile is not a perf result --
    rc_a, out_a = run_once(bin_a)
    rc_b, out_b = run_once(bin_b)
    result["arm_a"]["warmup"] = {"rc": rc_a, "stdout_sha256": sha256_of_str(out_a)}
    result["arm_b"]["warmup"] = {"rc": rc_b, "stdout_sha256": sha256_of_str(out_b)}
    if (rc_a, out_a) != (rc_b, out_b):
        print(
            "\n  CORRECTNESS FAILURE: the two arms disagree, so no timing "
            "comparison is meaningful.\n"
            f"    {args.name_a}: rc={rc_a} stdout={out_a[:120]!r}\n"
            f"    {args.name_b}: rc={rc_b} stdout={out_b[:120]!r}",
            file=sys.stderr,
        )
        result["verdict"] = "correctness-failure"
        emit(args.json, result)
        return 4
    print(f"  both arms agree: rc={rc_a}, stdout {len(out_a)} bytes")

    # ---- uninformative-comparison guard -------------------------------
    if identical and not args.allow_identical:
        print(
            "\n  UNINFORMATIVE: both arms are byte-identical (sha256 match).\n"
            "  Any timing difference between them is measurement noise, not a\n"
            "  property of the change under test. No verdict emitted.\n"
            "  (Pass --allow-identical to measure the noise floor deliberately.)",
            file=sys.stderr,
        )
        result["verdict"] = "uninformative-identical-binaries"
        emit(args.json, result)
        return 3

    # ---- interleaved, order-alternated timing -------------------------
    ta: list[float] = []
    tb: list[float] = []
    total = args.warmup + args.rounds
    for r in range(total):
        if r % 2 == 0:
            a, b = time_run(bin_a), time_run(bin_b)
        else:
            b, a = time_run(bin_b), time_run(bin_a)
        if r >= args.warmup:
            ta.append(a)
            tb.append(b)

    sa, sb = summarise(args.name_a, ta), summarise(args.name_b, tb)
    diffs = [x - y for x, y in zip(ta, tb)]  # >0 means A slower than B
    p = sign_test_p(diffs)
    med_ratio = statistics.median(tb) / statistics.median(ta)
    min_ratio = min(tb) / min(ta)
    a_faster = statistics.median(ta) < statistics.median(tb)

    print(f"  {args.name_a}: min={sa['min_ms']:8.2f} med={sa['median_ms']:8.2f} "
          f"mean={sa['mean_ms']:8.2f} sd={sa['stdev_ms']:6.2f} n={sa['n']}")
    print(f"  {args.name_b}: min={sb['min_ms']:8.2f} med={sb['median_ms']:8.2f} "
          f"mean={sb['mean_ms']:8.2f} sd={sb['stdev_ms']:6.2f} n={sb['n']}")
    print(f"  median ratio {args.name_b}/{args.name_a} = {med_ratio:.4f}   "
          f"min ratio = {min_ratio:.4f}   sign-test p = {p:.4f}")

    agrees = (med_ratio < 1.0) == (min_ratio < 1.0)
    delta = abs(1.0 - med_ratio) * 100.0
    verdict = f"{args.name_a} {'FASTER' if a_faster else 'SLOWER'} by {delta:.2f}% (median)"
    if p > 0.05:
        verdict += " — NOT significant (sign test p > 0.05); treat as noise"
    elif not agrees:
        verdict += " — median and min DISAGREE on direction; do not conclude"
    print(f"  => {verdict}")

    result.update(
        {
            "arm_a_timings": sa,
            "arm_b_timings": sb,
            "paired_diffs_ms": [round(d, 3) for d in diffs],
            "median_ratio_b_over_a": round(med_ratio, 5),
            "min_ratio_b_over_a": round(min_ratio, 5),
            "sign_test_p": round(p, 5),
            "median_and_min_agree": agrees,
            "significant": p <= 0.05,
            "verdict": verdict,
        }
    )
    emit(args.json, result)
    return 0


def sha256_of_str(s: str) -> str:
    return hashlib.sha256(s.encode("utf-8", "replace")).hexdigest()


def emit(path: str | None, result: dict) -> None:
    if not path:
        return
    os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
    with open(path, "w") as f:
        json.dump(result, f, indent=2)
    print(f"  raw samples -> {path}")


if __name__ == "__main__":
    sys.exit(main())
