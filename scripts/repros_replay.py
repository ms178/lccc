#!/usr/bin/env python3
"""Replay the historical reproducers in `artifacts/repros/` against the current
compiler (AUD-HYGIENE-1).

Those files are the permanent evidence of every crash (`crash_*`) and
miscompile (`miscompile_*`) this project has found.  They are only worth
keeping while they are *checked*: a reproducer that stopped reproducing is a
fixed bug (and evidence), one that still reproduces is an open bug -- and a
directory nobody replays silently accumulates files that no longer compile at
all, which is how 100 reproducers became unreviewable in the first place.

Contract per file
-----------------
* the optimization level encoded in the file name (`_-O0` / `_-O2`) is used;
* the file must compile with the current LCCC (a crash reproducer that no
  longer crashes means the bug is fixed: it must now compile *and* run);
* if the file has `main`, LCCC's output and exit code must equal GCC's --
  same-compiler agreement proves nothing about a miscompile;
* the GCC arm gets `-march=x86-64-v3`, because that IS lccc's documented
  default code-generation target (absent `-march=`).  Comparing a v3 binary
  against a baseline-x86-64 binary reports the ISA difference (FMA contraction,
  AVX2 vectorisation) as a miscompile, which is exactly how a whole family of
  `miscompile_gen_fp_stress_*` reproducers was filed incorrectly;
* files without `main` are compile-only checks.

Known-still-failing files live in `scripts/repros_replay_allow.txt` (one path
per line, with a reason after `#`), so the gate ratchets down instead of
turning CI red on a pre-existing bug -- and a file that starts PASSING while
still allow-listed is reported (stale entry), which keeps the list honest.

Usage
-----
    scripts/repros_replay.py                 # replay everything, report
    scripts/repros_replay.py --check         # gate: fail on new failures (CI)
    scripts/repros_replay.py --update-allowlist
    scripts/repros_replay.py --selftest
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
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
REPRO_DIR = REPO / "artifacts" / "repros"
ALLOWLIST = REPO / "scripts" / "repros_replay_allow.txt"
OPT_RE = re.compile(r"_-(O[0-3s])\.c$")


GCC_ISA = "-march=x86-64-v3"


def opt_level(path: Path) -> str:
    match = OPT_RE.search(path.name)
    return f"-{match.group(1)}" if match else "-O2"


def gcc_include() -> list[str]:
    try:
        out = subprocess.run(
            ["gcc", "-print-file-name=include"], capture_output=True, text=True, check=True
        ).stdout.strip()
    except (OSError, subprocess.SubprocessError):
        return []
    return [f"-I{out}"] if out else []


def compile_and_run(
    cc: str, src: Path, work: Path, opt: str, inc: list[str], run: bool, isa: str = ""
) -> tuple[str, str]:
    """Return ("ok", output) or ("skip"/"fail", message)."""
    binary = work / f"{src.stem}.{Path(cc).name}.bin"
    extra = [isa] if isa else []
    proc = subprocess.run(
        [cc, *inc, opt, *extra, str(src), "-o", str(binary)],
        capture_output=True,
        text=True,
        timeout=180,
    )
    if proc.returncode != 0:
        first = proc.stderr.strip().splitlines()[:1]
        text = first[0] if first else ""
        if "csmith.h" in text:
            # The generated-program reproducers need the Csmith runtime
            # headers (tests/corpus provides them); without that harness the
            # file cannot be replayed here.  A skip is loud, never a pass.
            return "skip", f"needs csmith.h: {text[:120]}"
        return "fail", f"compile failed: {text[:200]}"
    if not run:
        return "ok", ""
    try:
        executed = subprocess.run([str(binary)], capture_output=True, text=True, timeout=60)
    except subprocess.TimeoutExpired:
        return "fail", "timed out at run time"
    return "ok", f"{executed.stdout}|{executed.returncode}"


def replay_one(lccc: str, gcc: str, src: Path, work: Path, inc: list[str]) -> tuple[str, str]:
    opt = opt_level(src)
    has_main = "int main" in src.read_text(errors="ignore")
    status, lccc_out = compile_and_run(lccc, src, work, opt, inc, has_main)
    if status != "ok":
        return "lccc-" + status, lccc_out
    if not has_main:
        return "ok", "compile-only"
    status, gcc_out = compile_and_run(gcc, src, work, opt, inc, True, isa=GCC_ISA)
    if status != "ok":
        return "gcc-" + status, gcc_out
    if lccc_out != gcc_out:
        return "mismatch", f"lccc={lccc_out!r} gcc={gcc_out!r}"
    return "ok", ""


def load_allowlist() -> dict[str, str]:
    if not ALLOWLIST.is_file():
        return {}
    entries = {}
    for line in ALLOWLIST.read_text().splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        name, _, reason = line.partition("#")
        entries[name.strip()] = reason.strip()
    return entries


def selftest() -> int:
    tmp = Path(tempfile.mkdtemp(prefix="repros-replay-selftest."))
    try:
        lccc = tmp / "fake-lccc"
        gcc = tmp / "fake-gcc"
        lccc.write_text("#!/bin/sh\n# emits a binary that always prints L\nexec gcc \"$@\"\n")
        gcc.write_text("#!/bin/sh\nexec gcc \"$@\"\n")
        for p in (lccc, gcc):
            p.chmod(0o755)
        (tmp / "hello.c").write_text('#include <stdio.h>\nint main(void){puts("x");return 0;}\n')
        work = tmp / "w"
        work.mkdir()
        status, _ = replay_one(str(lccc), str(gcc), tmp / "hello.c", work, [])
        if status != "ok":
            print(f"selftest FAIL: identical arms reported {status}", file=sys.stderr)
            return 1
        # A compiler that mangles the output must be reported as a mismatch.
        liar = tmp / "liar-cc"
        liar.write_text(
            "#!/bin/sh\n"
            "out=\"\"\n"
            "while [ $# -gt 0 ]; do\n"
            "  if [ \"$1\" = \"-o\" ]; then out=\"$2\"; fi\n"
            "  shift\n"
            "done\n"
            "tmp=$(mktemp -d)\n"
            "printf '#include <stdio.h>\\nint main(void){puts(\"DIFFERENT\");return 0;}\\n' > \"$tmp/m.c\"\n"
            "gcc \"$tmp/m.c\" -o \"$out\"\n"
        )
        liar.chmod(0o755)
        status, _ = replay_one(str(liar), str(gcc), tmp / "hello.c", work, [])
        if status != "mismatch":
            print(f"selftest FAIL: divergent output reported as {status}", file=sys.stderr)
            return 1
        print("repros replay selftest: PASS")
        return 0
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--lccc", default=str(REPO / "target" / "fastbuild" / "lccc"))
    ap.add_argument("--gcc", default=shutil.which("gcc") or "gcc")
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--update-allowlist", action="store_true")
    ap.add_argument("--jobs", type=int, default=max(1, (os.cpu_count() or 2)))
    ap.add_argument("--selftest", action="store_true")
    args = ap.parse_args()

    if args.selftest:
        return selftest()
    if not REPRO_DIR.is_dir():
        print(f"FAIL: {REPRO_DIR} is missing", file=sys.stderr)
        return 2
    if not Path(args.lccc).is_file():
        print(f"FAIL: compiler {args.lccc} not built", file=sys.stderr)
        return 2

    sources = sorted(REPRO_DIR.glob("*.c"))
    inc = gcc_include()
    results: dict[str, tuple[str, str]] = {}
    with tempfile.TemporaryDirectory(prefix="repros-replay.") as td, concurrent.futures.ThreadPoolExecutor(
        max_workers=args.jobs
    ) as pool:
        futures = {
            pool.submit(replay_one, args.lccc, args.gcc, src, Path(td), inc): src for src in sources
        }
        for future in concurrent.futures.as_completed(futures):
            src = futures[future]
            try:
                results[src.name] = future.result()
            except (OSError, subprocess.SubprocessError) as exc:
                results[src.name] = ("harness-error", str(exc))

    allow = load_allowlist()
    skipped = {n: r for n, (status, r) in results.items() if status.endswith("skip")}
    failures = {n: r for n, (status, r) in results.items() if status != "ok" and not status.endswith("skip")}
    new_failures = {n: r for n, r in failures.items() if n not in allow}
    stale = [n for n, (status, _) in results.items() if status == "ok" and n in allow]

    print(f"repros replayed: {len(results)}  passing: {len(results) - len(failures) - len(skipped)}  "
          f"failing: {len(failures)} (allow-listed: {len(failures) - len(new_failures)})  skipped: {len(skipped)}")
    for name in sorted(skipped):
        print(f"SKIP: {name}: {skipped[name]}")
    for name in sorted(new_failures):
        print(f"FAIL: {name}: {new_failures[name]}")
    for name in sorted(stale):
        print(f"NOTE: {name} passes now; drop it from {ALLOWLIST.name}")

    if args.update_allowlist:
        header = (
            "# Reproducers in artifacts/repros/ that are still known-failing.\n"
            "# One file name per line, optional `# reason`.  Entries are owed an\n"
            "# audit item id; a passing entry is reported as stale by the gate.\n"
        )
        body = "".join(f"{n}\n" for n in sorted(failures))
        ALLOWLIST.write_text(header + body)
        print(f"allowlist updated: {ALLOWLIST} ({len(failures)} entries)")
        return 0

    if args.check and new_failures:
        print("repros replay: FAIL (new failures above)", file=sys.stderr)
        return 1
    if failures:
        print(f"repros replay: PASS (all {len(failures)} failures are allow-listed)")
    else:
        print("repros replay: PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
