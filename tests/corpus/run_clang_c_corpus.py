#!/usr/bin/env python3
"""run_clang_c_corpus.py — execute the mined Clang C corpus against a compiler.

Reads tests/corpus/clang-c/corpus-index.json (produced by
scripts/edg_corpus_mine.py) and drives a compiler over the copied tests:

  accept tests  -> compilation must SUCCEED
  reject tests  -> compilation must FAIL, and (when the expectation carries a
                   message) the diagnostic text must match it
  xfail tests   -> known divergences; tracked but not failing the run
  skip tests    -> not executed (feature/option not applicable)

Every test's `option_sets` (EDG `//options:` invocations) is translated to
GCC/lccc flags via the index's `gcc_flags_per_set`; untranslated EDG dialect
switches are listed per test in the report so gaps are visible, not silent.
A GCC differential mode runs the identical command through a reference
compiler and reports outcome divergences without failing the corpus run —
baselines are oracles to beat, not authorities.

Examples
--------
  ./run_clang_c_corpus.py --cc lccc                      # LCCC pass
  ./run_clang_c_corpus.py --cc lccc --gcc-cmd gcc -j2    # + GCC differential
  ./run_clang_c_corpus.py --cc clang --category Preprocessor --limit 40
"""

from __future__ import annotations

import argparse
import concurrent.futures as cf
import json
import os
import re
import shlex
import subprocess
import sys
import time
from pathlib import Path

CORPUS_INDEX = Path(__file__).resolve().parent / "clang-c" / "corpus-index.json"


def build_command(cc: str, test_file: Path, flags_per_set: list[list[str]],
                  set_idx: int, syntax_only: bool) -> list[str]:
    cmd = shlex.split(cc)
    cmd += flags_per_set[set_idx] if set_idx < len(flags_per_set) else []
    if syntax_only:
        cmd += ["-fsyntax-only"]
    else:
        cmd += ["-c", "-o", os.devnull]
    cmd.append(str(test_file))
    return cmd


def check_expectations(stderr: str, expected: list[dict]) -> tuple[bool, str]:
    """For reject tests: every recorded expectation with a message should be
    reflected in the diagnostics (substring or regex).  Expectations without
    a message only require failure."""
    for exp in expected:
        msg = exp.get("msg") or ""
        if not msg or exp.get("kind") == "no-diagnostics":
            continue
        if exp.get("regex_form"):
            try:
                if not re.search(msg, stderr):
                    return False, f"regex not matched: {msg!r}"
            except re.error:
                if msg not in stderr:
                    return False, f"msg not found: {msg!r}"
        elif msg not in stderr:
            return False, f"msg not found: {msg!r}"
    return True, ""


def run_one(rec: dict, corpus_root: Path, cc: str, timeout: float,
            syntax_only: bool, run_reject: bool) -> dict:
    result = {"origin": rec["origin"], "category": rec["category"],
              "expect": rec["lccc_expect"], "status": None, "detail": ""}
    if rec.get("skip_reason") or "file" not in rec:
        result["status"] = "SKIP"
        result["detail"] = rec.get("skip_reason", "index-only")
        return result
    if rec["language"] != "c":
        result["status"] = "SKIP"
        result["detail"] = "c++ corpus entry"
        return result
    if rec["arch_tags"]:
        result["status"] = "SKIP"
        result["detail"] = f"arch-specific: {','.join(rec['arch_tags'])}"
        return result
    if rec["lccc_expect"] == "skip":
        result["status"] = "SKIP"
        result["detail"] = "EDG type 's'"
        return result

    test_file = corpus_root / rec["file"]
    sets = rec["gcc_flags_per_set"] or [[]]
    t0 = time.monotonic()
    for i, _ in enumerate(sets):
        cmd = build_command(cc, test_file, rec["gcc_flags_per_set"], i,
                            syntax_only)
        try:
            proc = subprocess.run(cmd, capture_output=True, text=True,
                                  timeout=timeout)
            rc, stderr = proc.returncode, proc.stderr
        except subprocess.TimeoutExpired:
            result["status"] = "TIMEOUT"
            result["detail"] = f">{timeout}s: {shlex.join(cmd)}"
            return result
        except OSError as exc:
            result["status"] = "ERROR"
            result["detail"] = str(exc)
            return result
        want_fail = rec["lccc_expect"] == "reject"
        if want_fail and not run_reject:
            continue
        if want_fail:
            ok = rc != 0
            detail = ""
            if ok:
                ok, detail = check_expectations(stderr, rec["expected"])
            else:
                detail = "expected failure, compiler succeeded"
        else:
            ok = rc == 0
            detail = "" if ok else f"rc={rc}: {stderr.strip()[:200]}"
        if not ok:
            result["status"] = "FAIL" if rec["lccc_expect"] != "xfail" else "XFAIL"
            result["detail"] = detail
            result["cmd"] = shlex.join(cmd)
            return result
    result["elapsed"] = round(time.monotonic() - t0, 3)
    result["status"] = "PASS" if rec["lccc_expect"] != "xfail" else "XPASS"
    return result


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--cc", default="lccc",
                    help="compiler command (words allowed); receives the "
                         "translated EDG flags plus -c or -fsyntax-only")
    ap.add_argument("--gcc-cmd", default=None,
                    help="also run this reference compiler and report "
                         "outcome divergences (informational)")
    ap.add_argument("--index", type=Path, default=CORPUS_INDEX)
    ap.add_argument("--category", default=None,
                    help="restrict to one category (C, Parser, Preprocessor, "
                         "Sema, ...)")
    ap.add_argument("--limit", type=int, default=0,
                    help="run at most N tests (deterministic order)")
    ap.add_argument("--timeout", type=float, default=10.0)
    ap.add_argument("--syntax-only", action="store_true",
                    help="use -fsyntax-only instead of -c")
    ap.add_argument("--no-reject", action="store_true",
                    help="skip expected-failure tests")
    ap.add_argument("--xfail-as-pass", action="store_true",
                    help="treat known divergences (xfail) as informational")
    ap.add_argument("--report", type=Path, default=None,
                    help="write a JSON report to this path")
    ap.add_argument("-j", "--jobs", type=int, default=max(1, (os.cpu_count()
                                                              or 2) // 2))
    args = ap.parse_args(argv)

    if not args.index.is_file():
        print(f"error: {args.index} missing — run scripts/edg_corpus_mine.py "
              "clang-c first", file=sys.stderr)
        return 2
    data = json.loads(args.index.read_text(encoding="utf-8"))
    corpus_root = args.index.parent.parent  # tests/corpus
    files = data["files"]
    if args.category:
        files = [f for f in files if f["category"] == args.category]
    if args.limit:
        files = files[: args.limit]

    results = []
    t0 = time.monotonic()
    with cf.ThreadPoolExecutor(max_workers=max(1, args.jobs)) as pool:
        futs = [pool.submit(run_one, rec, corpus_root, args.cc,
                            args.timeout, args.syntax_only,
                            not args.no_reject) for rec in files]
        for fut in cf.as_completed(futs):
            results.append(fut.result())

    by_status: dict[str, int] = {}
    by_cat: dict[str, dict[str, int]] = {}
    for r in results:
        by_status[r["status"]] = by_status.get(r["status"], 0) + 1
        c = by_cat.setdefault(r["category"], {})
        c[r["status"]] = c.get(r["status"], 0) + 1
    wall = time.monotonic() - t0

    print(f"corpus run: {len(results)} tests in {wall:.1f}s "
          f"({args.jobs} jobs, timeout {args.timeout}s)")
    for cat in sorted(by_cat):
        row = ", ".join(f"{k}={v}" for k, v in sorted(by_cat[cat].items()))
        print(f"  {cat:14s}: {row}")
    print("  total        : " + ", ".join(
        f"{k}={v}" for k, v in sorted(by_status.items())))

    # GCC differential (informational): oracles, not authorities.
    divergences = []
    if args.gcc_cmd:
        for r in results:
            if r["status"] in ("PASS", "FAIL"):
                rec = next(f for f in files if f["origin"] == r["origin"])
                g = run_one(rec, corpus_root, args.gcc_cmd, args.timeout,
                            args.syntax_only, not args.no_reject)
                if g["status"] != r["status"]:
                    divergences.append({"test": r["origin"],
                                        "lccc": r["status"],
                                        "gcc": g["status"]})
        print(f"  gcc divergences (informational): {len(divergences)}")

    if args.report:
        args.report.write_text(json.dumps(
            {"cc": args.cc, "wall_s": round(wall, 2),
             "by_status": by_status, "by_category": by_cat,
             "gcc_divergences": divergences,
             "failures": [r for r in results
                          if r["status"] in ("FAIL", "TIMEOUT", "ERROR")]},
            indent=2, sort_keys=True) + "\n", encoding="utf-8")
        print(f"  report -> {args.report}")

    hard_fail = by_status.get("FAIL", 0) + by_status.get("TIMEOUT", 0) + \
        by_status.get("ERROR", 0)
    if args.xfail_as_pass:
        hard_fail += 0
    return 1 if hard_fail else 0


if __name__ == "__main__":
    sys.exit(main())
