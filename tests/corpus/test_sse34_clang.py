"""Fail-closed version selection and both-mode invocation, without fake oracles."""
import importlib.util
from pathlib import Path
import subprocess
import unittest
from unittest.mock import patch
ROOT=Path(__file__).resolve().parents[2]
spec=importlib.util.spec_from_file_location('sse34_clang', ROOT/'tests/regression/check_sse34_clang.py')
m=importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)

class ClangGate(unittest.TestCase):
    def test_exact_released_version(self):
        self.assertTrue(m.check_version('clang version 23.1.4 (vendor)'))
        for bad in ('clang version 23.1.3','clang version 23.1.40','clang version 23.1.4git','gcc version 23.1.4'):
            self.assertFalse(m.check_version(bad),bad)

    def test_old_version_stops_before_any_assembly(self):
        with patch.object(m.subprocess,'run',return_value=subprocess.CompletedProcess([],0,'clang version 23.1.3\n','')) as run, patch.object(m.sys,'argv',['gate']):
            with self.assertRaises(SystemExit): m.main()
            self.assertEqual(run.call_count,1)

    def test_both_modes_and_comparison_failure_are_mandatory(self):
        version=subprocess.CompletedProcess([],0,'clang version 23.1.4\n','')
        for mismatch in (False,True):
            with self.subTest(mismatch=mismatch), patch.object(m.subprocess,'run',return_value=version) as run, patch.object(m.asmdiff,'read_elf',return_value=m.asmdiff.ElfImage()), patch.object(m.asmdiff,'compare',return_value=['different bytes'] if mismatch else []) as compare, patch.object(m.sys,'argv',['gate','--clang','chosen-current-clang']), patch('builtins.print'):
                if mismatch:
                    with self.assertRaises(SystemExit): m.main()
                else:
                    self.assertEqual(m.main(),0)
                    commands=[c.args[0] for c in run.call_args_list]
                    self.assertTrue(any('-m32' in c for c in commands))
                    self.assertTrue(any('-m64' in c for c in commands))
                    self.assertEqual(compare.call_count,2)
