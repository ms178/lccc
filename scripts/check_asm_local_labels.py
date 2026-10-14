#!/usr/bin/env python3
"""Check every local-symbol operand in i686 assembly data, not just operand one.

This is a lexical dangling-reference check, not an assembler: quoted symbol
names are references in data expressions, but comments and longer symbol names
are not. Assembly/relocation validation remains the compiler regression's job.
"""

import re
import sys
import unittest
from pathlib import Path

SYMBOL = r"[A-Za-z_.$][\w.$]*"
DEFINITION = re.compile(rf"^\s*({SYMBOL}|[0-9]+):")
DATA = re.compile(r"^\s*\.(long|quad|word|short|byte|hword|value)\s+(.+)$")
LOCAL = re.compile(r"(?<![\w.$])\.L[\w.$]+")


def statements(assembly):
    """Split GAS statements, respecting quoted strings and # line comments."""
    for line in assembly.splitlines():
        start = 0
        quote = None
        escaped = False
        for i, char in enumerate(line):
            if quote:
                if escaped:
                    escaped = False
                elif char == "\\":
                    escaped = True
                elif char == quote:
                    quote = None
            elif char in "\"'":
                quote = char
            elif char == "#":
                yield line[start:i]
                break
            elif char == ";":
                yield line[start:i]
                start = i + 1
        else:
            yield line[start:]


def undefined_data_labels(assembly):
    defined = set()
    references = []
    for statement in statements(assembly):
        while match := DEFINITION.match(statement):
            defined.add(match[1])
            statement = statement[match.end():]
        if match := DATA.match(statement):
            references.extend((match[1], name) for name in LOCAL.findall(match[2]))
    return [(kind, name) for kind, name in references if name not in defined]


class LocalLabelTests(unittest.TestCase):
    def test_each_operand_and_each_supported_width(self):
        for kind in ("long", "quad", "word", "short", "byte", "hword", "value"):
            with self.subTest(kind=kind):
                self.assertEqual(
                    undefined_data_labels(f".Lok:\n.{kind} .Lok - .Lmissing, .Lother"),
                    [(kind, ".Lmissing"), (kind, ".Lother")],
                )

    def test_forward_references_and_multiple_statements(self):
        self.assertEqual(undefined_data_labels(
            ".long .La - .Lb; .quad .Lc\n  .La: .Lb: .Lc: # .Lcomment\n"
        ), [])

    def test_comments_and_symbol_boundaries(self):
        self.assertEqual(undefined_data_labels(
            ".long prefix.Lnot_local, .Lok # .Lcomment\n.Lok:\n"
            ".string \".Lstring; # .Ltext\"\n"
        ), [])

    def test_quoted_data_symbols_are_references(self):
        self.assertEqual(undefined_data_labels('.quad ".Lmissing" # .Lcomment'),
                         [("quad", ".Lmissing")])

    def test_dot_and_dollar_are_part_of_the_symbol(self):
        self.assertEqual(undefined_data_labels(".Lbase:\n.long .Lbase.extra, .Lbase$tail"),
                         [("long", ".Lbase.extra"), ("long", ".Lbase$tail")])

    def test_comment_cannot_supply_a_definition(self):
        self.assertEqual(undefined_data_labels("# .Lmissing:\n.long .Lmissing"),
                         [("long", ".Lmissing")])

    def test_escaped_quote_keeps_comment_and_semicolon_in_string(self):
        self.assertEqual(undefined_data_labels('.string "a\\\"; # .Ltext"\n.long .Lmissing'),
                         [("long", ".Lmissing")])


if __name__ == "__main__":
    if sys.argv[1:] == ["--selftest"]:
        unittest.main(argv=[sys.argv[0]])
    elif len(sys.argv) == 2:
        missing = undefined_data_labels(Path(sys.argv[1]).read_text())
        if missing:
            sys.exit("undefined data label(s): " + ", ".join(f".{k} {n}" for k, n in missing))
    else:
        sys.exit(f"usage: {sys.argv[0]} ASSEMBLY | --selftest")
