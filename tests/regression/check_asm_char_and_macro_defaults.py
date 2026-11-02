#!/usr/bin/env python3
"""Verify GAS character literals and quoted/char .macro defaults across all 4 assemblers.

Compares the emitted .data section bytes of lccc (x86_64), lccc-i686, lccc-arm,
and lccc-riscv byte-for-byte against GNU `as` on:
  - Quoted ('X') and unclosed ('X) GAS character literals containing spaces,
    commas, semicolons, comment characters (#, /, @), parentheses, C integer
    suffix letters ('u, 'l, 'z), and a trailing unclosed space literal (' \\n).
  - .macro parameter defaults with double-quoted strings containing commas,
    spaces, :req/:vararg substrings, and escaped quotes (\\"), plus positional
    and keyword=value overrides.
  - .macro parameter defaults with character literals (a=',', b=' ', c=';').
"""

import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path


HEX_ROW = re.compile(r"^[0-9a-fA-F]+\s+((?:[0-9a-fA-F]{2,8}\s)+)")


def extract_section_hex(obj_path: Path, section: str = ".data") -> str:
    proc = subprocess.run(
        ["objdump", "-s", "-j", section, str(obj_path)],
        text=True,
        capture_output=True,
        check=False,
    )
    if proc.returncode != 0:
        raise RuntimeError(f"objdump failed: {proc.stderr.strip()}")
    chunks = []
    for line in proc.stdout.splitlines():
        m = HEX_ROW.match(line.strip())
        if m:
            chunks.append("".join(m.group(1).split()).lower())
    return "".join(chunks)


def assemble_and_dump(cmd: list[str], asm: str) -> str:
    with tempfile.NamedTemporaryFile(suffix=".o") as obj:
        proc = subprocess.run(
            [*cmd, obj.name],
            input=asm,
            text=True,
            capture_output=True,
            check=False,
        )
        if proc.returncode != 0:
            raise AssertionError(
                f"command {' '.join(cmd)} failed ({proc.returncode}):\n{proc.stderr.strip()}"
            )
        return extract_section_hex(Path(obj.name))


CASES: list[tuple[str, str]] = [
    (
        "quoted_char_literals",
        ".data\n.byte 'A', ' ', ',', ';', '#', '/', '@', '(', ')', '\\n', '\\t', '\\\\', '\\''\n",
    ),
    (
        "unclosed_char_literals_and_eol_space",
        ".data\n.byte 'x, 'y, ',, ';, '#, 'u, 'l, 'z, ' \n",
    ),
    (
        "macro_quoted_defaults_and_overrides",
        (
            '.data\n'
            '.macro m p="1, 2" q="x y:req" r=9\n'
            '.ascii "<\\p><\\q><\\r>"\n'
            '.endm\n'
            'm\n'
            'm "3, 4", "z", 8\n'
            'm r=7\n'
            'm q="k, v", p="a b"\n'
        ),
    ),
    (
        "macro_escaped_quotes_in_default_and_call",
        (
            '.data\n'
            '.macro m p="ab\\"cd" q=5\n'
            '.ascii "<\\p>"\n'
            '.byte \\q\n'
            '.endm\n'
            'm\n'
            'm "ef\\"gh", 6\n'
        ),
    ),
    (
        "macro_char_literal_defaults",
        (
            ".data\n"
            ".macro m a=',', b=' ', c=';'\n"
            ".byte \\a, \\b, \\c\n"
            ".endm\n"
            "m\n"
            "m 'X', 'Y', 'Z'\n"
            "m b='!'\n"
        ),
    ),
]


def main() -> int:
    root = Path(__file__).resolve().parents[2]
    ccc = Path(os.environ.get("CCC", str(root / "target/fastbuild/lccc")))
    bin_dir = ccc.parent
    as_bin = shutil.which(os.environ.get("AS", "as"))
    if not as_bin:
        print("FAIL: GNU `as` not found in PATH", file=sys.stderr)
        return 1

    targets = [
        ("x86_64", ccc),
        ("i686", Path(os.environ.get("CCC_I686", str(bin_dir / "lccc-i686")))),
        ("aarch64", Path(os.environ.get("CCC_ARM", str(bin_dir / "lccc-arm")))),
        ("riscv64", Path(os.environ.get("CCC_RISCV", str(bin_dir / "lccc-riscv")))),
    ]
    for arch, path in targets:
        if not path.is_file() or not os.access(path, os.X_OK):
            print(f"FAIL: missing executable for {arch}: {path}", file=sys.stderr)
            return 1

    checked = 0
    for case_name, asm in CASES:
        expected = assemble_and_dump([as_bin, "-o"], asm)
        assert expected, f"{case_name}: GNU as produced empty .data"
        for arch, bin_path in targets:
            got = assemble_and_dump(
                [str(bin_path), "-x", "assembler", "-c", "-", "-o"],
                asm,
            )
            if got != expected:
                print(
                    f"FAIL: {case_name} [{arch}]: expected {expected}, got {got}",
                    file=sys.stderr,
                )
                return 1
            checked += 1

    print(f"PASS: {checked} assembler character-literal and .macro default checks matched GNU as")
    return 0


if __name__ == "__main__":
    sys.exit(main())
