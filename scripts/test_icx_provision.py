"""Compiler-free, offline tests for authenticated ICX package selection/publication."""
from __future__ import annotations

import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import icx_provision as icx


class PackageIdentity(unittest.TestCase):
    def test_release_line_and_patch_macro(self):
        self.assertEqual(icx.semver('2026.2.0-123'), '2026.2.0')
        self.assertEqual(icx.macro_for('2026.2.1-123'), '20260201')
        for version in ('2026.1.1-325', '2027.2.0-1', '(none)', '2026.2.100-1', '2026.2.0;id'):
            with self.subTest(version=version), self.assertRaises(icx.ProvisionError):
                icx.semver(version)

    def record(self, **changes):
        fields = dict(Package=icx.PACKAGES[0], Version='2026.2.0-123', Architecture='amd64',
                      Filename='pool/main/icx.deb', SHA256='a' * 64)
        fields.update(changes)
        return '\n'.join(f'{k}: {v}' for k, v in fields.items()) + '\n'

    def test_exact_package_record(self):
        data = icx.package_record(self.record(), icx.PACKAGES[0], '2026.2.0-123')
        self.assertEqual(data['SHA256'], 'a' * 64)

    def test_invalid_or_duplicate_records_fail_closed(self):
        for text in (self.record(Architecture='arm64'), self.record(SHA256=''),
                     self.record(Filename='pool/../evil.deb'), self.record(Package='other'),
                     self.record() + '\n' + self.record()):
            with self.subTest(text=text), self.assertRaises(icx.ProvisionError):
                icx.package_record(text, icx.PACKAGES[0], '2026.2.0-123')

    def test_continuation_lines_are_not_fields(self):
        text = self.record() + 'Description: test\n SHA256: bad\n'
        data = icx.package_record(text, icx.PACKAGES[0], '2026.2.0-123')
        self.assertEqual(data['SHA256'], 'a' * 64)

    def test_insecure_mirrors_rejected_without_network(self):
        with tempfile.TemporaryDirectory() as td, patch.object(icx.urllib.request, 'urlopen') as net:
            for url in ('http://insecure/repo', 'https://good/repo\nmalicious', 'https://good/[bad]'):
                with self.assertRaises(icx.ProvisionError):
                    icx.apt_environment(Path(td), url)
            net.assert_not_called()

    def test_apt_has_isolated_sources_and_requires_authentication(self):
        with tempfile.TemporaryDirectory() as td, patch.object(icx.urllib.request, 'urlopen') as net:
            net.return_value.__enter__.return_value.read.return_value = (
                b'-----BEGIN PGP PUBLIC KEY BLOCK-----\nfixture\n-----END PGP PUBLIC KEY BLOCK-----\n')
            work = Path(td)
            env = icx.apt_environment(work, icx.REPOSITORY)
            config = Path(env['APT_CONFIG']).read_text()
            self.assertIn('APT::Get::AllowUnauthenticated "false"', config)
            self.assertIn('Acquire::AllowInsecureRepositories "false"', config)
            self.assertIn(f'Dir::Etc "{work / "etc"}"', config)
            self.assertIn('signed-by=', (work / 'etc/sources.list').read_text())
            self.assertEqual(env['LC_ALL'], 'C')


class SnapshotPublication(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.prefix = Path(self.tmp.name) / 'snapshot'

    def fake_resolve(self, work, repository):
        binary = work / 'install/opt/compiler/bin/icx'
        binary.parent.mkdir(parents=True)
        binary.write_bytes(b'offline compiler fixture -- never executed\n')
        return {'release_line': '2026.2', 'semver': '2026.2.1', 'repository': repository,
                'packages': []}

    def test_validates_before_publication_and_reuses_snapshot(self):
        with patch.object(icx, 'resolve', side_effect=self.fake_resolve) as resolve, \
             patch.object(icx, 'probe', return_value='20260201'):
            binary = icx.install(self.prefix)
            self.assertTrue(binary.is_file())
            manifest = json.loads((self.prefix / icx.MANIFEST).read_text())
            self.assertEqual(manifest['binary_sha256'], icx.sha256(binary))
            self.assertEqual(icx.install(self.prefix), binary)
            resolve.assert_called_once()

    def test_wrong_version_does_not_publish_or_leave_staging(self):
        with patch.object(icx, 'resolve', side_effect=self.fake_resolve), \
             patch.object(icx, 'probe', return_value='20260101'):
            with self.assertRaisesRegex(icx.ProvisionError, 'disagrees'):
                icx.install(self.prefix)
        self.assertFalse(self.prefix.exists())
        self.assertEqual(list(self.prefix.parent.glob('.lccc-icx-*')), [])

    def test_missing_binary_does_not_publish(self):
        def empty(work, repository):
            data = self.fake_resolve(work, repository)
            (work / 'install/opt/compiler/bin/icx').unlink()
            return data
        with patch.object(icx, 'resolve', side_effect=empty):
            with self.assertRaisesRegex(icx.ProvisionError, 'exactly one'):
                icx.install(self.prefix)
        self.assertFalse(self.prefix.exists())

    def test_changed_snapshot_digest_is_refused(self):
        with patch.object(icx, 'resolve', side_effect=self.fake_resolve), \
             patch.object(icx, 'probe', return_value='20260201'):
            binary = icx.install(self.prefix)
            binary.write_bytes(b'tampered')
            with self.assertRaisesRegex(icx.ProvisionError, 'digest changed'):
                icx.install(self.prefix)

    def test_manifest_path_cannot_escape_prefix(self):
        self.prefix.mkdir()
        (self.prefix / icx.MANIFEST).write_text(json.dumps({
            'release_line': '2026.2', 'binary': '../outside',
        }))
        with self.assertRaisesRegex(icx.ProvisionError, 'escapes'):
            icx.validate(self.prefix)

    def test_existing_unmanaged_prefix_is_not_overwritten(self):
        self.prefix.mkdir()
        keep = self.prefix / 'keep'
        keep.write_text('keep')
        with patch.object(icx, 'resolve') as resolve:
            with self.assertRaises(OSError):
                icx.install(self.prefix)
            resolve.assert_not_called()
        self.assertEqual(keep.read_text(), 'keep')


class AuthenticatedResolution(unittest.TestCase):
    def test_download_hashes_and_exact_versions_are_enforced(self):
        for corrupt in (False, True):
            with self.subTest(corrupt=corrupt), tempfile.TemporaryDirectory() as td:
                work = Path(td)
                (work / 'etc').mkdir()
                (work / 'etc/intel.asc').write_bytes(b'key fixture')
                (work / 'state/lists').mkdir(parents=True)
                (work / 'state/lists/InRelease').write_bytes(b'signed index fixture')
                (work / 'downloads').mkdir()
                calls = []

                def fake_run(args, **kwargs):
                    calls.append(args)
                    if args[:2] == ['apt-cache', 'policy']:
                        return '  Candidate: 2026.2.1-123\n'
                    if args[:2] == ['apt-cache', 'show']:
                        name, version = args[2].split('=')
                        digest = hashlib.sha256(name.encode()).hexdigest()
                        return (f'Package: {name}\nVersion: {version}\nArchitecture: amd64\n'
                                f'Filename: pool/main/{name}.deb\nSHA256: {digest}\n')
                    if args[:2] == ['apt-get', 'download']:
                        for name in icx.PACKAGES:
                            payload = b'corrupt' if corrupt else name.encode()
                            (work / 'downloads' / f'{name}.deb').write_bytes(payload)
                    return ''

                with patch.object(icx, 'apt_environment', return_value={}), \
                     patch.object(icx, 'run', side_effect=fake_run):
                    if corrupt:
                        with self.assertRaisesRegex(icx.ProvisionError, 'missing or corrupt'):
                            icx.resolve(work, icx.REPOSITORY)
                    else:
                        data = icx.resolve(work, icx.REPOSITORY)
                        self.assertEqual(data['semver'], '2026.2.1')
                        self.assertEqual(len(data['packages']), 5)
                        download = next(c for c in calls if c[:2] == ['apt-get', 'download'])
                        self.assertTrue(all(c.endswith('=2026.2.1-123') for c in download[2:]))
                    self.assertIn(['apt-get', 'update', '--error-on=any'], calls)


if __name__ == '__main__':
    unittest.main()
