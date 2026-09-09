#!/usr/bin/env python3
"""Cross-backend execution oracle: does every LCCC target agree with GCC?

WHY THIS EXISTS
---------------
LCCC has four production backends (x86-64, i686, AArch64, RISC-V) that share
the front end, the IR and the optimiser, but each has its *own* code generator.
Every one of them had defects that no amount of x86-64 testing could see:

* AArch64 silently dropped the `state[3]` term of `sha256_transform`'s
  checksum (a fused `eor x0, x1, x2, lsl #16` whose shifted operand was
  materialised as zero) — and the kernel still passed its own known-answer
  self-check, so only a differential comparison against another target found
  it (BUG-2026-09-09).
* `lccc-arm -O0` refused to compile at all: the arch-agnostic driver emitted
  the x86 `pushq %rax` into AArch64 assembly.

Neither is visible from a whole-program exit status alone, and neither is
visible from assembly diffs. They are visible the moment the SAME program is
compiled for four targets and executed.

THE ORACLE
----------
A deterministic C program's observable behaviour (exit status + stdout) is a
property of the *program*, not of the target it was compiled for. So the host
GCC build is the reference and every LCCC backend must reproduce it exactly.
That removes the need for cross toolchains: the reference is always the native
one, and correctness is checked by execution, not by eyeballing assembly.

Binaries are linked statically and run natively (x86-64, i686) or under
`qemu-user` (AArch64, RISC-V).

WORKLOAD SCALING
----------------
The benchmark corpus is sized for timing runs (1 MiB buffers, dozens of
passes), which is far too slow under TCG. Every corpus program therefore has
its workload-size macros wrapped in `#ifndef` guards so a test run can shrink
them with `-D`; the defaults are unchanged, so the timing suite and the
benchmark-output oracle still see the full-size programs.

USAGE
-----
    # every program, every backend, -O0..-Os:
    scripts/cross_backend_check.py

    # a single program, with a JSON report:
    scripts/cross_backend_check.py --programs sha256_transform \\
        --json /tmp/cross.json

    # CI gate (non-zero exit on any mismatch or compile failure):
    scripts/cross_backend_check.py --gate --opts -O0,-O2

Exit status is 0 when every backend on every selected program reproduces the
GCC reference, 1 when any backend disagrees or fails to compile, and 2 for
usage/environment problems.
"""
from __future__ import annotations

import argparse
import json
import os
import shlex
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
CORPUS = REPO_ROOT / "tests" / "benchmark" / "programs"

# Backend binary -> (qemu arch, sysroot for -L, reference compiler flags).
#
# The reference must model the SAME C ABI as the target: `unsigned long` is
# 64-bit on x86-64/AArch64/RISC-V (LP64) but 32-bit on i686 (ILP32), so a
# checksum printed with `%lu` legitimately differs between the two models.
# Comparing i686 output against a 32-bit reference (`gcc -m32`) keeps the
# oracle honest — otherwise every LP64/ILP32 difference is a false positive.
BACKENDS: dict[str, tuple[str, str | None, list[str]]] = {
    "lccc": ("x86_64", None, []),
    "lccc-i686": ("i386", None, ["-m32"]),
    "lccc-arm": ("aarch64", "/usr/aarch64-linux-gnu", []),
    "lccc-riscv": ("riscv64", "/usr/riscv64-linux-gnu", []),
}

# Workload macros honoured by the corpus (see the #ifndef guards added to
# tests/benchmark/programs/*.c). Unused -D defines are harmless, so the whole
# set is passed to every program; only size macros are listed (never
# algorithmic constants such as MAX_ITER / NMAX / SLOT_2_0).
DEFAULT_SCALE: dict[str, str] = {
    "BLOCK_COUNT": "64",
    "PASSES": "1",
    "XML_SIZE": "4096",
    "QUERY_COUNT": "64",
    "HAYSTACK_LEN": "8192",
    "WORD_COUNT": "2048",
    "DATA_SIZE": "4096",
    "VALUE_COUNT": "4096",
    "SRC_SIZE": "4096",
    "BUFFER_SIZE": "4096",
    "NODE_COUNT": "512",
    "LOOKUP_ROUNDS": "1",
    "TABLE_SIZE": "512",
    "NUM_OPS": "256",
    "SIZE": "256",
    "N": "256",
    "WIDTH": "64",
    "HEIGHT": "64",
    "NBODIES": "32",
    "NSTRINGS": "32",
    "MAX_LEN": "32",
    "LANES": "4",
}

# Programs that need a host facility the emulated targets cannot provide.
SKIP = {
    "tls_seg_access.c": "thread-local storage needs a target libc TLS layout",
    "vecreg_new_ops.c": "x86 SIMD intrinsics (immintrin.h) — x86-only program",
}


def qemu_for(arch: str) -> str:
    return f"qemu-{arch}"


def backend_available(binary: Path, arch: str) -> tuple[bool, str]:
    if not binary.exists():
        return False, f"{binary} not built"
    if arch in ("x86_64", "i386"):
        return True, ""
    qemu = qemu_for(arch)
    if shutil.which(qemu) is None:
        return False, f"{qemu} not installed"
    return True, ""


def run_binary(path: Path, arch: str, sysroot: str | None, timeout: int) -> tuple[int, str]:
    if sysroot:
        argv = [qemu_for(arch), "-L", sysroot, str(path)]
    else:
        argv = [str(path)]
    try:
        proc = subprocess.run(argv, capture_output=True, text=True, timeout=timeout)
        return proc.returncode, proc.stdout
    except subprocess.TimeoutExpired:
        return -999, "<<TIMEOUT>>"
    except OSError as exc:
        return -998, f"<<exec failed: {exc}>>"


def compile_with(cmd: list[str], src: Path, out: Path, defines: list[str],
                 flags: list[str], timeout: int) -> tuple[bool, str]:
    argv = cmd + flags + defines + [str(src), "-o", str(out)]
    try:
        proc = subprocess.run(argv, capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        return False, "compile timeout"
    except OSError as exc:
        return False, f"compile exec failed: {exc}"
    if proc.returncode != 0 or not out.exists():
        msg = (proc.stderr or proc.stdout or "compile failed").strip()
        return False, msg[:400]
    return True, ""


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--corpus", type=Path, default=CORPUS,
                    help="directory of C programs (default: tests/benchmark/programs)")
    ap.add_argument("--programs", default="",
                    help="comma-separated subset of program stems")
    ap.add_argument("--backends", default=",".join(BACKENDS),
                    help="comma-separated backend binary names")
    ap.add_argument("--opts", default="-O0,-O1,-O2,-O3,-Os",
                    help="comma-separated optimization levels")
    ap.add_argument("--reference", default="gcc", help="reference compiler (default: gcc)")
    ap.add_argument("--no-scale", action="store_true",
                    help="do not shrink the workload macros (full-size runs)")
    ap.add_argument("--define", action="append", default=[],
                    metavar="NAME=VALUE", help="extra/override -D define")
    ap.add_argument("--lccc-dir", type=Path, default=REPO_ROOT / "target" / "fastbuild")
    ap.add_argument("--timeout", type=int, default=120, help="per-run timeout (s)")
    ap.add_argument("--json", type=Path, help="write the full report")
    ap.add_argument("--gate", action="store_true",
                    help="exit 1 if any backend fails to compile or disagrees")
    ap.add_argument("--verbose", "-v", action="store_true")
    args = ap.parse_args(argv)

    if not args.corpus.is_dir():
        print(f"FATAL: corpus {args.corpus} is not a directory", file=sys.stderr)
        return 2

    programs = sorted(args.corpus.glob("*.c"))
    if args.programs:
        wanted = {p.strip() for p in args.programs.split(",") if p.strip()}
        programs = [p for p in programs if p.stem in wanted]

    defines: list[str] = []
    if not args.no_scale:
        defines += [f"-D{k}={v}" for k, v in DEFAULT_SCALE.items()]
    defines += [f"-D{d}" for d in args.define]

    backends: dict[str, tuple[str, str | None]] = {}
    for name in (b.strip() for b in args.backends.split(",") if b.strip()):
        if name not in BACKENDS:
            print(f"FATAL: unknown backend {name!r}", file=sys.stderr)
            return 2
        backends[name] = BACKENDS[name]

    usable: list[tuple[str, str, str | None, Path, list[str]]] = []
    for name, (arch, sysroot, refflags) in backends.items():
        binary = args.lccc_dir / name
        ok, why = backend_available(binary, arch)
        if ok:
            usable.append((name, arch, sysroot, binary, refflags))
        else:
            print(f"SKIP  {name}: {why}", file=sys.stderr)

    if not usable:
        print("FATAL: no usable backends", file=sys.stderr)
        return 2

    failures: list[str] = []
    report: dict[str, object] = {
        "tool": "cross_backend_check.py",
        "reference": args.reference,
        "opts": args.opts.split(","),
        "scale": {} if args.no_scale else DEFAULT_SCALE,
        "results": [],
    }

    print(f"{'program':<28} {'opt':<4} " +
          " ".join(f"{n:<11}" for n, *_ in usable))
    print("-" * (34 + 12 * len(usable)))

    with tempfile.TemporaryDirectory(prefix="crossbe-") as td:
        tmp = Path(td)
        for src in programs:
            if src.name in SKIP:
                print(f"SKIP  {src.name}: {SKIP[src.name]}")
                continue
            for opt in (o.strip() for o in args.opts.split(",") if o.strip()):
                flags = [opt, "-static"]
                # One reference per ABI model (LP64 vs ILP32): the expected
                # output is a property of the program AND the data model.
                refs: dict[str, tuple[int, str]] = {}
                for name, _arch, _sysroot, _binary, refflags in usable:
                    model = "ilp32" if "-m32" in refflags else "lp64"
                    if model in refs:
                        continue
                    ref_bin = tmp / f"{src.stem}.ref{model}{opt}"
                    ok, err = compile_with([args.reference] + refflags, src,
                                           ref_bin, defines, flags, args.timeout)
                    if not ok:
                        print(f"{src.name:<28} {opt:<4} reference ({model}) failed: {err[:70]}")
                        failures.append(f"{src.name}{opt}: {model} reference compile failed")
                        continue
                    refs[model] = run_binary(ref_bin, "x86_64", None, args.timeout)
                if not ok:
                    print(f"{src.name:<28} {opt:<4} reference gcc failed: {err[:80]}")
                    failures.append(f"{src.name}{opt}: reference compile failed")
                    continue
                if not refs:
                    continue

                row: list[str] = []
                entry: dict[str, object] = {
                    "program": src.name,
                    "opt": opt,
                    "reference": {
                        m: {"exit": rc, "stdout_head": out[:200]}
                        for m, (rc, out) in refs.items()
                    },
                    "backends": {},
                }
                for name, arch, sysroot, binary, refflags in usable:
                    model = "ilp32" if "-m32" in refflags else "lp64"
                    if model not in refs:
                        continue
                    ref_rc, ref_out = refs[model]
                    bin_path = tmp / f"{src.stem}.{name}{opt}"
                    ok, err = compile_with([str(binary)], src, bin_path, defines,
                                           flags, args.timeout)
                    if not ok:
                        row.append("COMPILEFAIL")
                        entry["backends"][name] = {"compile_ok": False, "error": err}
                        failures.append(f"{src.name} {opt} {name}: compile failed: {err[:120]}")
                        continue
                    rc, out = run_binary(bin_path, arch, sysroot, args.timeout)
                    same = (rc == ref_rc and out == ref_out)
                    verdict = "ok" if same else "MISMATCH"
                    row.append(verdict)
                    rec: dict[str, object] = {
                        "compile_ok": True, "exit": rc, "matches": same,
                    }
                    if not same:
                        rec["expected_exit"] = ref_rc
                        rec["stdout_head"] = out[:200]
                        rec["expected_stdout_head"] = ref_out[:200]
                        failures.append(
                            f"{src.name} {opt} {name}: exit {rc} != {ref_rc} or stdout differs")
                    entry["backends"][name] = rec
                print(f"{src.name:<28} {opt:<4} " + " ".join(f"{c:<11}" for c in row))
                report["results"].append(entry)  # type: ignore[union-attr]

    if args.json:
        args.json.parent.mkdir(parents=True, exist_ok=True)
        args.json.write_text(json.dumps(report, indent=2))
        print(f"\nwrote {args.json}")

    if failures:
        print(f"\n{len(failures)} FAILURE(S):")
        for f in failures[:40]:
            print(f"  - {f}")
        if len(failures) > 40:
            print(f"  … and {len(failures) - 40} more")
        return 1 if args.gate else 1
    print("\nALL BACKENDS AGREE WITH GCC")
    return 0


if __name__ == "__main__":
    sys.exit(main())
