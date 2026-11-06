#!/usr/bin/env python3
"""EXECUTE the parameter-home fixtures on all four supported ISAs.

Compile-only evidence is vacuous for a parameter-home index: a home resolved
into the wrong slot still assembles and links.  This harness compiles each
self-checking freestanding fixture with every LCCC target driver, links it
with one LLD that is capability-probed for all four emulations, verifies the
ELF ``e_machine`` (so a wrong-ISA binary cannot pass), and RUNS it -- natively
on x86-64 and under qemu-user for i686 / AArch64 / RISC-V 64.  GCC 16.2 and
Clang 23.1.5 compile the same sources natively as independent host oracles.

No libc, no headers, no output comparison: ``_start`` turns ``main``'s return
value into the raw kernel exit syscall, and each fixture hardcodes its own
expected checksum.  A green run therefore proves the incoming arguments
arrived in the right homes, not merely that some object was produced.

Usage:
  python3 tests/regression/check_param_home_cross_execute.py --self-test
  python3 tests/regression/check_param_home_cross_execute.py \
      --lccc-dir target/fastbuild --out /tmp/param_home_cross.json
"""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]

ARCHES = {
    "x86_64": dict(driver="lccc", triple="x86_64-linux-gnu", emu="elf_x86_64",
                   runner=None, machine=62),
    "i686": dict(driver="lccc-i686", triple="i386-linux-gnu", emu="elf_i386",
                 runner="qemu-i386", machine=3),
    "aarch64": dict(driver="lccc-arm", triple="aarch64-linux-gnu", emu="aarch64linux",
                    runner="qemu-aarch64", machine=183),
    "riscv64": dict(driver="lccc-riscv", triple="riscv64-linux-gnu", emu="elf64lriscv",
                    runner="qemu-riscv64", machine=243),
}
FIXTURES = ("param_home_shifted_cross", "param_home_wide_cross")
LEVELS = ("-O0", "-O2", "-Os")
HOST_ORACLES = ("gcc-16", "clang-23")

# Raw syscall exit stubs.  main()'s return value becomes the process status;
# no libc, no crt, no constructor, no TLS.  memcpy/memset are provided for
# the 32-bit ABI (cdecl) and the 64-bit ABIs because a freestanding compiler
# may emit them for aggregate copies; our fixtures do not need them, but a
# missing symbol would otherwise look like a compiler defect.
START = {
    "x86_64": r"""
.text
.globl _start
_start:
    andq $-16, %rsp
    call main
    mov %eax, %edi
    mov $60, %eax
    syscall
    hlt
.globl abort
abort:
    ud2
""",
    "i686": r"""
.text
.globl _start
_start:
    andl $-16, %esp
    call main
    mov %eax, %ebx
    mov $1, %eax
    int $0x80
    hlt
.globl abort
abort:
    ud2
.globl memcpy
memcpy:
    push %ebp
    mov %esp, %ebp
    push %esi
    push %edi
    mov 8(%ebp), %edi
    mov 12(%ebp), %esi
    mov 16(%ebp), %ecx
    mov 8(%ebp), %eax
    test %ecx, %ecx
    je 2f
1:  movb (%esi), %dl
    movb %dl, (%edi)
    inc %esi
    inc %edi
    dec %ecx
    jne 1b
2:  pop %edi
    pop %esi
    pop %ebp
    ret
.globl memset
memset:
    push %ebp
    mov %esp, %ebp
    push %edi
    mov 8(%ebp), %edi
    mov 12(%ebp), %eax
    mov 16(%ebp), %ecx
    mov 8(%ebp), %edx
    test %ecx, %ecx
    je 2f
1:  movb %al, (%edi)
    inc %edi
    dec %ecx
    jne 1b
2:  mov %edx, %eax
    pop %edi
    pop %ebp
    ret
""",
    "aarch64": r"""
.text
.globl _start
_start:
    bl main
    mov w8, #93
    svc #0
.globl abort
abort:
    brk #0
.globl memcpy
memcpy:
    mov x3, x0
1:  cbz x2, 2f
    ldrb w4, [x1], #1
    strb w4, [x3], #1
    sub x2, x2, #1
    b 1b
2:  ret
.globl memset
memset:
    mov x3, x0
1:  cbz x2, 2f
    strb w1, [x3], #1
    sub x2, x2, #1
    b 1b
2:  ret
""",
    "riscv64": r"""
.text
.globl _start
_start:
 .option push
 .option norelax
 lla gp, __global_pointer$
 .option pop
    call main
    li a7, 93
    ecall
.globl abort
abort:
    ebreak
.globl memcpy
memcpy:
    mv t0, a0
1:  beqz a2, 2f
    lbu t1, 0(a1)
    sb t1, 0(t0)
    addi a1, a1, 1
    addi t0, t0, 1
    addi a2, a2, -1
    j 1b
2:  ret
.globl memset
memset:
    mv t0, a0
1:  beqz a2, 2f
    sb a1, 0(t0)
    addi t0, t0, 1
    addi a2, a2, -1
    j 1b
2:  ret
""",
}


def which(name: str) -> str:
    found = shutil.which(name)
    if not found:
        raise FileNotFoundError(f"required tool {name!r} is not on PATH (fail closed)")
    return found


def find_linker(explicit: str | None) -> str:
    """LLD-first: GNU ld cannot serve all four emulations from one binary."""
    if explicit:
        return explicit
    for name in ("ld.lld", "ld.lld-23", "rust-lld"):
        if found := shutil.which(name):
            return found
    # Rust ships a multi-emulation LLD next to its target libdir.
    libdir = subprocess.check_output(
        [which("rustc"), "--print", "target-libdir"], text=True, timeout=15).strip()
    for rel in ("../bin/gcc-ld/ld.lld", "../bin/rust-lld", "../gcc-ld/ld.lld"):
        candidate = (Path(libdir) / rel).resolve()
        if candidate.is_file():
            return str(candidate)
    raise FileNotFoundError("no LLD found; install ld.lld or use --ld (fail closed)")


def probe_linker(linker: str) -> None:
    """Refuse a linker that cannot handle ANY of the four emulations."""
    for arch, spec in ARCHES.items():
        r = subprocess.run([linker, "-m", spec["emu"], "--version"],
                           capture_output=True, timeout=20)
        if r.returncode != 0:
            raise RuntimeError(
                f"linker {linker} rejects -m {spec['emu']} ({arch}): "
                + (r.stdout + r.stderr).decode("utf-8", "replace")[-200:])


def expect_machine(path: Path, want: int) -> None:
    """Read e_machine from the ELF header directly (bytes 18..19, LE).

    A compile-only gate cannot tell an x86-64 object that was silently linked
    for the wrong target from a genuinely cross-compiled one; this can.
    """
    head = path.read_bytes()[:20]
    if head[:4] != b"\x7fELF" or len(head) < 20:
        raise ValueError(f"{path}: not an ELF file")
    got = struct.unpack_from("<H", head, 18)[0]
    if got != want:
        raise ValueError(f"{path}: e_machine {got}, expected {want}")


def run(argv: list[str], timeout: int = 60) -> int:
    return subprocess.run(argv, capture_output=True, timeout=timeout).returncode


def compile_and_run(*, arch: str, fixture: str, level: str, leg: str,
                    work: Path, lccc_dir: Path, clang: str, linker: str,
                    qemu_dir: Path, source: Path, expect_rc: int = 0) -> dict:
    """One executed case.  Returns a JSON row; never raises on a bad answer."""
    spec = ARCHES[arch]
    stem = f"{arch}-{fixture}-{level.replace('-', '')}-{leg.replace('-', '')}"
    obj = work / f"{stem}.o"
    exe = work / f"{stem}.elf"
    startup_s = work / f"start-{arch}.s"
    startup_o = work / f"start-{arch}.o"

    steps: list[list[str]] = []
    if leg.startswith("lccc"):
        steps.append([str(lccc_dir / spec["driver"]), level, "-c", str(source), "-o", str(obj)])
    elif leg.startswith("gcc"):
        steps.append([leg, level, "-ffreestanding", "-fno-builtin", "-fno-pic",
                      "-fno-pie", "-fno-stack-protector", "-c", str(source), "-o", str(obj)])
    else:
        steps.append([clang, level, "-ffreestanding", "-fno-builtin", "-fno-pic",
                      "-fno-pie", "-fno-stack-protector", "-c", str(source), "-o", str(obj)])

    if not startup_o.is_file():
        startup_s.write_text(START[arch])
        r = subprocess.run([clang, f"--target={spec['triple']}", "-c",
                            str(startup_s), "-o", str(startup_o)],
                           capture_output=True, timeout=30)
        if r.returncode != 0:
            return dict(leg=leg, arch=arch, fixture=fixture, opt=level,
                        stage="startup", rc=r.returncode,
                        detail=(r.stdout + r.stderr).decode("utf-8", "replace")[-300:])

    steps.append([linker, "-m", spec["emu"], "-static", "-e", "_start",
                  str(startup_o), str(obj), "-o", str(exe)])

    for step in steps:
        p = subprocess.run(step, capture_output=True, timeout=60)
        if p.returncode != 0:
            return dict(leg=leg, arch=arch, fixture=fixture, opt=level,
                        stage=Path(step[0]).name, rc=p.returncode,
                        detail=(p.stdout + p.stderr).decode("utf-8", "replace")[-300:])

    try:
        expect_machine(exe, spec["machine"])
    except (ValueError, OSError) as exc:
        return dict(leg=leg, arch=arch, fixture=fixture, opt=level,
                    stage="e_machine", rc=1, detail=str(exc))

    if spec["runner"]:
        candidate = qemu_dir / spec["runner"]
        runner = str(candidate) if candidate.is_file() else which(spec["runner"])
    else:
        runner = str(exe)
    argv = [runner] if spec["runner"] is None else [runner, str(exe)]
    p = subprocess.run(argv, capture_output=True, timeout=60)
    ok = p.returncode == expect_rc
    return dict(leg=leg, arch=arch, fixture=fixture, opt=level, stage="run",
                rc=p.returncode, ok=ok, detail="",
                **(dict(expected_rc=expect_rc) if not ok else {}))


def self_test(args) -> int:
    """Prove the harness is non-vacuous: it must ACCEPT a correct answer and
    REJECT a wrong answer, a wrong expected status, and a wrong e_machine."""
    linker = find_linker(args.ld)
    probe_linker(linker)
    clang = args.clang
    qemu_dir = args.qemu_dir
    with tempfile.TemporaryDirectory(prefix="param-home-cross-selftest-") as td:
        work = Path(td)
        source = ROOT / "tests/regression" / "param_home_shifted_cross.c"
        good = compile_and_run(arch="x86_64", fixture="param_home_shifted_cross",
                               level="-O0", leg="lccc_x86_64", work=work,
                               lccc_dir=args.lccc_dir, clang=clang, linker=linker,
                               qemu_dir=qemu_dir, source=source)
        if good.get("stage") != "run" or not good.get("ok"):
            print(f"SELF_TEST_FAIL: good case did not pass: {good}")
            return 1
        print(f"  self-test good: stage={good['stage']} rc={good['rc']}")

        # Same binary, wrong expected status: the verdict MUST flip.
        bad = compile_and_run(arch="x86_64", fixture="param_home_shifted_cross",
                              level="-O0", leg="lccc_x86_64", work=work,
                              lccc_dir=args.lccc_dir, clang=clang, linker=linker,
                              qemu_dir=qemu_dir, source=source, expect_rc=1)
        if bad.get("ok"):
            print("SELF_TEST_FAIL: wrong expected status was accepted")
            return 1
        print(f"  self-test bad: stage={bad['stage']} rc={bad['rc']} (correctly rejected)")

        # A genuinely wrong checksum must be reported, not swallowed.
        wrong = work / "wrong.c"
        wrong.write_text("int main(void) { return 7; }\n")
        wrong_case = compile_and_run(arch="x86_64", fixture="wrong_checksum",
                                     level="-O0", leg="lccc_x86_64", work=work,
                                     lccc_dir=args.lccc_dir, clang=clang, linker=linker,
                                     qemu_dir=qemu_dir, source=wrong)
        if wrong_case.get("ok"):
            print("SELF_TEST_FAIL: wrong checksum was accepted")
            return 1
        print(f"  self-test wrong-checksum: rc={wrong_case['rc']} (correctly rejected)")

        # e_machine guard: a well-formed SPARC ELF header (e_machine 2 at
        # offset 18) must be rejected as x86-64 for the RIGHT reason.
        sparc = work / "sparc.elf"
        hdr = bytearray(20)
        hdr[:4] = b"\x7fELF"
        hdr[4], hdr[5], hdr[6] = 2, 1, 1
        struct.pack_into("<H", hdr, 16, 2)  # e_type = ET_REL
        struct.pack_into("<H", hdr, 18, 2)  # e_machine = EM_SPARC
        sparc.write_bytes(hdr)
        try:
            expect_machine(sparc, 62)
            print("SELF_TEST_FAIL: SPARC e_machine was accepted as x86-64")
            return 1
        except ValueError as exc:
            if "e_machine 2" not in str(exc):
                print(f"SELF_TEST_FAIL: SPARC rejected for the wrong reason: {exc}")
                return 1
            print("  self-test e_machine guard: correctly rejected SPARC")
    print("PARAM_HOME_CROSS_SELFTEST_OK (non-vacuous verdicts + ISA guard)")
    return 0


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--lccc-dir", type=Path, default=ROOT / "target/fastbuild")
    ap.add_argument("--clang", default=os.environ.get("CLANG", "clang-23"))
    ap.add_argument("--ld", help="explicit multi-emulation LLD")
    ap.add_argument("--qemu-dir", type=Path, default=Path("/usr/bin"))
    ap.add_argument("--out", type=Path, default=None)
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args(argv)

    if args.self_test:
        return self_test(args)

    which(args.clang)
    linker = find_linker(args.ld)
    probe_linker(linker)
    # Fail closed on a missing qemu instead of silently skipping an ISA.
    for spec in ARCHES.values():
        if spec["runner"] and not ((args.qemu_dir / spec["runner"]).is_file()
                                   or shutil.which(spec["runner"])):
            raise FileNotFoundError(f"qemu runner {spec['runner']} absent (fail closed)")

    rows: list[dict] = []
    with tempfile.TemporaryDirectory(prefix="param-home-cross-") as td:
        work = Path(td)
        # Host oracles: the same fixture must ALSO be correct with two mature
        # compilers, otherwise the fixture itself (not LCCC) is under suspicion.
        for oracle in HOST_ORACLES:
            which(oracle)
            for fixture in FIXTURES:
                source = ROOT / "tests/regression" / f"{fixture}.c"
                rows.append(compile_and_run(
                    arch="x86_64", fixture=fixture, level="-O2", leg=oracle,
                    work=work, lccc_dir=args.lccc_dir, clang=args.clang,
                    linker=linker, qemu_dir=args.qemu_dir, source=source))
        for arch in ARCHES:
            for fixture in FIXTURES:
                source = ROOT / "tests/regression" / f"{fixture}.c"
                for level in LEVELS:
                    rows.append(compile_and_run(
                        arch=arch, fixture=fixture, level=level,
                        leg=f"lccc_{arch}", work=work, lccc_dir=args.lccc_dir,
                        clang=args.clang, linker=linker, qemu_dir=args.qemu_dir,
                        source=source))

    passed = sum(1 for r in rows if r.get("stage") == "run" and r.get("ok"))
    failed = len(rows) - passed
    report = dict(complete=True, lccc_dir=str(args.lccc_dir), linker=linker,
                  arches=list(ARCHES), opts=list(LEVELS), fixtures=list(FIXTURES),
                  legs=sorted({r["leg"] for r in rows}), cases=len(rows),
                  passed=passed, failed=failed, rows=rows)
    if args.out:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(json.dumps(report, indent=2) + "\n")
    print(f"PARAM_HOME_CROSS: {passed}/{len(rows)} executed correctly"
          + (f" -> {args.out}" if args.out else ""))
    for r in rows:
        if not r.get("ok"):
            print(f"  FAIL {r['leg']} {r['arch']} {r['fixture']} {r['opt']}: "
                  f"{r['stage']} rc={r['rc']} {r.get('detail', '')[:160]}")
    if failed:
        return 1
    print(f"PARAM_HOME_CROSS_OK ({len(rows)} executed cases, {len(ARCHES)} ISAs, "
          f"{len(FIXTURES)} fixtures, {len(LEVELS)} opt levels, GCC + Clang host oracles)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
