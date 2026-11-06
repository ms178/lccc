#!/usr/bin/env python3
"""D1b: codegen-proven parameter homes are debuggable, not just named.

At -O0 the x86 backends home incoming arguments to frame-pointer-relative
slots.  The writer describes each home with a location list whose range is the
line table's prologue_end..epilogue_begin, so GDB prints the argument after the
prologue and does not claim the home at the exact entry instruction.  The gate
also proves -g changed no instruction bytes: debugging must be free at runtime.

The probe is deliberately wider than the register window: floating-point and
stack-passed arguments, a pointer whose pointee is mutated, and parameters the
function itself assigns.  A home that merely names the incoming slot would fail
the post-assignment GDB reads; a home that double-counts the function base would
fail the decoder and the i686 link.
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
    DW_TAG_FORMAL_PARAMETER,
    DW_TAG_SUBPROGRAM,
    Report,
    breg_location,
    check_parameter_location,
    children,
    code_sections,
    die_name,
    location_lists,
    read_elf,
    units,
)
from check_dwarf_quality import self_test as quality_self_test  # noqa: E402

SOURCE = r"""
int local = 11;
struct big { long long a, b, c; };
__attribute__((noinline)) struct big make_big(int a, long long b, int c) {
    struct big r = {a, b, c};
    return r;
}
__attribute__((noinline)) int frame(int n, long long wide, int *p, double ratio,
                                    unsigned int tag, unsigned char small,
                                    int stack7, int stack8) {
    n += 5;
    wide += 1;
    *p += 11;
    ratio += 0.25;
    tag -= 1;
    small = (unsigned char)(small + 1);
    stack7 += 1;
    stack8 += 1;
    int observed = n; /* after-update */
    return observed == 12 && wide == 123456790LL && *p == 22 && ratio == 2.75
        && tag == 3999999999U && small == 201 && stack7 == 71 && stack8 == 81 ? 0 : 1;
}
int main(void) {
    int result = frame(7, 123456789LL, &local, 2.5, 4000000000U, 200, 70, 80);
    struct big r = make_big(11, 22, 33);
    return result || !(r.a == 11 && r.b == 22 && r.c == 33);
}
"""

PARAMETERS = ("n", "wide", "p", "ratio", "tag", "small", "stack7", "stack8")
REQUIRED_HOMES = {"frame": PARAMETERS, "make_big": ("a", "b", "c")}
INITIAL = {"n": 7, "wide": 123456789, "p": 11, "ratio": 2.5, "tag": 4000000000,
           "small": 200, "stack7": 70, "stack8": 80}
UPDATED = {"n": 12, "wide": 123456790, "p": 22, "ratio": 2.75, "tag": 3999999999,
           "small": 201, "stack7": 71, "stack8": 81}


def run(argv: list[str], cwd: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(argv, cwd=cwd, text=True, capture_output=True, timeout=120)


def fail(label: str, detail: str) -> None:
    print(f"FAIL: {label}: {detail}")
    raise SystemExit(1)


def marker_line(source: str) -> int:
    for number, line in enumerate(source.splitlines(), 1):
        if "/* after-update */" in line:
            return number
    raise ValueError("probe lost its after-update marker")


def gdb_number(text: str) -> int | float:
    """GDB prints integers in decimal or hex, doubles as decimal literals."""
    token = text.strip().split()[0]
    if re.fullmatch(r"-?\d+", token):
        return int(token)
    if re.fullmatch(r"0[xX][0-9a-fA-F]+", token):
        return int(token, 16)
    return float(token)


def gdb_values(gdb: str, exe: Path, cwd: Path, source_name: str) -> tuple[dict[str, str], dict[str, str]]:
    script = f"""set pagination off
set confirm off
break frame
run
print n
print wide
print *p
print ratio
print tag
print small
print stack7
print stack8
break {source_name}:{marker_line(SOURCE)}
continue
print n
print wide
print *p
print ratio
print tag
print small
print stack7
print stack8
quit
"""
    command_file = cwd / "body.gdb"
    command_file.write_text(script)
    result = run([gdb, "-q", "-batch", "-x", str(command_file), str(exe)], cwd)
    if result.returncode != 0:
        fail("gdb", result.stdout + result.stderr)
    prints = re.findall(r"\$\d+ = ([^\n]+)", result.stdout)
    if len(prints) < 2 * len(PARAMETERS):
        fail("gdb", f"expected {2 * len(PARAMETERS)} printed values, got {result.stdout!r}")
    initial = dict(zip(PARAMETERS, (value.strip() for value in prints[:len(PARAMETERS)])))
    updated = dict(zip(PARAMETERS, (value.strip() for value in prints[len(PARAMETERS):])))
    return initial, updated


def gdb_sret(gdb: str, exe: Path, cwd: Path) -> dict[str, str]:
    command_file = cwd / "sret.gdb"
    command_file.write_text(
        "set pagination off\nset confirm off\nbreak make_big\nrun\n"
        "print a\nprint b\nprint c\nquit\n"
    )
    result = run([gdb, "-q", "-batch", "-x", str(command_file), str(exe)], cwd)
    if result.returncode != 0:
        fail("gdb-sret", result.stdout + result.stderr)
    prints = re.findall(r"\$\d+ = ([^\n]+)", result.stdout)
    if len(prints) < 3:
        fail("gdb-sret", f"expected three printed values, got {result.stdout!r}")
    return dict(zip(("a", "b", "c"), (value.strip() for value in prints[:3])))


def gdb_entry(gdb: str, exe: Path, cwd: Path) -> str:
    command_file = cwd / "entry.gdb"
    command_file.write_text("set pagination off\nset confirm off\nbreak *frame\nrun\nprint n\nquit\n")
    result = run([gdb, "-q", "-batch", "-x", str(command_file), str(exe)], cwd)
    if result.returncode != 0:
        fail("gdb-entry", result.stdout + result.stderr)
    match = re.search(r"\$1 = ([^\n]+)", result.stdout)
    return match.group(1).strip() if match else result.stdout


def check_locations(label: str, path: Path) -> None:
    """Every frame parameter has a decodable, live, frame-relative home."""
    elf = read_elf(path)
    version = 5 if ".debug_loclists" in elf.sections else 4
    list_section = ".debug_loclists" if version >= 5 else ".debug_loc"
    if not elf.sections.get(list_section):
        fail(label, f"no {list_section} section")
    report = Report()
    located: dict[str, set[str]] = {name: set() for name in REQUIRED_HOMES}
    for unit in units(elf):
        try:
            location_lists(elf, unit.version)
        except ValueError as error:
            fail(label, f"location-list decode: {error}")
        for subprogram in unit.by_tag(DW_TAG_SUBPROGRAM):
            function = die_name(subprogram, unit)
            if function not in REQUIRED_HOMES:
                continue
            for die in children(unit, subprogram):
                if die.tag != DW_TAG_FORMAL_PARAMETER or DW_AT_LOCATION not in die.attrs:
                    continue
                name = die_name(die, unit)
                located[function].add(name)
                check_parameter_location(report, elf, unit, die, subprogram, label, f"{function}.{name}")
    if report.failures:
        fail(label, "; ".join(str(failure) for failure in report.failures))
    for function, parameters in REQUIRED_HOMES.items():
        missing = set(parameters) - located[function]
        if missing:
            fail(label, f"{function} parameters without a proven home: {sorted(missing)}")


def check_mode(lccc: str, gcc: str, gdb: str | None, bits: int, version: int, root: Path) -> int:
    label = f"i686-v{version}" if bits == 32 else f"x86-64-v{version}"
    work = root / label.replace("-", "_")
    work.mkdir()
    source = work / "params.c"
    source.write_text(SOURCE)
    target = [lccc] + (["-m32"] if bits == 32 else [])
    common = ["-O0", f"-gdwarf-{version}"]
    obj = work / "direct.o"
    result = run(target + common + ["-g", "-c", str(source), "-o", str(obj)], work)
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

    # -S + GAS must deliver the same location lists (D7 renders the DIE bytes).
    asm = work / "via.s"
    via_obj = work / "via.o"
    result = run(target + common + ["-g", "-S", str(source), "-o", str(asm)], work)
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
            initial, updated = gdb_values(gdb, candidate, work, source.name)
            for name, expected in INITIAL.items():
                if gdb_number(initial[name]) != expected:
                    fail(label + "/gdb-body", f"{name} = {initial[name]}, expected {expected}")
            for name, expected in UPDATED.items():
                if gdb_number(updated[name]) != expected:
                    fail(label + "/gdb-after-update", f"{name} = {updated[name]}, expected {expected}")
            entry = gdb_entry(gdb, candidate, work)
            if entry == "7":
                fail(label + "/gdb-entry", "parameter home is claimed at the exact entry instruction")
            sret = gdb_sret(gdb, candidate, work)
            for name, expected in (("a", 11), ("b", 22), ("c", 33)):
                if gdb_number(sret[name]) != expected:
                    fail(label + "/gdb-sret", f"make_big.{name} = {sret[name]}, expected {expected}")
        # The linked location list must start after the prologue.  The decoder
        # above already proved the range; this is the consumer-visible address.
        nm = run(["nm", str(exe)], work)
        entry_address = next((int(line.split()[0], 16) for line in nm.stdout.splitlines()
                              if line.split()[-1:] == ["frame"]), None)
        dump = run(["readelf", "--debug-dump=loc", str(exe)], work)
        ranges = [
            (start, end)
            for _, start, end in re.findall(
                r"^\s*([0-9a-fA-F]+)\s+([0-9a-fA-F]+)\s+([0-9a-fA-F]+)",
                dump.stdout, re.M,
            )
            if set(start.lower()) != {"f"}
        ]
        if entry_address is None or not ranges:
            fail(label + "/location-range", f"no location ranges for frame: {dump.stdout!r} {dump.stderr!r}")
        if not any(int(start, 16) > entry_address for start, _ in ranges):
            fail(label + "/location-range", "a parameter home is claimed at the exact entry instruction")

    # Runtime performance guard: -g must not change a single code byte, in any
    # code section, at any optimization tier the driver accepts.
    for level in ("-O0", "-O1", "-O2", "-O3", "-Os"):
        plain = work / f"plain{level}.o"
        debug = work / f"debug{level}.o"
        for output, extra in ((plain, []), (debug, ["-g"])):
            result = run(target + [level] + extra + ["-c", str(source), "-o", str(output)], work)
            if result.returncode != 0:
                fail(label + level, result.stderr)
        if code_sections(read_elf(plain)) != code_sections(read_elf(debug)):
            fail(label + level, "-g changed code bytes; debug info must be free at runtime")
    return 4 if gdb else 3


def self_test() -> int:
    """The gate's own parser/policy helpers, plus the shared DWARF decoder."""
    if quality_self_test() != 0:
        return 1
    assert marker_line(SOURCE) == SOURCE.splitlines().index("    int observed = n; /* after-update */") + 1
    assert gdb_number("0x75bcd15") == 123456789
    assert gdb_number("3999999999") == 3999999999
    assert gdb_number("2.75") == 2.75
    assert breg_location(bytes([0x76, 0x6F])) == (6, -17)
    assert breg_location(bytes([0x75, 0x0C])) == (5, 12)
    print("parameter gate self-test: PASS (marker, GDB literals, breg homes; shared decoder self-test above)")
    return 0


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
        return self_test()
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
    with tempfile.TemporaryDirectory(prefix="lccc-dwarf-params-") as directory:
        root = Path(directory)
        for bits in (64, 32):
            for version in (2, 3, 4, 5):
                contracts += check_mode(args.lccc, args.gcc, gdb, bits, version, root)
    report = {"status": "pass", "contracts": contracts, "gdb": gdb,
              "parameters": {name: list(params) for name, params in REQUIRED_HOMES.items()},
              "initial": INITIAL, "updated": UPDATED}
    if args.json:
        args.json.write_text(json.dumps(report, indent=2) + "\n")
    print(f"PASS: DWARF parameter homes ({contracts} contracts{'; GDB ' + gdb if gdb else ''})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
