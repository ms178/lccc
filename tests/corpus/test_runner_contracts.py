#!/usr/bin/env python3
"""Compiler-free per-invocation execution/report contracts, not corpus passes.

Every compilation is injected. Real subprocess tests elsewhere run Python,
never a compiler. Legacy substring/broadcast/XFAIL-collapse assertions are
replaced by strict v2 behavioral contracts rather than preserving false PASS.
"""
import copy
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

REPO=Path(__file__).resolve().parents[2];sys.path.insert(0,str(REPO))
spec=importlib.util.spec_from_file_location('runner_contract',REPO/'tests/corpus/run_clang_c_corpus.py')
runner=importlib.util.module_from_spec(spec);spec.loader.exec_module(runner)
from scripts import edg_corpus_mine as miner
from tools.corpus import schema


class SpawnMock:
    def __init__(self,script):self.script=list(script);self.calls=[]
    def __call__(self,cmd,timeout):
        self.calls.append(cmd);result=self.script[len(self.calls)-1]
        if isinstance(result,BaseException):raise result
        return result


class Harness(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup);self.root=Path(self.temp.name)
    def fixture(self,body='// RUN: %clang_cc1 -std=c99 -fsyntax-only -verify %s\nint x; // expected-no-diagnostics\n'):
        output=miner.SPDX.encode()+body.encode();path=self.root/'clang-c/X.c';path.parent.mkdir(exist_ok=True);path.write_bytes(output)
        return miner._record('tests/tests/imported/clang/c/X.sft.c','Sema',body.encode(),output,'clang-c/X.c',
                    commit='1'*40,source_blob=miner.blob(body.encode()),prefix_lines=3)
    def diag(self,rec,kind='error',msg='bad',line=None,file=None):
        if line is None:line=next(e['location']['line'] for e in rec['expected'] if e['kind']!='no-diagnostics')
        return f'{file or self.root/rec["file"]}:{line}:1: {kind}: {msg}\n'.encode()
    def negative(self,extra=''):
        return self.fixture('// RUN: %clang_cc1 -std=c99 -verify %s\n'+extra+'int x; // expected-error {{bad}}\n')
    def execute(self,rec,script,**kwargs):
        spawn=SpawnMock(script)
        result=runner.run_one(rec,self.root,'cc',spawn=spawn,compiler_family='clang',want_arch='x86_64',default_arch='x86_64',**kwargs)
        return result['observations'],spawn


class ClassificationContract(unittest.TestCase):
    def test_crashes_and_panic(self):
        for rc,text in [(-11,''),(139,''),(1,'internal compiler error: boom'),(101,"thread 'main' panicked at x.rs:1"),(101,"thread 'main' (42) panicked at x.rs:1")]:
            self.assertEqual(runner.classify(rc,text),runner.Outcome.CRASH)
    def test_wrapper_and_unknown_codes_are_not_rejection(self):
        for rc,text in [(127,'x.c:1: error: bad'),(126,''),(101,''),(2,'x.c:1: error: bad')]:
            self.assertEqual(runner.classify(rc,text),runner.Outcome.ERROR)
    def test_rejection_needs_located_error_and_rc1(self):
        self.assertEqual(runner.classify(1,'x.c:1:1: error: bad'),runner.Outcome.REJECT)
        self.assertEqual(runner.classify(1,'error: unknown argument'),runner.Outcome.ERROR)
        self.assertEqual(runner.classify(1,'ordinary stderr'),runner.Outcome.ERROR)
    def test_success_with_error_is_inconsistent_not_accept(self):
        self.assertEqual(runner.classify(0,'x.c:1:1: error: bad'),runner.Outcome.ERROR)
    def test_ICE_source_echo_is_not_crash(self):
        self.assertEqual(runner.classify(1,'x.c:1:1: error: #error internal compiler error\n // internal compiler error'),runner.Outcome.REJECT)
    def test_accept_warnings_and_invalid_utf8(self):
        self.assertEqual(runner.classify(0,'x.c:1:1: warning: unused'),runner.Outcome.ACCEPT)
        self.assertEqual(runner.classify(1,'x.c:1:1: error: \ufffd'),runner.Outcome.ERROR)


class VerdictContract(Harness):
    def test_accept_ND_pass(self):
        rows,_=self.execute(self.fixture(),[(0,b'',b'')]);self.assertEqual(rows[0]['status'],'PASS');self.assertTrue(rows[0]['executed'])
    def test_clean_rejection_pass(self):
        rec=self.negative();rows,_=self.execute(rec,[(1,b'',self.diag(rec))]);self.assertEqual(rows[0]['status'],'PASS')
    def test_accept_instead_of_rejection_fails(self):
        rows,_=self.execute(self.negative(),[(0,b'',b'')]);self.assertEqual(rows[0]['status'],'FAIL')
    def test_crash_timeout_infrastructure_hard_under_XFAIL(self):
        rec=self.negative('// XFAIL: *\n')
        for raw,state in [((-11,b'',b''),'CRASH'),((127,b'',b'not found'),'ERROR'),(subprocess.TimeoutExpired(['cc'],1,output=b'partial'),'TIMEOUT')]:
            rows,_=self.execute(rec,[raw]);self.assertEqual(rows[0]['status'],state)
            self.assertNotIn(rows[0]['status'],('XFAIL','PASS'))
    def test_standard_XFAIL_and_XPASS(self):
        rec=self.negative('// XFAIL: *\n')
        rows,_=self.execute(rec,[(0,b'',b'')]);self.assertEqual(rows[0]['status'],'XFAIL')
        rows,_=self.execute(rec,[(1,b'',self.diag(rec))]);self.assertEqual(rows[0]['status'],'XPASS')
    def test_no_reject_before_spawn(self):
        rows,spawn=self.execute(self.negative(),[],run_reject=False);self.assertEqual(rows[0]['status'],'SKIP');self.assertFalse(spawn.calls)
    def test_plan_no_spawn_no_coverage(self):
        rows,spawn=self.execute(self.fixture(),[],plan_only=True);self.assertFalse(spawn.calls);self.assertFalse(rows[0]['executed'])
    def test_source_hash_mismatch_error_no_spawn(self):
        rec=self.fixture();(self.root/rec['file']).write_text('changed')
        rows,spawn=self.execute(rec,[]);self.assertEqual(rows[0]['status'],'ERROR');self.assertFalse(spawn.calls)
    def test_source_change_during_spawn_is_terminal(self):
        rec=self.fixture()
        def mutate(cmd,timeout):
            (self.root/rec['file']).write_text('changed');return 0,b'',b''
        r=runner.run_one(rec,self.root,'cc',compiler_family='clang',want_arch='x86_64',spawn=mutate)['observations'][0]
        self.assertEqual(r['status'],'ERROR');self.assertIn('changed',r['detail'])
    def test_missing_compiler_never_passes(self):
        rows,spawn=self.execute(self.fixture(),[FileNotFoundError('missing')]);self.assertEqual(rows[0]['status'],'ERROR');self.assertFalse(rows[0]['spawned'])
    def test_undecodable_stderr_error_not_abort(self):
        rows,_=self.execute(self.negative(),[(1,b'',b'\xff\xfe')]);self.assertEqual(rows[0]['status'],'ERROR')
    def test_bounded_evidence_output_failure_hard(self):
        import hashlib
        rec=self.negative('// XFAIL: *\n')
        raw=dict(returncode=1,stdout=b'',stderr=self.diag(rec),failure='output-limit',elapsed=.01,
                 stdout_sha256=hashlib.sha256(b'').hexdigest(),stderr_sha256=hashlib.sha256(self.diag(rec)).hexdigest())
        rows,_=self.execute(rec,[raw]);self.assertEqual(rows[0]['status'],'ERROR');self.assertEqual(rows[0]['returncode'],1)
        self.assertIsNotNone(rows[0]['stderr_sha256'])
    def test_diagnostic_resource_failure_not_XFAIL(self):
        rec=self.negative('// XFAIL: *\n');rows,_=self.execute(rec,[(1,b'',self.diag(rec,msg='bad'+'x'*9000))])
        self.assertEqual(rows[0]['status'],'ERROR')

    def test_unexpected_worker_failure_keeps_other_invocations(self):
        rec = self.fixture('// RUN: %clang_cc1 -std=c99 -verify %s\n' * 3 + '// expected-no-diagnostics\n')
        rows, _ = self.execute(rec, [(0, b'', b''), RuntimeError('injected worker fault'), (0, b'', b'')])
        self.assertEqual([r['status'] for r in rows], ['PASS', 'ERROR', 'PASS'])
        self.assertEqual(len({r['id'] for r in rows}), 3)
        self.assertIn('RuntimeError', rows[1]['detail'])

    def test_malformed_process_result_terminal_not_report_abort(self):
        rec = self.fixture()
        for bad in ({'returncode': 0}, (0, None, b'')):
            rows, _ = self.execute(rec, [bad])
            self.assertEqual(rows[0]['status'], 'ERROR')
            self.assertFalse(rows[0]['diagnostics_verified'])

    def test_verifier_exception_retains_raw_executed_evidence(self):
        rec = self.negative('// XFAIL: *\n')
        with mock.patch.object(runner, 'check_diagnostics', side_effect=RuntimeError('verifier fault')):
            rows, _ = self.execute(rec, [(1, b'', self.diag(rec))])
        self.assertEqual(rows[0]['status'], 'ERROR')
        self.assertTrue(rows[0]['executed'])
        self.assertEqual(rows[0]['returncode'], 1)
        self.assertIsNotNone(rows[0]['stderr_sha256'])


class DiagnosticContract(Harness):
    def test_wrong_line(self):
        rec=self.negative();line=rec['expected'][0]['location']['line']
        rows,_=self.execute(rec,[(1,b'',self.diag(rec,line=line+1))]);self.assertEqual(rows[0]['status'],'FAIL')
    def test_wrong_file(self):
        rec=self.negative();rows,_=self.execute(rec,[(1,b'',self.diag(rec,file='/other/X.c'))]);self.assertEqual(rows[0]['status'],'FAIL')
    def test_wrong_severity(self):
        rec=self.negative();rows,_=self.execute(rec,[(0,b'',self.diag(rec,kind='warning'))]);self.assertEqual(rows[0]['status'],'FAIL')
    def test_echo_cannot_certify(self):
        rec=self.negative();text=self.diag(rec,msg='different')+b' int x; // expected-error {{bad}}\n'
        rows,_=self.execute(rec,[(1,b'',text)]);self.assertEqual(rows[0]['status'],'FAIL')
    def test_unexpected_warning_fails_ND(self):
        rec=self.fixture();rows,_=self.execute(rec,[(0,b'',b'X.c:5:1: warning: unexpected\n')]);self.assertEqual(rows[0]['status'],'FAIL')
    def test_repeated_words_not_repeated_diagnostics(self):
        rec=self.fixture('// RUN: %clang_cc1 -std=c99 -verify %s\nint x; // expected-error 2 {{bad}}\n')
        rows,_=self.execute(rec,[(1,b'',self.diag(rec,msg='bad bad'))]);self.assertEqual(rows[0]['status'],'FAIL')
    def test_unbounded_count_counts_headers(self):
        rec=self.fixture('// RUN: %clang_cc1 -std=c99 -verify %s\nint x; // expected-error 1+ {{bad}}\n')
        rows,_=self.execute(rec,[(1,b'',self.diag(rec)*4)]);self.assertEqual(rows[0]['status'],'PASS')
    def test_literal_braces_not_regex(self):
        rec=self.fixture('// RUN: %clang_cc1 -std=c99 -verify %s\nint x; // expected-error {{bad ({{.*}})}}\n')
        rows,_=self.execute(rec,[(1,b'',self.diag(rec,msg='bad (anything)'))]);self.assertEqual(rows[0]['status'],'FAIL')
    def test_regex_splicing_shared(self):
        self.assertEqual(runner.template_to_regex("literal ({{.*}})"),miner.template_to_regex("literal ({{.*}})"))
    def test_active_prefixes_independent_per_RUN(self):
        rec=self.fixture('//options: --c99:--c11:--c23\n// RUN: %clang_cc1 -std=c99 -verify=old %s\n// RUN: %clang_cc1 -std=c23 -verify=new %s\n// old-no-diagnostics\nint x; // new-error {{bad}}\n')
        rows,spawn=self.execute(rec,[(0,b'',b''),(1,b'',self.diag(rec))]);self.assertEqual([r['status'] for r in rows],['PASS','PASS'])
        self.assertEqual([r['prefixes'] for r in rows],[['old'],['new']]);self.assertEqual(len(spawn.calls),2)
    def test_anyline_is_still_same_file(self):
        rec=self.fixture('// RUN: %clang_cc1 -std=c99 -verify %s\nint x; // expected-error@* {{bad}}\n')
        rows,_=self.execute(rec,[(1,b'',self.diag(rec,line=100))]);self.assertEqual(rows[0]['status'],'PASS')
        rows,_=self.execute(rec,[(1,b'',self.diag(rec,line=100,file='other.c'))]);self.assertEqual(rows[0]['status'],'FAIL')
    def test_named_line_control(self):
        rec=self.fixture('// RUN: %clang_cc1 -std=c99 -verify %s\n#line 40 "virtual.h"\nint x; // expected-error {{bad}}\n')
        rows,_=self.execute(rec,[(1,b'',self.diag(rec,file='virtual.h',line=40))]);self.assertEqual(rows[0]['status'],'PASS')
    def test_outcome_only_explicit_no_diagnostic_certificate(self):
        rec=self.negative();rows,_=self.execute(rec,[(1,b'',self.diag(rec,line=100,msg='different'))],mode='outcome-only')
        self.assertEqual(rows[0]['status'],'PASS');self.assertFalse(rows[0]['diagnostics_verified'])

    def test_pathological_regex_is_bounded_not_XFAIL(self):
        import time
        rec=self.fixture('// RUN: %clang_cc1 -std=c99 -verify %s\n// XFAIL: *\nint x; // expected-error-re {{{{a*a*$}}}}\n')
        start=time.monotonic();rows,_=self.execute(rec,[(1,b'',self.diag(rec,msg='a'*6000+'!'))])
        self.assertEqual(rows[0]['status'],'ERROR');self.assertLess(time.monotonic()-start,3)


class PlanningContract(Harness):
    def test_CPP_no_spawn_C_observation_survives(self):
        rec=self.fixture('// RUN: %clang_cc1 -std=c99 -verify=good %s\n// RUN: %clang_cc1 -x c++ -std=c++11 -verify=good %s\n// good-no-diagnostics\n')
        rows,spawn=self.execute(rec,[(0,b'',b'')]);self.assertEqual([r['status'] for r in rows],['PASS','UNSUPPORTED']);self.assertEqual(len(spawn.calls),1)
    def test_FILECHECK_not_counted_as_run(self):
        rec=self.fixture('// RUN: %clang_cc1 -std=c99 -fsyntax-only %s | FileCheck %s\nint x;\n')
        rows,spawn=self.execute(rec,[]);self.assertEqual(rows[0]['status'],'UNSUPPORTED');self.assertFalse(spawn.calls)
    def test_arch_filter_matching_and_capability(self):
        rec=self.fixture('// RUN: %clang_cc1 -triple aarch64-linux-gnu -std=c99 -fsyntax-only %s\n')
        a=runner.plan_invocations(rec,None,compiler_family='clang')[0];self.assertTrue(a['unsupported'])
        b=runner.plan_invocations(rec,'arm64',compiler_family='clang')[0];self.assertFalse(b['unsupported'])
        self.assertIn('--target=aarch64-linux-gnu',b['flags'])
        c=runner.plan_invocations(rec,'aarch64',compiler_family='lccc')[0];self.assertTrue(c['unsupported'])
    def test_EDG_shared_not_in_Clang_command(self):
        rec=self.fixture('//options_all: -DEDG_ONLY\n// RUN: %clang_cc1 -std=c99 -DEXACT -verify %s\n// expected-no-diagnostics\n')
        _,spawn=self.execute(rec,[(0,b'',b'')]);self.assertIn('-DEXACT',spawn.calls[0]);self.assertNotIn('-DEDG_ONLY',spawn.calls[0])
    def test_unsupported_flag_not_silently_dropped(self):
        rec=self.fixture('// RUN: %clang_cc1 -unknown-switch -std=c99 %s\n');rows,spawn=self.execute(rec,[])
        self.assertEqual(rows[0]['status'],'UNSUPPORTED');self.assertFalse(spawn.calls)
    def test_phase_E_preserved(self):
        rec=self.fixture('// RUN: %clang_cc1 -std=c99 -E %s\n');rows,spawn=self.execute(rec,[(0,b'text',b'')])
        self.assertIn('-E',spawn.calls[0]);self.assertNotIn('-fsyntax-only',spawn.calls[0])
    def test_implicit_nonClang_dialect_unsupported(self):
        rec=self.fixture('// RUN: %clang_cc1 -fsyntax-only %s\n')
        self.assertTrue(runner.plan_invocations(rec,compiler_family='lccc')[0]['unsupported'])
    def test_old_C90_prefix_not_silently_certified(self):
        rec=self.fixture('// RUN: %clang_cc1 -std=c89 -verify %s\n// expected-no-diagnostics\n')
        self.assertTrue(runner.plan_invocations(rec,compiler_family='clang')[0]['unsupported'])


class PairingContract(Harness):
    def test_per_ID_not_last_outcome(self):
        rec=self.fixture('// RUN: %clang_cc1 -std=c99 -verify=one %s\n// RUN: %clang_cc1 -std=c99 -verify=two %s\n// one-no-diagnostics\nint x; // two-error {{bad}}\n')
        candidate,_=self.execute(rec,[(0,b'',b''),(1,b'',self.diag(rec))],mode='outcome-only')
        reference,_=self.execute(rec,[(1,b'',self.diag(rec)),(0,b'',b'')],mode='outcome-only')
        pairs=runner.pairwise(candidate,reference);self.assertEqual([p['state'] for p in pairs],['DIVERGENCE','DIVERGENCE'])
    def test_terminal_not_comparable_or_discarded(self):
        rec=self.fixture();candidate,_=self.execute(rec,[(-11,b'',b'')]);reference,_=self.execute(rec,[(0,b'',b'')])
        pair=runner.pairwise(candidate,reference)[0];self.assertEqual(pair['state'],'NONCOMPARABLE');self.assertEqual(pair['candidate'],'CRASH')
    def test_missing_and_unsupported_not_compared(self):
        rec=self.fixture();rows,_=self.execute(rec,[(0,b'',b'')]);self.assertEqual(runner.pairwise(rows,[])[0]['state'],'NONCOMPARABLE')
        other=copy.deepcopy(rows);other[0].update(outcome='UNSUPPORTED',status='UNSUPPORTED',executed=False)
        self.assertFalse(runner.is_divergence(rows[0],other[0]))
    def test_translation_mismatch_not_compared(self):
        rec=self.fixture();a,_=self.execute(rec,[(0,b'',b'')]);b,_=self.execute(rec,[(1,b'',b'X.c:5: error: bad')])
        b[0]['translation_key']='different';self.assertEqual(runner.pairwise(a,b)[0]['state'],'NONCOMPARABLE')
    def test_duplicate_ID_schema_fails(self):
        rec=self.fixture();rows,_=self.execute(rec,[(0,b'',b'')])
        with self.assertRaises(ValueError):schema.validate_results(rows*2)

    def test_arch_filter_not_proof_of_default_target(self):
        rec=self.fixture()
        r=runner.run_one(rec,self.root,'cc',compiler_family='clang',want_arch='aarch64',spawn=SpawnMock([(0,b'',b'')]))['observations'][0]
        self.assertIsNone(r['translation_key'])
    def test_m32_actual_arch_and_target_precedence(self):
        self.assertEqual(runner.effective_arch({'flags':['-m32']},['cc'],'x86_64'),'x86')
        self.assertEqual(runner.effective_arch({'flags':['--target=aarch64-linux-gnu']},['cc'],'x86_64'),'aarch64')


class CliContract(unittest.TestCase):
    def test_invalid_category_usage_no_probe(self):
        with mock.patch.object(runner,'spawn_process',side_effect=AssertionError('unexpected compiler call')):
            self.assertEqual(runner.main(['--category','NoSuchCategory','--cc','true']),2)
    def test_invalid_budgets_usage_error(self):
        for flags in [('--jobs','0'),('--limit','-1'),('--timeout','nan')]:
            self.assertEqual(runner.main(list(flags)),2)
    def test_removed_collapse_XFAIL_flag_rejected(self):
        with self.assertRaises(SystemExit) as ctx:runner.main(['--xfail-as-pass'])
        self.assertEqual(ctx.exception.code,2)
    def test_plan_REPORT_complete_zero_execution(self):
        with tempfile.TemporaryDirectory() as td,mock.patch.object(runner,'spawn_process',side_effect=AssertionError('compiler invoked')):
            report=Path(td)/'report.json'
            rc=runner.main(['--plan','--arch','x86_64','--limit','10','--report',str(report)])
            self.assertEqual(rc,0);doc=schema.loads(report.read_text());self.assertEqual(doc['schema'],schema.REPORT_VERSION)
            m=doc['coverage']['candidate'];self.assertEqual(m['executed_invocations'],0)
            self.assertEqual(sum(m['by_status'].values()),m['selected_planned_invocations'])
            self.assertEqual(len(doc['observations']['candidate']),m['selected_planned_invocations']+m['selected_unplanned_source_skips'])
            self.assertEqual(doc['inventory_only'],dict(gnu_c_records=18188,gnu_execution=0,cxx_execution=0))
    def test_arch_cli_reaches_planner(self):
        with mock.patch.object(runner,'plan_invocations',wraps=runner.plan_invocations) as planner:
            self.assertEqual(runner.main(['--plan','--arch','aarch64','--limit','1']),0)
            self.assertTrue(any(call.args[1]=='aarch64' for call in planner.call_args_list))

if __name__=='__main__':unittest.main()
