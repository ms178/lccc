#!/usr/bin/env python3
"""Compiler-free fail-closed contracts for strict metadata and RUN identity."""
import copy
import hashlib
import importlib.util
from pathlib import Path
import sys
import unittest

REPO=Path(__file__).resolve().parents[2];sys.path.insert(0,str(REPO))
from tools.corpus import diagnostics,directives,schema
spec=importlib.util.spec_from_file_location('contract_miner',REPO/'scripts/edg_corpus_mine.py')
miner=importlib.util.module_from_spec(spec);spec.loader.exec_module(miner)


def parse(text,prefix=0):return directives.parse_text(text,prefix_lines=prefix)
def inv(text):
    p=parse(text);return p,directives.build_invocations('synthetic.sft.c',p,miner.translate_options)


class JsonContract(unittest.TestCase):
    def test_nonfinite_tokens_rejected(self):
        for token in ('Infinity','-Infinity','NaN'):
            with self.subTest(token=token),self.assertRaises(ValueError):schema.loads('{"n":'+token+'}')
    def test_duplicate_keys_rejected(self):
        for text in ('{"a":1,"a":2}','{"a":{"b":0,"b":1}}'):
            with self.assertRaises(ValueError):schema.loads(text)
    def test_emitter_nonfinite_rejected(self):
        for value in (float('inf'),float('-inf'),float('nan')):
            with self.assertRaises(ValueError):schema.dumps({'n':value})
    def test_nullable_unbounded_and_compact(self):
        self.assertEqual(schema.dumps({'max':schema.count('2+')[1],'min':2}),'{"max":null,"min":2}\n')
    def test_count_domain(self):
        for value in ('3-2','-1','2.0','1++'):
            with self.assertRaises(ValueError):schema.count(value)
    def test_unversioned_index_not_guessed(self):
        with self.assertRaises(ValueError):schema.validate(dict(schema='legacy'))
    def test_strict_full_shipped_schema(self):
        doc=miner.verify_index(REPO/'tests/corpus/clang-c/corpus-index.json')
        self.assertEqual((doc['count'],doc['copied'],doc['skipped']),(1529,1434,95))
    def test_copied_transform_is_declared(self):
        doc=schema.loads((REPO/'tests/corpus/clang-c/corpus-index.json').read_text())
        lossy=[r for r in doc['files'] if r['file'] and r['transform']['decoding']!='utf8']
        self.assertEqual([r['id'] for r in lossy],['C/C23/n2927.sft.c'])
        self.assertEqual(lossy[0]['source_sha256'],'bec9ccd04a910598f582b0ee6274ed63c29c20259800a0bba2118a89d281a34c')
        self.assertTrue(all('lossy upstream source decoding' in p['unsupported'] for p in lossy[0]['invocations']))


class DirectiveContract(unittest.TestCase):
    def test_masks_preserve_offsets_and_strings(self):
        text='char *s="// expected-error {{echo}}"; /* expected-warning {{real}} */\n'
        comments,code=directives.comments_and_code(text)
        self.assertEqual((len(comments),len(code)),(len(text),len(text)))
        self.assertEqual([e['msg'] for e in parse(text)['expected']],['real'])
    def test_header_stops_at_first_code(self):
        self.assertEqual(parse('//type: fp\nint x;\n//type: fn\n')['edg']['type'],'fp')
    def test_attribution_physical_line_mapping(self):
        exp=parse('int x; // expected-error {{bad}}\n',3)['expected'][0]
        self.assertEqual(exp['location']['line'],4)
    def test_absolute_relative_any_line(self):
        exp=parse('int x;\n// expected-error@1 {{one}} expected-note@-1 {{two}} expected-warning@* {{three}}\n',3)['expected']
        self.assertEqual([e['location']['line'] for e in exp],[4,4,None])
    def test_literal_line_control(self):
        e=parse('#line 80 "virtual.h"\nint x; // expected-error {{bad}}\n',3)['expected'][0]
        self.assertEqual(e['location'],dict(file='virtual.h',line=80,any_file=False));self.assertFalse(e['unsupported'])
    def test_empty_presumed_filename_and_zero_line(self):
        e=parse('#line 0 ""\n// expected-warning {{bad}}\n')['expected'][0]
        schema.expectation(e);self.assertEqual((e['location']['file'],e['location']['line']),('',0))
        self.assertEqual(diagnostics.parse(':0:1: warning: bad')[0]['file'],'')
    def test_macro_line_control_not_certified(self):
        self.assertTrue(parse('#define N 2\n#line N\n// expected-error {{bad}}\n')['expected'][0]['unsupported'])
    def test_line_marker_stack_not_certified(self):
        self.assertTrue(parse('# 4 "x.h" 1\n// expected-error {{bad}}\n')['expected'][0]['unsupported'])
    def test_labels_are_unsupported(self):
        self.assertTrue(parse('// expected-error@#here {{bad}}\n')['expected'][0]['unsupported'])
    def test_conditional_declarations_are_unsupported(self):
        e=parse('#if 0\n// expected-error {{hidden}}\n#endif\n')['expected'][0]
        self.assertIn('conditional diagnostic requires frontend preprocessing',e['unsupported'])
    def test_line_splices_are_unsupported(self):
        self.assertTrue(parse('// expected-error {{x}} \\\nnext\n')['expected'][0]['unsupported'])
    def test_nested_regex_only_re(self):
        e=parse('// expected-warning-re {{literal ({{.*}})}}\n')['expected'][0]
        self.assertEqual(e['msg'],'literal ({{.*}})');self.assertTrue(e['regex_form'])
    def test_bad_regex_is_unsupported(self):
        e=parse('// expected-warning-re {{{{(a+)+}}}}\n')['expected'][0]
        self.assertTrue(e['unsupported'])


class InvocationContract(unittest.TestCase):
    def test_RUN_cardinality_not_EDG_sets(self):
        p,plans=inv('//options: --c99:--c11:--c23\n// RUN: %clang_cc1 -verify=old -std=c99 %s\n// RUN: %clang_cc1 -verify=new -std=c23 %s\n// old-no-diagnostics\n// new-no-diagnostics\nint x;\n')
        self.assertEqual(len(plans),2);self.assertEqual([p['prefixes'] for p in plans],[['old'],['new']])
        self.assertEqual([p['flags'] for p in plans],[['-std=c99'],['-std=c23']])
        self.assertFalse(any(p['unsupported'] for p in plans))
    def test_shared_EDG_flags_not_broadcast_to_Clang(self):
        _,plans=inv('//options_all: -DEDG_ONLY\n// RUN: %clang_cc1 -DEXACT -verify %s\n// expected-no-diagnostics\n')
        self.assertEqual(plans[0]['flags'],['-DEXACT'])
    def test_active_prefixIDs_only(self):
        p,plans=inv('// RUN: %clang_cc1 -verify=c23 %s\n// expected-error {{inactive}}\n// c23-warning {{active}}\n')
        self.assertEqual(plans[0]['expected_ids'],[p['expected'][1]['id']]);self.assertEqual(plans[0]['expect'],'accept')
    def test_missing_active_prefix_is_unsupported(self):
        _,plans=inv('// RUN: %clang_cc1 -verify=c23 %s\n// expected-error {{inactive}}\n')
        self.assertTrue(plans[0]['unsupported'])
    def test_NO_diagnostics_conflict(self):
        _,plans=inv('// RUN: %clang_cc1 -verify %s\n// expected-no-diagnostics\n// expected-error {{bad}}\n')
        self.assertTrue(plans[0]['unsupported'])
    def test_XFAIL_is_separate_standard_policy(self):
        _,plans=inv('// RUN: %clang_cc1 -verify %s\n// XFAIL: *\n// expected-error {{bad}}\n')
        self.assertEqual((plans[0]['expect'],plans[0]['xfail']),('reject',True))
    def test_target_flags_and_capability(self):
        _,plans=inv('// RUN: %clang_cc1 -triple aarch64-linux-gnu -fsyntax-only %s\n')
        self.assertEqual(plans[0]['target'],'aarch64-linux-gnu');self.assertIn('clang-driver-target',plans[0]['capabilities'])
    def test_external_inputs_need_explicit_resolution(self):
        _,plans=inv('// RUN: %clang_cc1 -I %S -include %S/input.h %s\n')
        self.assertTrue(plans[0]['unsupported']);self.assertIn('external-inputs',plans[0]['capabilities'])
    def test_FILECHECK_final_phase(self):
        _,plans=inv('// RUN: %clang_cc1 -fsyntax-only %s | FileCheck %s\n')
        self.assertEqual(plans[0]['phase'],'filecheck');self.assertTrue(plans[0]['unsupported'])
    def test_not_verify_does_not_become_reject(self):
        _,plans=inv('// RUN: not %clang_cc1 -verify %s\n// expected-error {{bad}}\n')
        self.assertIn('not with -verify tests verifier failure, not a C rejection',plans[0]['unsupported'])
    def test_multi_line_RUN(self):
        _,plans=inv('// RUN: %clang_cc1 -verify \\\n// RUN: -std=c99 %s\n// expected-no-diagnostics\n')
        self.assertEqual(len(plans),1);self.assertIn('-std=c99',plans[0]['flags'])
    def test_EDG_without_RUN_stays_inventory(self):
        _,plans=inv('//options: --c99:--c11\n//cases: 9000\nint x;\n')
        self.assertEqual(len(plans),1);self.assertEqual(plans[0]['model'],'edg-options');self.assertTrue(plans[0]['unsupported'])
    def test_ignore_unexpected_explicit(self):
        _,plans=inv('// RUN: %clang_cc1 -verify -verify-ignore-unexpected=note %s\n// expected-warning {{x}}\n')
        self.assertEqual(plans[0]['ignore_unexpected'],['note'])

    def test_malformed_ignore_flag_not_dropped(self):
        _,plans=inv('// RUN: %clang_cc1 -verify-ignore-unexpectedly -verify %s\n// expected-no-diagnostics\n')
        self.assertTrue(plans[0]['unsupported'])
    def test_invalid_prefix_and_ignore_kind_are_explicit(self):
        for flags in ('-verify=','-verify-ignore-unexpected=banana -verify'):
            _,plans=inv('// RUN: %clang_cc1 '+flags+' %s\n// expected-no-diagnostics\n')
            self.assertTrue(plans[0]['unsupported'])
    def test_repeated_verify_not_last_prefix_broadcast(self):
        _,plans=inv('// RUN: %clang_cc1 -verify=one -verify=two %s\n// one-no-diagnostics\n// two-no-diagnostics\n')
        self.assertEqual(plans[0]['prefixes'],['one','two']);self.assertTrue(plans[0]['unsupported'])
    def test_malformed_no_diagnostics_unsupported(self):
        for marker in ('expected-no-diagnostics-re','expected-no-diagnostics 2','expected-no-diagnostics@1'):
            self.assertTrue(parse('// '+marker+'\n')['expected'][0]['unsupported'])

    def test_optional_error_not_forced_into_accept(self):
        _,plans=inv('// RUN: %clang_cc1 -verify %s\n// expected-error 0-1 {{optional}}\n')
        self.assertIn('optional-error verifier outcome needs native verifier adjudication',plans[0]['unsupported'])
    def test_other_languages_not_mislabeled_C(self):
        _,plans=inv('// RUN: %clang_cc1 -x objective-c %s\n')
        self.assertEqual(plans[0]['language'],'unsupported');self.assertTrue(plans[0]['unsupported'])

    def test_missing_auxiliary_input_cannot_certify_negative_test(self):
        _,plans=inv('// RUN: not %clang_cc1 -std=c99 -fsyntax-only %s\n#include "Inputs/aux.h"\n')
        self.assertIn('source include graph not resolved/pinned',plans[0]['unsupported'])


class PolicyContract(unittest.TestCase):
    def test_XFAIL_rejection_not_inverted(self):
        self.assertEqual(schema.verdict('REJECT','reject',True,True),'XPASS')
        self.assertEqual(schema.verdict('ACCEPT','reject',True,True),'XFAIL')
    def test_terminal_states_never_laundered(self):
        for out in schema.TERMINAL|schema.NONEXECUTED:
            self.assertEqual(schema.verdict(out,'reject',False,True),out)
    def test_standard_accept(self):self.assertEqual(schema.verdict('ACCEPT','accept',True),'PASS')

if __name__=='__main__':unittest.main()
