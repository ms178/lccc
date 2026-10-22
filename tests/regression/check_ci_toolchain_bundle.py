#!/usr/bin/env python3
"""Contract tests for the CI toolchain bundle: export, restore, verify, fallback.

The bundle is what turns seven 80-205 s apt transactions per pipeline into one
cached restore, so its failure modes matter more than its happy path:

* a *stale* archive must not be restored by filename alone (sha256 mismatch),
* a *complete* archive missing one required tool must be refused (`verify`),
* a *broken cache* must fall back to a real install, never to a green lane,
  and must never install into a prefix it was told to restore into.

Everything here runs unprivileged: `--root` is the same code path the runner
uses with `--root /`, pointed at a temporary directory, so tar layout, hashing
and probes are exercised for real instead of being mocked.
"""
from __future__ import annotations

import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
BUNDLE = ROOT / "scripts/ci_toolchain_bundle.sh"
PROVISION = ROOT / "scripts/ci_provision_toolchain.sh"

REQUIRED = ("gcc-16", "g++-16", "ld.lld", "qemu-i386", "mold")


def build_tree(root: Path) -> list[str]:
    """A miniature published toolchain: the five tools plus a compiler blob."""
    binaries = root / "usr/bin"
    binaries.mkdir(parents=True)
    for name in REQUIRED:
        path = binaries / name
        path.write_text("#!/bin/sh\nexit 0\n")
        path.chmod(0o755)
    compiler = root / "usr/lib/gcc/16/cc1"
    compiler.parent.mkdir(parents=True)
    compiler.write_text("compiler\n")
    docs = root / "usr/share/doc/gcc-16/README"
    docs.parent.mkdir(parents=True)
    docs.write_text("documentation must not enter the bundle\n")
    listed = [f"/usr/bin/{name}" for name in REQUIRED]
    listed += ["/usr/lib/gcc/16/cc1", "/usr/share/doc/gcc-16/README",
               "/usr/bin/absent-from-the-tree"]
    listing = root.parent / "files.txt"
    listing.write_text("\n".join(listed) + "\n")
    return listed


def run(*args: str, check: bool = False) -> subprocess.CompletedProcess:
    return subprocess.run([*args], capture_output=True, text=True, check=check)


class ToolchainBundle(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = Path(tempfile.mkdtemp(prefix="lccc-bundle-"))
        self.addCleanup(shutil.rmtree, self.temp, ignore_errors=True)
        self.tree = self.temp / "tree"
        self.listing = build_tree(self.tree)
        self.bundle = self.temp / "bundle"

    def export(self, *extra: str) -> subprocess.CompletedProcess:
        return run("bash", str(BUNDLE), "export", "--bundle", str(self.bundle),
                   "--root", str(self.tree),
                   "--from-file", str(self.tree.parent / "files.txt"), *extra)

    def test_export_uses_the_declared_file_list_and_drops_non_payload(self):
        result = self.export()
        self.assertEqual(result.returncode, 0, result.stderr)
        stored = (self.bundle / "toolchain.files").read_text().split()
        # Documentation and paths absent from the tree are not payload.
        self.assertNotIn("usr/share/doc/gcc-16/README", stored)
        self.assertNotIn("usr/bin/absent-from-the-tree", stored)
        self.assertEqual(sorted(stored), sorted(
            f"usr/bin/{name}" for name in REQUIRED) + ["usr/lib/gcc/16/cc1"])
        # The archive is accompanied by the digest restore checks against.
        self.assertTrue((self.bundle / "toolchain.sha256").is_file())
        self.assertIn("toolchain.tar.gz", (self.bundle / "toolchain.sha256").read_text())

    def test_restore_reproduces_modes_and_layout(self):
        self.assertEqual(self.export().returncode, 0)
        destination = self.temp / "restored"
        result = run("bash", str(BUNDLE), "restore", "--bundle", str(self.bundle),
                     "--root", str(destination))
        self.assertEqual(result.returncode, 0, result.stderr)
        for name in REQUIRED:
            restored = destination / "usr/bin" / name
            self.assertTrue(restored.is_file(), name)
            self.assertTrue(os.access(restored, os.X_OK), name)

    def test_a_stale_archive_is_refused_instead_of_restored(self):
        self.assertEqual(self.export().returncode, 0)
        archive = self.bundle / "toolchain.tar.gz"
        archive.write_bytes(archive.read_bytes() + b"corruption")
        destination = self.temp / "restored"
        result = run("bash", str(BUNDLE), "restore", "--bundle", str(self.bundle),
                     "--root", str(destination))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("does not match its manifest", result.stderr)
        # Nothing was unpacked: a failed restore leaves no half-toolchain.
        self.assertFalse((destination / "usr/bin/gcc-16").exists())

    def test_a_bundle_missing_a_required_tool_fails_verification(self):
        self.assertEqual(self.export().returncode, 0)
        destination = self.temp / "restored"
        self.assertEqual(run("bash", str(BUNDLE), "restore", "--bundle", str(self.bundle),
                             "--root", str(destination)).returncode, 0)
        (destination / "usr/bin/ld.lld").unlink()
        result = run("bash", str(BUNDLE), "verify", "--bundle", str(self.bundle),
                     "--root", str(destination))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("/usr/bin/ld.lld", result.stderr)

    def test_provision_uses_the_cached_bundle_when_it_verifies(self):
        self.assertEqual(self.export().returncode, 0)
        destination = self.temp / "prefix"
        result = run("bash", str(PROVISION), "--bundle-dir", str(self.bundle),
                     "--root", str(destination))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("cached bundle", result.stdout)
        self.assertTrue((destination / "usr/bin/gcc-16").is_file())

    def test_a_broken_cache_falls_back_to_install_not_to_a_fake_success(self):
        self.assertEqual(self.export().returncode, 0)
        (self.bundle / "toolchain.sha256").unlink()
        destination = self.temp / "prefix"
        result = run("bash", str(PROVISION), "--bundle-dir", str(self.bundle),
                     "--root", str(destination))
        self.assertNotEqual(result.returncode, 0)
        # It says why, and it refuses to run apt against a prefix.
        self.assertIn("failed verification", result.stdout)
        self.assertIn("cannot install into", result.stderr)
        self.assertFalse((destination / "usr/bin/gcc-16").exists())


class ProvisionToolchain(unittest.TestCase):
    """The provisioning contract, exercised against a fake repo and fake tools.

    The install path cannot be run for real here (it would apt-get install onto
    the machine running the test), and the live root cannot be restored into.
    So the *sequence of operations* is what is pinned: which installer mode,
    which extras, one transaction, an exported bundle, and a restore that is
    escalated exactly when the live root is the target.

    This is the failure mode that matters.  A restore of `/` runs as
    uid 1001 on a hosted runner: if the escalation is dropped, tar fails with
    permission denied on the first file, `set -e` exits the step, and every
    lane that depends on the bundle dies -- or worse, a future change makes it
    tolerate the failure and lanes silently run without gcc-16.
    """

    def setUp(self) -> None:
        self.temp = Path(tempfile.mkdtemp(prefix="lccc-provision-"))
        self.addCleanup(shutil.rmtree, self.temp, ignore_errors=True)
        self.repo = self.temp / "repo"
        (self.repo / "scripts").mkdir(parents=True)
        shutil.copy(PROVISION, self.repo / "scripts" / PROVISION.name)
        (self.repo / "scripts" / "ci_toolchain_bundle.sh").write_text(
            "#!/bin/sh\n"
            'printf "bundle %s\\n" "$*" >> "$MOCK_LOG"\n'
            'if [ "$1" = restore ] && [ "${MOCK_RESTORE_STATUS:-0}" != 0 ]; then exit 1; fi\n'
            # `export` publishes the artifact the next run restores from; a mock
            # that did not would make every test look like a cold cache.
            'if [ "$1" = export ]; then\n'
            '  while [ $# -gt 0 ]; do [ "$1" = --bundle ] && touch "$2/toolchain.tar.gz"; shift; done\n'
            "fi\n"
            "exit 0\n")
        (self.repo / "scripts" / "ci_install_gcc16.sh").write_text(
            "#!/bin/sh\n"
            'printf "install-gcc16 %s\\n" "$*" >> "$MOCK_LOG"\n'
            'touch "$MOCK_INSTALLED"\n')
        # The differential linker oracles are installed by their own script
        # (retries, fail-closed, warm-runner-free), so provisioning delegates
        # that pair instead of folding it into the apt transaction. The mock
        # records the delegation; the contract that the pair MUST reach it is
        # pinned by check_ci_link_oracle_provisioning.py.
        (self.repo / "scripts" / "ci_install_link_oracles.sh").write_text(
            "#!/bin/sh\n"
            'printf "install-link-oracles %s\\n" "$*" >> "$MOCK_LOG"\n')
        self.bin = self.temp / "bin"
        self.bin.mkdir()
        # `sudo` records and then RUNS its command, because escalation is only
        # meaningful if the escalated program still does the work.
        (self.bin / "sudo").write_text(
            "#!/bin/sh\n"
            'printf "sudo %s\\n" "$*" >> "$MOCK_LOG"\n'
            '[ "$1" = -- ] && shift\n'
            'exec "$@"\n')
        (self.bin / "sudo").chmod(0o755)
        (self.bin / "apt-get").write_text(
            "#!/bin/sh\n"
            'printf "apt-get %s\\n" "$*" >> "$MOCK_LOG"\n'
            "exit 0\n")
        (self.bin / "apt-get").chmod(0o755)
        # `dpkg-query -W -f=...` before/after: the installer's marker adds a
        # package, which is precisely how the real delta is computed.
        (self.bin / "dpkg-query").write_text(
            "#!/bin/sh\n"
            'printf "bash 5.2\\ncoreutils 9.4\\n"\n'
            'if [ -e "$MOCK_INSTALLED" ]; then printf "gcc-16 16.2.0\\nqemu-user 8.2\\n"; fi\n'
            "exit 0\n")
        (self.bin / "dpkg-query").chmod(0o755)
        self.log = self.temp / "argv.log"
        self.bundle = self.temp / "bundle"
        self.bundle.mkdir()

    def provision(self, *args: str) -> subprocess.CompletedProcess:
        env = dict(os.environ, MOCK_LOG=str(self.log),
                   MOCK_INSTALLED=str(self.temp / "installed"),
                   PATH=f"{self.bin}:{os.environ['PATH']}")
        env.pop("LCCC_TOOLCHAIN_BUNDLE_DIR", None)
        return subprocess.run(
            ["bash", str(self.repo / "scripts" / PROVISION.name),
             "--bundle-dir", str(self.bundle), *args],
            capture_output=True, text=True, env=env, timeout=120)

    def argv(self) -> list[str]:
        return self.log.read_text().splitlines() if self.log.exists() else []

    def invocations(self, verb: str) -> list[str]:
        """Every recorded `verb` run, escalated or not (sudo logs too)."""
        return [line for line in self.argv() if f" {verb} --bundle " in line]

    def calls(self, verb: str) -> list[str]:
        """Unescalated runs of `verb`: the program the argv actually names."""
        return [line for line in self.invocations(verb) if not line.startswith("sudo -- ")]

    def escalated(self, verb: str) -> list[str]:
        return [line for line in self.invocations(verb) if line.startswith("sudo -- ")]

    def test_install_path_runs_one_apt_transaction_and_exports_a_bundle(self):
        result = self.provision("--root", "/")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        lines = self.argv()
        self.assertEqual(lines[0], "install-gcc16 --multilib")
        apt = [line for line in lines if line.startswith("apt-get")]
        self.assertEqual(len(apt), 1, lines)
        # The pair with its own retrying installer is delegated; the rest of
        # the immutable set (qemu-user: the i686 execution oracle,
        # python3-yaml: the gate parser) stays in ONE apt transaction, so a
        # lane still pays one package transaction and not two.
        self.assertEqual(apt[0].split()[1:], [
            "install", "-y", "--no-install-recommends",
            "qemu-user", "python3-yaml"])
        delegated = [line.strip() for line in lines
                     if line.startswith("install-link-oracles")]
        self.assertEqual(delegated, ["install-link-oracles"],
                         "mold+lld must reach the retrying installer exactly "
                         f"once, got: {lines}")
        exports = [line for line in lines if line.startswith("bundle export")]
        self.assertEqual(len(exports), 1, lines)
        # The exported package set is the dpkg-delta, never a hard-coded list.
        self.assertIn("--packages gcc-16 qemu-user", exports[0])
        self.assertEqual(self.calls("verify"),
                         ["bundle verify --bundle %s --root /" % self.bundle])
        # verify is a read-only probe: it must never be escalated, so a lane
        # cannot end up "verified" against a tree only root can execute.
        self.assertEqual(self.escalated("verify"), [])

    def restore_tail(self, root: str) -> str:
        """The `restore` invocation's last three arguments, for assertion."""
        return "restore --bundle %s --root %s" % (self.bundle, root)

    def test_a_second_invocation_restores_instead_of_installing(self):
        self.assertEqual(self.provision("--root", "/").returncode, 0)
        self.log.unlink()
        result = self.provision("--root", "/")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(len(self.calls("restore")), 1, self.argv())
        self.assertIn(self.restore_tail("/"), self.calls("restore")[0])
        self.assertEqual(len(self.escalated("restore")), 1, self.argv())
        self.assertIn("cached bundle", result.stdout)

    def test_the_live_root_restore_is_escalated_and_a_prefix_is_not(self):
        self.assertEqual(self.provision("--root", "/").returncode, 0)
        self.log.unlink()
        self.assertEqual(self.provision("--root", "/").returncode, 0)
        self.assertEqual(len(self.calls("restore")), 1, self.argv())
        self.assertEqual(len(self.escalated("restore")), 1, self.argv())
        self.assertIn(self.restore_tail("/"), self.calls("restore")[0])
        self.log.unlink()
        prefix = self.temp / "prefix"
        prefix.mkdir()
        self.assertEqual(self.provision("--root", str(prefix)).returncode, 0)
        self.assertEqual(len(self.calls("restore")), 1, self.argv())
        self.assertEqual(self.escalated("restore"), [], self.argv())
        self.assertIn(self.restore_tail(str(prefix)), self.calls("restore")[0])

    def test_a_failing_restore_falls_through_to_the_installer(self):
        self.assertEqual(self.provision("--root", "/").returncode, 0)
        self.log.unlink()
        env = dict(os.environ, MOCK_LOG=str(self.log), MOCK_RESTORE_STATUS="1",
                   MOCK_INSTALLED=str(self.temp / "installed"),
                   PATH=f"{self.bin}:{os.environ['PATH']}")
        result = subprocess.run(
            ["bash", str(self.repo / "scripts" / PROVISION.name),
             "--bundle-dir", str(self.bundle), "--root", "/"],
            capture_output=True, text=True, env=env, timeout=120)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("failed verification", result.stdout)
        self.assertEqual(len(self.escalated("restore")), 1, self.argv())
        self.assertIn(self.restore_tail("/"), self.escalated("restore")[0])
        self.assertIn("install-gcc16 --multilib", self.argv())

    def test_extras_are_configurable_and_an_empty_list_skips_apt(self):
        result = self.provision("--root", "/", "--extras", "")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertFalse([line for line in self.argv() if line.startswith("apt-get")])
        self.assertTrue([line for line in self.argv() if line.startswith("install-gcc16")])

    def test_a_64_bit_only_mode_is_refused_rather_than_cached(self):
        """`--base` would build a bundle that cannot link -m32.

        `verify` refuses such a bundle, so a lane that restored it would fall
        back to apt and then fail to save over the shared cache entry -- every
        lane paying the install on every run.  The flag is therefore not part
        of the provisioning interface at all.
        """
        result = self.provision("--root", "/", "--base")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("unknown argument: --base", result.stderr)
        self.assertEqual(self.argv(), [])

    def test_the_bundle_carries_the_multilib_installer_contract(self):
        self.assertEqual(self.provision("--root", "/").returncode, 0)
        self.assertEqual(self.argv()[0], "install-gcc16 --multilib")

    def test_an_unknown_argument_is_refused_before_any_work(self):
        result = self.provision("--root", "/", "--frobnicate")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("unknown argument: --frobnicate", result.stderr)
        self.assertEqual(self.argv(), [])


if __name__ == "__main__":
    unittest.main()
