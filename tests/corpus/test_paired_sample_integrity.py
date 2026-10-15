#!/usr/bin/env python3
"""Every benchmark warmup/timed output must agree, not merely preflight."""
import importlib.util
import io
import json
import os
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

    def test_interleaved_every_sample_not_last_only(self):
        records=[]
        take=mock.Mock(side_effect=[(0.01,'ok'),(0.02,'bad')])
        self.assertEqual(interleaved.checked_sample(take,'a',None,records,0,'a',0,'warmup'),(0.01,'ok'))
        with self.assertRaisesRegex(ValueError,'inconsistent checksum'):
            interleaved.checked_sample(take,'b','ok',records,0,'b',1,'warmup')
        self.assertEqual(len(records),2)

if __name__=='__main__':unittest.main()
