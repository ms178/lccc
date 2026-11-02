#!/usr/bin/env python3
"""No-pessimisation sweep for `CCC_NO_BIT_SELECT` over the in-tree C corpus.

Every `.c` file under the given roots is compiled twice by the same binary --
once with the fold on (default) and once with `CCC_NO_BIT_SELECT=1` -- to
assembly, and the number of instruction lines in each output is compared.  The
knob is what makes the two builds differ in *one rule only*, so a file that
gets longer is a pessimisation of that rule and nothing else.

Files that either build rejects are counted separately (`both-reject` /
`verdict`), never silently skipped: a verdict change would mean the fold
changes acceptance, which it must not.

Usage:
  sweep.py --bin target/fastbuild/lccc --roots tests engineering [--jobs 2]
           [--level -O2] [--json out.json]
"""
from __future__ import annotations

import argparse
import concurrent.futures as cf
import json
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

# One instruction line: leading whitespace, then a mnemonic.  Directives
# (`.cfi_*`, `.p2align`) and comments (`#`) are not instructions; neither are
# labels (`foo:`).
INSN = re.compile(r"^\s+([a-z][a-z0-9.]*)\b")


def instruction_count(asm: str) -> int:
    n = 0
    for line in asm.splitlines():
        stripped = line.strip()
        if not stripped or stripped.startswith(("#", ".", "//")):
            continue
        if stripped.endswith(":"):
            continue
        if INSN.match(line):
            n += 1
    return n


def sibling(path: Path, suffix: str) -> Path | None:
    candidate = path.with_suffix(suffix)
    return candidate if candidate.is_file() else None


def build(binary: str, src: Path, level: str, off: bool) -> tuple[bool, int, str]:
    env = dict(os.environ)
    if off:
        env["CCC_NO_BIT_SELECT"] = "1"
    else:
        env.pop("CCC_NO_BIT_SELECT", None)
    # The corpus's own convention: `NAME.flags` appends words, `NAME.env` sets
    # KEY=VALUE lines (the regression harness reads the same files).
    flags: list[str] = []
    flags_file = sibling(src, ".flags")
    if flags_file is not None:
        flags = flags_file.read_text().split()
    env_file = sibling(src, ".env")
    if env_file is not None:
        for line in env_file.read_text().splitlines():
            line = line.strip()
            if line and not line.startswith("#") and "=" in line:
                key, value = line.split("=", 1)
                env[key.strip()] = value.strip()
    with tempfile.NamedTemporaryFile(suffix=".s", delete=True) as out:
        proc = subprocess.run(
            [binary, level, "-S", str(src), *flags, "-o", out.name],
            capture_output=True,
            text=True,
            env=env,
            timeout=120,
        )
        if proc.returncode != 0:
            return False, 0, proc.stderr.strip().splitlines()[-1] if proc.stderr.strip() else ""
        try:
            asm = Path(out.name).read_text(errors="replace")
        except OSError:
            return False, 0, "no output"
    return True, instruction_count(asm), ""


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", required=True)
    ap.add_argument("--roots", nargs="+", required=True)
    ap.add_argument("--level", default="-O2")
    ap.add_argument("--jobs", type=int, default=2)
    ap.add_argument("--json")
    args = ap.parse_args()

    files: list[Path] = []
    for root in args.roots:
        files.extend(sorted(Path(root).rglob("*.c")))
    if not files:
        print("no files", file=sys.stderr)
        return 2

    def one(path: Path):
        on_ok, on_n, on_err = build(args.bin, path, args.level, False)
        off_ok, off_n, off_err = build(args.bin, path, args.level, True)
        return str(path), (on_ok, on_n, on_err), (off_ok, off_n, off_err)

    buckets = {"shorter": [], "same": 0, "longer": [], "both-reject": 0, "verdict": []}
    total_delta = 0
    with cf.ThreadPoolExecutor(max_workers=args.jobs) as pool:
        for path, on, off in pool.map(one, files):
            if not on[0] and not off[0]:
                buckets["both-reject"] += 1
                continue
            if on[0] != off[0]:
                buckets["verdict"].append((path, on, off))
                continue
            delta = on[1] - off[1]
            total_delta += delta
            if delta < 0:
                buckets["shorter"].append((path, on[1], off[1]))
            elif delta > 0:
                buckets["longer"].append((path, on[1], off[1]))
            else:
                buckets["same"] += 1

    print(f"files: {len(files)}  level: {args.level}")
    print(f"shorter: {len(buckets['shorter'])}  same: {buckets['same']}  "
          f"longer: {len(buckets['longer'])}  both-reject: {buckets['both-reject']}  "
          f"verdict: {len(buckets['verdict'])}")
    print(f"net instruction delta: {total_delta:+d}")
    for path, on_n, off_n in buckets["shorter"][:20]:
        print(f"  shorter {off_n} -> {on_n}  {path}")
    for path, on_n, off_n in buckets["longer"]:
        print(f"  LONGER  {off_n} -> {on_n}  {path}")
    for path, on, off in buckets["verdict"]:
        print(f"  VERDICT {path}: on={on} off={off}")
    if args.json:
        Path(args.json).write_text(json.dumps({
            "level": args.level,
            "files": len(files),
            "shorter": buckets["shorter"],
            "same": buckets["same"],
            "longer": buckets["longer"],
            "both_reject": buckets["both-reject"],
            "verdict": buckets["verdict"],
            "net_delta": total_delta,
        }, indent=1))
    return 0 if not buckets["longer"] and not buckets["verdict"] else 1


if __name__ == "__main__":
    sys.exit(main())
