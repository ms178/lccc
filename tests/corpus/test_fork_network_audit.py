#!/usr/bin/env python3
"""Offline, tiny Git fixtures for the non-executing fork inventory."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'scripts'))
import audit_fork_network as audit


class ParsingTests(unittest.TestCase):
    def test_old_gh_consecutive_pages(self):
        self.assertEqual(audit.json_pages('[{"x":1}]\n []\n'), [[{"x": 1}], []])
        for text in ('', '[] trailing garbage'):
            with self.assertRaises(ValueError):
                audit.json_pages(text)

    def test_repository_and_object_ids_are_not_shell_or_ref_expressions(self):
        for name in ('../repo', 'a/../b', 'a/repo;cmd', 'a/..', 'https://github.com/a/b'):
            with self.assertRaises(ValueError):
                audit.repository_name(name)
        for sha in ('main', 'HEAD~1', '-x', '0' * 39, 'z' * 40):
            with self.assertRaises(ValueError):
                audit.object_id(sha)
        self.assertEqual(audit.repository_name('Some-owner/repo.name'), 'Some-owner/repo.name')

    def test_all_heads_including_nondefault_and_duplicate_rejection(self):
        raw = b'a' * 40 + b'\trefs/heads/main\n' + b'b' * 40 + b'\trefs/heads/topic/fix\n'
        self.assertEqual(audit.parse_heads(raw), {'main': 'a' * 40, 'topic/fix': 'b' * 40})
        with self.assertRaises(ValueError):
            audit.parse_heads(raw + b'c' * 40 + b'\trefs/heads/main\n')
        with self.assertRaises(ValueError):
            audit.parse_heads(b'a' * 40 + b'\trefs/tags/main\n')

    def test_nul_delta_handles_whitespace_and_empty_tree_difference(self):
        raw = b':100644 100755 ' + b'a' * 40 + b' ' + b'b' * 40 + b' M\0src/a\tb\nc.rs\0'
        change, = audit.parse_delta(raw)
        self.assertEqual(change['path'], 'src/a\tb\nc.rs')
        self.assertEqual(change['new_mode'], '100755')
        self.assertEqual(audit.parse_delta(b''), [])
        with self.assertRaises(ValueError):
            audit.parse_delta(raw[:-1])

    def test_atomic_report_replaces_complete_json(self):
        with tempfile.TemporaryDirectory() as td:
            path = Path(td) / 'sub' / 'report.json'
            audit.write_report(path, {'complete': False, 'message': 'first'})
            audit.write_report(path, {'complete': True})
            self.assertEqual(json.loads(path.read_text()), {'complete': True})
            self.assertEqual(list(path.parent.iterdir()), [path])

    def test_recursive_discovery_preserves_inaccessible_fork(self):
        a = {'full_name': 'owner/a', 'default_branch': 'main', 'forks_count': 1}
        b = {'full_name': 'owner/b', 'default_branch': 'main', 'forks_count': 0}
        def github(endpoint, **kwargs):
            if endpoint == 'repos/owner/root':
                return [{'full_name': 'owner/root', 'default_branch': 'main'}]
            if endpoint.startswith('repos/owner/root/forks?'):
                return [[a]]
            if endpoint.startswith('repos/owner/a/forks?'):
                return [[b]]
            self.fail(endpoint)
        def heads(name):
            return {'error': 'unavailable'} if name == 'owner/b' else {'heads': {'main': 'a' * 40}}
        with patch.object(audit, 'github', side_effect=github), \
             patch.object(audit, 'list_heads', side_effect=heads):
            report = audit.discover('owner/root', 'owner/target', 2)
        self.assertFalse(report['complete'])
        self.assertEqual(len(report['repositories']), 3)
        self.assertEqual(report['repositories'][1]['error'], 'unavailable')


class TreeTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.cache = Path(self.temp.name) / 'audit.git'
        subprocess.run(['git', 'init', '--bare', '-q', str(self.cache)], check=True)
        self.env = {**os.environ, 'GIT_AUTHOR_NAME': 'fixture',
                    'GIT_AUTHOR_EMAIL': 'fixture@example.invalid',
                    'GIT_COMMITTER_NAME': 'fixture',
                    'GIT_COMMITTER_EMAIL': 'fixture@example.invalid'}
        self.root = self.commit({'code.rs': b'pub fn value() -> i32 { 1 }\n'}, 'baseline')
        self.changed = self.commit({'code.rs': b'pub fn value() -> i32 { 2 }\n',
                                    'build.rs': b'fn main() { panic!("must not execute"); }\n'},
                                   'Edit README.md', self.root)
        self.same_tree = self.commit({'code.rs': b'pub fn value() -> i32 { 1 }\n'},
                                     'different commit, identical tree', self.root)

    def command(self, *args, data=None):
        return subprocess.run(['git', '-C', str(self.cache), *args], input=data,
                              capture_output=True, check=True, env=self.env).stdout.strip()

    def commit(self, files, subject, parent=None):
        # code.rs under src/, build.rs at the top level (never execute either).
        code = self.command('hash-object', '-w', '--stdin', data=files['code.rs']).decode()
        src = self.command('mktree', data=f'100644 blob {code}\tcode.rs\n'.encode()).decode()
        top = [f'040000 tree {src}\tsrc\n']
        if 'build.rs' in files:
            build = self.command('hash-object', '-w', '--stdin', data=files['build.rs']).decode()
            top.append(f'100644 blob {build}\tbuild.rs\n')
        tree = self.command('mktree', data=''.join(top).encode()).decode()
        args = ['commit-tree', tree, '-m', subject]
        if parent:
            args += ['-p', parent]
        return self.command(*args).decode()

    def inventory(self, unavailable=False):
        rows = [{'full_name': 'owner/root', 'default_branch': 'main', 'heads': {'main': self.root}},
                {'full_name': 'owner/fork', 'default_branch': 'main',
                 'heads': {'main': self.root, 'topic': self.changed, 'duplicate': self.changed,
                           'same-tree': self.same_tree}},
                {'full_name': 'owner/target', 'default_branch': 'main',
                 'heads': {'private-research': 'f' * 40}}]
        if unavailable:
            rows.append({'full_name': 'owner/missing', 'default_branch': 'main', 'error': 'unavailable'})
        return {'schema': 1, 'observed_utc': 'fixture', 'root': 'owner/root',
                'target': 'owner/target', 'repositories': rows}

    def test_deduplicates_tips_not_just_default_branches_and_flags_build_paths(self):
        report = audit.inspect(self.inventory(), self.cache, False)
        self.assertTrue(report['complete'])
        self.assertEqual(report['distinct_tips'], 3)
        self.assertEqual(report['distinct_trees'], 2)
        changed = next(t for t in report['tips'] if t['tip'] == self.changed)
        self.assertEqual(changed['source_paths'], ['src/code.rs'])
        self.assertEqual(changed['execution_boundary_paths'], ['build.rs'])
        self.assertEqual(len(changed['refs']), 2)
        self.assertEqual(changed['subject'], 'Edit README.md')
        unchanged = next(t for t in report['tips'] if t['tip'] == self.same_tree)
        self.assertEqual(unchanged['changed_paths'], 0)
        self.assertFalse((self.cache / 'src').exists(), 'must never check out foreign code')

    def test_against_compares_exact_entries_without_claiming_adoption(self):
        report = audit.inspect(self.inventory(), self.cache, False, self.changed)
        changed = next(t for t in report['tips'] if t['tip'] == self.changed)
        self.assertEqual(changed['against_entry_counts'], {'identical-entry': 1})
        self.assertEqual(report['against'], self.changed)
        report = audit.inspect(self.inventory(), self.cache, False, self.root)
        changed = next(t for t in report['tips'] if t['tip'] == self.changed)
        self.assertEqual(changed['against_entry_counts'], {'diverged-review-required': 1})
        with self.assertRaises(ValueError):
            audit.inspect(self.inventory(), self.cache, False, 'main')
        tree = self.command('rev-parse', self.root + '^{tree}').decode()
        with self.assertRaisesRegex(ValueError, 'commit'):
            audit.inspect(self.inventory(), self.cache, False, tree)

    def test_unavailable_repository_cannot_become_a_complete_audit(self):
        report = audit.inspect(self.inventory(unavailable=True), self.cache, False)
        self.assertFalse(report['complete'])
        self.assertEqual(len(report['failures']), 1)

    def test_missing_tip_is_reported_without_network_unless_fetch_is_explicit(self):
        inventory = self.inventory()
        inventory['repositories'][1]['heads']['missing'] = 'e' * 40
        report = audit.inspect(inventory, self.cache, False)
        self.assertFalse(report['complete'])
        self.assertEqual(report['failures'][0]['tip'], 'e' * 40)

    def test_non_bare_cache_is_refused(self):
        path = Path(self.temp.name) / 'ordinary'
        subprocess.run(['git', 'init', '-q', str(path)], check=True)
        with self.assertRaisesRegex(ValueError, 'bare'):
            audit.inspect(self.inventory(), path, False)


if __name__ == '__main__':
    unittest.main()
