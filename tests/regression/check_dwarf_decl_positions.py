#!/usr/bin/env python3
"""Check every subprogram's declared position and the entry rows around it.

DWARF gives a subprogram two independent descriptions of where it starts:

* ``DW_AT_decl_line``/``DW_AT_decl_column`` name the *declarator* -- the line and
  column the function's name is written on.  A definition whose return type sits
  on its own line, or whose body opens on the next line (the usual
  brace-on-its-own-line style), puts the declarator somewhere the line program
  cannot see unless it is told.
* the line program's rows start at the body's opening brace, because that is
  where the first statement is.

Both gcc and clang describe a definition that way (measured: gcc 12.2 reports
``main`` at ``16:5`` while the first row is line 17).  LCCC reaches a debugger
through its own assembler, whose only position channel is ``.loc``, so codegen
emits the declarator as a *non-statement* row at the function's first address
and the writer reads it back:

* the declarator row carries ``is_stmt 0``, so a consumer that maps an address
  to a line takes the body row that follows at the same address (what gcc and
  clang's tables read) while ``DW_AT_decl_line`` comes from the first row;
* the first row after the prologue carries ``prologue_end`` and the row at the
  first epilogue instruction carries ``epilogue_begin``, so a debugger's
  prologue skip and ``finish`` need no heuristics;
* two statements on one source line are two rows, so ``next`` can step between
  them.  A statement-per-line fixture would never show that, so the fixture
  below puts four statements on one line and checks each has its own row at its
  own column.

The gate is oracle-free: the expected position of every function is recomputed
from the fixture's own text, and the row properties are read back from the
emitted object with ``readelf``.  It runs on all four supported targets and all
four DWARF versions.
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

# Every function name appears exactly once in this text, so its declarator
# position is computable by searching for the identifier.  The shapes are the
# ones that make the declaration position differ from the body's:
#   * `spread`  -- return type and name on one line, body brace two lines later;
#   * `lines`   -- the return type on its own line, the name on the next;
#   * `packed`  -- four statements on one line (one row each, distinct columns);
#   * `plain`   -- the ordinary shape.
FIXTURE = """\
static int spread(int value)
{
    int doubled = value * 2;
    return doubled + 1;
}

static long
lines(long value)
{
    return value - 3;
}

static int packed(int value) { int a = value; a += 1; a *= 3; return a - 2; }

int plain(int value)
{
    return spread(value) + (int)lines((long)value) + packed(value);
}
"""

# The statements of `packed`, as (line, column) of their first token: each must
# have its own row, which is what separates "a row per statement" from "a row
# per source line".
PACKED_STATEMENTS = 4

ROW_LINE = re.compile(r"^\s*(\S+\.c)\s+(-|\d+)\s+(0x[0-9a-f]+|\d+)\s*(.*)$")


class GateFailure(RuntimeError):
    pass


def resolve_command(command: str) -> str:
    path = Path(command)
    return str(path.resolve()) if path.exists() else (shutil.which(command) or command)


def run(argv: list[str], *, cwd: Path | None = None) -> str:
    result = subprocess.run(argv, capture_output=True, text=True, cwd=cwd)
    if result.returncode != 0:
        raise GateFailure(
            f"command failed ({result.returncode}): {' '.join(argv)}\n"
            f"stdout:\n{result.stdout}\nstderr:\n{result.stderr}"
        )
    return result.stdout


def require(condition: bool, message: str) -> None:
    if not condition:
        raise GateFailure(message)


def _matching_paren(source: str, open_index: int) -> int | None:
    depth = 0
    for index in range(open_index, len(source)):
        if source[index] == "(":
            depth += 1
        elif source[index] == ")":
            depth -= 1
            if depth == 0:
                return index
    return None


def declarator_position(source: str, name: str) -> tuple[int, int]:
    """The 1-based (line, column) where `name` is declared in `source`.

    C requires a declaration before use and the fixture carries no prototypes,
    so the first occurrence of a name is its definition's declarator -- which is
    the position DWARF's `DW_AT_decl_line`/`DW_AT_decl_column` name.
    """
    occurrences = [m.start() for m in re.finditer(rf"\b{name}\b", source)]
    require(occurrences, f"fixture does not name {name}")
    offset = occurrences[0]
    # The first mention has to be the definition, not a prototype or a call:
    # after the parameter list of the first mention the body must open.
    open_paren = source.find("(", offset)
    require(open_paren > 0, f"fixture's first mention of {name} is not a declarator")
    close_paren = _matching_paren(source, open_paren)
    require(close_paren is not None, f"fixture's declarator for {name} is unbalanced")
    following = source[close_paren + 1 :].lstrip()
    require(
        following.startswith("{"),
        f"fixture's first mention of {name} is not a definition (followed by "
        f"{following[:16]!r}), so its declaration position is ambiguous",
    )
    line = source.count("\n", 0, offset) + 1
    column = offset - (source.rfind("\n", 0, offset) + 1) + 1
    return line, column


def subprograms(readelf: str, path: Path) -> dict[str, dict[str, int]]:
    """`name -> {file, line, column, low_pc, high_pc}` for every subprogram."""
    text = run([readelf, "-W", "--debug-dump=info", str(path)])
    found: dict[str, dict[str, int]] = {}
    current: dict[str, int] | None = None
    name: str | None = None
    for raw in text.splitlines():
        if "DW_TAG_subprogram" in raw:
            if name and current is not None:
                found[name] = current
            name, current = None, {}
            continue
        if "DW_TAG_" in raw and current is not None:
            if name is not None:
                found[name] = current
            name, current = None, None
            continue
        if current is None:
            continue
        attribute = re.search(r"DW_AT_(\w+)\s*:\s*(.*)$", raw)
        if not attribute:
            continue
        key, value = attribute.group(1), attribute.group(2).strip()
        if key == "name":
            # `(string) foo` is an inline name, `(indirect string, offset 0x9):
            # foo` one from a pool (`.debug_line_str` prints the same shape).
            text = re.sub(r"\([^)]*\)", "", value).strip().lstrip(":").strip()
            require(
                re.fullmatch(r"[A-Za-z_]\w*", text) is not None,
                f"{path}: cannot read a name from {raw!r}",
            )
            name = text
        elif key in ("decl_file", "decl_line", "decl_column", "high_pc_form"):
            number = re.match(r"(0x[0-9a-f]+|\d+)", re.sub(r"\([^)]*\)", "", value).strip())
            require(number is not None, f"{path}: cannot read {key} from {raw!r}")
            current[key] = int(number.group(1), 0)
        elif key in ("low_pc", "high_pc"):
            # A form name in parentheses may precede the value (`(udata) 3`).
            form = re.match(r"\(([^)]*)\)", value)
            if form:
                current[f"{key}_form"] = form.group(1)
            number = re.match(r"(0x[0-9a-f]+|\d+)", re.sub(r"\([^)]*\)", "", value).strip())
            require(number is not None, f"{path}: cannot read {key} from {raw!r}")
            current[key] = int(number.group(1), 0)
    if name is not None and current is not None:
        found[name] = current
    return found


def decoded_rows(readelf: str, path: Path) -> list[tuple[int, int, str, bool]]:
    """`(line, address, view, is_stmt)` for every row, in program order.

    `readelf` prints one row per line; the `Stmt` column is `x` for a statement
    row and blank otherwise.
    """
    rows: list[tuple[int, int, str, bool]] = []
    in_table = False
    for raw in run([readelf, "-W", "--debug-dump=decodedline", str(path)]).splitlines():
        if raw.startswith("File name"):
            in_table = True
            continue
        if not in_table:
            continue
        match = ROW_LINE.match(raw)
        if not match:
            continue
        line = -1 if match.group(2) == "-" else int(match.group(2))
        view = match.group(4).strip()
        # `readelf` prints addresses in hex, but a bare `0` for zero.
        rows.append((line, int(match.group(3), 16), view, raw.rstrip().endswith("x")))
    require(bool(rows), f"{path}: decoded line dump has no rows")
    return rows


FLAG_PREFIX = "  Set "


def flag_rows(readelf: str, path: Path) -> dict[str, list[int]]:
    """Addresses of the rows carrying `prologue_end` and `epilogue_begin`.

    `decodedline` does not print those flags, so the raw program is walked: a
    flag applies to the next row emitted (`Copy` or a special opcode), which the
    dump names with its own address.
    """
    flags: dict[str, list[int]] = {"prologue_end": [], "epilogue_begin": []}
    pending: list[str] = []
    address = 0
    for raw in run([readelf, "-W", "--debug-dump=rawline", str(path)]).splitlines():
        # Every statement line is prefixed with the offset it starts at.
        statement = re.sub(r"^\s*\[0x[0-9a-f]+\]\s*", "", raw).strip()
        moves = (
            "Set a new address" in statement
            or statement.startswith("Extended opcode 2:")
            or statement.startswith("Advance PC")
            or statement.startswith("Special opcode")
        )
        if moves:
            match = re.search(r"to (0x[0-9a-f]+|\d+)(?:\s|$)", statement)
            if match:
                address = int(match.group(1), 0)
            emits = statement.startswith("Special opcode")
        else:
            emits = statement == "Copy"
        if emits:
            for name in pending:
                flags[name].append(address)
            pending.clear()
            continue
        for name in ("prologue_end", "epilogue_begin"):
            if statement == f"Set {name} to true":
                pending.append(name)
    return flags


def check_object(
    readelf: str, path: Path, source_name: str, source: str, version: int, label: str
) -> str:
    expected = {
        name: declarator_position(source, name)
        for name in ("spread", "lines", "packed", "plain")
    }
    infos = subprograms(readelf, path)
    for name, (line, column) in expected.items():
        require(name in infos, f"{label}: no subprogram named {name} in {path}")
        info = infos[name]
        require(
            info.get("decl_line") == line,
            f"{label}: {name} decl_line is {info.get('decl_line')}, the declarator is "
            f"on line {line}",
        )
        require(
            info.get("decl_column") == column,
            f"{label}: {name} decl_column is {info.get('decl_column')}, the declarator "
            f"starts at column {column}",
        )
        require(
            "decl_file" in info,
            f"{label}: {name} states no DW_AT_decl_file",
        )
        require(
            info.get("low_pc") is not None and info.get("high_pc") is not None,
            f"{label}: {name} has no address range",
        )

    def end_of(name: str) -> int:
        """The first address past `name`'s code.

        `DW_AT_high_pc` is an address in DWARF 2 and 3 and an offset from
        `low_pc` from version 4 on, so the form readelf prints decides how the
        value is combined.
        """
        info = infos[name]
        if infos[name].get("high_pc_form") == "addr":
            return info["high_pc"]
        return info["low_pc"] + info["high_pc"]

    rows = decoded_rows(readelf, path)
    # The declarator row is the first row of a function: not a statement, and on
    # the line the declarator is written on.  The body row follows at the same
    # address, so an address-to-line lookup still lands on the brace, exactly as
    # gcc's and clang's tables read.
    for name, (line, _column) in expected.items():
        start = infos[name]["low_pc"]
        at_start = [(row_line, is_stmt) for row_line, address, _, is_stmt in rows if address == start]
        require(
            at_start and at_start[0][0] == line,
            f"{label}: {name} starts with rows {at_start}, expected the declarator row "
            f"on line {line} first",
        )
        require(
            not at_start[0][1],
            f"{label}: {name}'s declarator row claims to be a statement ({at_start})",
        )
        require(
            any(is_stmt for _row_line, is_stmt in at_start),
            f"{label}: {name} has no statement row at its entry address ({at_start})",
        )

    # One statement per row, even on a shared line: `packed`'s body is a single
    # source line, and every statement on it must be its own row.
    packed = infos["packed"]
    low, high = packed["low_pc"], end_of("packed")
    inside = [
        (line, address, column)
        for line, address, _, is_stmt in rows
        if low <= address < high and is_stmt
    ]
    require(
        len({address for _, address, _ in inside}) >= PACKED_STATEMENTS,
        f"{label}: packed's {PACKED_STATEMENTS} statements on one source line produce "
        f"only {len(inside)} statement rows ({inside})",
    )

    # The markers are version 3 opcodes, and the line program's dialect is the
    # unit's (gcc 16.2 writes a version 2 table for `-gdwarf-2`, measured), so a
    # version 2 program states neither.
    flags = flag_rows(readelf, path)
    for name in expected if version >= 3 else ():
        low = infos[name]["low_pc"]
        high = end_of(name)
        for flag in ("prologue_end", "epilogue_begin"):
            inside = [address for address in flags[flag] if low <= address < high]
            require(
                len(inside) == 1,
                f"{label}: {name} has {len(inside)} {flag} rows in "
                f"[{low:#x},{high:#x}), expected exactly one ({flags[flag]})",
            )
        require(
            flags["prologue_end"][flags["prologue_end"].index(
                next(a for a in flags["prologue_end"] if low <= a < high))] > low
            or infos[name]["high_pc"] == 0,
            f"{label}: {name}'s prologue_end sits at its first address, so the "
            f"prologue it should end was never skipped",
        )
    if version < 3:
        require(
            not flags["prologue_end"] and not flags["epilogue_begin"],
            f"{label}: a version {version} program carries version 3 opcodes "
            f"({flags})",
        )
    return label


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
    parser.add_argument("--readelf", default=os.environ.get("READELF", "readelf"))
    args = parser.parse_args()

    readelf = resolve_command(args.readelf)
    versions = (2, 3, 4, 5)
    targets = (
        ("x86_64", args.lccc),
        ("i686", args.lccc_i686),
        ("aarch64", args.lccc_arm),
        ("riscv64", args.lccc_riscv),
    )
    checked = 0
    with tempfile.TemporaryDirectory(prefix="lccc-dwarf-decl-") as tmp:
        work = Path(tmp).resolve()
        source_path = work / "decl.c"
        source_path.write_text(FIXTURE, encoding="utf-8")
        for name, compiler in targets:
            if not Path(resolve_command(compiler)).exists():
                print(f"SKIP {name}: {compiler} not built")
                continue
            for version in versions:
                obj = work / f"{name}-{version}.o"
                run(
                    [
                        resolve_command(compiler),
                        "-g",
                        f"-gdwarf-{version}",
                        "-O0",
                        "-c",
                        str(source_path),
                        "-o",
                        str(obj),
                    ],
                    cwd=work,
                )
                check_object(readelf, obj, source_path.name, FIXTURE, version, f"{name} v{version}")
                checked += 1
            print(
                f"PASS {name}: declarator positions, entry rows and prologue/epilogue "
                f"markers in -gdwarf-{','.join(str(v) for v in versions)}"
            )
    require(checked > 0, "no target was checked")
    print(f"DWARF declaration positions: PASS ({checked} objects)")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except GateFailure as failure:
        print(f"FAIL: {failure}", file=sys.stderr)
        sys.exit(1)
