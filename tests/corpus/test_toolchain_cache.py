import io
import json
import hashlib
import stat
import os
import subprocess
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

    def test_filter_probe_checks_actual_signature_and_fails_closed(self):
        def legacy_extractall(self, path, members=None):
            pass
        with patch.object(cache.tarfile.TarFile, 'extractall', legacy_extractall):
            self.assertFalse(cache.extractall_takes_filter())
        with patch.object(cache.inspect, 'signature', side_effect=ValueError('unknown callable')):
            self.assertFalse(cache.extractall_takes_filter())

    def test_unpack_fails_closed_without_filter_keyword(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with patch.object(cache, 'extractall_takes_filter', return_value=False):
                with self.assertRaisesRegex(RuntimeError, 'filter-capable'):
                    cache.unpack(root / 'not-even-opened.tar', root / 'out')
            self.assertFalse((root / 'out').exists())

    def test_verified_install_publishes_traversable_prefix_atomically(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            downloads = root / 'dl'
            downloads.mkdir()
            archive = downloads / 'source.tar.xz'
            with tarfile.open(archive, 'w:xz') as output:
                member = tarfile.TarInfo('pack/bin/rustc')
                member.size = 2
                member.mode = 0o755
                output.addfile(member, io.BytesIO(b'ok'))
            entry = self.entry()
            entry['sha256'] = hashlib.sha256(archive.read_bytes()).hexdigest()
            entry['url'] = (f'https://github.com/ms178/lccc/releases/download/harness-cache-4/'
                            f'rust-1.99.1-{entry["sha256"]}.tar.xz')
            archive.rename(downloads / entry['url'].rsplit('/', 1)[1])
            target = root / 'rust-1.99.1'
            def verify_staged(staging, name):
                self.assertFalse(target.exists())
                self.assertEqual(stat.S_IMODE(staging.stat().st_mode), 0o700)
                self.assertEqual((staging / 'bin/rustc').read_bytes(), b'ok')
            with patch.object(cache, 'verify', side_effect=verify_staged):
                cache.restore(entry, 'rust', root, downloads)
            self.assertEqual(stat.S_IMODE(target.stat().st_mode), 0o755)
            self.assertEqual((target / '.archive-sha256').read_text().strip(), entry['sha256'])

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


class OfflineCombinedInstaller(unittest.TestCase):
    def test_verification_modes_never_transport_or_write(self):
        script = cache.LOCK.parents[2] / 'scripts/oracle_toolchains_from_tags.sh'
        for mode in ('--offline', '--verify-only'):
            for missing_rust in (False, True):
                with self.subTest(mode=mode, missing_rust=missing_rust), tempfile.TemporaryDirectory() as directory:
                    root = Path(directory)
                    prefix, guard = root / 'prefix', root / 'guard'
                    guard.mkdir()
                    marker = root / 'unexpected-transport'
                    for name in ('git', 'curl', 'tar', 'xz', 'python3'):
                        command = guard / name
                        command.write_text(f'#!/bin/sh\necho invoked > "{marker}"\nexit 99\n')
                        command.chmod(0o755)
                    identities = {'gcc-16.2.0/bin/gcc-16': 'gcc-16 (GCC) 16.2.0',
                                  'clang-23.1.4/bin/clang-23': 'clang version 23.1.4 (fixture)',
                                  'bin/mold': 'mold 3.0.0',
                                  'rust-1.99.1/bin/rustc': 'rustc 1.99.1 (fixture)'}
                    for name, identity in identities.items():
                        if missing_rust and name.endswith('rustc'):
                            continue
                        command = prefix / name
                        command.parent.mkdir(parents=True, exist_ok=True)
                        command.write_text(f'#!/bin/sh\nprintf "%s\\n" "{identity}"\n')
                        command.chmod(0o755)
                    before = sorted(str(p.relative_to(prefix)) for p in prefix.rglob('*'))
                    result = subprocess.run(['/bin/bash', str(script), mode, '--prefix', str(prefix)],
                                            env={**os.environ, 'PATH': str(guard) + ':' + os.defpath},
                                            text=True, capture_output=True)
                    self.assertEqual(result.returncode, 1 if missing_rust else 0, result.stdout + result.stderr)
                    if missing_rust:
                        self.assertIn('rustc not executable', result.stdout + result.stderr)
                    self.assertFalse(marker.exists())
                    self.assertEqual(before, sorted(str(p.relative_to(prefix)) for p in prefix.rglob('*')))


class ToolchainCacheFreshness(unittest.TestCase):
    """The freshness checker's offline half must stay green on this tree,
    and its upstream parsers must pick the newest *release* out of
    canned upstream answers (never an -rc, never a lookalike)."""

    def freshness(self):
        from scripts import check_toolchain_cache_freshness as fresh
        return fresh

    def test_offline_pins_are_internally_consistent(self):
        fresh = self.freshness()
        pins = fresh.read_pins()
        self.assertEqual(fresh.offline_problems(pins), [])
        # The packages this checker exists for (the 2026-10-27 refresh
        # found mold/qemu/kernel-relocs stale precisely because no
        # such list was asserted anywhere).
        for package in ('gcc', 'binutils', 'clang', 'rust', 'mold', 'qemu',
                        'kernel-relocs', 'arm-a64-xml', 'icx', 'valgrind'):
            self.assertIn(package, pins)

    def test_offline_main_never_touches_network(self):
        fresh = self.freshness()
        with patch.object(fresh, 'upstream_latest', side_effect=AssertionError('network')):
            self.assertEqual(fresh.main(['--offline']), 0)

    def test_version_ordering_rejects_lookalikes(self):
        fresh = self.freshness()
        self.assertGreater(fresh.vtuple('3.0.0'), fresh.vtuple('2.42.1'))
        self.assertGreater(fresh.vtuple('11.1.2'), fresh.vtuple('8.2.2'))
        self.assertGreater(fresh.vtuple('2.47'), fresh.vtuple('2.46.1'))
        self.assertEqual(fresh.vtuple('23.1.4'), (23, 1, 4))

    def test_listing_parsers_pick_newest_release(self):
        fresh = self.freshness()
        self.assertEqual(fresh.parse_gcc_listing(
            'gcc-15.3.0/ gcc-16.1.0/ gcc-16.2.0/ gcc-9.5.0/'), '16.2.0')
        self.assertEqual(fresh.parse_binutils_listing(
            'binutils-2.46.1.tar.xz binutils-2.47.tar.xz binutils-2.45.tar.bz2'), '2.47')
        self.assertEqual(fresh.parse_qemu_listing(
            'qemu-10.2.4.tar.xz qemu-11.1.2.tar.xz qemu-11.0.5.tar.xz'), '11.1.2')
        self.assertEqual(fresh.parse_valgrind_listing(
            'valgrind-3.27.1.tar.bz2 valgrind-3.28.0.tar.bz2'), '3.28.0')

    def test_llvm_parser_skips_prereleases(self):
        fresh = self.freshness()
        data = [{'tag_name': 'llvmorg-24.1.0-rc1', 'prerelease': True, 'draft': False},
                {'tag_name': 'llvmorg-23.1.4', 'prerelease': False, 'draft': False, 'assets': []}]
        version, _ = fresh.parse_llvm_releases(data)
        self.assertEqual(version, '23.1.4')

    def test_icx_and_madison_parsers(self):
        fresh = self.freshness()
        packages = ('Package: intel-oneapi-compiler-dpcpp-cpp-runtime-2026.1\n'
                    'Package: intel-oneapi-compiler-dpcpp-cpp-runtime-2026.2\n'
                    'Package: intel-oneapi-compiler-dpcpp-cpp-runtime\n')
        self.assertEqual(fresh.parse_icx_packages(packages), '2026.2')
        madison = (' libc6 | 2.36-9+deb12u7  | bookworm-security | amd64\n'
                   ' libc6 | 2.36-9+deb12u14 | bookworm          | amd64\n'
                   ' libc6 | 2.43-7          | sid               | amd64\n')
        self.assertEqual(fresh.parse_madison(madison, 'bookworm'), '2.36-9+deb12u14')

    def test_stale_pin_is_reported_by_main(self):
        fresh = self.freshness()
        pins = fresh.read_pins()
        stale_rows = [(name, pins[name], 'canned') for name in
                      ('gcc', 'binutils', 'clang', 'rust', 'qemu', 'kernel-relocs',
                       'valgrind', 'icx', 'arm-a64-xml', 'debs-bookworm-libc', 'debs-bookworm-gcc12')]
        stale_rows.append(('mold', '99.0.0', 'canned newer mold'))
        with patch.object(fresh, 'upstream_latest', return_value=stale_rows):
            self.assertEqual(fresh.main([]), 1)
