#!/usr/bin/env python3
"""D14: local variables with codegen-proven homes are debuggable.

The probe uses locals whose storage survives mem2reg (an array, an aggregate,
and a volatile object), assigns them, and asks GDB for their values before and
after the assignments.  A location that names the wrong slot, double-counts the
function base, or claims the object before its declaration fails the decoder or
the debugger.  `-g` must still leave every instruction byte unchanged.

A second probe covers the two shapes where the description used to be lost
outright rather than merely incomplete: a GNU nested function (whose record was
built without its parameter map, scopes and locals) and a record type defined
inside a body (whose layout was popped with the block, leaving an incomplete
declaration of size zero).  Both are ordinary C a user can write today.

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
    DW_AT_BYTE_SIZE,
    DW_AT_DECLARATION,
    DW_AT_LOCATION,
    DW_AT_LOW_PC,
    DW_AT_TYPE,
    DW_TAG_FORMAL_PARAMETER,
    DW_TAG_LEXICAL_BLOCK,
    DW_TAG_MEMBER,
    DW_TAG_STRUCTURE_TYPE,
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
volatile int shadow_sink;
__attribute__((noinline)) int shadow(int n) {
    volatile int x = n + 1;
    {
        volatile int x = n + 10;
        shadow_sink = x; /* shadow-inner */
        x += 1;
    }
    shadow_sink = x; /* shadow-outer */
    return x;
}
int main(void) {
    return locals(7) == 8 + 10 + 19 + 27 + 40 && shadow(7) == 8 ? 0 : 1;
}
"""

#: A GNU nested function that also declares a block-scope record type.  Both
#: descriptions were once lost: the nested function's record was finalized
#: without its parameter map, its lexical scopes and its locals, and the record's
#: layout was popped when the block closed, so `pair`'s type reached the writer
#: as a name it no longer knew.
SCOPED = r"""
volatile int scoped_sink;
__attribute__((noinline)) int outer(int n) {
    int inner(int k) {
        volatile int m = k * n;
        struct { int a, b; } pair = { m, n };
        return pair.a + pair.b; /* nested-inner */
    }
    int t = inner(3);
    scoped_sink = t;
    return t + n;
}
int main(void) { return outer(4) == 20 ? 0 : 1; }
"""

#: The nested function's mangled name, which is also its `LOCAL` symbol.
NESTED = "outer.inner"
#: `outer(4)` calls `inner(3)` with `n = 4`: `m = 12` and `pair = {12, 4}`.
SCOPED_VALUES = {"k": 3, "m": 12, "pair.a": 12, "pair.b": 4}

LOCALS = ("array", "s", "v")
SHADOW_INNER = 17
SHADOW_OUTER = 8
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


def gdb_shadow(gdb: str, exe: Path, cwd: Path, source_name: str) -> tuple[str, str]:
    script = f"""set pagination off
set confirm off
break {source_name}:{marker_line('/* shadow-inner */')}
run
print x
break {source_name}:{marker_line('/* shadow-outer */')}
continue
print x
quit
"""
    command_file = cwd / "shadow.gdb"
    command_file.write_text(script)
    result = run([gdb, "-q", "-batch", "-x", str(command_file), str(exe)], cwd)
    if result.returncode != 0:
        fail("gdb-shadow", result.stdout + result.stderr)
    values = re.findall(r"\$\d+ = ([^\n]+)", result.stdout)
    if len(values) < 2:
        fail("gdb-shadow", f"expected two printed values, got {result.stdout!r}")
    return values[0].strip(), values[1].strip()


def scoped_marker_line(marker: str = "/* nested-inner */") -> int:
    for number, line in enumerate(SCOPED.splitlines(), 1):
        if marker in line:
            return number
    raise ValueError(f"scoped probe lost {marker}")


def gdb_scoped(gdb: str, exe: Path, cwd: Path, source_name: str) -> dict[str, str]:
    expressions = list(SCOPED_VALUES)
    prints = "\n".join(f"print {expr}" for expr in expressions)
    script = f"""set pagination off
set confirm off
break {source_name}:{scoped_marker_line()}
run
{prints}
quit
"""
    command_file = cwd / "scoped.gdb"
    command_file.write_text(script)
    result = run([gdb, "-q", "-batch", "-x", str(command_file), str(exe)], cwd)
    if result.returncode != 0:
        fail("gdb-scoped", result.stdout + result.stderr)
    values = re.findall(r"\$\d+ = ([^\n]+)", result.stdout)
    if len(values) < len(expressions):
        fail("gdb-scoped", f"expected {len(expressions)} printed values, got {result.stdout!r}")
    return dict(zip(expressions, (value.strip() for value in values)))


def check_scoped(label: str, path: Path) -> None:
    """A GNU nested function and a block-scope record are both described.

    Neither is reachable from the file-scope probe, and both failed closed in a
    way the DWARF itself cannot show: a nested function's record used to be
    finalized without its parameter map, scopes and locals, so `outer.inner`
    claimed to take no arguments; and a record defined inside a body lost its
    layout when the block closed, so `pair`'s type was written as
    `DW_AT_declaration` with a zero size -- gdb prints `<incomplete type>` and
    no member can be read.

    The subprogram is found at depth 1 because that is where this writer puts
    it: the nested function is a sibling of its lexical parent, named by the
    mangled symbol it is emitted under.
    """
    elf = read_elf(path)
    report = Report()
    described = False
    for unit in units(elf):
        nested = [
            die
            for die in unit.dies
            if die.tag == DW_TAG_SUBPROGRAM
            and die.depth == 1
            and die_name(die, unit) == NESTED
        ]
        if not nested:
            continue
        described = True
        subprogram = nested[0]
        if DW_AT_LOW_PC not in subprogram.attrs:
            fail(f"{label}/nested", f"{NESTED} states no code range")
        kids = children(unit, subprogram)
        parameter = next(
            (die for die in kids
             if die.tag == DW_TAG_FORMAL_PARAMETER and die_name(die, unit) == "k"),
            None,
        )
        if parameter is None:
            fail(
                f"{label}/nested",
                f"{NESTED} describes no `DW_TAG_formal_parameter k`; a debugger "
                "cannot name its argument",
            )
        if DW_AT_LOCATION not in parameter.attrs:
            fail(f"{label}/nested", f"{NESTED}'s parameter k has no DW_AT_location")
        check_parameter_location(
            report, elf, unit, parameter, subprogram, label, f"{NESTED}.k"
        )
        for name in ("m", "pair"):
            die = next(
                (candidate for candidate in kids
                 if candidate.tag == DW_TAG_VARIABLE and die_name(candidate, unit) == name),
                None,
            )
            if die is None:
                fail(f"{label}/nested", f"{NESTED} describes no local `{name}`")
            if DW_AT_LOCATION not in die.attrs:
                fail(f"{label}/nested", f"{NESTED}'s local {name} has no DW_AT_location")
            check_parameter_location(
                report, elf, unit, die, subprogram, label, f"{NESTED}.{name}"
            )
        pair = next(
            (candidate for candidate in kids
             if candidate.tag == DW_TAG_VARIABLE and die_name(candidate, unit) == "pair"),
            None,
        )
        type_offset = pair.attrs.get(DW_AT_TYPE) if pair is not None else None
        record = unit.die_at(type_offset) if isinstance(type_offset, int) else None
        if record is None:
            fail(f"{label}/nested", f"{NESTED}'s local pair names no type DIE")
        if record.tag != DW_TAG_STRUCTURE_TYPE:
            fail(f"{label}/nested", f"pair's type is tag {record.tag:#x}, not a structure")
        if DW_AT_DECLARATION in record.attrs:
            fail(
                f"{label}/nested",
                "pair's block-scope struct type is described as a declaration, so "
                "the object cannot be inspected",
            )
        size = record.attrs.get(DW_AT_BYTE_SIZE)
        if size != 8:
            fail(f"{label}/nested", f"pair's struct says DW_AT_byte_size {size}, expected 8")
        members = sorted(
            die_name(die, unit)
            for die in children(unit, record)
            if die.tag == DW_TAG_MEMBER
        )
        if members != ["a", "b"]:
            fail(f"{label}/nested", f"pair's struct members are {members}, expected ['a', 'b']")
    if not described:
        fail(f"{label}/nested", f"no subprogram DIE named {NESTED}")
    if report.failures:
        fail(label + "/nested", "; ".join(str(failure) for failure in report.failures))


def check_locations(label: str, path: Path) -> None:
    elf = read_elf(path)
    report = Report()
    located: set[str] = set()
    located_parameters: set[str] = set()
    for unit in units(elf):
        for subprogram in unit.by_tag(DW_TAG_SUBPROGRAM):
            function_name = die_name(subprogram, unit)
            if function_name not in {"locals", "shadow"}:
                continue
            for die in children(unit, subprogram):
                if die.tag == DW_TAG_FORMAL_PARAMETER and DW_AT_LOCATION in die.attrs:
                    name = die_name(die, unit)
                    located_parameters.add(name)
                    check_parameter_location(report, elf, unit, die, subprogram, label, f"locals.{name}")
                if die.tag != DW_TAG_VARIABLE or DW_AT_LOCATION not in die.attrs:
                    continue
                name = die_name(die, unit)
                located.add(name)
                check_parameter_location(report, elf, unit, die, subprogram, label, name)
            if function_name == "shadow":
                outer = [die for die in children(unit, subprogram)
                         if die.tag == DW_TAG_VARIABLE and die_name(die, unit) == "x"
                         and DW_AT_LOCATION in die.attrs]
                inner = []
                for block in children(unit, subprogram):
                    if block.tag != DW_TAG_LEXICAL_BLOCK:
                        continue
                    inner.extend(die for die in children(unit, block)
                                 if die.tag == DW_TAG_VARIABLE and die_name(die, unit) == "x"
                                 and DW_AT_LOCATION in die.attrs)
                if len(outer) != 1 or len(inner) != 1:
                    fail(label + "/shadow", f"expected one outer and one inner x, got {len(outer)}/{len(inner)}")
                for die in (*outer, *inner):
                    check_parameter_location(report, elf, unit, die, subprogram, label, "shadow.x")
    if report.failures:
        fail(label, "; ".join(str(failure) for failure in report.failures))
    missing = set(LOCALS) - located
    if missing:
        fail(label, f"locals without a proven home: {sorted(missing)}")
    if "n" not in located_parameters:
        fail(label, "locals.n has no proven parameter home")


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
            inner, outer = gdb_shadow(gdb, candidate, work, source.name)
            if gdb_number(inner) != SHADOW_INNER:
                fail(label + "/gdb-shadow-inner", f"x = {inner}, expected {SHADOW_INNER}")
            if gdb_number(outer) != SHADOW_OUTER:
                fail(label + "/gdb-shadow-outer", f"x = {outer}, expected {SHADOW_OUTER}")
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

    # A nested function and a block-scope record type: both are ordinary C, and
    # both used to lose their description before the writer saw them.
    scoped_source = work / "scoped.c"
    scoped_source.write_text(SCOPED)
    scoped_obj = work / "scoped.o"
    result = run(target + common + ["-c", str(scoped_source), "-o", str(scoped_obj)], work)
    if result.returncode != 0:
        fail(label + "/scoped", result.stderr)
    check_scoped(label, scoped_obj)
    scoped_exe = work / "scoped"
    result = run([gcc, "-m32" if bits == 32 else "-m64", "-no-pie", str(scoped_obj),
                  "-o", str(scoped_exe)], work)
    if result.returncode != 0:
        fail(label + "/scoped-link", result.stderr)
    result = run([str(scoped_exe)], work)
    if result.returncode != 0:
        fail(label + "/scoped-run", f"exit {result.returncode}")
    asm = work / "scoped.s"
    via_obj = work / "scoped-via.o"
    result = run(target + common + ["-S", str(scoped_source), "-o", str(asm)], work)
    if result.returncode != 0:
        fail(label + "/scoped -S", result.stderr)
    result = run([gcc, "-m32" if bits == 32 else "-m64", "-c", str(asm), "-o", str(via_obj)], work)
    if result.returncode != 0:
        fail(label + "/scoped-gas", result.stderr)
    check_scoped(label + "/gas", via_obj)
    if gdb:
        for candidate in (scoped_exe,):
            values = gdb_scoped(gdb, candidate, work, scoped_source.name)
            for expr, expected in SCOPED_VALUES.items():
                if gdb_number(values[expr]) != expected:
                    fail(
                        label + "/gdb-scoped",
                        f"{expr} = {values[expr]}, expected {expected}",
                    )
    for level in ("-O0", "-O2"):
        plain = work / f"scoped-plain{level}.o"
        debug = work / f"scoped-debug{level}.o"
        for output, extra in ((plain, []), (debug, ["-g"])):
            result = run(target + [level] + extra + ["-c", str(scoped_source),
                                                     "-o", str(output)], work)
            if result.returncode != 0:
                fail(label + level + "/scoped", result.stderr)
        if code_sections(read_elf(plain)) != code_sections(read_elf(debug)):
            fail(label + level + "/scoped",
                 "-g changed code bytes; debug info must be free at runtime")
    return 8 if gdb else 6


def check_cross(lccc: str, label: str, version: int, root: Path) -> int:
    """AArch64/RISC-V: structural homes and debug-free code bytes.

    The pinned GDB is x86-only, so these legs prove the DIE/location-list shape
    and the frame-pointer register through the shared decoder rather than
    pretending a cross debugger ran.  QEMU execution is covered by the wider
    cross gates; this gate owns the debug contract.
    """
    work = root / f"{label}_v{version}"
    work.mkdir()
    source = work / "locals.c"
    source.write_text(SOURCE)
    common = ["-O0", f"-gdwarf-{version}", "-g"]
    obj = work / "direct.o"
    result = run([lccc, *common, "-c", str(source), "-o", str(obj)], work)
    if result.returncode != 0:
        fail(label, result.stderr)
    check_locations(f"{label}-v{version}", obj)
    for level in ("-O0", "-O2"):
        plain = work / f"plain{level}.o"
        debug = work / f"debug{level}.o"
        for output, extra in ((plain, []), (debug, ["-g"])):
            result = run([lccc, level, *extra, "-c", str(source), "-o", str(output)], work)
            if result.returncode != 0:
                fail(label + level, result.stderr)
        if code_sections(read_elf(plain)) != code_sections(read_elf(debug)):
            fail(label + level, "-g changed code bytes; debug info must be free at runtime")
    scoped_source = work / "scoped.c"
    scoped_source.write_text(SCOPED)
    scoped_obj = work / "scoped.o"
    result = run([lccc, *common, "-c", str(scoped_source), "-o", str(scoped_obj)], work)
    if result.returncode != 0:
        fail(label + "/scoped", result.stderr)
    check_scoped(f"{label}-v{version}", scoped_obj)
    return 3


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lccc")
    parser.add_argument("--lccc-arm")
    parser.add_argument("--lccc-riscv")
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
        for label, value in (("aarch64", args.lccc_arm), ("riscv64", args.lccc_riscv)):
            if value:
                compiler = str(Path(value).resolve()) if Path(value).exists() else value
                for version in (2, 3, 4, 5):
                    contracts += check_cross(compiler, label, version, root)
    report = {"status": "pass", "contracts": contracts, "gdb": gdb,
              "locals": list(LOCALS), "before": BEFORE, "after": AFTER,
              "shadow": {"inner": SHADOW_INNER, "outer": SHADOW_OUTER},
              "cross_structural": {"aarch64": bool(args.lccc_arm), "riscv64": bool(args.lccc_riscv)}}
    if args.json:
        args.json.write_text(json.dumps(report, indent=2) + "\n")
    print(f"PASS: DWARF local homes ({contracts} contracts{'; GDB ' + gdb if gdb else ''})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
