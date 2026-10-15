#!/usr/bin/env python3
"""Benchmarks must not report a ratio from an incomplete or divergent sample set."""
import importlib.util
import math
from pathlib import Path
import io
import json
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from unittest import mock

spec = importlib.util.spec_from_file_location('bench_kernels', Path(__file__).resolve().parents[2] / 'scripts/bench_kernels.py')
b = importlib.util.module_from_spec(spec)
spec.loader.exec_module(b)

class SampleIntegrity(unittest.TestCase):
    def rows(self):
        return [dict(arm=arm, round=r, order=pos, rc=0, checksum='123',
                     seconds=0.01, stdout_sha256='a'*64)
                for r in range(3) for pos, arm in enumerate(('a','b'))]

    def test_complete_controls(self):
        values, error = b.checked_samples(self.rows(), ['a','b'], 3)
        self.assertIsNone(error)
        self.assertEqual(values, {'a':0.01,'b':0.01})

    def test_every_run_counts_not_just_first_checksum(self):
        for mutation in ('different-third', 'failed-third', 'missing', 'duplicate', 'bad-time', 'same-failure'):
            with self.subTest(mutation=mutation):
                rows = self.rows()
                if mutation == 'different-third': rows[4]['checksum'] = '999'
                if mutation == 'failed-third': rows[4]['rc'] = 9
                if mutation == 'missing': rows.pop()
                if mutation == 'duplicate': rows[4]['round'] = 1
                if mutation == 'bad-time': rows[4]['seconds'] = math.nan
                if mutation == 'same-failure':
                    for row in rows: row['rc'] = 9
                result, error = b.checked_samples(rows, ['a','b'], 3)
                self.assertEqual(result, {})
                self.assertIsNotNone(error)

    def test_equal_normalized_hash_never_waives_regression(self):
        with tempfile.TemporaryDirectory() as td:
            baseline=Path(td)/'baseline.json'
            baseline.write_text(json.dumps({'adler32': {'lccc':0.005,'_hash_lccc':'same'}}))
            argv=['bench_kernels.py','--lccc',str(Path(__file__)),
                  '--gcc','fake-gcc','--clang','', '--reference-cc','fake-gcc',
                  '--kernels','adler32','--reps','1','--cpu','none',
                  '--baseline',str(baseline),'--tolerance','3']
            def fake_sample(binary, inner, pin):
                return {'rc':0,'seconds':0.01,'checksum':'same',
                        'stdout_sha256':'a'*64}
            out=io.StringIO()
            with mock.patch.object(sys,'argv',argv),\
                 mock.patch.object(b,'build_arm',return_value=(True,'','same')),\
                 mock.patch.object(b,'time_once',side_effect=fake_sample),\
                 mock.patch.object(b.subprocess,'run',return_value=mock.Mock(returncode=0)),\
                 redirect_stdout(out):
                self.assertEqual(b.main(),1)
            self.assertIn('REGRESSED adler32',out.getvalue())
            self.assertIn('normalized function hash equal',out.getvalue())
            self.assertNotIn('NOISE',out.getvalue())

    def test_compile_failures_are_saved_as_structured_receipts(self):
        with tempfile.TemporaryDirectory() as td:
            report=Path(td)/'nested'/'report.json'
            argv=['bench_kernels.py','--lccc',str(Path(__file__)),
                  '--gcc','fake-gcc','--clang','','--reference-cc','fake-gcc',
                  '--kernels','adler32','--reps','1','--cpu','none',
                  '--save',str(report)]
            def fail_build(arm,cc,env,cflags,kernel,driver,out,workdir,receipts=None):
                receipts.append({'arm':arm,'phase':'compile','command':[cc,'-c'],
                                 'returncode':9,'stdout':'','stderr':'bad compile'})
                return False,'kernel compile failed: bad compile',''
            with mock.patch.object(sys,'argv',argv),\
                 mock.patch.object(b,'build_arm',side_effect=fail_build),\
                 mock.patch.object(b.subprocess,'run',return_value=mock.Mock(returncode=0,stdout='',stderr='')),\
                 redirect_stdout(io.StringIO()):
                self.assertEqual(b.main(),1)
            data=json.loads(report.read_text())
            self.assertEqual(data['adler32']['_invalid'],
                             'one or more compiler arms failed to build')
            self.assertEqual(len(data['adler32']['_builds']),2)
            self.assertTrue(all(row['returncode']==9
                                for row in data['adler32']['_builds']))

    def test_time_once_rejects_nonzero_or_partial_output(self):
        for rc, stdout in ((9,'0.1 2'), (0,'0.1'), (0,'NaN 2'), (0,'0.0 2'), (0,'0.1 2 extra')):
            with self.subTest(rc=rc, stdout=stdout):
                fake = mock.Mock(returncode=rc, stdout=stdout)
                with mock.patch.object(b.subprocess,'run',return_value=fake):
                    result=b.time_once(Path('/unused'),1,[])
                self.assertIn('error',result)

if __name__ == '__main__': unittest.main()
