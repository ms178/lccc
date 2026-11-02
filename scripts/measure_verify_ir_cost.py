#!/usr/bin/env python3
"""Measure what the IR verifier costs and whether the corpus passes it (T6).

For every C file under the given roots, compile at each requested -O level
twice with the same compiler -- once plainly, once with
``CCC_VERIFY_IR=abort`` (every pass boundary verified, first violation fatal)
-- and record:

* the compile-time ratio (verified wall time / plain wall time), summed over
  the corpus and as a per-file median, measured sequentially so the two
  timings of one file run back to back;
* every file where the verified compile fails, with the first violation line.

A non-zero exit from the verified leg is a verifier finding (a pass produced
malformed IR), not a harness issue, so the script reports it as a violation
rather than hiding it.  ``--json`` writes the full table for the follow-up
document.

Sequential by design: on the two-core CI host, parallel timing measures
contention, not the verifier.
"""
from __future__ import annotations

import argparse
import json
import os
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path


def timed(cmd: list[str], env: dict[str, str], timeout: float) -> tuple[float, int, str]:
    start = time.perf_counter()
    try:
        proc = subprocess.run(
            cmd, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE,
            timeout=timeout, check=False, text=True, errors="replace",
        )
        rc, err = proc.returncode, proc.stderr
    except subprocess.TimeoutExpired as exc:
        rc = 124
        err = (exc.stderr or b"").decode(errors="replace") if isinstance(exc.stderr, bytes) else (exc.stderr or "")
    return time.perf_counter() - start, rc, err


def first_violation(stderr: str) -> str:
    for line in stderr.splitlines():
        if "IR verification failed" in line or "violation" in line.lower() or "dominat" in line:
            return line.strip()[:300]
    return stderr.strip().splitlines()[-1][:300] if stderr.strip() else "(no stderr)"


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("roots", nargs="+", type=Path)
    ap.add_argument("--ccc", default="target/fastbuild/lccc")
    ap.add_argument("--levels", default="-O2", help="comma list, e.g. -O1,-O2")
    ap.add_argument("--timeout", type=float, default=60.0)
    ap.add_argument("--limit", type=int, default=0, help="0 = all files")
    ap.add_argument("--json", type=Path)
    args = ap.parse_args()

    files: list[Path] = []
    for root in args.roots:
        files += sorted(p for p in root.rglob("*.c") if p.is_file())
    if args.limit:
        files = files[: args.limit]
    levels = [lvl.strip() for lvl in args.levels.split(",") if lvl.strip()]

    plain_env = dict(os.environ)
    plain_env.pop("CCC_VERIFY_IR", None)
    verify_env = dict(plain_env, CCC_VERIFY_IR="abort")

    rows: list[dict] = []
    with tempfile.TemporaryDirectory(prefix="verify-cost-") as tmp:
        out = Path(tmp) / "o.o"
        for src in files:
            for lvl in levels:
                base = [args.ccc, lvl, "-c", str(src), "-o", str(out)]
                t_plain, rc_plain, _ = timed(base, plain_env, args.timeout)
                t_ver, rc_ver, err_ver = timed(base, verify_env, args.timeout)
                rows.append({
                    "file": str(src), "level": lvl,
                    "plain_s": round(t_plain, 4), "plain_rc": rc_plain,
                    "verify_s": round(t_ver, 4), "verify_rc": rc_ver,
                    "violation": None if rc_ver == 0 else first_violation(err_ver),
                })

    ok = [r for r in rows if r["plain_rc"] == 0 and r["verify_rc"] == 0]
    plain_total = sum(r["plain_s"] for r in ok)
    verify_total = sum(r["verify_s"] for r in ok)
    ratios = [r["verify_s"] / r["plain_s"] for r in ok if r["plain_s"] > 0]
    violations = [r for r in rows if r["verify_rc"] != 0 and r["plain_rc"] == 0]
    print(f"files={len(files)} levels={levels} compared={len(ok)}")
    if ok:
        print(f"total plain={plain_total:.2f}s verified={verify_total:.2f}s "
              f"ratio={verify_total / plain_total:.3f}")
        print(f"per-file ratio median={statistics.median(ratios):.3f} "
              f"p90={sorted(ratios)[int(0.9 * (len(ratios) - 1))]:.3f} "
              f"max={max(ratios):.3f}")
    print(f"verifier violations (plain ok, verified fails): {len(violations)}")
    for r in violations[:20]:
        print(f"  {r['file']} {r['level']}: {r['violation']}")
    if args.json:
        args.json.write_text(json.dumps({"rows": rows}, indent=1) + "\n")
    return 1 if violations else 0


if __name__ == "__main__":
    sys.exit(main())
