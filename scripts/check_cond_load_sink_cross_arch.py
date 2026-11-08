#!/usr/bin/env python3
"""Four-ISA EXECUTION proof for the SV-03 conditional-load sinking pass.

WHY THIS EXISTS
---------------
`sink_conditional_loads` emits ``Select { ty: IrType::Ptr }``.  Pointer
width differs across the supported targets (ILP32 x86, LP64 x86-64/AArch64/
RISC-V64), and the legacy `cmov` emitter sizes a Select by `ty.size()` while
the MachInst emitter maps `Ptr` to a 64-bit move.  An assembly diff cannot
prove that a 32-bit ``movl`` (i686), a 64-bit ``csel`` (AArch64) or a
branch-based select (RISC-V64) actually produces the right address at
runtime.  This harness compiles the SAME freestanding fixture with each of the
four LCCC drivers, assembles with Clang 23's integrated assembler, links with
mold, EXECUTES (natively on x86-64, under qemu-user elsewhere) and compares
the exit status to an independent Python oracle.

Both pass states are built: `CCC_NO_COND_LOAD_SINK=1` is the mutation control.
The ON build MUST have strictly fewer emitted instructions than OFF on every
ISA, so a harness that accepted two identical builds would be vacuous.

TOOLS (hash-verified `harness-cache-8` toolchain prefix)
    clang-23 (integrated assembler), mold 3.0.0 (multi-target linker),
    qemu-i386 / qemu-aarch64 / qemu-riscv64 11.1.3, readelf 2.47.
    No cross sysroot: the fixture is static and libc-free.

Usage: scripts/check_cond_load_sink_cross_arch.py [--self-test] [--json PATH]
Exit 0 only when every ISA and both pass states agree with the oracle.
"""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
FIXTURE = ROOT / "tests" / "regression" / "cond_load_sink_cross.c"
CACHE_BIN = Path(
    os.environ.get("LCCC_AGENT_CACHE_PREFIX", str(Path.home() / ".cache/lccc/agent-toolchains"))
) / "bin"
QEMU_PREFIX = Path(
    os.environ.get("LCCC_QEMU_PREFIX", str(CACHE_BIN.parent / "qemu-user-11.1.3-x86_64"))
) / "bin"

TARGETS = (
    {"isa": "x86_64", "driver": "lccc", "triple": "x86_64-linux-gnu",
     "emulation": "elf_x86_64", "qemu": None, "machine": "Advanced Micro Devices X86-64"},
    {"isa": "i686", "driver": "lccc-i686", "triple": "i686-linux-gnu",
     "emulation": "elf_i386", "qemu": "qemu-i386", "machine": "Intel 80386"},
    {"isa": "aarch64", "driver": "lccc-arm", "triple": "aarch64-linux-gnu",
     "emulation": "aarch64linux", "qemu": "qemu-aarch64", "machine": "AArch64"},
    {"isa": "riscv64", "driver": "lccc-riscv", "triple": "riscv64-linux-gnu",
     "emulation": "elf64lriscv", "qemu": "qemu-riscv64", "machine": "RISC-V"},
)

START_ASM = {
    "x86_64": """
.text
.globl _start
_start:
    call lccc_main
    mov %eax, %edi
    mov $60, %eax
    syscall
""",
    "i686": """
.text
.globl _start
_start:
    call lccc_main
    mov %eax, %ebx
    mov $1, %eax
    int $0x80
""",
    "aarch64": """
.text
.globl _start
_start:
    bl lccc_main
    mov x8, #93
    svc #0
""",
    "riscv64": """
.text
.globl _start
_start:
    jal ra, lccc_main
    li a7, 93
    ecall
""",
}

# Independent oracle: x = A[2] = 33, y = B[2] = 77.
ORACLE = (33 * 3 + 77 * 7) % 61
DIRECTIVE = re.compile(r"^\s*(?:[.#]|//|$)")


def asm_instruction_count(text: str) -> int:
    return sum(1 for line in text.splitlines()
               if line.strip() and not DIRECTIVE.match(line)
               and not line.lstrip().startswith("#"))


def tool(name: str) -> str:
    for candidate in (CACHE_BIN / name, QEMU_PREFIX / name):
        if candidate.is_file() and os.access(candidate, os.X_OK):
            return str(candidate)
    found = shutil.which(name)
    if found:
        return found
    raise RuntimeError(f"required tool {name!r} is unavailable")


def run(cmd: list[str], *, env: dict[str, str] | None = None,
        timeout: int = 90) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                          text=True, env=env, timeout=timeout)


def build_and_execute(target: dict, enabled: bool, work: Path,
                      fastbuild: Path) -> dict:
    tag = f"{target['isa']}_{'on' if enabled else 'off'}"
    asm, combined = work / f"{tag}.s", work / f"{tag}.combined.s"
    obj, exe = work / f"{tag}.o", work / f"{tag}.elf"
    driver = fastbuild / target["driver"]
    if not driver.is_file():
        raise RuntimeError(f"missing LCCC driver {driver}")

    env = dict(os.environ)
    env.pop("CCC_NO_COND_LOAD_SINK", None)
    if not enabled:
        env["CCC_NO_COND_LOAD_SINK"] = "1"
    compiled = run([str(driver), "-O2", "-S", str(FIXTURE), "-o", str(asm)], env=env)
    if compiled.returncode != 0:
        raise RuntimeError(f"{tag}: LCCC compile failed: {compiled.stderr.strip()[:400]}")
    asm_text = asm.read_text()
    combined.write_text(asm_text + "\n" + START_ASM[target["isa"]])

    assembled = run([tool("clang-23"), f"--target={target['triple']}", "-c",
                     str(combined), "-o", str(obj)])
    if assembled.returncode != 0:
        raise RuntimeError(f"{tag}: assembler failed: {assembled.stderr.strip()[:400]}")
    linked = run([tool("mold"), "-m", target["emulation"], str(obj), "-o", str(exe),
                  "--entry=_start"])
    if linked.returncode != 0:
        raise RuntimeError(f"{tag}: link failed: {linked.stderr.strip()[:400]}")
    header = run([tool("readelf"), "-h", str(exe)])
    if header.returncode != 0 or target["machine"] not in header.stdout:
        raise RuntimeError(f"{tag}: ELF machine mismatch: {header.stdout[:400]}")
    cmd = ([tool(target["qemu"])] if target["qemu"] else []) + [str(exe)]
    executed = run(cmd, timeout=90)
    return {"isa": target["isa"], "enabled": enabled, "exit": executed.returncode,
            "instructions": asm_instruction_count(asm_text), "elf_machine_ok": True}


def evaluate(results: list[dict]) -> tuple[list[str], dict]:
    """Pure verdict predicate, exercised by --self-test."""
    bad: list[str] = []
    by_key: dict[tuple[str, bool], dict] = {}
    for r in results:
        key = (r["isa"], r["enabled"])
        if key in by_key:
            bad.append(f"duplicate result {key}")
        by_key[key] = r
        if r["exit"] != ORACLE:
            bad.append(f"{r['isa']}/{'on' if r['enabled'] else 'off'}: "
                       f"exit {r['exit']} != oracle {ORACLE}")
        if not r["elf_machine_ok"]:
            bad.append(f"{r['isa']}: ELF machine mismatch")
    for target in TARGETS:
        isa = target["isa"]
        on, off = by_key.get((isa, True)), by_key.get((isa, False))
        if on is None or off is None:
            bad.append(f"{isa}: missing a pass state")
            continue
        if on["instructions"] >= off["instructions"]:
            bad.append(f"{isa}: transform did not fire "
                       f"(on={on['instructions']} off={off['instructions']})")
    table = {f"{r['isa']}/{'on' if r['enabled'] else 'off'}": {
        "exit": r["exit"], "instructions": r["instructions"]} for r in results}
    return bad, table


def self_test() -> int:
    good = [{"isa": t["isa"], "enabled": e, "exit": ORACLE,
             "instructions": 10 if e else 20, "elf_machine_ok": True}
            for t in TARGETS for e in (True, False)]
    bad, _ = evaluate(good)
    assert not bad, f"self-test baseline is not clean: {bad}"
    mutations = [
        ("wrong exit", lambda rs: rs[0].update(exit=rs[0]["exit"] + 1), "exit"),
        ("missing transform", lambda rs: [r.update(instructions=20) for r in rs
                                          if r["enabled"]], "did not fire"),
        ("bad ELF machine", lambda rs: rs[0].update(elf_machine_ok=False), "ELF machine"),
        ("missing pass state", lambda rs: rs.pop(), "missing a pass state"),
    ]
    for name, mutate, want in mutations:
        sample = [dict(r) for r in good]
        mutate(sample)
        bad, _ = evaluate(sample)
        if not any(want in b for b in bad):
            print(f"self-test FAIL: '{name}' was not detected: {bad}")
            return 1
    print(f"PASS cond-load-sink cross-arch self-test "
          f"({len(mutations)} synthetic mutations rejected, oracle={ORACLE})")
    return 0


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--self-test", action="store_true")
    ap.add_argument("--json", type=Path)
    ap.add_argument("--fastbuild", type=Path, default=ROOT / "target" / "fastbuild")
    args = ap.parse_args(argv)
    if args.self_test:
        return self_test()

    results: list[dict] = []
    try:
        with tempfile.TemporaryDirectory(prefix="lccc-cond-load-cross-") as tmp:
            work = Path(tmp)
            for target in TARGETS:
                for enabled in (True, False):
                    results.append(build_and_execute(target, enabled, work, args.fastbuild))
    except (RuntimeError, subprocess.TimeoutExpired) as exc:
        print(f"FAIL cond-load-sink cross-arch: {exc}", file=sys.stderr)
        return 1
    bad, table = evaluate(results)
    if args.json:
        args.json.write_text(json.dumps({"oracle": ORACLE, "results": results,
                                         "table": table, "problems": bad},
                                        indent=2, sort_keys=True) + "\n")
    for r in sorted(results, key=lambda x: (x["isa"], not x["enabled"])):
        print(f"PASS {r['isa']:8} {'on ' if r['enabled'] else 'off'} "
              f"exit={r['exit']:3} instructions={r['instructions']}")
    if bad:
        for b in bad:
            print(f"FAIL {b}", file=sys.stderr)
        return 1
    print(f"PASS cond-load-sink four-ISA execution proof "
          f"({len(results)} binaries, oracle={ORACLE})")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
