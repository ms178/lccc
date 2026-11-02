"""Adversarial PATH control for both actual C++ EH fixture functions.

Mock runtime/ELF observations here; the strict linker suite executes the real
ones. Driver selection itself is exercised through real subprocess/PATH lookup.
"""
import importlib.util
import os
from pathlib import Path
import subprocess
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]


class CxxSelection(unittest.TestCase):
    def test_selected_driver_beats_incompatible_path_driver(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            selected, bad, log = root / 'selected-cxx', root / 'g++', root / 'calls'
            selected.write_text('#!/bin/sh\nprintf "selected\\n" >> "$SELECTION_LOG"\nexit 0\n')
            bad.write_text('#!/bin/sh\nexit 99\n')
            selected.chmod(0o755)
            bad.chmod(0o755)
            env = {'LINKTEST_CXX': str(selected), 'SELECTION_LOG': str(log),
                   'PATH': str(root) + os.pathsep + os.environ['PATH']}
            with patch.dict(os.environ, env):
                spec = importlib.util.spec_from_file_location('linker_fixture_selection', ROOT / 'tests/linker/run_linker_tests.py')
                module = importlib.util.module_from_spec(spec)
                spec.loader.exec_module(module)
                real_sh = module.sh

                def shell(argv, **kwargs):
                    if argv[0] == 'candidate-linker':
                        return subprocess.CompletedProcess(argv, 0, b'', b'')
                    if argv[0] == 'readelf':
                        return subprocess.CompletedProcess(argv, 0, b'R_X86_64_64 __cxxabiv1', b'')
                    return real_sh(argv, **kwargs)

                with patch.object(module, 'sh', shell):
                    for name, text in (('_cxx_eh_local_type_test', 'caught=3 sum=84'),
                                       ('_cxx_eh_test', 'caught: boom-3')):
                        with self.subTest(fixture=name), patch.object(module, 'run_bin', return_value=(0, text)):
                            result = getattr(module, name)(SimpleNamespace(lccc='candidate-linker'), [])
                            self.assertEqual(result.status, 'PASS', repr(result))
                self.assertEqual(log.read_text().splitlines(), ['selected'] * 4)
