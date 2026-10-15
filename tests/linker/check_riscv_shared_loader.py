#!/usr/bin/env python3
"""Real RV64 glibc-loader check: data and GOT pointers, with executable overrides.

Supplemental to check_riscv_shared_binding.py's ELF/loader-model matrix. Needs
an explicitly selected cross Clang, ELF linker, QEMU and RV64 glibc sysroot.
No skip/fallback or target-hardware performance claim. The host linker builds
only the test executable; the selected LCCC builds every DSO under test.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile

from check_riscv_relocation_contract import run

CASES = [
    ("default", "int target=42;", "address"),
    ("weak-definition", "__attribute__((weak)) int target=42;", "address"),
    ("import", "extern int target;", "address"),
    ("weak-import", "extern __attribute__((weak)) int target;", "address"),
    ("weak-missing", "extern __attribute__((weak)) int target;", "missing"),
    ("hidden", '__attribute__((visibility("hidden"))) int target=42;', "value"),
    ("protected", '__attribute__((visibility("protected"))) int target=42;', "value"),
    ("local", "static int target=42;", "value"),
    ("weak-hidden-missing", 'extern __attribute__((weak,visibility("hidden"))) int target;', "zero"),
    ("weak-protected-missing", 'extern __attribute__((weak,visibility("protected"))) int target;', "zero"),
    ("absolute-default", "extern int target;", "address"),
    ("absolute-weak", "extern int target;", "address"),
    ("absolute-protected", "extern int target;", "absolute"),
]


def checked(command, commands):
    entry = dict(argv=command)
    commands.append(entry)
    rc, output = run(command)
    entry.update(returncode=rc, output=output)
    if rc:
        raise ValueError(f"command failed ({rc}): {command}: {output}")


def runner(expect):
    if expect == "address":
        check = "la s1,target\n bne s0,s1,.fail"
    elif expect == "value":
        check = "lw t1,0(s0)\n li t2,42\n bne t1,t2,.fail"
    else:
        check = f"li t1,{256 if expect == 'absolute' else 0}\n bne s0,t1,.fail"
    provider = "" if expect == "missing" else ".data\n.globl target\ntarget: .word 99\n"
    return f""".option pic
.text
.globl _start
_start:
 la t0,observed
 ld s0,0(t0)
 {check}
 la t0,get_ptr
 jalr ra,t0,0
 bne a0,s0,.fail
 li a0,0
 j .exit
.fail:
 li a0,91
.exit:
 li a7,93
 ecall
{provider}"""


def measure(args, report):
    for name in ("lccc", "clang", "linker", "qemu"):
        command = getattr(args, name)
        resolved = shutil.which(command)
        if not resolved:
            raise ValueError(f"missing {name}: {command}")
        rc, banner = run([command, "--version"])
        if rc or not banner.strip():
            raise ValueError(f"failed version probe: {name}")
        report["tools"][name] = dict(path=str(Path(resolved).resolve()), version=banner,
            sha256=hashlib.sha256(Path(resolved).read_bytes()).hexdigest())
    loader = args.sysroot / "lib/ld-linux-riscv64-lp64d.so.1"
    report["loader"] = dict(path=str(loader), sha256=hashlib.sha256(loader.read_bytes()).hexdigest())
    with tempfile.TemporaryDirectory(prefix="rv-shared-loader-") as tmp:
        root = Path(tmp)
        for name, declaration, expect in CASES:
            commands = []
            row = dict(case=name, passed=False, commands=commands)
            report["records"].append(row)
            try:
                source = root / "shared.c"
                source.write_text(declaration + "\nint *observed=&target;\nint *get_ptr(void){return &target;}\n")
                asm = root / "runner.s"
                asm.write_text(runner(expect))
                row["source"] = source.read_text()
                row["runner"] = asm.read_text()
                obj, so, runner_obj, exe = [root / n for n in ("shared.o", "shared.so", "runner.o", "runner")]
                for stale in (obj, so, runner_obj, exe):
                    stale.unlink(missing_ok=True)
                cc = [args.clang, "--target=riscv64-linux-gnu", "-march=rv64gc", "-mabi=lp64d"]
                checked(cc + ["-O2", "-fPIC", "-c", str(source), "-o", str(obj)], commands)
                objects = [str(obj)]
                if name.startswith("absolute-"):
                    # A separate definition keeps the use-side relocations
                    # symbolic with addend zero. Same-TU inline .set is folded
                    # by LLVM into a different, sym0+256 GOT experiment.
                    definition = root / "absolute.s"
                    qualifiers = ".weak target" if name == "absolute-weak" else ".globl target"
                    if name == "absolute-protected":
                        qualifiers += "\n.protected target"
                    definition.write_text(qualifiers + "\n.set target,256\n")
                    row["definition"] = definition.read_text()
                    absolute_obj = root / "absolute.o"
                    absolute_obj.unlink(missing_ok=True)
                    checked(cc + ["-c", str(definition), "-o", str(absolute_obj)], commands)
                    objects.append(str(absolute_obj))
                checked([args.lccc, "-shared", "-nostdlib", *objects, "-o", str(so)], commands)
                checked(cc + ["-c", str(asm), "-o", str(runner_obj)], commands)
                checked([args.linker, "-m", "elf64lriscv", "-pie", "-E", "--dynamic-linker",
                         "/lib/ld-linux-riscv64-lp64d.so.1", str(runner_obj), str(so), "-o", str(exe)], commands)
                row["file_sha256"] = {p.name: hashlib.sha256(p.read_bytes()).hexdigest()
                                      for p in (obj, so, runner_obj, exe)}
                checked([args.qemu, "-L", str(args.sysroot), str(exe)], commands)
                row["passed"] = True
            except (AssertionError, OSError, ValueError, subprocess.TimeoutExpired, struct.error) as error:
                row["error"] = str(error)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--lccc", required=True)
    ap.add_argument("--clang", required=True)
    ap.add_argument("--linker", required=True)
    ap.add_argument("--qemu", required=True)
    ap.add_argument("--sysroot", type=Path, required=True)
    ap.add_argument("--json", type=Path, required=True)
    args = ap.parse_args()
    report = dict(tools={}, records=[])
    try:
        measure(args, report)
    except (AssertionError, OSError, ValueError, subprocess.TimeoutExpired) as error:
        report["records"].append(dict(case="infrastructure", passed=False, error=str(error)))
    report.update(passed=sum(r["passed"] for r in report["records"]),
                  failed=sum(not r["passed"] for r in report["records"]), skipped=0)
    args.json.parent.mkdir(parents=True, exist_ok=True)
    args.json.write_text(json.dumps(report, indent=2) + "\n")
    for row in report["records"]:
        if not row["passed"]:
            print("FAIL", row["case"], row["error"])
    print(f"RV64 shared loader: {report['passed']} PASS / {report['failed']} FAIL / 0 SKIP")
    return int(bool(report["failed"]))


if __name__ == "__main__":
    raise SystemExit(main())
