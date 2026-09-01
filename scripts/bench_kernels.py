#!/usr/bin/env python3
"""Runtime A/B benchmark of lccc against system GCC/Clang on real hot loops.

Instruction counts from the Compiler Explorer oracle say which compiler emits
*fewer* instructions, not which emits *faster* code -- a vectorised loop with
more instructions usually wins, and a shorter loop that spills does not. This
harness measures wall-clock time of the same kernel compiled by each compiler
and reports the ratio, so a codegen claim can be backed by a number.

Method, chosen because this VM exposes no PMU:

  * each kernel is a separate translation unit compiled by each compiler, then
    linked against a shared driver compiled ONCE by a fixed reference compiler,
    so only the kernel's codegen differs between arms;
  * the driver runs the kernel `--inner` times per timed sample and reports the
    best of `--reps` samples. Minimum, not mean: it is the sample least
    perturbed by scheduler noise, and the distribution is one-sided;
  * the kernel result is accumulated into a volatile sink and printed, so no
    arm can delete the work, and mismatched output between arms is reported as
    a CORRECTNESS failure rather than a speed win;
  * arms are interleaved round-robin across repetitions rather than run in
    blocks, so slow thermal or noisy-neighbour drift hits every arm equally.

Usage:
    scripts/bench_kernels.py                          # all kernels, default arms
    scripts/bench_kernels.py --kernels adler32,memchr
    scripts/bench_kernels.py --reps 9 --inner 2000
    scripts/bench_kernels.py --baseline before.json --save after.json
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import os
import shutil
import statistics
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
BENCH_DIR = REPO / "tests" / "bench"


def which(*names: str) -> str | None:
    for n in names:
        p = shutil.which(n)
        if p:
            return p
    return None


def timed_function_hash(obj: Path) -> str:
    """Fingerprint of the TIMED function's machine code, and nothing else.

    Hashing the whole object is too coarse: `bench_setup` is compiled into the
    same translation unit but runs once, outside the timed region, so a change
    there must not be attributed to a runtime delta. Disassemble `bench_run`
    alone and strip the address column, which moves whenever anything ahead of
    it changes size, and the relocation-dependent RIP displacements.

    Falls back to hashing the whole object if objdump is unavailable, which
    only makes the artifact filter more conservative (fewer deltas classified
    as noise).
    """
    try:
        r = subprocess.run(
            ["objdump", "-d", "--disassemble=bench_run", str(obj)],
            capture_output=True, text=True, timeout=60,
        )
        if r.returncode == 0 and "bench_run" in r.stdout:
            body = []
            for line in r.stdout.splitlines():
                if "\t" not in line:
                    continue
                parts = line.split("\t")
                # parts[0] = "  30:", parts[1] = raw bytes, parts[2] = mnemonic
                text = parts[-1].strip()
                # Drop RIP-relative displacements and branch targets: both are
                # pure position, not code.
                text = re.sub(r"0x[0-9a-f]+\(%rip\)", "RIP", text)
                text = re.sub(r"^(j\w+|call\w*)\s+[0-9a-f]+", r"\1 T", text)
                text = re.sub(r"#.*$", "", text).strip()
                body.append(text)
            if body:
                return hashlib.sha256("\n".join(body).encode()).hexdigest()[:16]
    except (OSError, subprocess.SubprocessError):
        pass
    return hashlib.sha256(obj.read_bytes()).hexdigest()[:16]


def build_arm(cc: str, cflags: list[str], kernel: Path, driver_obj: Path,
              out: Path, workdir: Path) -> tuple[bool, str, str]:
    """Compile + link one arm. Returns (ok, error, codegen_hash)."""
    kobj = workdir / f"{kernel.stem}.{Path(cc).name}.o"
    r = subprocess.run([cc, "-c", str(kernel), "-o", str(kobj), *cflags],
                       capture_output=True, text=True)
    if r.returncode != 0:
        return False, f"kernel compile failed: {r.stderr.strip()[:300]}", ""
    # Hash the kernel object. Relocatable objects carry no timestamp, so this
    # is a stable fingerprint of the generated code -- see `codegen_hash` use
    # in the baseline comparison below.
    digest = timed_function_hash(kobj)
    # Link with the reference compiler so the linker is identical across arms.
    r = subprocess.run([REFERENCE_CC, str(kobj), str(driver_obj), "-o", str(out), "-lm"],
                       capture_output=True, text=True)
    if r.returncode != 0:
        return False, f"link failed: {r.stderr.strip()[:300]}", ""
    return True, "", digest


def time_once(binary: Path, inner: int) -> tuple[float, str] | None:
    r = subprocess.run([str(binary), str(inner)], capture_output=True, text=True,
                       timeout=300)
    if r.returncode != 0:
        return None
    # Driver prints: "<seconds> <checksum>"
    parts = r.stdout.split()
    if len(parts) < 2:
        return None
    try:
        return float(parts[0]), parts[1]
    except ValueError:
        return None


REFERENCE_CC = "cc"


def main() -> int:
    global REFERENCE_CC
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--lccc", default=str(REPO / "target/fastbuild/lccc"))
    ap.add_argument("--gcc", default=which("gcc", "cc"))
    ap.add_argument("--clang", default=which("clang"))
    ap.add_argument("--reference-cc", default=which("gcc", "cc"),
                    help="compiler used for the driver and for linking every arm")
    ap.add_argument("--flags", default="-O3",
                    help="optimisation flags given to every arm")
    ap.add_argument("--kernels", default="", help="comma-separated kernel names")
    ap.add_argument("--reps", type=int, default=7, help="timed samples per arm")
    ap.add_argument("--inner", type=int, default=0,
                    help="iterations per sample (0 = per-kernel default)")
    ap.add_argument("--save", metavar="JSON")
    ap.add_argument("--baseline", metavar="JSON")
    ap.add_argument("--tolerance", type=float, default=3.0,
                    help="percent slowdown vs baseline that counts as a regression")
    args = ap.parse_args()

    if not args.reference_cc:
        print("error: no reference C compiler found", file=sys.stderr)
        return 2
    REFERENCE_CC = args.reference_cc

    driver = BENCH_DIR / "driver.c"
    if not driver.exists():
        print(f"error: missing {driver}", file=sys.stderr)
        return 2

    kernels = sorted(p for p in BENCH_DIR.glob("k_*.c"))
    if args.kernels:
        want = {k.strip() for k in args.kernels.split(",") if k.strip()}
        kernels = [k for k in kernels if k.stem[2:] in want or k.stem in want]
    if not kernels:
        print("error: no kernels selected", file=sys.stderr)
        return 2

    arms: list[tuple[str, str]] = []
    if args.lccc and Path(args.lccc).exists():
        arms.append(("lccc", args.lccc))
    if args.gcc:
        arms.append(("gcc", args.gcc))
    if args.clang:
        arms.append(("clang", args.clang))
    if len(arms) < 2:
        print("error: need at least two compilers to compare", file=sys.stderr)
        return 2

    cflags = args.flags.split()
    results: dict[str, dict[str, float]] = {}
    hashes: dict[str, dict[str, str]] = {}
    failures: list[str] = []

    with tempfile.TemporaryDirectory(prefix="lccc-bench-") as td:
        work = Path(td)
        driver_obj = work / "driver.o"
        r = subprocess.run([REFERENCE_CC, "-O2", "-c", str(driver), "-o", str(driver_obj)],
                           capture_output=True, text=True)
        if r.returncode != 0:
            print(f"error: driver build failed:\n{r.stderr}", file=sys.stderr)
            return 2

        for kernel in kernels:
            name = kernel.stem[2:]
            inner = args.inner or default_inner(name)
            binaries: dict[str, Path] = {}
            for arm, cc in arms:
                out = work / f"{name}.{arm}"
                ok, err, digest = build_arm(cc, cflags, kernel, driver_obj, out, work)
                if not ok:
                    failures.append(f"{name}/{arm}: {err}")
                    continue
                binaries[arm] = out
                hashes.setdefault(name, {})[arm] = digest
            if len(binaries) < 2:
                continue

            # Interleave arms across repetitions so drift is shared.
            samples: dict[str, list[float]] = {a: [] for a in binaries}
            checksums: dict[str, set[str]] = {a: set() for a in binaries}
            for _ in range(args.reps):
                for arm, binary in binaries.items():
                    got = time_once(binary, inner)
                    if got is None:
                        failures.append(f"{name}/{arm}: run failed")
                        continue
                    secs, checksum = got
                    samples[arm].append(secs)
                    checksums[arm].add(checksum)

            # A faster arm that computes something else is not faster.
            distinct = {next(iter(v)) for v in checksums.values() if v}
            if len(distinct) > 1:
                failures.append(
                    f"{name}: CORRECTNESS - arms disagree: "
                    + ", ".join(f"{a}={next(iter(v))}" for a, v in checksums.items() if v)
                )
                continue

            results[name] = {a: min(s) for a, s in samples.items() if s}
            results[name]["_hash_lccc"] = hashes.get(name, {}).get("lccc", "")

    # ── report ──────────────────────────────────────────────────────────────
    arm_names = [a for a, _ in arms]
    print()
    print(f"runtime benchmark  flags={args.flags!r}  reps={args.reps} (best-of)")
    print()
    header = f"{'KERNEL':<18}" + "".join(f"{a + ' (ms)':>14}" for a in arm_names)
    if "lccc" in arm_names:
        header += f"{'vs gcc':>10}{'vs clang':>10}"
    print(header)
    print("-" * len(header))
    for name in sorted(results):
        row = results[name]
        line = f"{name:<18}"
        for a in arm_names:
            line += f"{row.get(a, float('nan')) * 1e3:>14.3f}" if a in row else f"{'-':>14}"
        if "lccc" in row:
            for other in ("gcc", "clang"):
                if other in row and row[other] > 0:
                    ratio = row[other] / row["lccc"]
                    line += f"{ratio:>9.2f}x"
                else:
                    line += f"{'-':>10}"
        print(line)

    if results and "lccc" in arm_names:
        print()
        for other in ("gcc", "clang"):
            ratios = [r[other] / r["lccc"] for r in results.values()
                      if other in r and "lccc" in r and r["lccc"] > 0]
            if ratios:
                print(f"  geomean vs {other}: {statistics.geometric_mean(ratios):.3f}x "
                      f"(>1 means lccc is faster)")

    for f in failures:
        print(f"  FAIL {f}")

    if args.save:
        Path(args.save).write_text(json.dumps(results, indent=2, sort_keys=True) + "\n")
        print(f"\n  saved -> {args.save}")

    if args.baseline:
        base = json.loads(Path(args.baseline).read_text())
        regressed = 0
        noise = 0
        print()
        for name, row in sorted(results.items()):
            if "lccc" not in row or name not in base or "lccc" not in base[name]:
                continue
            was, now = base[name]["lccc"], row["lccc"]
            delta = (now - was) / was * 100.0
            if abs(delta) <= args.tolerance:
                continue

            # CODE-LAYOUT ARTIFACT FILTER.
            #
            # A change elsewhere in the translation unit shifts the benchmarked
            # function's address, and a 16-byte shift alone is worth several
            # percent on this core (instruction-fetch and uop-cache windows).
            # Observed for real: a fold that only touched the UNTIMED
            # `bench_setup` moved `bench_run` from 0x1270 to 0x1260 and was
            # reported as a 3.7% regression, while the two `bench_run` bodies
            # were byte-for-byte identical.
            #
            # The kernel object's hash settles it. Identical codegen means the
            # delta cannot be attributed to the change under test, so report it
            # as noise instead of manufacturing a regression -- or, just as
            # importantly, a win.
            old_hash = base[name].get("_hash_lccc", "")
            new_hash = row.get("_hash_lccc", "")
            same_codegen = bool(old_hash) and old_hash == new_hash

            if same_codegen:
                print(f"  NOISE     {name}: {was*1e3:.3f} -> {now*1e3:.3f} ms "
                      f"({delta:+.1f}%) - kernel codegen is IDENTICAL "
                      f"({new_hash}); this is layout/measurement noise")
                noise += 1
            elif delta > 0:
                print(f"  REGRESSED {name}: {was*1e3:.3f} -> {now*1e3:.3f} ms "
                      f"({delta:+.1f}%)")
                regressed += 1
            else:
                print(f"  IMPROVED  {name}: {was*1e3:.3f} -> {now*1e3:.3f} ms "
                      f"({delta:+.1f}%)")

        if noise:
            print(f"\n  note: {noise} kernel(s) moved beyond the tolerance with "
                  f"UNCHANGED codegen. That is the layout-noise floor of this "
                  f"host; treat deltas smaller than it as unmeasurable.")
        if regressed:
            print(f"\n  FAIL: {regressed} kernel(s) regressed beyond "
                  f"{args.tolerance}% WITH changed codegen")
            return 1
        print("\n  OK: no runtime regressions against baseline")

    return 1 if failures else 0


def default_inner(name: str) -> int:
    """Per-kernel iteration counts, sized so each sample runs ~20-80 ms."""
    return {
        "adler32": 3000,
        "matchlen": 20000,
        "namechars": 3000,
        "classify": 3000,
        "varint": 20000,
        "memchr": 3000,
        "hashmix": 20000,
    }.get(name, 3000)


if __name__ == "__main__":
    sys.exit(main())
