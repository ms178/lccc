#!/usr/bin/env python3
"""Audit the DIE vocabulary a unit carries, and ratchet it against the oracles.

A debugger can do exactly what the DIE tree tells it. LCCC describes units and
subprograms (declared positions, ranges, return type, frame base), the C type
graph the fixture uses, global variables with their addresses, and named formal
parameter lists. It does NOT describe enumerated types, enumerator constants,
lexical block scopes or call-site bookkeeping, and it does not give optimized
locals or parameters a location -- GDB 18.1 reports those as
``<optimized out>`` while GCC 16.2 and Clang 23.1.5 resolve them through
``DW_AT_location`` plus entry-value expressions. That remaining gap is a
*feature* gap, not a bug; the only honest way to keep it from silently growing
-- or from being claimed closed without being measured -- is to measure it on
every run:

* what LCCC emits is pinned as a baseline, and a run that loses ANY of it fails;
* what gcc and clang emit for the same source is measured at the same time, and
  every tag or attribute they have and LCCC does not must be *declared* below
  with the reason it is absent -- a new one fails the gate until implemented or
  added here deliberately;
* the declaration list is itself checked: an entry naming something LCCC
  already emits is a lie that would mask a regression in BOTH checks, so
  ``tests/corpus/test_dwarf_die_census_ratchet.py`` asserts the baseline and
  the deferred sets are disjoint and that the measured gap matches them;
* the two dialect rules LCCC does state are asserted directly: a subprogram
  states ``DW_AT_frame_base`` exactly when its dialect can decode
  ``DW_OP_call_frame_cfa`` (version 3 and newer), and never in a version 2 unit.

Every object is also required to read back cleanly: a malformed DIE tree shows
up in ``readelf`` as a warning or an error, never as a silent pass.

The measured census on the fixture below (x86-64, ``-std=c17 -O0``, versions
2..5, readelf from binutils 2.47):

=========================  ==============  ==============  ============
                           LCCC            gcc 16.2        clang 23.1.5
=========================  ==============  ==============  ============
DIE tags                   12              14              14
attributes                 20              27              21
parameter/variable DIEs    yes             yes             yes
type DIEs                  partial         yes             yes
``DW_AT_location``         globals only    yes             yes
``DW_AT_frame_base``       v3+             v3+ (v2: list)  all versions
=========================  ==============  ==============  ============

An earlier revision of this gate claimed LCCC emitted only three tags and no
variable or parameter DIEs, and pre-declared fifteen tags and five attributes as
"deferred" that the compiler had since started emitting. Both halves of the
ratchet then ignored those names: losing the entire type graph or every
parameter DIE would have passed. The baseline below is the measured union, and
the deferral list is the measured difference; neither is a wish list.
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
    {
        "DW_TAG_array_type",
        "DW_TAG_base_type",
        "DW_TAG_compile_unit",
        "DW_TAG_formal_parameter",
        "DW_TAG_member",
        "DW_TAG_pointer_type",
        "DW_TAG_structure_type",
        "DW_TAG_subprogram",
        "DW_TAG_subrange_type",
        "DW_TAG_typedef",
        "DW_TAG_unspecified_type",
        "DW_TAG_variable",
    }
)
BASELINE_ATTRIBUTES = frozenset(
    {
        "DW_AT_byte_size",
        "DW_AT_comp_dir",
        "DW_AT_data_member_location",
        "DW_AT_decl_column",
        "DW_AT_decl_file",
        "DW_AT_decl_line",
        "DW_AT_encoding",
        "DW_AT_external",
        "DW_AT_frame_base",
        "DW_AT_high_pc",
        "DW_AT_language",
        "DW_AT_language_name",
        "DW_AT_language_version",
        "DW_AT_location",
        "DW_AT_low_pc",
        "DW_AT_name",
        "DW_AT_producer",
        "DW_AT_stmt_list",
        "DW_AT_type",
        "DW_AT_upper_bound",
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
    # Measured: the fixture declares `enum color { RED, GREEN = 3 }`, and both
    # oracles emit an enumeration_type with two enumerators. LCCC's unit spec
    # can represent DwarfType::Enum (dwarf_spec.rs), but the frontend spec
    # builder does not add one for a C enum, so no enumerator DIE follows.
    "DW_TAG_enumeration_type": "C enum types are absent from the unit spec (measured)",
    "DW_TAG_enumerator": "enumerator constants follow DW_TAG_enumeration_type",
    "DW_TAG_lexical_block": "block scopes are absent from the unit spec (measured)",
}

# The measured attribute difference. Every name LCCC already states belongs in
# BASELINE_ATTRIBUTES instead: listing it here too would let a regression pass.
DEFERRED_ATTRIBUTES = {
    "DW_AT_GNU_all_call_sites": "gcc's call-site bookkeeping; no call-site DIEs",
    "DW_AT_GNU_all_tail_call_sites": "gcc's call-site bookkeeping; no call-site DIEs",
    "DW_AT_call_all_calls": "call-site bookkeeping; no call-site DIEs",
    "DW_AT_call_all_tail_calls": "call-site bookkeeping; no call-site DIEs",
    "DW_AT_const_value": "enumerator constants are not emitted (measured)",
    # clang 23 states an array's element count; LCCC states its upper bound,
    # which is what gcc emits for the same dialect. Both are valid.
    "DW_AT_count": "clang's DWARF-5 array-count form; LCCC writes gcc's upper_bound",
    "DW_AT_prototyped": "subprogram DIEs do not state the prototype flag (measured)",
    "DW_AT_sibling": "an optimization: a consumer reads children without it",
    # clang 23's default for version 5 is the indexed string/address form, which
    # needs `.debug_str_offsets`/`.debug_addr` bases. LCCC writes `strp`/`addr`
    # -- byte for byte what gcc writes for the same dialect -- and the two are
    # equally valid; adopting the indexed form is a size optimization, not a
    # capability.
    "DW_AT_addr_base": "clang's version-5 indexed form; LCCC writes gcc's strp/addr",
    "DW_AT_str_offsets_base": "clang's version-5 indexed form; LCCC writes gcc's strp/addr",
}

# A deferral and a baseline entry cannot describe the same name: the deferral
# would make the oracle-only check ignore it while the baseline claims it is
# present. Assert the invariant at import time so a stale edit cannot land.
assert not (set(DEFERRED_TAGS) & set(BASELINE_TAGS)), \
    "a DEFERRED_TAGS entry is already in BASELINE_TAGS"
assert not (set(DEFERRED_ATTRIBUTES) & set(BASELINE_ATTRIBUTES)), \
    "a DEFERRED_ATTRIBUTES entry is already in BASELINE_ATTRIBUTES"

DIE_LINE = re.compile(r"^\s*<(\d+)><([0-9a-f]+)>:\s*(.*)$")
ATTRIBUTE_LINE = re.compile(r"^\s*<[0-9a-f]+>\s+(DW_AT_\w+)\s*:")
TAG = re.compile(r"DW_TAG_(\w+)")
# A decode failure: the section we wrote cannot be read back.
BROKEN = ("Error:", "unable to", "corrupt", "truncated")
# A *knowledge* limit of this readelf: it does not know an attribute or form the
# dialect defines (an older binutils meeting `DW_AT_language_name`, say).  The
# encoding is fine and the attribute is still decoded, just not named, so this is
# reported rather than treated as a defect: a gate that failed here would fail on
# every host whose binutils trails the specification, not on anything LCCC did.
UNKNOWN_TO_READELF = ("nrecognized", "nknown", "not supported", "unsupported")


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


def census(
    readelf: str, obj: Path, *, require_clean: bool = True
) -> dict[str, frozenset[str]]:
    """The tags and attributes `obj` carries, and the ones on its subprograms.

    ``require_clean`` is set for LCCC's own objects -- a diagnostic about a
    section we wrote is a defect -- and cleared for a reference object, where a
    diagnostic means only that this ``readelf`` cannot decode another producer's
    encoding; the vocabulary it did decode is still worth comparing.
    """
    completed = subprocess.run(
        [readelf, "--debug-dump=info", str(obj)], capture_output=True, text=True
    )
    require(completed.returncode == 0, f"{obj.name}: readelf failed: {completed.stderr}")
    if require_clean:
        for marker in BROKEN:
            require(
                marker not in completed.stdout and marker not in completed.stderr,
                f"{obj.name}: readelf reports {marker!r}: "
                f"{completed.stderr.strip() or completed.stdout.strip()[:400]}",
            )
        for line in completed.stderr.splitlines():
            if "arning:" in line:
                if any(limit in line for limit in UNKNOWN_TO_READELF):
                    print(f"note: {obj.name}: {line.strip()}")
                    continue
                raise GateFailure(f"{obj.name}: readelf reports {line.strip()}")
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
    """Compile the fixture, answering whether the compiler accepts the dialect.

    The standard is pinned: a unit states `DW_AT_language_name`/`_version` for
    C17 and newer, so a run without `-std=` measures whatever the host's default
    happens to be -- and the vocabulary would differ between two hosts running
    the same gate.  Every compiler is asked the same question with the same
    flags.
    """
    completed = subprocess.run(
        [
            compiler,
            "-g",
            f"-gdwarf-{version}",
            "-std=c17",
            "-O0",
            "-c",
            str(source),
            "-o",
            str(out),
        ],
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
    parser.add_argument("--gcc", default=os.environ.get("GCC16", "gcc-16"))
    parser.add_argument("--clang", default=os.environ.get("CLANG23", "clang-23"))
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
                compared = census(readelf, reference, require_clean=False)
                if not compared["tags"]:
                    print(
                        f"SKIP {name} {label}: this readelf decoded no DIE from the "
                        f"reference object, so it cannot be compared"
                    )
                    continue
                oracle_tags[name] |= compared["tags"]
                oracle_attributes[name] |= compared["attributes"]
            print(
                f"PASS {label}: unit and subprogram vocabulary, and the "
                f"{'version 3+' if version >= 3 else 'version 2 (absent)'} frame base"
            )

    # A deferral may never name something LCCC actually emits. The static
    # disjointness assertion above covers the pinned baseline; this one covers
    # *new* capabilities: if the compiler starts emitting a DIE that is still
    # listed as deferred, both halves of the gate would ignore it and a later
    # regression would pass silently. Requiring the measured sets to be clean
    # makes the deferral list self-invalidating as the compiler improves.
    require(
        not (ours_tags & set(DEFERRED_TAGS)),
        f"DEFERRED_TAGS names DIEs LCCC now emits: "
        f"{describe(ours_tags & set(DEFERRED_TAGS))} -- move them to BASELINE_TAGS",
    )
    require(
        not (ours_attributes & set(DEFERRED_ATTRIBUTES)),
        f"DEFERRED_ATTRIBUTES names attributes LCCC now emits: "
        f"{describe(ours_attributes & set(DEFERRED_ATTRIBUTES))} -- "
        f"move them to BASELINE_ATTRIBUTES",
    )

    # The baseline is what LCCC states, measured with a readelf that knows every
    # name in it.  A host whose readelf predates an attribute (binutils before
    # `DW_AT_language_name` knew its name, say) cannot *see* it in any object --
    # including the reference compilers' -- so a baseline name that is missing
    # from ours and from every oracle as well is a limit of the reader, reported
    # and not failed.  A name the oracles do carry and we do not is a real loss.
    oracle_vocabulary: set[str] = set()
    for name, _ in oracles:
        oracle_vocabulary |= oracle_tags[name] | oracle_attributes[name]
    lost_tags = set(BASELINE_TAGS) - ours_tags
    lost_attributes = set(BASELINE_ATTRIBUTES) - ours_attributes
    for label, lost in (("tags", lost_tags), ("attributes", lost_attributes)):
        unreadable = {name for name in lost if name not in oracle_vocabulary}
        for name in sorted(unreadable):
            print(f"note: {readelf} cannot name {name}: absent here and in every reference")
        lost -= unreadable
        require(
            not lost,
            f"the DIE {label} lost {describe(lost)}",
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

    version = subprocess.run([readelf, "--version"], capture_output=True, text=True)
    print(
        f"DWARF DIE census: PASS ({len(ours_tags)} tags, "
        f"{len(ours_attributes)} attributes; every oracle-only one is declared; "
        f"{version.stdout.splitlines()[0] if version.stdout else readelf})"
    )
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (GateFailure, OSError, subprocess.SubprocessError) as failure:
        print(f"FAIL: {failure}", file=sys.stderr)
        # A workflow's log is not always readable from where a failure is
        # triaged; an annotation carries the message to the pull request itself.
        # Only Actions reads this syntax, so nothing changes locally.
        if os.environ.get("GITHUB_ACTIONS") == "true":
            print(f"::error title=dwarf DIE census::{failure}")
        sys.exit(1)
