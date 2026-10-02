#!/usr/bin/env python3
"""Pure/mocked Callgrind orchestration; no compiler/program/Valgrind runs."""
import hashlib
import importlib.util
from pathlib import Path
import os
import sys
import tempfile
import unittest
from unittest import mock

REPO=Path(__file__).resolve().parents[2];sys.path.insert(0,str(REPO))
def load():
    spec=importlib.util.spec_from_file_location('cg_contract',REPO/'scripts/callgrind_ab.py')
    module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module);return module
cg=load()
from tools.corpus import process,schema


def ev(ir=100):return dict(Ir=ir,I1mr=1,D1mr=2,D1mw=3,ILmr=4,DLmr=5,DLmw=6,Bcm=7,Bim=8)
def result(rc=0,stdout=b'ok\n',failure=None):
    r=process.normalize((rc,stdout,b''));r['failure']=failure;return r


class CallgrindContract(unittest.TestCase):
    def test_import_no_discovery(self):
        with mock.patch.object(process,'run',side_effect=AssertionError('eager discovery')):load()
    def test_no_builtin_shadow(self):self.assertFalse(hasattr(cg,'compile'));self.assertTrue(callable(cg.compile_program))
    def test_lazy_gcc_include(self):
        cg.gcc_include.cache_clear()
        with tempfile.TemporaryDirectory() as td,mock.patch.object(cg,'_run',return_value=result(stdout=(td+'\n').encode())) as run:
            self.assertEqual(cg.gcc_include('fake-gcc'),'-I'+td);self.assertEqual(run.call_count,1)
            self.assertEqual(cg.gcc_include('fake-gcc'),'-I'+td);self.assertEqual(run.call_count,1)
    def test_invalid_include_path(self):
        cg.gcc_include.cache_clear()
        with mock.patch.object(cg,'_run',return_value=result(stdout=b'/definitely/not/headers\n')):
            with self.assertRaises(ValueError):cg.gcc_include('fake-include')
    def test_geometry_fixed_and_visible_override(self):
        with mock.patch.dict(os.environ,{'LCCC_CG_D1':'65536,8,64'}):
            model,override=cg.geometry();self.assertEqual(model['I1'],'32768,8,64');self.assertEqual(model['LL'],'33554432,16,64')
            self.assertEqual(override['LCCC_CG_D1'],model['D1'])
    def test_invalid_geometry_rejected(self):
        for value in ('0,8,64','32768,8,63','abc','32769,8,64'):
            with mock.patch.dict(os.environ,{'LCCC_CG_I1':value}),self.assertRaises(ValueError):cg.geometry()
    def test_zero_Ir_both_sides(self):
        for a,b in ((ev(0),ev()),(ev(),ev(0))):
            with self.assertRaises(ValueError):cg.compare_events(a,b)
    def test_missing_negative_bool_counters_rejected(self):
        for events in ({'Ir':1},dict(ev(),D1mw=-1),dict(ev(),Ir=True)):
            with self.assertRaises(ValueError):cg.validate_events(events)
    def test_write_misses_included(self):
        m=cg.compare_events(ev(),ev(90));self.assertEqual(m['d1_misses'],[5,5]);self.assertEqual(m['ll_misses'],[15,15]);self.assertEqual(m['ir_ratio'],.9)
    def test_summary_strict_cardinality(self):
        with tempfile.TemporaryDirectory() as td:
            p=Path(td)/'counts';p.write_text('events: '+ ' '.join(ev())+'\nsummary: '+ ' '.join(map(str,ev().values()))+'\n')
            self.assertEqual(cg.parse_summary(p),ev())
            for text in ('events: Ir Ir\nsummary: 1 2\n','events: Ir\nsummary: 1 2\n','events: Ir\nsummary: x\n','events: Ir\nsummary: 0\n'):
                p.write_text(text)
                with self.assertRaises(ValueError):cg.parse_summary(p)
    def test_correctness_exit_AND_stdout(self):
        self.assertIsNone(cg.same_output(result(),result()))
        for other in (result(rc=-11),result(rc=127),result(stdout=b'different'),result(failure='timeout')):
            self.assertIsNotNone(cg.same_output(result(),other))
    def test_compile_quoted_options_and_no_stale_binary(self):
        with tempfile.TemporaryDirectory() as td:
            out=Path(td)/'program';out.write_text('stale');out.chmod(0o755)
            with mock.patch.object(cg,'_run',return_value=result(rc=1)) as run:
                self.assertIsNotNone(cg.compile_program('fake-cc','-O2 -DNAME="a b"','source.c',out,include='-Ifake'))
                self.assertIn('-DNAME=a b',run.call_args.args[0]);self.assertFalse(out.exists())
    def test_zero_exit_without_binary_is_failure(self):
        with tempfile.TemporaryDirectory() as td,mock.patch.object(cg,'_run',return_value=result()):
            self.assertIsNotNone(cg.compile_program('fake-cc','-O2','source.c',Path(td)/'missing',include='-Ifake'))
    def test_instrumented_output_checked_and_stale_counts_removed(self):
        with tempfile.TemporaryDirectory() as td:
            path=Path(td);(path/'program.cg').write_text('stale')
            with mock.patch.object(cg,'_run',return_value=result(stdout=b'wrong')):
                events,error,info=cg.callgrind(path/'program',path,expected_stdout=b'ok\n')
                self.assertIsNone(events);self.assertIn('stdout',error);self.assertFalse((path/'program.cg').exists())
                self.assertIn('process',info)


class MainContract(unittest.TestCase):
    def mock_measurement(self,outroot,events):
        def compiler(cmd,opt,src,out,**kw):out.write_bytes(b'dummy-not-a-compiler-output');out.chmod(0o755)
        with mock.patch.object(cg,'compile_program',side_effect=compiler),mock.patch.object(cg,'tool_identity',return_value={'version':'mock','sha256':'a'*64}),mock.patch.object(cg,'gcc_include',return_value='-Ifake'),mock.patch.object(cg,'_run',return_value=result()),mock.patch.object(cg,'callgrind',return_value=(events,None,{'mock':True})):
            return cg.main(['mock-mine','mock-ref','-O2','fib','--out-root',str(outroot)])
    def test_success_manifest_equal_paths_geometry_and_hashes(self):
        with tempfile.TemporaryDirectory() as td:
            root=Path(td);self.assertEqual(self.mock_measurement(root,ev()),0)
            doc=schema.loads((root/'manifest.json').read_text());self.assertEqual(doc['comparable'],1)
            self.assertEqual(len(doc['paths']['mine']),len(doc['paths']['ref']));self.assertEqual(Path(doc['paths']['mine']).name,'aa');self.assertEqual(Path(doc['paths']['ref']).name,'bb')
            self.assertTrue(doc['results']['fib']['correctness']['mine']['stdout_sha256'])
            self.assertIn('not PMU',doc['model'])
    def test_zero_Ir_failure_manifest_not_division_or_log_error(self):
        with tempfile.TemporaryDirectory() as td:
            root=Path(td);self.assertEqual(self.mock_measurement(root,ev(0)),2)
            doc=schema.loads((root/'manifest.json').read_text());self.assertEqual(doc['comparable'],0);self.assertIn('fib',doc['failures']);self.assertNotIn('geomean_ir_mine_over_ref',doc)
    def test_partial_failure_returns_failure_not_success(self):
        with tempfile.TemporaryDirectory() as td,mock.patch.object(cg,'_measure',return_value=1) as measure,mock.patch.object(cg,'geometry',return_value=({'I1':cg.CG_I1,'D1':cg.CG_D1,'LL':cg.CG_LL},{})):
            self.assertEqual(cg.main(['mock-a','mock-b','-O2','fib','--out-root',td]),1)
    def test_setup_error_manifest_observable(self):
        with tempfile.TemporaryDirectory() as td,mock.patch.object(cg,'gcc_include',side_effect=ValueError('missing GCC')):
            self.assertEqual(cg.main(['mock-a','mock-b','-O2','fib','--out-root',td]),2)
            self.assertIn('setup',schema.loads((Path(td)/'manifest.json').read_text())['failures'])
    def test_heavy_is_used_opt_in(self):
        with tempfile.TemporaryDirectory() as td,mock.patch.object(cg,'_measure',return_value=2) as measure:
            cg.main(['mock-a','mock-b','-O2','--heavy','--out-root',td]);self.assertTrue(cg.HEAVY<=set(measure.call_args.args[1]))

if __name__=='__main__':unittest.main()
