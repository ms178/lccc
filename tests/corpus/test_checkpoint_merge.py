"""Lossless checkpoint compaction must never turn a failure into lost history."""
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
SPEC = importlib.util.spec_from_file_location('checkpoint_merge', ROOT / 'scripts/merge_checkpoint_streams.py')
MERGE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MERGE)


def tar_bytes(value):
    stream = io.BytesIO()
    with tarfile.open(fileobj=stream, mode='w') as archive:
        entry = tarfile.TarInfo('MANIFEST.json')
        entry.size = len(value)
        archive.addfile(entry, io.BytesIO(value))
    return stream.getvalue()


class CheckpointMergeTests(unittest.TestCase):
    def test_every_tar_byte_and_duplicate_manifest_survive(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            sources = [root / 'a.xz', root / 'b.xz']
            payloads = [tar_bytes(b'first'), tar_bytes(b'second')]
            for path, data in zip(sources, payloads):
                path.write_bytes(lzma.compress(data))
            out = root / 'merged.xz'
            receipt = MERGE.merge(sources, out, remove=True)
            raw = lzma.decompress(out.read_bytes())
            self.assertEqual(raw, b''.join(payloads))
            self.assertTrue(all(not path.exists() for path in sources))
            self.assertEqual(receipt['sha256'], hashlib.sha256(out.read_bytes()).hexdigest())
            self.assertEqual(json.loads(Path(str(out) + '.receipt.json').read_text()), receipt)
            with tarfile.open(fileobj=io.BytesIO(raw), ignore_zeros=True) as archive:
                self.assertEqual([archive.extractfile(m).read() for m in archive], [b'first', b'second'])

    def test_corrupt_source_does_not_remove_either_original(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            a, b, out = [root / name for name in ['a.xz', 'b.xz', 'out.xz']]
            a.write_bytes(lzma.compress(tar_bytes(b'first')))
            b.write_bytes(b'corrupt')
            with self.assertRaises(lzma.LZMAError):
                MERGE.merge([a, b], out, remove=True)
            self.assertTrue(a.exists() and b.exists())
            self.assertFalse(out.exists())
            self.assertEqual(sorted(p.name for p in root.iterdir()), ['.snapshot.lock', 'a.xz', 'b.xz'])

    def test_destination_is_rechecked_after_waiting_for_lock(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source, out = root / 'a.xz', root / 'out.xz'
            source.write_bytes(lzma.compress(tar_bytes(b'first')))
            with patch.object(MERGE.fcntl, 'flock', side_effect=lambda *args: out.write_bytes(b'other publisher')):
                with self.assertRaisesRegex(ValueError, 'appeared'):
                    MERGE.merge([source], out, remove=True)
            self.assertEqual(out.read_bytes(), b'other publisher')
            self.assertTrue(source.exists())


if __name__ == '__main__':
    unittest.main()
