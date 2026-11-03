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
        # A brace list is a list of QUOTED FLAGS: bare words (including
        # `-rf`), metacharacters and absolute paths never reach the compiler
        # argv.  Bare flags are accepted only when the whole payload is bare.
        self.assertNotIn("/", " ".join(dejagnu.extract_option_flags(text)))
        self.assertNotIn(";", " ".join(dejagnu.extract_option_flags(text)))

    def test_separated_param_values_are_preserved_in_both_spellings(self):
        # GCC 17-git's compile leg uses `--param NAME=N` as two argv entries
        # (pr86637-1.c, pr88347.c, pr89235.c).  Dropping the value would turn
        # `--param` into a missing-argument error, i.e. a silent regression
        # for every quoted directive of that shape.
        quoted = ('/* { dg-options "-ftree-slp-vectorize --param '
                  'ggc-min-expand=1 --param ggc-min-heapsize=1024" } */')
        brace = ('/* { dg-options { "-ftree-slp-vectorize" "--param" '
                 '"ggc-min-expand=1" "--param" "ggc-min-heapsize=1024" } } */')
        expected = ("-ftree-slp-vectorize", "--param", "ggc-min-expand=1",
                    "--param", "ggc-min-heapsize=1024")
        self.assertEqual(dejagnu.extract_option_flags(quoted), expected)
        self.assertEqual(dejagnu.extract_option_flags(brace), expected)

    def test_limit_is_a_prefix_bound_not_a_truncation_license(self):
        text = '/* { dg-options { "-O2" } } */' + 'x' * 100 + '/* { dg-options "-O3" } */'
        self.assertEqual(dejagnu.extract_option_flags(text, limit=32), ("-O2",))
        self.assertEqual(dejagnu.extract_option_flags(text), ("-O2", "-O3"))

    def test_negative_limit_is_rejected_not_silently_reinterpreted(self):
        with self.assertRaises(ValueError):
            dejagnu.extract_option_flags('/* { dg-options "-O2" } */', limit=-1)

    def test_window_bound_is_shared_by_search_and_structural_parse(self):
        # An unterminated directive just before the bound must NOT be closed
        # by a `}` that appears in C code after the window.
        text = '/* { dg-options { "-O2" } */' + ('x' * 100) + '} /* { dg-options "-O3" } */'
        self.assertEqual(dejagnu.extract_option_flags(text, limit=28), ())
        self.assertEqual(dejagnu.extract_option_flags(text), ("-O2", "-O3"))

    def test_escaped_quote_is_conservative(self):
        # A Tcl string with an escaped quote is not guessed at: no partial
        # flag list is better than a silently different compiler invocation.
        self.assertEqual(
            dejagnu.extract_option_flags('/* { dg-options { "-Wl,--defsym=\\"x\\"" } } */'),
            (),
        )

    def test_real_quoted_corpus_directives_are_unchanged(self):
        # The most frequent unconditional quoted directives in GCC 17-git's
        # C torture corpus (c94b688a).  A grammar widening must not change
        # any of these, including the target-qualified ones.
        for directive, expected in (
            ('/* { dg-options "-std=gnu89" } */', ("-std=gnu89",)),
            ('/* { dg-additional-options "-fpermissive" } */', ("-fpermissive",)),
            ('/* { dg-options "-std=gnu17 -fpermissive" } */',
             ("-std=gnu17", "-fpermissive")),
            ('/* { dg-options "-fwrapv" } */', ("-fwrapv",)),
            ('/* { dg-options "-fno-inline" } */', ("-fno-inline",)),
            ('/* { dg-options "-O2" { target { ! __OPTIMIZE__ } } } */', ()),
        ):
            with self.subTest(directive=directive):
                self.assertEqual(dejagnu.extract_option_flags(directive), expected)

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
