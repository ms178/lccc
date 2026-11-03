"""Contract: DejaGnu option directives must not be silently dropped.

The witness is `gcc.c-torture/execute/930529-1.c` (GCC 17 trunk
c94b688a83fc84015c6fab8ab0ff142051598fc8): it spells `-fwrapv` in a Tcl
brace list.  Before the shared parser existed, both native runners ignored
that flag, GCC 16.2 folded the signed-overflow loop to `ud2`, and the
reference process died with SIGILL -- reported as `reference-fail` on
x86-64 AND i686 even though LCCC passed the case.  These tests pin the
grammar so the same silent reinterpretation cannot come back.
"""
import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))
from tools.corpus import dejagnu  # noqa: E402


def load(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / (name + ".py"))
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


class DirectiveGrammar(unittest.TestCase):
    def test_quoted_form_is_unchanged(self):
        self.assertEqual(
            dejagnu.extract_option_flags('/* { dg-options "-O2 -fwrapv" } */'),
            ("-O2", "-fwrapv"),
        )
        self.assertEqual(
            dejagnu.extract_option_flags('/* { dg-additional-options "-fpermissive" } */'),
            ("-fpermissive",),
        )

    def test_tcl_brace_list_is_honoured(self):
        # The exact directive from 930529-1.c, including the closing `} */`.
        self.assertEqual(
            dejagnu.extract_option_flags('/* { dg-options { "-fwrapv" } } */'),
            ("-fwrapv",),
        )
        self.assertEqual(
            dejagnu.extract_option_flags('/* { dg-options { "-std=gnu89" "-fwrapv" } } */'),
            ("-std=gnu89", "-fwrapv"),
        )

    def test_multiple_directives_accumulate_in_source_order(self):
        text = ('/* { dg-options { "-fwrapv" } } */\n'
                '/* { dg-additional-options "-fno-inline" } */\n')
        self.assertEqual(dejagnu.extract_option_flags(text),
                         ("-fwrapv", "-fno-inline"))

    def test_target_qualified_directives_are_skipped_entirely(self):
        # Both spellings: a target expression is Tcl, not a compiler flag.
        for text in (
            '/* { dg-options "-O2" { target { ! __OPTIMIZE__ } } } */',
            '/* { dg-options { "-O2" } { target arm*-*-* aarch64*-*-* } } */',
            '/* { dg-options -Wa,--no-verify { target nvptx-*-* } } */',
        ):
            with self.subTest(text=text):
                self.assertEqual(dejagnu.extract_option_flags(text), ())

    def test_bare_flag_token(self):
        self.assertEqual(
            dejagnu.extract_option_flags('/* { dg-options -Wa,--no-verify } */'),
            ("-Wa,--no-verify",),
        )

    def test_unterminated_group_contributes_nothing(self):
        self.assertEqual(dejagnu.extract_option_flags('/* { dg-options { "-O2" } */'), ())
        self.assertEqual(dejagnu.extract_option_flags('/* { dg-options "-O2 */'), ())

    def test_no_shell_words_or_stray_tcl_words_become_flags(self):
        text = '/* { dg-options { "-O2" ; rm -rf / } } */'
        self.assertEqual(dejagnu.extract_option_flags(text), ("-O2",))
        # Only the quoted flag survives: no metacharacter, no stray bare word
        # and no absolute path can reach the compiler argv from a mixed list.
        self.assertNotIn("/", " ".join(dejagnu.extract_option_flags(text)))
        self.assertNotIn(";", " ".join(dejagnu.extract_option_flags(text)))

    def test_limit_is_a_prefix_bound_not_a_truncation_license(self):
        text = '/* { dg-options { "-O2" } } */' + 'x' * 100 + '/* { dg-options "-O3" } */'
        self.assertEqual(dejagnu.extract_option_flags(text, limit=32), ("-O2",))
        self.assertEqual(dejagnu.extract_option_flags(text), ("-O2", "-O3"))

    def test_both_runners_use_the_shared_parser(self):
        torture = load("x86_gcc_torture")
        aarch64 = load("aarch64_execute_suite")
        with tempfile.TemporaryDirectory() as td:
            source = Path(td) / "930529-1.c"
            source.write_text('/* { dg-options { "-fwrapv" } } */\nint main(void){return 0;}\n')
            self.assertEqual(torture.native_directive_flags(source), ("-fwrapv",))
            self.assertEqual(aarch64.dejagnu_flags(source), ("-fwrapv",))
            source.write_text('/* { dg-options "-O2" { target arm*-*-* } } */\n')
            self.assertEqual(torture.native_directive_flags(source), ())
            self.assertEqual(aarch64.dejagnu_flags(source), ())


if __name__ == "__main__":
    unittest.main()
