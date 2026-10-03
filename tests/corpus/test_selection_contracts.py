"""Actual shell --list paths, plus shared selection identity/collision policy."""
from __future__ import annotations

import importlib.util
import os
from pathlib import Path
import subprocess
import tempfile
import sys
from unittest import mock
import unittest

REPO=Path(__file__).resolve().parents[2]
SPEC=importlib.util.spec_from_file_location('selection',REPO/'scripts/corpus_selection.py')
s=importlib.util.module_from_spec(SPEC);SPEC.loader.exec_module(s)


class SelectionContracts(unittest.TestCase):
    def listing(self, script, root, every=1):
        p=subprocess.run(['bash',str(script),'--list',str(root)],cwd=REPO,
                         env=dict(os.environ,CORPUS_EVERY=str(every)),
                         capture_output=True,text=True,timeout=10)
        self.assertEqual(p.returncode,0,p.stderr)
        return p.stdout.splitlines()

    def test_real_whitespace_list_excludes_imported_sources(self):
        actual=self.listing(REPO/'tests/regression/check_peephole_whitespace.sh',REPO/'tests',15)
        self.assertFalse(any('/tests/corpus/' in p for p in actual))
        # Current contaminated sample: 55 regression + 6 benchmarks. Do not
        # merely make the sample empty to satisfy exclusion.
        self.assertGreaterEqual(sum('/tests/regression/' in p for p in actual),55)
        self.assertGreaterEqual(sum('/tests/benchmark/' in p for p in actual),6)
        self.assertEqual(actual,[str(p) for p in s.source_paths(REPO/'tests')[::15]])

    def test_whitespace_and_differential_share_same_full_selection(self):
        a=self.listing(REPO/'tests/regression/check_peephole_whitespace.sh','tests')
        b=self.listing(REPO/'scripts/differential_corpus.sh','tests')
        self.assertEqual(a,b)

    def test_equivalent_roots_and_symlinked_repository_alias(self):
        with tempfile.TemporaryDirectory() as td:
            alias=Path(td)/'repo with spaces';alias.symlink_to(REPO,target_is_directory=True)
            scripts=[REPO/'scripts/differential_corpus.sh',alias/'scripts/differential_corpus.sh',
                     alias/'tests/regression/check_peephole_whitespace.sh']
            roots=['tests','./tests',str(REPO/'tests')+'/',str(alias/'tests')]
            expected=self.listing(scripts[0],roots[0])
            for script in scripts:
                for root in roots:
                    self.assertEqual(self.listing(script,root),expected)

    def test_direct_script_symlink_and_curated_subtrees(self):
        with tempfile.TemporaryDirectory() as td:
            alias=Path(td)/'whitespace.sh';alias.symlink_to(REPO/'tests/regression/check_peephole_whitespace.sh')
            self.assertEqual(self.listing(alias,REPO/'tests'),self.listing(REPO/'scripts/differential_corpus.sh',REPO/'tests'))
            for root in [REPO/'tests/corpus',REPO/'tests/corpus/clang-c/Parser']:
                self.assertEqual(self.listing(alias,root),[])

    def test_same_basename_and_symlinked_file_identity(self):
        with tempfile.TemporaryDirectory() as td:
            repo=Path(td);a=repo/'tests/a';b=repo/'tests/b';curated=repo/'tests/corpus'
            for p in (a,b,curated):p.mkdir(parents=True)
            for p in (a/'dup.c',b/'dup.c',curated/'hidden.c'):p.write_text('int x;\n')
            (a/'leak.c').symlink_to(curated/'hidden.c')
            (a/'same.c').symlink_to(b/'dup.c')
            selected=s.source_paths(repo/'tests',repo=repo)
            self.assertEqual(selected,[a/'dup.c',b/'dup.c'])
            self.assertEqual(len({f'source{i}_{p.stem}-O2.s' for i,p in enumerate(selected)}),2)

    def test_bad_sample_and_missing_root_are_nonzero(self):
        for args in [['--every','0'],['--every','-1'],['/nonexistent/source-root']]:
            p=subprocess.run(['python3',str(REPO/'scripts/corpus_selection.py'),*args],capture_output=True)
            self.assertEqual(p.returncode,2)


class OtherConsumerContracts(unittest.TestCase):
    def load(self,name):
        spec=importlib.util.spec_from_file_location('selection_'+name,REPO/'scripts'/name)
        module=importlib.util.module_from_spec(spec);sys.modules[spec.name]=module
        with mock.patch.object(subprocess,'run',side_effect=AssertionError('compiler/tool discovery at import')):
            spec.loader.exec_module(module)
        return module
    def test_dynamic_imports_no_discovery(self):
        for name in ('check_codegen_refactor_identity.py','aarch64_execute_suite.py'):
            self.load(name)
    def test_codegen_same_root_names_and_overlaps_unique(self):
        module=self.load('check_codegen_refactor_identity.py')
        with tempfile.TemporaryDirectory() as td:
            root=Path(td);a=root/'a'/'same';b=root/'b'/'same'
            a.mkdir(parents=True);b.mkdir(parents=True);(a/'dup.c').write_text('int a;');(b/'dup.c').write_text('int b;')
            items=module.source_items([a,b,a]);self.assertEqual(len(items),2)
            self.assertEqual(len({f'source_{n:06d}/{p.name}' for n,p in items}),2)
    def test_codegen_external_file_symlink_no_relative_to_failure(self):
        module=self.load('check_codegen_refactor_identity.py')
        with tempfile.TemporaryDirectory() as td:
            root=Path(td);a=root/'a';a.mkdir();source=root/'other.c';source.write_text('int a;');(a/'alias.c').symlink_to(source)
            self.assertEqual(module.source_items([a]),[(0,source)])
    def test_AArch64_sanitized_path_collisions_disambiguated(self):
        module=self.load('aarch64_execute_suite.py')
        with tempfile.TemporaryDirectory() as td:
            root=Path(td);a=root/'a_b'/'c.c';b=root/'a'/'b_c.c'
            a.parent.mkdir();b.parent.mkdir();a.write_text('int a;');b.write_text('int b;')
            self.assertNotEqual(module.source_key(a),module.source_key(b));self.assertEqual(module.discover(root,True),sorted([a,b]))
            self.assertEqual(module.discover(REPO/'tests/corpus',True),[])
    def test_codegen_broad_tests_rejected_without_compilation(self):
        module=self.load('check_codegen_refactor_identity.py')
        with mock.patch.object(sys,'argv',['identity','--before',sys.executable,'--after',sys.executable,'--corpus',str(REPO/'tests'),'--out','/tmp/not-created-identity']),self.assertRaises(SystemExit) as ctx:
            module.main()
        self.assertEqual(ctx.exception.code,2)


if __name__=='__main__':unittest.main()
