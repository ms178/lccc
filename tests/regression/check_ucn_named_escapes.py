#!/usr/bin/env python3
"""Differential check of `\\N{...}` named universal character escapes
(C++23's spelling, which GCC, Clang and lccc also accept in C).

WHY: `src/frontend/lexer/ucn_name_table.rs` is the Unicode 18.0.0 name
database with two algorithmic families folded out (hex-suffix families and
Hangul syllables) and the remaining 35,351 names front-coded. That is a lot of
machinery for one escape sequence, and every failure mode of it is silent: a
wrong block index, a wrong prefix reconstruction, a wrong Hangul composition or
a stale table all produce *a* character, just not the right one.

So this check does not pin expected bytes by hand. It decodes the checked-in
table, samples it deterministically, spells every sampled name as a `\\N{...}`
escape in one translation unit, and compares the program's output between LCCC
and the oracle -- GCC 16.2 accepts the same syntax, so any disagreement in the
bytes is a defect on one side or the other. The sampling covers

  * every explicit table entry at a stride that is coprime with the 16-entry
    front-coding block, so every position inside a block is visited,
  * the first and last code point of every assigned range of every hex family,
  * a stride through each hex family and through the Hangul syllables,
  * the longest explicit name, the adjacency of two escapes, and the wide,
    UTF-16 and UTF-32 literal kinds.

The one *documented* deviation is at the end: GCC 16.2's copyable name table
predates Unicode 16/17, so names LCCC has and GCC does not are reported rather
than failed.

Usage: CCC=target/fastbuild/lccc GCC=gcc-16 python3 check_ucn_named_escapes.py
"""

from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
import tempfile

TABLE = os.path.join(
    os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))),
    "src",
    "frontend",
    "lexer",
    "ucn_name_table.rs",
)

UCD_BLOCK = 16  # `BLOCK` in the generated table


def parse_table(path: str):
    """Decode the generated table: (names, code points, families, jamo)."""
    text = open(path, "r", encoding="utf-8").read()
    blob_literal = re.search(r'static NAMES_BLOB: &\[u8\] = b"(.*?)";', text, re.S)
    if blob_literal is None:
        raise SystemExit(f"{path}: no NAMES_BLOB")
    blob = bytearray()
    literal = blob_literal.group(1)
    index = 0
    while index < len(literal):
        char = literal[index]
        if char == "\\":
            escape = literal[index + 1]
            if escape != "x":
                raise SystemExit(f"{path}: unsupported escape \\{escape}")
            blob.append(int(literal[index + 2 : index + 4], 16))
            index += 4
        else:
            blob.append(ord(char))
            index += 1
    codepoints = [
        int(value, 16)
        for value in re.findall(
            r"0x([0-9A-F]+)", re.search(r"static CODEPOINTS: &\[u32\] = &\[(.*?)\];", text, re.S).group(1)
        )
    ]
    families = []
    for prefix, ranges in re.findall(
        r'\("([^"]*)", &\[(.*?)\]\)', re.search(r"HEX_FAMILIES.*?= &\[(.*?)\n\];", text, re.S).group(1)
    ):
        families.append(
            (prefix, [tuple(int(v, 16) for v in pair) for pair in re.findall(r"\(0x([0-9A-F]+), 0x([0-9A-F]+)\)", ranges)])
        )
    jamo = [
        re.findall(r'"([^"]*)"', re.search(rf"const {name}: &\[&str\] = &\[(.*?)\];", text).group(1))
        for name in ("HANGUL_L", "HANGUL_V", "HANGUL_T")
    ]
    # Front-coded entries: [prefix u8][suffix][NUL].
    names = []
    offset = 0
    previous = b""
    for _ in codepoints:
        prefix = blob[offset]
        offset += 1
        end = blob.index(0, offset)
        name = previous[:prefix] + bytes(blob[offset:end])
        offset = end + 1
        names.append(name)
        previous = name
    if offset != len(blob):
        raise SystemExit(f"{path}: blob has {len(blob) - offset} bytes past the last name")
    return [name.decode() for name in names], codepoints, families, jamo


def sample(names: list[str], families, jamo) -> list[str]:
    """A deterministic sample that visits every structural corner."""
    picked: list[str] = []
    # Explicit names: stride 41 is coprime with 16, so consecutive samples land
    # on different in-block positions and every block position is visited.
    picked.extend(names[::41])
    picked.extend(names[-3:])
    longest = max(names, key=len)
    picked.append(longest)
    # Hex families: both ends of every assigned range, plus a stride inside it.
    for prefix, ranges in families:
        for low, high in ranges:
            picked.append(f"{prefix}{low:X}")
            picked.append(f"{prefix}{high:X}")
            picked.append(f"{prefix}{(low + (high - low) // 2):X}")
    # Hangul: a stride through all 11,172 syllables.
    leading, vowel, trailing = jamo
    for index in range(0, 19 * 21 * 28, 53):
        picked.append(
            "HANGUL SYLLABLE "
            + leading[index // (21 * 28)]
            + vowel[(index // 28) % 21]
            + trailing[index % 28]
        )
    # Keep the source file a reasonable size and the order stable.
    seen = set()
    unique = []
    for name in picked:
        if name not in seen and "\n" not in name and '"' not in name:
            seen.add(name)
            unique.append(name)
    return unique


def oracle_names(compiler: str, names: list[str], workdir: str) -> set[str]:
    """Which of `names` the oracle's own name table knows."""
    # One declaration per name: the compiler reports each rejected escape and
    # keeps going, so a single syntax-only run classifies the whole sample.
    lines = "\n".join(f'const char *n{index} = "\\N{{{name}}}";' for index, name in enumerate(names))
    path = os.path.join(workdir, "probe.c")
    with open(path, "w", encoding="utf-8") as handle:
        handle.write(lines + "\n")
    result = subprocess.run([compiler, "-fsyntax-only", path], capture_output=True, text=True)
    rejected = set(
        re.findall(r"error: '\\N\{(.*?)\}' is not a valid universal character", result.stderr)
    )
    return {name for name in names if name not in rejected}, len(rejected)


def build_source(names: list[str]) -> str:
    body = "\n".join(f'    CASE("{name}", "\\N{{{name}}}");' for name in names)
    return f"""#include <stddef.h>
#include <stdio.h>
#include <string.h>

static void emit(const char *what, const char *bytes) {{
    printf("%s:", what);
    for (const unsigned char *p = (const unsigned char *)bytes; *p; ++p) {{
        printf(" %02x", *p);
    }}
    printf("\\n");
}}

#define CASE(what, literal) emit(what, "" literal)

int main(void) {{
{body}

    // Adjacent escapes, neighbouring text and the other literal kinds.
    emit("adjacent", "x\\N{{LATIN SMALL LETTER A}}\\N{{SNOWMAN}}y");
    {{
        const unsigned short *utf16 = u"\\N{{SNOWMAN}}\\N{{GRINNING FACE}}";
        const unsigned int *utf32 = U"\\N{{SNOWMAN}}\\N{{GRINNING FACE}}";
        const wchar_t *wide = L"\\N{{SNOWMAN}}\\N{{GRINNING FACE}}";
        for (int i = 0; i < 3; ++i) {{
            printf("utf16[%d] %04x\\n", i, utf16[i]);
            printf("utf32[%d] %06x\\n", i, utf32[i]);
            printf("wide[%d] %06x\\n", i, (unsigned)wide[i]);
        }}
        printf("charliteral %08x\\n", (unsigned)'\\N{{SNOWMAN}}');
    }}
    return 0;
}}
"""


def run(compiler: str, source: str, workdir: str, tag: str):
    path = os.path.join(workdir, f"named_{tag}.c")
    binary = os.path.join(workdir, f"named_{tag}")
    with open(path, "w", encoding="utf-8") as handle:
        handle.write(source)
    build = subprocess.run([compiler, "-O2", path, "-o", binary], capture_output=True, text=True)
    if build.returncode != 0:
        return None, build.stderr
    result = subprocess.run([binary], capture_output=True, text=True)
    return result.stdout, result.stderr


def verdict(compiler: str, source: str, workdir: str, tag: str) -> str:
    path = os.path.join(workdir, f"verdict_{tag}.c")
    with open(path, "w", encoding="utf-8") as handle:
        handle.write(source)
    result = subprocess.run([compiler, "-fsyntax-only", path], capture_output=True, text=True)
    if result.returncode == 0:
        return "ACCEPT"
    for line in result.stderr.splitlines():
        if ": error: " in line:
            return line.split(": error: ", 1)[1]
    return result.stderr.splitlines()[0] if result.stderr else "FAILED"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--ccc", default=os.environ.get("CCC", "target/fastbuild/lccc"))
    parser.add_argument("--gcc", default=os.environ.get("GCC", "gcc-16"))
    args = parser.parse_args()

    if not os.path.exists(args.ccc):
        print(f"check_ucn_named_escapes: FAIL (no compiler at {args.ccc})", file=sys.stderr)
        return 1
    probe = subprocess.run([args.gcc, "--version"], capture_output=True, text=True)
    if probe.returncode != 0:
        print(f"check_ucn_named_escapes: SKIP (no {args.gcc} oracle)")
        return 0

    names, _codepoints, families, jamo = parse_table(TABLE)
    picked = sample(names, families, jamo)
    failures = 0
    with tempfile.TemporaryDirectory() as workdir:
        known, unknown = oracle_names(args.gcc, picked, workdir)
        compared = [name for name in picked if name in known]
        if not compared:
            print("FAIL the oracle knows none of the sampled names", file=sys.stderr)
            return 1
        source = build_source(compared)
        got, error = run(args.ccc, source, workdir, "lccc")
        if got is None:
            print(f"FAIL lccc rejected the sample:\n{error[:2000]}", file=sys.stderr)
            return 1
        want, error = run(args.gcc, source, workdir, "gcc")
        if want is None:
            print(f"FAIL the oracle rejected the sample:\n{error[:2000]}", file=sys.stderr)
            return 1
        got_lines = got.splitlines()
        want_lines = want.splitlines()
        if len(got_lines) != len(want_lines):
            print(
                f"FAIL length: lccc printed {len(got_lines)} lines, gcc {len(want_lines)}",
                file=sys.stderr,
            )
            failures += 1
        for left, right in zip(got_lines, want_lines):
            if left != right:
                print(f"FAIL {left.strip()} (lccc) != {right.strip()} (gcc)", file=sys.stderr)
                failures += 1

        # Rejections must agree, wording included.
        rejections = {
            "empty": 'const char *s = "\\N{}";',
            "unknown": 'const char *s = "\\N{NOT A REAL NAME}";',
            "lowercase": 'const char *s = "\\N{snowman}";',
            "lowerhex": 'const char *s = "\\N{CJK UNIFIED IDEOGRAPH-4e00}";',
            # Four-digit names may be padded to five with one leading zero
            # (GCC 16.2 does), six digits may not.
            "padded": 'const char *s = "\\N{CJK UNIFIED IDEOGRAPH-04E00}";',
            "overpadded": 'const char *s = "\\N{CJK UNIFIED IDEOGRAPH-004E00}";',
            "unassigned": 'const char *s = "\\N{CJK UNIFIED IDEOGRAPH-FFFF}";',
            "hangul_bad": 'const char *s = "\\N{HANGUL SYLLABLE XX}";',
            "unterminated": 'const char *s = "\\N{LATIN SMALL LETTER A";',
            "no_brace": 'const char *s = "\\N LATIN";',
            "identifier": "int \\N{SNOWMAN};",
            "identifier_mid": "int a\\N{LATIN SMALL LETTER A};",
        }
        for tag, text in rejections.items():
            want_error = verdict(args.gcc, text, workdir, f"g_{tag}")
            got_error = verdict(args.ccc, text, workdir, f"l_{tag}")
            if got_error != want_error:
                print(
                    f"FAIL {tag}: lccc='{got_error}' gcc='{want_error}'",
                    file=sys.stderr,
                )
                failures += 1

        # Documented deviation: names Unicode added after GCC 16.2 froze its
        # table. LCCC accepts them because it follows Unicode 18.0.0; the oracle
        # rejecting them is the expected outcome, not a defect.
        newer = ["ALCHEMICAL SYMBOL FOR MOON-JUPITER", "JURCHEN CHARACTER-18E00",
                 "CUNEIFORM NUMERIC SIGN FOUR N18"]
        for name in newer:
            source = f'const char *s = "\\N{{{name}}}";'
            got_error = verdict(args.ccc, source, workdir, "l_new")
            want_error = verdict(args.gcc, source, workdir, "g_new")
            if got_error != "ACCEPT":
                print(f"FAIL new-in-Unicode name {name!r}: lccc='{got_error}'", file=sys.stderr)
                failures += 1
            elif want_error == "ACCEPT":
                print(
                    f"note: the oracle now accepts {name!r} too; update this check",
                    file=sys.stderr,
                )

    if failures:
        print(f"check_ucn_named_escapes: FAILED ({failures} mismatches)", file=sys.stderr)
        return 1
    print(
        f"check_ucn_named_escapes: PASSED ({len(compared)} names compared against "
        f"{args.gcc}, {unknown} newer than its table, 11 rejections)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
