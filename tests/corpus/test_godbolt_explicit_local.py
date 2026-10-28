"""Explicit local assembly comparators must remain offline and fail closed."""
import contextlib
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts'))
import godbolt  # noqa: E402

FAKE = '''#!/bin/sh
if [ "$1" = "--version" ]; then echo "fixture compiler 16.2.0"; exit 0; fi
for arg in "$@"; do
  if [ "$arg" = "-E" ]; then
    printf '#define __x86_64__ 1\\n#define __SIZEOF_POINTER__ 8\\n'
    exit 0
  fi
done
out=''
while [ "$#" -gt 0 ]; do
  if [ "$1" = "-o" ]; then shift; out="$1"; break; fi
  shift
done
[ -n "$out" ] || exit 1
printf 'f:\\n\\tmov $1, %%eax\\n\\tret\\n.size f, .-f\\n' > "$out"
'''

class OfflineCompare(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root=Path(self.temp.name)
        self.compiler=self.root/'cc'; self.compiler.write_text(FAKE); self.compiler.chmod(0o755)
        self.source=self.root/'input.c'; self.source.write_bytes(b'int f(void){return 1;}\r\n')
        self.report=self.root/'manifest.json'

    def args(self,*extra):
        return godbolt.build_parser().parse_args(['compare',str(self.source),'--local',str(self.compiler),
            '--flags=-O2','--function','f','--json',str(self.report),*extra])

    def compare(self,args):
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            return godbolt.command_compare(args)

    def test_local_only_runs_without_catalog_and_tracks_launcher_and_raw_source(self):
        with patch.object(godbolt,'list_compilers',side_effect=AssertionError('network requested')):
            rc=self.compare(self.args('--oracles','','--local-ref',f'gcc={self.compiler}',
                                      '--local-ref',f'clang={self.compiler}'))
        self.assertEqual(rc,0)
        report=json.loads(self.report.read_text())
        self.assertEqual(report['scope'],'static-assembly-only')
        self.assertNotEqual(report['source_sha256'],report['remote_source_sha256'])
        self.assertEqual([r['id'] for r in report['oracles']],[str(self.compiler),'local:gcc','local:clang'])
        self.assertIn('wrapper targets are not pinned',report['oracles'][1]['identity_scope'])
        self.assertEqual(report['oracles'][1]['instructions'],2)

    def test_failed_remote_catalog_is_recorded_and_cannot_pass(self):
        with patch.object(godbolt,'list_compilers',side_effect=godbolt.GodboltError('offline')):
            rc=self.compare(self.args('--oracles','icc,icx','--local-ref',f'gcc={self.compiler}'))
        self.assertEqual(rc,1)
        report=json.loads(self.report.read_text())
        self.assertEqual(report['oracles'][1]['id'],'local:gcc')
        self.assertTrue(all('catalog unavailable' in x['error'] for x in report['oracles'][2:]))

    def test_empty_oracle_duplicate_and_path_labels_rejected(self):
        for args in [('--oracles',''),('--oracles','gcc,gcc'),
                     ('--oracles','','--local-ref',f'../bad={self.compiler}'),
                     ('--oracles','','--local-ref',f'x={self.compiler}','--local-ref',f'x={self.compiler}')]:
            with self.subTest(args=args):
                self.assertEqual(self.compare(self.args(*args)),1)
                self.assertFalse(self.report.exists())

    def test_missing_reference_fails_and_is_recorded(self):
        self.assertEqual(self.compare(self.args('--oracles','','--local-ref','missing=/no/such/cc')),1)
        report=json.loads(self.report.read_text())
        self.assertIn('not executable',report['oracles'][1]['error'])

    def test_opaque_or_foreign_target_rejected(self):
        for flags in ('-m32','--target=aarch64-linux-gnu','@settings','-B/tmp/toolchain'):
            with self.subTest(flags=flags), self.assertRaises(godbolt.GodboltError):
                godbolt.local_reference_metadata('gcc',str(self.compiler),flags)
        notnative=self.root/'notnative'
        notnative.write_text(FAKE.replace('__SIZEOF_POINTER__ 8','__SIZEOF_POINTER__ 4'))
        notnative.chmod(0o755)
        with self.assertRaises(godbolt.GodboltError):
            godbolt.local_reference_metadata('gcc',str(notnative),'-O2')

    def test_launcher_and_source_mutation_fail_closed(self):
        original = godbolt._compile_local
        calls = []
        def changing_launcher(executable, source, flags):
            lines = original(executable, source, flags)
            calls.append(executable)
            if len(calls) == 2:
                Path(executable).write_text(FAKE + "# changed after compile\n")
            return lines
        with patch.object(godbolt, '_compile_local', side_effect=changing_launcher):
            rc = self.compare(self.args('--oracles', '', '--local-ref', f'gcc={self.compiler}'))
        self.assertEqual(rc, 1)
        report = json.loads(self.report.read_text())
        self.assertIn('changed during compilation', report['oracles'][1]['error'])
        self.compiler.write_text(FAKE)
        calls.clear()
        def changing_source(executable, source, flags):
            lines = original(executable, source, flags)
            calls.append(executable)
            if len(calls) == 2:
                self.source.write_text('int f(void){return 2;}\n')
            return lines
        with patch.object(godbolt, '_compile_local', side_effect=changing_source):
            rc = self.compare(self.args('--oracles', '', '--local-ref', f'gcc={self.compiler}'))
        self.assertEqual(rc, 1)
        self.assertIn('source changed', json.loads(self.report.read_text())['oracles'][-1]['error'])

    def test_ignored_option_and_bad_quote_fail_closed(self):
        for stderr in ("warning: argument unused during compilation: '-march=wrong'",
                       "warning: unrecognized command-line option '-bogus' is tolerated but not implemented"):
            result=subprocess.CompletedProcess([],0,'',stderr)
            with self.subTest(stderr=stderr), patch.object(godbolt.subprocess,'run',return_value=result):
                with self.assertRaises(godbolt.GodboltError):
                    godbolt._compile_local(str(self.compiler),self.source,'-O2')
        with self.assertRaises(godbolt.GodboltError):
            godbolt._compile_local(str(self.compiler),self.source,"-I'broken")

if __name__=='__main__': unittest.main()
