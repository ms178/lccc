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

    def test_free_function_idiom_is_matched(self):
        # i686 uses a free function, not a method. The original regex required a
        # literal `.trimmed(`, so this whole architecture was scanned vacuously
        # while five real sites sat in it. This fixture is the real i686 shape.
        src = (
            "    let jcc = trimmed(store, &infos[a], a).to_string();\n"
            "    let Some((cc, true_label)) = parse_condjmp(&jcc) else {\n"
            "        return false;\n"
            "    };\n"
        )
        got = gate.violations(src)
        self.assertEqual(len(got), 1, "the i686 free-function shape must be detected")
        self.assertEqual(got[0][1], "jcc")
        self.assertEqual(got[0][2], "parse_condjmp")

    def test_free_function_compliant_shape_is_accepted(self):
        src = (
            "    let Some((cc, true_label)) = parse_condjmp(trimmed(store, &infos[a], a)) else {\n"
            "        return false;\n"
            "    };\n"
        )
        self.assertEqual(gate.violations(src), [])

    def test_default_root_covers_every_backend(self):
        # An x86-only default is what made the gate vacuous for i686.
        self.assertEqual(gate.PEEPHOLE, gate.ROOT / "src/backend")
        # The four codegen backends: x86-64, i686, arm/aarch64, riscv.
        for arch in ("x86", "i686", "arm", "riscv"):
            self.assertTrue(
                (gate.PEEPHOLE / arch / "codegen").is_dir(),
                f"{arch}/codegen must be inside the default scan root",
            )

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


class PreviouslyInvisibleShapes(unittest.TestCase):
    """The four filter shapes the two-regex version could not see.

    Together these hid 23 of the 66 real sites, including the pass ablation
    measured at 30.4% of the whole peephole. Each fixture is the real shape from
    the tree, not an invented one.
    """

    def test_if_let_else_chain_is_detected(self):
        # relay_and_lea.rs::fold_copy_into_lea_base -- the 30.4% pass.
        src = (
            '        let lea = infos[li].trimmed(store.get(li)).to_string();\n'
            '        let (lea_rest, prod_w) = if let Some(r) = lea.strip_prefix("leaq ") {\n'
            '            (r, 64u8)\n'
            '        } else if let Some(r) = lea.strip_prefix("leal ") {\n'
            '            (r, 32u8)\n'
            '        } else {\n'
            '            continue;\n'
            '        };\n'
        )
        got = gate.violations(src)
        self.assertEqual(len(got), 1, "the if-let/else chain must be one statement")
        self.assertEqual(got[0][1], "lea")

    def test_positive_guard_is_detected(self):
        # `if pred(&t) { continue; }` -- the old gate only knew `if !pred(&t)`.
        src = (
            '    let lea = infos[i].trimmed(store.get(i)).to_string();\n'
            '    if has_implicit_reg_usage(&lea) {\n'
            '        continue;\n'
            '    }\n'
        )
        got = gate.violations(src)
        self.assertEqual(len(got), 1)
        self.assertEqual(got[0][2], "has_implicit_reg_usage")

    def test_let_else_method_is_detected(self):
        src = (
            '    let t = infos[i].trimmed(store.get(i)).to_string();\n'
            '    let Some(rest) = t.strip_prefix("movzbl ") else {\n'
            '        continue;\n'
            '    };\n'
        )
        self.assertEqual(len(gate.violations(src)), 1)

    def test_negated_method_is_detected(self):
        src = (
            '    let t = infos[i].trimmed(store.get(i)).to_string();\n'
            '    if !t.starts_with("movl ") {\n'
            '        continue;\n'
            '    }\n'
        )
        got = gate.violations(src)
        self.assertEqual(len(got), 1)
        self.assertEqual(got[0][2], "t.starts_with")

    def test_closure_copy_is_detected(self):
        # The window matchers hid the copy behind a closure name: every call
        # copies, so five Strings were built before one selective matcher.
        src = (
            '    let text = |k: usize| infos[k].trimmed(store.get(k)).to_string();\n'
            '    let Some(m) = match_highbit_sub(&[text(0), text(1), text(2)]) else {\n'
            '        continue;\n'
            '    };\n'
        )
        got = gate.violations(src)
        self.assertEqual(len(got), 1)
        self.assertEqual(got[0][1], "text")

    def test_mut_binding_is_detected(self):
        src = (
            '    let mut lj = infos[j].trimmed(store.get(j)).to_string();\n'
            '    let Some(x) = parse_const_increment(&lj) else {\n'
            '        continue;\n'
            '    };\n'
        )
        self.assertEqual(len(gate.violations(src)), 1)

    def test_break_and_return_count_as_rejection(self):
        for verb in ("break", "return false"):
            src = (
                '    let t = infos[i].trimmed(store.get(i)).to_string();\n'
                f'    if t.is_empty() {{ {verb}; }}\n'
            )
            self.assertEqual(len(gate.violations(src)), 1, verb)


class CompliantShapesStillAccepted(unittest.TestCase):
    """The gate must not push authors back toward a copy to silence it."""

    def test_borrowed_text_is_accepted(self):
        src = (
            '    let lea = infos[li].trimmed(store.get(li));\n'
            '    let Some(r) = lea.strip_prefix("leaq ") else { continue; };\n'
        )
        self.assertEqual(gate.violations(src), [])

    def test_probe_then_allocate_is_accepted(self):
        # Filter on the borrow, own it only for a candidate.
        src = (
            '    let probe = infos[i].trimmed(store.get(i));\n'
            '    if parse_memory_load(probe).is_none() { continue; }\n'
            '    let load = infos[i].trimmed(store.get(i)).to_string();\n'
            '    store.replace(i, load);\n'
        )
        self.assertEqual(gate.violations(src), [])

    def test_conditional_copy_is_accepted(self):
        # The i686 staging shape: the copy is made only when an `s` parser can
        # match, so the reject path allocates nothing.
        src = (
            '    let t: Option<String> = (s.starts_with("movl ")\n'
            '        || s.starts_with("xorl "))\n'
            '        .then(|| trimmed(store, &infos[j], j).to_string());\n'
            '    if let Some(x) = parse_xor_self(&s) { continue; }\n'
        )
        self.assertEqual(gate.violations(src), [])

    def test_unconditional_consumption_is_accepted(self):
        src = (
            '    let t = infos[i].trimmed(store.get(i)).to_string();\n'
            '    store.replace(i, t);\n'
        )
        self.assertEqual(gate.violations(src), [])

    def test_copy_stored_in_a_record_is_accepted(self):
        src = (
            '    let t = infos[i].trimmed(store.get(i)).to_string();\n'
            '    parts.push(t);\n'
        )
        self.assertEqual(gate.violations(src), [])

    def test_rejection_word_in_a_comment_does_not_trigger(self):
        src = (
            '    let t = infos[i].trimmed(store.get(i)).to_string();\n'
            '    store.replace(i, t); // continue with the next line\n'
        )
        self.assertEqual(gate.violations(src), [])

    def test_comment_line_between_copy_and_use_is_skipped(self):
        src = (
            '    let t = infos[i].trimmed(store.get(i)).to_string();\n'
            '\n'
            '    // consume unconditionally\n'
            '    parts.push(t);\n'
        )
        self.assertEqual(gate.violations(src), [])


class ScopeBoundary(unittest.TestCase):
    """Pin the deliberate limit: the filter must be the FIRST statement.

    A copy consumed by an intervening binding and rejected afterwards is not
    flagged. That is a known, documented scope choice -- widening it would flag
    legitimate multi-step consumers -- and this test exists so the boundary is a
    decision rather than an accident.
    """

    def test_rejection_in_a_later_statement_is_out_of_scope(self):
        src = (
            '    let t = infos[i].trimmed(store.get(i)).to_string();\n'
            '    let n = t.len();\n'
            '    if n == 0 { continue; }\n'
        )
        self.assertEqual(gate.violations(src), [])

    def test_comments_cannot_hide_the_filter(self):
        # Comment and blank lines between the copy and its filter are skipped
        # without bound: padding a defect with prose must not silence the gate.
        src = (
            '    let t = infos[i].trimmed(store.get(i)).to_string();\n'
            + '    // filler\n' * (gate.MAX_STATEMENT_LINES + 5)
            + '    if t.is_empty() { continue; }\n'
        )
        self.assertEqual(len(gate.violations(src)), 1)

    def test_statement_scanner_is_bounded_on_code(self):
        # The bound applies to statement lines, so a very long first statement
        # that merely consumes the copy is not chased into a later rejection.
        src = (
            '    let t = infos[i].trimmed(store.get(i)).to_string();\n'
            '    let n = foo(\n'
            + '        arg,\n' * (gate.MAX_STATEMENT_LINES + 5)
            + '        t,\n'
            '    );\n'
            '    if n == 0 { continue; }\n'
        )
        self.assertEqual(gate.violations(src), [])


if __name__ == "__main__":
    unittest.main()
