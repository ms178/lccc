#!/usr/bin/env python3
"""LCCC stress laboratory — oracle-free, self-checking, self-reducing.

Generates deterministic C programs from the families in ``families.py``, the
peephole-targeted families in ``scripts/peephole_families.py`` (imported below
so they register themselves; select them with ``--families flags,narrow,...``
or run everything by default) and the two-TU ABI matrix in ``abi_family.py``,
compiles them with LCCC at every
requested optimisation level in two argument modes, runs them, and classifies
every outcome:

  PASS            all cases returned the value the C standard prescribes
  MISMATCH        at least one case returned a different value (miscompile);
                  the failing cases are named and a one-case reproducer is
                  written next to the full program
  ICE             the compiler exited non-zero (or died by signal) on a
                  program GCC accepts
  CRASH           the generated binary died by signal
  TIMEOUT         compile or run exceeded the budget
  ORACLE-DISAGREE gcc -O0 disagrees with the emulator — a generator (or GCC)
                  bug; reported separately and never attributed to LCCC

Why this exists next to the differential fuzzers in ``tests/fuzz``:

* The expected values are computed from the language definition, so a shared
  GCC/LCCC misunderstanding cannot hide, and every failure says which value is
  right.
* Cases are independent functions, so reduction is free: the failing case is
  re-emitted alone as ``<name>.repro.c`` — no creduce, no bisecting.
* Each family aims at one compiler subsystem with boundary-biased inputs, so a
  few hundred programs reach corners (near-wrap IVs, straddling bit-fields,
  ``-0.0`` selects, ``uint64 > 2^63`` conversions, INT_MIN divisors) that
  uniform random generation reaches only by luck.

Usage
-----
  python3 tests/stress/run_stress.py --jobs 2 --seeds 0:20
  python3 tests/stress/run_stress.py --families loops,fpcmp --levels O2,O3 \\
      --seeds 0:200 --out /tmp/stress --json /tmp/stress/report.json
  python3 tests/stress/run_stress.py --families abi --seeds 0:50

Exit status: 0 when LCCC produced no MISMATCH/ICE/CRASH/TIMEOUT, 1 otherwise.
Oracle disagreements do not affect the exit status but are listed.
"""
from __future__ import annotations

import argparse
import concurrent.futures as futures
import json
import os
import random
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import zlib
from dataclasses import dataclass, asdict, field
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
# The peephole family generators live beside the other lab scripts.
SCRIPTS = REPO / "scripts"
sys.path.insert(0, str(HERE))
if SCRIPTS.is_dir():
    sys.path.insert(0, str(SCRIPTS))

import families  # noqa: E402
from families import Case, Param  # noqa: E402
import abi_family  # noqa: E402
import peephole_families  # noqa: E402,F401  (registers the peephole family set)

DEFAULT_LCCC = REPO / "target" / "fastbuild" / "lccc"


# ---------------------------------------------------------------------------
# program assembly
# ---------------------------------------------------------------------------

def emit_program(cases: list[Case], mode: str) -> str:
    """Assemble a self-checking translation unit from independent cases."""
    lines = ["#include <stdint.h>", "#include <stdio.h>", "#include <string.h>", ""]
    seen: set[str] = set()
    for c in cases:
        for d in c.decls:
            if d not in seen:
                seen.add(d)
                lines.append(d)
    lines.append("")
    attr = "__attribute__((noinline))" if mode == "rt" else "static inline"
    for c in cases:
        rt = c.ret.name if not isinstance(c.ret, str) else c.ret
        plist = ", ".join(f"{p.cty} p{i}" for i, p in enumerate(c.params)) or "void"
        lines.append(f"{attr} {rt} {c.name}({plist}) {{")
        lines.append(f"    {c.body}")
        lines.append("}")
        if mode == "rt":
            # Route every argument through volatile storage so the callee sees
            # a runtime value and no argument can be constant-folded.
            for i, p in enumerate(c.params):
                if p.kind == "int":
                    lines.append(f"static volatile {p.cty} {c.name}_a{i} = {p.value};")
                else:
                    lines.append(f"static volatile {p.cty} {c.name}_a{i} = {p.value};")
        lines.append("")
    lines.append("int main(void) {")
    lines.append("    int fails = 0;")
    for c in cases:
        if mode == "rt":
            args = ", ".join(f"{c.name}_a{i}" for i in range(len(c.params)))
        else:
            args = ", ".join(p.value for p in c.params)
        rt = c.ret
        if rt.bits == 128:
            u = c.expected & ((1 << 128) - 1)
            hi, lo = u >> 64, u & ((1 << 64) - 1)
            lines.append(f"    {{ unsigned __int128 r = (unsigned __int128){c.name}({args});")
            lines.append(f"      if ((uint64_t)(r >> 64) != 0x{hi:x}ull || (uint64_t)r != 0x{lo:x}ull) {{ fails++;")
            lines.append(f"        printf(\"FAIL {c.name} [{c.family}: {c.desc}] got hi=%llx lo=%llx expected hi={hi:x} lo={lo:x}\\n\","
                         f" (unsigned long long)(r >> 64), (unsigned long long)r); }} }}")
        else:
            fmt, cast = families.ret_check(rt)
            exp = rt.literal(c.expected)
            lines.append(f"    {{ {rt.name} r = {c.name}({args});")
            lines.append(f"      if (r != {exp}) {{ fails++;")
            lines.append(f"        printf(\"FAIL {c.name} [{c.family}: {c.desc}] got {fmt} expected {fmt}\\n\", {cast}r, {cast}{exp}); }} }}")
    lines.append("    if (fails == 0) puts(\"ALL OK\");")
    lines.append("    return fails > 100 ? 100 : fails;")
    lines.append("}")
    return "\n".join(lines) + "\n"


# ---------------------------------------------------------------------------
# execution
# ---------------------------------------------------------------------------

@dataclass
class Outcome:
    family: str
    seed: int
    mode: str
    level: str
    verdict: str
    detail: str = ""
    failing_cases: list[str] = field(default_factory=list)
    compile_s: float = 0.0
    run_s: float = 0.0
    artifact: str = ""


def run_cmd(cmd: list[str], timeout: float, cwd: Path | None = None) -> tuple[int, str, str, float, bool]:
    t0 = time.monotonic()
    try:
        p = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, timeout=timeout,
                           errors="replace")
        return p.returncode, p.stdout, p.stderr, time.monotonic() - t0, False
    except subprocess.TimeoutExpired as e:
        return -1, (e.stdout or "") if isinstance(e.stdout, str) else "", \
            (e.stderr or "") if isinstance(e.stderr, str) else "", time.monotonic() - t0, True


def signal_name(rc: int) -> str:
    if rc < 0:
        try:
            return signal.Signals(-rc).name
        except ValueError:
            return f"SIG{-rc}"
    return f"exit {rc}"


def compile_and_run(cc: list[str], src: Path, exe: Path, level: str, extra: list[str],
                    timeout: float) -> tuple[str, str, list[str], float, float]:
    """Return (verdict, detail, failing_cases, compile_s, run_s)."""
    cmd = cc + [f"-{level}", *extra, str(src), "-o", str(exe), "-lm"]
    rc, out, err, cs, to = run_cmd(cmd, timeout)
    if to:
        return "TIMEOUT", "compile timeout", [], cs, 0.0
    if rc != 0:
        return "ICE", f"compiler {signal_name(rc)}: {err.strip().splitlines()[-1] if err.strip() else out.strip()[-200:]}", [], cs, 0.0
    rc, out, err, rs, to = run_cmd([str(exe)], timeout)
    if to:
        return "TIMEOUT", "run timeout", [], cs, rs
    if rc < 0:
        return "CRASH", f"binary died with {signal_name(rc)}", [], cs, rs
    fails = [ln for ln in out.splitlines() if ln.startswith("FAIL ")]
    if rc != 0 or fails or "ALL OK" not in out:
        names = [ln.split()[1] for ln in fails]
        return "MISMATCH", "\n".join(fails[:8]) or f"exit {rc} without ALL OK", names, cs, rs
    return "PASS", "", [], cs, rs


def job(args: argparse.Namespace, family: str, seed: int, workdir: Path) -> list[Outcome]:
    outcomes: list[Outcome] = []
    rng = random.Random(seed * 1000003 + zlib.crc32(family.encode()))  # stable across processes
    if family == "abi":
        return abi_family.run(args, seed, workdir)
    cases = families.FAMILIES[family](rng, args.cases)
    if not cases:
        return [Outcome(family, seed, "-", "-", "EMPTY", "generator produced no cases")]
    for mode in args.modes:
        text = emit_program(cases, mode)
        base = workdir / f"{family}_s{seed}_{mode}"
        src = base.with_suffix(".c")
        src.write_text(text)
        # 1. oracle sanity: gcc -O0 must agree with the emulator.
        if not args.no_oracle:
            v, d, names, cs, rs = compile_and_run([args.gcc], src, base.with_suffix(".gcc"), "O0", ["-w"], args.timeout)
            if v != "PASS":
                keep = args.out / "oracle-disagree" / src.name
                keep.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy(src, keep)
                outcomes.append(Outcome(family, seed, mode, "gcc-O0", "ORACLE-DISAGREE", d, names, cs, rs, str(keep)))
                # Cases the oracle rejects are removed from LCCC's judgement so
                # a generator slip cannot masquerade as a miscompile.
                bad = set(names)
                cases_l = [c for c in cases if c.name not in bad] if v == "MISMATCH" else []
                if not cases_l:
                    continue
                text = emit_program(cases_l, mode)
                src.write_text(text)
        # 2. LCCC at every level.
        for level in args.levels:
            exe = base.with_suffix(f".{level}")
            extra = list(args.extra_flags)
            v, d, names, cs, rs = compile_and_run([args.lccc], src, exe, level, extra, args.timeout)
            o = Outcome(family, seed, mode, level, v, d, names, cs, rs)
            if v != "PASS":
                keep_dir = args.out / v.lower() / family
                keep_dir.mkdir(parents=True, exist_ok=True)
                keep = keep_dir / f"{src.stem}_{level}.c"
                shutil.copy(src, keep)
                o.artifact = str(keep)
                (keep_dir / f"{src.stem}_{level}.txt").write_text(d + "\n")
                # Free reduction: re-emit each failing case alone.
                by_name = {c.name: c for c in cases}
                for n in names[:16]:
                    if n in by_name:
                        (keep_dir / f"{src.stem}_{level}.{n}.repro.c").write_text(emit_program([by_name[n]], mode))
                # Repro script.
                (keep_dir / f"{src.stem}_{level}.sh").write_text(
                    "#!/bin/sh\n"
                    f"# {family} seed={seed} mode={mode} level={level}: {v}\n"
                    f"{args.lccc} -{level} {' '.join(extra)} {keep} -o /tmp/{src.stem}_{level} -lm && /tmp/{src.stem}_{level}\n")
            outcomes.append(o)
            if not args.keep_binaries:
                exe.unlink(missing_ok=True)
    return outcomes


# ---------------------------------------------------------------------------
# reporting
# ---------------------------------------------------------------------------

def summarize(outcomes: list[Outcome]) -> dict:
    table: dict[str, dict[str, dict[str, int]]] = {}
    for o in outcomes:
        table.setdefault(o.family, {}).setdefault(o.level, {}).setdefault(o.verdict, 0)
        table[o.family][o.level][o.verdict] += 1
    return table


def render_markdown(outcomes: list[Outcome], args: argparse.Namespace, elapsed: float) -> str:
    table = summarize(outcomes)
    verdicts = ["PASS", "MISMATCH", "ICE", "CRASH", "TIMEOUT", "ORACLE-DISAGREE", "EMPTY"]
    lines = ["# LCCC stress laboratory report", "",
             f"- compiler: `{args.lccc}`", f"- oracle: `{args.gcc}` (-O0)",
             f"- families: {', '.join(args.families)}", f"- seeds: {args.seed_lo}..{args.seed_hi - 1}",
             f"- cases/program: {args.cases}; modes: {', '.join(args.modes)}; levels: {', '.join(args.levels)}",
             f"- extra flags: `{' '.join(args.extra_flags) or '-'}`",
             f"- programs: {len({(o.family, o.seed, o.mode) for o in outcomes})}; outcomes: {len(outcomes)}; wall: {elapsed:.1f}s", "",
             "| family | level | " + " | ".join(verdicts) + " |", "|---|---|" + "---|" * len(verdicts)]
    for fam in sorted(table):
        for lvl in sorted(table[fam]):
            row = table[fam][lvl]
            lines.append(f"| {fam} | {lvl} | " + " | ".join(str(row.get(v, 0)) for v in verdicts) + " |")
    bad = [o for o in outcomes if o.verdict not in ("PASS", "EMPTY")]
    if bad:
        lines += ["", "## Findings", ""]
        for o in bad:
            lines.append(f"### {o.verdict} — {o.family} seed {o.seed} mode {o.mode} level {o.level}")
            if o.artifact:
                lines.append(f"artifact: `{o.artifact}`")
            lines.append("```")
            lines.append(o.detail.strip()[:2000])
            lines.append("```")
    return "\n".join(lines) + "\n"


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--lccc", default=str(DEFAULT_LCCC))
    ap.add_argument("--gcc", default="gcc")
    ap.add_argument("--families", default=",".join(list(families.FAMILIES) + ["abi"]))
    ap.add_argument("--seeds", default="0:8", help="half-open seed range a:b")
    ap.add_argument("--cases", type=int, default=40, help="cases per generated program")
    ap.add_argument("--levels", default="O0,O1,O2,O3,Os")
    ap.add_argument("--modes", default="rt,cf", help="rt = volatile runtime args, cf = constant args")
    ap.add_argument("--extra-flags", default="", help="extra flags for LCCC (e.g. '-march=x86-64-v3')")
    ap.add_argument("--jobs", type=int, default=2)
    ap.add_argument("--timeout", type=float, default=60.0)
    ap.add_argument("--out", default="/tmp/lccc-stress")
    ap.add_argument("--json", default="")
    ap.add_argument("--markdown", default="")
    ap.add_argument("--no-oracle", action="store_true", help="skip the gcc -O0 generator sanity check")
    ap.add_argument("--keep-binaries", action="store_true")
    ap.add_argument("--keep-work", action="store_true", help="keep all generated sources, not just failures")
    args = ap.parse_args()

    args.families = [f.strip() for f in args.families.split(",") if f.strip()]
    for f in args.families:
        if f not in families.FAMILIES and f != "abi":
            ap.error(f"unknown family {f!r}; known: {', '.join(list(families.FAMILIES) + ['abi'])}")
    lo, hi = args.seeds.split(":")
    args.seed_lo, args.seed_hi = int(lo), int(hi)
    args.levels = [l.strip() for l in args.levels.split(",") if l.strip()]
    args.modes = [m.strip() for m in args.modes.split(",") if m.strip()]
    args.extra_flags = args.extra_flags.split()
    args.out = Path(args.out)
    args.out.mkdir(parents=True, exist_ok=True)
    if not Path(args.lccc).exists():
        print(f"error: compiler not found: {args.lccc}", file=sys.stderr)
        return 2

    work = Path(tempfile.mkdtemp(prefix="lccc-stress-", dir=str(args.out) if args.keep_work else None))
    t0 = time.monotonic()
    outcomes: list[Outcome] = []
    tasks = [(fam, seed) for seed in range(args.seed_lo, args.seed_hi) for fam in args.families]
    with futures.ThreadPoolExecutor(max_workers=args.jobs) as ex:
        futs = {ex.submit(job, args, fam, seed, work): (fam, seed) for fam, seed in tasks}
        done = 0
        for fut in futures.as_completed(futs):
            fam, seed = futs[fut]
            done += 1
            try:
                res = fut.result()
            except Exception as e:  # generator bug: report, never hide
                res = [Outcome(fam, seed, "-", "-", "ORACLE-DISAGREE", f"generator exception: {e!r}")]
            outcomes.extend(res)
            bad = [o for o in res if o.verdict not in ("PASS", "EMPTY")]
            status = "ok" if not bad else ",".join(sorted({o.verdict for o in bad}))
            print(f"[{done}/{len(tasks)}] {fam:9s} seed {seed:<5d} {status}", flush=True)
    elapsed = time.monotonic() - t0
    if not args.keep_work:
        shutil.rmtree(work, ignore_errors=True)

    md = render_markdown(outcomes, args, elapsed)
    print(md)
    if args.markdown:
        Path(args.markdown).write_text(md)
    if args.json:
        Path(args.json).write_text(json.dumps({
            "compiler": args.lccc, "oracle": args.gcc, "families": args.families,
            "seeds": [args.seed_lo, args.seed_hi], "cases": args.cases, "levels": args.levels,
            "modes": args.modes, "extra_flags": args.extra_flags, "elapsed_s": elapsed,
            "summary": summarize(outcomes), "outcomes": [asdict(o) for o in outcomes]}, indent=2))
    lccc_bad = [o for o in outcomes if o.verdict in ("MISMATCH", "ICE", "CRASH", "TIMEOUT")]
    return 1 if lccc_bad else 0


if __name__ == "__main__":
    sys.exit(main())
