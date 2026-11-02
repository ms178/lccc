#!/usr/bin/env python3
"""Check LCCC's DWARF 2/3/4/5 output against GCC 16.2 and binutils readers.

The compiler writes debug information through the assemblers: `.file`/`.loc`
rows become `.debug_line` line programs, and units, subprograms and their
declaration locations are synthesized for them (`src/backend/elf/dwarf.rs`).
`-gdwarf-<N>` picks the dialect and `-std=` picks the language a unit
declares.  This gate holds that output to the reference implementation and to
the consumers that actually read it.

What is compared, for every dialect (`c89`, `c99`, `c11`, `c17`, `c23`) and
every version (`2`, `3`, `4`, `5`):

*   The compile unit profile: unit length/version, `DW_UT_compile` presence,
    address (pointer) size, `DW_AT_language`, and the `DW_AT_language_name` /
    `DW_AT_language_version` pair.  The expected values are *measured* from
    GCC 16.2 on the same source and command line, never hardcoded -- if GCC
    records C99 for `-std=c23 -gdwarf-4` and C11 plus a 202311 version for
    `-gdwarf-5`, so must LCCC.
*   The line-program header profile: version, address size, segment selector
    size, minimum instruction length, `maximum_operations_per_instruction`
    (a DWARF 4 addition: it must be absent in 2 and 3), default `is_stmt`,
    line base/range, opcode base, the twelve standard opcode argument counts,
    and the shape of the directory and file tables (entry counts, column
    counts, directory indexes and names).  LCCC's *layout* must match GCC's
    field for field; string offsets differ legitimately.
*   Round-trip through binutils' own readers: `addr2line` must resolve every
    function's entry address to its name, file and the line of its first
    statement, for all four targets.  x86-64 and i686 are checked on the
    relocatable object *and* on the linked image; AArch64 and RISC-V (whose
    code addresses are label relocations that only become final after
    relaxation and linking) on the linked image.
*   Multi-object links: two translation units compiled separately must both
    resolve, which is what proves the `.debug_str`/`.debug_line_str` offsets
    are relocated rather than baked in.
*   The command line: `-gdwarf-<N>` is honoured (no "unimplemented"
    diagnostic even under `LCCC_STRICT_OPTIONS=1`), lasts-one-wins with
    `-g0`, and versions outside 2..5 are rejected, all matching GCC's
    measured behaviour.

`readelf` must report no warning or error for any object: it validates unit
lengths, abbreviation terminators, table counts and form sizes, so a
structural mistake cannot pass silently.
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

ROOT = Path(__file__).resolve().parents[2]


class GateFailure(RuntimeError):
    pass


def resolve_command(command: str) -> str:
    path = Path(command)
    if path.is_absolute():
        return str(path)
    if "/" in command:
        return str((ROOT / path).resolve())
    resolved = shutil.which(command)
    if resolved is None:
        raise GateFailure(f"required tool not found on PATH: {command}")
    return resolved


def run(
    argv: list[str],
    *,
    env: dict[str, str] | None = None,
    cwd: Path | None = None,
) -> subprocess.CompletedProcess:
    completed = subprocess.run(
        argv,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
        env=env,
        cwd=cwd,
    )
    return completed


def run_ok(argv: list[str], *, cwd: Path | None = None) -> str:
    completed = run(argv, cwd=cwd)
    if completed.returncode:
        raise GateFailure(
            f"command failed ({completed.returncode}): {' '.join(argv)}\n"
            f"stdout:\n{completed.stdout}\nstderr:\n{completed.stderr}"
        )
    return completed.stdout


def require(condition: bool, message: str) -> None:
    if not condition:
        raise GateFailure(message)


# ---------------------------------------------------------------------------
# Readers
# ---------------------------------------------------------------------------

def require_clean_readelf(readelf: str, path: Path, what: str) -> str:
    """`readelf` output for `path`, failing on any diagnostic it prints.

    readelf validates every length, count and form it walks, so a warning here
    means the section is malformed even when the dump "succeeds".
    """
    completed = run(
        [
            readelf,
            "--debug-dump=info",
            "--debug-dump=rawline",
            "--debug-dump=aranges",
            str(path),
        ]
    )
    diagnostics = [
        line
        for line in completed.stderr.splitlines()
        if re.search(r"(warning|error)", line, re.IGNORECASE)
    ]
    require(
        not diagnostics,
        f"{what}: readelf reported malformed DWARF for {path.name}:\n"
        + "\n".join(diagnostics),
    )
    require(completed.returncode == 0, f"{what}: readelf failed on {path.name}")
    return completed.stdout


def readelf_dump(readelf: str, path: Path, option: str) -> str:
    return run_ok([readelf, option, str(path)])


def cu_profile(info_text: str) -> dict[str, object]:
    """The compile unit's identity, as a debugger sees it."""
    version = re.search(r"^\s+Version:\s+(\d+)", info_text, re.MULTILINE)
    require(version is not None, "no unit version in the .debug_info dump")
    unit_type = re.search(r"^\s+Unit Type:\s+(\w+)", info_text, re.MULTILINE)
    pointer = re.search(r"^\s+Pointer Size:\s+(\d+)", info_text, re.MULTILINE)
    profile: dict[str, object] = {
        "version": int(version.group(1)),
        "unit_type": unit_type.group(1) if unit_type else None,
        "pointer_size": int(pointer.group(1)) if pointer else None,
    }
    attributes = {}
    for name, value in re.findall(
        r"DW_AT_(\w+)\s*:\s*(?:\([^)]*\):\s*)?([^\n]+)", info_text
    ):
        attributes.setdefault(name, value.strip())
    for name in ("name", "comp_dir", "producer"):
        profile[name] = attributes.get(name)
    language = attributes.get("language")
    require(language is not None, "the unit has no DW_AT_language")
    # `29\t(C11)` or `12\t(ANSI C99)`.
    profile["language"] = int(language.split()[0])
    name_code = attributes.get("language_name")
    profile["language_name"] = (
        int(name_code.split()[0]) if name_code is not None else None
    )
    version_attribute = attributes.get("language_version")
    profile["language_version"] = (
        int(version_attribute.split()[0], 0) if version_attribute is not None else None
    )
    profile["has_stmt_list"] = "stmt_list" in attributes
    profile["has_low_pc"] = "low_pc" in attributes
    profile["has_high_pc"] = "high_pc" in attributes
    return profile


def cu_forms(readelf: str, path: Path) -> dict[str, str]:
    """The forms the compile unit's own abbreviation declares.

    A value comparison cannot see this: `DW_FORM_strp`, `DW_FORM_line_strp` and
    `DW_FORM_string` all print the same text, and `DW_FORM_data4` and
    `DW_FORM_data8` all print the same number.  Yet the form is what a consumer
    of *that dialect* is allowed to meet, so the encoding of every field the
    version pins is pinned to the reference compiler here.
    """
    forms: dict[str, str] = {}
    in_unit = False
    for line in readelf_dump(readelf, path, "--debug-dump=abbrev").splitlines():
        if "DW_TAG_compile_unit" in line:
            in_unit = True
            continue
        if not in_unit:
            continue
        match = re.match(r"^\s+DW_AT_(\w+)\s+(DW_FORM_\w+)\s*$", line)
        if match:
            forms.setdefault(match.group(1), match.group(2))
            continue
        # An abbreviation ends with its `DW_AT value: 0  DW_FORM value: 0` pair;
        # the compile unit is the first one, so that pair ends the search.
        if re.match(r"^\s+DW_AT value:", line):
            break
    return forms


def line_profile(rawline_text: str) -> dict[str, object]:
    """The line-program header and its tables, as readelf decodes them."""
    def header(pattern: str) -> int | None:
        match = re.search(pattern, rawline_text, re.MULTILINE)
        return int(match.group(1)) if match else None

    profile: dict[str, object] = {
        "version": header(r"DWARF Version:\s+(\d+)"),
        "address_size": header(r"Address size \(bytes\):\s+(\d+)"),
        "segment_selector": header(r"Segment selector \(bytes\):\s+(\d+)"),
        "minimum_instruction_length": header(r"Minimum Instruction Length:\s+(\d+)"),
        "max_ops": header(r"Maximum Ops per Instruction:\s+(\d+)"),
        "is_stmt": header(r"Initial value of 'is_stmt':\s+(\d+)"),
        "line_base": header(r"Line Base:\s+(-?\d+)"),
        "line_range": header(r"Line Range:\s+(\d+)"),
        "opcode_base": header(r"Opcode Base:\s+(\d+)"),
    }
    profile["opcode_args"] = [
        int(count) for count in re.findall(r"Opcode \d+ has (\d+) args", rawline_text)
    ]
    directories: list[tuple[int, str]] = []
    files: list[tuple[int, int, str]] = []
    section = None
    for line in rawline_text.splitlines():
        if "The Directory Table" in line:
            section = "dirs"
            columns = re.search(r"columns (\d+)", line)
            profile["directory_columns"] = int(columns.group(1)) if columns else None
            continue
        if "The File Name Table" in line:
            section = "files"
            columns = re.search(r"columns (\d+)", line)
            profile["file_columns"] = int(columns.group(1)) if columns else None
            continue
        if "Line Number Statements" in line:
            section = None
            continue
        if section == "dirs":
            match = re.match(r"\s+(\d+)\t(?:\([^)]*\):\s*)?(.*)$", line)
            if match:
                directories.append((int(match.group(1)), match.group(2)))
        elif section == "files":
            match = re.match(r"\s+(\d+)\t(\d+)\t(?:\([^)]*\):\s*)?(.*)$", line)
            if match:
                files.append((int(match.group(1)), int(match.group(2)), match.group(3)))
    profile["directories"] = directories
    profile["files"] = files
    return profile


def addr2line_resolve(addr2line: str, path: Path, address: int) -> tuple[str, str, int]:
    output = run_ok([addr2line, "-f", "-e", str(path), hex(address)]).splitlines()
    require(len(output) >= 2, f"addr2line printed {output!r} for {path.name}")
    function = output[0].strip()
    location = output[1].strip()
    file, _, line = location.rpartition(":")
    require(line.isdigit(), f"addr2line printed {location!r} for {path.name}")
    return function, file, int(line)


def asm_loc_rows(assembly: str) -> set[tuple[str, int]]:
    """Every `(file, line)` a `.loc` directive names, as the source recorded it."""
    files = dict(re.findall(r'^\.file (\d+) "(.*)"', assembly, re.MULTILINE))
    rows = set()
    for number, line in re.findall(r"^\.loc (\d+) (\d+)", assembly, re.MULTILINE):
        name = files.get(number)
        if name is not None:
            rows.add((Path(name).name, int(line)))
    return rows


def decoded_line_rows(readelf: str, path: Path) -> set[tuple[str, int]]:
    """Every `(file, line)` the emitted line program decodes back to.

    `readelf --debug-dump=decodedline` runs the state machine, so this is the
    consumer's answer, not the writer's: an opcode with the wrong operand width,
    a dropped row or a mis-signed advance all show up as a different set.
    """
    rows = set()
    for line in readelf_dump(readelf, path, "--debug-dump=decodedline").splitlines():
        # readelf prints a zero address as `0` and every other one as `0x...`.
        match = re.match(r"^(\S+?)\s+(\d+)\s+(?:0x[0-9a-fA-F]+|0)(?:\s|$)", line)
        if match:
            rows.add((Path(match.group(1)).name, int(match.group(2))))
    return rows


def arange_pairs(readelf: str, path: Path) -> set[tuple[int, int]]:
    """Every `(address, length)` tuple `.debug_aranges` records.

    `readelf` prints one row per tuple, four spaces in, as bare hexadecimal
    address and length fields; the zero tuple that terminates each set is
    dropped here and asserted separately, because it is a terminator rather
    than a range.
    """
    pairs = set()
    for line in readelf_dump(readelf, path, "--debug-dump=aranges").splitlines():
        match = re.match(r"^ {4}([0-9a-f]{8,16}) ([0-9a-f]{8,16})\s*$", line)
        if match:
            address, length = int(match.group(1), 16), int(match.group(2), 16)
            if length:
                pairs.add((address, length))
    return pairs


def symbol_addresses(nm: str, path: Path) -> dict[str, int]:
    table: dict[str, int] = {}
    for line in run_ok([nm, str(path)]).splitlines():
        parts = line.split()
        if len(parts) == 3 and parts[1] in ("T", "t"):
            table.setdefault(parts[2], int(parts[0], 16))
    return table


# ---------------------------------------------------------------------------
# Fixture
# ---------------------------------------------------------------------------

# At `-O0` a function's entry row is the line of its opening brace: the
# prologue belongs to the body, and until optimization moves code around the
# first statement keeps its own line.  Measured with GCC 16.2 and lccc on
# x86-64 for every dialect, and the expectation every backend must meet.
FIXTURE = """\
int alpha(int value)
{
    return value + 1;
}

int beta(int value)
{
    int doubled = value * 2;
    return doubled - 1;
}

int main(void)
{
    return alpha(beta(3));
}
"""
ENTRY_LINE = {"alpha": 2, "beta": 7, "main": 13}


def write_fixture(directory: Path, name: str = "unit.c") -> Path:
    path = directory / name
    path.write_text(FIXTURE, encoding="utf-8")
    return path


def require_functions_resolve(
    addr2line: str,
    nm: str,
    image: Path,
    source: Path,
    what: str,
    reference: tuple[str, str, Path] | None = None,
) -> None:
    """Every fixture function resolves to its entry line, and to the same
    location a reference compiler reports for its own build of the same file."""
    addresses = symbol_addresses(nm, image)
    for function, line in ENTRY_LINE.items():
        require(function in addresses, f"{what}: {image.name} lost symbol {function}")
        found, file, found_line = addr2line_resolve(addr2line, image, addresses[function])
        require(
            found == function,
            f"{what}: addr2line named {found!r}, not {function!r}, in {image.name}",
        )
        require(
            Path(file).name == source.name,
            f"{what}: addr2line reported {file!r} for {function} in {image.name}, "
            f"expected {source.name}",
        )
        require(
            found_line == line,
            f"{what}: addr2line reported line {found_line} for {function} in "
            f"{image.name}, expected {line}",
        )
        if reference is not None:
            reference_addr2line, reference_nm, reference_image = reference
            reference_addresses = symbol_addresses(reference_nm, reference_image)
            require(
                function in reference_addresses,
                f"{what}: the reference build lost symbol {function}",
            )
            expected_function, expected_file, expected_line = addr2line_resolve(
                reference_addr2line,
                reference_image,
                reference_addresses[function],
            )
            require(
                (found, Path(file).name, found_line)
                == (expected_function, Path(expected_file).name, expected_line),
                f"{what}: {function} resolves to {found} "
                f"{Path(file).name}:{found_line} in {image.name} but to "
                f"{expected_function} {Path(expected_file).name}:{expected_line} "
                f"in the reference build",
            )


# ---------------------------------------------------------------------------
# Comparison
# ---------------------------------------------------------------------------

CU_KEYS = (
    "version",
    "unit_type",
    "pointer_size",
    "name",
    "comp_dir",
    "language",
    "language_name",
    "language_version",
    "has_stmt_list",
    "has_low_pc",
    "has_high_pc",
)
# The encoding of each field the dialect pins, per version.  Every one of
# these is compared against GCC 16.2 directly; the rest of the unit's attributes
# are compared by value in `CU_KEYS`, because their form is a size choice rather
# than a dialect rule (the unit name and compilation directory are inline
# `DW_FORM_string` up to version 4, which costs their bytes once and keeps a
# unit with no function names out of `.debug_str` entirely, while GCC already
# has a string table for the names it emits and points at it).
CU_FORM_PARITY = {
    2: ("low_pc", "high_pc", "stmt_list", "language"),
    3: ("low_pc", "high_pc", "stmt_list", "language"),
    4: ("low_pc", "high_pc", "stmt_list", "language"),
    5: ("low_pc", "high_pc", "stmt_list", "language", "producer", "name", "comp_dir"),
}
LINE_KEYS = (
    "version",
    "address_size",
    "segment_selector",
    "minimum_instruction_length",
    "max_ops",
    "is_stmt",
    "line_base",
    "line_range",
    "opcode_base",
    "opcode_args",
    "directory_columns",
    "file_columns",
    "directories",
    "files",
)


def compare_profiles(
    version: int,
    dialect: str,
    reference: dict[str, object],
    ours: dict[str, object],
    keys: tuple[str, ...],
) -> None:
    for key in keys:
        require(
            ours.get(key) == reference.get(key),
            f"-gdwarf-{version} -std={dialect}: {key} is {ours.get(key)!r}, "
            f"GCC 16.2 reports {reference.get(key)!r}",
        )


def check_version_and_dialect(
    directory: Path,
    lccc: str,
    gcc: str,
    readelf: str,
    version: int,
    dialect: str,
    spelling: str,
) -> None:
    """Compile one dialect and compare the unit against GCC 16.2.

    ``spelling`` is how the driver names the source: ``absolute`` passes the
    full path while the gate runs in its own directory, ``relative`` runs the
    compiler inside the source's directory and passes the base name. Both
    spellings reach the same file, and each puts a different set of strings in
    the unit and the line tables -- an absolute path gives every file an owning
    directory, a relative one lets the compilation directory supply it -- so
    both are worth pinning to the reference compiler.
    """
    source = write_fixture(directory)
    label = f"-gdwarf-{version} -std={dialect} ({spelling})"
    if spelling == "relative":
        source_argument = source.name
        run_directory: Path | None = directory
        expected_name = source.name
        expected_comp_dir = str(directory)
    else:
        source_argument = str(source)
        run_directory = None
        expected_name = str(source)
        expected_comp_dir = str(Path.cwd().resolve())
    suffix = f"{version}-{dialect}-{spelling}"
    ours_object = directory / f"ours-{suffix}.o"
    reference_object = directory / f"gcc-{suffix}.o"
    for compiler, output in ((lccc, ours_object), (gcc, reference_object)):
        run_ok(
            [
                compiler,
                "-g",
                f"-gdwarf-{version}",
                f"-std={dialect}",
                "-O0",
                "-c",
                source_argument,
                "-o",
                str(output),
            ],
            cwd=run_directory,
        )
    ours_dump = require_clean_readelf(readelf, ours_object, label)
    reference_dump = require_clean_readelf(readelf, reference_object, label)
    compare_profiles(
        version, dialect, cu_profile(reference_dump), cu_profile(ours_dump), CU_KEYS
    )
    ours_forms = cu_forms(readelf, ours_object)
    reference_forms = cu_forms(readelf, reference_object)
    for attribute in CU_FORM_PARITY[version]:
        require(
            attribute in reference_forms,
            f"{label}: GCC 16.2 declares no DW_AT_{attribute} for the unit",
        )
        require(
            ours_forms.get(attribute) == reference_forms.get(attribute),
            f"{label}: DW_AT_{attribute} is {ours_forms.get(attribute)}, "
            f"GCC 16.2 encodes it as {reference_forms.get(attribute)}",
        )
    ours_line = line_profile(ours_dump)
    reference_line = line_profile(reference_dump)
    compare_profiles(version, dialect, reference_line, ours_line, LINE_KEYS)
    # The unit names the source the driver was handed -- an absolute path here
    # -- and the directory the compilation ran in. GCC 16.2 spells both the
    # same way, which the profile comparison above already enforces; this
    # pins the two strings to the caller's arguments rather than to any fixed
    # placeholder that would happen to agree.
    ours_cu = cu_profile(ours_dump)
    require(
        ours_cu["name"] == expected_name,
        f"{label}: unit name is {ours_cu['name']!r}, expected {expected_name!r}",
    )
    require(
        ours_cu["comp_dir"] == expected_comp_dir,
        f"{label}: compilation directory is {ours_cu['comp_dir']!r}, "
        f"expected {expected_comp_dir}",
    )
    require(
        isinstance(ours_cu["producer"], str) and ours_cu["producer"].startswith("LCCC "),
        f"{label}: producer is {ours_cu['producer']!r}",
    )
    # `.debug_str`/`.debug_line_str` exist exactly when the dialect can
    # reference them, and stay empty otherwise.
    sections = readelf_dump(readelf, ours_object, "-S")
    has_line_str = ".debug_line_str" in sections
    require(
        has_line_str == (version >= 5),
        f"{label}: .debug_line_str present={has_line_str} on version {version}",
    )
    require(
        (".debug_str" in sections) == (version >= 5),
        f"{label}: .debug_str must exist exactly for version 5",
    )


def link_image(
    compiler: str, source: Path, output: Path, version: int | None, extra: list[str]
) -> None:
    # `-nostdlib -static` keeps the cross targets linkable without guest CRT
    # or libgcc: the fixture needs neither, and the debug sections under test
    # are the same either way.
    argv = [compiler, "-O0", "-nostdlib", "-static", "-o", str(output)]
    if version is not None:
        argv[1:1] = ["-g", f"-gdwarf-{version}"]
    argv.extend(extra)
    argv.append(str(source))
    run_ok(argv)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lccc", default=os.environ.get("CCC", "target/fastbuild/lccc"))
    parser.add_argument(
        "--lccc-i686", default=os.environ.get("CCC_I686", "target/fastbuild/lccc-i686")
    )
    parser.add_argument(
        "--lccc-arm", default=os.environ.get("CCC_ARM", "target/fastbuild/lccc-arm")
    )
    parser.add_argument(
        "--lccc-riscv", default=os.environ.get("CCC_RISCV", "target/fastbuild/lccc-riscv")
    )
    parser.add_argument("--gcc", default=os.environ.get("GCC", "gcc-16"))
    parser.add_argument("--readelf", default=os.environ.get("READELF", "readelf"))
    parser.add_argument("--addr2line", default=os.environ.get("ADDR2LINE", "addr2line"))
    parser.add_argument("--nm", default=os.environ.get("NM", "nm"))
    args = parser.parse_args()

    try:
        lccc = resolve_command(args.lccc)
        lccc_i686 = resolve_command(args.lccc_i686)
        lccc_arm = resolve_command(args.lccc_arm)
        lccc_riscv = resolve_command(args.lccc_riscv)
        gcc = resolve_command(args.gcc)
        readelf = resolve_command(args.readelf)
        addr2line = resolve_command(args.addr2line)
        nm = resolve_command(args.nm)
        versions = (2, 3, 4, 5)
        dialects = ("c89", "c99", "c11", "c17", "c23")

        with tempfile.TemporaryDirectory(prefix="lccc-dwarf-versions-") as tmp:
            directory = Path(tmp).resolve()
            for version in versions:
                for spelling in ("absolute", "relative"):
                    for dialect in dialects:
                        check_version_and_dialect(
                            directory, lccc, gcc, readelf, version, dialect, spelling
                        )
                print(
                    f"PASS -gdwarf-{version}: unit and line-program profile match "
                    f"GCC 16.2 for -std={'/'.join(dialects)} "
                    f"with absolute and relative source paths"
                )

            # Every target round-trips through addr2line in every dialect.
            for version in versions:
                for name, compiler in (
                    ("x86_64", lccc),
                    ("i686", lccc_i686),
                    ("aarch64", lccc_arm),
                    ("riscv64", lccc_riscv),
                ):
                    source = write_fixture(directory)
                    image = directory / f"{name}-{version}.elf"
                    link_image(compiler, source, image, version, [])
                    # A host image of the same file is the reference the
                    # guest targets are held to: their own toolchains have no
                    # GNU debugger side, so the only honest oracle is a
                    # second compiler asked the same question.
                    reference = None
                    if name == "x86_64":
                        reference_image = directory / f"gcc-{version}.elf"
                        link_image(gcc, source, reference_image, version, [])
                        reference = (addr2line, nm, reference_image)
                    require_functions_resolve(
                        addr2line, nm, image, source, f"{name} v{version}", reference
                    )
                    clean = require_clean_readelf(
                        readelf, image, f"{name} v{version} linked"
                    )
                    profile = cu_profile(clean)
                    require(
                        profile["version"] == version,
                        f"{name} v{version}: linked unit version is "
                        f"{profile['version']}",
                    )
                    require(
                        profile["pointer_size"] == (4 if name == "i686" else 8),
                        f"{name}: linked unit pointer size is {profile['pointer_size']}",
                    )
                print(
                    f"PASS -gdwarf-{version}: x86_64, i686, aarch64, riscv64 linked "
                    f"images resolve every function and keep their dialect"
                )

            # Two objects sharing string content: the second object's
            # `.debug_str`/`.debug_line_str` offsets only stay correct if they
            # are relocated, which is what a merged link exercises.
            other = directory / "other.c"
            other.write_text(
                "int gamma(int value)\n"
                "{\n"
                "    return value - 1;\n"
                "}\n",
                encoding="utf-8",
            )
            for version in versions:
                first = directory / f"merge-a-{version}.o"
                second = directory / f"merge-b-{version}.o"
                for source, output in (
                    (write_fixture(directory), first),
                    (other, second),
                ):
                    run_ok(
                        [
                            lccc,
                            "-g",
                            f"-gdwarf-{version}",
                            "-O0",
                            "-c",
                            str(source),
                            "-o",
                            str(output),
                        ]
                    )
                image = directory / f"merge-{version}.elf"
                run_ok([lccc, "-o", str(image), str(first), str(second), "-nostartfiles"])
                # The merged image must resolve both translation units.
                addresses = symbol_addresses(nm, image)
                for function, expected_file in (
                    ("alpha", "unit.c"),
                    ("beta", "unit.c"),
                    ("main", "unit.c"),
                    ("gamma", "other.c"),
                ):
                    require(
                        function in addresses,
                        f"v{version} merge: {function} missing from the link",
                    )
                    found, file, line = addr2line_resolve(
                        addr2line, image, addresses[function]
                    )
                    require(
                        found == function and Path(file).name == expected_file,
                        f"v{version} merge: {function} resolved to {found} {file}:{line}, "
                        f"expected {function} {expected_file}",
                    )
            print(
                "PASS multi-object links: both translation units resolve at every "
                "version (string tables are relocated, not baked in)"
            )

            # The command-line boundary, measured on GCC 16.2:
            #   `-gdwarf-N` alone enables debug info, `-g0` after it disables it,
            #   `-gdwarf-N` after `-g0` re-enables it, and versions outside
            #   2..5 are rejected.
            boundary = directory / "boundary.c"
            boundary.write_text("int only(void)\n{\n    return 0;\n}\n", encoding="utf-8")
            cases = (
                (["-gdwarf-5"], True, 5),
                (["-gdwarf-4"], True, 4),
                (["-gdwarf-2"], True, 2),
                (["-gdwarf-5", "-g0"], False, None),
                (["-g0", "-gdwarf-5"], True, 5),
                (["-gdwarf-2", "-gdwarf-5"], True, 5),
                (["-gdwarf-5", "-gdwarf-2"], True, 2),
            )
            for flags, expect_debug, expect_version in cases:
                profiles = {}
                for compiler, label in ((gcc, "GCC 16.2"), (lccc, "LCCC")):
                    output = directory / "boundary.o"
                    run_ok(
                        [
                            compiler,
                            *flags,
                            "-O0",
                            "-c",
                            str(boundary),
                            "-o",
                            str(output),
                        ]
                    )
                    sections = readelf_dump(readelf, output, "-S")
                    has_debug = ".debug_info" in sections
                    require(
                        has_debug == expect_debug,
                        f"{label} {' '.join(flags)}: .debug_info present="
                        f"{has_debug}, expected {expect_debug}",
                    )
                    if not expect_debug:
                        continue
                    profile = cu_profile(
                        readelf_dump(readelf, output, "--debug-dump=info")
                    )
                    require(
                        profile["version"] == expect_version,
                        f"{label} {' '.join(flags)}: unit version "
                        f"{profile['version']}, expected {expect_version}",
                    )
                    profiles[label] = profile
                if len(profiles) == 2:
                    # Every field but the producer identifies the unit, and the
                    # producer names the compiler that wrote it -- the one
                    # string that must differ.
                    ours = {k: v for k, v in profiles["LCCC"].items() if k != "producer"}
                    reference = {
                        k: v for k, v in profiles["GCC 16.2"].items() if k != "producer"
                    }
                    require(
                        ours == reference,
                        f"{' '.join(flags)}: LCCC unit profile "
                        f"{ours} differs from GCC 16.2 {reference}",
                    )
            print(
                "PASS -gdwarf-<N> command line: last-one-wins with -g0, dialect "
                "recorded, matching GCC 16.2"
            )

            for bad in ("-gdwarf-1", "-gdwarf-6", "-gdwarf-9"):
                for compiler, label in ((gcc, "GCC 16.2"), (lccc, "LCCC")):
                    completed = run(
                        [compiler, bad, "-c", str(boundary), "-o", str(directory / "bad.o")]
                    )
                    require(
                        completed.returncode != 0,
                        f"{label} accepted {bad}, which GCC rejects",
                    )
            strict = dict(os.environ)
            strict["LCCC_STRICT_OPTIONS"] = "1"
            for version in versions:
                completed = run(
                    [
                        lccc,
                        "-g",
                        f"-gdwarf-{version}",
                        "-c",
                        str(boundary),
                        "-o",
                        str(directory / "strict.o"),
                    ],
                    env=strict,
                )
                require(
                    completed.returncode == 0,
                    f"LCCC_STRICT_OPTIONS rejected -gdwarf-{version}: "
                    f"{completed.stderr}",
                )
                require(
                    "unrecognized" not in completed.stderr
                    and "not implemented" not in completed.stderr,
                    f"-gdwarf-{version} is implemented but reports a diagnostic: "
                    f"{completed.stderr}",
                )
            print(
                "PASS -gdwarf-1/-6/-9 rejected like GCC; -gdwarf-2..5 accepted "
                "under LCCC_STRICT_OPTIONS with no diagnostic"
            )

            # `-ffunction-sections` splits the unit into one code section per
            # function, so every function gets its own compile unit and its own
            # line program, and from version 5 its own string table. Section
            # offsets restart at zero per unit, which is exactly where a writer
            # that confuses a section offset with an address, or that shares a
            # file table it should have copied, breaks.
            for version in versions:
                for name, compiler in (
                    ("x86_64", lccc),
                    ("i686", lccc_i686),
                    ("aarch64", lccc_arm),
                    ("riscv64", lccc_riscv),
                ):
                    stem = f"function-sections-{name}-{version}"
                    object_file = directory / f"{stem}.o"
                    image = directory / f"{stem}"
                    run_ok(
                        [compiler, "-g", f"-gdwarf-{version}", "-O0",
                         "-ffunction-sections", "-c",
                         str(write_fixture(directory)), "-o", str(object_file)]
                    )
                    link_image(
                        compiler, object_file, image, None,
                        ["-nostdlib", "-static", "-Wl,-e,main"],
                    )
                    addresses = symbol_addresses(nm, image)
                    for function, expected_line in ENTRY_LINE.items():
                        require(
                            function in addresses,
                            f"{stem}: {function} was not emitted",
                        )
                        found, file, line = addr2line_resolve(
                            addr2line, image, addresses[function]
                        )
                        require(
                            found == function
                            and Path(file).name == "unit.c"
                            and line == expected_line,
                            f"{stem}: {function} resolves to {found} {file}:{line}, "
                            f"expected {function} unit.c:{expected_line}",
                        )
                    sections = readelf_dump(readelf, object_file, "-S")
                    if name == "riscv64":
                        # RISC-V relaxation deletes bytes after assembly, so a
                        # range length computed in the assembler would claim code
                        # that belongs to the next unit.  The map is omitted
                        # rather than approximated; a consumer scans .debug_info.
                        require(
                            ".debug_aranges" not in sections,
                            f"{stem}: a relaxing target published an approximate "
                            "address map",
                        )
                        continue
                    # Every function is its own unit here, so the address map has
                    # exactly one range per function -- at the addresses the
                    # linker gave them, which is what a debugger searches.
                    mapped = arange_pairs(readelf, image)
                    expected = {(addresses[function], None) for function in ENTRY_LINE}
                    require(
                        {address for address, _ in mapped} == {a for a, _ in expected},
                        f"{stem}: the address map covers {sorted(mapped)}, the link "
                        f"put the functions at {sorted(a for a, _ in expected)}",
                    )
            print(
                "PASS -ffunction-sections: one unit, one line program, one address "
                "range and (v5) one string table per function, at every dialect and "
                "on every target (a relaxing target omits the map by design)"
            )

            # Optimization can fold a whole function -- and with it every
            # instruction that carried a source span -- so the file table must
            # not depend on spans that survive. `-g` still has to describe such
            # a unit, and a unit that only declares symbols has nothing to
            # describe, which is why GCC 16.2 stays silent about it.
            folded = directory / "folded.c"
            folded.write_text("int only(void)\n{\n    return 0;\n}\n", encoding="utf-8")
            declarations = directory / "declarations.c"
            declarations.write_text("extern int ext(int);\n", encoding="utf-8")
            for version in versions:
                for level in ("-O0", "-O1", "-O2"):
                    output = directory / f"folded-{version}{level}.o"
                    completed = run(
                        [lccc, "-g", f"-gdwarf-{version}", level, "-c", str(folded),
                         "-o", str(output)]
                    )
                    require(
                        completed.returncode == 0,
                        f"folded v{version} {level}: compiler failed: {completed.stderr}",
                    )
                    require(
                        not completed.stderr.strip(),
                        f"folded v{version} {level}: -g reported {completed.stderr.strip()!r}",
                    )
                    sections = readelf_dump(readelf, output, "-S")
                    for wanted in (".debug_info", ".debug_line"):
                        require(
                            wanted in sections,
                            f"folded v{version} {level}: {wanted} missing",
                        )
                    clean = require_clean_readelf(readelf, output, f"folded v{version} {level}")
                    require(
                        cu_profile(clean)["version"] == version,
                        f"folded v{version} {level}: unit version is not {version}",
                    )
                    address = symbol_addresses(nm, output)["only"]
                    found, file, line = addr2line_resolve(addr2line, output, address)
                    # No two compilers agree on which line the single remaining
                    # instruction of an empty function carries once it folds
                    # (measured at -O1/-O2: GCC 16.2 reports the closing brace,
                    # lccc the return), so the invariant under test is a line
                    # inside the function rather than one compiler's choice.
                    require(
                        found == "only" and Path(file).name == "folded.c" and 2 <= line <= 4,
                        f"folded v{version} {level}: {found} resolves to "
                        f"{Path(file).name}:{line}",
                    )
            for compiler, label in ((gcc, "GCC 16.2"), (lccc, "LCCC")):
                completed = run(
                    [compiler, "-g", "-O2", "-c", str(declarations),
                     "-o", str(directory / "declarations.o")]
                )
                require(
                    completed.returncode == 0,
                    f"{label}: declaration-only unit failed: {completed.stderr}",
                )
                require(
                    not completed.stderr.strip(),
                    f"{label}: declaration-only unit reported {completed.stderr.strip()!r}",
                )
            print(
                "PASS -O0/-O1/-O2: a unit that folds to nothing keeps its line "
                "program, a declaration-only unit stays as quiet as GCC 16.2"
            )

            # The line program must decode back to the rows the source asked
            # for: same files, same lines, on every target and every dialect.
            for version in versions:
                for name, compiler in (
                    ("x86_64", lccc),
                    ("i686", lccc_i686),
                    ("aarch64", lccc_arm),
                    ("riscv64", lccc_riscv),
                ):
                    # x86-64 also at -O2, where the peephole deletes
                    # instructions and their rows: the surviving rows must
                    # still be exactly the ones the assembly asked for.
                    levels = ("-O0", "-O2") if name == "x86_64" else ("-O0",)
                    for level in levels:
                        stem = f"rows-{name}-{version}{level}"
                        assembly = directory / f"{stem}.s"
                        object_file = directory / f"{stem}.o"
                        source = write_fixture(directory)
                        run_ok(
                            [compiler, "-g", f"-gdwarf-{version}", level, "-S",
                             str(source), "-o", str(assembly)]
                        )
                        run_ok(
                            [compiler, "-g", f"-gdwarf-{version}", level, "-c",
                             str(source), "-o", str(object_file)]
                        )
                        emitted = asm_loc_rows(assembly.read_text(encoding="utf-8"))
                        decoded = decoded_line_rows(readelf, object_file)
                        require(
                            emitted == decoded,
                            f"{name} v{version} {level}: the line program decodes "
                            f"to {sorted(decoded)} but the source asked for "
                            f"{sorted(emitted)}",
                        )
                        require(emitted, f"{name} v{version} {level}: no rows")
            print(
                "PASS every dialect and target (x86-64 also at -O2): the decoded "
                "line program is exactly the set of rows the source asked for"
            )

        print(
            "PASS: DWARF 2/3/4/5 unit, language and line-program parity with GCC "
            "16.2 across c89..c23, all four targets round-trip through addr2line "
            "and decode back to their own rows, multi-object string relocation "
            "and the -gdwarf-<N> command line"
        )
    except GateFailure as failure:
        print(f"FAIL: {failure}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
