import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from scripts import restore_toolchain_cache as cache

tarfile = cache.tarfile


class ToolchainCache(unittest.TestCase):
    def test_reviewed_lock_has_both_requested_versions(self):
        lock = json.loads(cache.LOCK.read_text())
        self.assertEqual(set(lock['tools']), set(cache.VERSIONS))
        for name, entry in lock['tools'].items():
            cache.validate(entry, name)

    def entry(self):
        sha = 'a' * 64
        return {'version': '1.99.1', 'sha256': sha,
                'url': f'https://github.com/ms178/lccc/releases/download/harness-cache-4/rust-1.99.1-{sha}.tar.xz'}

    def test_version_digest_and_origin_are_independent_requirements(self):
        cache.validate(self.entry(), 'rust')
        for key, value in (('version', '1.97.0'), ('sha256', 'bad'),
                           ('url', 'https://example.org/archive.tar.xz')):
            entry = self.entry()
            entry[key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                cache.validate(entry, 'rust')

    def test_corrupt_archive_never_extracts_or_runs(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            entry = self.entry()
            (root / entry['url'].rsplit('/', 1)[1]).write_bytes(b'corruption')
            with patch.object(cache, 'unpack') as unpack, patch.object(cache, 'verify') as verify:
                with self.assertRaisesRegex(ValueError, 'SHA-256 mismatch'):
                    cache.restore(entry, 'rust', root, root)
                unpack.assert_not_called()
                verify.assert_not_called()

    def test_unpack_removes_only_packaging_root(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            archive = root / 'payload.tar'
            with tarfile.open(archive, 'w') as output:
                member = tarfile.TarInfo('pack/bin/rustc')
                member.size = 2
                output.addfile(member, io.BytesIO(b'ok'))
            destination = root / 'out'
            cache.unpack(archive, destination)
            self.assertEqual((destination / 'bin/rustc').read_bytes(), b'ok')

    def test_unpack_refuses_traversal_and_escaping_links(self):
        for name, link in (('pack/../../escaped', None), ('pack/link', '../../escaped')):
            with self.subTest(name=name, link=link), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                archive = root / 'payload.tar'
                with tarfile.open(archive, 'w') as output:
                    member = tarfile.TarInfo(name)
                    if link:
                        member.type = tarfile.SYMTYPE
                        member.linkname = link
                    output.addfile(member)
                with self.assertRaises(tarfile.FilterError):
                    cache.unpack(archive, root / 'out')
                self.assertFalse((root / 'escaped').exists())
