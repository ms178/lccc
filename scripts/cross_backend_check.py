#!/usr/bin/env python3
"""Cross-backend execution oracle: does every LCCC target agree with the reference compilers?

WHY THIS EXISTS
---------------
LCCC has four production backends (x86-64, i686, AArch64, RISC-V) that share
the front end, the IR and the optimiser, but each has its *own* code generator.
Every defect this oracle has found was invisible to single-target testing and
to assembly diffing:

* AArch64 silently dropped the `state[3]` term of `sha256_transform`'s
  checksum (a fused `eor x0, x1, x2, lsl #16` whose shifted operand was
  materialised as zero) — and the kernel still passed its own known-answer
  self-check, so only a differential comparison against another target found it.
* `lccc-arm -O0` refused to compile at all: the arch-agnostic driver emitted
  the x86 `pushq %rax` into AArch64 assembly.
* i686 miscompiled Expat's UTF-8 name scanner at every level ≥ -O1: a peephole
  `cmpw $128, %ax` silently compared an UNRELATED register.
* AArch64 `-Os` deleted the prologue's `mov x19, x24` while a later block still
  read `x19` as a pointer (SIGSEGV), and orphaned a value whose only home was
  `x0`.

THE ORACLE
----------
A deterministic C program's observable behaviour (exit status + stdout) is a
property of the *program*, not of the target it was compiled for — **provided
the reference models the target's ABI in both relevant dimensions**:

* DATA MODEL — `unsigned long` is 64-bit on x86-64/AArch64/RISC-V (LP64) and
  32-bit on i686 (ILP32), so a `%lu` checksum legitimately differs.
* FP EVALUATION MODEL — lccc evaluates `double` in strict IEEE (SSE2 on i686);
  GCC's i686 *default* is x87 with 80-bit excess precision, which legitimately
  differs in the last bits.

Get either wrong and the tool reports false positives; get them right and a
mismatch is a real defect. Binaries are linked statically and run natively
(x86-64, i686) or under `qemu-user` (AArch64, RISC-V).

MULTI-ORACLE
------------
`--reference` may be given more than once (default: every one of gcc/clang that
is installed). All references must agree with each other — if they do not, the
program's expected output is not well defined (excess precision, libm
differences, …) and it is SKIPPED rather than reported as a compiler bug. A
backend must then match *every* reference.

USAGE
-----
    scripts/cross_backend_check.py                      # whole corpus, -O0..-Os
    scripts/cross_backend_check.py --programs sha256_transform --repeat 3
    scripts/cross_backend_check.py --gate --quick       # CI-usable exit status
    scripts/cross_backend_check.py --compare baseline.json   # regressions only
    scripts/cross_backend_check.py --jobs 4 --json report.json

Exit status: 0 when every backend reproduces the references, 1 when a backend
disagrees or fails to compile (`--gate` makes timeouts fatal too), 2 for
usage/environment problems.
"""
from __future__ import annotations

import argparse
import difflib
import json
import os
import shutil
import subprocess
import sys
import tempfile
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
CORPUS = REPO_ROOT / "tests" / "benchmark" / "programs"

# Backend binary -> (qemu arch, sysroot for -L or None, reference compiler flags)
BACKENDS: dict[str, tuple[str, str | None, list[str]]] = {
    "lccc": ("x86_64", None, []),
    "lccc-i686": ("i386", None, ["-m32", "-msse2", "-mfpmath=sse"]),
    "lccc-arm": ("aarch64", "/usr/aarch64-linux-gnu", []),
    "lccc-riscv": ("riscv64", "/usr/riscv64-linux-gnu", []),
}

# Static linking needs `-lm` for programs that call sqrt/pow/…; without it the
# reference build dies with "undefined reference to `sqrt'" (nbody.c).
LINK_LIBS: list[str] = ["-lm"]

# Workload macros honoured by the corpus (see the #ifndef guards added to
# tests/benchmark/programs/*.c). Unused -D defines are harmless, so the whole
# set is passed to every program.
#
# ONLY pure size/step knobs belong here. A macro that also defines the program's
# DATA does not: nbody.c's `static Body bodies[NBODIES]` has five initialisers,
# so -DNBODIES=32 zero-fills 27 bodies and every energy term becomes NaN — the
# "mismatch" was the harness corrupting the program. Likewise never override
# algorithmic constants (MAX_ITER / NMAX / SLOT_2_0): they change the ANSWER.
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
    "STEPS": "20000",
    "WIDTH": "64",
    "HEIGHT": "64",
    "NSTRINGS": "32",
    "MAX_LEN": "32",
    "LANES": "4",
}

# Programs that need a facility the emulated targets cannot provide.
SKIP = {
    "tls_seg_access.c": "thread-local storage needs a target libc TLS layout",
    "vecreg_new_ops.c": "x86 SIMD intrinsics (immintrin.h) — x86-only program",
}


def qemu_for(arch: str) -> str:
    return f"qemu-{arch}"


def compile_with(cmd: list[str], src: Path, out: Path, defines: list[str],
                 flags: list[str], timeout: int) -> tuple[bool, str]:
    argv = cmd + flags + defines + [str(src), "-o", str(out)] + LINK_LIBS
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


def run_binary(path: Path, arch: str, sysroot: str | None, timeout: int) -> tuple[int, str]:
    """Run one binary; returns (exit code, stdout). -999 = timeout, -998 = exec error."""
    argv = [str(path)]
    if arch not in ("x86_64", "i386"):
        argv = [qemu_for(arch)]
        # Only pass -L when the sysroot really exists: static binaries do not
        # need it, and a stale path makes qemu fail before the program runs.
        if sysroot and Path(sysroot).is_dir():
            argv += ["-L", sysroot]
        argv += [str(path)]
    try:
        proc = subprocess.run(argv, capture_output=True, text=True, timeout=timeout)
        return proc.returncode, proc.stdout
    except subprocess.TimeoutExpired:
        return -999, "<<TIMEOUT>>"
    except OSError as exc:
        return -998, f"<<exec failed: {exc}>>"


def short_diff(expected: str, actual: str, limit: int = 4) -> str:
    """A compact, human-readable difference between two program outputs."""
    e = expected.splitlines()
    a = actual.splitlines()
    for i, (x, y) in enumerate(zip(e, a)):
        if x != y:
            return f"line {i + 1}: expected {x!r} got {y!r}"
    if len(e) != len(a):
        d = "\n".join(list(difflib.unified_diff(e, a, "expected", "actual", lineterm=""))[:limit])
        return f"line count {len(e)} vs {len(a)}: {d}"
    return ""


def preflight(references: list[str], usable: list[tuple[str, str, str | None, Path, list[str]]],
              fail_fast: bool) -> list[str]:
    """Verify the toolchain before burning 45 minutes on a broken setup."""
    problems: list[str] = []
    for ref in references:
        if shutil.which(ref) is None:
            problems.append(f"reference compiler {ref!r} not found")
    for name, arch, _sysroot, binary, refflags in usable:
        if not binary.exists():
            problems.append(f"backend binary {binary} not built (run scripts/build_lccc_fast.sh)")
        if arch not in ("x86_64", "i386") and shutil.which(qemu_for(arch)) is None:
            problems.append(f"{qemu_for(arch)} not installed (apt-get install qemu-user)")
        if "-m32" in refflags:
            import tempfile as _tf
            with _tf.NamedTemporaryFile("w", suffix=".c", delete=False) as fh:
                fh.write("int main(void){return 0;}\n")
                probe = fh.name
            ok, err = compile_with([references[0]] + refflags, Path(probe),
                                   Path(probe + ".out"), [], [], 60)
            Path(probe).unlink(missing_ok=True)
            Path(probe + ".out").unlink(missing_ok=True)
            if not ok:
                problems.append(f"32-bit reference unavailable for {name}: "
                                f"{err.splitlines()[0][:90]} (apt-get install gcc-multilib)")
    return problems


def evaluate_program(src: Path, opt: str, usable, references: list[str],
                     defines: list[str], gen_defines: list[str], flags: list[str],
                     scratch: Path, timeout: int, repeat: int
                     ) -> tuple[dict, list[str], list[str]]:
    """Compile + run one program at one opt level. Returns (entry, failures, timeouts)."""
    entry: dict[str, object] = {"program": src.name, "opt": opt, "backends": {}}
    failures: list[str] = []
    timeouts: list[str] = []
    row: list[str] = []

    with tempfile.TemporaryDirectory(prefix="crossbe-", dir=scratch) as td:
        tmp = Path(td)

        # --- references, one per ABI/FP model -----------------------------
        # The scale macros are applied to every program because unused -D
        # defines are harmless — unless a program uses one of those names as an
        # identifier (an enum member `N`, a variable `SIZE`), which is a
        # *program* compile error, not a compiler bug. Retry unscaled; the
        # backends are then built the same way so every target sees the same
        # program.
        refs: dict[str, dict[str, tuple[int, str]]] = {}
        ref_bins: list[Path] = []
        chosen: list[str] | None = None
        last_err = ""
        for cand in (defines, []):
            cand_refs: dict[str, dict[str, tuple[int, str]]] = {}
            cand_bins: list[Path] = []
            ok_all = True
            for ref in references:
                for name, _arch, _sys, _bin, refflags in usable:
                    model = "ilp32" if "-m32" in refflags else "lp64"
                    slot = cand_refs.setdefault(model, {})
                    if ref in slot:
                        continue
                    ref_bin = tmp / f"{src.stem}.{os.path.basename(ref)}.{model}{opt}"
                    ok, err = compile_with([ref] + refflags, src, ref_bin, cand,
                                           flags, timeout)
                    if not ok:
                        ok_all = False
                        last_err = err
                        break
                    slot[ref] = run_binary(ref_bin, "x86_64", None, timeout)
                    cand_bins.append(ref_bin)
                if not ok_all:
                    break
            if ok_all:
                chosen = cand
                refs = cand_refs
                ref_bins = cand_bins
                break

        if chosen is None:
            failures.append(f"{src.name}{opt}: reference compile failed: {last_err[:120]}")
            entry["error"] = last_err
            return entry, failures, timeouts
        entry["scaled"] = chosen == gen_defines

        # References must agree with each other, otherwise "the" expected
        # output is not well defined (excess precision, libm, …) and any
        # comparison would be noise.
        for model, per_ref in refs.items():
            distinct = {out for (_rc, out) in per_ref.values()}
            if len(distinct) > 1:
                entry["note"] = f"reference disagreement ({model}); not comparable"
                for rb in ref_bins:
                    rb.unlink(missing_ok=True)
                return entry, failures, timeouts

        # --- backends ------------------------------------------------------
        for name, arch, sysroot, binary, refflags in usable:
            model = "ilp32" if "-m32" in refflags else "lp64"
            expected = refs.get(model, {})
            if not expected:
                continue
            exp_rc, exp_out = next(iter(expected.values()))
            bin_path = tmp / f"{src.stem}.{name}{opt}"
            ok, err = compile_with([str(binary)], src, bin_path, chosen, flags, timeout)
            if not ok:
                row.append("COMPILEFAIL")
                entry["backends"][name] = {"compile_ok": False, "error": err[:300]}
                failures.append(f"{src.name} {opt} {name}: compile failed: {err[:160]}")
                continue

            # Repeat to expose nondeterminism (uninitialised reads, ASLR
            # dependence, timing): every run must be identical AND correct.
            results = [run_binary(bin_path, arch, sysroot, timeout) for _ in range(repeat)]
            bin_path.unlink(missing_ok=True)
            if any(r != results[0] for r in results[1:]):
                row.append("NONDET")
                entry["backends"][name] = {"compile_ok": True, "nondeterministic": True}
                failures.append(f"{src.name} {opt} {name}: nondeterministic output "
                                f"across {repeat} runs")
                continue

            rc, out = results[0]
            if rc == -999:
                row.append("TIMEOUT")
                entry["backends"][name] = {"compile_ok": True, "timeout": True}
                timeouts.append(f"{src.name} {opt} {name}")
                continue
            if rc == -998:
                row.append("EXECFAIL")
                entry["backends"][name] = {"compile_ok": True, "exec_error": out[:200]}
                failures.append(f"{src.name} {opt} {name}: could not execute: {out[:120]}")
                continue

            same = all(rc == r and out == o for (r, o) in expected.values())
            rec: dict[str, object] = {"compile_ok": True, "exit": rc, "matches": same}
            if same:
                row.append("ok")
            else:
                row.append("MISMATCH")
                rec["expected_exit"] = exp_rc
                rec["first_difference"] = short_diff(exp_out, out)
                rec["actual_head"] = out[:200]
                rec["expected_head"] = exp_out[:200]
                failures.append(
                    f"{src.name} {opt} {name}: {rec['first_difference'] or f'exit {rc} != {exp_rc}'}")
            entry["backends"][name] = rec

        for rb in ref_bins:
            rb.unlink(missing_ok=True)

    entry["row"] = row
    return entry, failures, timeouts


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--corpus", type=Path, default=CORPUS,
                    help="directory of C programs (default: tests/benchmark/programs)")
    ap.add_argument("--programs", default="", help="comma-separated subset of program stems")
    ap.add_argument("--backends", default=",".join(BACKENDS), help="comma-separated backend names")
    ap.add_argument("--opts", default="-O0,-O1,-O2,-O3,-Os", help="comma-separated opt levels")
    ap.add_argument("--reference", action="append", default=None,
                    help="reference compiler (repeatable). Default: every one of "
                         "gcc/clang that is installed; all must agree.")
    ap.add_argument("--no-scale", action="store_true", help="do not shrink the workload macros")
    ap.add_argument("--define", action="append", default=[], metavar="NAME=VALUE",
                    help="extra/override -D define")
    ap.add_argument("--lccc-dir", type=Path, default=REPO_ROOT / "target" / "fastbuild")
    ap.add_argument("--scratch", type=Path, default=REPO_ROOT / "target" / "crossbe-scratch",
                    help="disk-backed scratch directory. MUST NOT be /tmp: that is a "
                         "~1 GB tmpfs here and a full run's static binaries exhaust it "
                         "('final link failed: No space left on device').")
    ap.add_argument("--timeout", type=int, default=120, help="per-run timeout (s)")
    ap.add_argument("--repeat", type=int, default=1,
                    help="run each backend binary N times to catch nondeterminism")
    ap.add_argument("--jobs", type=int, default=2, help="parallel (program, opt) tasks")
    ap.add_argument("--json", type=Path, help="write the full report")
    ap.add_argument("--compare", type=Path,
                    help="previous --json report; report only NEW failures and now-fixed entries")
    ap.add_argument("--quick", action="store_true",
                    help="CI-friendly subset: -O0,-O2,-Os (implies nothing else)")
    ap.add_argument("--gate", action="store_true",
                    help="exit non-zero on any mismatch/compile failure (and on timeouts "
                         "with --strict-timeout)")
    ap.add_argument("--strict-timeout", action="store_true", help="treat timeouts as failures")
    ap.add_argument("--verbose", "-v", action="store_true")
    args = ap.parse_args(argv)

    if not args.corpus.is_dir():
        print(f"FATAL: corpus {args.corpus} is not a directory", file=sys.stderr)
        return 2

    opts = "-O0,-O2,-Os" if args.quick else args.opts

    references = args.reference or [r for r in ("gcc", "clang") if shutil.which(r)]
    if not references:
        print("FATAL: no reference compiler found (need gcc or clang)", file=sys.stderr)
        return 2

    programs = sorted(args.corpus.glob("*.c"))
    if args.programs:
        wanted = {p.strip() for p in args.programs.split(",") if p.strip()}
        programs = [p for p in programs if p.stem in wanted]

    defines: list[str] = []
    if not args.no_scale:
        defines += [f"-D{k}={v}" for k, v in DEFAULT_SCALE.items()]
    defines += [f"-D{d}" for d in args.define]

    backends: dict[str, tuple[str, str | None, list[str]]] = {}
    for name in (b.strip() for b in args.backends.split(",") if b.strip()):
        if name not in BACKENDS:
            print(f"FATAL: unknown backend {name!r}", file=sys.stderr)
            return 2
        backends[name] = BACKENDS[name]

    usable: list[tuple[str, str, str | None, Path, list[str]]] = []
    for name, (arch, sysroot, refflags) in backends.items():
        usable.append((name, arch, sysroot, args.lccc_dir / name, refflags))

    problems = preflight(references, usable, True)
    if problems:
        print("FATAL: environment problems (fix these before trusting any result):")
        for p in problems:
            print(f"  - {p}")
        return 2

    args.scratch.mkdir(parents=True, exist_ok=True)

    tasks = [(p, o) for p in programs if p.name not in SKIP
             for o in (x.strip() for x in opts.split(",") if x.strip())]

    names = [n for n, *_ in usable]
    print(f"{'program':<28} {'opt':<4} " + " ".join(f"{n:<11}" for n in names))
    print("-" * (34 + 12 * len(names)))

    results: list[tuple[dict, list[str], list[str]]] = []
    if args.jobs > 1 and len(tasks) > 1:
        with ThreadPoolExecutor(max_workers=args.jobs) as pool:
            futs = [pool.submit(evaluate_program, src, opt, usable, references, defines,
                                defines, [opt, "-static"], args.scratch, args.timeout,
                                args.repeat)
                    for src, opt in tasks]
            for (src, opt), fut in zip(tasks, futs):
                res = fut.result()
                results.append(res)
                entry = res[0]
                row = entry.get("row", [])
                note = f"  [{entry['note']}]" if entry.get("note") else ""
                print(f"{src.name:<28} {opt:<4} " +
                      " ".join(f"{c:<11}" for c in row) + note)
    else:
        for src, opt in tasks:
            res = evaluate_program(src, opt, usable, references, defines, defines,
                                   [opt, "-static"], args.scratch, args.timeout, args.repeat)
            results.append(res)
            entry = res[0]
            note = f"  [{entry['note']}]" if entry.get("note") else ""
            print(f"{src.name:<28} {opt:<4} " +
                  " ".join(f"{c:<11}" for c in entry.get("row", [])) + note)

    failures: list[str] = []
    timeouts: list[str] = []
    report: dict[str, object] = {
        "tool": "cross_backend_check.py",
        "references": references,
        "opts": opts,
        "scale": {} if args.no_scale else DEFAULT_SCALE,
        "results": [],
    }
    for entry, f, t in results:
        row = entry.pop("row", None)
        report["results"].append(entry)  # type: ignore[union-attr]
        failures += f
        timeouts += t

    if args.json:
        args.json.parent.mkdir(parents=True, exist_ok=True)
        args.json.write_text(json.dumps(report, indent=2))
        print(f"\nwrote {args.json}")

    if args.compare and args.compare.exists():
        base = json.loads(args.compare.read_text())
        def key(e): return f"{e['program']}-{e['opt']}"
        def bad(e): return {n for n, r in (e.get("backends") or {}).items()
                            if not r.get("matches", True)}
        old = {key(e): bad(e) for e in base.get("results", [])}
        new = {key(e): bad(e) for e in report["results"]}  # type: ignore[union-attr]
        regressed = {k: (old.get(k, set()), v) for k, v in new.items() if v - old.get(k, set())}
        fixed = {k: (old[k], new.get(k, set())) for k in old if old[k] and not new.get(k)}
        print(f"\n--compare: {len(regressed)} regression(s), {len(fixed)} fixed")
        for k, (o, n) in sorted(regressed.items()):
            print(f"  REGRESSED {k}: {sorted(n - o)}")
        for k, (o, n) in sorted(fixed.items()):
            print(f"  FIXED     {k}: was {sorted(o)}")

    if timeouts:
        print(f"\n{len(timeouts)} TIMEOUT(S) (speed, not correctness — raise --timeout or "
              f"shrink the program's scale macros):")
        for t in timeouts[:20]:
            print(f"  - {t}")
        if len(timeouts) > 20:
            print(f"  … and {len(timeouts) - 20} more")

    fatal = list(failures)
    if args.strict_timeout:
        fatal += [f"timeout: {t}" for t in timeouts]

    if fatal:
        print(f"\n{len(fatal)} FAILURE(S):")
        for f in fatal[:40]:
            print(f"  - {f}")
        if len(fatal) > 40:
            print(f"  … and {len(fatal) - 40} more")
        return 1
    print("\nALL BACKENDS AGREE WITH THE REFERENCE COMPILERS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
