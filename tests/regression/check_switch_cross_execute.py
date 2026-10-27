#!/usr/bin/env python3
"""Run self-checking switch fixtures on AArch64/RV64 without a cross libc.

Requires LCCC target drivers, Clang's integrated assembler, a GNU linker with
both target emulations, and QEMU user emulators. printf/puts are intentionally
no-ops: every selected fixture checks its own result through main's exit code.
No output comparison or target-performance claim is made by this smoke gate.
"""
import argparse
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
FIXTURES = (
    "switch_i32_negative_jump_table", "switch_i64_min_jump_table",
    "switch_u32_high_jump_table", "switch_jump_table_dirty_abi",
    "switch_lut_bitpack", "switch_lut_boundaries", "switch_lut_default_slot",
    "switch_lut_narrow_index", "switch_lut_two_val_modulo",
)
START = {
    "aarch64": """.text
.globl _start
_start:
 bl main
 mov x8, #93
 svc #0
.globl abort
abort:
 brk #0
.globl printf
.globl puts
printf:
puts:
 mov w0, #0
 ret
""",
    "riscv64": """.text
.globl _start
_start:
 call main
 li a7, 93
 ecall
.globl abort
abort:
 ebreak
.globl printf
.globl puts
printf:
puts:
 li a0, 0
 ret
""",
}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lccc-dir", type=Path, default=ROOT / "target/fastbuild")
    parser.add_argument("--clang", default="clang-23")
    parser.add_argument("--ld", help="optional GNU linker supporting both target emulations")
    parser.add_argument("--qemu-dir", type=Path, default=Path("/usr/bin"))
    args = parser.parse_args()
    version = subprocess.check_output([args.clang, "-dumpversion"], text=True).strip()
    if version != "23.1.4":
        parser.error(f"Clang 23.1.4 required, found {version}")
    resource = subprocess.check_output([args.clang, "-print-resource-dir"], text=True).strip()
    count = 0
    with tempfile.TemporaryDirectory(prefix="switch-cross-") as directory:
        work = Path(directory)
        for arch, driver, emulation in [
            ("aarch64", "lccc-arm", "aarch64linux"),
            ("riscv64", "lccc-riscv", "elf64lriscv"),
        ]:
            target = f"--target={arch}-linux-gnu"
            start = work / f"start-{arch}.s"
            start.write_text(START[arch])
            startup = start.with_suffix(".o")
            subprocess.run([args.clang, target, "-c", str(start), "-o", str(startup)], check=True, timeout=30)
            for fixture in FIXTURES:
                for disable in [False, True]:
                    env = os.environ.copy()
                    # Replace, rather than inherit, pass controls: a host's
                    # bisection environment must not silently erase coverage.
                    env.pop("CCC_DISABLE_PASSES", None)
                    if disable:
                        env["CCC_DISABLE_PASSES"] = "switch_lut"
                    asm, obj, exe = (work / name for name in ["test.s", "test.o", "test"])
                    subprocess.run([str(args.lccc_dir / driver), "-O2", "-isystem", str(Path(resource) / "include"), "-S",
                                    str(ROOT / "tests/regression" / f"{fixture}.c"),
                                    "-o", str(asm)], env=env, check=True, timeout=30)
                    subprocess.run([args.clang, target, "-c", str(asm), "-o", str(obj)], check=True, timeout=30)
                    subprocess.run([args.ld or f"{arch}-linux-gnu-ld", "-m", emulation, "-static", "-e", "_start",
                                    str(startup), str(obj), "-o", str(exe)], check=True, timeout=30)
                    subprocess.run([str(args.qemu_dir / f"qemu-{arch}"), str(exe)],
                                   check=True, timeout=30)
                    count += 1
                    print(f"PASS {arch} {fixture} LUT={'off' if disable else 'on'}", flush=True)
    print(f"{count} cross-target execution contracts passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
