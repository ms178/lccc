#!/usr/bin/env python3
"""RV64 volatile accesses survive the text peephole; run freestanding when possible."""
import argparse
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile

from check_switch_cross_execute import select_linker

FIXTURE_DIR = Path(__file__).parent / "fixtures" / "riscv-volatile"
FIXTURE = FIXTURE_DIR / "probe.c"
FAR_JUMP = FIXTURE_DIR / "far_jump.c"
ACCESS = re.compile(
    r"^\s*(lb|lbu|lh|lhu|lw|lwu|ld|sb|sh|sw|sd)\s+([a-z0-9]+),\s*"
    r"(-?\d+)\((\w+)\)\s*$"
)
START = """.text
.globl _start
_start:
    call main
    li a7, 93
    ecall
"""


def body(assembly, name):
    match = re.search(rf"(?ms)^{name}:\s*$(.*?)^[ \t]*\.size\s+{name}\b", assembly)
    if not match:
        raise AssertionError(f"missing function {name}")
    return match.group(1)


def accesses(assembly, name):
    found = []
    for line in body(assembly, name).splitlines():
        if match := ACCESS.match(line):
            op, _reg, offset, base = match.groups()
            found.append((op, int(offset), base))
    return found


def initialized_then_read_twice(assembly, name, store, reads):
    ops = accesses(assembly, name)
    for index, (op, off, base) in enumerate(ops):
        if op == store and base in ("s0", "sp"):
            later = sum(o in reads and (a, b) == (off, base)
                        for o, a, b in ops[index + 1:])
            if later >= 2:
                return True
    return False


def inspect(assembly, mode, expect_debug=False):
    errors = []
    for name, store, reads in (
        ("local_reads", "sw", {"lw", "lwu"}),
        ("parameter_reads", "sw", {"lw", "lwu"}),
        ("byte_reads", "sb", {"lb", "lbu"}),
        ("wide_reads", "sd", {"ld"}),
        ("local_field_reads", "sw", {"lw", "lwu"}),
    ):
        if not initialized_then_read_twice(assembly, name, store, reads):
            errors.append(f"{mode} {name}: missing initialized slot and two later loads: "
                          f"{accesses(assembly, name)}")
    stores = accesses(assembly, "local_stores")
    by_slot = {}
    for op, off, base in stores:
        if op == "sw" and base in ("s0", "sp"):
            by_slot[(off, base)] = by_slot.get((off, base), 0) + 1
    if max(by_slot.values(), default=0) < 2:
        errors.append(f"{mode} local_stores: fewer than two same-slot stores: {stores}")
    for name, opcode in (("pointer_reads", "lw"), ("global_reads", "lw"),
                         ("pointer_field_reads", "lw"), ("pointer_stores", "sw")):
        ops = accesses(assembly, name)
        count = sum(op == opcode and base not in ("s0", "sp") for op, _off, base in ops)
        if count < 2:
            errors.append(f"{mode} {name}: only {count} indirect {opcode} operations: {ops}")
    if re.search(r"(?m)^\s*(?:lb|lbu|lh|lhu|lw|lwu|ld)\s", body(assembly, "plain_discard")):
        errors.append(f"{mode} plain_discard unexpectedly loads nonvolatile data")
    if any(marker in assembly for marker in ("LCCC_VOLATILE_BEGIN", "LCCC_VOLATILE_END",
                                              "LCCC_RISCV_OPAQUE")):
        errors.append(f"{mode}: internal volatile markers leaked into assembly")
    if expect_debug and not re.search(r"(?m)^\s*\.loc\s", body(assembly, "local_reads")):
        errors.append(f"{mode}: local_reads lost its debug rows")
    return errors


def run_runtime(compiler, clang, qemu, linker, work, env):
    startup = work / "start.s"
    startup.write_text(START)
    start_obj = work / "start.o"
    subprocess.run([clang, "--target=riscv64-linux-gnu", "-c", str(startup),
                    "-o", str(start_obj)], check=True, timeout=30)
    for level in ("0", "1", "2", "3", "s"):
        obj = work / f"probe-{level}.o"
        exe = work / f"probe-{level}"
        subprocess.run([compiler, f"-O{level}", "-ffreestanding", "-c", str(FIXTURE),
                        "-o", str(obj)], env=env, check=True, timeout=60)
        subprocess.run([*linker, "-m", "elf64lriscv", "-static", "-e", "_start",
                        str(start_obj), str(obj), "-o", str(exe)], check=True, timeout=30)
        subprocess.run([qemu, str(exe)], check=True, timeout=30)
    print("RV64 volatile freestanding execution: PASS (-O0/-O1/-O2/-O3/-Os)")
    obj, exe = work / "far.o", work / "far"
    subprocess.run([compiler, "-O2", "-ffreestanding", "-c", str(FAR_JUMP),
                    "-o", str(obj)], env=env, check=True, timeout=60)
    subprocess.run([*linker, "-m", "elf64lriscv", "-static", "-e", "_start",
                    str(start_obj), str(obj), "-o", str(exe)], check=True, timeout=30)
    subprocess.run([qemu, str(exe)], check=True, timeout=30)
    print("RV64 >1 MiB forward jump: PASS (link + QEMU)")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compiler", default="target/fastbuild/lccc-riscv")
    parser.add_argument("--clang", default=os.environ.get("CLANG", "clang-23"))
    parser.add_argument("--qemu", default="qemu-riscv64")
    parser.add_argument("--ld", help="optional GNU/LLD linker")
    parser.add_argument("--require-qemu", action="store_true")
    args = parser.parse_args()
    compiler = str(Path(args.compiler).resolve())
    errors = []
    no_unwind = ["-O2", "-fno-unwind-tables", "-fno-asynchronous-unwind-tables"]
    modes = [(f"-O{level}", [f"-O{level}"], {}) for level in ("0", "1", "2", "3", "s")]
    modes += [
        ("-O2 -g", ["-O2", "-g"], {}),
        ("-O2 without unwind tables", no_unwind, {}),
        ("-O2 without unwind tables, peephole disabled", no_unwind, {"LCCC_NO_PEEPHOLE": "1"}),
        ("-O2 LCCC_NO_PEEPHOLE", ["-O2"], {"LCCC_NO_PEEPHOLE": "1"}),
        ("-O2 CCC_NO_PEEPHOLE", ["-O2"], {"CCC_NO_PEEPHOLE": "1"}),
    ]
    clean_env = os.environ.copy()
    clean_env.pop("LCCC_NO_PEEPHOLE", None)
    clean_env.pop("CCC_NO_PEEPHOLE", None)
    outputs = {}
    with tempfile.TemporaryDirectory(prefix="lccc-riscv-volatile-") as directory:
        work = Path(directory)
        output = work / "probe.s"
        for mode, flags, overrides in modes:
            env = clean_env | overrides
            command = [compiler, *flags, "-S", str(FIXTURE), "-o", str(output)]
            result = subprocess.run(command, env=env, capture_output=True, text=True,
                                    timeout=60, check=False)
            if result.returncode:
                errors.append(f"{mode} compile failed: {result.stderr}")
                continue
            outputs[mode] = output.read_text()
            try:
                errors.extend(inspect(outputs[mode], mode, expect_debug="-g" in flags))
            except AssertionError as error:
                errors.append(f"{mode}: {error}")
        far_result = subprocess.run([compiler, "-O2", "-S", str(FAR_JUMP), "-o", str(output)],
                                    env=clean_env, capture_output=True, text=True, timeout=60, check=False)
        if far_result.returncode:
            errors.append(f"far-jump compile failed: {far_result.stderr}")
        else:
            try:
                far_body = body(output.read_text(), "farjump")
                far_pattern = r"(?ms)^\s*jump\s+(\.L\w+),\s*t6\s*$.*?^\s*\.space\s+1048576\s*$.*?^\1:"
                if not re.search(far_pattern, far_body):
                    errors.append("far-jump fixture lacks a range-safe jump across its 1 MiB directive")
            except AssertionError as error:
                errors.append(f"far-jump fixture: {error}")
        if len(outputs) == len(modes):
            if outputs["-O2 LCCC_NO_PEEPHOLE"] != outputs["-O2 CCC_NO_PEEPHOLE"]:
                errors.append("the two peephole escape hatches produced different assembly")
            # No CFI function bounds: the whole unit must be the unoptimized
            # output, not merely the few fixture accesses that we count.
            if outputs["-O2 without unwind tables"] != outputs["-O2 without unwind tables, peephole disabled"]:
                errors.append("missing-CFI fallback differs from unoptimized output")
            def instructions(text):
                return sum(bool(re.match(r"^\s+[a-z][a-z0-9.]*\s", line)) or line.strip() == "ret"
                           for line in text.splitlines())
            optimized = instructions(body(outputs["-O2"], "ordinary"))
            raw = instructions(body(outputs["-O2 LCCC_NO_PEEPHOLE"], "ordinary"))
            if optimized >= raw:
                errors.append(f"ordinary nonvolatile function lost peephole optimization: {optimized} >= {raw}")
        if errors:
            for error in errors:
                print("FAIL:", error)
            return 1
        print(f"RV64 volatile assembly: PASS ({compiler}; {len(modes)} modes)")
        if not shutil.which(args.qemu) or not shutil.which(args.clang):
            message = "RV64 runtime: qemu-riscv64 or Clang unavailable"
            if args.require_qemu:
                print("FAIL:", message)
                return 1
            print("SKIP:", message)
            return 0
        try:
            linker = select_linker("riscv64", args.ld)
        except FileNotFoundError as error:
            if args.require_qemu:
                print("FAIL:", error)
                return 1
            print("SKIP:", error)
            return 0
        run_runtime(compiler, args.clang, args.qemu, linker, work, clean_env)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
