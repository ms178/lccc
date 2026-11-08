#!/usr/bin/env python3
"""Contracts for the allocate-before-filter ratchet (compiler-free).

The important test here is `test_nested_parens_are_matched`: the first version of
this ratchet used `[^)]*` for the `trimmed(...)` argument, which cannot match
`trimmed(store.get(i))`. It reported PASS on both the fixed *and* the unfixed
tree -- a vacuous gate, which is worse than no gate. These fixtures pin the
regex against the real shape so that regression cannot recur.
"""
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import check_peephole_alloc_before_filter as gate


class DetectionTests(unittest.TestCase):
    def test_nested_parens_are_matched(self):
        # The exact real-world shape. `store.get(i)` nests parentheses.
        src = (
            "        let load = infos[i].trimmed(store.get(i)).to_string();\n"
            "        let Some((q, mem, t)) = parse_memory_load(&load) else {\n"
            "            i += 1;\n"
            "            continue;\n"
            "        };\n"
        )
        got = gate.violations(src)
        self.assertEqual(len(got), 1, "the real shape must be detected")
        self.assertEqual(got[0][1], "load")
        self.assertEqual(got[0][2], "parse_memory_load")

    def test_indexed_receiver_is_matched(self):
        src = (
            "let t = infos[cmov_i].trimmed(store.get(cmov_i)).to_string();\n"
            "let Some((cc, w, s, d)) = parse_cmov(&t) else { continue; };\n"
        )
        self.assertEqual(len(gate.violations(src)), 1)

    def test_negated_bool_filter_is_matched(self):
        src = (
            "let k_text = infos[k].trimmed(store.get(k)).to_string();\n"
            "if !is_full_flag_clobber(&k_text) {\n"
            "    continue;\n"
            "}\n"
        )
        got = gate.violations(src)
        self.assertEqual(len(got), 1)
        self.assertEqual(got[0][2], "is_full_flag_clobber")

    def test_intervening_comment_and_blank_do_not_hide_it(self):
        src = (
            "let x = infos[i].trimmed(store.get(i)).to_string();\n"
            "\n"
            "            // a comment must not defeat the scan\n"
            "let Some(a) = parse_x(&x) else { continue; };\n"
        )
        self.assertEqual(len(gate.violations(src)), 1)

    def test_probe_then_allocate_is_compliant(self):
        # The accepted shape for a parser whose result borrows the text.
        src = (
            "let load = infos[i].trimmed(store.get(i));\n"
            "if parse_memory_load(load).is_none() {\n"
            "    i += 1;\n"
            "    continue;\n"
            "}\n"
            "let load = load.to_string();\n"
            "let Some((q, mem, t)) = parse_memory_load(&load) else {\n"
            "    i += 1;\n"
            "    continue;\n"
            "};\n"
        )
        self.assertEqual(gate.violations(src), [])

    def test_direct_borrowed_parse_is_compliant(self):
        src = (
            "let copy1 = infos[i].trimmed(store.get(i));\n"
            "let Some((q, a, b)) = parse_reg_copy(copy1) else { continue; };\n"
        )
        self.assertEqual(gate.violations(src), [])

    def test_inlined_borrowed_parse_is_compliant(self):
        src = "if !is_full_flag_clobber(infos[k].trimmed(store.get(k))) { continue; }\n"
        self.assertEqual(gate.violations(src), [])

    def test_different_variable_is_not_a_violation(self):
        # The filter tests some other value; the copy is genuinely needed.
        src = (
            "let owned = infos[i].trimmed(store.get(i)).to_string();\n"
            "let Some(a) = parse_x(&other) else { continue; };\n"
        )
        self.assertEqual(gate.violations(src), [])

    def test_unconditional_owned_use_is_not_a_violation(self):
        # No rejecting filter follows: the text is needed regardless.
        src = (
            "let text = infos[n].trimmed(store.get(n)).to_string();\n"
            'rename.push((n, text.replace("%rax", "%rcx")));\n'
        )
        self.assertEqual(gate.violations(src), [])

    def test_line_numbers_are_one_based(self):
        src = "\n\nlet v = infos[i].trimmed(store.get(i)).to_string();\nlet Some(a) = p(&v) else { continue; };\n"
        self.assertEqual(gate.violations(src)[0][0], 3)


class MainTests(unittest.TestCase):
    def _run(self, files: dict[str, str]) -> int:
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        root = Path(tmp.name)
        for name, text in files.items():
            p = root / name
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_text(text)
        argv = ["gate", "--root", str(root)]
        old = sys.argv
        try:
            sys.argv = argv
            return gate.main()
        finally:
            sys.argv = old

    def test_clean_tree_passes(self):
        self.assertEqual(self._run({"a.rs": "fn f() {}\n"}), 0)

    def test_violation_fails(self):
        bad = ("let t = infos[i].trimmed(store.get(i)).to_string();\n"
               "let Some(a) = p(&t) else { continue; };\n")
        self.assertEqual(self._run({"passes/a.rs": bad}), 1)

    def test_empty_tree_is_not_a_vacuous_pass(self):
        # No sources means the scan proved nothing; it must not report PASS.
        self.assertEqual(self._run({}), 2)

    def test_missing_directory_fails(self):
        old = sys.argv
        try:
            sys.argv = ["gate", "--root", "/nonexistent-peephole-dir"]
            self.assertEqual(gate.main(), 2)
        finally:
            sys.argv = old


if __name__ == "__main__":
    unittest.main()
