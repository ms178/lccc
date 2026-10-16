#!/usr/bin/env python3
"""Exercise GCC provisioning with PATH stubs: no root, apt or network effects."""
from __future__ import annotations

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
MOCK = r'''
import json, os, pathlib, sys
name = pathlib.Path(sys.argv[0]).name
args = sys.argv[1:]
with open(os.environ['MOCK_LOG'], 'a') as f:
    f.write(json.dumps([name, *args]) + '\n')
if name == 'sudo':
    if args and args[0] == '--': args.pop(0)
    os.execvp(args[0], args)
elif name == 'uname': print('Linux')
elif name == 'dpkg-query':
    print('installed' if os.environ.get('READY') == '1' else 'not-installed')
elif name == 'apt-cache': print('  Candidate: ' + os.environ.get('CANDIDATE', '16.2.0-1'))
elif name in ('gcc-16', 'g++-16'):
    if args == ['-dumpfullversion']: print(os.environ.get('GXX_VERSION' if name == 'g++-16' else 'VERSION', '16.2.0'))
    elif os.environ.get('BANNER_FAIL') == '1': sys.exit(7)
    else: print(name + ' (mock Debian) 16.2.0\nCopyright')
elif name == 'apt-get' and os.environ.get('APT_FAIL') == '1': sys.exit(9)
elif name == 'apt-get' and os.environ.get('APT_INSTALL_FAIL') == '1' \
        and args[:1] == ['install'] and '--dry-run' not in args: sys.exit(9)
'''


class ProvisioningTests(unittest.TestCase):
    def invoke(self, mode="--base", distro="debian", uid=0, **overrides):
        with tempfile.TemporaryDirectory() as name:
            directory = Path(name)
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
            env = {**os.environ, "PATH": f"{directory}:{os.defpath}",
                   "MOCK_LOG": str(log), "MOCK_EUID": str(uid), "LCCC_CI_OS_RELEASE": str(release),
                   "LCCC_CI_GCC_VERSION": "16.2.0", "LCCC_CI_APT_RETRY_DELAY": "0",
                   **overrides}
            result = subprocess.run(["bash", str(script), mode],
                                    env=env, text=True, capture_output=True, timeout=15)
            commands = [json.loads(line) for line in log.read_text().splitlines()] if log.exists() else []
            return result, commands

    def test_distribution_first(self):
        result, commands = self.invoke()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual([c for c in commands if c[0] == "apt-get"], [
            ["apt-get", "update"],
            ["apt-get", "install", "-y", "--no-install-recommends",
             "-o", "Acquire::Retries=3", "gcc-16", "g++-16"],
        ])
        self.assertFalse(any(c[0] == "add-apt-repository" for c in commands))

    def test_root_and_nonroot_share_one_privileged_sequence(self):
        for uid in (0, 1000):
            with self.subTest(uid=uid):
                result, commands = self.invoke("--multilib", distro="ubuntu", uid=uid, CANDIDATE="(none)")
                self.assertEqual(result.returncode, 0, result.stderr)
                privileged = [c for c in commands if c[0] in ("apt-get", "dpkg", "add-apt-repository")]
                sudo = [c for c in commands if c[0] == "sudo"]
                if uid == 0:
                    self.assertFalse(sudo)
                else:
                    self.assertEqual([c[2:] for c in sudo], privileged)
                    self.assertTrue(all(c[1] == "--" for c in sudo))

    def test_multilib_retains_both_compilers_and_development_objects(self):
        result, commands = self.invoke("--multilib")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(["dpkg", "--add-architecture", "i386"], commands)
        install = next(c for c in commands if c[:2] == ["apt-get", "install"])
        for package in ("gcc-16-multilib", "g++-16-multilib", "gcc-multilib",
                        "g++-multilib", "libc6-dev-i386", "lib32gcc-16-dev"):
            self.assertIn(package, install)

    def test_ubuntu_only_fallback(self):
        result, commands = self.invoke(distro="ubuntu", CANDIDATE="(none)")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(sum(c[0] == "add-apt-repository" for c in commands), 1)
        self.assertEqual(commands.count(["apt-get", "update"]), 2)
        result, commands = self.invoke(CANDIDATE="(none)")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("refusing an Ubuntu PPA", result.stderr)
        self.assertFalse(any(c[0] == "add-apt-repository" for c in commands))

    def test_ready_toolchain_does_not_reprovision(self):
        result, commands = self.invoke(READY="1")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(any(c[0] in ("apt-get", "apt-cache", "dpkg") for c in commands))

    def test_errors_propagate(self):
        for overrides in ({"VERSION": "15.3.0"}, {"VERSION": "16.1.0"},
                          {"BANNER_FAIL": "1"}, {"APT_FAIL": "1"}):
            with self.subTest(overrides=overrides):
                result, _ = self.invoke(**overrides)
                self.assertNotEqual(result.returncode, 0)
        result, commands = self.invoke("--invalid")
        self.assertEqual(result.returncode, 2)
        self.assertEqual(commands, [])

    def test_transient_apt_failures_are_retried_then_propagate(self):
        # Hosted-runner mirrors flake; 'apt-get update' must be attempted
        # exactly three times before the failure propagates.
        result, commands = self.invoke(APT_FAIL="1")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(commands.count(["apt-get", "update"]), 3)
        self.assertIn("attempt 3/3", result.stderr)

    def test_install_failure_names_the_resolver_state(self):
        # PPA publication skew between gcc-16 and the lib32 binaries must
        # leave a self-diagnosing log: three install attempts, per-package
        # resolver state, and a --dry-run naming the broken dependency.
        result, commands = self.invoke("--multilib", APT_INSTALL_FAIL="1")
        self.assertNotEqual(result.returncode, 0)
        installs = [c for c in commands
                    if c[:2] == ["apt-get", "install"] and "--dry-run" not in c]
        self.assertEqual(len(installs), 3)
        self.assertIn("per-package resolver state", result.stderr)
        for package in ("gcc-16", "g++-16", "gcc-16-multilib", "g++-16-multilib",
                        "gcc-multilib", "g++-multilib", "libc6-dev-i386",
                        "lib32gcc-16-dev"):
            self.assertIn(["apt-cache", "policy", package], commands)
        self.assertTrue(any(c[:2] == ["apt-get", "install"] and "--dry-run" in c
                            for c in commands))

    def test_c_and_cxx_versions_must_match(self):
        for version in ("15.3.0", "16.1.0", "", "garbage"):
            with self.subTest(gxx=version):
                result, commands = self.invoke(READY="1", GXX_VERSION=version)
                self.assertNotEqual(result.returncode, 0, result.stdout)
                self.assertIn("g++-16", result.stderr)
                self.assertIn(["g++-16", "-dumpfullversion"], commands)

    def test_rolling_major_policy_is_explicit(self):
        result, _ = self.invoke(LCCC_CI_GCC_VERSION="", VERSION="16.3.0", GXX_VERSION="16.3.0")
        self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == "__main__":
    unittest.main()
