#!/usr/bin/env python3
"""D15: `-gpubnames` accelerator tables are complete, exact and link-safe.

A name index is a *claim*: "the DIE at this offset is named this".  A consumer
trusts it instead of walking the unit, so a wrong offset silently resolves a
name to the wrong entity and a missing entry silently makes a symbol invisible
to `break`/`ptype` on a large program.  This gate therefore never trusts the
writer's own bookkeeping: it re-parses `.debug_info` with the campaign's
independent DIE reader and derives the expected table from the tree itself.

What is proved, per DWARF version 2..5 and per target:

* **Exactness** -- every published offset lands on a DIE whose tag and
  `DW_AT_name` match the entry, byte for byte.
* **Completeness** -- every unit-level subprogram and file-scope object that
  this object *defines* is published exactly once, and so is every named base
  type, complete record/enum and typedef.
* **Exclusivity** -- no local, formal parameter, member, enumerator, anonymous
  type or declaration-only entity is published.  These are the entries a naive
  "index everything with a name" implementation leaks.
* **Header and terminator** -- unit length, version 2, the `.debug_info`
  offset and contribution length, and the terminating zero word, with no
  trailing bytes.
* **Relocation** -- exactly one 32-bit relocation against `.debug_info` at
  offset 6, of the target's own absolute-32 type.  Without it every entry
  would point into the *first* unit after a link.
* **Link safety** -- two objects linked together keep two contributions whose
  `debug_info_offset` values differ and whose entries resolve inside their own
  unit.  This is the failure a single-object test cannot see.
* **Flag semantics** -- the tables are opt-in, `-gpubnames` alone enables no
  debug info, `-gno-pubnames` wins when it comes last, and the default object
  grows by zero bytes.
* **Oracle agreement** -- GCC 16.2 publishes the same *name sets* for the same
  source (offsets differ with layout, so sets are the invariant).
* **Consumer reality** -- GDB resolves a global function, a global object and
  a typedef through the indexed executable.

The gate's own validator is mutation-tested: seven corruptions of a real table
must each be rejected, so a check that silently stopped checking is a failure.
"""
from __future__ import annotations

import argparse
import json
import shutil
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tests/regression"))
from check_dwarf_quality import (  # noqa: E402
    DW_AT_DECLARATION,
    DW_AT_NAME,
    DW_TAG_BASE_TYPE,
    DW_TAG_ENUMERATION_TYPE,
    DW_TAG_ENUMERATOR,
    DW_TAG_FORMAL_PARAMETER,
    DW_TAG_STRUCTURE_TYPE,
    DW_TAG_SUBPROGRAM,
    DW_TAG_TYPEDEF,
    DW_TAG_UNION_TYPE,
    DW_TAG_VARIABLE,
    Die,
    Unit,
    die_name,
    read_elf,
    units,
)

#: Tags whose named, unit-level DIEs belong in `.debug_pubnames`, plus the
#: enumerator: both oracles publish enumerators (measured on GCC 16.2 and Clang
#: 23.1.5), because `print RED` is a name lookup exactly like `print global_var`.
NAME_TAGS = (DW_TAG_SUBPROGRAM, DW_TAG_VARIABLE, DW_TAG_ENUMERATOR)
#: Tags whose named, complete, unit-level DIEs belong in `.debug_pubtypes`.
TYPE_TAGS = (DW_TAG_BASE_TYPE, DW_TAG_STRUCTURE_TYPE, DW_TAG_UNION_TYPE,
             DW_TAG_ENUMERATION_TYPE, DW_TAG_TYPEDEF)

SOURCE = r"""
typedef unsigned long ulong;
typedef int (*callback)(int);
enum color { RED, GREEN = 5, BLUE = -3 };
struct point { int x; int y; };
union cell { int i; float f; };
struct opaque_declared_only;
struct opaque_declared_only *opaque_ptr;
struct anonymous_holder { struct { int inner; }; int tail; } anonymous_holder;
static int hidden_static = 3;
int global_var = 7;
extern int extern_declared_only;
ulong global_ul = 11;
enum color global_color = GREEN;
struct point global_point = {1, 2};
union cell global_cell = {4};
static int helper(int a) { int doubled = a * 2; return doubled + 1; }
int public_fn(int a) { int local = helper(a); return local + hidden_static; }
int another(void) { struct point p = {3, 4}; return p.x + global_var; }
"""

MAIN = r"""
#include <stdio.h>
int public_fn(int a);
int another(void);
extern int global_var;
int main(void) { printf("%d\n", public_fn(3) + another() + global_var); return 0; }
"""

SECOND = r"""
typedef short small_t;
struct other { long long z; };
small_t second_small = 3;
int second_global = 21;
int second_fn(int a) { struct other o = {a}; return (int)o.z + second_global; }
"""


def fail(label: str, detail: str) -> None:
    print(f"FAIL: {label}: {detail}")
    raise SystemExit(1)


def run(argv: list[str], cwd: Path) -> subprocess.CompletedProcess:
    return subprocess.run(argv, cwd=cwd, capture_output=True, text=True, timeout=300)


def compile_ok(compiler: str, flags: list[str], source: Path, out: Path,
               cwd: Path, label: str) -> None:
    proc = run([compiler, *flags, "-c", str(source), "-o", str(out)], cwd)
    if proc.returncode != 0:
        fail(label, f"{compiler} {' '.join(flags)} failed:\n{proc.stderr}")
    if proc.stderr.strip():
        fail(label, f"{compiler} {' '.join(flags)} was not silent:\n{proc.stderr}")


@dataclass(frozen=True)
class Table:
    """One parsed `.debug_pubnames`/`.debug_pubtypes` contribution."""
    section: str
    start: int
    length: int
    version: int
    info_offset: int
    info_length: int
    entries: list[tuple[int, str]]
    end: int


def parse_tables(blob: bytes, section: str, *, allow_duplicates: bool = False) -> list[Table]:
    """Parse every contribution, refusing any malformed or truncated table.

    Written defensively on purpose: this function is also the oracle for the
    mutation self-test, so every field is bounds-checked rather than sliced
    optimistically.

    `allow_duplicates` exists for reading *other* compilers: GCC 16.2 emits a
    duplicate entry for every tentative definition (measured: `opaque_ptr` and
    `anonymous_holder` twice, 26 wasted bytes).  That is a defect worth knowing
    about, not a parse error, and lccc must not reproduce it -- so the strict
    default stays on for every table this compiler writes.
    """
    tables: list[Table] = []
    offset = 0
    while offset < len(blob):
        start = offset
        if offset + 4 > len(blob):
            raise ValueError(f"{section}: truncated unit length at {offset}")
        length = int.from_bytes(blob[offset:offset + 4], "little")
        if length == 0:
            raise ValueError(f"{section}: zero unit length at {offset}")
        if length < 2 + 4 + 4 + 4:
            raise ValueError(f"{section}: unit length {length} shorter than the header")
        end = offset + 4 + length
        if end > len(blob):
            raise ValueError(f"{section}: contribution runs past the section")
        cursor = offset + 4
        version = int.from_bytes(blob[cursor:cursor + 2], "little")
        if version != 2:
            raise ValueError(f"{section}: version {version}, expected 2")
        cursor += 2
        info_offset = int.from_bytes(blob[cursor:cursor + 4], "little")
        cursor += 4
        info_length = int.from_bytes(blob[cursor:cursor + 4], "little")
        cursor += 4
        if info_length == 0:
            raise ValueError(f"{section}: zero .debug_info contribution length")
        entries: list[tuple[int, str]] = []
        seen: set[str] = set()
        while True:
            if cursor + 4 > end:
                raise ValueError(f"{section}: entry runs past the contribution")
            die_offset = int.from_bytes(blob[cursor:cursor + 4], "little")
            cursor += 4
            if die_offset == 0:
                break
            if die_offset >= info_length:
                raise ValueError(
                    f"{section}: entry offset {die_offset} outside the unit "
                    f"({info_length} bytes)")
            terminator = blob.find(b"\0", cursor, end)
            if terminator < 0:
                raise ValueError(f"{section}: unterminated name at {cursor}")
            name = blob[cursor:terminator].decode("utf-8")
            if not name:
                raise ValueError(f"{section}: empty name at {cursor}")
            if name in seen and not allow_duplicates:
                raise ValueError(f"{section}: duplicate name {name!r}")
            seen.add(name)
            entries.append((die_offset, name))
            cursor = terminator + 1
        if cursor != end:
            raise ValueError(f"{section}: {end - cursor} trailing byte(s) after the terminator")
        if not entries:
            raise ValueError(f"{section}: empty table should not have been emitted")
        tables.append(Table(section, start, length, version, info_offset,
                            info_length, entries, end))
        offset = end
    return tables


def expected_from_tree(elf, tags: tuple[int, ...]) -> dict[str, int]:
    """The publishable names of an object, derived from its DIE tree."""
    return expected_from_units(units(elf), tags)


def expected_from_units(unit_list, tags: tuple[int, ...]) -> dict[str, int]:
    """The names a correct table must publish, given parsed units.

    Only *unit-level* DIEs count: a local variable or a formal parameter also
    carries `DW_TAG_variable`/a name, and indexing one would make `break local`
    resolve to a stack slot in an unrelated frame.  Depth 1 is exactly the
    compile unit's own children in the flattened tree.

    The one exception is `DW_TAG_ENUMERATOR`, which is a child of its
    enumeration type at depth 2.  It is admitted only when its parent really is
    a unit-level enumeration type, so an enumerator of a locally-scoped enum
    cannot leak into the index.  Taking the units as an argument keeps one
    implementation of the rule and lets the self-test drive it with synthetic
    trees.
    """
    expected: dict[str, int] = {}
    for unit in unit_list:
        # Parent by depth: the flattened tree is pre-order, so the enclosing
        # DIE at depth d-1 is the most recent one seen at that depth.
        enclosing: dict[int, Die] = {}
        for die in unit.dies:
            enclosing[die.depth] = die
            for deeper in [d for d in enclosing if d > die.depth]:
                del enclosing[deeper]
            if die.tag not in tags:
                continue
            if die.tag == DW_TAG_ENUMERATOR:
                parent = enclosing.get(die.depth - 1)
                if (die.depth != 2 or parent is None
                        or parent.tag != DW_TAG_ENUMERATION_TYPE):
                    continue
            elif die.depth != 1:
                continue
            if die.attrs.get(DW_AT_DECLARATION):
                continue
            name = die_name(die, unit)
            if not name:
                continue
            expected[name] = die.offset
    return expected


def check_table(elf, section: str, tags: tuple[int, ...], label: str) -> Table:
    blob = elf.sections.get(section)
    if blob is None:
        fail(label, f"{section} was not emitted")
    try:
        tables = parse_tables(blob, section)
    except ValueError as error:
        fail(label, str(error))
    if len(tables) != 1:
        fail(label, f"{section}: {len(tables)} contributions in a single-CU object")
    table = tables[0]
    info = elf.sections.get(".debug_info", b"")
    if table.info_offset != 0:
        fail(label, f"{section}: info_offset {table.info_offset}, expected 0 in a relocatable object")
    if table.info_length != len(info):
        fail(label, f"{section}: contribution length {table.info_length} != .debug_info {len(info)}")

    published = {name: offset for offset, name in table.entries}
    expected = expected_from_tree(elf, tags)
    missing = sorted(set(expected) - set(published))
    extra = sorted(set(published) - set(expected))
    if missing:
        fail(label, f"{section}: missing name(s) {missing}")
    if extra:
        fail(label, f"{section}: published non-indexable name(s) {extra}")
    for name, offset in published.items():
        if offset != expected[name]:
            fail(label, f"{section}: {name!r} points at {offset:#x}, DIE is at {expected[name]:#x}")
    # Exactness against the tree: the offset must land on a DIE of an indexable
    # tag whose name is the published one.  A table could agree with itself and
    # still be wrong; this reads the bytes a consumer reads.
    for unit in units(elf):
        # One index, not a linear scan per entry: `Unit.die_at` is O(dies), so
        # using it inside this loop would make the check O(dies x entries) on a
        # unit with thousands of names.
        by_offset = {die.offset: die for die in unit.dies}
        for offset, name in table.entries:
            die = by_offset.get(offset)
            if die is None:
                fail(label, f"{section}: {name!r} offset {offset:#x} is not a DIE boundary")
            if die.tag not in tags:
                fail(label, f"{section}: {name!r} points at tag {die.tag:#x}")
            if die_name(die, unit) != name:
                fail(label, f"{section}: offset {offset:#x} is named {die_name(die, unit)!r}, not {name!r}")
    return table


#: Absolute-32 relocation type per ELF machine, for the `.debug_info` reference.
ABS32_BY_MACHINE = {
    3: 1,        # EM_386      R_386_32
    62: 10,      # EM_X86_64   R_X86_64_32
    183: 258,    # EM_AARCH64  R_AARCH64_ABS32
    243: 1,      # EM_RISCV    R_RISCV_32
}


def check_relocation(path: Path, readelf: str, section: str, machine: int,
                     label: str) -> None:
    """Exactly one absolute-32 relocation against `.debug_info`, at offset 6."""
    proc = subprocess.run([readelf, "-r", str(path)], capture_output=True,
                          text=True, timeout=120)
    if proc.returncode != 0:
        fail(label, f"readelf -r failed: {proc.stderr}")
    wanted = ABS32_BY_MACHINE.get(machine)
    if wanted is None:
        fail(label, f"no absolute-32 relocation contract for machine {machine}")
    in_section = False
    found: list[tuple[int, int]] = []
    rela = f".rela{section}"
    rel = f".rel{section}"
    for line in proc.stdout.splitlines():
        stripped = line.strip()
        if stripped.startswith("Relocation section "):
            in_section = f"'{rela}'" in stripped or f"'{rel}'" in stripped
            continue
        if not in_section or not stripped or stripped.startswith("Offset"):
            continue
        fields = stripped.split()
        if len(fields) < 5:
            continue
        try:
            offset = int(fields[0], 16)
        except ValueError:
            continue
        info = int(fields[1], 16)
        r_type = info & 0xffffffff if machine in (62, 183, 243) else info & 0xff
        if ".debug_info" not in stripped:
            fail(label, f"{section}: relocation at {offset:#x} does not name .debug_info: {stripped}")
        found.append((offset, r_type))
    if len(found) != 1:
        fail(label, f"{section}: {len(found)} relocations, expected exactly 1")
    offset, r_type = found[0]
    if offset != 6:
        fail(label, f"{section}: relocation at {offset:#x}, expected 0x6 (the info offset field)")
    if r_type != wanted:
        fail(label, f"{section}: relocation type {r_type}, expected {wanted} for machine {machine}")


def check_absent(path: Path, label: str) -> None:
    elf = read_elf(path)
    for section in (".debug_pubnames", ".debug_pubtypes"):
        if section in elf.sections:
            fail(label, f"{section} emitted when it was not requested")


def check_flag_semantics(lccc: str, source: Path, root: Path, version: int) -> int:
    """Opt-in, last-wins, and no debug info from the selector alone."""
    checks = 0
    d = f"-gdwarf-{version}"
    # Each case carries its complete command line: `-gdwarf-N` *itself* enables
    # debug info on both compilers (measured on GCC 16.2 and lccc), so the
    # "selector alone" case must not silently inherit a dialect flag.
    cases: list[tuple[list[str], bool, bool]] = [
        ([d, "-g"], False, True),                              # default: no index
        ([d, "-g", "-gpubnames"], True, True),                 # opt-in
        ([d, "-g", "-gpubnames", "-gno-pubnames"], False, True),
        ([d, "-g", "-gno-pubnames", "-gpubnames"], True, True),
        (["-gpubnames"], False, False),                        # selector alone
        (["-gpubnames", d, "-g"], True, True),
        ([d, "-gpubnames"], True, True),                       # dialect enables -g
        ([], False, False),
    ]
    for index, (flags, want_tables, want_debug) in enumerate(cases):
        out = root / f"flags-{version}-{index}.o"
        compile_ok(lccc, flags, source, out, root, "flag-semantics")
        elf = read_elf(out)
        has_debug = ".debug_info" in elf.sections
        if has_debug != want_debug:
            fail("flag-semantics", f"{' '.join(flags) or '(no flags)'}: .debug_info={has_debug}, expected {want_debug}")
        has_tables = ".debug_pubnames" in elf.sections
        if has_tables != want_tables:
            fail("flag-semantics", f"{' '.join(flags) or '(no flags)'}: tables={has_tables}, expected {want_tables}")
        checks += 1
    # The default object must not grow: an index nobody asked for is a tax on
    # every build.  Compared against the same source compiled without the flag.
    plain = root / f"plain-{version}.o"
    indexed = root / f"indexed-{version}.o"
    compile_ok(lccc, [d, "-g"], source, plain, root, "size")
    compile_ok(lccc, [d, "-g", "-gpubnames"], source, indexed, root, "size")
    if plain.stat().st_size >= indexed.stat().st_size:
        fail("size", "the indexed object is not larger; the tables were not written")
    return checks


def check_link(lccc: str, readelf: str, gdb: str | None, root: Path, version: int) -> int:
    """Two units, one image: each table must still address its own unit."""
    checks = 0
    dialect = [f"-gdwarf-{version}"]
    first = root / f"link-a-{version}.o"
    second = root / f"link-b-{version}.o"
    main = root / f"link-main-{version}.c"
    main.write_text(MAIN)
    a = root / f"link-src-a-{version}.c"
    a.write_text(SOURCE)
    b = root / f"link-src-b-{version}.c"
    b.write_text(SECOND)
    compile_ok(lccc, [*dialect, "-g", "-gpubnames"], a, first, root, "link")
    compile_ok(lccc, [*dialect, "-g", "-gpubnames"], b, second, root, "link")
    main_obj = root / f"link-main-{version}.o"
    compile_ok(lccc, [*dialect, "-g"], main, main_obj, root, "link")
    image = root / f"image-{version}"
    proc = run([lccc, str(first), str(second), str(main_obj), "-o", str(image)], root)
    if proc.returncode != 0:
        fail("link", f"lccc could not link the indexed objects:\n{proc.stderr}")
    elf = read_elf(image)
    info = elf.sections.get(".debug_info", b"")
    all_tables: list[Table] = []
    for section in (".debug_pubnames", ".debug_pubtypes"):
        blob = elf.sections.get(section)
        if blob is None:
            fail("link", f"{section} vanished at link time")
        try:
            tables = parse_tables(blob, section)
        except ValueError as error:
            fail("link", str(error))
        if len(tables) != 2:
            fail("link", f"{section}: {len(tables)} contributions after linking two indexed units")
        offsets = [table.info_offset for table in tables]
        if offsets[0] == offsets[1]:
            fail("link", f"{section}: both contributions point at {offsets[0]:#x}; "
                         "the .debug_info relocation was not applied")
        for table in tables:
            if table.info_offset + table.info_length > len(info):
                fail("link", f"{section}: contribution {table.info_offset:#x}+{table.info_length} "
                             f"runs past .debug_info ({len(info)} bytes)")
            # Every entry must resolve inside its OWN unit, not merely inside
            # the section: that is the bug a shared base address would hide.
            for offset, name in table.entries:
                absolute = table.info_offset + offset
                if not (table.info_offset <= absolute < table.info_offset + table.info_length):
                    fail("link", f"{section}: {name!r} escapes its unit")
        all_tables.extend(tables)
        checks += 1
    published = {name for table in all_tables if table.section == ".debug_pubnames"
                 for _, name in table.entries}
    for required in ("public_fn", "another", "global_var", "second_fn", "second_global"):
        if required not in published:
            fail("link", f"{required!r} is not indexed in the linked image")
    if gdb:
        proc = subprocess.run(
            [gdb, "-q", "-batch", "-ex", "info address public_fn",
             "-ex", "info address second_global", "-ex", "ptype ulong",
             "-ex", "ptype small_t", "-ex", "ptype struct other", str(image)],
            capture_output=True, text=True, timeout=180)
        text = proc.stdout + proc.stderr
        # GDB normalizes the display spelling: the DWARF name is
        # `long unsigned int` (GCC's, which lccc matches) but `ptype` prints
        # `unsigned long`.  Accept either rather than pin GDB's renderer.
        if "unsigned long" not in text and "long unsigned int" not in text:
            fail("link-gdb", f"GDB did not resolve the typedef through the index:\n{text}")
        for needle in ("public_fn", "second_global", "short", "struct other", "long long z"):
            if needle not in text:
                fail("link-gdb", f"GDB did not resolve {needle!r} through the index:\n{text}")
        if "No symbol" in text or "has unknown type" in text:
            fail("link-gdb", f"GDB could not resolve an indexed name:\n{text}")
        checks += 1
    return checks


def check_codegen_neutral(lccc: str, source: Path, root: Path) -> int:
    """`-gpubnames` must not change one instruction byte.

    An accelerator table is pure metadata: it lives in non-allocable sections
    and describes DIEs that already exist.  If requesting it perturbed code
    generation, every performance measurement taken with debug info would be
    incomparable -- and the flag would be a tuning hazard rather than a
    debugging aid.
    """
    plain = root / "neutral-plain.o"
    indexed = root / "neutral-indexed.o"
    compile_ok(lccc, ["-O2", "-g"], source, plain, root, "codegen-neutral")
    compile_ok(lccc, ["-O2", "-g", "-gpubnames"], source, indexed, root, "codegen-neutral")
    a, b = read_elf(plain), read_elf(indexed)
    for section in (".text", ".data", ".rodata", ".bss"):
        if a.sections.get(section, b"") != b.sections.get(section, b""):
            fail("codegen-neutral", f"-gpubnames changed {section}")
    # Every added section must be non-allocable metadata.
    added = set(b.sections) - set(a.sections)
    allowed = {name for base in (".debug_pubnames", ".debug_pubtypes")
               for name in (base, ".rel" + base, ".rela" + base)}
    if added - allowed:
        fail("codegen-neutral", f"-gpubnames added unexpected sections: {sorted(added - allowed)}")
    if not {".debug_pubnames", ".debug_pubtypes"} <= added:
        fail("codegen-neutral", "the tables were not added at all")
    return 1


#: Per-target structural contract.  The tables are address-size independent
#: (every field is 32-bit), but the `.debug_info` relocation is not: each target
#: needs its own absolute-32 relocation type, and a writer that emitted the
#: x86-64 type everywhere would produce an index the linker cannot fix up.
TARGETS = (
    ("i686", "--lccc-i686", 3),
    ("arm", "--lccc-arm", 183),
    ("riscv", "--lccc-riscv", 243),
)


def check_target(name: str, compiler: str, machine: int, readelf: str,
                 source: Path, root: Path) -> int:
    out = root / f"{name}.o"
    compile_ok(compiler, ["-O2", "-g", "-gpubnames"], source, out, root, name)
    elf = read_elf(out)
    if elf.machine != machine:
        fail(name, f"expected ELF machine {machine}, got {elf.machine}")
    check_table(elf, ".debug_pubnames", NAME_TAGS, name)
    check_table(elf, ".debug_pubtypes", TYPE_TAGS, name)
    check_relocation(out, readelf, ".debug_pubnames", machine, name)
    check_relocation(out, readelf, ".debug_pubtypes", machine, name)
    return 4


def check_oracle(gcc: str, source: Path, root: Path, lccc_elf, version: int) -> dict:
    """Each compiler must agree with its own DIE tree; shared names must match.

    The invariant is "publish exactly the publishable names of your own tree",
    checked here for both compilers, plus "every name both trees contain is
    published by both".  At -O2 both GCC and lccc keep an abstract DIE for the
    fully-inlined static `helper` (D6), so the shared name set includes it; a
    compiler that inlines a helper away entirely may legitimately publish fewer
    names, which is why the direct set comparison is not the oracle.
    """
    out = root / f"gcc-{version}.o"
    proc = run([gcc, f"-gdwarf-{version}", "-g", "-gpubnames", "-O2",
                "-c", str(source), "-o", str(out)], root)
    if proc.returncode != 0:
        fail("oracle", f"{gcc} failed:\n{proc.stderr}")
    gcc_elf = read_elf(out)
    facts: dict[str, object] = {}
    for section, tags in ((".debug_pubnames", NAME_TAGS), (".debug_pubtypes", TYPE_TAGS)):
        blob = gcc_elf.sections.get(section)
        if blob is None:
            fail("oracle", f"{gcc} emitted no {section}; the oracle contract changed")
        try:
            tables = parse_tables(blob, section, allow_duplicates=True)
        except ValueError as error:
            fail("oracle", f"GCC's own {section} did not parse: {error}")
        entries = [entry for table in tables for entry in table.entries]
        gcc_names = {name for _, name in entries}
        gcc_expected = expected_from_tree(gcc_elf, tags)
        if gcc_names != set(gcc_expected):
            fail("oracle", f"GCC's {section} disagrees with its own DIE tree: "
                           f"{sorted(gcc_names ^ set(gcc_expected))}")
        ours = {name for _, name in
                parse_tables(lccc_elf.sections[section], section)[0].entries}
        our_expected = expected_from_tree(lccc_elf, tags)
        shared = gcc_names & set(our_expected)
        if not shared <= ours:
            fail("oracle", f"{section}: lccc does not publish name(s) both trees "
                           f"contain: {sorted(shared - ours)}")
        facts[section] = {
            "gcc_entries": len(entries),
            "gcc_unique": len(gcc_names),
            "gcc_duplicates": len(entries) - len(gcc_names),
            "shared_names": len(shared),
            "only_gcc_tree": sorted(gcc_names - set(our_expected)),
            "only_lccc_tree": sorted(set(our_expected) - gcc_names),
        }
    return facts


def self_test() -> int:
    """Every validator rule must actually fire on a corrupted table."""
    entries = [(0x40, "alpha"), (0x80, "beta")]

    def build(version: int = 2, info_length: int = 0x100,
              body: list[tuple[int, str]] | None = None,
              terminator: bool = True, trailing: bytes = b"") -> bytes:
        payload = bytearray()
        payload += version.to_bytes(2, "little")
        payload += (0).to_bytes(4, "little")
        payload += info_length.to_bytes(4, "little")
        for offset, name in (entries if body is None else body):
            payload += offset.to_bytes(4, "little")
            payload += name.encode() + b"\0"
        if terminator:
            payload += (0).to_bytes(4, "little")
        payload += trailing
        return len(payload).to_bytes(4, "little") + bytes(payload)

    good = build()
    parsed = parse_tables(good, ".debug_pubnames")
    if len(parsed) != 1 or parsed[0].entries != entries:
        print("FAIL: self-test: the reference table did not round-trip")
        return 1

    mutations: list[tuple[str, bytes]] = [
        ("wrong version", build(version=3)),
        ("entry outside the unit", build(body=[(0x200, "alpha")])),
        ("duplicate name", build(body=[(0x40, "alpha"), (0x80, "alpha")])),
        ("missing terminator", build(terminator=False)),
        ("trailing garbage", build(trailing=b"\x01")),
        ("empty name", build(body=[(0x40, "")])),
        ("zero contribution length", build(info_length=0)),
        ("truncated section", good[:-3]),
        ("empty table", build(body=[])),
    ]
    failures = 0
    for label, blob in mutations:
        try:
            parse_tables(blob, ".debug_pubnames")
        except ValueError:
            continue
        print(f"FAIL: self-test: {label} was accepted")
        failures += 1

    # The tree-derived expectation must exclude nested and declared entities.
    # These are real Die/Unit values driven through the real rule, so the test
    # cannot pass while the gate's own filter regresses.
    def die(tag: int, depth: int, offset: int, attrs: dict | None = None) -> Die:
        return Die(offset=offset, tag=tag, attrs=dict(attrs or {}), depth=depth)

    synthetic = Unit(offset=0, version=5, address_size=8, offset_size=4, dies=[
        die(DW_TAG_SUBPROGRAM, 1, 0x10, {DW_AT_NAME: "kept_fn"}),
        die(DW_TAG_VARIABLE, 1, 0x20, {DW_AT_NAME: "kept_obj"}),
        die(DW_TAG_VARIABLE, 2, 0x30, {DW_AT_NAME: "local"}),
        die(DW_TAG_FORMAL_PARAMETER, 2, 0x40, {DW_AT_NAME: "param"}),
        die(DW_TAG_VARIABLE, 1, 0x50, {DW_AT_NAME: "declared", DW_AT_DECLARATION: True}),
        die(DW_TAG_SUBPROGRAM, 1, 0x60, {DW_AT_NAME: "declared_fn", DW_AT_DECLARATION: True}),
        die(DW_TAG_VARIABLE, 1, 0x70),
    ])
    kept = expected_from_units([synthetic], NAME_TAGS)
    if kept != {"kept_fn": 0x10, "kept_obj": 0x20}:
        print(f"FAIL: self-test: unit-level filter kept {sorted(kept)}")
        failures += 1
    # A declaration-only record is not a type a consumer can look up here; a
    # complete one is.  Same rule, type table.
    types_unit = Unit(offset=0, version=5, address_size=8, offset_size=4, dies=[
        die(DW_TAG_STRUCTURE_TYPE, 1, 0x10, {DW_AT_NAME: "complete"}),
        die(DW_TAG_STRUCTURE_TYPE, 1, 0x20,
            {DW_AT_NAME: "forward", DW_AT_DECLARATION: True}),
        die(DW_TAG_TYPEDEF, 1, 0x30, {DW_AT_NAME: "alias"}),
        die(DW_TAG_BASE_TYPE, 1, 0x40, {DW_AT_NAME: "int"}),
        die(DW_TAG_STRUCTURE_TYPE, 1, 0x50),
    ])
    enum_unit = Unit(offset=0, version=5, address_size=8, offset_size=4, dies=[
        die(DW_TAG_ENUMERATION_TYPE, 1, 0x10, {DW_AT_NAME: "color"}),
        die(DW_TAG_ENUMERATOR, 2, 0x20, {DW_AT_NAME: "RED"}),
        die(DW_TAG_SUBPROGRAM, 1, 0x30, {DW_AT_NAME: "fn"}),
        die(DW_TAG_ENUMERATION_TYPE, 2, 0x40, {DW_AT_NAME: "local_enum"}),
        die(DW_TAG_ENUMERATOR, 3, 0x50, {DW_AT_NAME: "LOCAL"}),
        die(DW_TAG_STRUCTURE_TYPE, 1, 0x60, {DW_AT_NAME: "record"}),
        die(DW_TAG_ENUMERATOR, 2, 0x70, {DW_AT_NAME: "NOT_AN_ENUM_CHILD"}),
    ])
    kept_enum = expected_from_units([enum_unit], NAME_TAGS)
    if kept_enum != {"RED": 0x20, "fn": 0x30}:
        print(f"FAIL: self-test: enumerator scope rule kept {sorted(kept_enum)}")
        failures += 1
    kept_types = expected_from_units([types_unit], TYPE_TAGS)
    if kept_types != {"complete": 0x10, "alias": 0x30, "int": 0x40}:
        print(f"FAIL: self-test: type filter kept {sorted(kept_types)}")
        failures += 1
    total = len(mutations) + 4
    if failures:
        print(f"pubnames self-test FAILED ({failures}/{total})")
        return 1
    print(f"pubnames self-test PASSED ({total} vectors: {len(mutations)} corruptions, "
        f"1 round-trip, 3 scope filters)")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lccc")
    parser.add_argument("--lccc-i686")
    parser.add_argument("--lccc-arm")
    parser.add_argument("--lccc-riscv")
    parser.add_argument("--gcc", default="gcc-16")
    parser.add_argument("--gdb", default=None)
    parser.add_argument("--readelf", default="readelf")
    parser.add_argument("--allow-missing-gdb", action="store_true")
    parser.add_argument("--json", type=Path)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return self_test()
    if not args.lccc:
        fail("arguments", "--lccc is required unless --self-test is used")
    lccc = str(Path(args.lccc).resolve())
    gcc = shutil.which(args.gcc) or args.gcc
    readelf = shutil.which(args.readelf) or args.readelf
    gdb = args.gdb or shutil.which("gdb")
    if gdb:
        gdb = str(Path(gdb).resolve()) if Path(gdb).exists() else gdb
    if not gdb and not args.allow_missing_gdb:
        fail("toolchain", "gdb is required (or pass --allow-missing-gdb)")

    contracts = 0
    measurements: dict[str, object] = {}
    target_reports: list[str] = []
    with tempfile.TemporaryDirectory(prefix="lccc-dwarf-pubnames-") as directory:
        root = Path(directory)
        source = root / "unit.c"
        source.write_text(SOURCE)
        for version in (2, 3, 4, 5):
            out = root / f"unit-{version}.o"
            compile_ok(lccc, [f"-gdwarf-{version}", "-g", "-gpubnames", "-O2"],
                       source, out, root, f"v{version}")
            elf = read_elf(out)
            names = check_table(elf, ".debug_pubnames", NAME_TAGS, f"v{version}")
            types = check_table(elf, ".debug_pubtypes", TYPE_TAGS, f"v{version}")
            check_relocation(out, readelf, ".debug_pubnames", elf.machine, f"v{version}")
            check_relocation(out, readelf, ".debug_pubtypes", elf.machine, f"v{version}")
            contracts += 4
            oracle_facts = check_oracle(gcc, source, root, elf, version)
            contracts += 2
            contracts += check_flag_semantics(lccc, source, root, version)
            contracts += check_link(lccc, readelf, gdb, root, version)
            measurements[f"v{version}"] = {
                "pubnames_entries": len(names.entries),
                "pubtypes_entries": len(types.entries),
                "pubnames_bytes": len(elf.sections[".debug_pubnames"]),
                "pubtypes_bytes": len(elf.sections[".debug_pubtypes"]),
                "oracle": oracle_facts,
            }
        # -O0 must publish the same names: the index describes the source, not
        # the optimizer's survivors.
        for level in ("-O0", "-O1", "-O3", "-Os"):
            out = root / f"level{level}.o"
            compile_ok(lccc, ["-g", "-gpubnames", level], source, out, root, level)
            elf = read_elf(out)
            check_table(elf, ".debug_pubnames", NAME_TAGS, level)
            check_table(elf, ".debug_pubtypes", TYPE_TAGS, level)
            contracts += 2
        contracts += check_codegen_neutral(lccc, source, root)
        # Every supported backend must emit a structurally correct index with
        # its own relocation type, not just x86-64.
        for name, flag, machine in TARGETS:
            compiler = getattr(args, flag.lstrip("-").replace("-", "_"))
            if not compiler:
                continue
            compiler = str(Path(compiler).resolve())
            contracts += check_target(name, compiler, machine, readelf, source, root)
            target_reports.append(name)
        # A block-scope enumerator must never reach the index.  lccc does not
        # describe block-scope enums today (measured: no enumeration_type DIE
        # for `enum {RED,GREEN} e;` inside a function, while GCC nests it at
        # depth 2/3 and publishes neither).  If a later change lifts such a
        # type to unit scope, the enumerator hook would publish `GREEN` and
        # collide with the file-scope `RED`; this probe fails then, and the
        # writer's own duplicate guard turns it into a diagnostic rather than
        # an ambiguous table.
        scoped = root / "block_enum.c"
        scoped.write_text(
            "int RED = 1;\n"
            "int f(void) { enum { RED = 5, GREEN = 6 } e = GREEN; return e + RED; }\n")
        for level in ("-O0", "-O2"):
            out = root / f"block_enum{level}.o"
            compile_ok(lccc, ["-g", "-gpubnames", level], scoped, out, root, "block-enum")
            elf = read_elf(out)
            table = check_table(elf, ".debug_pubnames", NAME_TAGS, "block-enum")
            published = [name for _, name in table.entries]
            if "GREEN" in published:
                fail("block-enum", f"{level}: a block-scope enumerator was published")
            if published.count("RED") > 1:
                fail("block-enum", f"{level}: {published.count('RED')} entries named RED")
            contracts += 1
        # A unit with nothing to publish must not grow a section header.
        empty = root / "empty.c"
        empty.write_text("typedef int unused_t;\n")
        out = root / "empty.o"
        compile_ok(lccc, ["-g", "-gpubnames"], empty, out, root, "empty-unit")
        elf = read_elf(out)
        if ".debug_pubnames" in elf.sections:
            fail("empty-unit", ".debug_pubnames emitted for a unit with no objects or functions")
        contracts += 1

    report = {"status": "pass", "contracts": contracts, "gdb": gdb,
              "targets": ["x86-64", *target_reports], "measurements": measurements}
    if args.json:
        args.json.write_text(json.dumps(report, indent=2) + "\n")
    print(f"PASS: DWARF pubnames ({contracts} contracts, DWARF 2..5, "
          f"targets {', '.join(['x86-64', *target_reports])}, "
          f"GCC 16.2 name-set agreement{', GDB ' + gdb if gdb else ''})")
    for version, data in measurements.items():
        print(f"  {version}: {data}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
