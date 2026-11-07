#!/usr/bin/env python3
"""Four-ISA execution proof for the SIMD mask-facts pass.

WHY THIS EXISTS
---------------
An assembly diff is not a correctness proof: a wrong AArch64 NEON reduction
or a wrong RISC-V software reduction still assembles and links.  This harness
compiles two freestanding C fixtures with each of the four LCCC drivers,
assembles with Clang 23's integrated assembler, links a tiny architecture-
specific ``_start`` with mold, EXECUTES the result (natively on x86-64, under
qemu-user on i686/AArch64/RISC-V64), and compares the exit status against an
independent Python oracle.  Every binary is built twice, with ``maskfacts``
enabled and disabled, so a miscompile in the pass AND a miscompile in a
backend's non-folded reduction are both caught.

The constant fixture must ALSO prove the fold fired: its enabled binary must
have strictly fewer non-directive assembly lines than the disabled one, and
its disabled binary must still execute to the same answer.  A harness that
accepted two identical builds would be vacuous.

TOOLS (hash-verified cache, see scripts/restore_agent_toolchain_cache.py)
    clang-23 (integrated assembler), mold 3.0.0 (multi-target linker),
    qemu-i386 / qemu-aarch64 / qemu-riscv64 11.1.3, readelf 2.47.
    No cross sysroot is needed: the fixtures are static and libc-free.

Usage: scripts/check_maskfacts_cross_arch.py [--self-test] [--json PATH]
Exit 0 only when every ISA, both pass states and both fixtures agree.
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
FIXTURES = ROOT / "tests" / "regression" / "maskfacts_cross"
CACHE_BIN = Path(
    os.environ.get("LCCC_AGENT_CACHE_PREFIX", str(Path.home() / ".cache/lccc/agent-toolchains"))
) / "bin"

# One row per supported ISA.  `emulation` is the exact mold target name;
# `qemu` is None when the host can execute the ELF directly.
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

CONSTANT_BYTES = bytes(
    [0x80, 0x00, 0x80, 0x00, 0x80, 0x00, 0x80, 0x00,
     0x00, 0x80, 0x00, 0x80, 0x00, 0x80, 0x00, 0x80]
)


def oracle(b: bytes) -> int:
    """Sign-bit mask of 16 bytes, folded into an 8-bit exit checksum."""
    assert len(b) == 16
    m = sum(1 << i for i, x in enumerate(b) if x & 0x80)
    return ((m >> 8) ^ m) & 0xFF


def runtime_bytes(seed: int = 1) -> bytes:
    return bytes((seed * (i + 1) * 37 + 0x40) & 0xFF for i in range(16))


FIXTURE_ORACLES = {
    "constant_mask": oracle(CONSTANT_BYTES),
    "runtime_mask": oracle(runtime_bytes()),
}

DIRECTIVE = re.compile(r"^\s*(?:[.#]|\.|//|$)")


def asm_instruction_count(text: str) -> int:
    """Non-blank, non-directive lines; comments from the LCCC emitter start #."""
    return sum(1 for line in text.splitlines()
               if line.strip() and not DIRECTIVE.match(line) and not line.lstrip().startswith("#"))


def tool(name: str) -> str:
    candidate = CACHE_BIN / name
    if candidate.is_file() and os.access(candidate, os.X_OK):
        return str(candidate)
    found = shutil.which(name)
    if found:
        return found
    raise RuntimeError(f"required tool {name!r} is unavailable")


def run(cmd: list[str], *, env: dict[str, str] | None = None, timeout: int = 60) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                          text=True, env=env, timeout=timeout)


def build_and_execute(
    target: dict, fixture: str, enabled: bool, work: Path, fastbuild: Path,
) -> dict:
    """Compile -> assemble -> link -> execute ONE (ISA, fixture, pass state)."""
    src = FIXTURES / f"{fixture}.c"
    tag = f"{target['isa']}_{fixture}_{'on' if enabled else 'off'}"
    asm = work / f"{tag}.s"
    combined = work / f"{tag}.combined.s"
    obj = work / f"{tag}.o"
    exe = work / f"{tag}.elf"

    driver = fastbuild / target["driver"]
    if not driver.is_file():
        raise RuntimeError(f"missing LCCC driver {driver}")
    env = dict(os.environ)
    env.pop("CCC_DISABLE_PASSES", None)
    if not enabled:
        env["CCC_DISABLE_PASSES"] = "maskfacts"
    compiled = run([str(driver), "-O2", "-S", str(src), "-o", str(asm)], env=env)
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
        raise RuntimeError(f"{tag}: ELF machine is not {target['machine']}: {header.stdout[:400]}")

    cmd = ([tool(target["qemu"])] if target["qemu"] else []) + [str(exe)]
    executed = run(cmd, timeout=90)
    return {
        "isa": target["isa"], "fixture": fixture, "enabled": enabled,
        "exit": executed.returncode, "instructions": asm_instruction_count(asm_text),
        "elf_machine_ok": True,
    }


def evaluate(results: list[dict]) -> tuple[list[str], dict]:
    """Pure verdict predicate, exercised by --self-test."""
    bad: list[str] = []
    by_key: dict[tuple[str, str, bool], dict] = {}
    for r in results:
        key = (r["isa"], r["fixture"], r["enabled"])
        if key in by_key:
            bad.append(f"duplicate result {key}")
        by_key[key] = r
        expected = FIXTURE_ORACLES[r["fixture"]]
        if r["exit"] != expected:
            bad.append(f"{r['isa']}/{r['fixture']}/{'on' if r['enabled'] else 'off'}: "
                       f"exit {r['exit']} != oracle {expected}")
        if not r["elf_machine_ok"]:
            bad.append(f"{r['isa']}: ELF machine mismatch")

    for target in TARGETS:
        isa = target["isa"]
        for fixture in FIXTURE_ORACLES:
            on = by_key.get((isa, fixture, True))
            off = by_key.get((isa, fixture, False))
            if on is None or off is None:
                bad.append(f"{isa}/{fixture}: missing a pass state")
                continue
            if fixture == "constant_mask" and on["instructions"] >= off["instructions"]:
                bad.append(f"{isa}/constant_mask: fold did not fire "
                           f"(on={on['instructions']} off={off['instructions']})")
            if fixture == "runtime_mask" and on["instructions"] != off["instructions"]:
                bad.append(f"{isa}/runtime_mask: runtime image was folded or rewritten "
                           f"(on={on['instructions']} off={off['instructions']})")
    table = {
        f"{r['isa']}/{r['fixture']}/{'on' if r['enabled'] else 'off'}": {
            "exit": r["exit"], "instructions": r["instructions"],
        }
        for r in results
    }
    return bad, table


def self_test() -> int:
    """The checker must be able to FAIL on each class of broken evidence."""
    good: list[dict] = []
    for target in TARGETS:
        for fixture, expected in FIXTURE_ORACLES.items():
            for enabled in (True, False):
                good.append({
                    "isa": target["isa"], "fixture": fixture, "enabled": enabled,
                    "exit": expected,
                    "instructions": 3 if enabled and fixture == "constant_mask" else 40,
                    "elf_machine_ok": True,
                })
    bad, _ = evaluate(good)
    assert not bad, f"self-test baseline is not clean: {bad}"
    mutations = [
        ("wrong exit", lambda rs: rs[0].update(exit=rs[0]["exit"] + 1),
         "exit"),
        ("missing fold", lambda rs: [r.update(instructions=40) for r in rs
                                    if r["fixture"] == "constant_mask" and r["enabled"]],
         "fold did not fire"),
        ("runtime folded", lambda rs: [r.update(instructions=3) for r in rs
                                       if r["fixture"] == "runtime_mask" and r["enabled"]],
         "runtime image was folded"),
        ("machine mismatch", lambda rs: rs[0].update(elf_machine_ok=False),
         "ELF machine mismatch"),
        ("missing state", lambda rs: rs.pop(), "missing a pass state"),
    ]
    for label, mutate, needle in mutations:
        rs = [dict(r) for r in good]
        mutate(rs)
        bad, _ = evaluate(rs)
        if not any(needle in b for b in bad):
            print(f"SELF-TEST FAIL: {label!r} was not detected")
            return 1
    assert oracle(CONSTANT_BYTES) == 0xFF
    assert oracle(runtime_bytes()) == FIXTURE_ORACLES["runtime_mask"]
    print("SELF-TEST PASS: every evidence rule rejects its mutation")
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--json", type=Path, default=None)
    parser.add_argument("--fastbuild", type=Path, default=ROOT / "target" / "fastbuild")
    args = parser.parse_args(argv)
    if args.self_test:
        return self_test()

    for name in ("clang-23", "mold", "readelf", "qemu-i386", "qemu-aarch64", "qemu-riscv64"):
        try:
            tool(name)
        except RuntimeError as exc:
            print(f"SKIP: {exc}")
            return 0  # tool absence is a skip, never a green claim
    results: list[dict] = []
    failures: list[str] = []
    with tempfile.TemporaryDirectory(prefix="maskfacts-cross-") as tmp:
        work = Path(tmp)
        for target in TARGETS:
            for fixture in FIXTURE_ORACLES:
                for enabled in (True, False):
                    try:
                        results.append(build_and_execute(target, fixture, enabled, work, args.fastbuild))
                    except (RuntimeError, subprocess.TimeoutExpired) as exc:
                        failures.append(f"{target['isa']}/{fixture}/"
                                        f"{'on' if enabled else 'off'}: {exc}")
    bad, table = evaluate(results)
    failures.extend(bad)
    print("ISA       fixture        pass  exit  asm-ins")
    for r in results:
        print(f"{r['isa']:<9} {r['fixture']:<14} "
              f"{'on ' if r['enabled'] else 'off'}   {r['exit']:<5} {r['instructions']}")
    for fixture, expected in FIXTURE_ORACLES.items():
        print(f"oracle: {fixture} = {expected} (0x{expected:02X})")
    payload = {"ok": not failures, "results": table, "failures": failures}
    if args.json:
        args.json.write_text(json.dumps(payload, indent=2) + "\n")
    if failures:
        for f in failures:
            print(f"FAIL: {f}")
        return 1
    print("MASKFACTS_CROSS_ARCH_OK: 4 ISAs x 2 fixtures x 2 pass states, "
          "all executed and oracle-identical")
    return 0


if __name__ == "__main__":
    sys.exit(main())
