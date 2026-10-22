#!/usr/bin/env python3
"""Provision the differential linker oracles under PATH stubs: no root, apt or
network effects.

The retrying script began as a replacement for six bare apt steps in
`.github/workflows/ci.yml`. The workflow now provisions one verified toolchain
bundle instead: the retryable compiler installer owns the cold apt transaction,
this helper is a fail-closed linker-oracle postcondition/fallback, and bundle
verification covers the warm path. Hosted mirrors still answer 5xx and reset
connections transiently, so an unguarded extra apt step would turn a network
blip into a skipped compiler/test lane. The suite pins both the retry contract
and the current ownership boundary so bare workflow installs cannot return.

The stub models the apt behaviours that decide the design, both observed on
real runners:

* ``apt-get update`` can exit non-zero on a transient index fetch failure that
  a retry resolves.
* a zero exit from ``apt-get update`` does not prove a package is resolvable;
  apt warns and carries on. Resolution is therefore keyed on
  ``apt-cache pkgnames``, never on the exit status.
"""
from __future__ import annotations

import json
import os
import re
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts/ci_install_link_oracles.sh"
PROVISION = ROOT / "scripts/ci_provision_toolchain.sh"
WORKFLOW = ROOT / ".github/workflows/ci.yml"

ORACLE_PACKAGES = ("mold", "lld")
# What tests/linker/run_linker_tests.py puts on PATH once both are installed.
ORACLE_BINARIES = ("mold", "ld.lld")

MOCK = r'''
import os, pathlib, sys

state = pathlib.Path(os.environ['MOCK_STATE'])
state.mkdir(parents=True, exist_ok=True)
name = pathlib.Path(sys.argv[0]).name
args = sys.argv[1:]
with open(os.environ['MOCK_LOG'], 'a') as handle:
    handle.write(__import__('json').dumps([name, *args]) + '\n')

index_names = set(os.environ.get('INDEX_PACKAGES', 'mold lld').split())
bin_dir = pathlib.Path(os.environ['MOCK_BIN'])
mock_path = pathlib.Path(os.environ['MOCK'])


def path(relative):
    return state / relative


def bump(relative):
    target = path(relative)
    count = int(target.read_text()) if target.exists() else 0
    count += 1
    target.write_text(str(count))
    return count


def budget(key):
    value = os.environ.get(key)
    return int(value) if value else 0


def option(arguments, key):
    prefix = key + '='
    for position, value in enumerate(arguments):
        if (value == '-o' and position + 1 < len(arguments)
                and arguments[position + 1].startswith(prefix)):
            return arguments[position + 1][len(prefix):]
        if value.startswith(prefix):
            return value[len(prefix):]
    return None


def subcommand(arguments):
    # Options may precede the subcommand (`apt-cache -o KEY=VALUE pkgnames`).
    position = 0
    while position < len(arguments):
        value = arguments[position]
        if value == '-o':
            position += 2
            continue
        if value.startswith('-'):
            position += 1
            continue
        return value, arguments[position + 1:]
    return None, []


def lists_dir(arguments):
    return pathlib.Path(option(arguments, 'Dir::State::lists') or (state / 'lists'))


def published(arguments):
    """The names apt would actually resolve from the index this call reads."""
    marker = lists_dir(arguments) / '.index'
    return set(marker.read_text().split()) if marker.exists() else set()


def publish_index(arguments):
    lists = lists_dir(arguments)
    lists.mkdir(parents=True, exist_ok=True)
    (lists / '.index').write_text(' '.join(sorted(index_names)))


def install_binary(binary):
    target = bin_dir / binary
    if target.is_symlink() or target.exists():
        target.unlink()
    target.symlink_to(mock_path)


def attempt_within(limit):
    return bool(limit) and int(path('.installs').read_text()) <= limit


if name == 'sudo':
    if args and args[0] == '--':
        args.pop(0)
    os.execvp(args[0], args)
elif name == 'uname':
    print(os.environ.get('MOCK_UNAME', 'Linux'))
elif name == 'apt-cache':
    command, rest = subcommand(args)
    pattern = rest[-1] if rest else ''
    if command == 'pkgnames':
        for candidate in sorted(published(args)):
            if candidate.startswith(pattern):
                print(candidate)
elif name == 'apt-get':
    verb, _ = subcommand(args)
    if verb == 'update':
        if bump('.updates') <= budget('UPDATE_FAILURES'):
            print('E: Failed to fetch http://mirror/InRelease  503  Service Unavailable')
            sys.exit(100)
        publish_index(args)
        sys.exit(0)
    if verb == 'install':
        bump('.installs')
        missing = [package for package in ORACLE_PACKAGES
                   if package not in published(args)]
        if missing:
            for package in missing:
                print('E: Unable to locate package %s' % package)
            sys.exit(100)
        if attempt_within(budget('INSTALL_FAILURES')):
            print('E: Failed to fetch http://mirror/pool/main/m/mold/mold.deb  '
                  '503  Service Unavailable')
            sys.exit(100)
        if os.environ.get('INSTALL_LEAVES_BINARY_MISSING') != '1':
            for binary in ORACLE_BINARIES:
                install_binary(binary)
        sys.exit(0)
elif name in ('mold', 'ld.lld', 'lld'):
    print({'mold': 'mold 2.42.1 (compatible with GNU ld)',
           'ld.lld': 'LLD 23.1.3 (compatible with GNU linkers)',
           'lld': 'lld'}[name])
'''


class LinkOracleProvisioningTests(unittest.TestCase):
    def setUp(self):
        self.directory = Path(tempfile.mkdtemp(prefix="lccc-link-oracles-"))
        self.addCleanup(shutil.rmtree, self.directory, ignore_errors=True)

    def invoke(self, oracles_present=False, uid=0, **overrides):
        """Run the script against the stub. Returns (result, commands)."""
        mock = self.directory / "mock"
        mock.write_text(f"#!{sys.executable}\n"
                        + f"ORACLE_PACKAGES = {list(ORACLE_PACKAGES)!r}\n"
                        + f"ORACLE_BINARIES = {list(ORACLE_BINARIES)!r}\n"
                        + MOCK)
        mock.chmod(0o755)

        bin_dir = self.directory / "bin"
        bin_dir.mkdir()
        for command in ("sudo", "uname", "apt-get", "apt-cache"):
            (bin_dir / command).symlink_to(mock)
        # The oracles are deliberately ABSENT from PATH unless the case asks
        # for them: presence is the whole fast-path contract under test.
        if oracles_present:
            for binary in ORACLE_BINARIES:
                (bin_dir / binary).symlink_to(mock)

        # Bash's readonly EUID cannot be assigned. Change exactly the branch
        # expression in this temporary copy; never add a production switch
        # that could bypass privilege checks.
        text = SCRIPT.read_text()
        self.assertEqual(text.count("(( EUID == 0 ))"), 1)
        script = self.directory / "helper.sh"
        script.write_text(text.replace("(( EUID == 0 ))", "(( MOCK_EUID == 0 ))"))

        log = self.directory / "log"
        env = {**os.environ,
               "PATH": f"{bin_dir}:{os.defpath}",
               "MOCK_LOG": str(log),
               "MOCK_STATE": str(self.directory / "state"),
               "MOCK_BIN": str(bin_dir),
               "MOCK": str(mock),
               "MOCK_EUID": str(uid),
               "LCCC_CI_APT_RETRY_DELAY": "0",
               **overrides}
        result = subprocess.run(["bash", str(script)], env=env, text=True,
                                capture_output=True, timeout=120)
        commands = ([json.loads(line) for line in log.read_text().splitlines()]
                    if log.exists() else [])
        return result, commands

    @staticmethod
    def updates(commands):
        return [c for c in commands if c[:2] == ["apt-get", "update"]]

    @staticmethod
    def installs(commands):
        return [c for c in commands if c[:2] == ["apt-get", "install"]]

    def test_clean_install_succeeds(self):
        result, commands = self.invoke()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(self.updates(commands)), 1)
        self.assertEqual(len(self.installs(commands)), 1)
        self.assertIn("differential linker oracles ready", result.stdout)

    def test_already_installed_never_touches_the_network(self):
        """The warm-runner fast path: present oracles must cost no apt call."""
        result, commands = self.invoke(oracles_present=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual([c for c in commands if c[0] in ("apt-get", "apt-cache")],
                         [], "a warm runner must not re-provision")

    def test_transient_index_503_is_retried_until_it_resolves(self):
        """The exact failure this script exists to absorb."""
        result, commands = self.invoke(UPDATE_FAILURES="2")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(self.updates(commands)), 3)
        self.assertEqual(len(self.installs(commands)), 1,
                         "install must wait for a usable index")
        self.assertIn("failed (exit 100, attempt 1/3)", result.stderr)

    def test_transient_download_failure_is_retried(self):
        result, commands = self.invoke(INSTALL_FAILURES="1")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(self.installs(commands)), 2)

    def test_retry_budget_is_bounded(self):
        """A permanently broken mirror must fail, not hang the lane."""
        result, commands = self.invoke(UPDATE_FAILURES="99")
        self.assertEqual(result.returncode, 1)
        self.assertLessEqual(len(self.installs(commands)), 3)

    def test_unresolvable_packages_fail_closed(self):
        """An index without the oracles must not be papered over."""
        result, commands = self.invoke(INDEX_PACKAGES="gcc-16 clang-23")
        self.assertEqual(result.returncode, 1)
        self.assertIn("cannot resolve 'mold lld'", result.stderr)
        self.assertEqual(self.installs(commands), [],
                         "never install against an index that lacks the names")

    def test_apt_success_without_the_binary_fails_closed(self):
        """run_linker_tests.py skips an absent oracle, so a half install would
        silently narrow the differential coverage instead of failing."""
        result, _ = self.invoke(INSTALL_LEAVES_BINARY_MISSING="1")
        self.assertEqual(result.returncode, 1)
        self.assertIn("not on PATH", result.stderr)

    def test_non_linux_is_a_usage_error(self):
        result, commands = self.invoke(MOCK_UNAME="Darwin")
        self.assertEqual(result.returncode, 2)
        self.assertEqual(commands[0][0], "uname")
        self.assertNotIn("apt-get", [c[0] for c in commands])

    def test_bad_retry_delay_is_rejected(self):
        result, _ = self.invoke(LCCC_CI_APT_RETRY_DELAY="soon")
        self.assertEqual(result.returncode, 2)
        self.assertIn("must be a non-negative integer", result.stderr)

    def test_index_options_are_threaded_to_probe_and_install(self):
        """The resolvability probe must read the same index the install uses,
        or a cached index reads as 'unresolvable' for a package apt can
        install offline."""
        archive = self.directory / "archive"
        result, commands = self.invoke(LCCC_CI_APT_ARCHIVE_DIR=str(archive))
        self.assertEqual(result.returncode, 0, result.stderr)
        expected = f"Dir::State::lists={archive}/lists"
        probes = [c for c in commands if c[0] == "apt-cache"]
        self.assertTrue(probes, "the resolvability probe must run")
        for call in probes + self.installs(commands):
            self.assertIn("-o", call)
            self.assertIn(expected, call, f"missing index option in {call}")
        self.assertTrue((archive / "lists").is_dir())


class WorkflowUsesTheScriptTests(unittest.TestCase):
    """The current architecture caches one verified package bundle per lane.

    The retrying oracle installer belongs inside the cold provisioner, not as
    another workflow apt step; the warm path verifies the exact same binaries
    in the bundle. These checks keep that ownership boundary intact.
    """

    @classmethod
    def setUpClass(cls):
        cls.workflow = WORKFLOW.read_text()
        cls.provisioner = PROVISION.read_text()

    def test_workflow_has_no_bare_apt_install_of_the_linker_oracles(self):
        offenders = [line.strip() for line in self.workflow.splitlines()
                     if "apt-get install" in line
                     and ("mold" in line or "lld" in line)]
        self.assertEqual(offenders, [],
                         "CI must provision linkers through the cached bundle path")

    def test_ci_jobs_use_the_common_bundle_provisioner(self):
        self.assertIn("bash scripts/ci_provision_toolchain.sh --multilib",
                      self.workflow)
        self.assertNotRegex(
            self.workflow,
            r"(?m)^\\s*run:\\s*bash scripts/ci_install_link_oracles\\.sh\\b",
            "workflow jobs must not bypass bundle creation/verification")

    def test_cold_provisioner_checks_oracles_after_retryable_install(self):
        compiler_install = self.provisioner.index(
            'ci_install_gcc16.sh" --multilib --extras "$EXTRAS"')
        oracle_check = self.provisioner.index("ci_install_link_oracles.sh")
        bundle_export = self.provisioner.index('"$BUNDLE_TOOL" export')
        self.assertLess(compiler_install, oracle_check)
        self.assertLess(oracle_check, bundle_export)
        self.assertEqual(self.provisioner.count("ci_install_link_oracles.sh"), 1)

    def test_cache_key_tracks_link_oracle_installer(self):
        toolchain_keys = [line for line in self.workflow.splitlines()
                          if "key: lccc-toolchain-v1-" in line]
        self.assertGreater(len(toolchain_keys), 0)
        for line in toolchain_keys:
            self.assertIn("'scripts/ci_install_link_oracles.sh'", line,
                          "changes to the cold-path installer must invalidate each bundle key")

    def test_script_is_executable_and_parses(self):
        self.assertTrue(os.access(SCRIPT, os.X_OK), "the script must be executable")
        check = subprocess.run(["bash", "-n", str(SCRIPT)], text=True,
                               capture_output=True)
        self.assertEqual(check.returncode, 0, check.stderr)
        check = subprocess.run(["bash", "-n", str(PROVISION), "--help"],
                               text=True, capture_output=True)
        self.assertEqual(check.returncode, 0, check.stderr)


if __name__ == "__main__":
    unittest.main(verbosity=2)
