#!/usr/bin/env python3
"""run_clang_c_corpus.py — execute the mined Clang C corpus against a compiler.

Reads tests/corpus/clang-c/corpus-index.json (produced by
scripts/edg_corpus_mine.py) and drives a compiler over the copied tests.

Verdict architecture (PR #721 review, findings P1-1..P1-3).  Four concepts
are kept strictly separate, and every one of them is recorded per test:

  observation  what the compiler DID:  ACCEPT / REJECT / CRASH / TIMEOUT /
               ERROR / UNSUPPORTED.  Signals (rc<0, rc>=128) and ICE text
               are CRASH — a crash is never a correct rejection.
  expectation  what the test DECLARES: accept / reject / xfail.
  diagnostics  the Clang `-verify` policy: template-regex messages
               (`literal {{regex}} literal` splicing), repetition counts,
               and active custom prefixes per invocation.
  verdict      PASS / FAIL / XFAIL / XPASS / TIMEOUT / CRASH / ERROR /
               SKIP.  XFAIL covers semantic divergence only; CRASH,
               TIMEOUT and ERROR are hard failures under every policy.

Invocations: EDG `//options:` runs are separate compiler invocations.
Each set is planned with its own language, flags (`options_all` prepended),
active `-verify` prefixes and arch prerequisites; C++ sets and unsupported
phases are recorded as UNSUPPORTED with a reason — never silently dropped.

Differential mode compares OBSERVED OUTCOMES with a reference compiler
(GCC is an oracle, not an authority): a divergence is an outcome mismatch,
independent of diagnostic-policy failures.

Examples
--------
  ./run_clang_c_corpus.py --cc lccc                      # corpus pass
  ./run_clang_c_corpus.py --cc lccc --gcc-cmd gcc -j2    # + GCC outcome diff
  ./run_clang_c_corpus.py --category Preprocessor --limit 40
"""

from __future__ import annotations

import argparse
import concurrent.futures as cf
import enum
import json
import os
import re
import shlex
import subprocess
import sys
import time
from pathlib import Path

CORPUS_INDEX = Path(__file__).resolve().parent / "clang-c" / "corpus-index.json"
VALID_CATEGORIES = ("C", "Parser", "Preprocessor", "Sema", "torture-top")


class Outcome(enum.Enum):
    ACCEPT = "ACCEPT"
    REJECT = "REJECT"
    CRASH = "CRASH"
    TIMEOUT = "TIMEOUT"
    ERROR = "ERROR"
    UNSUPPORTED = "UNSUPPORTED"


CRASH_TEXT_RE = re.compile(
    r"internal compiler error|segmentation fault|signal SIG|\bpanic:",
    re.IGNORECASE)
DIAG_LINE_RE = re.compile(r"\b(?:error|warning|note):")


def classify(returncode: int, stderr: str) -> Outcome:
    """Map a raw process result to an OBSERVATION.

    A crash (signal delivery: negative returncode from subprocess, or the
    128+N shell convention) or ICE text is never a REJECT: rejecting for the
    wrong reason is a compiler defect, not a passing test (review P1-1).
    """
    if returncode < 0 or returncode >= 128:
        return Outcome.CRASH
    if CRASH_TEXT_RE.search(stderr):
        return Outcome.CRASH
    if returncode == 0:
        return Outcome.ACCEPT
    return Outcome.REJECT


def template_to_regex(msg: str) -> str:
    """Clang `-verify` message template -> Python regex source.

    The message is literal text with `{{regex}}` fragments spliced in
    (review P1-3): `'(unnamed struct at {{.*}})' cannot be ...` must match
    `'(unnamed struct at /x.c:1:1)' cannot be ...`.  Literal chunks are
    escaped so parens/brackets stay literal; only the `{{...}}` interiors
    are raw regex.
    """
    out, pos = [], 0
    for m in re.finditer(r"\{\{((?:(?!\}\}).)*)\}\}", msg, re.DOTALL):
        out.append(re.escape(msg[pos:m.start()]))
        out.append(f"(?:{m.group(1)})")
        pos = m.end()
    out.append(re.escape(msg[pos:]))
    return "".join(out)


def parse_count(count: str | None) -> tuple[float, float]:
    """`None`->(1,1); `2`->(2,2); `1+`->(1,inf); `0-1`->(0,1)."""
    if not count:
        return 1.0, 1.0
    if count.endswith("+"):
        return float(count[:-1]), float("inf")
    if "-" in count:
        lo, hi = count.split("-", 1)
        return float(lo), float(hi)
    n = float(count)
    return n, n


def check_diagnostics(expected: list[dict], active_prefixes: list[str],
                      stderr: str) -> tuple[bool, str]:
    """Apply the Clang `-verify` policy for one invocation.

    Only expectations whose `prefix` is active for this invocation are
    enforced (custom `-verify=expected,c,c23` prefixes, review P1-3).
    `no-diagnostics` fails on any diagnostic line.  Every other
    expectation must match its `msg_regex` between `count_min` and
    `count_max` times.
    """
    active = set(active_prefixes or ["expected"])
    for exp in expected:
        if exp.get("prefix", "expected") not in active:
            continue
        if exp.get("kind") == "no-diagnostics":
            if DIAG_LINE_RE.search(stderr):
                return False, "expected no diagnostics, got diagnostic lines"
            continue
        src = exp.get("msg_regex") or template_to_regex(exp.get("msg") or "")
        try:
            pat = re.compile(src, re.DOTALL)
        except re.error as exc:
            return False, f"bad expectation regex {src!r}: {exc}"
        lo, hi = (exp.get("count_min", 1), exp.get("count_max", 1))
        n = sum(1 for _ in pat.finditer(stderr))
        if n < lo or n > hi:
            return False, (f"expected {exp.get('kind')} x[{lo},{hi}] "
                           f"matching {src!r}, saw {n}")
    return True, ""


def plan_invocations(rec: dict, want_arch: str | None,
                     include_cpp: bool) -> list[dict]:
    """One plan entry per EDG `//options:` invocation (review P1-2)."""
    sets = rec.get("option_sets") or [""]
    langs = rec.get("languages_per_set") or [rec.get("language", "c")] * len(sets)
    flags_per = rec.get("gcc_flags_per_set") or [[] for _ in sets]
    pre_per = (rec.get("verify_prefixes_per_set")
               or [["expected"] for _ in sets])
    shared = rec.get("options_all") or []
    plan = []
    for i, opt in enumerate(sets):
        lang = langs[i] if i < len(langs) else "c"
        if lang != "c" and not include_cpp:
            plan.append({"set": i, "unsupported": f"{lang} invocation"})
            continue
        arch = rec.get("arch_tags") or []
        if arch and want_arch not in arch:
            plan.append({"set": i,
                         "unsupported": f"arch-specific: {','.join(arch)}"})
            continue
        if rec.get("phase") == "filecheck":
            plan.append({"set": i,
                         "unsupported": "filecheck phase not executed"})
            continue
        flags = list(shared) + list(flags_per[i] if i < len(flags_per) else [])
        plan.append({"set": i, "flags": flags,
                     "prefixes": pre_per[i] if i < len(pre_per) else
                     ["expected"]})
    return plan


def spawn_process(cmd: list[str], timeout: float):
    """Returns (returncode, stdout_bytes, stderr_bytes)."""
    proc = subprocess.run(cmd, capture_output=True, timeout=timeout)
    return proc.returncode, proc.stdout, proc.stderr


def run_one(rec: dict, corpus_root: Path, cc: str, timeout: float,
            syntax_only: bool, run_reject: bool, mode: str,
            xfail_as_pass: bool = False, want_arch: str | None = None,
            spawn=None) -> dict:
    spawn = spawn or spawn_process
    result = {"origin": rec["origin"], "category": rec["category"],
              "expect": rec["lccc_expect"], "status": None,
              "outcome": None, "detail": "",
              "untranslated_edg_flags": rec.get("untranslated_edg_flags", []),
              "phase": rec.get("phase", "compile")}
    expect = rec["lccc_expect"]
    if expect not in ("accept", "reject", "xfail"):
        result["status"] = "SKIP"
        result["detail"] = f"EDG type maps to {expect!r}"
        return result
    if not run_reject and expect == "reject":
        # Skip BEFORE any spawn (review P2: spawning can TIMEOUT a test we
        # never intended to evaluate).
        result["status"] = "SKIP"
        result["detail"] = "--no-reject"
        return result
    if rec.get("skip_reason") or "file" not in rec:
        result["status"] = "SKIP"
        result["detail"] = rec.get("skip_reason", "index-only")
        return result

    plan = plan_invocations(rec, want_arch, include_cpp=False)
    runnable = [p for p in plan if "unsupported" not in p]
    if not runnable:
        result["status"] = "UNSUPPORTED"
        result["detail"] = "; ".join(p["unsupported"] for p in plan) or "no invocations"
        return result

    test_file = corpus_root / rec["file"]
    obs_outcomes: list[Outcome] = []
    worst = None
    for p in runnable:
        cmd = shlex.split(cc) + p["flags"]
        if syntax_only:
            cmd += ["-fsyntax-only"]
        else:
            cmd += ["-c", "-o", os.devnull]
        cmd.append(str(test_file))
        try:
            rc, _out, err_b = spawn(cmd, timeout)
            stderr = err_b.decode("utf-8", "replace") \
                if isinstance(err_b, bytes) else str(err_b)
        except subprocess.TimeoutExpired:
            result.update(status="TIMEOUT", outcome="TIMEOUT",
                          detail=f">{timeout}s: {shlex.join(cmd)}")
            result["cmd"] = shlex.join(cmd)
            return result
        except OSError as exc:
            result.update(status="ERROR", outcome="ERROR", detail=str(exc))
            return result
        obs = classify(rc, stderr)
        obs_outcomes.append(obs)
        want_fail = expect in ("reject", "xfail")
        if obs in (Outcome.CRASH, Outcome.ERROR):
            result.update(status="FAIL", outcome=obs.value,
                          detail=f"{obs.value}: rc={rc}: "
                                 f"{stderr.strip()[:200]}")
            result["cmd"] = shlex.join(cmd)
            return result
        if want_fail and obs == Outcome.REJECT:
            ok, detail = check_diagnostics(rec.get("expected", []),
                                           p.get("prefixes"), stderr)
            if not ok:
                worst = (obs, detail)
        elif want_fail and obs == Outcome.ACCEPT:
            worst = (obs, "expected rejection, compiler accepted")
        elif not want_fail and obs == Outcome.REJECT:
            worst = (obs, f"expected acceptance, compiler rejected: "
                          f"{stderr.strip()[:200]}")
        elif not want_fail and obs == Outcome.ACCEPT:
            if mode == "diagnostics" and rec.get("expected"):
                ok, detail = check_diagnostics(rec.get("expected", []),
                                               p.get("prefixes"), stderr)
                if not ok:
                    worst = (obs, detail)

    outcome_name = (obs_outcomes[-1].value if obs_outcomes else "ERROR")
    result["outcome"] = outcome_name
    result["cmd"] = shlex.join(cmd) if runnable else ""
    if worst is None:
        result["status"] = "PASS" if expect != "xfail" else \
            ("PASS" if xfail_as_pass else "XPASS")
        return result
    obs, detail = worst
    result["detail"] = detail
    if expect == "xfail":
        result["status"] = "PASS" if xfail_as_pass else "XFAIL"
    else:
        result["status"] = "FAIL"
    return result


def is_divergence(cand: dict, ref: dict) -> bool:
    """Outcome mismatch, independent of diagnostic-policy verdicts."""
    return cand.get("outcome") != ref.get("outcome")


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--cc", default="lccc",
                    help="compiler command; receives translated EDG flags "
                         "plus -c or -fsyntax-only")
    ap.add_argument("--gcc-cmd", default=None,
                    help="reference compiler for an OBSERVED-OUTCOME "
                         "divergence report (informational)")
    ap.add_argument("--index", type=Path, default=CORPUS_INDEX)
    ap.add_argument("--category", default=None,
                    help="restrict to one category (%s)" %
                    ", ".join(VALID_CATEGORIES))
    ap.add_argument("--limit", type=int, default=0)
    ap.add_argument("--timeout", type=float, default=10.0)
    ap.add_argument("--syntax-only", action="store_true")
    ap.add_argument("--no-reject", action="store_true",
                    help="skip expected-rejection tests before any compile")
    ap.add_argument("--xfail-as-pass", action="store_true",
                    help="collapse XFAIL to PASS in the summary and never "
                         "report it separately")
    ap.add_argument("--mode", choices=["diagnostics", "outcome-only"],
                    default="diagnostics",
                    help="diagnostics: enforce -verify expectations where "
                         "declared; outcome-only: accept/reject only")
    ap.add_argument("--report", type=Path, default=None)
    ap.add_argument("-j", "--jobs", type=int,
                    default=max(1, (os.cpu_count() or 2) // 2))
    args = ap.parse_args(argv)

    if not args.index.is_file():
        print(f"error: {args.index} missing — run scripts/edg_corpus_mine.py "
              "clang-c first", file=sys.stderr)
        return 2
    data = json.loads(args.index.read_text(encoding="utf-8"))
    corpus_root = args.index.parent.parent  # tests/corpus
    files = data["files"]
    if args.category:
        known = sorted({f["category"] for f in files})
        if args.category not in known:
            print(f"error: unknown category {args.category!r}; "
                  f"known: {', '.join(known)}", file=sys.stderr)
            return 2
        files = [f for f in files if f["category"] == args.category]
    if args.limit:
        files = files[: args.limit]

    results = []
    t0 = time.monotonic()
    with cf.ThreadPoolExecutor(max_workers=max(1, args.jobs)) as pool:
        futs = [pool.submit(run_one, rec, corpus_root, args.cc,
                            args.timeout, args.syntax_only,
                            not args.no_reject, args.mode,
                            args.xfail_as_pass) for rec in files]
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
          f"({args.jobs} jobs, timeout {args.timeout}s, mode={args.mode})")
    for cat in sorted(by_cat):
        row = ", ".join(f"{k}={v}" for k, v in sorted(by_cat[cat].items()))
        print(f"  {cat:14s}: {row}")
    print("  total        : " + ", ".join(
        f"{k}={v}" for k, v in sorted(by_status.items())))

    divergences = []
    if args.gcc_cmd:
        for r in results:
            if r["status"] in ("PASS", "FAIL", "XFAIL"):
                rec = next(f for f in files if f["origin"] == r["origin"])
                g = run_one(rec, corpus_root, args.gcc_cmd, args.timeout,
                            args.syntax_only, not args.no_reject,
                            "outcome-only")
                if is_divergence(r, g):
                    divergences.append({"test": r["origin"],
                                        "candidate": r["outcome"],
                                        "reference": g["outcome"]})
        print(f"  outcome divergences vs reference (informational): "
              f"{len(divergences)}")

    if args.report:
        args.report.write_text(json.dumps(
            {"cc": args.cc, "wall_s": round(wall, 2), "mode": args.mode,
             "by_status": by_status, "by_category": by_cat,
             "outcome_divergences": divergences,
             "failures": [r for r in results
                          if r["status"] in ("FAIL", "TIMEOUT", "ERROR")]},
            indent=2, sort_keys=True) + "\n", encoding="utf-8")
        print(f"  report -> {args.report}")

    hard = sum(by_status.get(k, 0) for k in ("FAIL", "TIMEOUT", "ERROR"))
    return 1 if hard else 0


if __name__ == "__main__":
    sys.exit(main())
