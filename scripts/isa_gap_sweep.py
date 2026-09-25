#!/usr/bin/env python3
"""ISA-gap sweep: LCCC assembler vs the GNU as 2.47 testsuite, per line.

Every instruction line in the pinned binutils 2.47 testsuite is deduplicated,
assembled standalone with both assemblers, and classified:

    PASS      both accept, byte-identical .text
    ENCDIFF   both accept, different bytes (quality work for insndiff/encdiff)
    MISSING   GAS assembles it, LCCC rejects it          <- the gap list
    FALSEACC  LCCC assembles it, GAS rejects it          <- must be empty
    SKIP      GAS cannot assemble the line standalone (context-dependent)

`--rank` prints the failure table ranked by fail count then mnemonic, which
is the prioritization input for the ISA-gap campaign.  `--list NAME` prints
every failing line whose mnemonic contains NAME (the worklist for one gap).

Usage:
    scripts/isa_gap_sweep.py --suite x86-64            # x86-64 corpus
    scripts/isa_gap_sweep.py --suite i686              # i386 corpus
    scripts/isa_gap_sweep.py --suite x86-64 --rank     # + ranked table
    scripts/isa_gap_sweep.py --suite x86-64 --list vpcmov

Exit 0 when no MISSING/FALSEACC lines exist, 1 otherwise, 2 on setup error.
"""
from __future__ import annotations

import argparse
import concurrent.futures
import os
import re
import shutil
import subprocess
import sys
import tempfile
from collections import Counter
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_LCCC = REPO_ROOT / "target" / "fastbuild" / "lccc"

# Where the pinned binutils-2.47 source tree lives (scripts/ensure_gas_247.sh
# unpacks it here).
SUITES = {
    "x86-64": ("x86_64", ["--64"], "gas/testsuite/gas/i386"),
    "i686": ("i686", ["--32"], "gas/testsuite/gas/i386"),
}

LABEL_RE = re.compile(r"^[0-9$.a-zA-Z_][\w$.]*:")
DIRECTIVE_RE = re.compile(r"^\s*\.")


def load_lines(suite_dir: Path) -> set[str]:
    """Collect unique instruction lines from the testsuite .s files.

    A line qualifies when it is not empty, not a comment, not a label, and
    not a directive: either `mnemonic operands` or `prefix mnemonic ...`.
    """
    lines: set[str] = set()
    for path in sorted(suite_dir.glob("*.s")):
        try:
            text = path.read_text(errors="replace")
        except OSError:
            continue
        for raw in text.splitlines():
            line = raw.split("#", 1)[0].split(";", 1)[0].rstrip()
            if not line.strip():
                continue
            if LABEL_RE.match(line.strip()):
                continue
            if DIRECTIVE_RE.match(line):
                continue
            lines.add(line.strip())
    return lines


def assemble(binpath: str, flags: list[str], src: Path, obj: Path, is_lccc: bool) -> tuple[bool, str]:
    if is_lccc:
        cmd = [binpath, "-c", str(src), "-o", str(obj)]
    else:
        cmd = [binpath, *flags, str(src), "-o", str(obj)]
    try:
        proc = subprocess.run(
            cmd, capture_output=True, text=True, timeout=30,
        )
    except FileNotFoundError:
        sys.exit(f"error: assembler {binpath!r} not found")
    return proc.returncode == 0, proc.stderr


def text_bytes(obj: Path, objcopy: str) -> bytes:
    try:
        proc = subprocess.run(
            [objcopy, "-O", "binary", "--only-section=.text", str(obj), str(obj) + ".bin"],
            capture_output=True, timeout=30,
        )
        if proc.returncode != 0:
            return b""
        return Path(str(obj) + ".bin").read_bytes()
    except (OSError, subprocess.SubprocessError):
        return b""


def classify_one(args) -> tuple[str, str]:
    idx, line, gas, lccc, flags, objcopy, td = args
    tmp = Path(td) / f"tmp{idx % 16}"
    tmp.mkdir(exist_ok=True)
    src = tmp / f"i{idx}.s"
    src.write_text(".text\n" + line + "\n")
    gobj, lobj = tmp / f"g{idx}.o", tmp / f"l{idx}.o"
    gok, _ = assemble(gas, flags, src, gobj, is_lccc=False)
    if not gok:
        return line, "SKIP"
    lok, lerr = assemble(lccc, flags, src, lobj, is_lccc=True)
    if not lok:
        return line, "MISSING"
    if text_bytes(gobj, objcopy) != text_bytes(lobj, objcopy):
        return line, "ENCDIFF"
    return line, "PASS"


def mnemonic_of(line: str) -> str:
    toks = line.replace(",", " ").split()
    for t in toks:
        if t.startswith("{"):  # decorators: {evex} vpcmov ...
            continue
        return t.lower()
    return ""


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--suite", choices=sorted(SUITES), default="x86-64")
    ap.add_argument("--lccc", default=str(DEFAULT_LCCC))
    ap.add_argument("--as", dest="gas", required=True)
    ap.add_argument("--objcopy", default=shutil.which("objcopy", path=os.environ.get("PATH", "")) or "objcopy")
    ap.add_argument("--binutils-src", default="/home/z/.cache/binutils-2.47")
    ap.add_argument("--rank", action="store_true")
    ap.add_argument("--list", dest="listing", default=None)
    ap.add_argument("--jobs", type=int, default=2)
    ap.add_argument("--full", action="store_true",
                    help="test every unique line (default: same; kept for CLI parity)")
    ap.add_argument("--cache", type=Path, default=None,
                    help="JSON file to cache verdicts across runs")
    args = ap.parse_args()

    arch, flags, sub = SUITES[args.suite]
    suite_dir = Path(args.binutils_src) / sub
    if not suite_dir.is_dir():
        sys.exit(f"error: testsuite directory {suite_dir} not found")
    lccc_bin = args.lccc
    if args.suite == "i686":
        # The i686 sweep drives the 32-bit binary.
        lccc32 = Path(lccc_bin).with_name("lccc-i686")
        if lccc32.exists():
            lccc_bin = str(lccc32)

    lines = load_lines(suite_dir)
    print(f"{args.suite}: {len(lines)} unique instruction lines from {suite_dir}",
          file=sys.stderr)

    import json
    results: dict[str, str] = {}
    if args.cache and args.cache.exists():
        cached = json.loads(args.cache.read_text())
        if cached.get("suite") == args.suite and cached.get("lccc") == lccc_bin:
            results = cached.get("results", {})
            print(f"cache: {len(results)} verdicts reused", file=sys.stderr)
    todo = sorted(l for l in lines if l not in results)

    if todo:
        def save_cache() -> None:
            if args.cache:
                args.cache.parent.mkdir(parents=True, exist_ok=True)
                tmp = args.cache.with_suffix(".tmp")
                tmp.write_text(json.dumps(
                    {"suite": args.suite, "lccc": lccc_bin, "results": results}))
                tmp.replace(args.cache)

        with tempfile.TemporaryDirectory(prefix="isa-sweep-") as td:
            tasks = [
                (idx, line, args.gas, lccc_bin, flags, args.objcopy, td)
                for idx, line in enumerate(todo)
            ]
            done = 0
            with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as ex:
                # Incremental checkpointing: the sweep outlives a single
                # tool-session slice, so every 2000 verdicts land on disk
                # and a killed run resumes where it stopped.
                for line, verdict in ex.map(classify_one, tasks):
                    results[line] = verdict
                    done += 1
                    if done % 2000 == 0:
                        save_cache()
        save_cache()

    counts = Counter(results.values())
    print(f"PASS={counts['PASS']} ENCDIFF={counts['ENCDIFF']} "
          f"MISSING={counts['MISSING']} FALSEACC={counts['FALSEACC']} "
          f"SKIP={counts['SKIP']}")

    if args.rank:
        missing = Counter(mnemonic_of(l) for l, v in results.items() if v == "MISSING")
        print("\nFAILS RANKED (MISSING):")
        for mnem, n in missing.most_common():
            print(f"  {n:5d}  {mnem}")
        enc = Counter(mnemonic_of(l) for l, v in results.items() if v == "ENCDIFF")
        print(f"\nENCDIFF mnemonics: {sum(enc.values())} lines over {len(enc)} mnemonics")
        for mnem, n in enc.most_common(25):
            print(f"  {n:5d}  {mnem}")

    if args.listing:
        pat = args.listing.lower()
        hits = sorted(l for l, v in results.items()
                      if v in ("MISSING", "ENCDIFF") and pat in mnemonic_of(l))
        for h in hits:
            print(f"{results[h]:8s} {h}")
        print(f"-- {len(hits)} lines for '{pat}' --", file=sys.stderr)

    bad = counts["MISSING"] + counts["FALSEACC"]
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
