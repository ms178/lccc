#!/usr/bin/env python3
"""Missed-alarm pairs from real YARPGen (the blog-faithful transplant).

Regehr & Livinskii, "Looking for Missed Alarm Bugs in a Formal
Verification Tool" (https://blog.regehr.org/archives/2124), needed pairs
of programs that are *known* to behave differently: take a YARPGen
function (UB-free by construction), mutate it inside the generator --
where value ranges are tracked, so UB-freedom is preserved -- keep only
pairs whose observable behaviour differs (execution filter), and require
the checker to tell them apart.

scripts/mutpair.py transplants that method with a dependency-free
miniature generator in the YARPGen role.  This script is the same method
on the real generator, for hosts that have it (scripts/ensure_yarpgen.sh
provisions a pinned intel/yarpgen): modern YARPGen ships the mutation
half upstream as `--mutate=exprs --mutation-seed=N`, i.e. same --seed,
generator-internal expression mutations under the same UB-freedom
guarantee as base generation.  What this harness adds is the pair
discipline the blog post is about:

  1. Generate orig (seed S) and mutant (seed S, mutation-seed M).
     init.h/driver.c are shared in practice; func.c carries the
     mutation.  Textually identical func.c (modulo the invocation
     comment) -> discard.
  2. Reference consensus: compile+run both members under the reference
     compilers.  Any failure/disagreement -> SKIP (counted, never
     judged).  Equal hashes -> discard: YARPGen inputs are fixed, so
     equal output is an observably-equivalent pair and carries no
     missed-alarm signal here.
  3. The survivor is a *known-inequivalent, generator-UB-free pair*.
     LCCC (every requested -O level) must reproduce both reference
     hashes.  The diagnostic flavours are reported separately: a plain
     mismatch vs the specific missed-alarm shape -- LCCC collapsing the
     pair to one hash -- because they point at different optimiser bugs.

Honesty about sensitivity: orig and mutant are separate translation
units here, so the optimiser sees them independently (mutpair.py's
same-TU pairs are sharper per case; these programs are far richer).
A same-TU merge (init(); test(); hash; re-init; test_mut(); hash) is
possible because driver.c separates init/test/check, and is tracked as
follow-up work rather than faked here.

Usage:
    scripts/yarpgen_mutpair.py --self-test
    scripts/yarpgen_mutpair.py --count 20 --seed 1 --lccc target/fastbuild/lccc
    scripts/yarpgen_mutpair.py --ci-smoke --lccc target/fastbuild/lccc
        # CI mode: explicit SKIP (exit 0) when no yarpgen is provisioned.

Exit status is non-zero iff a judged pair diverges (or, outside
--ci-smoke, no yarpgen binary can be resolved).
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
HASH_RE = re.compile(r"^\d+$")


def resolve_yarpgen(explicit: str | None) -> str | None:
    """--yarpgen > $YARPGEN > provisioned cache (ensure_yarpgen.sh) > PATH."""
    candidates: list[str] = []
    if explicit:
        candidates.append(explicit)
    if os.environ.get("YARPGEN"):
        candidates.append(os.environ["YARPGEN"])
    ensure = REPO / "scripts/ensure_yarpgen.sh"
    if ensure.is_file():
        try:
            r = subprocess.run([str(ensure), "--check", "--print-bin"],
                               capture_output=True, text=True, timeout=120)
            if r.returncode == 0 and r.stdout.strip():
                candidates.append(r.stdout.strip().splitlines()[-1])
        except (OSError, subprocess.TimeoutExpired):
            pass
    candidates.append("yarpgen")
    for cand in candidates:
        if shutil.which(cand) or Path(cand).is_file():
            return cand
    return None


def func_body(text: str) -> str:
    """func.c modulo the invocation comment (the only seed-independent
    line that always differs between orig and mutant)."""
    return "\n".join(l for l in text.splitlines()
                     if not l.startswith("Invocation:"))


def parse_hash(stdout: str) -> int | None:
    s = stdout.strip()
    return int(s) if HASH_RE.match(s) else None


def classify(ref_o: int, ref_m: int, got_o: int | None, got_m: int | None) -> str:
    """Verdict for one compiler on one known-inequivalent pair.
    Pure function; the unit tests pin its whole truth table."""
    if got_o is None or got_m is None:
        return "build/run failure"
    if got_o == ref_o and got_m == ref_m:
        return "pass"
    if got_o == got_m:
        return "COLLAPSED an inequivalent pair (missed alarm)"
    return "hash mismatch"


def generate(yarpgen: str, seed: int, mseed: int | None, outdir: Path,
             timeout: float) -> bool:
    outdir.mkdir(parents=True, exist_ok=True)
    cmd = [yarpgen, "--std=c", f"--seed={seed}", "--max-array-dims=3",
           "-o", str(outdir)]
    if mseed is not None:
        cmd += ["--mutate=exprs", f"--mutation-seed={mseed}"]
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        return False
    return r.returncode == 0 and all(
        (outdir / n).is_file() for n in ("driver.c", "func.c", "init.h"))


def build_run_hash(cc: list[str], case: Path, opt: str, exe: Path,
                   timeout: float) -> int | None:
    try:
        b = subprocess.run(
            [*cc, opt, "-std=c99", "-w", str(case / "driver.c"),
             str(case / "func.c"), "-o", str(exe), "-lm"],
            capture_output=True, text=True, timeout=timeout)
        if b.returncode != 0:
            return None
        r = subprocess.run([str(exe)], capture_output=True, text=True,
                           timeout=timeout)
    except subprocess.TimeoutExpired:
        return None
    return parse_hash(r.stdout) if r.returncode == 0 else None


def judge_pair(orig: Path, mut: Path, ref_o: int, ref_m: int,
               compilers: dict[str, list[str]], opts: list[str],
               tmp: Path, timeout: float) -> list[str]:
    failures: list[str] = []
    for name, cc in compilers.items():
        for opt in opts:
            got_o = build_run_hash(cc, orig, opt, tmp / "o.bin", timeout)
            got_m = build_run_hash(cc, mut, opt, tmp / "m.bin", timeout)
            verdict = classify(ref_o, ref_m, got_o, got_m)
            if verdict != "pass":
                failures.append(
                    f"{name} {opt}: {verdict} "
                    f"(ref {ref_o}/{ref_m}, got {got_o}/{got_m})")
    return failures


def run_pairs(yarpgen: str, count: int, seed0: int,
              compilers: dict[str, list[str]], refs: list[str],
              opts: list[str], repro_dir: Path, timeout: float,
              max_attempts_factor: int = 30) -> tuple[int, int, dict]:
    """Judge `count` kept pairs.  Returns (judged, failures, stats)."""
    stats = {"attempts": 0, "identical": 0, "equal_hash": 0, "skip": 0}
    judged = failures = 0
    seed = seed0
    max_attempts = count * max_attempts_factor + 100
    while judged < count:
        stats["attempts"] += 1
        if stats["attempts"] > max_attempts:
            raise RuntimeError(
                f"pair filter starved: judged={judged} stats={stats}")
        mseed = seed * 2654435761 % (2**31) + 1
        with tempfile.TemporaryDirectory(prefix="ygpair_") as td:
            tmp = Path(td)
            orig, mut = tmp / "orig", tmp / "mut"
            ok = generate(yarpgen, seed, None, orig, timeout) and \
                generate(yarpgen, seed, mseed, mut, timeout)
            this_seed, seed = seed, seed + 1
            if not ok:
                stats["skip"] += 1
                continue
            if func_body((orig / "func.c").read_text(errors="replace")) == \
                    func_body((mut / "func.c").read_text(errors="replace")):
                stats["identical"] += 1
                continue
            # Reference consensus at -O0 and -O2 per reference compiler.
            ref_o = ref_m = None
            consistent = True
            for ref in refs:
                for opt in ("-O0", "-O2"):
                    ho = build_run_hash([ref], orig, opt, tmp / "ro.bin", timeout)
                    hm = build_run_hash([ref], mut, opt, tmp / "rm.bin", timeout)
                    if ho is None or hm is None:
                        consistent = False
                        break
                    if ref_o is None:
                        ref_o, ref_m = ho, hm
                    elif (ho, hm) != (ref_o, ref_m):
                        consistent = False
                        break
                if not consistent:
                    break
            if not consistent or ref_o is None or ref_m is None:
                stats["skip"] += 1
                continue
            if ref_o == ref_m:
                stats["equal_hash"] += 1
                continue
            judged += 1
            fails = judge_pair(orig, mut, ref_o, ref_m, compilers, opts,
                               tmp, timeout)
            if fails:
                failures += 1
                dest = repro_dir / f"yarpgen_mutpair_{this_seed}_{mseed}"
                for src, name in ((orig, "orig"), (mut, "mut")):
                    d = dest / name
                    d.mkdir(parents=True, exist_ok=True)
                    for f in src.iterdir():
                        if f.is_file():
                            shutil.copy(f, d / f.name)
                print(f"X seed={this_seed} mseed={mseed}: {fails[0]}")
                for f in fails[1:]:
                    print(f"    {f}")
                print(f"    repro: {dest}")
            else:
                print(".", end="", flush=True)
    return judged, failures, stats


def self_test() -> int:
    problems: list[str] = []
    # classify() truth table.
    cases = {
        (1, 2, 1, 2): "pass",
        (1, 2, 1, 1): "COLLAPSED an inequivalent pair (missed alarm)",
        (1, 2, 2, 2): "COLLAPSED an inequivalent pair (missed alarm)",
        (1, 2, 3, 2): "hash mismatch",
        (1, 2, None, 2): "build/run failure",
    }
    for args, want in cases.items():
        if classify(*args) != want:
            problems.append(f"classify{args} != {want!r}")
    if parse_hash(" 6871103402060576105\n") != 6871103402060576105:
        problems.append("parse_hash rejects a plain decimal hash")
    if parse_hash("Segmentation fault") is not None:
        problems.append("parse_hash accepts non-hash output")
    if func_body("Invocation: x\nbody\n") != "body":
        problems.append("func_body does not strip the invocation line")
    # Live plumbing (only when the real generator + gcc exist): one kept
    # pair judged with gcc-as-compiler-under-test must pass, and the
    # collapse simulation -- mutant judged in the orig slot -- must be
    # reported.  A harness that cannot report a planted collapse has a
    # missed-alarm hole of its own.
    yarpgen = resolve_yarpgen(None)
    gcc = shutil.which("gcc")
    if yarpgen and gcc:
        with tempfile.TemporaryDirectory(prefix="ygpair_self_") as td:
            tmp = Path(td)
            kept = None
            for seed in range(1, 40):
                orig, mut = tmp / f"o{seed}", tmp / f"m{seed}"
                if not (generate(yarpgen, seed, None, orig, 60)
                        and generate(yarpgen, seed, seed * 7 + 1, mut, 60)):
                    continue
                if func_body((orig / "func.c").read_text(errors="replace")) == \
                        func_body((mut / "func.c").read_text(errors="replace")):
                    continue
                ro = build_run_hash([gcc], orig, "-O0", tmp / "a.bin", 120)
                rm = build_run_hash([gcc], mut, "-O0", tmp / "b.bin", 120)
                if ro is not None and rm is not None and ro != rm:
                    kept = (orig, mut, ro, rm)
                    break
            if kept is None:
                problems.append("no kept pair in 39 seeds (generator/filter broken?)")
            else:
                orig, mut, ro, rm = kept
                if judge_pair(orig, mut, ro, rm, {"gcc": [gcc]}, ["-O2"], tmp, 120):
                    problems.append("gcc-as-DUT failed on a kept pair (plumbing)")
                collapsed = judge_pair(mut, mut, ro, rm, {"gcc": [gcc]},
                                       ["-O0"], tmp, 120)
                if not any("COLLAPSED" in f or "mismatch" in f for f in collapsed):
                    problems.append("planted collapse NOT reported (missed alarm)")
    else:
        print("note: yarpgen or gcc absent; live plumbing checks skipped")
    for p in problems:
        print(f"  FAIL: {p}")
    print("yarpgen_mutpair self-test: " + ("FAIL" if problems else "PASS"))
    return 1 if problems else 0


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--self-test", action="store_true")
    ap.add_argument("--ci-smoke", action="store_true",
                    help="judge 3 pairs if yarpgen is provisioned; explicit SKIP (exit 0) otherwise")
    ap.add_argument("--count", type=int, default=20, help="kept pairs to judge")
    ap.add_argument("--seed", type=int, default=1, help="first yarpgen seed")
    ap.add_argument("--yarpgen", help="yarpgen binary (default: resolve)")
    ap.add_argument("--lccc", default=str(REPO / "target/fastbuild/lccc"))
    ap.add_argument("--refs", default="gcc")
    ap.add_argument("--opts", default="-O0,-O2")
    ap.add_argument("--timeout", type=float, default=120.0,
                    help="per compile/run timeout (yarpgen cases are large)")
    ap.add_argument("--repro-dir", default=str(REPO / "artifacts/repros"))
    ap.add_argument("--json", dest="json_out")
    args = ap.parse_args(argv)
    if args.self_test:
        return self_test()

    yarpgen = resolve_yarpgen(args.yarpgen)
    if yarpgen is None:
        if args.ci_smoke:
            print("SKIP yarpgen-pair-smoke: no yarpgen binary "
                  "(provision with scripts/ensure_yarpgen.sh)")
            return 0
        print("error: yarpgen binary not found; run scripts/ensure_yarpgen.sh "
              "or pass --yarpgen", file=sys.stderr)
        return 2
    refs = [r.strip() for r in args.refs.split(",") if r.strip()
            and shutil.which(r.strip())]
    if not refs:
        print("error: no reference compiler available", file=sys.stderr)
        return 2
    compilers: dict[str, list[str]] = {}
    if Path(args.lccc).exists():
        compilers["lccc"] = [args.lccc]
    else:
        print(f"WARNING: lccc not found at {args.lccc}; judging references "
              "against themselves (plumbing check only)", file=sys.stderr)
        compilers[refs[0]] = [refs[0]]
    opts = [o.strip() for o in args.opts.split(",") if o.strip()]
    count = 3 if args.ci_smoke else args.count
    try:
        judged, failures, stats = run_pairs(
            yarpgen, count, args.seed, compilers, refs, opts,
            Path(args.repro_dir), args.timeout)
    except RuntimeError as e:
        print(f"error: {e}", file=sys.stderr)
        return 2
    print(f"\nyarpgen_mutpair: judged={judged} failures={failures} "
          f"stats={stats} compilers={','.join(compilers)} opts={','.join(opts)}")
    if args.json_out:
        Path(args.json_out).write_text(json.dumps(
            {"judged": judged, "failures": failures, "stats": stats,
             "compilers": list(compilers), "opts": opts}, indent=2))
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
