"""Source-only completeness check; no compiler/build/executable is invoked.

Keep three independent inventories: enum declarations, the shared IR test
matrix, and the args-write match. Parsing is deliberately limited to their
current unit-VecStore / literal-list / matches! forms. An unfamiliar form must
fail, not silently lose variants. Rust tests separately exercise predicate
semantics and all optimizer consumers; this test does not replace them.
"""
from collections import Counter
from pathlib import Path
import re
import unittest

SOURCE = Path(__file__).resolve().parents[2] / "src/ir/intrinsics.rs"
IDENT = r"[A-Za-z_][A-Za-z_0-9]*"
ENUM = r"^pub enum IntrinsicOp\s*\{(.*?)^\}"
MATRIX = r"^pub\(crate\) const VECTOR_STORES: \[IntrinsicOp; (\d+)\] = \[(.*?)^\];"
WRITES = r"^    pub fn writes_memory_via_args\(&self\) -> bool \{(.*?)^    \}"


def require(condition, message):
    if not condition:
        raise AssertionError(message)


def extract(pattern, source):
    matches = list(re.finditer(pattern, source, re.S | re.M))
    require(len(matches) == 1, "inventory declaration missing or ambiguous")
    return matches[0]


def without_comments(body):
    """Strip line/nested block comments from these string-free declarations."""
    result = []
    i = 0
    while i < len(body):
        if body.startswith("//", i):
            end = body.find("\n", i)
            i = len(body) if end < 0 else end
        elif body.startswith("/*", i):
            depth = 1
            i += 2
            while depth and i < len(body):
                if body.startswith("/*", i):
                    depth += 1
                    i += 2
                elif body.startswith("*/", i):
                    depth -= 1
                    i += 2
                else:
                    i += 1
            require(depth == 0, "unterminated inventory comment")
            result.append(" ")
        else:
            result.append(body[i])
            i += 1
    return "".join(result)


def names(body, separator, qualified=False):
    pattern = (r"IntrinsicOp::" if qualified else "") + f"({IDENT})"
    items = without_comments(body).strip().split(separator)
    if separator == "," and not items[-1].strip():
        items.pop()  # A single optional trailing comma, not an empty entry.
    result = []
    for item in items:
        match = re.fullmatch(pattern, item.strip())
        require(match is not None, f"unsupported inventory entry: {item!r}")
        result.append(match[1])
    duplicates = [name for name, count in Counter(result).items() if count > 1]
    require(not duplicates, f"duplicate inventory entries: {duplicates}")
    return result


def enum_names(body):
    # Some unrelated opcodes have tuple payloads (e.g. signed scalar FMA).
    # Split only top-level commas; VecStore variants must remain unit variants.
    clean = without_comments(body)
    entries = []
    depth = 0
    start = 0
    for i, char in enumerate(clean):
        if char == "(":
            depth += 1
        elif char == ")":
            depth -= 1
            require(depth >= 0, "unbalanced enum payload")
        elif char == "," and depth == 0:
            entries.append(clean[start:i].strip())
            start = i + 1
    require(depth == 0 and not clean[start:].strip(), "unterminated enum entry")
    result = []
    for entry in entries:
        match = re.fullmatch(f"({IDENT})(\\(\\s*{IDENT}(?:\\s*,\\s*{IDENT})*\\s*,?\\s*\\))?", entry)
        require(match is not None, f"unsupported inventory entry: {entry!r}")
        require(not (match[1].startswith("VecStore") and match[2]),
                f"unsupported inventory entry: payload-bearing {match[1]}")
        result.append(match[1])
    require(len(set(result)) == len(result), "duplicate enum variants")
    return result


def inventory(source):
    declared = enum_names(extract(ENUM, source)[1])
    stores = {name for name in declared if name.startswith("VecStore")}
    require(stores, "no VecStore variants found")
    matrix_match = extract(MATRIX, source)
    matrix = names(matrix_match[2], ",", qualified=True)
    require(len(matrix) == int(matrix_match[1]), "matrix array length disagrees")
    match = re.fullmatch(r"\s*matches!\(\s*self\s*,(.*?)\)\s*",
                         without_comments(extract(WRITES, source)[1]), re.S)
    require(match is not None, "write classification no longer a literal matches! list")
    writes = names(match[1], "|", qualified=True)
    for label, entries in (("matrix", matrix), ("writes", writes)):
        require(set(entries) == stores,
                f"{label}: missing={sorted(stores - set(entries))}, "
                f"extra={sorted(set(entries) - stores)}")
    return stores


class IntrinsicStoreInventoryTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.source = SOURCE.read_text(encoding="utf-8")
        cls.stores = sorted(inventory(cls.source))

    def matrix(self, source, entries):
        match = extract(MATRIX, source)
        body = "\n".join(f"    IntrinsicOp::{name}," for name in entries)
        return (source[:match.start()] +
                f"pub(crate) const VECTOR_STORES: [IntrinsicOp; {len(entries)}] = [\n" +
                body + "\n];" + source[match.end():])

    def writes(self, source, entries):
        match = extract(WRITES, source)
        body = " | ".join(f"IntrinsicOp::{name}" for name in entries)
        return (source[:match.start(1)] + f"\n        matches!(self, {body})\n" +
                source[match.end(1):])

    def test_repository_inventory(self):
        self.assertEqual(inventory(self.source), set(self.stores))

    def test_each_missing_matrix_entry_fails_even_with_correct_length(self):
        for omitted in self.stores:
            with self.subTest(omitted=omitted), self.assertRaisesRegex(AssertionError, "matrix: missing"):
                inventory(self.matrix(self.source, [x for x in self.stores if x != omitted]))

    def test_each_missing_write_classification_fails(self):
        for omitted in self.stores:
            with self.subTest(omitted=omitted), self.assertRaisesRegex(AssertionError, "writes: missing"):
                inventory(self.writes(self.source, [x for x in self.stores if x != omitted]))

    def test_new_unshaped_variant_requires_both_inventories(self):
        name = "VecStoreFuture_widthless"
        match = extract(ENUM, self.source)
        source = self.source[:match.end(1)] + f"    {name},\n" + self.source[match.end(1):]
        with self.assertRaisesRegex(AssertionError, "matrix: missing"):
            inventory(source)
        source = self.matrix(source, self.stores + [name])
        with self.assertRaisesRegex(AssertionError, "writes: missing"):
            inventory(source)
        self.assertIn(name, inventory(self.writes(source, self.stores + [name])))

    def test_duplicate_does_not_hide_an_omission(self):
        entries = self.stores[:-1] + [self.stores[0]]
        for source in (self.matrix(self.source, entries), self.writes(self.source, entries)):
            with self.assertRaisesRegex(AssertionError, "duplicate"):
                inventory(source)

    def test_agreeing_phantom_lists_do_not_override_enum(self):
        entries = self.stores + ["VecStorePhantom"]
        source = self.writes(self.matrix(self.source, entries), entries)
        with self.assertRaisesRegex(AssertionError, "extra=.*VecStorePhantom"):
            inventory(source)

    def test_comments_do_not_add_variants(self):
        source = self.source.replace("pub enum IntrinsicOp {", """pub enum IntrinsicOp {
    // VecStoreLineComment,
    /* VecStoreOuter, /* VecStoreInner, */ */
""", 1)
        self.assertEqual(inventory(source), set(self.stores))

    def test_unknown_enum_shape_fails_closed(self):
        source = self.source.replace("pub enum IntrinsicOp {",
                                     "pub enum IntrinsicOp {\n    VecStorePayload(u32),", 1)
        with self.assertRaisesRegex(AssertionError, "unsupported inventory entry"):
            inventory(source)

    def test_changed_write_expression_fails_closed(self):
        match = extract(WRITES, self.source)
        source = self.source[:match.start(1)] + "\n        false\n" + self.source[match.end(1):]
        with self.assertRaisesRegex(AssertionError, "no longer a literal"):
            inventory(source)

    def test_declared_length_and_comment_errors_fail_closed(self):
        source = self.source.replace(f"[IntrinsicOp; {len(self.stores)}]",
                                     f"[IntrinsicOp; {len(self.stores) + 1}]", 1)
        with self.assertRaisesRegex(AssertionError, "array length"):
            inventory(source)
        with self.assertRaisesRegex(AssertionError, "unterminated"):
            without_comments("/* VecStoreUnclosed,")


if __name__ == "__main__":
    unittest.main()
