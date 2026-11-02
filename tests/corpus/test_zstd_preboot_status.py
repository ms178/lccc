"""Whole shell entrypoint, mocked compilers: every oracle failure must propagate."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
ROOT=Path(os.environ.get('LCCC_TEST_REPO',Path(__file__).resolve().parents[2]))
SCRIPT=Path(os.environ.get('ZSTD_PREBOOT_SCRIPT',ROOT/'scripts/zstd_preboot_oracle.sh'))
FAKE_CC='''#!/usr/bin/env python3
import os,sys
from pathlib import Path
args=sys.argv[1:];out=Path(args[args.index('-o')+1])
if '-c' in args:out.write_bytes(b'fixture object')
else:
 out.write_text('#!/usr/bin/env python3\\nimport os,sys\\nfrom pathlib import Path\\nn=Path(sys.argv[0]).name\\nprint(n)\\nsys.exit(int(os.environ.get("FAIL_"+n.replace("-","_"),"0")))\\n')
 out.chmod(0o755)
'''
class ZstdPrebootStatus(unittest.TestCase):
    def run_gate(self, failure=None, payload='both'):
        with tempfile.TemporaryDirectory(prefix='zstd-status-') as td:
            root=Path(td);tools=root/'tools';tools.mkdir();compiler=tools/'gcc';compiler.write_text(FAKE_CC);compiler.chmod(0o755)
            z=root/'data.zst';plain=root/'data';out=root/'output'
            if payload in ('both','partial'):z.touch()
            if payload=='both':plain.touch()
            env=dict(os.environ,PATH=str(tools)+os.pathsep+os.environ['PATH'],GCC=str(compiler),LCCC=str(compiler),KERNEL_DIR=str(root/'kernel'),OUT=str(out),ZFILE=str(z),PFILE=str(plain))
            if failure:env['FAIL_'+failure.replace('-','_')]='23'
            result=subprocess.run(['bash',str(SCRIPT)],env=env,capture_output=True,text=True,timeout=15)
            expected=1 if failure or payload=='partial' else 0
            self.assertEqual(result.returncode,expected,result.stdout+result.stderr)
            self.assertIn('oracle-gcc',result.stdout)
            self.assertIn('oracle-lccc',result.stdout)
            if payload=='both':
                self.assertIn('oracle-inplace-gcc',result.stdout)
                self.assertIn('oracle-inplace-lccc',result.stdout)
            if failure:self.assertIn('FAIL rc=23',result.stderr)
            if payload=='none':self.assertIn('SKIP in-place',result.stdout)
    def test_all_four_successes(self):self.run_gate()
    def test_separate_gcc_failure(self):self.run_gate('oracle-gcc')
    def test_separate_lccc_failure(self):self.run_gate('oracle-lccc')
    def test_inplace_gcc_failure(self):self.run_gate('oracle-inplace-gcc')
    def test_inplace_lccc_failure(self):self.run_gate('oracle-inplace-lccc')
    def test_absent_optional_payload_is_explicit(self):self.run_gate(payload='none')
    def test_half_a_payload_pair_is_not_silently_skipped(self):self.run_gate(payload='partial')
if __name__=='__main__':unittest.main()
