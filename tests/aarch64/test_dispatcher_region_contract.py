"""The dispatcher ratchet's region scan must be structural, not textual.

`scripts/gen_aarch64_dispatcher_goldens.py` enumerates the dispatcher's arms by
reading `encoder/mod.rs`.  Each generation of that scan has failed the same way:
a *layout* property of the file stood in for a *syntax* property of the function
-- "up to the next `pub fn`, else EOF", then "up to the first `}` in column 0".
Both are true of rustfmt'd code and both are forgeable by content: a raw string
with a line starting `}` truncates the region to nothing, a `pub fn` inserted
above the dispatcher widens it.  A ratchet that silently enumerates zero arms is
worse than no ratchet, because it prints a satisfied count over the hole.

These tests are the witnesses: each one breaks the naive rule and the hardened
scanner must report the truth (or refuse loudly).
"""
from __future__ import annotations

import importlib.util
import pathlib
import sys
import tempfile
import unittest
from unittest import mock

ROOT = pathlib.Path(__file__).resolve().parents[2]


def load():
    path = ROOT / "scripts" / "gen_aarch64_dispatcher_goldens.py"
    spec = importlib.util.spec_from_file_location("gen_aarch64_dispatcher_goldens", path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


gen = load()


SOURCE = '''
pub fn encode_instruction(mnemonic: &str, ops: &[Operand]) -> Result<u32> {
    match mnemonic {
        "adc" => encode_data_proc(m, ops),
        "adcs" | "sbc"
        | "sbcs" => encode_flag_setting(m, ops),
        // "commented-out" => never_an_arm(),
        r#"a raw string with a
} at column zero and a fake arm head:
        "not-an-arm" => nothing(),
"#,
        "crc32b" => encode_crc(m, ops),
        _ => return Err(BadMnemonic),
    }
}

fn helper(&self) {
    match self.kind {
        "inside-a-different-fn" => not_counted(),
        _ => {}
    }
}
'''


class RegionContract(unittest.TestCase):
    def arms_from(self, source: str) -> list[str]:
        with tempfile.TemporaryDirectory() as td:
            path = pathlib.Path(td) / "mod.rs"
            path.write_text(source)
            with mock.patch.object(gen, "DISPATCHER", path):
                return gen.dispatcher_arms()

    def test_real_dispatcher_still_enumerates_the_baked_count(self):
        """Hardening must not silently re-cut the region on the real file."""
        header = next(int(line.split(":", 1)[1].split()[0])
                      for line in (ROOT / "tests" / "aarch64"
                                   / "dispatcher-goldens.tsv").read_text().splitlines()
                      if line.startswith("# arms:"))
        self.assertEqual(len(gen.dispatcher_arms()), header)

    def test_raw_string_cannot_end_the_region(self):
        # The line `} at column zero...` inside the raw string would have closed
        # the body under the column-0 rule, leaving zero arms and a green run.
        self.assertEqual(self.arms_from(SOURCE),
                         ["adc", "adcs", "crc32b", "sbc", "sbcs"])

    def test_commented_out_arm_is_not_an_arm(self):
        self.assertNotIn("commented-out", self.arms_from(SOURCE))

    def test_arm_head_inside_a_string_is_not_an_arm(self):
        self.assertNotIn("not-an-arm", self.arms_from(SOURCE))

    def test_arms_outside_the_dispatcher_are_not_counted(self):
        self.assertNotIn("inside-a-different-fn", self.arms_from(SOURCE))

    def test_multi_pattern_heads_are_fully_enumerated(self):
        """The 423-of-526 bug: continuation `|` keys must be counted."""
        self.assertIn("sbcs", self.arms_from(SOURCE))

    def test_visibility_and_modifier_forms_are_recognised(self):
        for head in ("pub fn", "pub(crate) fn", "pub(super) unsafe extern \"C\" fn",
                     "async fn", "fn"):
            with self.subTest(head=head):
                source = SOURCE.replace("pub fn encode_instruction(",
                                        f"{head} encode_instruction(")
                self.assertIn("adc", self.arms_from(source))

    def test_empty_region_is_a_loud_failure_not_a_green_nothing(self):
        without_arms = ('pub fn encode_instruction(m: &str) {\n'
                        '    let _ = "no arm heads here";\n}\n')
        with self.assertRaises(SystemExit) as ctx:
            self.arms_from(without_arms)
        self.assertIn("no arms", str(ctx.exception))

    def test_missing_definition_is_a_loud_failure(self):
        with self.assertRaises(SystemExit):
            self.arms_from('pub fn something_else() { "adc" => 1, }\n')

    def test_views_preserve_offsets_and_lines(self):
        code, text, spans = gen.lexical_views(SOURCE)
        for view in (code, text):
            self.assertEqual(len(view), len(SOURCE))
            self.assertEqual(view.count("\n"), SOURCE.count("\n"))
        self.assertNotIn("commented-out", code)
        # Both views blank comments -- that is what stops a commented-out arm
        # head from being enumerated.  Keys are then read from the *original*
        # line, but only once the blanked head test matched, so a comment can
        # never inject a name.
        self.assertNotIn("commented-out", text)
        self.assertNotIn("a raw string with a", code, "literal content is not code")
        self.assertTrue(spans, "the raw literal must be recorded as a span")

    def test_a_literal_prefix_must_be_a_prefix_not_an_inner_letter(self):
        """`"xzr" } else { "wzr" }`: the `r` inside a literal is not `r"`.

        The first version of this masker treated that `r` as a raw-string
        prefix, so masking resumed at the *closing* quote and blanked the real
        code `} else {` between two literals -- which unbalanced the brace depth
        and cost the ratchet 403 of its 526 arms.  A regression test because it
        was found by the file, not invented: the dispatcher's own `mem_rm`
        contains exactly that `if r.is_64 { "xzr" } else { "wzr" }`.
        """
        snippet = 'if r.is_64 { "xzr" } else { "wzr" }\n'
        code, _text, spans = gen.lexical_views(snippet)
        self.assertEqual(code, 'if r.is_64 { "   " } else { "   " }\n')
        self.assertEqual(code.count("{"), code.count("}"), "braces stay balanced")
        self.assertEqual(spans, [(14, 17), (29, 32)])
        self.assertEqual(list(gen.lexical_views('let x = 0b11010;\n')[2]), [])

    def test_char_and_raw_literal_braces_do_not_move_depth(self):
        tricky = ('pub fn encode_instruction(m: &str) {\n'
                  '    let c = \'}\';\n'
                  '    let raw = r#"}{"#;\n'
                  '    let nested = format!("{} {{", m);\n'
                  '        "adc" => 1,\n'
                  '}\n'
                  '}\n')
        self.assertEqual(self.arms_from(tricky), ["adc"])


if __name__ == "__main__":
    unittest.main()
