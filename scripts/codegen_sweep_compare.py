#!/usr/bin/env python3
"""Compare assembly from two lccc binaries over a corpus of C files.

For every C file under the corpus roots, and each optimisation level, both
binaries compile the file to assembly (`-S`). The outputs are compared after
normalising the run-specific lines (PGO pid names, temp paths). Each file ends
up in one bucket:

  identical      same assembly from both binaries
  different      both compiled, the assembly differs
  pgo-pid        differs only in a normalised PGO/temp token
  compile-fail   exactly one binary rejected the file: a verdict difference
  both-reject    both binaries rejected the file (same verdict, not compared)
  no-output      a binary exited 0 but wrote no assembly (not compared)

Usage:
  codegen_sweep_compare.py --base BIN_A --new BIN_B --out DIR [--roots D ...]
                           [--levels -O0,-O1,-O2] [--jobs N]

(--levels is comma-separated: an option-looking value such as -O0 would be
parsed by argparse as a flag when given as a separate word.)

Files with a sibling `NAME.flags` file have its whitespace-separated words
appended to the command line, and a sibling `NAME.env` file's KEY=VALUE lines
are set in the environment of both binaries, as the regression harness does.
Comparing without them would compare a different configuration (for example
CCC_LOOP_ROTATE or CCC_DISABLE_PASSES change the generated code).
"""
from __future__ import annotations

import argparse
import concurrent.futures as cf
import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

# Only tokens that are run-specific by construction. A generic `pid<digits>`
# pattern is not here: it would also hide real differences in symbol names.
PID_RE = re.compile(r"(\.lto_priv\d*|\.pgo\b|-?\d+\.profraw|/tmp/[^\s\"']+)")
BYTE_LINE_RE = re.compile(r"^(\s*)\.byte\s+((?:\d+,\s*)*\d+)\s*$")
# The PGO dump routine is named after its build hash and the compiling process
# id: `__lccc_pgo_dump_<hash>_<pid>`. Only the trailing pid is run-specific.
PGO_DUMP_RE = re.compile(r"(__lccc_pgo_dump_[0-9a-f]+)_\d+\b")


def _decode_byte_line(line: str) -> str:
    """Render a `.byte` string literal as text so PID_RE can see inside it.

    PGO profile names are emitted as byte lists; the process id inside them is
    run-specific, so the text must be normalised, not the bytes.
    """
    m = BYTE_LINE_RE.match(line)
    if not m:
        return line
    vals = [int(x) for x in m.group(2).split(",")]
    if vals[-1] != 0 or not all(32 <= v < 127 for v in vals[:-1]):
        return line
    text = bytes(vals[:-1]).decode("ascii")
    return f"{m.group(1)}.ascii {PID_RE.sub('<tok>', text)!r}"


def normalise(text: str) -> str:
    lines = [_decode_byte_line(line) for line in text.split("\n")]
    text = PGO_DUMP_RE.sub(r"\1_<tok>", "\n".join(lines))
    return PID_RE.sub("<tok>", text)


def env_for(src: Path) -> dict[str, str]:
    """KEY=VALUE lines of a sibling NAME.env file, as run_regression.sh sources them."""
    env: dict[str, str] = {}
    f = src.with_suffix(".env")
    if f.exists():
        for line in f.read_text().splitlines():
            m = re.match(r"^([A-Za-z_][A-Za-z_0-9]*)=(.*)$", line.strip())
            if m:
                env[m.group(1)] = m.group(2)
    return env


def compile_one(binary: str, src: Path, level: str, flags: list[str], workdir: Path,
                env: dict[str, str] | None = None):
    out = workdir / f"{hashlib.sha1(f'{binary}{src}{level}'.encode()).hexdigest()}.s"
    cmd = [binary, level, *flags, "-S", "-o", str(out), str(src.resolve())]
    try:
        run_env = {**os.environ, **(env or {})}
        p = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                           timeout=120, cwd=str(src.parent), env=run_env)
    except subprocess.TimeoutExpired:
        return {"rc": -1, "asm": None, "diag": "timeout"}
    asm = None
    if p.returncode == 0 and out.exists():
        asm = out.read_text(errors="replace")
    diag = (p.stderr or b"").decode(errors="replace")[-300:]
    if out.exists():
        out.unlink()
    return {"rc": p.returncode, "asm": asm, "diag": diag}


def flags_for(src: Path) -> list[str]:
    f = src.with_suffix(".flags")
    if f.exists():
        return f.read_text().split()
    return []


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--base", required=True)
    ap.add_argument("--new", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--roots", nargs="+", default=["tests/regression"])
    ap.add_argument("--levels", default="-O0,-O1,-O2",
                    help="comma-separated optimisation levels")
    ap.add_argument("--jobs", type=int, default=2)
    ap.add_argument("--limit", type=int, default=0)
    args = ap.parse_args()
    args.levels = [x for x in args.levels.split(",") if x]
    if not args.levels or not all(x.startswith("-") for x in args.levels):
        ap.error("--levels must be a comma-separated list of options such as -O0,-O2")

    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    files: list[Path] = []
    for root in args.roots:
        files += sorted(Path(root).rglob("*.c"))
    if args.limit:
        files = files[: args.limit]

    results: dict[str, dict] = {}
    with tempfile.TemporaryDirectory(prefix="sweep-") as td:
        work = Path(td)
        jobs = []
        with cf.ThreadPoolExecutor(max_workers=args.jobs) as ex:
            for src in files:
                for level in args.levels:
                    flags = flags_for(src)
                    env = env_for(src)
                    jobs.append((src, level, ex.submit(compile_one, args.base, src, level, flags, work, env),
                                 ex.submit(compile_one, args.new, src, level, flags, work, env)))
            for src, level, fb, fn in jobs:
                b = fb.result()
                n = fn.result()
                key = f"{src}|{level}"
                if b["rc"] != 0 and n["rc"] != 0:
                    bucket = "both-reject"
                elif b["rc"] != 0 or n["rc"] != 0:
                    bucket = "compile-fail"
                elif b["asm"] is None or n["asm"] is None:
                    bucket = "no-output"
                elif b["asm"] == n["asm"]:
                    bucket = "identical"
                elif normalise(b["asm"]) == normalise(n["asm"]):
                    bucket = "pgo-pid"
                else:
                    bucket = "different"
                results[key] = {
                    "bucket": bucket,
                    "base_rc": b["rc"],
                    "new_rc": n["rc"],
                    "base_diag": b["diag"] if b["rc"] else "",
                    "new_diag": n["diag"] if n["rc"] else "",
                }

    counts: dict[str, int] = {}
    for v in results.values():
        counts[v["bucket"]] = counts.get(v["bucket"], 0) + 1
    per_file: dict[str, dict[str, str]] = {}
    for key, v in results.items():
        f, level = key.split("|")
        per_file.setdefault(f, {})[level] = v["bucket"]
    summary = {
        "base": args.base,
        "new": args.new,
        "roots": args.roots,
        "levels": args.levels,
        "files": len(files),
        "counts": counts,
        "different_files": sorted(f for f, lv in per_file.items()
                                  if any(b == "different" for b in lv.values())),
        "compile_fail_files": sorted(f for f, lv in per_file.items()
                                     if any(b == "compile-fail" for b in lv.values())),
        "no_output_files": sorted(f for f, lv in per_file.items()
                                  if any(b == "no-output" for b in lv.values())),
    }
    (out / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    (out / "results.json").write_text(json.dumps(results, indent=2) + "\n")
    print(json.dumps({k: summary[k] for k in ("files", "counts")}, indent=2))
    for f in summary["different_files"]:
        print("DIFFERENT", f)
    for f in summary["compile_fail_files"]:
        print("COMPILE-FAIL", f)
    for f in summary["no_output_files"]:
        print("NO-OUTPUT", f)
    return 0


if __name__ == "__main__":
    sys.exit(main())
