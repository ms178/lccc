"""No stale/missing native executable may become benchmark evidence."""
import hashlib
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock
ROOT=Path(os.environ.get('LCCC_TEST_REPO',Path(__file__).resolve().parents[2]))
sys.path.insert(0,str(ROOT))
from tests.benchmark import run_benchmarks as runner

class NativeCompileLifecycle(unittest.TestCase):
    def run_compile(self, mode, stale=False):
        with tempfile.TemporaryDirectory() as td:
            root=Path(td);src=root/'case.c';src.write_text('int main(void){return 0;}\n');binary=root/'program'
            if stale:binary.write_bytes(b'stale');binary.chmod(0o755)
            def invoke(command,**kwargs):
                self.assertFalse(binary.exists(),'compiler must start with no stale output')
                if mode in ('executable','non-executable'):
                    binary.write_bytes(b'fresh image')
                    binary.chmod(0o755 if mode=='executable' else 0o644)
                return {'ok':mode!='failure','returncode':17 if mode=='failure' else 0,'error':'','stdout':'','stderr':'','command':command}
            compiler=runner.CompilerSpec('fake','fixture','fake-cc',())
            case=runner.Benchmark('case','fixture',('case.c',),())
            with mock.patch.object(runner,'invoke',side_effect=invoke),mock.patch.object(runner,'text_size',return_value=1):
                result=runner.compile_one(compiler,case,src,binary,None)
            if mode=='executable':
                self.assertTrue(result['ok'])
                self.assertEqual(result['binary_sha256'],hashlib.sha256(b'fresh image').hexdigest())
            else:
                self.assertFalse(result['ok'])
                self.assertIsNone(result['binary_sha256'])
            return result
    def test_old_executable_is_removed_before_zero_exit_no_output(self):self.run_compile('empty',True)
    def test_missing_executable_is_a_compile_failure(self):self.run_compile('empty')
    def test_non_executable_artifact_is_not_accepted(self):self.run_compile('non-executable')
    def test_failed_compiler_cannot_leave_an_old_executable(self):self.run_compile('failure',True)
    def test_actual_executable_hash_is_recorded(self):self.run_compile('executable',True)

if __name__=='__main__':unittest.main()
