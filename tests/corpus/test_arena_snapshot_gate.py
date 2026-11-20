"""Contract tests for scripts/arena_snapshot.sh's gate stamp.

The publisher's gate MUST be keyed on the same content address `ci_local.sh`
writes (`scripts/worktree_tree.sh`, fields `tree=`/`mode=`), because a gate
keyed on anything else can never fire and then every publication reads as
`UNGATED` -- which is exactly the defect these tests pin down.
"""
from __future__ import annotations

import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

REPO = Path(__file__).resolve().parents[2]


class ArenaSnapshotGateTests(unittest.TestCase):
    def make_repo(self, root: Path) -> Path:
        repo = root / 'repo'
        repo.mkdir()
        def git(*args):
            return subprocess.check_output(['git', '-C', str(repo), *args],
                                           stderr=subprocess.DEVNULL).decode().strip()
        git('init', '-q')
        git('config', 'user.name', 'test')
        git('config', 'user.email', 'test@example.invalid')
        (repo / 'scripts').mkdir()
        for name in ('arena_snapshot.sh', 'worktree_tree.sh'):
            shutil.copy2(REPO / 'scripts' / name, repo / 'scripts' / name)
        # `target/` is ignored in the real tree, and it must be here too: the
        # ci_local pass stamp lives in it, and an untracked `target/` would
        # change what `worktree_tree.sh` (git add -A) hashes.
        (repo / '.gitignore').write_text('target/\n')
        (repo / 'a.txt').write_text('base\n')
        git('add', '.')
        git('commit', '-qm', 'base')
        git('branch', '-M', 'main')
        return repo

    def run_publisher(self, repo: Path, root: Path, *extra_env):
        env = dict(os.environ,
                   LCCC_REPO=str(repo),
                   MS178_OUTDIR=str(root / 'snapshots'),
                   MS178_PUBLISH_DIR=str(root / 'publish'),
                   MS178_BASE_REF='main',
                   HOME=str(root))
        for key, value in extra_env:
            env[key] = value
        return subprocess.run(['bash', str(repo / 'scripts' / 'arena_snapshot.sh'),
                               'gate-test', 'gate contract'],
                              env=env, capture_output=True, text=True, timeout=120)

    def test_matching_stamp_is_a_pass(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            repo = self.make_repo(root)
            (repo / 'a.txt').write_text('change\n')
            tree = subprocess.check_output(
                ['bash', str(repo / 'scripts' / 'worktree_tree.sh')],
                cwd=repo).decode().strip()
            stamp = repo / 'target' / 'ci_local.pass'
            stamp.parent.mkdir()
            stamp.write_text(f'tree={tree}\nmode=fast\nhead=deadbeef\nos=debian-13\n')
            result = self.run_publisher(repo, root)
            self.assertEqual(result.returncode, 0, result.stderr)
            ledger = (root / 'snapshots' / 'LINEAGE.log').read_text()
            self.assertIn('gate       ci_local --fast PASS', ledger)
            self.assertNotIn('gate       UNGATED', ledger)

    def test_stamp_for_another_tree_is_ungated(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            repo = self.make_repo(root)
            (repo / 'a.txt').write_text('change\n')
            stamp = repo / 'target' / 'ci_local.pass'
            stamp.parent.mkdir()
            stamp.write_text('tree=' + '0' * 40 + '\nmode=fast\n')
            result = self.run_publisher(repo, root, ('MS178_UNGATED', '1'))
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn('not the tree being published', result.stderr)
            self.assertIn('gate       UNGATED',
                          (root / 'snapshots' / 'LINEAGE.log').read_text())

    def test_absent_stamp_is_ungated_and_loud_without_override(self):
        # The publisher warns loudly; it does not exit non-zero.  The exit
        # status is left alone ON PURPOSE: the ledger records `gate UNGATED`,
        # which is the honest label, and the warning names the override.  What
        # must never happen is the silent UNGATED this defect produced while a
        # green stamp sat right there.
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            repo = self.make_repo(root)
            (repo / 'a.txt').write_text('change\n')
            refused = self.run_publisher(repo, root)
            self.assertEqual(refused.returncode, 0, refused.stderr)
            self.assertIn('no ci_local stamp for this tree', refused.stderr)
            self.assertIn('gate       UNGATED',
                          (root / 'snapshots' / 'LINEAGE.log').read_text())


if __name__ == '__main__':
    unittest.main()
