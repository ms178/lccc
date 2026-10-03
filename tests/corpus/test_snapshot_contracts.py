"""Compiler-free snapshot boundary tests; actual miniature Git repositories."""
from __future__ import annotations

import gzip
import importlib.util
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

REPO = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location('snapshot_policy', REPO / 'scripts/snapshot_policy.py')
policy = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(policy)


class SnapshotPolicyTests(unittest.TestCase):
    def record(self, old, new, status='M'):
        return f':{old} {new} {"a" * 40} {"b" * 40} {status}\0a path\nwith newline\0'.encode()

    def test_modes_default_strict(self):
        self.assertTrue(policy.inspect(self.record('100644', '100755'))[0])

    def test_exec_override_logged(self):
        errors, allowed = policy.inspect(self.record('100644', '100755'), allow_modes=True)
        self.assertFalse(errors)
        self.assertEqual(len(allowed), 1)

    def test_deletion_not_allowed_by_mode_override(self):
        self.assertTrue(policy.inspect(self.record('100644', '000000', 'D'), allow_modes=True)[0])

    def test_symlink_not_allowed_by_mode_override(self):
        self.assertTrue(policy.inspect(self.record('100644', '120000', 'T'), allow_modes=True)[0])

    def test_gitlink_not_allowed_by_mode_override(self):
        self.assertTrue(policy.inspect(self.record('100644', '160000', 'T'), allow_modes=True)[0])

    def test_new_symlink_requires_structural_override(self):
        self.assertTrue(policy.inspect(self.record('000000', '120000', 'A'), allow_modes=True)[0])
        self.assertFalse(policy.inspect(self.record('000000', '120000', 'A'), allow_structural=True)[0])

    def test_new_regular_executable_is_admitted(self):
        self.assertFalse(policy.inspect(self.record('000000', '100755', 'A'))[0])

    def test_explicit_structural_override(self):
        self.assertFalse(policy.inspect(self.record('100644', '000000', 'D'), allow_structural=True)[0])

    def test_malformed_fails_closed(self):
        with self.assertRaises(ValueError):
            policy.inspect(b'not a git record\0')

    def test_real_snapshot_compact_recoverable(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td); repo = root / 'repo'; repo.mkdir()
            def git(*args):
                return subprocess.check_output(['git', '-C', str(repo), *args], stderr=subprocess.DEVNULL).decode().strip()
            git('init', '-q'); git('config', 'user.name', 'test'); git('config', 'user.email', 'test@example.invalid')
            (repo / 'scripts').mkdir()
            for name in ('lccc-snapshot.sh', 'worktree_tree.sh', 'snapshot_policy.py', 'lccc_delivery.py', 'session_bundle.py'):
                shutil.copy2(REPO / 'scripts' / name, repo / 'scripts' / name)
            (repo / 'a.txt').write_text('base\n')
            git('add', '.'); git('commit', '-qm', 'base'); base = git('rev-parse', 'HEAD')
            env = dict(os.environ, LCCC_REPO=str(repo), LCCC_ARTIFACTS=str(root/'art'),
                       LCCC_DELIVERABLE=str(root/'ms178-1.patch'), LCCC_SNAPSHOT_UNGATED='1',
                       LCCC_BASE_REF=base, LCCC_SNAPSHOT_KEEP='2')
            env.pop('LCCC_BULK_ARTIFACTS', None)
            for i in range(3):
                (repo / 'a.txt').write_text(f'change {i}\n')
                p = subprocess.run(['bash', str(repo/'scripts/lccc-snapshot.sh'), f'fix{i}', 'tested'],
                                   env=env, capture_output=True, text=True, timeout=45)
                self.assertEqual(p.returncode, 0, p.stderr)
            art = root / 'art'
            self.assertFalse((art / 'ms178-1.patch').exists())
            self.assertFalse((art / 'series').exists())
            snapshots = sorted(art.glob('ms178-1.S*.patch.gz'))
            self.assertEqual(len(snapshots), 2)
            self.assertEqual(gzip.decompress(snapshots[-1].read_bytes()), (root/'ms178-1.patch').read_bytes())
            self.assertIn(b'change 2', gzip.decompress((art/'lccc-src.tar.gz').read_bytes()))
            recovered = root/'recovered'
            subprocess.run(['git', 'init', '-q', str(recovered)], check=True)
            subprocess.run(['git','-C',str(recovered),'fetch','-q','--depth=1',str(repo),base],check=True)
            saved_branch=git('branch','--show-current')
            subprocess.run(['git','-C',str(recovered),'fetch','-q',str(art/'lccc-session.bundle'),saved_branch],check=True)
            subprocess.run(['git','-C',str(recovered),'checkout','-q','FETCH_HEAD'],check=True)
            self.assertEqual((recovered/'a.txt').read_text(), 'change 2\n')

    def test_failed_bundle_clone_preserves_previous_durable_bundle(self):
        with tempfile.TemporaryDirectory() as td:
            root=Path(td); repo=root/'repo'; repo.mkdir()
            def git(*args):
                return subprocess.check_output(['git','-C',str(repo),*args], stderr=subprocess.DEVNULL).decode().strip()
            git('init','-q'); git('config','user.name','test'); git('config','user.email','test@example.invalid')
            (repo/'scripts').mkdir()
            for name in ('lccc-snapshot.sh','worktree_tree.sh','snapshot_policy.py','lccc_delivery.py','session_bundle.py'):
                shutil.copy2(REPO/'scripts'/name,repo/'scripts'/name)
            (repo/'a.txt').write_text('base\n'); git('add','.'); git('commit','-qm','base')
            env=dict(os.environ, LCCC_REPO=str(repo), LCCC_ARTIFACTS=str(root/'art'),
                     LCCC_DELIVERABLE=str(root/'ms178-1.patch'), LCCC_SNAPSHOT_UNGATED='1',
                     LCCC_BASE_REF=git('rev-parse','HEAD'))
            env.pop('LCCC_BULK_ARTIFACTS',None)
            (repo/'a.txt').write_text('first validated change\n')
            cmd=['bash',str(repo/'scripts/lccc-snapshot.sh'),'fixture','fixture save']
            first=subprocess.run(cmd,env=env,capture_output=True,text=True,timeout=45)
            self.assertEqual(first.returncode,0,first.stderr)
            previous=(root/'art/lccc-session.bundle').read_bytes()
            previous_zip=(root/'ms178-1.patch.zip').read_bytes()
            shim=root/'shim'; shim.mkdir()
            real_git=shutil.which('git')
            (shim/'git').write_text('#!/usr/bin/env bash\n'
                                   'for arg in "$@"; do [[ "$arg" == verify ]] && exit 1; done\n'
                                   'exec '+real_git+' "$@"\n')
            (shim/'git').chmod(0o755)
            env['PATH']=str(shim)+os.pathsep+env['PATH']
            (repo/'a.txt').write_text('second validated change\n')
            second=subprocess.run(cmd,env=env,capture_output=True,text=True,timeout=45)
            self.assertNotEqual(second.returncode,0)
            self.assertIn('previous bundle preserved',second.stderr)
            self.assertEqual((root/'art/lccc-session.bundle').read_bytes(),previous)
            self.assertEqual((root/'ms178-1.patch.zip').read_bytes(),previous_zip)
            self.assertIn(b'second validated change',(root/'ms178-1.patch').read_bytes())
            self.assertIn(b'second validated change',gzip.decompress((root/'art/lccc-src.tar.gz').read_bytes()))


if __name__ == '__main__':
    unittest.main()
