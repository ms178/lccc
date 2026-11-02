#!/usr/bin/env python3
"""RWX LOAD-segment diagnostics across ELF32/ELF64 and every link driver path.

The contract is GNU ld's:

* a writable+executable PT_LOAD warns by default;
* ``--fatal-warnings`` turns that warning into a failed link but retains the
  completed ELF output;
* ``--no-warn-rwx-segments`` suppresses the diagnostic and performs no layout
  change (byte-identical output);
* the last toggle wins;
* ``--oformat=binary`` is checked *before* the program headers are destroyed.

When a bfd oracle is available the warning/fatal verdict and the loadable bytes
are compared against it. The test never silently skips the lccc assertions.
AArch64/RISC-V shared links prove the common driver plumbing; when qemu-user is
available a freestanding static smoke also proves cross codegen/link/runtime.
"""

from __future__ import annotations

import argparse
import pathlib
import shutil
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[2]

START_ASM = """.text
.globl _start
_start:
    ret
.section .note.GNU-stack,"",@progbits
"""

RWX_SCRIPT = """ENTRY(_start)
PHDRS { all PT_LOAD FLAGS(7); }
SECTIONS {
    . = 0x400000;
    .text : { *(.text) } :all
}
"""

SPLIT_SCRIPT = """ENTRY(_start)
PHDRS { text PT_LOAD FLAGS(5); }
SECTIONS {
    . = 0x400000;
    .text : { *(.text) } :text
}
"""


def run(cmd: list[object], expect: int = 0) -> subprocess.CompletedProcess[str]:
    argv = [str(part) for part in cmd]
    result = subprocess.run(argv, capture_output=True, text=True)
    if result.returncode != expect:
        raise AssertionError(
            f"command exited {result.returncode}, expected {expect}: {argv}\n"
            f"stdout:\n{result.stdout}\nstderr:\n{result.stderr}"
        )
    return result


def has_rwx_warning(result: subprocess.CompletedProcess[str]) -> bool:
    return "RWX permissions" in result.stderr


def loadable_bytes(objcopy: str | None, elf: pathlib.Path,
                   dest: pathlib.Path) -> bytes | None:
    if objcopy is None:
        return None
    run([objcopy, "-O", "binary", elf, dest])
    return dest.read_bytes()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lccc", type=pathlib.Path,
                        default=ROOT / "target/fastbuild/lccc")
    parser.add_argument("--ld", type=pathlib.Path,
                        default=ROOT / "target/fastbuild/lccc-ld")
    parser.add_argument("--bfd", default="ld.bfd",
                        help="bfd oracle (empty string disables comparison)")
    args = parser.parse_args()

    lccc = args.lccc.resolve()
    ld = args.ld.resolve()
    if not lccc.is_file() or not ld.is_file():
        print(f"FAIL: build lccc and lccc-ld first ({lccc}, {ld})", file=sys.stderr)
        return 1
    bfd = shutil.which(args.bfd) if args.bfd else None
    objcopy = shutil.which("objcopy")

    with tempfile.TemporaryDirectory(prefix="lccc-rwx-") as td:
        tmp = pathlib.Path(td)
        src = tmp / "start.S"
        src.write_text(START_ASM)
        (tmp / "rwx.ld").write_text(RWX_SCRIPT)
        (tmp / "split.ld").write_text(SPLIT_SCRIPT)

        for bits in (64, 32):
            emulation = "elf_x86_64" if bits == 64 else "elf_i386"
            obj = tmp / f"start{bits}.o"
            run([lccc, f"-m{bits}", "-c", src, "-o", obj])

            for script_name, unsafe in (("rwx.ld", True), ("split.ld", False)):
                script = tmp / script_name
                out = tmp / f"{bits}-{script_name}.elf"
                base = [ld, "-m", emulation, "-T", script, obj]

                default = run([*base, "-o", out])
                assert has_rwx_warning(default) == unsafe, default.stderr
                good = out.read_bytes()

                fatal = run([*base, "--fatal-warnings", "-o", out],
                            expect=1 if unsafe else 0)
                assert has_rwx_warning(fatal) == unsafe, fatal.stderr
                assert out.read_bytes() == good, "fatal warning changed output bytes"

                quiet = run([*base, "--fatal-warnings", "--no-warn-rwx-segments",
                             "-o", out])
                assert not quiet.stderr, quiet.stderr
                assert out.read_bytes() == good

                enabled = run([*base, "--no-warn-rwx-segments",
                               "--warn-rwx-segments", "-o", out])
                assert has_rwx_warning(enabled) == unsafe, enabled.stderr

                disabled = run([*base, "--warn-rwx-segments",
                                "--no-warn-rwx-segments", "--fatal-warnings",
                                "-o", out])
                assert not disabled.stderr, disabled.stderr
                assert out.read_bytes() == good

                if unsafe:
                    flat = tmp / f"{bits}-flat.bin"
                    run([*base, "--no-warn-rwx-segments", "--oformat=binary",
                         "-o", flat])
                    assert flat.read_bytes() == b"\xc3", flat.read_bytes()
                    run([*base, "--fatal-warnings", "--oformat=binary", "-o", flat],
                        expect=1)
                    assert flat.read_bytes()[:4] == b"\x7fELF", (
                        "fatal RWX diagnostic must run before flat conversion"
                    )

                # Compiler-driver smoke. The internal driver uses its own
                # default layout (it does not route -Wl,-T through the
                # standalone script linker), so the verdict is measured rather
                # than assumed: if that layout is RWX the fatal link must fail;
                # if it is split the link must stay quiet. Suppression must
                # always be byte-identical. This proves -Wl, forwarding reaches
                # the shared diagnostic without depending on driver script
                # support.
                for shared in (False, True):
                    suffix = "so" if shared else "elf"
                    driver_out = tmp / f"{bits}-driver-{script_name}.{suffix}"
                    driver_base = [lccc, f"-m{bits}", "-nostdlib", obj, "-o", driver_out]
                    if shared:
                        driver_base.insert(2, "-shared")
                    default_driver = run(driver_base)
                    driver_unsafe = has_rwx_warning(default_driver)
                    driver_bytes = driver_out.read_bytes()
                    run([*driver_base, "-Wl,--fatal-warnings"],
                        expect=1 if driver_unsafe else 0)
                    quiet_driver = run([*driver_base, "-Wl,--fatal-warnings,--no-warn-rwx-segments"])
                    assert not quiet_driver.stderr, quiet_driver.stderr
                    assert driver_out.read_bytes() == driver_bytes

                if bfd:
                    bo = tmp / f"{bits}-bfd.elf"
                    bc = [bfd, "-m", emulation, "-T", script, obj]
                    bfd_fatal = run([*bc, "--fatal-warnings", "-o", bo],
                                    expect=1 if unsafe else 0)
                    assert has_rwx_warning(bfd_fatal) == unsafe, bfd_fatal.stderr
                    run([*bc, "--no-warn-rwx-segments", "--fatal-warnings", "-o", bo])
                    lccc_load = loadable_bytes(objcopy, out, tmp / "lccc.load")
                    bfd_load = loadable_bytes(objcopy, bo, tmp / "bfd.load")
                    if lccc_load is not None and bfd_load is not None:
                        assert lccc_load == bfd_load, "loadable bytes differ from bfd"

            print(f"PASS ELF{bits}: default/fatal/suppression/last-wins/flat/driver"
                  + (" + bfd" if bfd else " (bfd unavailable)"))

        # Cross-architecture driver plumbing. The script linker is x86-only,
        # so the RWX warning matrix above cannot force a cross RWX layout;
        # these shared links prove the common diagnostic is reached on AArch64
        # and RISC-V, obeys the measured layout, and suppression is byte-exact.
        cross_src = tmp / "cross.c"
        cross_src.write_text("int f(int x) { return x + 1; }\n")
        for target in ("arm", "riscv"):
            cross_cc = ROOT / f"target/fastbuild/lccc-{target}"
            if not cross_cc.is_file():
                print("SKIP cross {target}: lccc-{target} not built")
                continue
            obj = tmp / f"{target}.o"
            out = tmp / f"{target}.so"
            run([cross_cc, "-fPIC", "-c", cross_src, "-o", obj])
            base = [cross_cc, "-shared", "-nostdlib", obj, "-o", out]
            default = run(base)
            unsafe = has_rwx_warning(default)
            good = out.read_bytes()
            run([*base, "-Wl,--fatal-warnings"], expect=1 if unsafe else 0)
            quiet = run([*base, "-Wl,--fatal-warnings,--no-warn-rwx-segments"])
            assert not quiet.stderr, quiet.stderr
            assert out.read_bytes() == good
            print(f"PASS {target}: shared driver RWX plumbing and suppression")

            qemu_name = "aarch64" if target == "arm" else "riscv64"
            qemu = shutil.which(f"qemu-{qemu_name}")
            if qemu:
                start_asm = tmp / f"{target}-start.S"
                if target == "arm":
                    start_asm.write_text(
                        ".global _start\n_start:\n"
                        "    mov x0, #41\n    bl f\n    cmp x0, #42\n    b.ne 1f\n"
                        "    mov x8, #93\n    mov x0, #0\n    svc #0\n"
                        "1:  mov x8, #93\n    mov x0, #1\n    svc #0\n")
                else:
                    start_asm.write_text(
                        ".global _start\n_start:\n"
                        "    li a0, 41\n    call f\n    li t0, 42\n    bne a0, t0, 1f\n"
                        "    li a7, 93\n    li a0, 0\n    ecall\n"
                        "1:  li a7, 93\n    li a0, 1\n    ecall\n")
                sobj = tmp / f"{target}-static.o"
                start_obj = tmp / f"{target}-start.o"
                smoke = tmp / f"{target}-smoke"
                run([cross_cc, "-c", cross_src, "-o", sobj])
                run([cross_cc, "-c", start_asm, "-o", start_obj])
                run([cross_cc, "-nostdlib", "-static", start_obj, sobj, "-o", smoke])
                run([qemu, smoke])
                print(f"PASS {target}: qemu-{qemu_name} freestanding static runtime")
            else:
                print(f"SKIP {target}: qemu-{qemu_name} runtime (not installed)")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
