#!/usr/bin/env python3
"""Interleaved A/B for two lccc binaries on one kernel.

The box this runs on is noisy enough that running arm A to completion and then
arm B lets slow drift masquerade as a speedup. This alternates the two arms
sample by sample, so any drift hits both equally, and reports the MINIMUM of
each arm -- the sample least perturbed by scheduler noise, and the right
statistic for a one-sided distribution.

Also reports a paired win-rate (how often the optimised arm was faster in the
same adjacent pair), which is far more robust than comparing two minima when
the effect is smaller than the run-to-run spread.
"""
import argparse, re, statistics, subprocess, sys, tempfile, os
from pathlib import Path

def build(cc, kern, out, flags, repo, env=None, program=None, defines=()):
    """Compile one arm.

    Kernel mode (the default) links the shared driver against
    tests/bench/k_<kern>.c.  --program builds a standalone source instead, so
    the real workload programs in tests/benchmark/programs can be measured the
    same way.

    --a-env/--b-env and --defines exist so the two arms can differ by a kill
    switch or a -D on the SAME compiler binary.  That is the honest way to
    price one transformation: A/B against a stale binary also prices every
    unrelated change that landed in between, which is how a "speedup" that no
    single commit delivers gets published.
    """
    src = ([f"{repo}/{program}"] if program
           else [f"{repo}/tests/bench/driver.c", f"{repo}/tests/bench/k_{kern}.c"])
    run_env = dict(os.environ)
    run_env.update(env or {})
    cmd = [cc, *flags, *[f"-D{d}" for d in defines], "-w", *src, "-o", out]
    r = subprocess.run(cmd, capture_output=True, text=True, env=run_env)
    if r.returncode != 0:
        sys.exit(f"build failed for {cc} {program or kern}: {r.stderr[:400]}")

def sample_program(binary, cpu, reps=1):
    """Time a standalone program.

    tests/benchmark/programs entries print a checksum and nothing else -- no
    driver, no "<seconds> <checksum>" line -- so the wall clock has to be taken
    here.  Process start-up is inside the measurement; with the corpus's
    PASSES-sized loops it is noise, and it is noise that hits both arms.
    """
    import time
    best = None
    out = ""
    for _ in range(reps):
        t0 = time.monotonic()
        r = subprocess.run(["taskset", "-c", cpu, binary], capture_output=True, text=True)
        dt = time.monotonic() - t0
        if r.returncode != 0:
            sys.exit(f"{binary} exited {r.returncode}: {r.stderr[:200]}")
        out = r.stdout.strip()
        best = dt if best is None else min(best, dt)
    return best, out


def sample(binary, cpu):
    r = subprocess.run(["taskset", "-c", cpu, binary], capture_output=True, text=True)
    m = re.search(r"([0-9.]+)\s+(\d+)", r.stdout)
    if not m:
        sys.exit(f"cannot parse driver output: {r.stdout!r}")
    return float(m.group(1)), int(m.group(2))

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--a", required=True); ap.add_argument("--b", required=True)
    ap.add_argument("--kernel", help="kernel name (tests/bench/k_<name>.c); "
                                     "required unless --program is given")
    ap.add_argument("--samples", type=int, default=41)
    ap.add_argument("--flags", default="-O3 -march=x86-64-v3")
    ap.add_argument("--cpu", default="0")
    # Default to the repository this script lives in. The old absolute
    # "/home/user/lccc" made the tool fail for every other checkout and
    # every other user -- a measurement harness that only works on the
    # machine that wrote it is not a measurement harness.
    ap.add_argument("--repo", default=str(Path(__file__).resolve().parents[1]))
    ap.add_argument("--program",
                    help="standalone source to build instead of the driver + "
                         "k_<kernel> pair, e.g. tests/benchmark/programs/lz4_compress.c")
    ap.add_argument("--defines", default="",
                    help="comma-separated -D names applied to both arms")
    ap.add_argument("--a-env", default="", metavar="NAME=VALUE[,NAME=VALUE]",
                    help="environment for arm A only (kill switches etc.)")
    ap.add_argument("--b-env", default="", metavar="NAME=VALUE[,NAME=VALUE]",
                    help="environment for arm B only")
    a = ap.parse_args()
    if not a.kernel and not a.program:
        ap.error("give --kernel or --program")

    flags = a.flags.split()
    defines = [d for d in a.defines.split(",") if d]
    a_env = dict(kv.split("=", 1) for kv in a.a_env.split(",") if kv)
    b_env = dict(kv.split("=", 1) for kv in a.b_env.split(",") if kv)
    ta, tb = [], []
    ta_c = tb_c = None
    wins = 0
    take = (lambda b: sample_program(b, a.cpu)) if a.program else (lambda b: sample(b, a.cpu))
    with tempfile.TemporaryDirectory() as d:
        ba, bb = os.path.join(d, "a"), os.path.join(d, "b")
        build(a.a, a.kernel, ba, flags, a.repo, env=a_env,
              program=a.program, defines=defines)
        build(a.b, a.kernel, bb, flags, a.repo, env=b_env,
              program=a.program, defines=defines)
        # warm the page cache / branch predictors equally
        for _ in range(3):
            take(ba); take(bb)
        for i in range(a.samples):
            if i % 2 == 0:
                ta.append(take(ba)[0]); tb.append(take(bb)[0])
            else:
                tb.append(take(bb)[0]); ta.append(take(ba)[0])
        # checksums must agree
        _, ta_c = take(ba); _, tb_c = take(bb)

    if ta_c != tb_c:
        print(f"CHECKSUM MISMATCH: base={ta_c} opt={tb_c}", file=sys.stderr)
        return 2

    pairs = [(x, y) for x, y in zip(ta, tb)]
    wins = sum(1 for x, y in pairs if y < x)
    loss = sum(1 for x, y in pairs if y > x)
    label = a.program or a.kernel
    parts = [f"-D{d}" for d in defines]
    if a.a_env:
        parts.append("A:" + a.a_env)
    if a.b_env:
        parts.append("B:" + a.b_env)
    extra = " ".join(parts)
    print(f"target            : {label}   ({a.samples} interleaved samples, {flags}"
          f"{', ' + extra if extra else ''})")
    print(f"arm A             : {a.a}{'  [' + a.a_env + ']' if a.a_env else ''}")
    print(f"arm B             : {a.b}{'  [' + a.b_env + ']' if a.b_env else ''}")
    print(f"checksum          : {ta_c} (identical)")
    print(f"base  min/median  : {min(ta)*1000:8.3f} / {statistics.median(ta)*1000:8.3f} ms")
    print(f"opt   min/median  : {min(tb)*1000:8.3f} / {statistics.median(tb)*1000:8.3f} ms")
    print(f"delta on min      : {(min(tb)/min(ta)-1)*100:+.2f} %")
    print(f"delta on median   : {(statistics.median(tb)/statistics.median(ta)-1)*100:+.2f} %")
    print(f"paired win/loss   : {wins} / {loss}  ({wins/len(pairs)*100:.0f}% faster)")
    return 0

if __name__ == "__main__":
    sys.exit(main())
