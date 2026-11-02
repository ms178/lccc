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
import argparse, json, math, re, statistics, subprocess, sys, tempfile, os
from pathlib import Path


class BuildFailure(RuntimeError):
    """A compiler invocation failed; ``record`` is the complete receipt."""

    def __init__(self, message, record):
        super().__init__(message)
        self.record = record


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
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, env=run_env)
    except (OSError, subprocess.SubprocessError) as exc:
        record = {
            "compiler": cc,
            "command": cmd,
            "env_overrides": dict(env or {}),
            "returncode": None,
            "stdout": "",
            "stderr": "",
            "error": str(exc),
        }
        raise BuildFailure(f"build invocation failed for {cc} {program or kern}: {exc}", record) from exc
    record = {
        "compiler": cc,
        "command": cmd,
        "env_overrides": dict(env or {}),
        "returncode": r.returncode,
        "stdout": r.stdout,
        "stderr": r.stderr,
    }
    if r.returncode != 0:
        raise BuildFailure(
            f"build failed for {cc} {program or kern}: {r.stderr[:400]}", record
        )
    return record

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
            raise ValueError(f"{binary} exited {r.returncode}: {r.stderr[:200]}")
        out = r.stdout.strip()
        best = dt if best is None else min(best, dt)
    return best, out


def sample(binary, cpu):
    r = subprocess.run(["taskset", "-c", cpu, binary], capture_output=True, text=True)
    if r.returncode != 0:
        raise ValueError(f"{binary} exited {r.returncode}: {r.stderr[:200]}")
    m = re.fullmatch(r"\s*([0-9.]+)\s+(\d+)\s*", r.stdout)
    if not m:
        raise ValueError(f"cannot parse driver output: {r.stdout!r}")
    return float(m.group(1)), int(m.group(2))

def checked_sample(take, binary, expected, records, round_index, arm, order, phase):
    """Every invocation gets a receipt, including exceptions and bad output."""
    try:
        elapsed, checksum = take(binary)
    except (ValueError, OSError, subprocess.SubprocessError) as exc:
        records.append(
            dict(
                round=round_index,
                arm=arm,
                order=order,
                phase=phase,
                rc=None,
                error=str(exc),
            )
        )
        raise
    record = dict(round=round_index, arm=arm, order=order, phase=phase,
                  rc=0, checksum=str(checksum), seconds=elapsed)
    records.append(record)
    if not math.isfinite(elapsed) or elapsed <= 0:
        raise ValueError(f"invalid sample time: {record}")
    if expected is not None and checksum != expected:
        raise ValueError(f"CORRECTNESS: inconsistent checksum: {record}; expected {expected}")
    return elapsed, checksum

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--a", required=True); ap.add_argument("--b", required=True)
    ap.add_argument("--kernel", help="kernel name (tests/bench/k_<name>.c); "
                                     "required unless --program is given")
    ap.add_argument("--samples", type=int, default=41)
    ap.add_argument("--json", help="save complete raw warmup and timed sample records")
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
    if a.samples <= 0:
        ap.error("--samples must be positive")
    ta, tb = [], []
    ta_c = None
    records = []
    build_records = []
    take = (lambda b: sample_program(b, a.cpu)) if a.program else (lambda b: sample(b, a.cpu))
    try:
        with tempfile.TemporaryDirectory() as d:
            ba, bb = os.path.join(d, "a"), os.path.join(d, "b")
            for arm, compiler, binary, arm_env in (
                ("a", a.a, ba, a_env),
                ("b", a.b, bb, b_env),
            ):
                try:
                    record = build(
                        compiler,
                        a.kernel,
                        binary,
                        flags,
                        a.repo,
                        env=arm_env,
                        program=a.program,
                        defines=defines,
                    )
                except BuildFailure as exc:
                    record = exc.record
                    record["arm"] = arm
                    build_records.append(record)
                    raise
                record["arm"] = arm
                build_records.append(record)
            for phase, count in (('warmup',3),('timed',a.samples)):
                for i in range(count):
                    order = (('a',ba,ta),('b',bb,tb)) if i%2 == 0 else (('b',bb,tb),('a',ba,ta))
                    for position,(arm,binary,times) in enumerate(order):
                        elapsed,checksum = checked_sample(take,binary,ta_c,records,i,arm,position,phase)
                        if ta_c is None: ta_c = checksum
                        if phase == 'timed': times.append(elapsed)
    except (BuildFailure, ValueError, OSError, subprocess.SubprocessError) as exc:
        if a.json:
            Path(a.json).write_text(json.dumps({'verdict':'correctness-or-infrastructure-failure',
                                               'detail':str(exc),'builds':build_records,
                                               'samples':records},indent=2)+'\n')
        print(f'CORRECTNESS/INFRASTRUCTURE FAILURE: {exc}',file=sys.stderr)
        return 2
    if a.json:
        Path(a.json).write_text(json.dumps({'verdict':'sample-complete',
                                           'checksum':str(ta_c),'builds':build_records,
                                           'samples':records},indent=2)+'\n')

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
