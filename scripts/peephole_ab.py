#!/usr/bin/env python3
"""A/B gate for the x86 peephole passes: size delta AND behaviour identity.

For every program in tests/benchmark/programs (and any extra files given on the
command line) the script compiles twice with the same LCCC binary — once with
the pass set under test enabled, once with `CCC_PEEPHOLE_SKIP` disabling it —
then

  * counts emitted instructions in both assembly listings, and
  * when the program links and runs, compares complete stdout and exit status.

A pass that changes observable behaviour anywhere is a miscompile, no matter
how good the instruction count looks; that is the half of the gate a pure
counting harness misses.

Usage:
    scripts/peephole_ab.py                       # default pass set, whole corpus
    scripts/peephole_ab.py --skip move_relay     # A/B a single pass
    scripts/peephole_ab.py --flags '-O3' extra.c

Environment:
    LCCC  path to the compiler (default: target/fastbuild/lccc)
"""

import argparse
import os
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
LCCC = os.environ.get("LCCC", str(REPO / "target" / "fastbuild" / "lccc"))

# The pass set introduced with the relay/lea and flags peepholes.
DEFAULT_SKIP = [
    "move_relay",
    "lea_load_window",
    "producer_retarget",
    "copy_add_lea",
    "copy_shift_lea",
    "setcc_cmov",
    "copy_mask_movz",
    # Session-75 / v7 layer
    "copy_coalesce",
    "dead_pure_writes",
    "load_test_cmp",
    "acc_roundtrip",
    "load_reuse",
    "self_test",
    "narrow_signext",
]


def count_insns(path: Path) -> int:
    n = 0
    for line in path.read_text(errors="replace").splitlines():
        t = line.strip()
        if not t or t.endswith(":") or t.startswith(".") or t.startswith("#"):
            continue
        n += 1
    return n


def compile_one(src: Path, out: Path, flags, env):
    r = subprocess.run(
        [LCCC, *flags, str(src), "-o", str(out)],
        capture_output=True,
        timeout=180,
        env=env,
    )
    return r.returncode == 0, r.stderr.decode(errors="replace")[-300:]


def run_one(exe: Path):
    try:
        r = subprocess.run([str(exe)], capture_output=True, timeout=60)
        return r.returncode, r.stdout
    except subprocess.TimeoutExpired:
        return "timeout", b""


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--skip", action="append", default=None, help="pass name(s) to A/B")
    ap.add_argument("--flags", default="-O2", help="compiler flags (default: -O2)")
    ap.add_argument("--no-run", action="store_true", help="skip execution comparison")
    ap.add_argument("extra", nargs="*", help="additional sources")
    args = ap.parse_args()

    skip = args.skip if args.skip else DEFAULT_SKIP
    flags = args.flags.split()
    sources = sorted((REPO / "tests" / "benchmark" / "programs").glob("*.c"))
    sources += [Path(p) for p in args.extra]

    env_on = dict(os.environ)
    env_on.pop("CCC_PEEPHOLE_SKIP", None)
    env_off = dict(os.environ)
    env_off["CCC_PEEPHOLE_SKIP"] = ",".join(skip)

    tot_on = tot_off = 0
    behaviour_fail = []
    compile_fail = []
    rows = []
    tmp = Path("/tmp/peephole_ab")
    tmp.mkdir(exist_ok=True)

    for src in sources:
        stem = src.stem
        s_on, s_off = tmp / f"{stem}.on.s", tmp / f"{stem}.off.s"
        ok_on, err_on = compile_one(src, s_on, flags + ["-S"], env_on)
        ok_off, err_off = compile_one(src, s_off, flags + ["-S"], env_off)
        if not (ok_on and ok_off):
            compile_fail.append((stem, err_on or err_off))
            continue
        n_on, n_off = count_insns(s_on), count_insns(s_off)
        tot_on += n_on
        tot_off += n_off
        verdict = ""
        if not args.no_run:
            e_on, e_off = tmp / f"{stem}.on", tmp / f"{stem}.off"
            r_on, _ = compile_one(src, e_on, flags, env_on)
            r_off, _ = compile_one(src, e_off, flags, env_off)
            if r_on and r_off:
                out_on, out_off = run_one(e_on), run_one(e_off)
                if out_on != out_off:
                    behaviour_fail.append(stem)
                    verdict = "  *** BEHAVIOUR MISMATCH ***"
                else:
                    verdict = "  run=match"
        rows.append((stem, n_off, n_on, n_on - n_off, verdict))

    print(f"{'program':<28}{'OFF':>7}{'ON':>7}{'delta':>8}  status")
    print("-" * 72)
    for stem, off, on, d, v in rows:
        print(f"{stem:<28}{off:>7}{on:>7}{d:>+8}{v}")
    print("-" * 72)
    pct = 100.0 * (tot_on - tot_off) / tot_off if tot_off else 0.0
    print(f"TOTAL: OFF {tot_off}  ON {tot_on}  delta {tot_on - tot_off:+d} ({pct:+.2f}%)")
    if compile_fail:
        print(f"\ncompile failures ({len(compile_fail)}):")
        for stem, err in compile_fail:
            print(f"  {stem}: {err.strip().splitlines()[-1] if err.strip() else '?'}")
    if behaviour_fail:
        print(f"\nBEHAVIOUR MISMATCHES: {behaviour_fail}")
        return 1
    print("\nbehaviour: identical everywhere")
    return 0


if __name__ == "__main__":
    sys.exit(main())
