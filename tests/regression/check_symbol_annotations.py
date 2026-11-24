#!/usr/bin/env python3
"""Every symbol the compiler defines is typed and sized.

A symbol table entry says what a name is (`FUNC`, `OBJECT`, `TLS`) and how
many bytes it covers. Tools that walk an object by its symbols depend on
both: objtool refuses a function without a size ("is missing an ELF size
annotation") and validates nothing in it, `nm -S`, perf, valgrind and bloaty
attribute the bytes to no symbol or to the wrong one, and a debugger shows a
frame of the neighbouring function.

The functions and objects of the program had both. The compiler's own
additions did not:

  * the CPUID probe helper that `__builtin_cpu_supports` appends to a unit:
    18 cached words and the `ready` flag were untyped and zero-sized, and
    the initialiser `__lccc_cpu_init` was a function of size 0; under i686
    PIC its PC thunk was code after the end of every symbol;
  * the profile writer of `-fprofile-generate`: its paths and format strings
    are string literals under symbol names, and were untyped and zero-sized.

Found with objtool (`objtool --orc --dry-run` over the regression corpus,
lccc against GCC 16.2, see scripts/objtool_differential.py); this gate needs
readelf only.

For each probe, on each target that has it, every defined symbol that is no
section, no file and no assembler-local `.L` label (RISC-V keeps those in
the table for its PC-relative relocation pairs) must be

  * of type FUNC, OBJECT, TLS or IFUNC, and
  * of a size above zero.

The negative control is an assembly source that defines one function and one
object without `.type` and `.size`: the check has to report both.
"""

from __future__ import annotations

import argparse
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
FIXTURES = HERE / "fixtures" / "symbol_annotations"
TYPED = {"FUNC", "OBJECT", "TLS", "IFUNC"}

# (probe, source, [(target driver, flags)]). `ordinary.c` is portable C and
# goes to all four targets; the probe helper and the profile writer exist
# on x86 only.
X86_64, ARM, RISCV = "lccc", "lccc-arm", "lccc-riscv"
PROBES = [
    ("ordinary", "ordinary.c", [
        (X86_64, ["-O2"]), (X86_64, ["-O0"]), (X86_64, ["-O2", "-fPIC"]),
        (X86_64, ["-O2", "-m32"]), (X86_64, ["-O2", "-m32", "-fPIC"]),
        (ARM, ["-O2"]), (ARM, ["-O0"]), (RISCV, ["-O2"]), (RISCV, ["-O0"]),
    ]),
    ("cpu-probe", "cpu_probe.c", [
        (X86_64, ["-O2"]), (X86_64, ["-O0", "-fPIC"]), (X86_64, ["-O2", "-fno-pie"]),
        (X86_64, ["-O2", "-m32"]), (X86_64, ["-O2", "-m32", "-fPIC"]),
        (X86_64, ["-O2", "-m32", "-fPIC", "-funwind-tables"]),
        (X86_64, ["-O2", "-m32", "-fno-pie"]),
    ]),
    ("profile", "profile.c", [
        (X86_64, ["-O2", "-fprofile-generate"]), (X86_64, ["-O0", "-fprofile-generate"]),
    ]),
]
# The symbols a probe has to show, so that a helper which stopped being
# emitted (or was renamed) cannot pass by being absent.
MUST_DEFINE = {
    "cpu-probe": ["__lccc_cpu_init", "__lccc_cpu_ready", "__lccc_cpu_w7ebx"],
    "profile": ["__lccc_pgo_path_", "__lccc_pgo_hdr_"],
    "ordinary": ["dispatch", "table", "tls_exported", "weak_fn"],
}
PC_THUNK = "__lccc_cpu_pc_thunk"


def symbols(readelf: str, obj: Path) -> list[tuple[str, int, str, str]]:
    """(name, size, type, binding) of every defined symbol worth judging."""
    text = subprocess.run([readelf, "-sW", str(obj)], capture_output=True, text=True,
                          check=True).stdout
    out = []
    for line in text.splitlines():
        fields = line.split()
        if len(fields) < 8 or not fields[0].rstrip(":").isdigit():
            continue
        size, kind, binding, index, name = fields[2], fields[3], fields[4], fields[6], fields[7]
        if index in ("UND", "ABS", "COM") or kind in ("SECTION", "FILE"):
            continue
        if name.startswith((".L", "$")):  # assembler-local labels, Arm mapping symbols
            continue
        out.append((name, int(size, 0), kind, binding))
    return out


def unannotated(table: list[tuple[str, int, str, str]]) -> list[str]:
    problems = []
    for name, size, kind, _ in table:
        if kind not in TYPED:
            problems.append(f"{name}: type {kind}")
        elif size == 0:
            problems.append(f"{name}: {kind} of size 0")
    return problems


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--bindir", default=str(ROOT / "target/fastbuild"),
                        help="directory of lccc, lccc-arm and lccc-riscv")
    parser.add_argument("--readelf", default=shutil.which("readelf") or "readelf")
    args = parser.parse_args()
    bindir = Path(args.bindir)

    failures = []
    cells = judged = 0
    with tempfile.TemporaryDirectory(prefix="lccc-symbols-") as tmp:
        tmp = Path(tmp)
        # The check itself: both bare symbols of the control are reported.
        control = tmp / "untyped.o"
        subprocess.run([str(bindir / X86_64), "-c", str(FIXTURES / "untyped.S"), "-o",
                        str(control)], check=True, capture_output=True)
        found = unannotated(symbols(args.readelf, control))
        if sorted(found) != ["bare_function: type NOTYPE", "bare_object: type NOTYPE"]:
            failures.append(f"negative control: expected both bare symbols, got {found}")

        for probe, source, configs in PROBES:
            for driver, flags in configs:
                cells += 1
                cell = f"{probe} [{driver} {' '.join(flags)}]"
                obj = tmp / f"{probe}-{cells}.o"
                proc = subprocess.run(
                    [str(bindir / driver), *flags, "-c", str(FIXTURES / source), "-o", str(obj)],
                    capture_output=True, text=True, cwd=tmp, check=False)
                if proc.returncode != 0:
                    failures.append(f"{cell}: does not compile: {proc.stderr.strip()[:300]}")
                    continue
                table = symbols(args.readelf, obj)
                judged += len(table)
                names = [name for name, _, _, _ in table]
                for wanted in MUST_DEFINE[probe]:
                    if not any(name.startswith(wanted) for name in names):
                        failures.append(f"{cell}: defines no {wanted}*")
                # Position-independent i686 (the default, as executables
                # are PIE) is the one flavour with the PC thunk.
                thunk = "-m32" in flags and "-fno-pie" not in flags
                if probe == "cpu-probe" and (PC_THUNK in names) != thunk:
                    failures.append(f"{cell}: {PC_THUNK} is {'missing' if thunk else 'unexpected'}")
                failures += [f"{cell}: {problem}" for problem in unannotated(table)]

    for failure in failures:
        print(f"FAIL: {failure}")
    verdict = "FAIL" if failures else "PASS"
    print(f"symbol-annotations: {cells} objects, {judged} defined symbols typed and sized, "
          f"negative control reported, {verdict}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
