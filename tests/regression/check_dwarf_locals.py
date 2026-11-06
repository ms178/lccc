#!/usr/bin/env python3
"""D14: local variables with codegen-proven homes are debuggable.

The probe uses locals whose storage survives mem2reg (an array, an aggregate,
and a volatile object), assigns them, and asks GDB for their values before and
after the assignments.  A location that names the wrong slot, double-counts the
function base, or claims the object before its declaration fails the decoder or
the debugger.  `-g` must still leave every instruction byte unchanged.

Lexical-block ranges and promoted scalar locals are not yet described; the gate
pins the proven subset so it cannot regress while those are added.
"""
from __future__ import annotations

import argparse
import json
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tests/regression"))
from check_dwarf_quality import (  # noqa: E402
    DW_AT_LOCATION,
    DW_TAG_SUBPROGRAM,
    DW_TAG_VARIABLE,
    Report,
    check_parameter_location,
    children,
    code_sections,
    die_name,
    read_elf,
    units,
)
from check_dwarf_quality import self_test as quality_self_test  # noqa: E402

SOURCE = r"""
__attribute__((noinline)) int locals(int n) {
    int array[4] = {n, n + 1, n + 2, n + 3};
    struct pair { int a; long b; } s = {n + 10, n + 20};
    volatile int v = n + 30;
    /* before-update */
    array[0] += 1;
    s.a += 2;
    v += 3;
    int observed = array[0]; /* after-update */
    return observed + array[3] + s.a + (int)s.b + v;
}
int main(void) {
    return locals(7) == 8 + 10 + 19 + 27 + 40 ? 0 : 1;
}
"""

LOCALS = ("array", "s", "v")
BEFORE = {"array[0]": 7, "array[3]": 10, "s.a": 17, "s.b": 27, "v": 37}
AFTER = {"array[0]": 8, "array[3]": 10, "s.a": 19, "s.b": 27, "v": 40}


def run(argv: list[str], cwd: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(argv, cwd=cwd, text=True, capture_output=True, timeout=120)


def fail(label: str, detail: str) -> None:
    print(f"FAIL: {label}: {detail}")
    raise SystemExit(1)


def marker_line(marker: str) -> int:
    for number, line in enumerate(SOURCE.splitlines(), 1):
        if marker in line:
            return number
    raise ValueError(f"probe lost {marker}")


def gdb_number(text: str) -> int:
    token = text.strip().split()[0]
    return int(token, 16) if token.lower().startswith("0x") else int(token)


def gdb_values(gdb: str, exe: Path, cwd: Path, source_name: str) -> tuple[dict[str, str], dict[str, str]]:
    expressions = list(BEFORE)
    prints = "\n".join(f"print {expr}" for expr in expressions)
    script = f"""set pagination off
set confirm off
break {source_name}:{marker_line('/* before-update */')}
run
{prints}
break {source_name}:{marker_line('/* after-update */')}
continue
{prints}
quit
"""
    command_file = cwd / "locals.gdb"
    command_file.write_text(script)
    result = run([gdb, "-q", "-batch", "-x", str(command_file), str(exe)], cwd)
    if result.returncode != 0:
        fail("gdb", result.stdout + result.stderr)
    values = re.findall(r"\$\d+ = ([^\n]+)", result.stdout)
    if len(values) < 2 * len(expressions):
        fail("gdb", f"expected {2 * len(expressions)} printed values, got {result.stdout!r}")
    before = dict(zip(expressions, (value.strip() for value in values[:len(expressions)])))
    after = dict(zip(expressions, (value.strip() for value in values[len(expressions):])))
    return before, after


def check_locations(label: str, path: Path) -> None:
    elf = read_elf(path)
    report = Report()
    located: set[str] = set()
    for unit in units(elf):
        for subprogram in unit.by_tag(DW_TAG_SUBPROGRAM):
            if die_name(subprogram, unit) != "locals":
                continue
            for die in children(unit, subprogram):
                if die.tag != DW_TAG_VARIABLE or DW_AT_LOCATION not in die.attrs:
                    continue
                name = die_name(die, unit)
                located.add(name)
                check_parameter_location(report, elf, unit, die, subprogram, label, name)
    if report.failures:
        fail(label, "; ".join(str(failure) for failure in report.failures))
    missing = set(LOCALS) - located
    if missing:
        fail(label, f"locals without a proven home: {sorted(missing)}")


def check_mode(lccc: str, gcc: str, gdb: str | None, bits: int, version: int, root: Path) -> int:
    label = f"i686-v{version}" if bits == 32 else f"x86-64-v{version}"
    work = root / label.replace("-", "_")
    work.mkdir()
    source = work / "locals.c"
    source.write_text(SOURCE)
    target = [lccc] + (["-m32"] if bits == 32 else [])
    common = ["-O0", f"-gdwarf-{version}", "-g"]
    obj = work / "direct.o"
    result = run(target + common + ["-c", str(source), "-o", str(obj)], work)
    if result.returncode != 0:
        fail(label, result.stderr)
    check_locations(label, obj)
    exe = work / "direct"
    result = run([gcc, "-m32" if bits == 32 else "-m64", "-no-pie", str(obj), "-o", str(exe)], work)
    if result.returncode != 0:
        fail(label + "/link", result.stderr)
    result = run([str(exe)], work)
    if result.returncode != 0:
        fail(label + "/run", f"exit {result.returncode}")

    asm = work / "via.s"
    via_obj = work / "via.o"
    result = run(target + common + ["-S", str(source), "-o", str(asm)], work)
    if result.returncode != 0:
        fail(label + "/-S", result.stderr)
    result = run([gcc, "-m32" if bits == 32 else "-m64", "-c", str(asm), "-o", str(via_obj)], work)
    if result.returncode != 0:
        fail(label + "/gas", result.stderr)
    check_locations(label + "/gas", via_obj)
    via_exe = work / "via"
    result = run([gcc, "-m32" if bits == 32 else "-m64", "-no-pie", str(via_obj), "-o", str(via_exe)], work)
    if result.returncode != 0:
        fail(label + "/via-link", result.stderr)

    if gdb:
        for candidate in (exe, via_exe):
            before, after = gdb_values(gdb, candidate, work, source.name)
            for expr, expected in BEFORE.items():
                if gdb_number(before[expr]) != expected:
                    fail(label + "/gdb-before", f"{expr} = {before[expr]}, expected {expected}")
            for expr, expected in AFTER.items():
                if gdb_number(after[expr]) != expected:
                    fail(label + "/gdb-after", f"{expr} = {after[expr]}, expected {expected}")

    for level in ("-O0", "-O2"):
        plain = work / f"plain{level}.o"
        debug = work / f"debug{level}.o"
        for output, extra in ((plain, []), (debug, ["-g"])):
            result = run(target + [level] + extra + ["-c", str(source), "-o", str(output)], work)
            if result.returncode != 0:
                fail(label + level, result.stderr)
        if code_sections(read_elf(plain)) != code_sections(read_elf(debug)):
            fail(label + level, "-g changed code bytes; debug info must be free at runtime")
    return 4 if gdb else 3


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lccc")
    parser.add_argument("--gcc", default="gcc-16")
    parser.add_argument("--gdb", default=None)
    parser.add_argument("--allow-missing-gdb", action="store_true")
    parser.add_argument("--json", type=Path)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        if quality_self_test() != 0:
            return 1
        assert marker_line("/* before-update */") < marker_line("/* after-update */")
        assert gdb_number("0x10") == 16
        print("local gate self-test: PASS")
        return 0
    if not args.lccc:
        fail("arguments", "--lccc is required unless --self-test is used")
    args.lccc = str(Path(args.lccc).resolve())
    args.gcc = str(Path(args.gcc).resolve()) if Path(args.gcc).exists() else args.gcc
    gdb = args.gdb or shutil.which("gdb")
    if gdb:
        gdb = str(Path(gdb).resolve()) if Path(gdb).exists() else gdb
    if not gdb and not args.allow_missing_gdb:
        fail("toolchain", "gdb is required (or pass --allow-missing-gdb for structural-only checks)")
    contracts = 0
    with tempfile.TemporaryDirectory(prefix="lccc-dwarf-locals-") as directory:
        root = Path(directory)
        for bits in (64, 32):
            for version in (2, 3, 4, 5):
                contracts += check_mode(args.lccc, args.gcc, gdb, bits, version, root)
    report = {"status": "pass", "contracts": contracts, "gdb": gdb,
              "locals": list(LOCALS), "before": BEFORE, "after": AFTER}
    if args.json:
        args.json.write_text(json.dumps(report, indent=2) + "\n")
    print(f"PASS: DWARF local homes ({contracts} contracts{'; GDB ' + gdb if gdb else ''})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
