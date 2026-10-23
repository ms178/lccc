#!/usr/bin/env python3
"""Popcount-loop legality, native-instruction shapes, and freestanding runtimes.

--run requires every QEMU runner: unavailable execution is a failure, not a pass.
The default checks assembly and native objects on every supported architecture.
"""
import argparse
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
BIN = ROOT / "target/fastbuild"
if not __debug__:
    raise SystemExit("contract checks require assertions; disable Python -O/-OO")

SOURCE = r"""
__attribute__((noinline)) unsigned pc32(unsigned x, unsigned n) {
    while (x) { x &= x - 1; ++n; }
    return n;
}
__attribute__((noinline)) unsigned pc64(unsigned long long x) {
    unsigned n = 0;
    while (x) { x &= x - 1; ++n; }
    return n;
}
__attribute__((noinline)) unsigned long long pc64acc(unsigned long long x, unsigned long long n) {
    while (x) { x &= x - 1; ++n; }
    return n;
}
__attribute__((noinline)) unsigned guarded(unsigned x) {
    unsigned n = 0;
    if (x) do { x &= x - 1; ++n; } while (x);
    return n;
}
__attribute__((noinline)) unsigned wide_test(signed char x) {
    unsigned n = 0;
    int wide;
    if (!x) return 0;
    do {
        wide = (int)x & ((int)x - 1);
        x = (signed char)(unsigned char)wide;
        ++n;
    } while (wide);
    return n;
}
__attribute__((noinline)) int budget(unsigned x) {
    int left = 40;
    while (x) { x &= x - 1; --left; }
    return left;
}
__attribute__((noinline)) unsigned char narrow(unsigned long long x, unsigned char n) {
    while (x) { x &= x - 1; n++; }
    return n;
}
static unsigned reference(unsigned long long x) {
    unsigned n = 0;
    for (unsigned bit = 0; bit < 64; ++bit) n += (unsigned)((x >> bit) & 1);
    return n;
}
int test(void) {
    for (unsigned x = 0; x < 65536; ++x) {
        unsigned ref = reference(x);
        if (pc32(x, ~0u - 3) != ref + (~0u - 3) || guarded(x) != ref) return 1;
        if (budget(x) != 40 - (int)ref || narrow(x, 250) != (unsigned char)(250 + ref)) return 5;
    }
    for (unsigned bit = 0; bit < 64; ++bit) {
        unsigned long long x = 1ULL << bit;
        if (pc64(x) != 1 || pc64(~x) != 63 || pc64acc(~x, ~0ULL - 3) != 59) return 2;
    }
    for (int x = -128; x < 128; ++x) {
        if (wide_test((signed char)x) != reference((unsigned char)x) + (unsigned)(x < 0)) return 3;
    }
    if (pc64(0) != 0 || pc64(~0ULL) != 64) return 4;
    if (narrow(~0ULL, 255) != 63 || narrow(0, 7) != 7 || budget(~0u) != 8) return 6;
    return 0;
}
"""
START = {
    "x86": ".text\n.globl _start\n_start: call test\nmovl %eax,%edi\nmovl $60,%eax\nsyscall\n",
    "i686": ".text\n.globl _start\n_start: call test\nmovl %eax,%ebx\nmovl $1,%eax\nint $0x80\n",
    "arm": ".text\n.globl _start\n_start: bl test\nmov x8,#93\nsvc #0\n",
    "riscv": ".text\n.globl _start\n_start: call test\nli a7,93\necall\n",
}


def invoke(command, env=None):
    p = subprocess.run(list(map(str, command)), capture_output=True, text=True, timeout=120, env=env)
    if p.returncode or p.stderr.strip():
        raise AssertionError((command, p.returncode, p.stdout, p.stderr))
    return p.stdout


def body(assembly, name):
    match = re.search(rf"(?m)^{name}:.*?^\s*\.size\s+{name}\b", assembly, re.S)
    if not match:
        raise AssertionError(("missing function", name, assembly))
    return match.group()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run", action="store_true")
    args = parser.parse_args()
    # Explicit baselines make feature permissions independent of the build host.
    cases = [
        ("x86", ["-march=x86-64", "-mpopcnt"], True, None),
        ("x86", ["-march=x86-64", "-mno-popcnt"], False, None),
        ("i686", ["-mpopcnt"], True, "qemu-i386"),
        ("i686", ["-mno-popcnt"], False, "qemu-i386"),
        ("arm", [], True, "qemu-aarch64"),
        ("arm", ["-mgeneral-regs-only"], False, "qemu-aarch64"),
        ("riscv", [], False, "qemu-riscv64"),
    ]
    runs = shapes = 0
    with tempfile.TemporaryDirectory(prefix="lccc-popcount-loop-") as directory:
        w = Path(directory)
        source = w / "loop.c"
        source.write_text(SOURCE)
        for index, (arch, flags, native, runner) in enumerate(cases):
            compiler = Path(os.environ.get("CCC_" + arch.upper(), BIN / ("lccc-" + arch)))
            assembly, obj, binary = (w / f"case-{index}.{suffix}" for suffix in ("s", "o", "bin"))
            opts = ["-O2", "-ffreestanding", *flags]
            invoke([compiler, *opts, "-S", source, "-o", assembly])
            text = assembly.read_text()
            pattern = r"\bpopcnt[lq]?\b" if arch in ("x86", "i686") else r"\bcnt\b"
            if arch == "riscv":
                assert not re.search(r"\bcpopw?\b", text)
            else:
                for function in ("pc32", "pc64", "pc64acc", "guarded", "budget", "narrow"):
                    assert bool(re.search(pattern, body(text, function))) == native, (arch, flags, function, text)
                assert not re.search(pattern, body(text, "wide_test")), (arch, flags, text)
            # Sensitivity control: disabling loop_idiom must remove the native
            # shape. A runtime-only test could pass without folding any loop.
            if native:
                env = dict(os.environ, CCC_DISABLE_PASSES="loop_idiom")
                disabled = w / "disabled.s"
                invoke([compiler, *opts, "-S", source, "-o", disabled], env)
                assert not re.search(pattern, body(disabled.read_text(), "pc32")), (arch, "gate insensitive")
            invoke([compiler, *opts, "-c", source, "-o", obj])
            start = w / f"start-{arch}.s"
            start.write_text(START[arch])
            invoke([compiler, "-c", start, "-o", w / "start.o"])
            invoke([compiler, "-nostdlib", "-static", w / "start.o", obj, "-o", binary])
            shapes += 1
            if args.run:
                if runner and not shutil.which(runner):
                    raise AssertionError(f"--run requires {runner}")
                invoke(([runner] if runner else []) + [binary])
                runs += 1
    print(f"PASS: {shapes} target/feature shape+link cases; {runs} execution cases; native-fold sensitivity controls")


if __name__ == "__main__":
    main()
