#!/usr/bin/env python3
"""Check LCCC DWARF line emission and linker preservation against GCC 16.2.

The test exercises the object writer's `.file`/`.loc` conversion, multi-object
relocation of line-table addresses, and preservation of GCC's full DWARF
sections through both executable and shared-library links. It deliberately
checks usable line/address relationships rather than byte-identical debug
output: GCC and LCCC emit different but valid line programs.
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


def run(argv: list[str], *, cwd: Path | None = None) -> str:
    completed = subprocess.run(
        argv,
        cwd=cwd,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if completed.returncode:
        raise GateFailure(
            f"command failed ({completed.returncode}): {' '.join(argv)}\n"
            f"stdout:\n{completed.stdout}\nstderr:\n{completed.stderr}"
        )
    if completed.stderr:
        # Compiler warnings are not fatal here; readelf warnings are, because
        # malformed offsets can otherwise be hidden by successful exit status.
        if Path(argv[0]).name.startswith("readelf") and "warning:" in completed.stderr.lower():
            raise GateFailure(
                f"readelf reported malformed ELF/DWARF for {' '.join(argv)}:\n"
                f"{completed.stderr}"
            )
    return completed.stdout


def require(condition: bool, message: str) -> None:
    if not condition:
        raise GateFailure(message)


def section_listing(readelf: str, path: Path) -> str:
    return run([readelf, "-SW", str(path)])


def decoded_lines(readelf: str, path: Path) -> str:
    return run([readelf, "--debug-dump=decodedline", str(path)])


def line_rows(output: str, filename: str) -> list[tuple[int, int]]:
    rows: list[tuple[int, int]] = []
    for raw in output.splitlines():
        fields = raw.split()
        if len(fields) < 3 or not fields[0].endswith(filename):
            continue
        if not fields[1].isdigit() or not (
            fields[2].startswith("0x") or fields[2].isdigit()
        ):
            continue
        try:
            rows.append((int(fields[1]), int(fields[2], 16)))
        except ValueError:
            continue
    return rows


def symbol_extent(readelf: str, path: Path, symbol_name: str) -> tuple[int, int]:
    symbols = run([readelf, "-sW", str(path)])
    pattern = re.compile(
        r"^\s*\d+:\s+([0-9a-fA-F]+)\s+(\d+)\s+\S+\s+\S+\s+\S+\s+\S+\s+(.+?)\s*$"
    )
    for raw in symbols.splitlines():
        match = pattern.match(raw)
        if match and match.group(3) == symbol_name:
            return int(match.group(1), 16), int(match.group(2))
    raise GateFailure(f"symbol {symbol_name!r} is missing from {path}")


def require_line_address(
    readelf: str, path: Path, filename: str, function: str
) -> None:
    rows = line_rows(decoded_lines(readelf, path), filename)
    require(rows, f"no decoded line rows for {filename} in {path}")
    start, size = symbol_extent(readelf, path, function)
    end = start + size
    require(
        any(start <= address <= end for _, address in rows),
        f"{filename} line addresses do not land in {function} ({start:#x}..{end:#x})",
    )


def require_object_dwarf(readelf: str, path: Path, *, full_info: bool) -> None:
    sections = section_listing(readelf, path)
    require(".debug_line" in sections, f"{path} has no .debug_line section")
    require(".rela.debug_line" in sections, f"{path} has no .rela.debug_line relocation section")
    if full_info:
        for name in (".debug_info", ".debug_abbrev", ".debug_str"):
            require(name in sections, f"{path} has no {name} section")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lccc", default=os.environ.get("CCC", "target/fastbuild/lccc"))
    parser.add_argument(
        "--lccc-i686", default=os.environ.get("CCC_I686", "target/fastbuild/lccc-i686")
    )
    parser.add_argument("--gcc", default=os.environ.get("GCC", "gcc-16"))
    parser.add_argument("--readelf", default=os.environ.get("READELF", "readelf"))
    args = parser.parse_args()

    try:
        lccc = resolve_command(args.lccc)
        lccc_i686 = resolve_command(args.lccc_i686)
        gcc = resolve_command(args.gcc)
        readelf = resolve_command(args.readelf)
        with tempfile.TemporaryDirectory(prefix="lccc-dwarf-") as tmp:
            work = Path(tmp)
            source_a = work / "unit_a.c"
            source_b = work / "unit_b.c"
            source_a.write_text(
                "int dwarf_helper(int x) {\n"
                "  return x + 3;\n"
                "}\n",
                encoding="utf-8",
            )
            source_b.write_text(
                "extern int dwarf_helper(int);\n"
                "int dwarf_entry(void) {\n"
                "  return dwarf_helper(4);\n"
                "}\n",
                encoding="utf-8",
            )

            lccc_objects: list[Path] = []
            gcc_objects: list[Path] = []
            for compiler, outputs in ((lccc, lccc_objects), (gcc, gcc_objects)):
                for source in (source_a, source_b):
                    obj = work / f"{Path(compiler).name}-{source.stem}.o"
                    run(
                        [
                            compiler,
                            "-std=c23",
                            "-O0",
                            "-g",
                            "-c",
                            str(source),
                            "-o",
                            str(obj),
                        ],
                        cwd=work,
                    )
                    outputs.append(obj)
                    require_object_dwarf(readelf, obj, full_info=(compiler == gcc))
                    rows = line_rows(decoded_lines(readelf, obj), source.name)
                    require(rows, f"{compiler} object lacks line rows for {source.name}")
                    require(
                        any(line in (2, 3) for line, _ in rows),
                        f"{compiler} object lacks a statement line for {source.name}: {rows}",
                    )

            # The i686 assembler shares the line-table writer but uses REL
            # relocations and four-byte code addresses.
            i686_object = work / "lccc-i686-unit-a.o"
            run(
                [lccc_i686, "-g", "-c", str(source_a), "-o", str(i686_object)],
                cwd=work,
            )
            require(
                "ELF32" in run([readelf, "-h", str(i686_object)]),
                "i686 assembler did not produce an ELF32 object",
            )
            i686_sections = section_listing(readelf, i686_object)
            require(".debug_line" in i686_sections, "i686 object has no .debug_line section")
            require(".rel.debug_line" in i686_sections, "i686 object has no REL debug-line relocation")
            i686_rows = line_rows(decoded_lines(readelf, i686_object), source_a.name)
            require(i686_rows and any(line == 2 for line, _ in i686_rows), "i686 line rows are missing")

            # LCCC's own line programs must survive a multi-input executable
            # link, with the two code-section contributions relocated apart.
            lccc_exec = work / "lccc-dwarf-exec"
            run(
                [
                    lccc,
                    "-nostdlib",
                    "-no-pie",
                    "-Wl,-e,dwarf_entry",
                    str(lccc_objects[0]),
                    str(lccc_objects[1]),
                    "-o",
                    str(lccc_exec),
                ],
                cwd=work,
            )
            require(".debug_line" in section_listing(readelf, lccc_exec), "linked LCCC output lost .debug_line")
            require_line_address(readelf, lccc_exec, "unit_a.c", "dwarf_helper")
            require_line_address(readelf, lccc_exec, "unit_b.c", "dwarf_entry")

            stripped_exec = work / "lccc-dwarf-stripped"
            run(
                [
                    lccc,
                    "-nostdlib",
                    "-no-pie",
                    "-Wl,--strip-all",
                    "-Wl,-e,dwarf_entry",
                    str(lccc_objects[0]),
                    str(lccc_objects[1]),
                    "-o",
                    str(stripped_exec),
                ],
                cwd=work,
            )
            require(
                ".debug_line" not in section_listing(readelf, stripped_exec),
                "--strip-all retained the LCCC .debug_line section",
            )

            # GCC 16.2 emits full DWARF 5. LCCC's linker must retain and fix
            # up its cross-section references, not merely copy the line table.
            gcc_exec = work / "gcc-dwarf-exec"
            run(
                [
                    lccc,
                    "-nostdlib",
                    "-no-pie",
                    "-Wl,-e,dwarf_entry",
                    str(gcc_objects[0]),
                    str(gcc_objects[1]),
                    "-o",
                    str(gcc_exec),
                ],
                cwd=work,
            )
            sections = section_listing(readelf, gcc_exec)
            for name in (".debug_info", ".debug_abbrev", ".debug_line", ".debug_str"):
                require(name in sections, f"LCCC executable link dropped GCC's {name}")
            info = run([readelf, "--debug-dump=info", str(gcc_exec)])
            require("dwarf_helper" in info and "dwarf_entry" in info, "GCC DWARF DIEs were not preserved")
            require_line_address(readelf, gcc_exec, "unit_a.c", "dwarf_helper")
            require_line_address(readelf, gcc_exec, "unit_b.c", "dwarf_entry")

            gcc_shared = work / "libgcc-dwarf.so"
            run(
                [
                    lccc,
                    "-shared",
                    "-nostdlib",
                    str(gcc_objects[0]),
                    str(gcc_objects[1]),
                    "-o",
                    str(gcc_shared),
                ],
                cwd=work,
            )
            require(".debug_info" in section_listing(readelf, gcc_shared), "shared link dropped GCC .debug_info")
            require_line_address(readelf, gcc_shared, "unit_a.c", "dwarf_helper")
            require_line_address(readelf, gcc_shared, "unit_b.c", "dwarf_entry")

        print(
            "PASS: x86-64/i686 LCCC line tables, multi-object relocation, GCC DWARF preservation, "
            "strip-all, executable and shared links"
        )
        return 0
    except (GateFailure, OSError, subprocess.SubprocessError) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
