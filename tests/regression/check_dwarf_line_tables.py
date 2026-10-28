#!/usr/bin/env python3
"""Check LCCC DWARF line emission and linker preservation against GCC 16.2.

The test exercises the object writer's `.file`/`.loc` conversion, the compile
and subprogram units it synthesizes for those line programs (GAS parity: a
debugger reaches a line table only through a unit's `DW_AT_stmt_list`),
multi-object relocation of line-table and unit addresses, and preservation of
GCC's full DWARF
sections through both executable and shared-library links. It deliberately
checks usable line/address relationships rather than byte-identical debug
output: GCC and LCCC emit different but valid line programs.

The AArch64 and RISC-V legs hold LCCC's built-in assemblers to GNU as on the
same assembly: every decoded row must name the same instruction, in each
object and after a GNU ld link (RISC-V relaxation included).  Rows are
compared by instruction ordinal, so the check is independent of encoding
size.  A leg whose cross binutils are absent prints a SKIP line.
"""

from __future__ import annotations

import argparse
import os
import re
import shutil
import struct
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
        if Path(argv[0]).name.startswith("readelf") and any(
            marker in completed.stderr.lower() for marker in ("warning:", "error:")
        ):
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


def require_debug_not_loaded(readelf: str, path: Path) -> None:
    program_headers = run([readelf, "-lW", str(path)])
    require("LOAD" in program_headers, f"{path} has no PT_LOAD segments")
    _, separator, mapping = program_headers.partition("Section to Segment mapping:")
    require(separator, f"readelf omitted the section-to-segment mapping for {path}")
    require(
        ".debug" not in mapping and ".zdebug" not in mapping,
        f"non-allocated debug sections were mapped into PT_LOAD in {path}",
    )


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
    names = (".debug_info", ".debug_abbrev", ".debug_str") if full_info else (
        ".debug_info", ".rela.debug_info", ".debug_abbrev")
    for name in names:
        require(name in sections, f"{path} has no {name} section")


def require_unit_covers(readelf: str, path: Path, function: str) -> None:
    """The function's DW_TAG_subprogram starts at its symbol, and every
    compile unit's DW_AT_stmt_list names a line program of the image."""
    info = run([readelf, "--debug-dump=info", str(path)])
    start, _ = symbol_extent(readelf, path, function)
    match = re.search(
        r"DW_TAG_subprogram\)\n(?:\s+<[0-9a-f]+>\s+DW_AT_\w+\s*:.*\n)*?"
        rf"\s+<[0-9a-f]+>\s+DW_AT_name\s*:\s*(?:\(.*\):\s*)?{re.escape(function)}\n"
        r"(?:\s+<[0-9a-f]+>\s+DW_AT_\w+\s*:.*\n)*?"
        r"\s+<[0-9a-f]+>\s+DW_AT_low_pc\s*:\s*(0x[0-9a-f]+|\d+)\b",
        info,
    )
    require(match is not None, f"{path} has no subprogram DIE for {function}")
    require(
        int(match.group(1), 0) == start,
        f"{function} DIE low_pc {match.group(1)} != symbol {start:#x} in {path}",
    )
    lines = run([readelf, "--debug-dump=rawline", str(path)])
    programs = {int(m, 0) for m in re.findall(r"^\s*Offset:\s+(0x[0-9a-f]+|\d+)\b", lines, re.M)}
    lists = [int(m, 0) for m in re.findall(r"DW_AT_stmt_list\s*:\s*(0x[0-9a-f]+|\d+)\b", info)]
    require(lists, f"{path} has no DW_AT_stmt_list")
    require(all(o in programs for o in lists), f"{path}: stmt_list {lists} vs programs {programs}")


def elf64_sections(path: Path) -> dict[str, tuple[int, bytes]]:
    """Map section name -> (sh_flags, file bytes) for a little-endian ELF64."""
    data = path.read_bytes()
    shoff = int.from_bytes(data[40:48], "little")
    count = int.from_bytes(data[60:62], "little")
    names_index = int.from_bytes(data[62:64], "little")
    headers = [struct.unpack_from("<IIQQQQIIQQ", data, shoff + i * 64) for i in range(count)]
    names = headers[names_index]
    strtab = data[names[4] : names[4] + names[5]]
    sections = {}
    for name_off, sh_type, flags, _addr, offset, size, *_ in headers:
        name = strtab[name_off : strtab.index(b"\0", name_off)].decode()
        sections[name] = (flags, b"" if sh_type == 8 else data[offset : offset + size])
    return sections


def objcopy_compression_forms(objcopy: str) -> list[str]:
    """The --compress-debug-sections forms this objcopy can write."""
    usage = run([objcopy, "--help"])
    forms = ["zlib-gabi", "zlib-gnu"]
    if "zstd" in usage:
        forms.append("zstd")
    return forms


def require_compressed_inputs_link_identically(
    lccc: str, objcopy: str, readelf: str, work: Path, objects: list[Path], tag: str
) -> list[str]:
    """Link `objects` uncompressed and with every compressed-debug form.

    GAS compresses DWARF by default when binutils is configured with
    --enable-compressed-debug-sections (the GAS 2.47 oracle is), and
    binutils/clang built with libzstd write ELFCOMPRESS_ZSTD. Relocations
    address the uncompressed bytes, so the linker must decompress; the
    output's debug sections must equal those of the uncompressed link.
    """
    def link(inputs: list[Path], name: str) -> Path:
        out = work / name
        run([lccc, "-nostdlib", "-no-pie", "-Wl,-e,dwarf_entry", *map(str, inputs), "-o", str(out)], cwd=work)
        return out

    reference = elf64_sections(link(objects, f"{tag}-plain"))
    debug_names = sorted(name for name in reference if name.startswith(".debug_"))
    require(".debug_info" in debug_names, f"{tag}: reference link has no .debug_info")
    forms = objcopy_compression_forms(objcopy)
    for form in forms:
        compressed = []
        for obj in objects:
            out = work / f"{obj.stem}-{form}.o"
            run([objcopy, f"--compress-debug-sections={form}", str(obj), str(out)])
            compressed.append(out)
        listing = section_listing(readelf, compressed[0])
        if form == "zlib-gnu":
            require(".zdebug_info" in listing, f"{tag}: objcopy did not write legacy .zdebug_info")
        sections = elf64_sections(link(compressed, f"{tag}-{form}"))
        for name in debug_names:
            require(name in sections, f"{tag} {form}: link dropped {name}")
            require(sections[name][0] & 0x800 == 0, f"{tag} {form}: {name} kept SHF_COMPRESSED")
            require(
                sections[name][1] == reference[name][1],
                f"{tag} {form}: {name} differs from the uncompressed link",
            )
        require(
            not any(name.startswith(".zdebug") for name in sections),
            f"{tag} {form}: a legacy .zdebug name reached the output",
        )
    return forms


def require_entry_and_statement_rows(readelf: str, path: Path, filename: str, function: str) -> None:
    """An object's own rows: the opening-brace row at the function's first
    byte (ahead of the prologue, as GCC emits it) and the body's statement
    row strictly inside the function. On i386 the statement row's address
    is a REL addend in the relocated field; dropping it put every row at
    the section start."""
    rows = line_rows(decoded_lines(readelf, path), filename)
    start, size = symbol_extent(readelf, path, function)
    require((1, start) in rows or (2, start) in rows, f"{path}: no entry row at {function} ({rows})")
    statements = [address for line, address in rows if line == (2 if function == "dwarf_helper" else 3)]
    require(
        statements and all(start < address < start + size for address in statements),
        f"{path}: statement row of {function} not inside {start:#x}+{size:#x}: {rows}",
    )


def require_no_debug_keeps_symtab(readelf: str, path: Path) -> None:
    sections = section_listing(readelf, path)
    require(".debug" not in sections, f"-S left debug sections in {path}")
    require(".symtab" in sections, f"-S dropped .symtab from {path}")


def require_debugger_breaks_function(path: Path, function: str, filename: str, line: int) -> bool:
    """`break FUNCTION` skips the prologue to the body's first statement:
    gdb needs the entry row to skip *from*."""
    gdb = shutil.which("gdb")
    if gdb is None:
        return False
    out = run([gdb, "-q", "-batch", "-nx", "-ex", f"break {function}", str(path)])
    require(
        re.search(rf"Breakpoint 1 at 0x[0-9a-f]+: file .*{re.escape(filename)}, line {line}\.", out)
        is not None,
        f"gdb `break {function}` did not land on {filename}:{line} in {path}:\n{out}",
    )
    return True


def require_debugger_resolves(path: Path, filename: str, line: int) -> bool:
    """gdb resolves `file:line` through the line table; False when absent."""
    gdb = shutil.which("gdb")
    if gdb is None:
        return False
    out = run([gdb, "-q", "-batch", "-nx", "-ex", f"break {filename}:{line}", str(path)])
    require(
        re.search(rf"Breakpoint 1 at 0x[0-9a-f]+: file .*{re.escape(filename)}, line {line}\.", out)
        is not None,
        f"gdb could not resolve {filename}:{line} in {path}:\n{out}",
    )
    return True


CROSS_MAIN = """\
extern int cross_far(int, int);
static int cross_helper(int x) {
  int y = x * 3;
  if (y > 10) {
    y -= 7;
  }
  return y;
}
int cross_entry(void) {
  int s = 0;
  for (int i = 0; i < 5; i++)
    s += cross_helper(i);
  return cross_far(s, 2);
}
"""
CROSS_FAR = "int cross_far(int a, int b) {\n  return a - b * 9 > 0 ? 0 : 1;\n}\n"
INSN_LINE = re.compile(r"^\s*([0-9a-f]+):\t")


def ordinal_rows(readelf: str, objdump: str, path: Path) -> list[tuple[str, str, int]]:
    """Decoded line rows as (file, line, instruction ordinal); -1 = sequence end."""
    starts: dict[int, int] = {}
    for raw in run([objdump, "-d", "--no-show-raw-insn", str(path)]).splitlines():
        match = INSN_LINE.match(raw)
        if match:
            starts.setdefault(int(match.group(1), 16), len(starts))
    rows: list[tuple[str, str, int]] = []
    for raw in run([readelf, "-W", "--debug-dump=decodedline", str(path)]).splitlines():
        fields = raw.split()
        if len(fields) < 3 or not fields[0].endswith(".c"):
            continue
        if not (fields[1].isdigit() or fields[1] == "-"):
            continue
        try:
            address = int(fields[2], 16)
        except ValueError:
            continue
        ordinal = starts.get(address)
        if ordinal is None:
            require(fields[1] == "-", f"{path}: row `{raw.strip()}` is not at an instruction")
            ordinal = -1
        rows.append((Path(fields[0]).name, fields[1], ordinal))
    require(rows, f"{path} has no line rows")
    return rows


def cross_leg(name: str, triple: str, lccc: str, work: Path) -> str:
    tools = {tool: shutil.which(f"{triple}-{tool}") for tool in ("gcc", "ld", "readelf", "objdump")}
    missing = [f"{triple}-{tool}" for tool, path in tools.items() if path is None]
    if missing:
        print(f"SKIP: {name} line tables vs GNU as ({', '.join(missing)} not installed)")
        return f"{name} skipped"
    gnu_cc, gnu_ld, readelf, objdump = (str(tools[t]) for t in ("gcc", "ld", "readelf", "objdump"))
    rows = 0
    for opt in ("-O0", "-O2"):
        objects: dict[str, list[Path]] = {"lccc": [], "gas": []}
        for stem, text in (("cross_main", CROSS_MAIN), ("cross_far", CROSS_FAR)):
            source = work / f"{name}-{stem}.c"
            source.write_text(text, encoding="utf-8")
            asm = work / f"{name}{opt}-{stem}.s"
            run([lccc, opt, "-g", "-S", str(source), "-o", str(asm)], cwd=work)
            ours, theirs = (work / f"{name}{opt}-{stem}.{who}.o" for who in ("lccc", "gas"))
            run([lccc, "-c", str(asm), "-o", str(ours)], cwd=work)
            run([gnu_cc, "-c", str(asm), "-o", str(theirs)], cwd=work)
            require(
                "LCCC" in run([readelf, "--debug-dump=info", str(ours)]),
                f"{ours}: the unit's producer is not LCCC's built-in assembler",
            )
            got, want = ordinal_rows(readelf, objdump, ours), ordinal_rows(readelf, objdump, theirs)
            require(got == want, f"{name} {opt} {stem}.o rows differ from GNU as:\n{got}\n{want}")
            rows += len(got)
            objects["lccc"].append(ours)
            objects["gas"].append(theirs)
        linked = {}
        for who, inputs in objects.items():
            exe = work / f"{name}{opt}.{who}.exe"
            run([gnu_ld, "-e", "cross_entry", *map(str, inputs), "-o", str(exe)], cwd=work)
            linked[who] = ordinal_rows(readelf, objdump, exe)
        require(
            linked["lccc"] == linked["gas"],
            f"{name} {opt} linked rows differ from GNU as:\n{linked['lccc']}\n{linked['gas']}",
        )
        rows += len(linked["lccc"])
    return f"{name} ({rows} rows = GNU as)"


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
    parser.add_argument("--objcopy", default=os.environ.get("OBJCOPY", "objcopy"))
    args = parser.parse_args()

    try:
        lccc = resolve_command(args.lccc)
        lccc_i686 = resolve_command(args.lccc_i686)
        lccc_arm = resolve_command(args.lccc_arm)
        lccc_riscv = resolve_command(args.lccc_riscv)
        gcc = resolve_command(args.gcc)
        readelf = resolve_command(args.readelf)
        objcopy = resolve_command(args.objcopy)
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
                    if compiler == lccc:
                        function = "dwarf_helper" if source == source_a else "dwarf_entry"
                        require_unit_covers(readelf, obj, function)
                        require_entry_and_statement_rows(readelf, obj, source.name, function)
                    rows = line_rows(decoded_lines(readelf, obj), source.name)
                    require(rows, f"{compiler} object lacks line rows for {source.name}")
                    require(
                        any(line in (2, 3) for line, _ in rows),
                        f"{compiler} object lacks a statement line for {source.name}: {rows}",
                    )

            # The i686 assembler shares the line-table writer but uses REL
            # relocations and four-byte code addresses.
            i686_objects = []
            for source in (source_a, source_b):
                obj = work / f"lccc-i686-{source.stem}.o"
                run([lccc_i686, "-std=c23", "-O0", "-g", "-c", str(source), "-o", str(obj)], cwd=work)
                function = "dwarf_helper" if source == source_a else "dwarf_entry"
                require_entry_and_statement_rows(readelf, obj, source.name, function)
                i686_objects.append(obj)
            i686_object = i686_objects[0]
            require(
                "ELF32" in run([readelf, "-h", str(i686_object)]),
                "i686 assembler did not produce an ELF32 object",
            )
            i686_sections = section_listing(readelf, i686_object)
            require(".debug_line" in i686_sections, "i686 object has no .debug_line section")
            require(".rel.debug_line" in i686_sections, "i686 object has no REL debug-line relocation")
            require(".rel.debug_info" in i686_sections, "i686 object has no REL debug-info relocation")
            require_unit_covers(readelf, i686_object, "dwarf_helper")
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
            require_debug_not_loaded(readelf, lccc_exec)
            require_line_address(readelf, lccc_exec, "unit_a.c", "dwarf_helper")
            require_line_address(readelf, lccc_exec, "unit_b.c", "dwarf_entry")
            require_unit_covers(readelf, lccc_exec, "dwarf_helper")
            require_unit_covers(readelf, lccc_exec, "dwarf_entry")
            debugger = require_debugger_resolves(lccc_exec, "unit_a.c", 2)
            require_debugger_breaks_function(lccc_exec, "dwarf_helper", "unit_a.c", 2)

            # i686 images: LCCC's own objects, GCC 16.2's (zlib-compressed by
            # default) objects, and a GCC -fPIC shared object -- the i386
            # writers emit a full section-header table with the debug
            # sections and statically resolved debug relocations.
            gcc32 = []
            for source in (source_a, source_b):
                obj = work / f"gcc32-{source.stem}.o"
                run([gcc, "-m32", "-std=c23", "-O0", "-g", "-c", str(source), "-o", str(obj)], cwd=work)
                gcc32.append(obj)
                pic = work / f"gcc32-pic-{source.stem}.o"
                run([gcc, "-m32", "-fPIC", "-std=c23", "-O0", "-g", "-c", str(source), "-o", str(pic)], cwd=work)
                gcc32.append(pic)
            i686_images = []
            for name, inputs, mode in (
                ("lccc32-exec", i686_objects, ["-no-pie", "-Wl,-e,dwarf_entry"]),
                ("gcc32-exec", gcc32[0::2], ["-no-pie", "-Wl,-e,dwarf_entry"]),
                ("libgcc32-dwarf.so", gcc32[1::2], ["-shared"]),
            ):
                image = work / name
                run([lccc_i686, "-nostdlib", *mode, *map(str, inputs), "-o", str(image)], cwd=work)
                sections = section_listing(readelf, image)
                for section in (".debug_info", ".debug_abbrev", ".debug_line", ".symtab"):
                    require(section in sections, f"i686 {name} lost {section}")
                require_debug_not_loaded(readelf, image)
                require_line_address(readelf, image, "unit_a.c", "dwarf_helper")
                require_line_address(readelf, image, "unit_b.c", "dwarf_entry")
                require_unit_covers(readelf, image, "dwarf_helper")
                require_unit_covers(readelf, image, "dwarf_entry")
                i686_images.append(image)
            require_debugger_resolves(i686_images[0], "unit_a.c", 2)
            require_debugger_resolves(i686_images[1], "unit_b.c", 3)
            require_debugger_breaks_function(i686_images[0], "dwarf_helper", "unit_a.c", 2)

            # -S/--strip-debug drops the debug sections but keeps .symtab.
            for driver, inputs in ((lccc, lccc_objects), (lccc_i686, i686_objects)):
                image = work / f"{Path(driver).name}-strip-debug"
                run(
                    [driver, "-nostdlib", "-no-pie", "-Wl,-S", "-Wl,-e,dwarf_entry", *map(str, inputs),
                     "-o", str(image)],
                    cwd=work,
                )
                require_no_debug_keeps_symtab(readelf, image)

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
            require_debug_not_loaded(readelf, stripped_exec)

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
            require_debug_not_loaded(readelf, gcc_exec)
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
            require_debug_not_loaded(readelf, gcc_shared)
            require_line_address(readelf, gcc_shared, "unit_a.c", "dwarf_helper")
            require_line_address(readelf, gcc_shared, "unit_b.c", "dwarf_entry")

            # Compressed debug input, in every form, from both producers.
            gcc_plain = []
            for source in (source_a, source_b):
                obj = work / f"gcc-plain-{source.stem}.o"
                run([gcc, "-std=c23", "-O0", "-g", "-gz=none", "-c", str(source), "-o", str(obj)], cwd=work)
                require(
                    "COMPRESSED" not in run([readelf, "-tW", str(obj)]),
                    "gcc -gz=none produced compressed debug sections",
                )
                gcc_plain.append(obj)
            forms = require_compressed_inputs_link_identically(lccc, objcopy, readelf, work, gcc_plain, "gcc")
            require_compressed_inputs_link_identically(lccc, objcopy, readelf, work, lccc_objects, "lccc")
            cross = [
                cross_leg("aarch64", "aarch64-linux-gnu", lccc_arm, work),
                cross_leg("riscv64", "riscv64-linux-gnu", lccc_riscv, work),
            ]

        print(
            "PASS: x86-64/i686 LCCC line tables (entry and statement rows) and compile units, "
            "multi-object relocation, GCC DWARF preservation (x86-64 and -m32), strip-all, "
            "strip-debug, executable and shared links (x86-64 and i686), "
            f"compressed debug input ({', '.join(forms)}); "
            f"built-in assemblers: {', '.join(cross)}"
            + ("; gdb resolves file:line" if debugger else "; gdb not installed, debugger check skipped")
        )
        return 0
    except (GateFailure, OSError, subprocess.SubprocessError) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
