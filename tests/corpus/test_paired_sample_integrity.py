#!/usr/bin/env python3
"""Every benchmark warmup/timed output must agree, not merely preflight."""
import importlib.util
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest
from contextlib import redirect_stdout, redirect_stderr
from unittest import mock

ROOT=Path(__file__).resolve().parents[2]
def load(name):
    spec=importlib.util.spec_from_file_location(name,ROOT/'scripts'/f'{name}.py')
    mod=importlib.util.module_from_spec(spec);spec.loader.exec_module(mod)
    return mod
paired=load('paired_ab')
interleaved=load('ab_interleaved')

class Samples(unittest.TestCase):
    def test_paired_third_run_mismatch_is_not_a_speed_verdict(self):
        with tempfile.TemporaryDirectory() as td:
            root=Path(td)
            for arm in ('a','b'):
                script=root/arm
                counter=str(root/(arm+'.count'))
                script.write_text('#!/usr/bin/env python3\nfrom pathlib import Path\n'
                                  f'p=Path({counter!r})\n'
                                  'n=int(p.read_text())+1 if p.exists() else 1\n'
                                  'p.write_text(str(n))\n'
                                  f'print("{"bad" if arm == "b" else "ok"}" if n == 3 else "ok")\n')
                script.chmod(0o755)
            report=root/'report.json'
            argv=['paired_ab.py','--bin-a',str(root/'a'),'--bin-b',str(root/'b'),
                  '--rounds','1','--warmup','1','--json',str(report)]
            with mock.patch.object(sys,'argv',argv),redirect_stdout(io.StringIO()),redirect_stderr(io.StringIO()):
                self.assertEqual(paired.main(),4)
            data=json.loads(report.read_text())
            self.assertEqual(data['verdict'],'correctness-failure')
            self.assertNotIn('median_ratio_b_over_a',data)
            self.assertEqual(data['samples'][-1]['phase'],'timed')

    def test_paired_zero_elapsed_is_a_failed_sample_not_a_ratio(self):
        with tempfile.TemporaryDirectory() as td:
            root=Path(td)
            for arm in ('a','b'):
                binary=root/arm
                binary.write_text(f'#!/bin/sh\necho {arm}\n')
                binary.chmod(0o755)
            report=root/'report.json'
            argv=['paired_ab.py','--bin-a',str(root/'a'),'--bin-b',str(root/'b'),
                  '--rounds','1','--warmup','0','--json',str(report)]
            with mock.patch.object(sys,'argv',argv),\
                 mock.patch.object(paired,'run_once',return_value=(0,'ok\n')),\
                 mock.patch.object(paired,'time_run',return_value=(0.0,0,'ok\n')),\
                 redirect_stdout(io.StringIO()),redirect_stderr(io.StringIO()):
                self.assertEqual(paired.main(),4)
            data=json.loads(report.read_text())
            self.assertEqual(data['verdict'],'correctness-failure')
            self.assertEqual(data['samples'][0]['ms'],0.0)
            self.assertNotIn('median_ratio_b_over_a',data)

    def test_paired_source_workdir_is_removed_on_return(self):
        with tempfile.TemporaryDirectory() as td:
            root=Path(td)
            source=root/'unused.c'; source.write_text('int main(void){return 0;}\n')
            workdirs=[]
            def fake_build(src,ccc,cflags,env_extra,out):
                workdirs.append(Path(out).parent)
                Path(out).write_text(f'#!/bin/sh\n# {out}\nexit 0\n')
                Path(out).chmod(0o755)
            argv=['paired_ab.py','--src',str(source),'--ccc','unused',
                  '--rounds','1','--warmup','0']
            with mock.patch.object(sys,'argv',argv),\
                 mock.patch.object(paired,'build',side_effect=fake_build),\
                 mock.patch.object(paired,'run_once',return_value=(0,'ok\n')),\
                 mock.patch.object(paired,'time_run',return_value=(1.0,0,'ok\n')),\
                 redirect_stdout(io.StringIO()),redirect_stderr(io.StringIO()):
                self.assertEqual(paired.main(),0)
            self.assertTrue(workdirs)
            self.assertTrue(all(not path.exists() for path in workdirs))

    def test_interleaved_every_sample_not_last_only(self):
        records=[]
        take=mock.Mock(side_effect=[(0.01,'ok'),(0.02,'bad')])
        self.assertEqual(interleaved.checked_sample(take,'a',None,records,0,'a',0,'warmup'),(0.01,'ok'))
        with self.assertRaisesRegex(ValueError,'inconsistent checksum'):
            interleaved.checked_sample(take,'b','ok',records,0,'b',1,'warmup')
        self.assertEqual(len(records),2)

    def test_interleaved_failed_invocation_is_not_dropped(self):
        records=[]
        take=mock.Mock(side_effect=OSError('cannot execute'))
        with self.assertRaisesRegex(OSError,'cannot execute'):
            interleaved.checked_sample(take,'missing',None,records,7,'b',1,'timed')
        self.assertEqual(len(records),1)
        self.assertEqual(records[0]['phase'],'timed')
        self.assertEqual(records[0]['round'],7)
        self.assertIsNone(records[0]['rc'])
        self.assertIn('cannot execute',records[0]['error'])

    def test_interleaved_build_failure_writes_a_complete_receipt(self):
        with tempfile.TemporaryDirectory() as td:
            root=Path(td)
            compiler=root/'fails'
            compiler.write_text('#!/bin/sh\necho compiler-out\necho compiler-err >&2\nexit 7\n')
            compiler.chmod(0o755)
            report=root/'report.json'
            argv=['ab_interleaved.py','--a',str(compiler),'--b',str(compiler),
                  '--kernel','unused','--samples','1','--repo',str(root),
                  '--json',str(report)]
            with mock.patch.object(sys,'argv',argv),redirect_stdout(io.StringIO()),redirect_stderr(io.StringIO()):
                self.assertEqual(interleaved.main(),2)
            data=json.loads(report.read_text())
            self.assertEqual(data['verdict'],'correctness-or-infrastructure-failure')
            self.assertEqual(data['samples'],[])
            self.assertEqual(len(data['builds']),1)
            self.assertEqual(data['builds'][0]['arm'],'a')
            self.assertEqual(data['builds'][0]['returncode'],7)
            self.assertEqual(data['builds'][0]['stdout'],'compiler-out\n')
            self.assertEqual(data['builds'][0]['stderr'],'compiler-err\n')

if __name__=='__main__':unittest.main()
