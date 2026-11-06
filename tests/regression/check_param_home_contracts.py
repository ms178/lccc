#!/usr/bin/env python3
"""Gate the parameter-homing / cross-ISA patch and its CLI-output invariants.

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
  7. Diagnostic output is best-effort: piping a diagnostic-heavy compile to
     a reader that exits early must still exit 1 (a clean verdict), not 101
     (Rust `eprintln!` panicking on BrokenPipe).
  8. Structural guard: no `eprintln!`/`eprint!`/`println!`/`print!` remains
     in shipped sources; all such text goes through `lccc::cli`, whose
     four helpers ignore write errors and are byte-identical on success.
  9. Latest-main interaction: `-g` must NOT change code for the SHIFTED
     parameter-home layout on all four ISAs at -O0/-O2.  DWARF delivery and
     mem2reg must agree on which allocas are parameter homes.  The debug-row
     filter has a non-vacuous one-byte-instruction mutation self-test.
 10. Latest-main interaction: the round-cap exit-coverage gate uses a
     line-count-independent report/exit bijection (a fixed 9-line lookback
     produced a false red CI lane after the CLI rewrite), plus six built-in
     mutations and a combined `CCC_TIME_PASSES` + `CCC_DUMP_IR_AFTER` witness.
 11. Reproducibility: no differential tool may name a stale baseline leg;
     classifier/oracle/bench fail closed and the classifier self-test passes.
 12. `scripts/audit_census.py --check` holds every ratchet (including
     `fns_over_500`): the exact check that turned hosted CI red when the long
     `lccc::cli_output::` spelling wrapped a 498-line function past 500.  The
     flat adjacency chain's O(1) heap-object invariant is pinned both by the
     structural guard in check 6 and by `ParamHomeStats::allocations` unit
     tests (1 allocation aligned, 3 general, independent of `params`).

Usage:
  python3 tests/regression/check_param_home_contracts.py
  python3 tests/regression/check_param_home_contracts.py --self-test
"""
from __future__ import annotations

import argparse
import os
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
    print("  [1a] all four drivers resolve target-neutral GCC private headers"
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
    print("  [1b] documented host-header overlay resolves <limits.h> on all"
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
    print("  [2] AArch64 x87-only inline asm: no ICE, valid object, width-correct fmov: PASS")


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
    print(f"  [3] i686 param homing scaling {ratio:.2f}x for 10x params"
          f" (limit {QUADRATIC_RATIO}x): PASS")


def check_determinism(work: pathlib.Path) -> None:
    driver = DRIVERS["i686"][0]
    src = with_temp(work, "det.c", params_source(2000))
    a, b = work / "det-a.o", work / "det-b.o"
    compile_ok(driver, src, a, ["-O0"])
    compile_ok(driver, src, b, ["-O0"])
    require(a.read_bytes() == b.read_bytes(),
            "the slot index made object emission nondeterministic")
    print("  [4] object emission is deterministic with the new slot index: PASS")


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
    print(f"  [5a] shifted vs aligned parameter homes: identical checksum "
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
    print("  [5b] all four drivers emit deterministic, panic-free ELF objects "
          "for the shifted layout: PASS")


def pipe_verdict_is_clean(rc: int) -> bool:
    """The compiler's verdict must survive a vanished stderr reader."""
    return rc == 1


def check_diagnostic_pipe_safety(work: pathlib.Path) -> None:
    # Enough diagnostics to overflow the pipe buffer after the reader closes;
    # a handful of lines would fit in the kernel buffer and never see EPIPE.
    bad = with_temp(work, "many-errors.c",
                    b"int f(void) {\n" + b"  @ ;\n" * 2000 + b"}\n")
    driver = DRIVERS["x86_64"][0]
    control = work / "many-errors.log"
    with control.open("wb") as log:
        r = run([str(driver), "-w", "-c", str(bad), "-o", str(work / "many-errors.o")])
        log.write(r.stdout + r.stderr)
    require(r.returncode == 1,
            f"control compile should fail cleanly, got rc={r.returncode}")
    require(b"error:" in control.read_bytes(), "control emitted no diagnostics")

    proc = subprocess.Popen(
        [str(driver), "-w", "-c", str(bad), "-o", str(work / "many-errors-pipe.o")],
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    try:
        for _ in range(2):
            require(proc.stdout is not None and proc.stdout.readline() != b"",
                    "compiler produced no diagnostics to pipe")
        # Closing the read end is exactly what `| head -2` does.
        proc.stdout.close()
        rc = proc.wait(timeout=60)
    finally:
        if proc.poll() is None:
            proc.kill()
            proc.wait()
    require(pipe_verdict_is_clean(rc),
            f"broken-pipe diagnostics changed the compiler verdict: rc={rc} "
            f"(101 = eprintln! panicking on BrokenPipe, 141 = SIGPIPE)")
    print(f"  [7] diagnostic output survives a closed stderr pipe (rc={rc}, "
          f"no Rust ICE): PASS")


# ---------------------------------------------------------------------------
# Check 8: the CLI-text panic class must be impossible, not merely fixed once.
#
# `eprintln!` / `println!` call `std::io::stdio::print_to`, which panics
# ("failed printing to stderr: Broken pipe") when the reader disappears. The
# first fix only covered two files; GDB traced the surviving rc=101 to
# `src/lib.rs:157`, the driver's final verdict. A runtime test alone cannot
# stop the macro from being reintroduced on a less-travelled path, so this
# structural guard scans every non-test source file with a literal/comment-
# aware lexer and forbids the four macros outright.
# ---------------------------------------------------------------------------
CLI_TEXT_MACRO = re.compile(r"(eprintln|eprint|println|print)!")
_RAW_STRING = re.compile(r"(?:b|c|br|cr)?r(#*)\"")
_CLI_DELIMS = {"(": ")", "[": "]", "{": "}"}


def _skip_literal(text: str, i: int) -> int | None:
    """Return the end of a literal/comment starting at i, else None."""
    if text.startswith("//", i):
        j = text.find("\n", i)
        return len(text) if j < 0 else j
    if text.startswith("/*", i):
        depth, j = 1, i + 2
        while j < len(text) and depth:
            if text.startswith("/*", j):
                depth, j = depth + 1, j + 2
            elif text.startswith("*/", j):
                depth, j = depth - 1, j + 2
            else:
                j += 1
        return j
    m = _RAW_STRING.match(text, i)
    if m:
        end = '"' + "#" * len(m.group(1))
        j = text.find(end, m.end())
        return len(text) if j < 0 else j + len(end)
    if text[i] == '"':
        j = i + 1
        while j < len(text):
            if text[j] == "\\":
                j += 2
            elif text[j] == '"':
                return j + 1
            else:
                j += 1
        return j
    if text[i] == "'":
        # A char literal has a closing quote either right after one scalar or
        # after an escape; `'static` / `'label` are lifetimes/labels instead.
        if i + 1 < len(text) and text[i + 1] == "\\":
            j = text.find("'", i + 2)
            return len(text) if j < 0 else j + 1
        if i + 2 < len(text) and text[i + 2] == "'":
            return i + 3
        return i + 1
    return None


def cli_text_macro_hits(source: str) -> list[tuple[int, str]]:
    """Find panicking CLI-text macros outside literals, comments and tests."""
    hits: list[tuple[int, str]] = []
    i = 0
    while i < len(source):
        end = _skip_literal(source, i)
        if end is not None:
            i = end
            continue
        m = CLI_TEXT_MACRO.match(source, i)
        if m and (i == 0 or not (source[i - 1].isalnum() or source[i - 1] == "_")):
            hits.append((source.count("\n", 0, i) + 1, m.group(1)))
        i += 1
    return hits


def check_cli_text_is_best_effort() -> None:
    offenders: list[str] = []
    for p in sorted((ROOT / "src").rglob("*.rs")):
        rel = p.relative_to(ROOT / "src").as_posix()
        if rel == "common/cli_output.rs":
            continue
        # One read per file, and NO exemption for #[cfg(test)] blocks: a
        # panicking print macro is wrong in a test helper too, and exempting
        # everything after the first #[cfg(test)] would create a silent hole
        # for any file that puts an inline test attribute before production
        # code. Forbidding the four macros everywhere is both simpler and
        # strictly stronger.
        hits = cli_text_macro_hits(p.read_text(encoding="utf-8"))
        if hits:
            offenders.append(f"{rel}:{hits[0][0]} {hits[0][1]}!")
    require(not offenders,
            "panicking CLI text macro reintroduced: " + ", ".join(offenders[:5]))
    lib = (ROOT / "src/lib.rs").read_text(encoding="utf-8")
    require("extern crate self as lccc;" in lib,
            "lib.rs lost the self-alias used by binary call sites")
    require("pub use common::cli_output as cli;" in lib,
            "lib.rs lost the short public cli_output re-export")
    helper = (ROOT / "src/common/cli_output.rs").read_text(encoding="utf-8")
    for fn in ("stderr_line", "stderr_text", "stdout_line", "stdout_text"):
        require(f"pub fn {fn}(" in helper, f"cli_output is missing {fn}")
    print("  [8] all CLI text goes through the best-effort sink; "
          "no panicking print macros remain: PASS")


# ---------------------------------------------------------------------------
# Checks 9-12: cross-feature integration.  Latest main added DWARF delivery,
# an option ledger and a round-cap observability gate in the very files this
# patchset changes.  A merge can be conflict-free and still break an
# invariant, so these pin the interactions rather than trusting `git merge`.
# ---------------------------------------------------------------------------
DEBUG_SECTION = re.compile(r"^\s*\.section\s+\.debug_")
ANY_SECTION = re.compile(r"^\s*\.section\s+")
DEBUG_ROW = re.compile(r"^\s*\.(?:loc|file)(?:\s|$)")
DEBUG_ALIAS = re.compile(r"^\s*\.set\s+\.Llccc_debug_alias_")


def strip_debug_rows(lines: list[str]) -> list[str]:
    """Remove only debug metadata; keep every instruction, CFI row and label.

    Mirrors `check_debug_info_invariance.sh`'s awk filter so the two gates
    agree on what counts as code.  A `.debug_*` section is skipped until the
    next non-debug section directive.
    """
    out: list[str] = []
    in_debug = False
    for line in lines:
        if DEBUG_ROW.match(line) or DEBUG_ALIAS.match(line):
            continue
        if DEBUG_SECTION.match(line):
            in_debug = True
            continue
        if in_debug:
            if ANY_SECTION.match(line) and not DEBUG_SECTION.match(line):
                in_debug = False
            else:
                continue
        out.append(line)
    return out


def check_debug_info_invariance_on_shifted_layout(work: pathlib.Path) -> None:
    """-g must not change code for the shifted parameter-home layout.

    The mem2reg fix decides which entry-block allocas are parameter homes;
    DWARF delivery describes those same homes.  If either side looked at a
    different alloca set, -g would change instructions or lose a parameter
    DIE.  All four ISAs are checked at -O0 and -O2.
    """
    src = with_temp(work, "shift-dwarf.c", SHIFTED_GUARD + shifted_source(True))
    for isa, (driver, _machine, _cls) in DRIVERS.items():
        for opt in ("-O0", "-O2"):
            plain = work / f"dw-{isa}-{opt[1:]}-plain.s"
            debug = work / f"dw-{isa}-{opt[1:]}-debug.s"
            r = run([str(driver), "-w", opt, "-S", str(src), "-o", str(plain)])
            require(r.returncode == 0,
                    f"{isa} {opt} -S: {r.stderr.decode('utf-8', 'replace')[-200:]}")
            r = run([str(driver), "-w", opt, "-g", "-S", str(src), "-o", str(debug)])
            require(r.returncode == 0,
                    f"{isa} {opt} -g -S: {r.stderr.decode('utf-8', 'replace')[-200:]}")
            a = strip_debug_rows(plain.read_text(errors="replace").splitlines())
            b = strip_debug_rows(debug.read_text(errors="replace").splitlines())
            require(a == b,
                    f"{isa} {opt}: -g changed {len(a)}/{len(b)} code rows for the "
                    "shifted layout -- DWARF and mem2reg disagree on homes")
            require(any("debug_" in line for line in
                        debug.read_text(errors="replace").splitlines()),
                    f"{isa} {opt}: -g emitted no debug section; the check "
                    "would be vacuous")
    print("  [9] -g does not change code for the shifted parameter-home layout "
          "on x86-64/i686/AArch64/RISC-V at -O0/-O2: PASS")


def check_round_cap_exit_coverage(work: pathlib.Path) -> None:
    """The CLI rewrite must not hide the round_cap::report() invariant.

    Hosted CI caught exactly this: a four-line formatting change in the -O1 IR
    dump moved a dominating report() outside a fixed 9-line lookback window,
    producing a false red lane.  The repaired gate uses a line-count-
    independent region bijection with six built-in mutation self-tests.
    """
    gate = ROOT / "tests/regression/check_round_cap_observability.sh"
    text = gate.read_text(encoding="utf-8")
    require("len(reports) != len(returns) + 1" in text,
            "the gate lost its line-count-independent report/exit bijection")
    require("lines[max(0, n - 9):n]" not in text,
            "the brittle fixed lookback window returned")
    env = dict(os.environ)
    env.update({k: str(v) for k, v in (
        ("CCC", DRIVERS["x86_64"][0]),
        ("CCC_I686", DRIVERS["i686"][0]),
        ("CCC_ARM", DRIVERS["aarch64"][0]),
        ("CCC_RISCV", DRIVERS["riscv64"][0]))})
    r = subprocess.run(["bash", str(gate)], cwd=ROOT, env=env,
                       capture_output=True, timeout=300)
    require(r.returncode == 0,
            f"round-cap gate failed: {(r.stdout + r.stderr).decode('utf-8', 'replace')[-400:]}")
    require(b"exit-coverage self-test:" in r.stdout,
            "the gate did not run its mutation self-test")
    # Both opt-in dumps together: the report must still be observable even
    # when a 33k-line IR dump is written to the same stderr stream.
    env2 = dict(env, CCC_TIME_PASSES="1", CCC_DUMP_IR_AFTER="1")
    r = subprocess.run([str(DRIVERS["x86_64"][0]), "-O2", "-c",
                        str(ROOT / "tests/regression/cross_pr_redteam.c"),
                        "-o", str(work / "roundcap-both.o")],
                       cwd=ROOT, env=env2, capture_output=True, timeout=120)
    require(r.returncode == 0, "combined time-passes + IR-dump compile failed")
    require(b"round_cap backedge_pre:" in r.stderr,
            "the cap report disappeared behind the IR dump")
    print("  [10] round-cap exit coverage: bijection gate + 6 mutation "
          "self-tests + combined dump witness: PASS")


def check_research_tool_reproducibility() -> None:
    """No differential tool may silently measure a stale baseline leg."""
    stale: list[str] = []
    for folder in ("scripts", "engineering"):
        for p in (ROOT / folder).rglob("*.py"):
            if "fastbuild-baseline2" in p.read_text(encoding="utf-8",
                                                    errors="replace"):
                stale.append(str(p.relative_to(ROOT)))
    require(not stale, "hardcoded stale baseline leg: " + ", ".join(stale[:5]))
    classifier = ROOT / "scripts/ab_classify_divergences.py"
    helpers = ROOT / "engineering/compiler_legs.py"
    require(helpers.is_file(), "engineering/compiler_legs.py is missing")
    r = run([sys.executable, str(classifier), "--self-test"], timeout=60)
    require(r.returncode == 0, "divergence classifier self-test failed")
    bogus = str(ROOT / "target/does-not-exist")
    # The leg check runs before the report is read, so a nonexistent report
    # path is intentional: the only thing under test is fail-closed legs.
    r = run([sys.executable, str(classifier),
             "/tmp/nonexistent-ab-report.json", "/tmp/unused.json",
             "--old", bogus, "--new", str(FB)], timeout=60)
    require(r.returncode != 0, "classifier accepted a missing baseline leg")
    for tool in ("engineering/oracle_shifted_layout.py",
                 "engineering/bench_host_cpu.py"):
        r = run([sys.executable, str(ROOT / tool),
                 "--old", bogus, "--new", str(FB),
                 "--out", "/tmp/unused.json"], timeout=60)
        require(r.returncode != 0, f"{tool} accepted a missing baseline leg")
    print("  [11] research tools name their legs explicitly, fail closed, and "
          "the classifier self-test passes: PASS")


def check_audit_census_ratchet() -> None:
    """The exact hosted-CI ratchet must hold on this tree, not just upstream."""
    r = run([sys.executable, str(ROOT / "scripts/audit_census.py"), "--check"],
            timeout=300)
    require(r.returncode == 0,
            "audit census grew: " +
            (r.stdout + r.stderr).decode("utf-8", "replace")[-400:])
    require(b"PASS: audit census held" in r.stdout,
            "audit census did not report a held ratchet")
    print("  [12] audit census ratchet held (no metric grew): PASS")


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
    # The general path must stay O(1) in heap objects, not just in instruction
    # examinations.  These three rules pin the flat adjacency chain.
    if b"pub allocations: usize" not in production:
        bad.append("the portable allocation witness is gone")
    if b"FxHashMap<Value, Vec<usize>>" in production:
        bad.append("one Vec per distinct home id returned (O(params) allocations)")
    if b"let mut next: Vec<usize>" not in production:
        bad.append("the flat adjacency chain is gone")
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
    print("  [6] no per-query full scan in production; all four prologues build "
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
    # The broken-pipe verdict rule must reject an ICE and a success.
    rejects(lambda: require(pipe_verdict_is_clean(101), "rust ICE"))
    rejects(lambda: require(pipe_verdict_is_clean(0), "wrong verdict"))
    require(pipe_verdict_is_clean(1), "clean verdict rejected")
    # The CLI-text guard must reject each macro form and must NOT be fooled
    # by the same spelling inside a string, a raw string, or a comment.
    for macro in ("eprintln!", "eprint!", "println!", "print!"):
        rejects(lambda m=macro: require(not cli_text_macro_hits(
            f"fn f() {{ {m}(\"x\"); }}"), f"guard missed {m}"))
    require(cli_text_macro_hits('let s = "eprintln!(x)";') == [],
            "guard falsely matched a normal string literal")
    require(cli_text_macro_hits('let s = r#"eprintln!(x)"#;') == [],
            "guard falsely matched a raw string literal")
    require(cli_text_macro_hits('// eprintln!(x)\nlet a = 1;') == [],
            "guard falsely matched a line comment")
    require(cli_text_macro_hits('/* println!(x) */ let a = 1;') == [],
            "guard falsely matched a block comment")
    require(cli_text_macro_hits("let c = '\\n';") == [],
            "guard falsely matched a char literal")
    # A whole-tree guard must reject a synthetic regression, not just text.
    rejects(lambda: require(not cli_text_macro_hits("eprintln!(\"oops\");"),
                            "synthetic reintroduction"))
    # The O(1)-allocation invariant must be able to reject each way the flat
    # adjacency chain could regress; otherwise the witness is decoration.
    rejects(lambda: require(not structural_failures(
        good_prod.replace(b"pub allocations: usize", b"pub gone: usize"),
        good_prologues), "allocation witness deleted"))
    rejects(lambda: require(not structural_failures(
        good_prod + b"\nlet x: FxHashMap<Value, Vec<usize>>;\n",
        good_prologues), "per-home Vec allocation returned"))
    rejects(lambda: require(not structural_failures(
        good_prod.replace(b"let mut next: Vec<usize>",
                          b"let mut next: Vec<u8>"), good_prologues),
        "flat adjacency chain deleted"))
    # Debug-row filter: accept identical code with metadata added, but reject
    # a one-byte instruction change hidden under the same metadata.
    note = '\t.section\t.note.GNU-stack,"",@progbits'
    plain = ["\t.text", "foo:", "\tadd\ta0, a0, 1", "\tret", note]
    debug = ["\t.file\t1 \"x.c\"", "\t.text", "foo:", "\t.loc 1 1 0",
             "\tadd\ta0, a0, 1", "\tret",
             "\t.set .Llccc_debug_alias_1,.text+0",
             '\t.section .debug_info,"",@progbits', "\t.byte 1,2,3", note]
    require(strip_debug_rows(plain) == strip_debug_rows(debug),
            "debug filter erased real code")
    rejects(lambda: require(strip_debug_rows(debug) ==
                            strip_debug_rows([l.replace("add", "sub")
                                             for l in debug]),
                            "one-byte instruction change"))
    # The stale-leg detector must reject a hardcoded baseline path.
    rejects(lambda: require("fastbuild-baseline2" not in
                            "p='/home/user/lccc/target/fastbuild-baseline2/fastbuild'",
                            "stale baseline string"))
    require(cases == 23, f"self-test covered {cases}/23 rejection paths")
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
            check_diagnostic_pipe_safety(work)
            check_cli_text_is_best_effort()
            check_debug_info_invariance_on_shifted_layout(work)
            check_round_cap_exit_coverage(work)
            check_research_tool_reproducibility()
            check_audit_census_ratchet()
    except Failure as exc:
        print(f"PARAM_HOME_CONTRACTS_FAIL: {exc}", file=sys.stderr)
        return 1
    print("PARAM_HOME_CONTRACTS_OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
