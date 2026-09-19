#!/usr/bin/env python3
"""Flag-consumer A/B: identity, code quality and compile time in one sweep.

A soundness fix to a peephole is only half a fix if it silently costs code
quality, and "it feels the same" is not a measurement. This answers the three
questions a flag-reasoning change has to answer, over every TU under `tests/`:

1. IDENTITY     which TUs change emitted assembly at all (and the diff is
                printed, so a change is read rather than counted);
2. QUALITY      total `testb $` byte-form tests and `cmpb $0` memory compares.
                These are the two folds the SF-consumer guard licenses, so their
                totals say whether a narrower guard gave back what a coarser one
                refused. `--insns` additionally counts emitted instruction lines
                (not labels, directives or comments), which is the metric that
                matters for the i686 fusions;
3. COMPILE TIME wall time per binary, PAIRED and interleaved -- both arms
                alternate every sweep, so thermal or background drift hits them
                equally -- reported as min and median of `--runs`.

Usage:
  scripts/flag_consumer_ab.py BIN_PRE BIN_POST [--flags "-O2"] [--runs 3]
                              [--insns] [--repo /path/to/lccc]

Environment: AB_FLAGS is honoured when --flags is absent. Exit status is 1 if
any TU differs, so this can gate a change as well as describe it.
"""
import argparse
import hashlib
import os
import statistics
import subprocess
import sys
import time


def corpus(repo):
    out = []
    for dp, dns, fns in os.walk(os.path.join(repo, "tests")):
        if "/target/" in dp or "/.git" in dp:
            continue
        for fn in sorted(fns):
            if fn.endswith((".c", ".cpp", ".cc")):
                out.append(os.path.relpath(os.path.join(dp, fn), repo))
    return sorted(out)


def gen(binary, src, out_s, flags, repo):
    t0 = time.perf_counter()
    r = subprocess.run(
        [binary, "-S", *flags, src, "-o", out_s],
        capture_output=True,
        text=True,
        timeout=600,
        cwd=repo,
    )
    dt = time.perf_counter() - t0
    if r.returncode != 0 or not os.path.exists(out_s):
        return None, dt
    with open(out_s, encoding="utf-8", errors="replace") as fh:
        return fh.read(), dt


def nins(text):
    """Emitted instruction lines: not blank, not a label, not a directive."""
    n = 0
    for ln in text.splitlines():
        s = ln.strip()
        if not s or s.endswith(":") or s.startswith((".", "#", "//", "/*")):
            continue
        n += 1
    return n


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("pre")
    ap.add_argument("post")
    ap.add_argument("--flags", default=os.environ.get("AB_FLAGS", "-O2"))
    ap.add_argument("--runs", type=int, default=2)
    ap.add_argument("--insns", action="store_true")
    ap.add_argument("--repo", default=os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    ap.add_argument("--tmp", default="/var/tmp/flag_consumer_ab")
    ap.add_argument("--max-diffs", type=int, default=25)
    a = ap.parse_args()
    flags = a.flags.split()
    repo = a.repo
    os.makedirs(a.tmp, exist_ok=True)
    sa, sb = os.path.join(a.tmp, "a.s"), os.path.join(a.tmp, "b.s")

    srcs = corpus(repo)
    same = diff = failed = 0
    qa = qb = ca = cb = ia = ib = 0
    rows = []
    for src in srcs:
        ta, _ = gen(a.pre, src, sa, flags, repo)
        tb, _ = gen(a.post, src, sb, flags, repo)
        if ta is None and tb is None:
            failed += 1
            continue
        ha = hashlib.sha256(ta.encode()).hexdigest()
        hb = hashlib.sha256(tb.encode()).hexdigest()
        qa += ta.count("testb $")
        qb += tb.count("testb $")
        ca += ta.count("cmpb $0")
        cb += tb.count("cmpb $0")
        if a.insns:
            ia += nins(ta)
            ib += nins(tb)
        if ha == hb:
            same += 1
        else:
            diff += 1
            rows.append((src, ta.count("testb $"), tb.count("testb $"), nins(ta), nins(tb)))

    print(f"pre ={a.pre}\npost={a.post}\nflags={a.flags}\nrepo={repo}\nTUs={len(srcs)}")
    print(f"identity : identical={same} differing={diff} both-failed={failed}")
    print(f"quality  : testb $ pre={qa} post={qb} delta={qb - qa:+d}")
    print(f"quality  : cmpb $0 pre={ca} post={cb} delta={cb - ca:+d}")
    if a.insns:
        pct = 100 * (ib / ia - 1) if ia else 0.0
        print(f"quality  : instructions pre={ia} post={ib} delta={ib - ia:+d} ({pct:+.3f}%)")
    for src, x1, x2, n1, n2 in rows[: a.max_diffs]:
        print(f"   DIFF {src}  testb {x1}->{x2}  insns {n1}->{n2}")
        da, db = os.path.join(a.tmp, "da.s"), os.path.join(a.tmp, "db.s")
        gen(a.pre, src, da, flags, repo)
        gen(a.post, src, db, flags, repo)
        subprocess.run(f"diff {da} {db} | grep -E '^[<>]' | head -6", shell=True, cwd=repo)

    if a.runs:
        best = {}
        samples = {}
        for r in range(a.runs):
            for tag, binary in (("pre", a.pre), ("post", a.post)):
                out = os.path.join(a.tmp, f"t_{tag}.s")
                t0 = time.perf_counter()
                for src in srcs:
                    subprocess.run(
                        [binary, "-S", *flags, src, "-o", out],
                        capture_output=True,
                        timeout=600,
                        cwd=repo,
                    )
                dt = time.perf_counter() - t0
                samples.setdefault(tag, []).append(dt)
                best[tag] = min(best.get(tag, float("inf")), dt)
                print(f"   run {r} {tag}: {dt:.2f}s", file=sys.stderr)
        for tag in ("pre", "post"):
            print(f"time     : {tag} min={best[tag]:.2f}s "
                  f"median={statistics.median(samples[tag]):.2f}s")
        print(f"time     : delta(min)={100 * (best['post'] / best['pre'] - 1):+.2f}% "
              f"delta(median)="
              f"{100 * (statistics.median(samples['post']) / statistics.median(samples['pre']) - 1):+.2f}%")
    sys.exit(1 if diff else 0)


if __name__ == "__main__":
    main()
