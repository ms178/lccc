#!/usr/bin/env python3
"""Run self-checking switch, memory and codegen fixtures on AArch64/RV64 without a cross libc.

Requires LCCC target drivers, Clang's integrated assembler, a GNU target linker or LLD (including Rust's bundled LLD), and QEMU user emulators. printf/puts are intentionally
no-ops: every selected fixture checks its own result through main's exit code.
No output comparison or target-performance claim is made by this smoke gate.

Fixtures in LEVELS pin backend lowering, assembler and peephole contracts whose
defects showed only at some optimization levels; they run at each listed level
instead of toggling one IR pass at -O2, and are assembled by LCCC itself (`-c`)
rather than by Clang, so its integrated assembler is under test as well.
"""
import argparse
import os
from pathlib import Path
import subprocess
import shutil
import tempfile

ROOT = Path(__file__).resolve().parents[2]
FIXTURES = (
    "switch_i32_negative_jump_table", "switch_i64_min_jump_table",
    "switch_u32_high_jump_table", "switch_jump_table_dirty_abi",
    "switch_lut_bitpack", "switch_lut_boundaries", "switch_lut_default_slot",
    "switch_lut_narrow_index", "switch_lut_two_val_modulo",
    "narrow_big_endian_cross", "join_store_forward_cross",
    "conditional_store_readonly_cross", "strided_reduction_cross",
    "switch_i128_high_half", "switch_i128_wide_cases", "complex_float_return_cross",
    "divrem_pair_cross", "urem_by_one_cross", "va_arg_align16_cross",
    "overaligned_aggregate_arg_cross", "indirect_variadic_call_cross",
)
ALL_LEVELS = ("-O0", "-O1", "-O2", "-Os")
LEVELS = {
    # 128-bit selectors: the low half alone was compared (AArch64, RV64) and
    # wide case values were truncated to i64 (every target).
    "switch_i128_high_half": ALL_LEVELS,
    "switch_i128_wide_cases": ALL_LEVELS,
    # RV64 global dead-store census missing FP loads (-O2, -O3, -Os).
    "complex_float_return_cross": ALL_LEVELS,
    # AArch64: `mov x6, w3` from the fused div/rem pair, `and w0, w0, #0` for
    # `x % 1`, and the assembler refusing `and w2, w2, #-16` in va_arg.
    "divrem_pair_cross": ALL_LEVELS,
    "urem_by_one_cross": ALL_LEVELS,
    "va_arg_align16_cross": ALL_LEVELS,
    # AArch64 over-aligned alloca address staged in x17, the aggregate
    # argument base register: the copy faulted at -O1 and up.
    "overaligned_aggregate_arg_cross": ALL_LEVELS,
    # Member/element/cast callees of variadic type classified as fixed calls
    # (RV64: unnamed doubles in FP registers that va_arg never reads).
    "indirect_variadic_call_cross": ALL_LEVELS,
}
# Two-unit psABI fixtures under interop/: `<name>_callee.c` and
# `<name>_caller.c`, linked LCCC-with-LCCC and LCCC-with-Clang in both
# directions.  A caller and a callee that merely agree with each other pass
# the single-unit fixtures above while disagreeing with every other compiler.
INTEROP = ("psabi_pair_interop", "psabi_float_interop", "psabi_hfa_interop", "psabi_wide_interop")
INTEROP_LEVELS = ("-O0", "-O2")
CLANG_FREESTANDING = ("-O2", "-ffreestanding", "-fno-builtin", "-fno-pic", "-fno-pie",
                      "-fno-stack-protector", "-nostdlibinc")
# No linker relaxation for Clang's RV64 objects: the startup code sets no gp.
CLANG_TARGET_FLAGS = {"aarch64": (), "riscv64": ("-mno-relax",)}
CONTROL_PASSES = {
    "narrow_big_endian_cross": "byte_load",
    "join_store_forward_cross": "join_store_forward",
    "conditional_store_readonly_cross": "vectorize",
    "strided_reduction_cross": "vectorize",
}
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


def select_linker(arch, explicit=None):
    """These freestanding contracts need a linker, not cross libc/GCC.

    Prefer installed GNU tools, but retain execution coverage when optional
    distro cross packages cannot be provisioned. Rust's bundled LLD supports
    both targets and is available with the toolchain used to build LCCC.
    """
    if explicit:
        return [explicit] + (["-flavor", "gnu"] if Path(explicit).name == "rust-lld" else [])
    for name in (f"{arch}-linux-gnu-ld", "ld.lld"):
        if linker := shutil.which(name):
            return [linker]
    if rustc := shutil.which("rustc"):
        libdir = subprocess.check_output([rustc, "--print", "target-libdir"], text=True, timeout=10).strip()
        linker = Path(libdir).parent / "bin" / "rust-lld"
        if linker.is_file():
            return [str(linker), "-flavor", "gnu"]
    raise FileNotFoundError(f"No GNU/LLD linker for {arch}; install cross binutils or Rust's rust-lld")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lccc-dir", type=Path, default=ROOT / "target/fastbuild")
    parser.add_argument("--clang", default=os.environ.get("CLANG", "clang-23"))
    parser.add_argument("--ld", help="optional GNU/LLD linker supporting both target emulations")
    parser.add_argument("--qemu-dir", type=Path, default=Path("/usr/bin"))
    parser.add_argument("--only", action="append", metavar="FIXTURE",
                        help="run only this fixture (repeatable; single-unit or interop name)")
    parser.add_argument("--keep-going", action="store_true",
                        help="run every contract and report each failure instead of stopping at the first")
    args = parser.parse_args()
    known = set(FIXTURES) | set(INTEROP)
    if args.only and (unknown := sorted(set(args.only) - known)):
        parser.error(f"unknown fixture(s): {', '.join(unknown)}")
    selected = set(args.only or known)
    failures = []

    def attempt(label, steps, env=None):
        """Run each argv in `steps` in order; the first nonzero exit fails the contract."""
        for step in steps:
            status = subprocess.run(step, env=env, timeout=30).returncode
            if status:
                failures.append(f"{label}: {Path(step[0]).name} exited {status}")
                print(f"FAIL {label} ({failures[-1].split(': ', 1)[1]})", flush=True)
                if not args.keep_going:
                    raise SystemExit(1)
                return False
        print(f"PASS {label}", flush=True)
        return True

    version = subprocess.check_output([args.clang, "-dumpversion"], text=True).strip()
    if version != "23.1.5":
        parser.error(f"Clang 23.1.5 required, found {version}")
    resource = subprocess.check_output([args.clang, "-print-resource-dir"], text=True).strip()
    count = 0
    with tempfile.TemporaryDirectory(prefix="switch-cross-") as directory:
        work = Path(directory)
        for arch, driver, emulation in [
            ("aarch64", "lccc-arm", "aarch64linux"),
            ("riscv64", "lccc-riscv", "elf64lriscv"),
        ]:
            linker = select_linker(arch, args.ld)
            target = f"--target={arch}-linux-gnu"
            start = work / f"start-{arch}.s"
            start.write_text(START[arch])
            startup = start.with_suffix(".o")
            subprocess.run([args.clang, target, "-c", str(start), "-o", str(startup)], check=True, timeout=30)
            for fixture in (f for f in FIXTURES if f in selected):
                pass_name = CONTROL_PASSES.get(fixture, "switch_lut")
                if fixture in LEVELS:
                    variants = [(level, None, level) for level in LEVELS[fixture]]
                else:
                    variants = [("-O2", None, f"{pass_name}=on"),
                                ("-O2", pass_name, f"{pass_name}=off")]
                for level, disabled, label in variants:
                    env = os.environ.copy()
                    # Replace, rather than inherit, pass controls: a host's
                    # bisection environment must not silently erase coverage.
                    env.pop("CCC_DISABLE_PASSES", None)
                    if disabled:
                        env["CCC_DISABLE_PASSES"] = disabled
                    asm, obj, exe = (work / name for name in ["test.s", "test.o", "test"])
                    source = str(ROOT / "tests/regression" / f"{fixture}.c")
                    include = ["-isystem", str(Path(resource) / "include")]
                    if fixture in LEVELS:
                        # LCCC's own assembler and ELF writer, as `-c` uses
                        # them by default: it once refused `and w2, w2, #-16`.
                        steps = [[str(args.lccc_dir / driver), level, *include, "-c", source,
                                  "-o", str(obj)]]
                    else:
                        steps = [[str(args.lccc_dir / driver), level, *include, "-S", source,
                                  "-o", str(asm)],
                                 [args.clang, target, "-c", str(asm), "-o", str(obj)]]
                    steps += [[*linker, "-m", emulation, "-static", "-e", "_start",
                               str(startup), str(obj), "-o", str(exe)],
                              [str(args.qemu_dir / f"qemu-{arch}"), str(exe)]]
                    count += attempt(f"{arch} {fixture} {label}", steps, env)
            include = ["-isystem", str(Path(resource) / "include")]
            clang_flags = [target, *CLANG_FREESTANDING, *CLANG_TARGET_FLAGS[arch], *include]
            for fixture in (f for f in INTEROP if f in selected):
                base = ROOT / "tests/regression/interop" / fixture
                units = {}
                for unit in ("callee", "caller"):
                    source = str(base) + f"_{unit}.c"
                    units[unit, "clang"] = work / f"{unit}-clang.o"
                    # Validates the fixture itself: Clang must accept it.
                    subprocess.run([args.clang, *clang_flags, "-c", source, "-o", str(units[unit, "clang"])],
                                   check=True, timeout=30)
                    for level in INTEROP_LEVELS:
                        units[unit, level] = work / f"{unit}-lccc{level}.o"
                        units[unit, level].unlink(missing_ok=True)
                        subprocess.run([str(args.lccc_dir / driver), level, *include, "-c", source,
                                        "-o", str(units[unit, level])], timeout=30)
                for level in INTEROP_LEVELS:
                    for callee, caller in (((level), (level)), ((level), "clang"), ("clang", (level))):
                        exe = work / "interop"
                        label = f"callee={'clang' if callee == 'clang' else 'lccc' + callee} " \
                                f"caller={'clang' if caller == 'clang' else 'lccc' + caller}"
                        steps = [[*linker, "-m", emulation, "-static", "-e", "_start", str(startup),
                                  str(units["callee", callee]), str(units["caller", caller]),
                                  "-o", str(exe)],
                                 [str(args.qemu_dir / f"qemu-{arch}"), str(exe)]]
                        count += attempt(f"{arch} {fixture} {label}", steps)
    if failures:
        print(f"{len(failures)} cross-target execution contract(s) FAILED, {count} passed:")
        for failure in failures:
            print(f"  {failure}")
        return 1
    print(f"{count} cross-target execution contracts passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
