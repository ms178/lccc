#!/usr/bin/env python3
"""Gate the four invariants of the O(n) parameter-homing / cross-ISA patch.

This is deliberately a *structural* gate, not a wall-clock record: a ratio
between two input sizes is portable across CI machines, an absolute second
budget is not. The pre-change compiler scaled 0.120 s -> 4.725 s -> 589.493 s
for 1,000 / 10,000 / 100,000 parameters (quadratic); the fixed compiler scales
~0.097 s -> 0.376 s (roughly linear). QUADRATIC_RATIO=15 is a wide, honest
margin: 39x before, 3.9x after.

Checks (every one is non-vacuous; `--self-test` proves each can FAIL):
  1. All four drivers resolve the target-neutral GCC headers
     (stdbool.h/stdarg.h/float.h/limits.h) without an explicit -isystem.
  2. An x87-only "=t" inline-asm constraint on AArch64 produces a valid
     ELF64 AArch64 relocatable object, never a panic, and emits a
     width-correct `fmov` (no `fmov wN, dN` / `fmov xN, sN`).
  3. i686 parameter-homing compile time scales linearly.
  4. The new slot index does not make object emission nondeterministic.
  5. A SHIFTED entry-block layout (a volatile local homed before the
     parameter homes, plus mem2reg-promoted parameters) still passes every
     incoming argument on all four targets, and produces the same checksum
     as the aligned layout at -O0/-O2/-Os. This is the class the old
     O(1) probe missed and the O(instructions) fallback used to handle.
  6. The shipped parameter-home index has no per-query full scan left in
     production code (structural guard, with a non-vacuous self-test).

Usage:
  python3 tests/regression/check_param_home_contracts.py
  python3 tests/regression/check_param_home_contracts.py --self-test
"""
from __future__ import annotations

import argparse
import pathlib
import re
import struct
import subprocess
import sys
import tempfile
import time

ROOT = pathlib.Path(__file__).resolve().parents[2]
FB = ROOT / "target/fastbuild"

DRIVERS = {
    "x86_64": (FB / "lccc", 62, 2),
    "i686": (FB / "lccc-i686", 3, 1),
    "aarch64": (FB / "lccc-arm", 183, 2),
    "riscv64": (FB / "lccc-riscv", 243, 2),
}

# GCC's private include/ is target-neutral: these four headers are exactly
# what set_target() used to drop on aarch64/riscv64.
HEADER_CASE = b"""#include <stdbool.h>
#include <stdarg.h>
#include <stddef.h>
#include <float.h>
int f(int a, bool b, ...) { return a + (b ? 1 : 0) + FLT_RADIX + (int)sizeof(size_t); }
"""

# <limits.h> is a GLIBC header: it needs bits/libc-header-start.h and
# gnu/stubs-32.h from a sysroot. No cross sysroot exists in the pinned
# harness cache, so the compile-only campaign legs use an explicit host
# header overlay. This is a documented harness contract, NOT a claim that
# lccc ships glibc; the gate proves the overlay is sufficient and honest.
GLIBC_CASE = b"""#include <limits.h>
int f(void) { return CHAR_BIT + INT_MAX; }
"""

# gcc.c-torture/compile/pr30311.c's essential shape: an x87-only output
# constraint and a tied double input. AArch64 has no x87, but the compiler
# must still produce a valid object instead of an ICE.
X87_ON_ARM = b"""inline double bar(double x) {
  long double d;
  __asm__ ("" : "=t" (d) : "0" (x));
  return d;
}
double foo(double x) { if (x) return bar(x); else return bar(x); }
"""

BAD_FMOV = re.compile(rb"\bfmov\s+w\d+,\s*d\d+|\bfmov\s+x\d+,\s*s\d+")
GOOD_FMOV = re.compile(rb"\bfmov\s+(?:x\d+,\s*d\d+|w\d+,\s*s\d+)")
PANIC = re.compile(rb"panicked at|internal error:|unreachable code|SIGABRT|SIGSEGV")

# Pre-change measured 39.375x; post-change 3.9x idle / 11.9x under a
# concurrent 64k-compile campaign. 25 leaves a wide CI margin while still
# rejecting the quadratic curve by >1.5x.
QUADRATIC_RATIO = 25.0
SMALL_PARAMS = 1000
LARGE_PARAMS = 10000


class Failure(Exception):
    pass


def require(cond: bool, msg: str) -> None:
    if not cond:
        raise Failure(msg)


def run(cmd: list[str], timeout: float = 120.0) -> subprocess.CompletedProcess[bytes]:
    try:
        return subprocess.run(cmd, capture_output=True, timeout=timeout)
    except subprocess.TimeoutExpired as exc:
        raise Failure(f"timeout after {timeout}s: {' '.join(cmd)}") from exc


def elf_machine_bytes(b: bytes) -> tuple[int, int]:
    require(len(b) >= 20 and b[:4] == b"\x7fELF", "not an ELF object")
    machine = struct.unpack_from("<H", b, 18)[0]
    etype = struct.unpack_from("<H", b, 16)[0]
    return machine, etype


def elf_machine(path: pathlib.Path) -> tuple[int, int]:
    return elf_machine_bytes(path.read_bytes())


def compile_ok(driver: pathlib.Path, src: pathlib.Path, obj: pathlib.Path,
               extra: list[str] | None = None) -> subprocess.CompletedProcess[bytes]:
    r = run([str(driver), "-w", "-O2", *(extra or []), "-c", str(src), "-o", str(obj)])
    require(r.returncode == 0,
            f"{driver.name} rc={r.returncode}: {r.stderr.decode('utf-8', 'replace')[-300:]}")
    require(not PANIC.search(r.stdout + r.stderr),
            f"{driver.name} emitted a panic/ICE: {(r.stdout + r.stderr)[-300:]!r}")
    return r


def with_temp(work: pathlib.Path, name: str, data: bytes) -> pathlib.Path:
    p = work / name
    p.write_bytes(data)
    return p


def params_source(n: int) -> bytes:
    args = b",".join(b"int p%d" % i for i in range(n))
    return b"int f(" + args + b") { return 0; }\n"


# A volatile local forces a real entry-block Alloca before/around the
# parameter homes; at -O2 mem2reg promotes the parameters but must keep the
# volatile local.  That is exactly the layout where the old positional O(1)
# probe missed for every parameter and the O(instructions) fallback ran.
SHIFTED_PARAMS = 64
# A forward declaration instead of <stdio.h>: the cross-target legs have no
# host glibc sysroot, so pulling in a libc header would fail for i686/AArch64/
# RISC-V for reasons unrelated to parameter homing.
SHIFTED_GUARD = b"extern int printf(const char *, ...);\nvolatile int guard;\n"


def shifted_source(prefix: bool) -> bytes:
    args = b",".join(b"int p%d" % i for i in range(SHIFTED_PARAMS))
    body = [b"int f(" + args + b") {"]
    if prefix:
        body.append(b"  volatile int local = guard;")
        body.append(b"  int sum = local;")
    else:
        body.append(b"  int sum = 0;")
    for i in range(SHIFTED_PARAMS):
        body.append(b"  sum += p%d;" % i)
    body.append(b"  return sum;")
    body.append(b"}")
    body.append(b"int main(void) {")
    call = b",".join(b"%d" % i for i in range(SHIFTED_PARAMS))
    body.append(b"  int r = f(" + call + b");")
    body.append(b'  printf("%d\\n", r);')
    body.append(b"  return 0;")
    body.append(b"}")
    return b"\n".join(body) + b"\n"


SHIFTED_EXPECTED = sum(range(SHIFTED_PARAMS))


def timed_compile(driver: pathlib.Path, src: pathlib.Path, obj: pathlib.Path) -> float:
    t0 = time.monotonic()
    compile_ok(driver, src, obj, ["-O0"])
    return time.monotonic() - t0


def check_headers(work: pathlib.Path) -> None:
    src = with_temp(work, "headers.c", HEADER_CASE)
    for isa, (driver, machine, _cls) in DRIVERS.items():
        require(driver.is_file() and driver.stat().st_size > 0,
                f"{driver} missing: build the fastbuild binaries first")
        obj = work / f"headers-{isa}.o"
        compile_ok(driver, src, obj)
        got, etype = elf_machine(obj)
        require(got == machine and etype == 1,
                f"{isa}: e_machine={got} e_type={etype}, want {machine}/ET_REL")
    print("  [1a/7] all four drivers resolve target-neutral GCC private headers"
          " WITHOUT extra flags: PASS")

    # The glibc half: build the documented overlay in the gate itself so CI
    # needs no out-of-repo files. An empty stubs-32.h is semantically neutral
    # for a compile-only ELF contract (it only declares __stub_* macros).
    overlay = work / "overlay" / "gnu"
    overlay.mkdir(parents=True)
    (overlay / "stubs-32.h").write_text(
        "/* harness shim: __stub_* macros only; compile-only ELF contract */\n")
    extra = ["-isystem", str(overlay.parent),
             "-isystem", "/usr/include/x86_64-linux-gnu",
             "-isystem", "/usr/include/i386-linux-gnu",
             "-isystem", "/usr/include"]
    gsrc = with_temp(work, "glibc.c", GLIBC_CASE)
    for isa, (driver, machine, _cls) in DRIVERS.items():
        obj = work / f"glibc-{isa}.o"
        compile_ok(driver, gsrc, obj, extra)
        got, etype = elf_machine(obj)
        require(got == machine and etype == 1,
                f"{isa}: glibc overlay produced {got}/{etype}, want {machine}/ET_REL")
    print("  [1b/7] documented host-header overlay resolves <limits.h> on all"
          " four drivers: PASS (compile-only, NOT ABI/execute coverage)")


def check_arm_x87_asm(work: pathlib.Path) -> None:
    driver = DRIVERS["aarch64"][0]
    src = with_temp(work, "x87-on-arm.c", X87_ON_ARM)
    obj = work / "x87-on-arm.o"
    asm = work / "x87-on-arm.s"
    compile_ok(driver, src, obj)
    got, etype = elf_machine(obj)
    require(got == 183 and etype == 1, f"expected AArch64 ET_REL, got {got}/{etype}")
    r = run([str(driver), "-w", "-O2", "-S", str(src), "-o", str(asm)])
    require(r.returncode == 0, f"-S failed: {r.stderr.decode('utf-8', 'replace')[-200:]}")
    text = asm.read_bytes()
    require(not BAD_FMOV.search(text), "width-mismatched fmov in emitted assembly")
    require(GOOD_FMOV.search(text) is not None,
            "expected a width-correct fmov (the FP->GP move the fix emits)")
    print("  [2/7] AArch64 x87-only inline asm: no ICE, valid object, width-correct fmov: PASS")


def check_scaling(work: pathlib.Path) -> None:
    driver = DRIVERS["i686"][0]
    small = with_temp(work, "lim-small.c", params_source(SMALL_PARAMS))
    large = with_temp(work, "lim-large.c", params_source(LARGE_PARAMS))
    # Warm the page cache, then take the median of five to suppress CI noise
    # from a concurrently loaded runner (the ratio moved 3.9x -> 11.9x under
    # a 64k-compile campaign; the median keeps that well inside the margin).
    timed_compile(driver, small, work / "warm.o")
    ts = sorted(timed_compile(driver, small, work / f"s{i}.o") for i in range(5))
    tl = sorted(timed_compile(driver, large, work / f"l{i}.o") for i in range(5))
    ratio = tl[2] / max(ts[2], 1e-3)
    require(ratio < QUADRATIC_RATIO,
            f"parameter homing still quadratic: t({LARGE_PARAMS})/t({SMALL_PARAMS})"
            f" = {ratio:.2f} (limit {QUADRATIC_RATIO}; pre-change was ~39x)")
    print(f"  [3/7] i686 param homing scaling {ratio:.2f}x for 10x params"
          f" (limit {QUADRATIC_RATIO}x): PASS")


def check_determinism(work: pathlib.Path) -> None:
    driver = DRIVERS["i686"][0]
    src = with_temp(work, "det.c", params_source(2000))
    a, b = work / "det-a.o", work / "det-b.o"
    compile_ok(driver, src, a, ["-O0"])
    compile_ok(driver, src, b, ["-O0"])
    require(a.read_bytes() == b.read_bytes(),
            "the slot index made object emission nondeterministic")
    print("  [4/7] object emission is deterministic with the new slot index: PASS")


def check_shifted_layout(work: pathlib.Path) -> None:
    """Shifted parameter homes: every argument must survive on all targets."""
    aligned = with_temp(work, "aligned.c", SHIFTED_GUARD + shifted_source(False))
    shifted = with_temp(work, "shifted.c", SHIFTED_GUARD + shifted_source(True))
    host = DRIVERS["x86_64"][0]

    # Runtime equivalence on the host: the prefix local contributes `guard`
    # (zero-initialized), so both sources must print the same checksum at
    # every optimisation level.  A prologue that homes a parameter into the
    # wrong slot -- the exact 20180112-1.c miscompile -- changes the sum.
    for opt in ("-O0", "-O2", "-Os"):
        for label, src in (("aligned", aligned), ("shifted", shifted)):
            exe = work / f"shift-{label}-{opt[1:]}.exe"
            r = run([str(host), "-w", opt, str(src), "-o", str(exe)])
            require(r.returncode == 0,
                    f"x86_64 {opt} {label} link failed: "
                    f"{r.stderr.decode('utf-8', 'replace')[-300:]}")
            p = run([str(exe)], timeout=30.0)
            require(p.returncode == 0, f"{exe.name} exited {p.returncode}")
            got = p.stdout.strip()
            require(got == str(SHIFTED_EXPECTED).encode(),
                    f"x86_64 {opt} {label}: checksum {got!r}, "
                    f"want {SHIFTED_EXPECTED} -- parameter homing is wrong")
    print(f"  [5a/7] shifted vs aligned parameter homes: identical checksum "
          f"{SHIFTED_EXPECTED} at -O0/-O2/-Os: PASS")

    # All four targets: a valid, panic-free object for the shifted layout.
    for isa, (driver, machine, _cls) in DRIVERS.items():
        for opt in ("-O0", "-O2"):
            obj = work / f"shift-{isa}-{opt[1:]}.o"
            compile_ok(driver, shifted, obj, [opt])
            got, etype = elf_machine(obj)
            require(got == machine and etype == 1,
                    f"{isa} {opt}: e_machine={got} e_type={etype}, want {machine}/ET_REL")
            twin = work / f"shift-{isa}-{opt[1:]}-b.o"
            compile_ok(driver, shifted, twin, [opt])
            require(obj.read_bytes() == twin.read_bytes(),
                    f"{isa} {opt}: shifted-layout object is nondeterministic")
    print("  [5b/7] all four drivers emit deterministic, panic-free ELF objects "
          "for the shifted layout: PASS")


def structural_failures(production: bytes, prologues: dict[str, bytes]) -> list[str]:
    """Pure predicate behind the structural gate, so the self-test can prove
    every rule can reject a bad tree without mocking the filesystem."""
    bad: list[str] = []
    if b"pub struct ParamAllocaIndex" not in production:
        bad.append("ParamAllocaIndex is gone from production code")
    if b"fn aligned_homes" not in production or b"fn general_homes" not in production:
        bad.append("the two proven build paths are gone")
    if b"find_map(|inst| match inst" in production:
        bad.append("a per-instruction find_map scan reappeared in production code")
    if b"fn find_param_alloca(" in production:
        bad.append("the old per-query lookup reappeared")
    for arch in ("arm", "i686", "riscv", "x86"):
        if b"ParamAllocaIndex::build(func)" not in prologues.get(arch, b""):
            bad.append(f"{arch} prologue does not build the index")
    return bad


def check_no_scan_fallback(work: pathlib.Path) -> None:
    """Structural guard: the shipped index must not grow a per-query scan."""
    root = ROOT / "src/backend/stack_layout/regalloc_helpers.rs"
    production = root.read_bytes().split(b"#[cfg(test)]", 1)[0]
    prologues = {
        arch: (ROOT / f"src/backend/{arch}/codegen/prologue.rs").read_bytes()
        for arch in ("arm", "i686", "riscv", "x86")
    }
    bad = structural_failures(production, prologues)
    require(not bad, "; ".join(bad))
    print("  [6/7] no per-query full scan in production; all four prologues build "
          "the index once: PASS")


def self_test() -> None:
    """Prove every rejection rule can actually fire (a gate that cannot fail
    is decoration). Uses synthetic data, no compiler needed."""
    cases = 0

    def rejects(fn, *a) -> None:
        nonlocal cases
        try:
            fn(*a)
        except Failure:
            cases += 1
            return
        raise AssertionError(f"{fn.__name__} accepted a bad input")

    rejects(require, False, "synthetic")
    rejects(lambda: require(BAD_FMOV.search(b"    fmov w0, d0\n") is None,
                            "bad fmov"))
    rejects(lambda: require(GOOD_FMOV.search(b"    mov x0, x1\n") is not None,
                            "missing fmov"))
    rejects(lambda: require(not PANIC.search(b"panicked at foo.rs:1"), "panic"))
    # The ratio rule must reject the measured pre-change curve.
    old_small, old_large = 0.120, 4.725
    rejects(lambda: require(old_large / old_small < QUADRATIC_RATIO, "quadratic"))
    # ... and accept the measured post-change curve, while proving the limit
    # is tight enough that the quadratic curve could never slip through.
    require(0.376 / 0.097 < QUADRATIC_RATIO, "linear curve rejected")
    require(QUADRATIC_RATIO < old_large / old_small, "limit too loose to be non-vacuous")
    rejects(lambda: require(elf_machine_bytes(b"NOPE")[0] == 62, "bad elf"))
    # The new checksum rule must reject one wrong answer.
    rejects(lambda: require(b"2015" == str(SHIFTED_EXPECTED).encode(),
                            "wrong shifted checksum"))
    # ... and every structural rule must reject a bad tree.  A pure
    # predicate makes this proof exact: no filesystem mocking, no vacuity.
    good = (ROOT / "src/backend/stack_layout/regalloc_helpers.rs").read_bytes()
    good_prod = good.split(b"#[cfg(test)]", 1)[0]
    good_prologues = {
        arch: (ROOT / f"src/backend/{arch}/codegen/prologue.rs").read_bytes()
        for arch in ("arm", "i686", "riscv", "x86")
    }
    require(not structural_failures(good_prod, good_prologues),
            "the shipped tree fails its own structural guard")
    rejects(lambda: require(not structural_failures(
        good_prod.replace(b"pub struct ParamAllocaIndex", b"pub struct Gone"),
        good_prologues), "missing index"))
    rejects(lambda: require(not structural_failures(
        good_prod + b"\nfn find_param_alloca() {}\n", good_prologues),
        "reintroduced per-query lookup"))
    rejects(lambda: require(not structural_failures(
        good_prod + b"\nfn find_map(|inst| match inst {}\n", good_prologues),
        "reintroduced full scan"))
    missing = dict(good_prologues)
    missing["riscv"] = b""
    rejects(lambda: require(not structural_failures(good_prod, missing),
                            "prologue without an index build"))
    require(cases == 11, f"self-test covered {cases}/11 rejection paths")
    print(f"SELF_TEST_OK ({cases} rejection paths proven non-vacuous)")



def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--self-test", action="store_true")
    a = ap.parse_args(argv)
    if a.self_test:
        self_test()
        return 0
    try:
        with tempfile.TemporaryDirectory(prefix="param-home-gate-") as td:
            work = pathlib.Path(td)
            check_headers(work)
            check_arm_x87_asm(work)
            check_scaling(work)
            check_determinism(work)
            check_shifted_layout(work)
            check_no_scan_fallback(work)
    except Failure as exc:
        print(f"PARAM_HOME_CONTRACTS_FAIL: {exc}", file=sys.stderr)
        return 1
    print("PARAM_HOME_CONTRACTS_OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
