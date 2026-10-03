"""Specification-led adversarial regressions for PR 723 F01/F02/F06.

These assert corrected behavior, not the old implementation's false passes.
No compiler is needed: diagnostic messages are immutable captured-style data.
"""
from __future__ import annotations

import sys
from pathlib import Path
import unittest

REPO = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO))
from tools.corpus import diagnostics as d


def exp(line=10, kind='warning', msg='unused', **kwargs):
    return dict(id='d0', prefix='expected', kind=kind, message=msg,
                regex_form=False, count_min=1, count_max=1,
                location=dict(file='$SOURCE', line=line, any_file=False),
                physical_line=10, unsupported_reason=None, **kwargs)


class DiagnosticContracts(unittest.TestCase):
    def check(self, expectations, stderr, **kw):
        return d.verify(expectations, d.parse(stderr), source=Path('/src/x.c'), **kw)

    def test_D1_severity_file_line_required(self):
        for stderr in ['other.c:999:1: error: unused', '/src/x.c:999:1: warning: unused',
                       'other.c:10:1: warning: unused', '/src/x.c:10:1: error: unused']:
            with self.subTest(stderr=stderr):
                self.assertFalse(self.check([exp()], stderr)['ok'])

    def test_D2_echo_does_not_match(self):
        self.assertFalse(self.check([exp()], '/src/x.c:10:1: warning: other\n10 | // expected-warning {{unused}}\n   | ^')['ok'])

    def test_D3_no_reuse_of_diagnostic(self):
        a, b = exp(), exp(); b['id'] = 'd1'
        self.assertFalse(self.check([a, b], '/src/x.c:10:1: warning: unused')['ok'])

    def test_D4_unexpected_warning_fails(self):
        self.assertFalse(self.check([exp()], '/src/x.c:10:1: warning: unused\n/src/x.c:20:1: warning: extra')['ok'])

    def test_D5_counts_diagnostics_not_substrings(self):
        self.assertTrue(self.check([exp()], '/src/x.c:10:1: warning: unused unused')['ok'])

    def test_D6_literal_is_not_regex(self):
        e = exp(msg='{{[0-9]+}}')
        self.assertFalse(self.check([e], '/src/x.c:10:1: warning: 123')['ok'])
        self.assertTrue(self.check([e], '/src/x.c:10:1: warning: {{[0-9]+}}')['ok'])
        e['regex_form'] = True
        self.assertTrue(self.check([e], '/src/x.c:10:1: warning: 123')['ok'])

    def test_one_message_per_location(self):
        a, b = exp(10), exp(20); b['id'] = 'd1'
        r = self.check([a,b], '/src/x.c:10:1: warning: unused\n/src/x.c:20:1: warning: unused')
        self.assertTrue(r['ok']); self.assertEqual(len(r['matches']), 2)

    def test_ambiguous_matching_finds_feasible_assignment(self):
        a, b = exp(msg='unused'), exp(msg='unused variable'); b['id'] = 'd1'
        r = self.check([a,b], '/src/x.c:10:1: warning: unused variable\n/src/x.c:10:1: warning: unused function')
        self.assertTrue(r['ok'])

    def test_empty_literal_and_count_range(self):
        e = exp(msg=''); e['count_min']=0; e['count_max']=2
        self.assertTrue(self.check([e], '/src/x.c:10:1: warning: anything')['ok'])
        self.assertFalse(self.check([e], '\n'.join(['/src/x.c:10:1: warning: anything']*3))['ok'])

    def test_unbounded_count_is_nullable(self):
        e = exp(); e['count_max'] = None
        self.assertTrue(self.check([e], '\n'.join(['/src/x.c:10:1: warning: unused']*4))['ok'])

    def test_no_diagnostics_is_exclusive(self):
        e = exp(kind='no-diagnostics', msg='')
        self.assertTrue(self.check([e], '')['ok'])
        self.assertFalse(self.check([e], '/src/x.c:10:1: warning: extra')['ok'])
        self.assertFalse(self.check([e, exp()], '')['ok'])

    def test_ignore_note_policy_explicit(self):
        text='/src/x.c:10:1: warning: unused\n/src/x.c:11:1: note: context'
        self.assertFalse(self.check([exp()], text)['ok'])
        self.assertTrue(self.check([exp()], text, ignore_unexpected=['note'])['ok'])

    def test_any_line_is_still_same_file(self):
        e = exp(); e['location']['line'] = None
        self.assertTrue(self.check([e], '/src/x.c:555:1: warning: unused')['ok'])
        self.assertFalse(self.check([e], 'other.c:555:1: warning: unused')['ok'])
        e['location']['any_file'] = True
        self.assertTrue(self.check([e], 'other.c:555:1: warning: unused')['ok'])

    def test_ansi_stripped_and_fatal_error_normalized(self):
        parsed=d.parse('\x1b[1m/src/x.c:4:2: \x1b[31mfatal error:\x1b[0m missing')
        self.assertEqual(parsed[0]['kind'], 'error')
        self.assertEqual(parsed[0]['message'], 'missing')

    def test_icc_diagnostics_supported(self):
        parsed=d.parse('/src/x.c(10): warning #177: unused')
        self.assertEqual(parsed[0]['line'], 10)
        self.assertTrue(d.verify([exp()], parsed, source=Path('/src/x.c'))['ok'])

    def test_regex_subset_fail_closed(self):
        e = exp(msg=r'{{\d+}}'); e['regex_form'] = True
        self.assertFalse(self.check([e], '/src/x.c:10:1: warning: 123')['ok'])


class RegexDialectContracts(unittest.TestCase):
    def test_unsupported_escape_unicode_possessive(self):
        for expr in (r'{{\t}}',r'{{\x41}}','{{[é]}}','{{a*+}}'):
            with self.assertRaises(ValueError):d.template_regex(expr)


if __name__ == '__main__':
    unittest.main()
