#!/usr/bin/env python3
"""Required compile/link/ISA contract; --run also requires and executes QEMU.

The ordinary fast gate needs no cross runtime. The explicit --run mode fails
if QEMU is missing: runtime coverage is never silently reported as a pass.
"""
import argparse
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile

# Assertions are the gate verdicts, not optional debugging checks. Refuse
# -O/-OO and PYTHONOPTIMIZE before any compiler or runtime can be invoked.
if not __debug__:
    raise SystemExit("AArch64 contract checks require Python assertions; disable -O/-OO and PYTHONOPTIMIZE")

ROOT = Path(__file__).resolve().parents[2]
CC = os.environ.get("CCC_ARM", str(ROOT / "target/fastbuild/lccc-arm"))
FIX = ROOT / "tests/regression/fixtures/arm_popcount"


def compile(args, good=True):
    p = subprocess.run([CC, *map(str, args)], capture_output=True, text=True, timeout=45)
    assert (p.returncode == 0 and not p.stderr.strip()) if good else (
        p.returncode > 0 and "-mgeneral-regs-only" in p.stderr), (args, p.returncode, p.stderr)
    return p

def check_user_asm(w):
    """Real C emission: ordinary, naked, file-scope and mixed compiler/user text."""
    bodies = {
        "ordinary": 'void f(void) { __asm__ volatile("movi v0.16b, #0" ::: "v0"); }',
        "naked": '__attribute__((naked)) void f(void) { __asm__("movi v0.16b, #0\\nret"); }',
        "toplevel": '__asm__(".text\\nf:\\nmovi v0.16b, #0\\nret");',
    }
    source, assembly = w / "user.c", w / "user.s"
    for name, body in bodies.items():
        source.write_text(body + "\n")
        for mode in ([], ["-mgeneral-regs-only"]):
            compile(["-O2", *mode, "-S", source, "-o", assembly])
            text = assembly.read_text()
            # One marker grammar across targets: #APP/#NO_APP (GAS special-
            # cases it everywhere; the native parser drops #-lines too).
            assert "#APP" in text and "#NO_APP" in text, (name, text)
            assert text.index("#APP") < text.index("movi v0.16b") < text.index("#NO_APP"), text
            compile(["-O2", *mode, "-c", source, "-o", w / "user.o"])
            gas = shutil.which("aarch64-linux-gnu-as")
            if gas:
                p = subprocess.run([gas, str(assembly), "-o", str(w / "gas.o")],
                                   capture_output=True, text=True, timeout=30)
                assert p.returncode == 0 and not p.stderr.strip(), (name, p.stderr)

        for adjacent in (body + '\ndouble bad(double x) { return x+1.; }',
                         'double bad(double x) { return x+1.; }\n' + body):
            source.write_text(adjacent + "\n")
            compile(["-O2", "-mgeneral-regs-only", "-S", source, "-o", assembly], False)
    # Input preparation and output writeback are not user-authored text.
    for body in (
        'void f(double *p) { __asm__ volatile("" :: "w"(*p)); }',
        'void f(double *p) { double x; __asm__ volatile("" : "=w"(x)); *p=x; }',
        'double f(double x) { __asm__ volatile("movi v0.16b, #0" ::: "v0"); return x+1.; }',
        'void f(double *p) { *p += 1.; __asm__ volatile("movi v0.16b, #0" ::: "v0"); }',
    ):
        source.write_text(body + "\n")
        compile(["-O2", "-mgeneral-regs-only", "-S", source, "-o", assembly], False)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run", action="store_true")
    args = parser.parse_args()
    if args.run and not shutil.which("qemu-aarch64"):
        parser.error("--run requires qemu-aarch64")
    execute = bool(args.run or shutil.which("qemu-aarch64"))
    runs = 0
    with tempfile.TemporaryDirectory(prefix="lccc-arm-popcount-") as td:
        w = Path(td)
        check_user_asm(w)
        for level in ["-O0", "-O1", "-O2", "-O3", "-Os"]:
            for restricted in [False, True]:
                flags = [level, "-ffreestanding"] + (["-mgeneral-regs-only"] if restricted else [])
                assembly = w / "count.s"
                compile([*flags, "-S", FIX / "count.c", "-o", assembly])
                text = assembly.read_text()
                if restricted:
                    assert not re.search(r"\b[vqdsbh][0-9]+\b", text), text
                    assert "mul w0, w0, w1" in text and "mul x0, x0, x1" in text
                else:
                    assert "cnt v0.8b" in text, "default NEON path changed"
                compile([*flags, "-c", FIX / "count.c", "-o", w / "count.o"])
                compile([*flags, "-c", FIX / "check.c", "-o", w / "check.o"])
                compile(["-c", FIX / "start.s", "-o", w / "start.o"])
                compile(["-nostdlib", w / "start.o", w / "count.o", w / "check.o", "-o", w / "test"])
                if execute:
                    p = subprocess.run(["qemu-aarch64", str(w / "test")], capture_output=True, timeout=60)
                    assert p.returncode == 0 and not p.stderr, (level, restricted, p.returncode, p.stderr)
                    runs += 1
        # Separate input-home regression: GP bits, FP homes, full SIMD lanes.
        # The same ten popcount executions remain mandatory above.
        start = w / "asm-start.s"
        start.write_text(".text\n.global _start\n_start: bl test\nmov x8,#93\nsvc #0\n")
        compile(["-c", start, "-o", w / "asm-start.o"])
        asm_runs = 0
        for level in ["-O0", "-O1", "-O2", "-O3", "-Os"]:
            compile([level, "-ffreestanding", "-c", FIX / "asm_inputs.c", "-o", w / "asm-inputs.o"])
            compile(["-nostdlib", w / "asm-start.o", w / "asm-inputs.o", "-o", w / "asm-inputs"])
            if execute:
                p = subprocess.run(["qemu-aarch64", str(w / "asm-inputs")], capture_output=True, timeout=60)
                assert p.returncode == 0 and not p.stderr, (level, p.returncode, p.stderr)
                asm_runs += 1
        # Sensitivity control: preserve SP but deliberately discard x17's
        # saved value. A fixture that never observes that operand must not pass.
        poisoned = w / "poisoned.s"
        compile(["-O0", "-ffreestanding", "-S", FIX / "asm_inputs.c", "-o", poisoned])
        text = poisoned.read_text()
        restore = "ldr x17, [sp], #16"
        assert restore in text, "fixed-register preservation path was not exercised"
        poisoned.write_text(text.replace(restore, "add sp, sp, #16"))
        compile(["-c", poisoned, "-o", w / "poisoned.o"])
        compile(["-nostdlib", w / "asm-start.o", w / "poisoned.o", "-o", w / "poisoned"])
        negative_runs = 0
        if execute:
            p = subprocess.run(["qemu-aarch64", str(w / "poisoned")], capture_output=True, timeout=60)
            assert p.returncode == 5 and not p.stderr, (p.returncode, p.stderr)
            negative_runs = 1
        print(f"PASS: user-asm input homes; {asm_runs} QEMU runs; {negative_runs} poisoned-x17 control")
        # Do not silently emit forbidden code for other FP/SIMD paths.
        source = w / "float.c"
        source.write_text("double f(double x) { return x+1.0; }\n")
        compile(["-O2", "-mgeneral-regs-only", "-S", source, "-o", w / "float.s"], False)
        # Register-like C symbols are addresses, not SIMD uses.
        source.write_text("extern int v0; int *address(void) { return &v0; }\n")
        compile(["-O2", "-mgeneral-regs-only", "-S", source, "-o", w / "symbol.s"])
    print(f"PASS: AArch64 popcount O0/O1/O2/O3/Os, default/restricted; FP/user-asm guard; {runs} QEMU runs (--run requires availability)")


if __name__ == "__main__":
    main()
