"""Publication failure/negative controls for lossless, repeatable compaction."""
import gzip
import hashlib
import importlib.util
import io
import json
import lzma
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('compact_session', ROOT / 'scripts/compact_session_artifacts.py')
compact = importlib.util.module_from_spec(spec)
spec.loader.exec_module(compact)


class CompactTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.scratch = self.root / 'scratch'
        self.scratch.mkdir()
        self.receipt = self.root / 'history.json'
        self.archive = self.root / 'old.tar.xz'
        members = {'old.patch': b'old patch\n', 'lccc-session.bundle': b'old bundle\n'}
        with tarfile.open(self.archive, 'w:xz', preset=0) as archive:
            for name, data in members.items():
                info = tarfile.TarInfo(name)
                info.size = len(data)
                archive.addfile(info, io.BytesIO(data))
        self.old = {'archive': self.archive.name, 'archive_sha256': compact.digest(self.archive),
                    'members': [{'name': n, 'bytes': len(b), 'sha256': hashlib.sha256(b).hexdigest()}
                                for n, b in members.items()]}
        self.save()

    def save(self):
        self.receipt.write_text(json.dumps(self.old))

    def run_compactor(self, additions=None):
        return compact.compact_archive(self.root, self.receipt.name, additions or {}, self.scratch, preset=0)

    def test_preservation_and_repeatability(self):
        source = self.root / 'new.patch.gz'
        source.write_bytes(gzip.compress(b'new patch\n', mtime=0))
        first = self.run_compactor({'new.patch': source})
        self.assertEqual(len(first['members']), 3)
        self.assertTrue(source.exists(), 'the function must not delete caller-owned inputs')
        second = self.run_compactor({'new.patch': source})
        self.assertEqual(first, second)
        with tarfile.open(self.root / first['archive'], 'r:xz') as archive:
            self.assertEqual(archive.extractfile('new.patch').read(), b'new patch\n')
            self.assertEqual(archive.extractfile('old.patch').read(), b'old patch\n')

    def test_old_rolling_bundle_is_retained(self):
        source = self.root / 'new.bundle'
        source.write_bytes(b'new bundle\n')
        result = self.run_compactor({'lccc-session.bundle': source})
        old_name = 'lccc-session.' + self.old['members'][1]['sha256'] + '.bundle'
        with tarfile.open(self.root / result['archive'], 'r:xz') as archive:
            self.assertEqual(archive.extractfile(old_name).read(), b'old bundle\n')
            self.assertEqual(archive.extractfile('lccc-session.bundle').read(), b'new bundle\n')

    def test_compacted_bundle_is_resolved_by_the_real_recovery_consumer(self):
        # This caught an actual end-to-end restore failure: updating only
        # BUNDLE_ARCHIVE left SESSION_BUNDLE pointing at a deleted loose file.
        receipt = self.root / 'BUNDLE_ARCHIVE.json'
        self.receipt.rename(receipt)
        self.receipt = receipt
        raw = self.root / 'lccc-session.bundle'
        raw.write_bytes(b'new bundle\n')
        session = self.root / 'SESSION_BUNDLE.json'
        session.write_text(json.dumps({'bundle': raw.name, 'bundle_sha256': compact.digest(raw)}))
        result = self.run_compactor({raw.name: raw})
        raw.unlink()
        linked = json.loads(session.read_text())
        self.assertEqual(linked['bundle_archive'], result['archive'])
        spec = importlib.util.spec_from_file_location('compact_recovery_consumer', ROOT / 'scripts/lccc_recover.py')
        recovery = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(recovery)
        recovered = recovery.resolve_bundle(self.scratch, self.root, linked)
        self.assertEqual(recovered.read_bytes(), b'new bundle\n')

    def test_bad_session_receipt_does_not_delete_old_archive(self):
        receipt = self.root / 'BUNDLE_ARCHIVE.json'
        self.receipt.rename(receipt)
        self.receipt = receipt
        session = self.root / 'SESSION_BUNDLE.json'
        original = {'bundle': 'lccc-session.bundle', 'bundle_sha256': '0' * 64}
        session.write_text(json.dumps(original))
        with self.assertRaisesRegex(ValueError, 'not authenticated'): self.run_compactor()
        self.assertTrue(self.archive.exists())
        self.assertEqual(json.loads(session.read_text()), original)

    def test_conflicting_checkpoint_leaves_old_publication(self):
        source = self.root / 'changed.patch'
        source.write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError, 'conflicting checkpoint'):
            self.run_compactor({'old.patch': source})
        self.assertEqual(json.loads(self.receipt.read_text()), self.old)
        self.assertTrue(self.archive.exists())

    def test_receipt_failure_keeps_old_archive_recoverable(self):
        source = self.root / 'new.patch'
        source.write_bytes(b'new')
        with patch.object(compact, 'atomic_json', side_effect=OSError('simulated publication failure')):
            with self.assertRaises(OSError): self.run_compactor({'new.patch': source})
        self.assertEqual(json.loads(self.receipt.read_text()), self.old)
        self.assertEqual(compact.digest(self.archive), self.old['archive_sha256'])
        self.assertEqual(len(self.run_compactor({'new.patch': source})['members']), 3)

    def test_compressed_corruption(self):
        self.archive.write_bytes(self.archive.read_bytes()[:-1])
        with self.assertRaisesRegex(ValueError, 'archive checksum'): self.run_compactor()

    def test_member_corruption_cannot_hide_in_valid_container(self):
        self.old['members'][0]['sha256'] = '0' * 64
        self.save()
        with self.assertRaisesRegex(ValueError, 'member checksum'): self.run_compactor()
        self.assertTrue(self.archive.exists())

    def test_extra_member_rejected(self):
        self.old['members'].pop()
        self.save()
        with self.assertRaisesRegex(ValueError, 'unexpected'): self.run_compactor()

    def test_missing_member_rejected(self):
        self.old['members'].append({'name': 'absent', 'bytes': 0, 'sha256': hashlib.sha256(b'').hexdigest()})
        self.save()
        with self.assertRaisesRegex(ValueError, 'missing'): self.run_compactor()

    def test_duplicate_receipt_member_rejected(self):
        self.old['members'].append(dict(self.old['members'][0]))
        self.save()
        with self.assertRaisesRegex(ValueError, 'duplicate'): self.run_compactor()

    def test_unsafe_names(self):
        for name in ('../escape', '/absolute', './alias', '.', ''):
            with self.subTest(name=name), self.assertRaises(ValueError): compact.safe_name(name)

    def test_truncated_gzip_rejected_before_publication(self):
        source = self.root / 'new.patch.gz'
        source.write_bytes(gzip.compress(b'data')[:-5])
        with self.assertRaises((OSError, EOFError)):
            self.run_compactor({'new.patch': source})
        self.assertEqual(json.loads(self.receipt.read_text()), self.old)

    def test_source_stream_preserved_and_repeatable(self):
        source = self.root / 'source.tar.gz'
        raw = b'exact tar stream\n' * 1024
        source.write_bytes(gzip.compress(raw, mtime=0))
        receipt = self.root / 'SOURCE_ARCHIVE.json'
        receipt.write_text(json.dumps({'archive': source.name, 'archive_sha256': compact.digest(source)}))
        result = compact.compact_source(self.root, self.scratch, preset=0)
        self.assertEqual(lzma.decompress((self.root / result['archive']).read_bytes()), raw)
        self.assertEqual(compact.compact_source(self.root, self.scratch, preset=0), result)
        self.assertFalse(source.exists())
