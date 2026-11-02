#!/usr/bin/env python3
"""Fail-closed contracts for attributing saved counters to rebuilt binaries."""
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'scripts'))
import verify_corpus_binaries as replay


class BinaryEquivalenceTests(unittest.TestCase):
    def exercise(self, *, actual=b'executable', expected=b'executable',
                 error=None, complete=True, changed_source=False):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            program = root / 'tests/benchmark/programs/probe.c'
            program.parent.mkdir(parents=True)
            program.write_bytes(b'int main(void) { return 0; }')
            compiler = root / 'compiler'
            compiler.write_bytes(b'compiler')
            matrix = root / 'matrix'
            (matrix / 'probe').mkdir(parents=True)
            (matrix / 'index.json').write_text(json.dumps(dict(
                complete=complete, selected=['probe'], results={'probe': {}}, failures=[])))
            manifest = dict(failures={}, comparable=1, compiler_environment={}, opt='-O2',
                            include='-I/include', tools={'mine': {'sha256': 'old-compiler'}},
                            results={'probe': dict(source_sha256=replay.digest(program),
                                binary_sha256={'mine': hashlib.sha256(expected).hexdigest()})})
            (matrix / 'probe/manifest.json').write_text(json.dumps(manifest))
            if changed_source:
                program.write_bytes(b'changed')
            def compile_program(_compiler, _opt, _source, binary, **_kwargs):
                if error is None:
                    binary.write_bytes(actual)
                return error
            out = root / 'output'
            with mock.patch.object(replay, 'ROOT', root), \
                    mock.patch.object(replay.cg, 'compile_program', side_effect=compile_program) as build:
                args = ['--compiler', str(compiler), '--matrix', str(matrix),
                        '--matrix', str(matrix), '--out', str(out)]
                if not complete or changed_source:
                    with self.assertRaises(ValueError):
                        replay.main(args)
                    self.assertFalse(json.loads((out / 'equivalence.json').read_text())['complete'])
                    self.assertEqual(build.call_count, 0)
                    return
                result = replay.main(args)
                report = json.loads((out / 'equivalence.json').read_text())
                self.assertTrue(report['complete'])
                self.assertEqual(build.call_count, 1)
                self.assertEqual(len(report['checks']), 2)
                self.assertEqual(result, int(error is not None or actual != expected))
                self.assertEqual(bool(report['failures']), bool(result))

    def test_identical_binary_reused_across_matrices(self):
        self.exercise()

    def test_single_byte_difference_rejected(self):
        self.exercise(actual=b'executablE')

    def test_compilation_failure_rejected(self):
        self.exercise(error='compile rc=1')

    def test_incomplete_matrix_rejected(self):
        self.exercise(complete=False)

    def test_changed_source_rejected(self):
        self.exercise(changed_source=True)


if __name__ == '__main__':
    unittest.main()
