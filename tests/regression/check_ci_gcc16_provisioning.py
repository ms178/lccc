#!/usr/bin/env python3
"""Exercise GCC provisioning with PATH stubs: no root, apt or network effects.

The stub is a small model of the apt behaviours that decide this script's
design, both of which were observed on a hosted runner:

* ``apt-get update`` exits 0 while an individual repository index failed to
  download.  The Launchpad PPA answers ``503 Service Unavailable`` under load
  and apt only warns, so a zero exit status is not evidence that ``gcc-16`` is
  resolvable.
* ``apt-cache policy <name>`` falls back to *regex* matching when the exact
  name is unknown, so asking about ``gcc-16`` reports the unrelated
  ``clang-16`` candidate.  Trusting that answer skips the PPA entirely.

``apt-get install`` rejects every package that is not in the modelled index,
exactly like the real resolver.  A provisioning script that starts from an
unusable index therefore fails this suite instead of passing it.
"""
from __future__ import annotations

import json
import os
import shutil
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]

BASE_PACKAGES = ("gcc-16", "g++-16")
MULTILIB_PACKAGES = BASE_PACKAGES + (
    "gcc-16-multilib",
    "g++-16-multilib",
    "gcc-multilib",
    "g++-multilib",
    "libc6-dev-i386",
    "lib32gcc-16-dev",
)
ORACLE_PACKAGES = ("qemu-user", "mold", "lld", "python3-yaml", "valgrind")

MOCK = r'''
import json, os, pathlib, re, sys

state = pathlib.Path(os.environ['MOCK_STATE'])
state.mkdir(parents=True, exist_ok=True)
name = pathlib.Path(sys.argv[0]).name
args = sys.argv[1:]
with open(os.environ['MOCK_LOG'], 'a') as handle:
    handle.write(json.dumps([name, *args]) + '\n')

distro = set(os.environ.get(
    'DISTRO_INDEX', 'gcc-multilib g++-multilib libc6-dev-i386 clang-16').split())
distro |= set(os.environ.get(
    'EXTRA_INDEX', 'qemu-user mold lld python3-yaml valgrind').split())
ppa = set(os.environ.get(
    'PPA_INDEX',
    'gcc-16 g++-16 gcc-16-multilib g++-16-multilib lib32gcc-16-dev').split())
if os.environ.get('DISTRO_HAS_GCC16') == '1':
    distro |= ppa
requested = os.environ.get('REQUESTED', '').split()


def path(relative):
    return state / relative


def bump(relative):
    target = path(relative)
    count = int(target.read_text()) if target.exists() else 0
    count += 1
    target.write_text(str(count))
    return count


def budget(relative):
    value = os.environ.get(relative)
    return int(value) if value else 0


def option(arguments, key):
    prefix = key + '='
    for index_, value in enumerate(arguments):
        if value == '-o' and index_ + 1 < len(arguments) and arguments[index_ + 1].startswith(prefix):
            return arguments[index_ + 1][len(prefix):]
        if value.startswith(prefix):
            return value[len(prefix):]
    return None


def subcommand(arguments):
    # Options may precede the subcommand (`apt-cache -o KEY=VALUE pkgnames`).
    index_ = 0
    while index_ < len(arguments):
        value = arguments[index_]
        if value == '-o':
            index_ += 2
            continue
        if value.startswith('-'):
            index_ += 1
            continue
        return value, arguments[index_ + 1:]
    return None, []


def lists_dir(arguments):
    # apt keeps its package index in Dir::State::lists; the default (unused
    # here) directory models a runner whose lists were never populated.
    return pathlib.Path(option(arguments, 'Dir::State::lists') or (state / 'lists'))


def index(arguments):
    names = set(distro)
    if (lists_dir(arguments) / '.ppa-index').exists():
        names |= ppa
    return names


def installed():
    target = path('.installed')
    return set(json.loads(target.read_text())) if target.exists() else set()


def mark_installed(names):
    path('.installed').write_text(json.dumps(sorted(set(names) | installed())))


if name == 'sudo':
    if args and args[0] == '--':
        args.pop(0)
    os.execvp(args[0], args)
elif name == 'uname':
    print('Linux')
elif name == 'dpkg':
    if args[:2] == ['--add-architecture', 'i386']:
        path('.i386').touch()
    elif '-i' in args:
        # `dpkg -i <file>.deb` needs no index: the package name is the part of
        # the filename before the version and the architecture.
        names = {pathlib.Path(value).name.rsplit('_', 2)[0]
                 for value in args if value.endswith('.deb')}
        bump('.dpkg-installs')
        mark_installed(names)
elif name == 'dpkg-query':
    package = args[-1]
    print('installed' if os.environ.get('READY') == '1' or package in installed()
          else 'not-installed')
elif name == 'apt-cache':
    command, rest = subcommand(args)
    pattern = rest[-1] if rest else ''
    known = index(args)
    if command == 'pkgnames':
        for candidate in sorted(known):
            if candidate.startswith(pattern):
                print(candidate)
    elif command == 'policy':
        # Real apt: exact name first, regex fallback for unknown names.
        matches = [pattern] if pattern in known else sorted(
            candidate for candidate in known if re.search(pattern, candidate))
        for candidate in matches:
            print('%s:\n  Installed: (none)\n  Candidate: 16.2.0-1\n' % candidate)
elif name == 'apt-get':
    verb, _ = subcommand(args)
    if verb == 'update':
        attempt = bump('.updates')
        if attempt <= budget('UPDATE_EXIT_FAILURES'):
            print('E: Failed to fetch http://mirror/InRelease  503  Service Unavailable')
            sys.exit(100)
        lists = lists_dir(args)
        if path('.ppa').exists() and not (lists / '.ppa-index').exists():
            ppa_attempt = bump('.ppa-updates')
            if ppa_attempt <= budget('PPA_INDEX_FAILURES'):
                # Observed on the runner: apt warns, then exits 0.
                print('W: Failed to fetch https://ppa.launchpadcontent.net/'
                      'ubuntu-toolchain-r/test/ubuntu/dists/noble/InRelease  '
                      '503  Service Unavailable')
                sys.exit(0)
            lists = lists_dir(args)
            lists.mkdir(parents=True, exist_ok=True)
            (lists / '.ppa-index').touch()
        sys.exit(0)
    if verb == 'install':
        missing = [package for package in requested if package not in index(args)]
        if budget('APT_INSTALL_FAILURES') and bump('.installs') <= budget('APT_INSTALL_FAILURES'):
            print('E: Failed to fetch http://ppa/pool/main/g/gcc-16/gcc-16.deb  '
                  '503  Service Unavailable')
            sys.exit(100)
        if os.environ.get('APT_INSTALL_FAIL') == '1':
            bump('.installs')
            print('E: Sub-process /usr/bin/dpkg returned an error code (1)')
            sys.exit(100)
        if missing:
            bump('.installs')
            for package in missing:
                print('E: Unable to locate package %s' % package)
            sys.exit(100)
        if '--dry-run' not in args:
            bump('.installs')
            omitted = os.environ.get('APT_OMIT_PACKAGE')
            mark_installed([package for package in requested if package != omitted])
        sys.exit(0)
elif name == 'add-apt-repository':
    path('.ppa').touch()
    print('Adding repository.')
elif name in ('gcc-16', 'g++-16'):
    installed_now = path('.installed').exists()
    version = os.environ.get(
        'UPGRADED_VERSION' if installed_now else 'VERSION',
        os.environ.get('VERSION', '16.2.0'))
    if args == ['-dumpfullversion']:
        print(os.environ.get('GXX_VERSION', version) if name == 'g++-16' else version)
    elif '-m32' in args:
        sys.stdin.read()
        if os.environ.get('M32_FAIL') == '1':
            sys.exit(1)
        pathlib.Path(args[args.index('-o') + 1]).write_bytes(b'')
    elif os.environ.get('BANNER_FAIL') == '1':
        sys.exit(7)
    else:
        print(name + ' (mock Debian) 16.2.0\nCopyright')
'''


class ProvisioningTests(unittest.TestCase):
    def sandbox(self):
        """A temp directory that outlives invoke(), so state stays assertable."""
        directory = Path(tempfile.mkdtemp(prefix="lccc-gcc16-"))
        self.addCleanup(shutil.rmtree, directory, ignore_errors=True)
        return directory

    def invoke(self, mode="--base", distro="ubuntu", uid=0,
               extras=None, print_packages=False, **overrides):
        requested = tuple(BASE_PACKAGES if mode == "--base" else MULTILIB_PACKAGES)
        if extras is not None:
            requested += tuple(extras)
        directory = self.sandbox()
        if True:
            mock = directory / "mock"
            mock.write_text(f"#!{sys.executable}\n" + MOCK)
            mock.chmod(0o755)
            for command in ("sudo", "uname", "dpkg-query", "dpkg", "apt-get",
                            "apt-cache", "add-apt-repository", "gcc-16", "g++-16"):
                (directory / command).symlink_to(mock)
            release = directory / "os-release"
            release.write_text(f"ID={distro}\n")
            # Bash's readonly EUID cannot be assigned. Change exactly the
            # branch expression in this temporary copy; never add a production
            # switch that could bypass privilege checks.
            text = (ROOT / "scripts/ci_install_gcc16.sh").read_text()
            self.assertEqual(text.count("(( EUID == 0 ))"), 1)
            script = directory / "helper.sh"
            script.write_text(text.replace("(( EUID == 0 ))", "(( MOCK_EUID == 0 ))"))
            log = directory / "log"
            env = {**os.environ,
                   "PATH": f"{directory}:{os.defpath}",
                   "MOCK_LOG": str(log),
                   "MOCK_STATE": str(directory / "state"),
                   "MOCK_EUID": str(uid),
                   "LCCC_CI_OS_RELEASE": str(release),
                   "LCCC_CI_GCC_VERSION": "16.2.0",
                   "LCCC_CI_APT_RETRY_DELAY": "0",
                   "REQUESTED": " ".join(requested),
                   **overrides}
            command = ["bash", str(script), mode]
            if extras is not None:
                command.extend(("--extras", " ".join(extras)))
            if print_packages:
                command.append("--print-packages")
            result = subprocess.run(command, env=env, text=True,
                                    capture_output=True, timeout=60)
            commands = ([json.loads(line) for line in log.read_text().splitlines()]
                        if log.exists() else [])
            state = directory / "state"
            return result, commands, state

    @staticmethod
    def installs(commands):
        return [c for c in commands if c[:2] == ["apt-get", "install"] and "--dry-run" not in c]

    @staticmethod
    def updates(commands):
        return [c for c in commands if c[:2] == ["apt-get", "update"]]

    def test_distribution_first(self):
        result, commands, _ = self.invoke(DISTRO_HAS_GCC16="1")
        self.assertEqual(result.returncode, 0, result.stderr)
        # A pre-seeded index that already resolves every requested package is
        # served without a network round trip; the version contract below is
        # what decides whether the local index is good enough.
        self.assertEqual(self.updates(commands), [])
        self.assertEqual(len(self.installs(commands)), 1)
        self.assertFalse(any(c[0] == "add-apt-repository" for c in commands))

    def test_all_oracles_share_one_multilib_install_transaction(self):
        requested = MULTILIB_PACKAGES + ORACLE_PACKAGES
        result, commands, _ = self.invoke(
            "--multilib", extras=ORACLE_PACKAGES, DISTRO_HAS_GCC16="1")
        self.assertEqual(result.returncode, 0, result.stderr)
        installs = self.installs(commands)
        self.assertEqual(len(installs), 1, commands)
        for package in requested:
            self.assertIn(package, installs[0])
        self.assertFalse(any(c[0] == "add-apt-repository" for c in commands))

    def test_print_packages_reports_the_exact_multilib_request_without_installing(self):
        result, commands, _ = self.invoke(
            "--multilib", extras=ORACLE_PACKAGES, print_packages=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            result.stdout.splitlines(), list(MULTILIB_PACKAGES + ORACLE_PACKAGES))
        self.assertFalse(any(c[0] in {
            "sudo", "dpkg", "apt-get", "apt-cache", "add-apt-repository",
            "gcc-16", "g++-16",
        } for c in commands), commands)

    def test_zero_status_install_missing_an_oracle_package_fails_closed(self):
        result, commands, _ = self.invoke(
            "--multilib", extras=ORACLE_PACKAGES, DISTRO_HAS_GCC16="1",
            APT_OMIT_PACKAGE="mold")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(len(self.installs(commands)), 1)
        self.assertIn("the install itself failed or left packages uninstalled: mold",
                      result.stderr)

    def test_missing_oracle_names_are_exact_and_never_trigger_the_gcc_ppa(self):
        # apt-cache pkgnames returns prefix matches too. The final exact-name
        # filter must reject both near-miss packages instead of accepting them
        # as mold/lld or adding the GCC-specific PPA for unrelated oracles.
        result, commands, _ = self.invoke(
            "--multilib", extras=ORACLE_PACKAGES, DISTRO_HAS_GCC16="1",
            EXTRA_INDEX="mold-tools lld-23 qemu-user python3-yaml valgrind")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("mold", result.stderr)
        self.assertIn("lld", result.stderr)
        self.assertFalse(any(c[0] == "add-apt-repository" for c in commands))
        self.assertEqual(self.installs(commands), [])

    def test_ppa_fallback_only_when_needed(self):
        result, commands, _ = self.invoke()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(sum(c[0] == "add-apt-repository" for c in commands), 1)
        # One refresh proves the distribution cannot resolve gcc-16, one
        # follows the PPA configuration. No retry budget is spent guessing.
        self.assertEqual(len(self.updates(commands)), 2)
        self.assertEqual(len(self.installs(commands)), 1)

    def test_ppa_index_503_is_retried_until_the_packages_resolve(self):
        # The exact runner failure: apt exits 0 while the PPA index 503s.
        # Provisioning must keep refreshing instead of trusting the exit code.
        result, commands, _ = self.invoke(PPA_INDEX_FAILURES="2")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertGreaterEqual(len(self.updates(commands)), 3)
        self.assertIn("unresolvable after index refresh", result.stderr)
        installs = self.installs(commands)
        self.assertEqual(len(installs), 1, "install must wait for a usable index")
        first_install = commands.index(installs[0])
        last_update = max(commands.index(update) for update in self.updates(commands))
        self.assertLess(last_update, first_install)

    def test_ppa_flap_beyond_the_original_window_is_still_survived(self):
        # The 2026-10-21 incident outlived the original four-round window, so
        # a live index must still be preferred -- and reached -- over the
        # offline fallback after more than four failed refreshes.
        result, commands, _ = self.invoke(PPA_INDEX_FAILURES="5")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(self.installs(commands)), 1)
        self.assertGreaterEqual(len(self.updates(commands)), 6)

    def test_unresolvable_index_fails_before_any_install_attempt(self):
        # The old script replayed `apt-get install` three times against an
        # index that never contained gcc-16: three guaranteed-to-fail
        # operations, ~75 s of sleeps, no diagnosis.
        result, commands, _ = self.invoke("--multilib", PPA_INDEX_FAILURES="99")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.installs(commands), [])
        self.assertIn("not resolvable from the configured repositories", result.stderr)
        for package in ("gcc-16", "g++-16", "gcc-16-multilib", "g++-16-multilib",
                        "lib32gcc-16-dev"):
            self.assertIn(package, result.stderr)
        # The regex flood that buried the diagnosis in unrelated packages is
        # gone: no per-package `apt-cache policy` probe at all.
        self.assertFalse(any(c[0] == "apt-cache" and "policy" in c for c in commands))

    def test_exact_presence_probe_ignores_the_regex_decoy(self):
        # Guard the model first: real apt reports clang-16's candidate when
        # asked about gcc-16. A script keyed on that answer skips the PPA.
        result, commands, _ = self.invoke()
        self.assertEqual(result.returncode, 0, result.stderr)
        probes = [c for c in commands if c[0] == "apt-cache" and "pkgnames" in c]
        self.assertTrue(probes, "presence must be probed with exact package names")
        self.assertTrue(any(c[-1] == "gcc-16" for c in probes))

    def test_hard_update_failure_is_retried_then_propagates(self):
        result, commands, _ = self.invoke(UPDATE_EXIT_FAILURES="99")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("attempt 3/3", result.stderr)
        self.assertEqual(self.installs(commands), [])
        self.assertIn("not resolvable from the configured repositories", result.stderr)

    def test_transient_index_failure_recovers_without_replaying_install(self):
        result, commands, _ = self.invoke(UPDATE_EXIT_FAILURES="2")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(self.installs(commands)), 1)
        self.assertGreaterEqual(len(self.updates(commands)), 3)

    def test_flapping_ppa_edge_is_survived_within_the_retry_budget(self):
        # 2026-10-21: probes a minute apart observed 503 and then a healthy
        # InRelease. Three consecutive failed refreshes must still succeed,
        # and no install may be attempted before the index resolves.
        result, commands, _ = self.invoke(PPA_INDEX_FAILURES="3")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(self.installs(commands)), 1)
        self.assertGreaterEqual(len(self.updates(commands)), 4)

    def test_index_cache_keeps_apt_lists_next_to_the_debs(self):
        # A warm runner must be able to resolve gcc-16 without fetching the
        # index at all; that is what makes provisioning survive a long CDN
        # outage. The cache lives beside the debs, so the existing weekly
        # actions/cache entry persists it.
        with tempfile.TemporaryDirectory() as outer:
            archive = Path(outer) / "apt-archive"
            result, commands, _ = self.invoke(LCCC_CI_APT_ARCHIVE_DIR=str(archive))
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertTrue((archive / "lists" / "partial").is_dir())
            option = f"Dir::State::lists={archive}/lists"
            for command in commands:
                if command[:2] in (["apt-get", "update"], ["apt-get", "install"]):
                    self.assertIn(option, command, command)

    def test_warm_index_provisions_with_the_cdn_dead(self):
        # The point of caching apt's index: once a run has populated it, a
        # later run must resolve and install gcc-16 with the PPA CDN
        # unreachable. The probe and the install must agree on which index
        # they read -- a probe that reads the default directory reports
        # "unresolvable" for a package apt can install offline.
        with tempfile.TemporaryDirectory() as outer:
            archive = Path(outer) / "apt-archive"
            first, _, _ = self.invoke(LCCC_CI_APT_ARCHIVE_DIR=str(archive))
            self.assertEqual(first.returncode, 0, first.stderr)
            result, commands, _ = self.invoke(LCCC_CI_APT_ARCHIVE_DIR=str(archive),
                                              PPA_INDEX_FAILURES="99")
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(self.updates(commands), [], "a warm index needs no network")
            self.assertEqual(len(self.installs(commands)), 1)
            self.assertIn(f"Dir::State::lists={archive}/lists", self.installs(commands)[0])

    def test_presence_probe_uses_the_same_index_as_the_install(self):
        with tempfile.TemporaryDirectory() as outer:
            archive = Path(outer) / "apt-archive"
            _, commands, _ = self.invoke(LCCC_CI_APT_ARCHIVE_DIR=str(archive),
                                         DISTRO_HAS_GCC16="1")
            probes = [c for c in commands if c[0] == 'apt-cache' and 'pkgnames' in c]
            self.assertTrue(probes)
            for probe in probes:
                self.assertIn(f"Dir::State::lists={archive}/lists", probe, probe)
            # -o pairs must precede the subcommand for apt-cache.
            self.assertEqual(probes[0][1], '-o')

    def test_install_failure_with_resolvable_packages_reports_the_verdict(self):
        result, commands, _ = self.invoke("--multilib", APT_INSTALL_FAIL="1")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(len(self.installs(commands)), 3)
        self.assertIn("the install itself failed", result.stderr)
        self.assertIn("resolver verdict follows", result.stderr)
        self.assertTrue(any(c[:2] == ["apt-get", "install"] and "--dry-run" in c
                            for c in commands))

    def test_root_and_nonroot_share_one_privileged_sequence(self):
        for uid in (0, 1000):
            with self.subTest(uid=uid):
                result, commands, _ = self.invoke("--multilib", uid=uid)
                self.assertEqual(result.returncode, 0, result.stderr)
                privileged = [c for c in commands
                              if c[0] in ("apt-get", "dpkg", "add-apt-repository")]
                sudo = [c for c in commands if c[0] == "sudo"]
                if uid == 0:
                    self.assertEqual(sudo, [])
                else:
                    self.assertEqual([c[2:] for c in sudo], privileged)
                    self.assertTrue(all(c[1] == "--" for c in sudo))

    def test_multilib_retains_both_compilers_and_development_objects(self):
        result, commands, state = self.invoke("--multilib")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(["dpkg", "--add-architecture", "i386"], commands)
        self.assertTrue((state / ".i386").exists())
        install = self.installs(commands)[0]
        for package in ("gcc-16-multilib", "g++-16-multilib", "gcc-multilib",
                        "g++-multilib", "libc6-dev-i386", "lib32gcc-16-dev"):
            self.assertIn(package, install)

    def test_multilib_link_probe_fails_closed(self):
        # A broken 32-bit runtime must fail here, not as dozens of unrelated
        # i686 differential failures forty minutes later.
        result, _, _ = self.invoke("--multilib", M32_FAIL="1")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("i686 multilib is incomplete", result.stderr)
        result, _, _ = self.invoke(M32_FAIL="1")
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_ready_toolchain_does_not_reprovision(self):
        result, commands, _ = self.invoke(READY="1")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(any(c[0] in ("apt-get", "apt-cache", "dpkg",
                                      "add-apt-repository", "sudo") for c in commands))

    def test_installed_wrong_minor_is_upgraded_not_mistaken_for_ready(self):
        result, commands, _ = self.invoke(READY="1", VERSION="16.1.0",
                                          UPGRADED_VERSION="16.2.0")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(self.installs(commands))

    def test_errors_propagate(self):
        for overrides in ({"VERSION": "15.3.0"}, {"VERSION": "16.1.0"},
                          {"BANNER_FAIL": "1"}, {"APT_INSTALL_FAIL": "1"}):
            with self.subTest(overrides=overrides):
                result, _, _ = self.invoke(**overrides)
                self.assertNotEqual(result.returncode, 0)
        result, commands, _ = self.invoke("--invalid")
        self.assertEqual(result.returncode, 2)
        self.assertEqual(commands, [])

    @staticmethod
    def write_cached_debs(archive, packages):
        """Populate an archive directory the way a previous successful run does."""
        archive.mkdir(parents=True, exist_ok=True)
        for package in packages:
            (archive / f"{package}_16.2.0-1_amd64.deb").write_bytes(b"deb")

    def test_index_outage_installs_from_the_verified_deb_cache(self):
        # 2026-10-21: the PPA index outlived the refresh window twice and took
        # whole lanes down. The debs from a previous verified run are already
        # on disk and dpkg needs no index, so the outage must cost a fetch,
        # not the lane -- and the result still has to pass the version
        # contract.
        with tempfile.TemporaryDirectory() as outer:
            archive = Path(outer) / "apt-archive"
            self.write_cached_debs(archive, MULTILIB_PACKAGES + ORACLE_PACKAGES)
            result, commands, _ = self.invoke(
                "--multilib", extras=ORACLE_PACKAGES,
                LCCC_CI_APT_ARCHIVE_DIR=str(archive),
                PPA_INDEX_FAILURES="99", GITHUB_ACTIONS="true")
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(self.installs(commands), [], "no index, no apt install")
            self.assertTrue(any(c[0] == "dpkg" and "-i" in c for c in commands), commands)
            self.assertIn("::warning title=GCC 16 provisioned from the deb cache",
                          result.stdout)
            self.assertNotIn("::error", result.stdout)

    def test_deb_cache_that_does_not_cover_the_request_still_fails_closed(self):
        # A cache missing one requested name proves nothing about that name;
        # the lane must fail exactly as before instead of installing a
        # partial toolchain.
        with tempfile.TemporaryDirectory() as outer:
            archive = Path(outer) / "apt-archive"
            self.write_cached_debs(archive, MULTILIB_PACKAGES + ORACLE_PACKAGES[:-1])
            result, commands, _ = self.invoke(
                "--multilib", extras=ORACLE_PACKAGES,
                LCCC_CI_APT_ARCHIVE_DIR=str(archive),
                PPA_INDEX_FAILURES="99", GITHUB_ACTIONS="true")
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(any(c[0] == "dpkg" and "-i" in c for c in commands), commands)
            self.assertIn("not resolvable from the configured repositories", result.stderr)
            self.assertIn("::error", result.stdout)

    def test_deb_cache_is_untouched_while_the_index_resolves(self):
        with tempfile.TemporaryDirectory() as outer:
            archive = Path(outer) / "apt-archive"
            self.write_cached_debs(archive, BASE_PACKAGES)
            result, commands, _ = self.invoke(LCCC_CI_APT_ARCHIVE_DIR=str(archive))
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(len(self.installs(commands)), 1)
            self.assertFalse(any(c[0] == "dpkg" and "-i" in c for c in commands), commands)

    def test_non_numeric_retry_delay_is_rejected(self):
        result, commands, _ = self.invoke(LCCC_CI_APT_RETRY_DELAY="soon")
        self.assertEqual(result.returncode, 2)
        self.assertIn("non-negative integer", result.stderr)
        self.assertFalse(any(c[0] == "apt-get" for c in commands))

    def test_ubuntu_only_fallback(self):
        result, commands, _ = self.invoke(distro="debian")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("refusing an Ubuntu PPA", result.stderr)
        self.assertFalse(any(c[0] == "add-apt-repository" for c in commands))

    def test_literal_gxx_diagnostics_do_not_report_clang(self):
        apt = shutil.which("apt-cache")
        if apt is None:
            self.skipTest("APT policy reader unavailable")
        # Read only an isolated synthetic status database: no system package
        # state, writes, root privileges, apt update/install or network calls.
        with tempfile.TemporaryDirectory() as name:
            directory = Path(name)
            status = directory / "status"
            status.write_text("Package: clang-16\nStatus: install ok installed\n"
                              "Architecture: amd64\nVersion: 16.0.6-1\n"
                              "Description: Isolated regex fixture\n\n")
            lists = directory / "lists"
            lists.mkdir()
            command = [apt, "-o", f"Dir::State::status={status}",
                       "-o", f"Dir::State::lists={lists}",
                       "-o", "Dir::Cache::pkgcache=", "-o", "Dir::Cache::srcpkgcache=", "policy"]
            broad = subprocess.run(command + ["g++-16"], capture_output=True, text=True, check=True)
            exact = subprocess.run(command + ["?exact-name(g++-16)"], capture_output=True, text=True, check=True)
            # Old APT releases expose Clang via the deprecated broad regex.
            # New releases may remove that fallback; literal lookup must work
            # in either case and must never select Clang.
            self.assertEqual(broad.returncode, 0)
            self.assertNotIn("clang-16:", exact.stdout)

    def test_c_and_cxx_versions_must_match(self):
        for version in ("15.3.0", "16.1.0", "", "garbage"):
            with self.subTest(gxx=version):
                result, commands, _ = self.invoke(READY="1", GXX_VERSION=version)
                self.assertNotEqual(result.returncode, 0, result.stdout)
                self.assertIn("g++-16", result.stderr)
                self.assertIn(["g++-16", "-dumpfullversion"], commands)

    def test_rolling_major_policy_is_explicit(self):
        result, _, _ = self.invoke(LCCC_CI_GCC_VERSION="", VERSION="16.3.0",
                                   GXX_VERSION="16.3.0")
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_archive_dir_keeps_debs_for_the_workflow_cache(self):
        with tempfile.TemporaryDirectory() as outer:
            archive = Path(outer) / "apt-archive"
            result, commands, _ = self.invoke(LCCC_CI_APT_ARCHIVE_DIR=str(archive))
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertTrue((archive / "partial").is_dir())
            install = self.installs(commands)[0]
            self.assertIn(f"Dir::Cache::archives={archive}", install)
            self.assertIn("APT::Keep-Downloaded-Packages=true", install)
            # apt update must not be redirected: the index stays live so a
            # stale deb cache can never pin an outdated toolchain.
            for update in self.updates(commands):
                self.assertNotIn(f"Dir::Cache::archives={archive}", update)
                # Translation indexes are pure download cost here and are a
                # recurring 503 source; never fetch them.
                self.assertIn("Acquire::Languages=none", update)


if __name__ == "__main__":
    unittest.main()
