#!/usr/bin/env python3
"""Audit the DIE vocabulary a unit carries, and ratchet it against the oracles.

A debugger can do exactly what the DIE tree tells it.  LCCC describes units and
subprograms (with their declared positions, ranges, return type and frame base)
and nothing else: there are no type DIEs and no variable or parameter DIEs, so a
consumer can break on ``file:line``, name a frame and unwind it, but cannot print
a local.  That gap is a *feature* gap, not a bug, and the only honest way to keep
a feature gap from silently growing -- or from being claimed closed without being
measured -- is to measure it on every run:

* what LCCC emits (unit, subprogram and their attributes) is pinned as a
  baseline, and a run that loses any of it fails;
* what gcc and clang emit for the same source is measured at the same time, and
  every tag or attribute they have and LCCC does not must be *declared* below
  with the reason it is absent -- a new one fails the gate until it is either
  implemented or added here deliberately;
* the two dialect rules LCCC does state are asserted directly: a subprogram
  states ``DW_AT_frame_base`` exactly when its dialect can decode
  ``DW_OP_call_frame_cfa`` (version 3 and newer), and never in a version 2 unit.

Every object is also required to read back cleanly: a malformed DIE tree shows
up in ``readelf`` as a warning or an error, never as a silent pass.

The measured census on the fixture below (x86-64, ``-O0``, versions 2..5):

=========================  ==============  ==============  ============
                           LCCC            gcc 16.2        clang 23.1.4
=========================  ==============  ==============  ============
DIE tags                   3               15              14
attributes                 15              30              27
parameter/variable DIEs    none            yes             yes
type DIEs                  none            yes             yes
``DW_AT_frame_base``       v3+             v3+ (v2: list)  all versions
=========================  ==============  ==============  ============
"""

from __future__ import annotations

import argparse
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

FIXTURE = """\
struct point { int x; int y; };
typedef struct point point_t;
enum color { RED, GREEN = 3 };
union value { int i; float f; };
static int counter;
int table[4] = {1, 2, 3, 4};
static int helper(point_t p) { return p.x + p.y; }
int compute(int a, char *name, double d) {
    struct point q = { a, 2 };
    int local = helper(q);
    enum color c = GREEN;
    for (int i = 0; i < local; i++) counter += i;
    return local + (int)d + (c == GREEN) + (name != 0);
}
int main(void) { return compute(1, "x", 2.0); }
"""

# What LCCC states today, measured from the fixture above at every dialect.
# Losing any of these is a regression: a consumer would stop seeing something it
# is entitled to.
BASELINE_TAGS = frozenset(
    {"DW_TAG_compile_unit", "DW_TAG_subprogram", "DW_TAG_unspecified_type"}
)
BASELINE_ATTRIBUTES = frozenset(
    {
        "DW_AT_comp_dir",
        "DW_AT_decl_column",
        "DW_AT_decl_file",
        "DW_AT_decl_line",
        "DW_AT_external",
        "DW_AT_frame_base",
        "DW_AT_high_pc",
        "DW_AT_language",
        "DW_AT_language_name",
        "DW_AT_language_version",
        "DW_AT_low_pc",
        "DW_AT_name",
        "DW_AT_producer",
        "DW_AT_stmt_list",
        "DW_AT_type",
    }
)

# The unit and subprogram attributes every dialect must state.
UNIT_ATTRIBUTES = frozenset(
    {
        "DW_AT_name",
        "DW_AT_comp_dir",
        "DW_AT_producer",
        "DW_AT_language",
        "DW_AT_low_pc",
        "DW_AT_high_pc",
        "DW_AT_stmt_list",
    }
)
SUBPROGRAM_ATTRIBUTES = frozenset(
    {
        "DW_AT_name",
        "DW_AT_external",
        "DW_AT_decl_file",
        "DW_AT_decl_line",
        "DW_AT_decl_column",
        "DW_AT_type",
        "DW_AT_low_pc",
        "DW_AT_high_pc",
    }
)

# What the reference compilers emit and LCCC does not, with the reason each one
# is absent.  This is the ratchet: a *new* oracle tag or attribute fails the gate
# until it is implemented, or listed here with its reason.
DEFERRED_TAGS = {
    # The type graph.  It needs the frontend's type information in the writer,
    # which today is fed by assembler records only (`.loc`-shaped directives
    # carry positions, not types), so this is a vocabulary to add, not a bug to
    # fix: `DW_TAG_base_type` for the scalar types, then the composites.
    "DW_TAG_base_type": "type graph: no type vocabulary in the lowering",
    "DW_TAG_pointer_type": "type graph: no type vocabulary in the lowering",
    "DW_TAG_array_type": "type graph: no type vocabulary in the lowering",
    "DW_TAG_subrange_type": "type graph: no type vocabulary in the lowering",
    "DW_TAG_structure_type": "type graph: no type vocabulary in the lowering",
    "DW_TAG_union_type": "type graph: no type vocabulary in the lowering",
    "DW_TAG_member": "type graph: no type vocabulary in the lowering",
    "DW_TAG_enumeration_type": "type graph: no type vocabulary in the lowering",
    "DW_TAG_enumerator": "type graph: no type vocabulary in the lowering",
    "DW_TAG_typedef": "type graph: no type vocabulary in the lowering",
    "DW_TAG_const_type": "type graph: no type vocabulary in the lowering",
    "DW_TAG_volatile_type": "type graph: no type vocabulary in the lowering",
    "DW_TAG_function_type": "type graph: no type vocabulary in the lowering",
    "DW_TAG_unspecified_parameters": "type graph: no type vocabulary in the lowering",
    # Variables need both the type graph (for `DW_AT_type`) and a location
    # channel in the writer (a stack offset per variable, which codegen knows
    # but the assembler records do not carry).
    "DW_TAG_variable": "variables: needs the type graph and a location channel",
    "DW_TAG_formal_parameter": "variables: needs the type graph and a location channel",
    "DW_TAG_lexical_block": "variables: needs block scopes in the lowering",
    "DW_TAG_label": "variables: no label vocabulary in the lowering",
}

DEFERRED_ATTRIBUTES = {
    "DW_AT_location": "variables: needs a location channel",
    "DW_AT_data_member_location": "type graph: member offsets are frontend data",
    "DW_AT_byte_size": "type graph: sizes are frontend data",
    "DW_AT_encoding": "type graph: scalar encodings are frontend data",
    "DW_AT_count": "type graph: array bounds are frontend data",
    "DW_AT_upper_bound": "type graph: array bounds are frontend data",
    "DW_AT_const_value": "type graph: enumerator values are frontend data",
    "DW_AT_prototyped": (
        "needs the declaration's parameter list in the writer; the DIE is built "
        "from symbol and `.loc` records, which carry no prototype information"
    ),
    "DW_AT_sibling": "an optimization: a consumer reads children without it",
    "DW_AT_GNU_all_call_sites": "gcc's call-site bookkeeping; no call-site DIEs",
    "DW_AT_GNU_all_tail_call_sites": "gcc's call-site bookkeeping; no call-site DIEs",
    "DW_AT_call_all_calls": "call-site bookkeeping; no call-site DIEs",
    "DW_AT_call_all_tail_calls": "call-site bookkeeping; no call-site DIEs",
    # clang 23's default for version 5 is the indexed string/address form, which
    # needs `.debug_str_offsets`/`.debug_addr` bases.  LCCC writes `strp`/`addr`
    # -- byte for byte what gcc writes for the same dialect -- and the two are
    # equally valid; adopting the indexed form is a size optimization, not a
    # capability.
    "DW_AT_addr_base": "clang's version-5 indexed form; LCCC writes gcc's strp/addr",
    "DW_AT_str_offsets_base": "clang's version-5 indexed form; LCCC writes gcc's strp/addr",
}

DIE_LINE = re.compile(r"^\s*<(\d+)><([0-9a-f]+)>:\s*(.*)$")
ATTRIBUTE_LINE = re.compile(r"^\s*<[0-9a-f]+>\s+(DW_AT_\w+)\s*:")
TAG = re.compile(r"DW_TAG_(\w+)")
SUSPICIOUS = ("Error:", "Warning:", "unable to", "corrupt", "truncated")


class GateFailure(Exception):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise GateFailure(message)


def resolve(command: str) -> str | None:
    """The command to invoke, absolute: the compiles below run in a temporary
    directory, so a relative path to a freshly built compiler would not resolve."""
    found = shutil.which(command)
    if found:
        return str(Path(found).resolve())
    path = Path(command)
    return str(path.resolve()) if path.exists() else None


def run(command: list[str], cwd: Path) -> None:
    completed = subprocess.run(command, cwd=cwd, capture_output=True, text=True)
    if completed.returncode != 0:
        raise GateFailure(
            f"{' '.join(command)} failed: {completed.stderr.strip() or completed.stdout.strip()}"
        )


def census(readelf: str, obj: Path) -> dict[str, frozenset[str]]:
    """The tags and attributes `obj` carries, and the ones on its subprograms."""
    completed = subprocess.run(
        [readelf, "--debug-dump=info", str(obj)], capture_output=True, text=True
    )
    require(completed.returncode == 0, f"{obj.name}: readelf failed: {completed.stderr}")
    for marker in SUSPICIOUS:
        require(
            marker not in completed.stdout and marker not in completed.stderr,
            f"{obj.name}: readelf reports {marker!r}: "
            f"{completed.stderr.strip() or completed.stdout.strip()[:400]}",
        )
    tags: set[str] = set()
    attributes: set[str] = set()
    subprogram: set[str] = set()
    unit: set[str] = set()
    current: str | None = None
    for line in completed.stdout.split("\n"):
        die = DIE_LINE.match(line)
        if die:
            found = TAG.search(die.group(3))
            current = f"DW_TAG_{found.group(1)}" if found else None
            if current:
                tags.add(current)
            continue
        attribute = ATTRIBUTE_LINE.match(line)
        if attribute:
            name = attribute.group(1)
            attributes.add(name)
            if current == "DW_TAG_subprogram":
                subprogram.add(name)
            elif current == "DW_TAG_compile_unit":
                unit.add(name)
    return {
        "tags": frozenset(tags),
        "attributes": frozenset(attributes),
        "subprogram": frozenset(subprogram),
        "unit": frozenset(unit),
    }


def compile_fixture(
    compiler: str, version: int, source: Path, out: Path, cwd: Path
) -> bool:
    """Compile the fixture, answering whether the compiler accepts the dialect."""
    completed = subprocess.run(
        [compiler, "-g", f"-gdwarf-{version}", "-O0", "-c", str(source), "-o", str(out)],
        cwd=cwd,
        capture_output=True,
        text=True,
    )
    return completed.returncode == 0 and out.exists()


def describe(missing: set[str]) -> str:
    return ", ".join(sorted(missing)) if missing else "none"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lccc", default=os.environ.get("CCC", "target/fastbuild/lccc"))
    parser.add_argument("--gcc", default=os.environ.get("GCC16", "gcc"))
    parser.add_argument("--clang", default=os.environ.get("CLANG", "clang"))
    parser.add_argument("--readelf", default=os.environ.get("READELF", "readelf"))
    parser.add_argument(
        "--versions", default="2,3,4,5", help="comma-separated dialects to compare"
    )
    args = parser.parse_args()

    readelf = resolve(args.readelf)
    require(readelf is not None, f"readelf {args.readelf!r} not found")
    lccc = resolve(args.lccc)
    require(lccc is not None, f"compiler {args.lccc!r} not found")
    versions = [int(value) for value in args.versions.split(",") if value.strip()]
    oracles = [
        (name, resolve(command))
        for name, command in (("gcc", args.gcc), ("clang", args.clang))
        if resolve(command) is not None
    ]
    require(oracles, "no reference compiler found: the census has nothing to compare")

    ours_tags: set[str] = set()
    ours_attributes: set[str] = set()
    oracle_tags: dict[str, set[str]] = {name: set() for name, _ in oracles}
    oracle_attributes: dict[str, set[str]] = {name: set() for name, _ in oracles}

    with tempfile.TemporaryDirectory(prefix="lccc-die-census-") as tmp:
        work = Path(tmp).resolve()
        source = work / "census.c"
        source.write_text(FIXTURE, encoding="utf-8")
        for version in versions:
            obj = work / f"ours-{version}.o"
            require(
                compile_fixture(lccc, version, source, obj, work),
                f"-gdwarf-{version}: {lccc} rejected the fixture",
            )
            found = census(readelf, obj)
            ours_tags |= found["tags"]
            ours_attributes |= found["attributes"]
            label = f"-gdwarf-{version}"
            require(
                found["unit"] >= UNIT_ATTRIBUTES,
                f"{label}: the unit is missing {describe(set(UNIT_ATTRIBUTES) - set(found['unit']))}",
            )
            expected = set(SUBPROGRAM_ATTRIBUTES)
            if version >= 3:
                # `DW_OP_call_frame_cfa` is a version 3 opcode: only a unit that
                # defines it may state the frame base with it.
                expected.add("DW_AT_frame_base")
            else:
                require(
                    "DW_AT_frame_base" not in found["subprogram"],
                    f"{label}: a version 2 subprogram states DW_AT_frame_base, "
                    f"but DW_OP_call_frame_cfa is a version 3 opcode",
                )
            require(
                found["subprogram"] >= expected,
                f"{label}: the subprograms are missing "
                f"{describe(expected - set(found['subprogram']))}",
            )
            for name, compiler in oracles:
                reference = work / f"{name}-{version}.o"
                if not compile_fixture(compiler, version, source, reference, work):
                    print(f"SKIP {name} {label}: the oracle rejected the dialect")
                    continue
                compared = census(readelf, reference)
                oracle_tags[name] |= compared["tags"]
                oracle_attributes[name] |= compared["attributes"]
            print(
                f"PASS {label}: unit and subprogram vocabulary, and the "
                f"{'version 3+' if version >= 3 else 'version 2 (absent)'} frame base"
            )

    require(
        ours_tags >= BASELINE_TAGS,
        f"the DIE tags lost {describe(set(BASELINE_TAGS) - ours_tags)}",
    )
    require(
        ours_attributes >= BASELINE_ATTRIBUTES,
        f"the attribute vocabulary lost "
        f"{describe(set(BASELINE_ATTRIBUTES) - ours_attributes)}",
    )

    for name, _ in oracles:
        unexpected_tags = oracle_tags[name] - ours_tags - set(DEFERRED_TAGS)
        unexpected_attributes = (
            oracle_attributes[name] - ours_attributes - set(DEFERRED_ATTRIBUTES)
        )
        require(
            not unexpected_tags,
            f"{name} emits DIE tags LCCC does not and that are not declared "
            f"deferred: {describe(unexpected_tags)} -- implement them, or record "
            f"the reason in DEFERRED_TAGS",
        )
        require(
            not unexpected_attributes,
            f"{name} emits attributes LCCC does not and that are not declared "
            f"deferred: {describe(unexpected_attributes)} -- implement them, or "
            f"record the reason in DEFERRED_ATTRIBUTES",
        )
        print(
            f"{name}: {len(oracle_tags[name])} tags, {len(oracle_attributes[name])} "
            f"attributes; deferred: "
            f"{len(oracle_tags[name] - ours_tags)} tags, "
            f"{len(oracle_attributes[name] - ours_attributes)} attributes"
        )

    print(
        f"DWARF DIE census: PASS ({len(ours_tags)} tags, "
        f"{len(ours_attributes)} attributes; every oracle-only one is declared)"
    )
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except GateFailure as failure:
        print(f"FAIL: {failure}")
        sys.exit(1)
