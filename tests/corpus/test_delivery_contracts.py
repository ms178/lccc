"""Transport/fault contracts; no compiler, network, or real user files touched."""
from __future__ import annotations

import hashlib
import importlib.util
import json
import os
from pathlib import Path
import stat
import subprocess
import tempfile
import unittest
from unittest import mock
import zipfile

REPO = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location('lccc_delivery', REPO/'scripts/lccc_delivery.py')
d = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(d)
META = dict(base='a'*40, head='b'*40, snapshot='S04-delivery', ci_gate='UNGATED')


class DeliveryContracts(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.patch = self.root/'ms178-1.patch'
        self.patch.write_bytes(b'diff --git a/a b/a\n'+b'+a complete source line\n'*100)
        self.old = self.root/'ms178-1.patch.zip'

    def tearDown(self):
        self.temp.cleanup()

    def publish(self, **kwargs):
        return d.publish(self.patch, self.root, META, **kwargs)

    def test_exact_bytes_hash_base_and_ci_status_in_zip(self):
        result = self.publish()
        with zipfile.ZipFile(self.old) as z:
            self.assertEqual(z.read('ms178-1.patch'), self.patch.read_bytes())
            meta=json.loads(z.read('DELIVERY.json'))
            self.assertEqual(meta['base'], META['base'])
            self.assertEqual(meta['ci_gate'], 'UNGATED')
            self.assertEqual(meta['patch_sha256'], hashlib.sha256(self.patch.read_bytes()).hexdigest())
            self.assertIsNone(z.testzip())
        self.assertEqual(result['zip_sha256'], d.digest(self.old))
        self.assertEqual(stat.S_IMODE(self.old.stat().st_mode), 0o644)

    def test_deterministic_archive_for_identical_metadata(self):
        self.publish(); first=self.old.read_bytes()
        self.publish(); self.assertEqual(self.old.read_bytes(), first)

    def test_replace_failure_preserves_previous_zip(self):
        self.old.write_bytes(b'previous complete delivery')
        with mock.patch.object(d.os, 'replace', side_effect=OSError('injected rename failure')):
            with self.assertRaises(OSError):
                self.publish()
        self.assertEqual(self.old.read_bytes(), b'previous complete delivery')
        self.assertEqual(list(self.root.glob('*.tmp')), [])

    def test_fsync_failure_preserves_previous_zip(self):
        self.old.write_bytes(b'previous complete delivery')
        with mock.patch.object(d.os, 'fsync', side_effect=OSError('injected fsync failure')):
            with self.assertRaises(OSError):
                self.publish()
        self.assertEqual(self.old.read_bytes(), b'previous complete delivery')
        self.assertFalse(any(p.name.endswith('.tmp') for p in self.root.iterdir()))

    def test_sidecar_failure_does_not_corrupt_published_zip(self):
        real_replace=d.os.replace
        def replace(src, dst):
            if Path(dst).name=='LCCC-DELIVERY.json':
                raise OSError('injected receipt failure')
            return real_replace(src, dst)
        with mock.patch.object(d.os, 'replace', side_effect=replace):
            with self.assertRaises(OSError):
                self.publish()
        with zipfile.ZipFile(self.old) as z:
            self.assertEqual(z.read('ms178-1.patch'), self.patch.read_bytes())
        self.assertFalse(any(p.name.endswith('.tmp') for p in self.root.iterdir()))

    def test_size_budget_fails_loudly_no_user_deletions(self):
        user_file=self.root/'precious.txt'; user_file.write_bytes(b'x'*3000)
        self.old.write_bytes(b'previous')
        with self.assertRaises(d.DeliveryError):
            self.publish(max_bytes=4096)
        self.assertEqual(user_file.read_bytes(), b'x'*3000)
        self.assertEqual(self.old.read_bytes(), b'previous')

    def test_file_budget_accounts_for_added_outputs(self):
        self.old.write_bytes(b'previous')
        with self.assertRaises(d.DeliveryError):
            self.publish(max_files=3)
        self.assertEqual(self.old.read_bytes(), b'previous')

    def test_current_large_caches_not_silently_ignored(self):
        cache=self.root/'.cache'; cache.mkdir(); (cache/'big').write_bytes(b'X'*10000)
        with self.assertRaises(d.DeliveryError):
            self.publish(max_bytes=5000)
        self.assertTrue((cache/'big').exists())

    def test_patch_symlink_rejected(self):
        self.patch.unlink(); (self.root/'other').write_bytes(b'patch')
        self.patch.symlink_to(self.root/'other')
        with self.assertRaises(d.DeliveryError):
            self.publish()

    def test_output_symlink_rejected_without_mutating_target(self):
        target=self.root/'precious'; target.write_bytes(b'keep')
        self.old.symlink_to(target)
        with self.assertRaises(d.DeliveryError):
            self.publish()
        self.assertEqual(target.read_bytes(), b'keep')

    def test_patch_mutation_during_generation_detected(self):
        real_digest=d.digest
        calls=[]
        def changing(path):
            calls.append(path)
            if path==self.patch and len(calls)==1:
                before=real_digest(path); path.write_bytes(b'changed after initial hash'); return before
            return real_digest(path)
        self.old.write_bytes(b'previous')
        with mock.patch.object(d, 'digest', side_effect=changing):
            with self.assertRaises(d.DeliveryError):
                self.publish()
        self.assertEqual(self.old.read_bytes(), b'previous')

    def test_concurrent_publisher_rejected(self):
        with d.writer_lock(self.root):
            with self.assertRaises(d.DeliveryError):
                self.publish()

    def test_empty_patch_cannot_certify_delivery(self):
        self.patch.write_bytes(b'')
        with self.assertRaises(d.DeliveryError):
            self.publish()

    def test_full_commit_ids_and_explicit_status_required(self):
        for meta in [dict(META, base='a'), dict(META, ci_gate=''), dict(META, unverified='extra')]:
            with self.subTest(meta=meta):
                with self.assertRaises(d.DeliveryError):
                    d.publish(self.patch, self.root, meta)

    def test_cli_failure_is_nonzero_not_a_false_pass(self):
        p=subprocess.run(['python3',str(REPO/'scripts/lccc_delivery.py'),
                          '--patch',str(self.patch),'--workspace',str(self.root),
                          '--base',META['base'],'--head',META['head'],
                          '--snapshot',META['snapshot'],'--ci-gate','UNGATED',
                          '--budget-mib','0'], capture_output=True, text=True)
        self.assertNotEqual(p.returncode,0)
        self.assertIn('FAILED',p.stderr)

    def test_inventory_does_not_follow_external_directory_symlink(self):
        with tempfile.TemporaryDirectory() as ext:
            (Path(ext)/'external').write_bytes(b'x'*9999)
            (self.root/'external-tree').symlink_to(ext, target_is_directory=True)
            r=d.inventory(self.root)
            self.assertEqual(r['symlinks'],1)
            self.assertLess(r['bytes'],9999)


if __name__=='__main__':
    unittest.main()
