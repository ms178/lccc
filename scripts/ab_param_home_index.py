#!/usr/bin/env python3
"""Byte-exact A/B object/assembly parity for the ParamAllocaIndex refactor.

For every gcc.c-torture/compile source, target and optimisation level, run a
pristine-HEAD compiler and the refactored compiler with IDENTICAL arguments
and compare the emitted assembly byte-for-byte.  A source that both compilers
reject is recorded as ``both-fail`` only when the diagnostics also match; any
other difference is a hard divergence and fails the campaign.

Usage:
  python3 scripts/ab_param_home_index.py --old <dir> --new <dir> \
      --suite <dir> --out <json> [--limit N] [--workers 2] \
      [--opts -O0,-O2] [--arches x86_64,i686,aarch64,riscv64]
"""
from __future__ import annotations

import argparse
import concurrent.futures
import hashlib
import json
import pathlib
import subprocess
import sys
import time
import re

BINARIES = {
    "x86_64": "lccc",
    "i686": "lccc-i686",
    "aarch64": "lccc-arm",
    "riscv64": "lccc-riscv",
}

# Unconditional DejaGnu dg-options, quoted or brace-list form.  Both A/B legs
# receive the exact same list, so this cannot bias the comparison.
_DG = re.compile(
    r"\{\s*dg-options\s+\"(?:[^\"\\]|\\.)*\"|\{\s*dg-options\s+\{([^}]*)\}",
    re.S,
)
_QUOTED = re.compile(r"\{\s*dg-options\s+\"((?:[^\"\\]|\\.)*)\"")


def directives(text: str) -> list[str]:
    out: list[str] = []
    head = text[:8192]
    for m in _QUOTED.finditer(head):
        out.extend(m.group(1).split())
    for m in _DG.finditer(head):
        if m.group(1):
            out.extend(m.group(1).split())
    # Only unconditional flags; dg-add-options / dg-additional-options need a
    # DejaGnu target board and are deliberately not honoured here.
    return [f for f in out if f.startswith("-") and not f.startswith("-Werror")]


def run(cmd: list[str], timeout: float = 20.0):
    try:
        p = subprocess.run(cmd, capture_output=True, timeout=timeout)
        return p.returncode, p.stdout, p.stderr
    except subprocess.TimeoutExpired:
        return 124, b"", b"HARNESS TIMEOUT"
    except OSError as e:
        return 125, b"", str(e).encode()


def one(job: tuple) -> dict:
    old_dir, new_dir, suite, arch, opt, source, directives_flags, idx, timeout = job
    binary = BINARIES[arch]
    tmp = pathlib.Path("/tmp/lccc-param-home-ab")
    tmp.mkdir(exist_ok=True)
    stem = f"{idx:06d}_{arch}_{opt.replace('-', '')}"
    old_out = tmp / f"{stem}_old.s"
    new_out = tmp / f"{stem}_new.s"
    common = [opt, "-S"]
    common += ["-I", suite]
    common += directives_flags
    args_tail = [str(source)]
    rco, _, eo = run([str(pathlib.Path(old_dir) / binary)] + common + ["-o", str(old_out)] + args_tail, timeout)
    rcn, _, en = run([str(pathlib.Path(new_dir) / binary)] + common + ["-o", str(new_out)] + args_tail, timeout)

    def digest(path: pathlib.Path) -> str | None:
        if not path.exists():
            return None
        return hashlib.sha256(path.read_bytes()).hexdigest()

    do, dn = digest(old_out), digest(new_out)
    old_out.unlink(missing_ok=True)
    new_out.unlink(missing_ok=True)
    if rco == 0 and rcn == 0:
        status = "identical" if do == dn else "divergent"
    elif rco != 0 and rcn != 0:
        status = "both-fail" if (rco == rcn and eo == en) else "diagnostic-divergent"
    else:
        status = "rc-divergent"
    return {
        "source": source.name,
        "arch": arch,
        "opt": opt,
        "old_rc": rco,
        "new_rc": rcn,
        "status": status,
        "old_sha256": do,
        "new_sha256": dn,
        "old_stderr": eo.decode("utf-8", "replace")[-400:] if status != "identical" else "",
        "new_stderr": en.decode("utf-8", "replace")[-400:] if status != "identical" else "",
    }


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--old", required=True)
    ap.add_argument("--new", required=True)
    ap.add_argument("--suite", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--limit", type=int, default=0)
    ap.add_argument("--workers", type=int, default=2)
    ap.add_argument("--opts", default="-O0,-O2")
    ap.add_argument("--arches", default="x86_64,i686,aarch64,riscv64")
    ap.add_argument("--timeout", type=float, default=20.0)
    ap.add_argument("--exclude", default="limits-*",
                    help="glob of source basenames to skip (default: the 15 "
                         "compiler stress cases, run separately with a long timeout)")
    ap.add_argument("--include", default="",
                    help="if set, ONLY sources matching this glob are run")
    args = ap.parse_args()

    suite = pathlib.Path(args.suite)
    import fnmatch
    sources = sorted((suite / "compile").glob("*.c"))
    if args.include:
        sources = [s for s in sources if fnmatch.fnmatch(s.name, args.include)]
    elif args.exclude:
        sources = [s for s in sources if not fnmatch.fnmatch(s.name, args.exclude)]
    if args.limit:
        sources = sources[: args.limit]
    opts = args.opts.split(",")
    arches = args.arches.split(",")
    jobs = []
    for src in sources:
        try:
            flags = directives(src.read_text(errors="replace"))
        except OSError:
            flags = []
        for arch in arches:
            for opt in opts:
                jobs.append((args.old, args.new, str(suite), arch, opt, src, flags,
                             len(jobs), args.timeout))
    print(f"A/B: {len(sources)} sources x {len(arches)} arches x {len(opts)} opts = {len(jobs)} pairs", flush=True)

    counts: dict[str, int] = {}
    divergences: list[dict] = []
    started = time.monotonic()
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.workers) as pool:
        for i, res in enumerate(pool.map(one, jobs), 1):
            counts[res["status"]] = counts.get(res["status"], 0) + 1
            if "divergent" in res["status"]:
                divergences.append(res)
                print(f"  DIVERGENCE {res['source']} {res['arch']} {res['opt']} "
                      f"status={res['status']} old_rc={res['old_rc']} new_rc={res['new_rc']} "
                      f"old_err={res['old_stderr'][-120:]!r} new_err={res['new_stderr'][-120:]!r}",
                      flush=True)
            if i % 2000 == 0:
                print(f"  {i}/{len(jobs)} {counts} {time.monotonic()-started:.1f}s", flush=True)
                pathlib.Path(args.out + ".partial").write_text(json.dumps({
                    "complete": False, "done": i, "pairs": len(jobs),
                    "counts": counts, "divergences": divergences}, indent=2) + "\n")

    payload = {
        "complete": True,
        "pairs": len(jobs),
        "sources": len(sources),
        "arches": arches,
        "opts": opts,
        "counts": counts,
        "divergences": divergences,
        "seconds": time.monotonic() - started,
        "timeout": args.timeout,
        "excluded": args.exclude,
        "included": args.include,
    }
    pathlib.Path(args.out).write_text(json.dumps(payload, indent=2) + "\n")
    print(json.dumps({k: v for k, v in payload.items() if k != "divergences"}, indent=2), flush=True)
    return 0 if not divergences else 1


if __name__ == "__main__":
    raise SystemExit(main())
