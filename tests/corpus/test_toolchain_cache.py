import io
import json
import hashlib
import re
import stat
import os
import subprocess
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from scripts import restore_toolchain_cache as cache

tarfile = cache.tarfile
REPO = Path(cache.LOCK).resolve().parents[2]
# The pinned gdb oracle version, read from the installer that enforces it, so
# the synthetic prefixes below mirror the pin instead of a second copy of it.
GDB_VERSION = re.search(r'(?m)^GDB_VERSION=([0-9.]+)$',
                        (REPO / 'scripts/ci_install_gdb_oracle.sh').read_text()).group(1)


class ToolchainCache(unittest.TestCase):
    def test_reviewed_lock_covers_every_restorable_tool(self):
        """Every tool this module restores is in the lock, and every lock
        entry it does NOT restore is a documented superseded payload.

        Clang left the lock: its pin moved to the digest-pinned upstream
        installer (23.1.5) and a patch cannot re-issue the 23.1.4 asset this
        lock describes. Asserting the exact non-restorable set (rather than
        just "VERSIONS is a subset") is what makes the exception visible: the
        day publish_toolchain_cache.py re-issues the asset and clang rejoins
        VERSIONS, this test fails and the stale note gets removed with it.
        """
        lock = json.loads(cache.LOCK.read_text())
        self.assertTrue(set(cache.VERSIONS) <= set(lock['tools']))
        self.assertEqual(set(lock['tools']) - set(cache.VERSIONS), {'clang'})
        self.assertEqual(lock['tools']['clang']['version'], '23.1.4')
        for tool in cache.VERSIONS:
            self.assertEqual(lock['tools'][tool]['version'], cache.VERSIONS[tool])
            # A restorable entry must still satisfy the full triple
            # (version, digest shape, content-addressed origin).
            cache.validate(lock['tools'][tool], tool)
        # ...and the superseded one must NOT be restorable: validate() keys
        # off VERSIONS, so this raises rather than silently trusting it.
        with self.assertRaises(KeyError):
            cache.validate(lock['tools']['clang'], 'clang')

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
                                  'clang-23.1.5/bin/clang-23': 'clang version 23.1.5 (fixture)',
                                  'bin/mold': 'mold 3.0.0',
                                  'rust-1.99.1/bin/rustc': 'rustc 1.99.1 (fixture)',
                                  # The pinned gdb oracle is part of the
                                  # offline contract now; derive its version
                                  # from the installer so this fixture cannot
                                  # drift away from the pin it mirrors.
                                  f'gdb-{GDB_VERSION}/bin/gdb': f'GNU gdb (GDB) {GDB_VERSION}'}
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

class GdbOraclePin(unittest.TestCase):
    """The pinned gdb oracle: newest-release parsing, the -march=haswell
    contract, and proof that both the freshness checks and the installer's
    own self-test actually reject a wrong pin."""

    def freshness(self):
        from scripts import check_toolchain_cache_freshness as fresh
        return fresh

    def installer_text(self):
        return (cache.LOCK.parents[2] / 'scripts/ci_install_gdb_oracle.sh').read_text()

    def test_listing_parser_picks_newest_release(self):
        fresh = self.freshness()
        listing = ('<a href="gdb-15.1.tar.xz">gdb-15.1.tar.xz</a>\n'
                   '<a href="gdb-16.3.tar.xz">gdb-16.3.tar.xz</a>\n'
                   '<a href="gdb-17.2.tar.xz">gdb-17.2.tar.xz</a>\n'
                   '<a href="gdb-18.1.tar.xz">gdb-18.1.tar.xz</a>\n'
                   '<a href="gdb-18.1.tar.xz.sig">gdb-18.1.tar.xz.sig</a>\n')
        self.assertEqual(fresh.parse_gdb_listing(listing), '18.1')
        # A lookalike major must not be mistaken for the pinned release.
        self.assertEqual(fresh.parse_gdb_listing(
            'gdb-18.1.tar.xz gdb-18.2.tar.xz'), '18.2')

    def test_pin_is_the_latest_release_and_the_source_is_digested(self):
        fresh = self.freshness()
        pins = fresh.read_pins()
        text = self.installer_text()
        self.assertEqual(pins['gdb'], '18.1')
        self.assertRegex(text, r'(?m)^GDB_SHA256=[0-9a-f]{64}$')
        # GNU publishes no checksum for gdb, so the pin must be pinned to a
        # byte size as well: a re-issued tarball of the same name is caught.
        self.assertRegex(text, r'(?m)^GDB_SIZE=[0-9]+$')
        # Two mirrors, one digest: the transport fallback is not a second
        # source of truth.
        self.assertIn('ftp.gnu.org/gnu/gdb/', text)
        self.assertIn('sourceware.org/pub/gdb/releases/', text)

    def test_haswell_is_part_of_the_contract(self):
        text = self.installer_text()
        march = re.search(r'(?m)^GDB_MARCH=(\S+)$', text).group(1)
        cflags = re.search(r'(?m)^GDB_CFLAGS="([^"]+)"$', text).group(1)
        self.assertEqual(march, 'haswell')
        self.assertIn('-march=haswell', cflags.replace('$GDB_MARCH', march))
        # The build stamp is what makes a warm prefix prove its ISA level.
        self.assertIn('march=$GDB_MARCH', text)

    def test_freshness_check_rejects_a_non_haswell_pin(self):
        """Give the check teeth: a generic-baseline pin must be a problem."""
        fresh = self.freshness()
        pins = fresh.read_pins()
        self.assertEqual(fresh.offline_problems(pins), [])
        real_read = fresh._read

        def generic_gdb(rel):
            text = real_read(rel)
            if rel.endswith('ci_install_gdb_oracle.sh'):
                text = text.replace('GDB_MARCH=haswell', 'GDB_MARCH=x86-64')
            return text

        with patch.object(fresh, '_read', generic_gdb):
            problems = fresh.offline_problems(pins)
        self.assertTrue(any('GDB_MARCH' in problem for problem in problems), problems)

    def test_installer_self_test_passes(self):
        script = cache.LOCK.parents[2] / 'scripts/ci_install_gdb_oracle.sh'
        result = subprocess.run(['/bin/bash', str(script), '--self-test'],
                                text=True, capture_output=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn('all cases passed', result.stdout)


class SupersededClangPayloads(unittest.TestCase):
    """The live Clang pin moved to 23.1.5 while two PUBLISHED payloads stay
    23.1.4 (a patch cannot re-issue them). These tests pin that state so it
    cannot rot silently in either direction."""

    def freshness(self):
        from scripts import check_toolchain_cache_freshness as fresh
        return fresh

    def test_nothing_restores_clang_from_a_superseded_payload(self):
        import importlib
        restore = importlib.import_module('scripts.restore_toolchain_cache')
        self.assertNotIn('clang', restore.VERSIONS)
        self.assertIn('rust', restore.VERSIONS)
        arena = (cache.LOCK.parents[2] / 'scripts/arena_provision.py').read_text()
        self.assertNotIn('CLANG_BLOB', arena)
        # The one live pin, stated once, in the installer CI also uses.
        installer = (cache.LOCK.parents[2] / 'scripts/ci_install_clang_oracle.sh').read_text()
        self.assertRegex(installer, r'(?m)^VERSION=23\.1\.5$')
        self.assertRegex(installer, r'(?m)^SHA256=[0-9a-f]{64}$')

    def test_the_superseded_versions_are_asserted_not_just_noted(self):
        """When the hosted publisher re-issues the payloads at 23.1.5 these
        assertions fail, forcing the exception to be retired."""
        fresh = self.freshness()
        pins = fresh.read_pins()
        self.assertEqual(pins['clang-cache-asset'], '23.1.4')
        self.assertEqual(pins['clang-agent-payload'], '23.1.4')
        self.assertEqual({name for name, _ in fresh.INFO_ROWS} &
                         {'clang-cache-asset', 'clang-agent-payload', 'clang-tag-tree'},
                         {'clang-cache-asset', 'clang-agent-payload', 'clang-tag-tree'})
        self.assertEqual(fresh.offline_problems(pins), [])

    def test_a_republished_asset_forces_the_exception_to_retire(self):
        fresh = self.freshness()
        pins = fresh.read_pins()
        real_read = fresh._read

        def republished(rel):
            text = real_read(rel)
            # The release-asset lock only. `agent-cache-lock.json` also ends
            # with 'cache-lock.json', and the offline check reads it too (the
            # retired-MSRV scan does); rewriting it here would KeyError on its
            # clang entry, which carries no url because the publisher derives
            # its payload instead of downloading one.
            if rel.endswith('/cache-lock.json'):
                lock = json.loads(text)
                lock['tools']['clang']['version'] = pins['clang']
                lock['tools']['clang']['url'] = lock['tools']['clang']['url'].replace(
                    'clang-23.1.4-', f"clang-{pins['clang']}-")
                return json.dumps(lock)
            return text

        with patch.object(fresh, '_read', republished):
            problems = fresh.offline_problems(pins)
        self.assertTrue(any('cache-lock.json' in problem for problem in problems), problems)



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
                        'kernel-relocs', 'arm-a64-xml', 'icx', 'valgrind', 'gdb'):
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
        self.assertEqual(fresh.vtuple('23.1.5'), (23, 1, 5))
        # The bump this pin just made: a point release is still newer.
        self.assertGreater(fresh.vtuple('23.1.5'), fresh.vtuple('23.1.4'))

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
                {'tag_name': 'llvmorg-23.1.5', 'prerelease': False, 'draft': False, 'assets': []}]
        version, _ = fresh.parse_llvm_releases(data)
        self.assertEqual(version, '23.1.5')

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
                       'valgrind', 'gdb', 'icx', 'arm-a64-xml',
                       'debs-bookworm-libc', 'debs-bookworm-gcc12')]
        stale_rows.append(('mold', '99.0.0', 'canned newer mold'))
        with patch.object(fresh, 'upstream_latest', return_value=stale_rows):
            self.assertEqual(fresh.main([]), 1)

    def test_agent_cache_lock_matches_canonical_pins(self):
        fresh = self.freshness()
        pins = fresh.read_pins()
        lock = json.loads(fresh.AGENT_LOCK.read_text())
        self.assertEqual(lock['tag'], 'harness-cache-7')
        for package in ('gcc', 'binutils', 'rust', 'mold', 'arm-a64-xml'):
            self.assertEqual(lock['tools'][package]['version'], pins[package])
            self.assertRegex(lock['tools'][package]['sha256'], r'^[0-9a-f]{64}$')
        # Three agent payloads are PUBLISHED bytes that a main-branch patch
        # cannot re-issue, so each is asserted against its recorded superseded
        # fact rather than the live pin -- and each fact is asserted to differ
        # from the live pin, so the exception cannot silently outlive the
        # staleness it documents.
        for package, fact in (('clang', 'clang-agent-payload'),
                              ('qemu', 'qemu-agent-payload'),
                              ('kernel-relocs', 'kernel-relocs-agent-payload')):
            self.assertEqual(lock['tools'][package]['version'], pins[fact])
            self.assertNotEqual(pins[fact], pins[package])
        # The 1.97.0 MSRV floor is retired: no lock entry, no rust-version key.
        self.assertNotIn('rust-msrv', lock['tools'])
        self.assertNotIn('rust-msrv', lock['priority'])
        self.assertNotIn('rust-version', pins)
        self.assertEqual(fresh.offline_problems(pins), [])

    def test_agent_cache_lock_drift_is_detected(self):
        fresh = self.freshness()
        pins = fresh.read_pins()
        lock = json.loads(fresh.AGENT_LOCK.read_text())
        lock['tools']['mold']['version'] = '99.0.0'
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'lock.json'
            path.write_text(json.dumps(lock))
            with patch.object(fresh, 'AGENT_LOCK', path):
                problems = fresh.offline_problems(pins)
        self.assertTrue(any('agent-cache-lock.json' in problem for problem in problems))
