#!/usr/bin/env python3
"""Contract tests for the CI toolchain bundle: export, restore, verify, fallback.

The bundle is what turns seven 80-205 s apt transactions per pipeline into one
cached restore, so its failure modes matter more than its happy path:

* a *stale* archive must not be restored by filename alone (sha256 mismatch),
* a *complete* archive missing one required tool must be refused (`verify`),
* a *broken cache* must fall back to a real install, never to a green lane,
  and must never install into a prefix it was told to restore into,
* a bundle built on *another runner image* must never be unpacked -- it is a
  package delta against a baseline this runner does not have -- yet the package
  files it carries must be what the reinstall starts from.

The last one is the failure of run 44345167051: the cache key named no image,
so image 20261116 restored a bundle built ten days earlier, its Memcheck probe
failed against a loader the bundle's debug symbols did not describe, and a
30-minute lane spent 1733 s fetching the toolchain from the Launchpad PPA.

Everything here runs unprivileged: `--root` is the same code path the runner
uses with `--root /`, pointed at a temporary directory, so tar layout, hashing
and probes are exercised for real instead of being mocked.
"""
from __future__ import annotations

import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import unittest

import yaml

ROOT = Path(__file__).resolve().parents[2]
BUNDLE = ROOT / "scripts/ci_toolchain_bundle.sh"
PROVISION = ROOT / "scripts/ci_provision_toolchain.sh"
KEY = ROOT / "scripts/ci_toolchain_key.sh"

# `valgrind` is in the REQUIRED set for the same reason the compilers are: the
# contract lanes run Memcheck oracles that REFUSE to run without it, so a bundle
# that dropped it would restore a lane whose gates cannot pass.
REQUIRED = ("gcc-16", "g++-16", "ld.lld", "qemu-i386", "mold", "valgrind")


def build_tree(root: Path) -> list[str]:
    """A miniature published toolchain: the six tools plus a compiler blob."""
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


def current_key() -> str:
    """The identity `ci_provision_toolchain.sh` computes on this machine."""
    return run("bash", str(KEY), check=True).stdout.strip()


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

    def test_a_bundle_without_valgrind_is_refused_by_name(self):
        """The failure that shipped: every compiler present, Memcheck missing.

        A bundle restored without valgrind is indistinguishable from a good one
        until a contract gate reaches its oracle, twelve minutes later, and
        reports the oracle rather than the missing tool.  `verify` therefore
        names the tool at restore time.
        """
        self.assertEqual(self.export().returncode, 0)
        destination = self.temp / "restored"
        self.assertEqual(run("bash", str(BUNDLE), "restore", "--bundle", str(self.bundle),
                             "--root", str(destination)).returncode, 0)
        (destination / "usr/bin/valgrind").unlink()
        result = run("bash", str(BUNDLE), "verify", "--bundle", str(self.bundle),
                     "--root", str(destination))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("/usr/bin/valgrind", result.stderr)

    def stamp(self, key: str | None = None) -> None:
        """Mark the exported bundle as built for `key` (default: this machine)."""
        (self.bundle / "toolchain.key").write_text((key or current_key()) + "\n")

    def test_provision_uses_the_cached_bundle_when_it_verifies(self):
        self.assertEqual(self.export().returncode, 0)
        self.stamp()
        destination = self.temp / "prefix"
        result = run("bash", str(PROVISION), "--bundle-dir", str(self.bundle),
                     "--root", str(destination))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("cached bundle", result.stdout)
        self.assertTrue((destination / "usr/bin/gcc-16").is_file())

    def test_a_broken_cache_falls_back_to_install_not_to_a_fake_success(self):
        self.assertEqual(self.export().returncode, 0)
        self.stamp()
        (self.bundle / "toolchain.sha256").unlink()
        destination = self.temp / "prefix"
        result = run("bash", str(PROVISION), "--bundle-dir", str(self.bundle),
                     "--root", str(destination))
        self.assertNotEqual(result.returncode, 0)
        # It says why, and it refuses to run apt against a prefix.
        self.assertIn("failed verification", result.stdout)
        self.assertIn("cannot install into", result.stderr)
        self.assertFalse((destination / "usr/bin/gcc-16").exists())

    def test_a_bundle_for_another_image_is_never_unpacked(self):
        """The archive is intact and would verify; only its baseline is wrong.

        An unstamped bundle is the same case: every entry written before
        bundles were stamped was keyed by `runner.os` alone.
        """
        for stamp in ("ubuntu24-20261106.1-0123456789abcdef0123", None):
            with self.subTest(stamp=stamp):
                shutil.rmtree(self.bundle, ignore_errors=True)
                self.assertEqual(self.export().returncode, 0)
                if stamp:
                    self.stamp(stamp)
                archive = self.bundle / "debs"
                archive.mkdir()
                (archive / "gcc-16_16.2.0_amd64.deb").write_text("package file\n")
                destination = self.temp / "prefix"
                shutil.rmtree(destination, ignore_errors=True)
                result = run("bash", str(PROVISION), "--bundle-dir", str(self.bundle),
                             "--root", str(destination))
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(f"cached bundle is for {stamp or 'an unstamped baseline'}, "
                              f"this runner is {current_key()}", result.stdout)
                self.assertNotIn("cached bundle in", result.stdout)
                self.assertIn("cannot install into", result.stderr)
                self.assertFalse((destination / "usr/bin/gcc-16").exists())
                # What this job would save under ITS key must not be that
                # archive -- but the package files are the next install's seed.
                for name in ("toolchain.tar.gz", "toolchain.sha256", "toolchain.files",
                             "toolchain.key"):
                    self.assertFalse((self.bundle / name).exists(), name)
                self.assertTrue((archive / "gcc-16_16.2.0_amd64.deb").is_file())


class ToolchainKey(unittest.TestCase):
    """`ci_toolchain_key.sh`: what makes two bundles the same bundle."""

    def setUp(self) -> None:
        self.temp = Path(tempfile.mkdtemp(prefix="lccc-key-"))
        self.addCleanup(shutil.rmtree, self.temp, ignore_errors=True)
        self.scripts = self.temp / "repo/scripts"
        self.scripts.mkdir(parents=True)
        shutil.copy(KEY, self.scripts / KEY.name)
        for name in self.installers():
            if name != KEY.name:
                (self.scripts / name).write_text(f"# {name}\n")
        self.bin = self.temp / "bin"
        self.bin.mkdir()
        (self.bin / "dpkg-query").write_text(
            "#!/bin/sh\n"
            'printf "bash 5.2\\nlibc6 ${MOCK_LIBC:-2.39-0ubuntu8.11}\\n"\n')
        (self.bin / "dpkg-query").chmod(0o755)

    @staticmethod
    def installers() -> list[str]:
        listed = re.search(r"(?ms)^INSTALLERS=\(\n(.*?)^\)", KEY.read_text())
        return listed.group(1).split()

    def key(self, *args: str, **env: str) -> str:
        environment = {k: v for k, v in os.environ.items()
                       if k not in ("ImageOS", "ImageVersion")}
        environment.update(env, PATH=f"{self.bin}:{os.environ['PATH']}")
        result = subprocess.run(["bash", str(self.scripts / KEY.name), *args],
                                capture_output=True, text=True, env=environment, timeout=60)
        self.assertEqual(result.returncode, 0, result.stderr)
        return result.stdout.strip()

    def test_the_same_baseline_and_installer_give_the_same_key(self):
        self.assertEqual(self.key(), self.key())
        self.assertRegex(self.key(), r"^[A-Za-z0-9._-]+-[0-9a-f]{20}$")

    def test_any_package_of_the_baseline_changes_the_key(self):
        """Run 44345167051: glibc 8.11 -> 8.12 between two images of one OS."""
        self.assertNotEqual(self.key(), self.key(MOCK_LIBC="2.39-0ubuntu8.12"))

    def test_every_installer_script_is_part_of_the_key(self):
        before = self.key()
        for name in self.installers():
            with self.subTest(installer=name):
                path = self.scripts / name
                original = path.read_text()
                path.write_text(original + "# changed\n")
                self.assertNotEqual(self.key(), before)
                path.write_text(original)
                self.assertEqual(self.key(), before)

    def test_the_installer_list_covers_what_the_provisioner_runs(self):
        """A script the provisioner calls but the key omits could change the
        installed tree without changing the bundle's identity."""
        called = set(re.findall(r"\$REPO/scripts/(\w+\.sh)", PROVISION.read_text()))
        self.assertTrue(called >= {"ci_install_gcc16.sh", "ci_toolchain_bundle.sh",
                                   "ci_toolchain_key.sh"}, called)
        self.assertEqual(called | {PROVISION.name}, set(self.installers()))

    def test_the_image_name_is_a_label_and_never_the_identity(self):
        plain = self.key()
        named = self.key(ImageOS="ubuntu24", ImageVersion="20261116.133.1")
        self.assertTrue(named.startswith("ubuntu24-20261116.133.1-"), named)
        self.assertEqual(named.rsplit("-", 1)[1], plain.rsplit("-", 1)[1])
        # It ends up in a cache key and a file name.
        hostile = self.key(ImageOS="ubuntu 24", ImageVersion="2026/11:16\n$(id)")
        self.assertRegex(hostile, r"^[A-Za-z0-9._-]+$")

    def test_the_workflow_form_is_the_same_key(self):
        self.assertEqual(self.key("--github-output"), "key=" + self.key())

    def test_an_unknown_argument_is_refused(self):
        result = run("bash", str(KEY), "--frobnicate")
        self.assertEqual(result.returncode, 2)
        self.assertIn("unknown argument", result.stderr)
        self.assertEqual(result.stdout, "")


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
        # The real key tool: the identity it computes from the fake package
        # database below is the contract under test.
        shutil.copy(KEY, self.repo / "scripts" / KEY.name)
        # The real provisioner delegates mold+lld to this retrying installer;
        # keep that boundary in the fixture instead of falling through to an
        # absent script and masking the provisioning contract with exit 127.
        (self.repo / "scripts" / "ci_install_link_oracles.sh").write_text(
            "#!/bin/sh\n"
            'printf "install-link-oracles\\n" >> "$MOCK_LOG"\n')
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
            'printf "gcc16-archive %s\\n" "${LCCC_CI_APT_ARCHIVE_DIR:-unset}" >> "$MOCK_LOG"\n'
            'touch "$MOCK_INSTALLED"\n')
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
        # MOCK_BASELINE models another runner image (a base package moved);
        # the three-column form is the installed-version query the archive is
        # pruned against.
        (self.bin / "dpkg-query").write_text(
            "#!/bin/sh\n"
            'case "$*" in *Architecture*)\n'
            '  [ -n "${MOCK_DPKG_BROKEN:-}" ] && exit 1\n'
            '  printf "bash\\tamd64\\t5.2\\n"\n'
            '  if [ -e "$MOCK_INSTALLED" ]; then\n'
            '    printf "gcc-16\\tamd64\\t16.2.0\\nqemu-user\\tamd64\\t8.2\\n"\n'
            '    printf "valgrind\\tamd64\\t1:3.22.0-0ubuntu3\\n"\n'
            '    printf "libc6-dbg\\tamd64\\t2.39-0ubuntu8.12\\n"\n'
            "  fi\n"
            "  exit 0;;\n"
            "esac\n"
            'printf "bash 5.2\\ncoreutils ${MOCK_BASELINE:-9.4}\\n"\n'
            'if [ -e "$MOCK_INSTALLED" ]; then printf "gcc-16 16.2.0\\nqemu-user 8.2\\n"; fi\n'
            "exit 0\n")
        (self.bin / "dpkg-query").chmod(0o755)
        self.log = self.temp / "argv.log"
        self.bundle = self.temp / "bundle"
        self.bundle.mkdir()

    def environment(self, **overrides: str) -> dict[str, str]:
        env = dict(os.environ, MOCK_LOG=str(self.log),
                   MOCK_INSTALLED=str(self.temp / "installed"),
                   PATH=f"{self.bin}:{os.environ['PATH']}")
        for inherited in ("LCCC_TOOLCHAIN_BUNDLE_DIR", "LCCC_CI_APT_ARCHIVE_DIR",
                          "GITHUB_ACTIONS", "ImageOS", "ImageVersion"):
            env.pop(inherited, None)
        env.update(overrides)
        return env

    def provision(self, *args: str, **env: str) -> subprocess.CompletedProcess:
        return subprocess.run(
            ["bash", str(self.repo / "scripts" / PROVISION.name),
             "--bundle-dir", str(self.bundle), *args],
            capture_output=True, text=True, env=self.environment(**env), timeout=120)

    def fresh_runner(self) -> None:
        """The next job: a new machine of the same image, nothing installed.

        Hosted runners are discarded after every job, so a second invocation
        never sees the first one's packages.  Leaving the marker in place
        models a persistent machine instead, whose baseline is no longer the
        one the bundle was built against (see the test of that name).
        """
        (self.temp / "installed").unlink(missing_ok=True)
        self.log.unlink(missing_ok=True)

    def key(self, **env: str) -> str:
        result = subprocess.run(["bash", str(self.repo / "scripts" / KEY.name)],
                                capture_output=True, text=True, check=True,
                                env=self.environment(**env))
        return result.stdout.strip()

    @property
    def archive(self) -> Path:
        return self.bundle / "debs"

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
        # qemu-user is the i686 execution oracle; python3-yaml is the gate
        # parser; valgrind is the Memcheck oracle and libc6-dbg lets it
        # relocate loader symbols; gdb is the DWARF value oracle. The linker
        # pair has its own retrying, fail-closed installer and must not be
        # folded back into this apt call.
        #
        # The transaction reads and fills the bundle's own package archive and
        # index, like the two installers: it was the one that did not, and
        # fetched 36.6 MB from the mirror on every install (6 min 59 s at
        # 87 kB/s in run 44345167051).
        self.assertEqual(apt[0].split()[1:], [
            "install", "-y", "--no-install-recommends",
            "-o", f"Dir::Cache::archives={self.archive}",
            "-o", "APT::Keep-Downloaded-Packages=true",
            "-o", f"Dir::State::lists={self.archive}/lists",
            "qemu-user", "python3-yaml", "valgrind", "libc6-dbg", "gdb"])
        self.assertIn(f"gcc16-archive {self.archive}", lines)
        delegated = [line for line in lines if line == "install-link-oracles"]
        self.assertEqual(delegated, ["install-link-oracles"],
                         f"mold+lld must use the dedicated installer: {lines}")
        exports = [line for line in lines if line.startswith("bundle export")]
        self.assertEqual(len(exports), 1, lines)
        # The exported package set is the dpkg-delta, never a hard-coded list.
        self.assertIn("--packages gcc-16 qemu-user", exports[0])
        self.assertEqual(self.calls("verify"),
                         ["bundle verify --bundle %s --root /" % self.bundle])
        # verify is a read-only probe: it must never be escalated, so a lane
        # cannot end up "verified" against a tree only root can execute.
        self.assertEqual(self.escalated("verify"), [])
        # The exported bundle says which baseline it is a delta against: the
        # one this job STARTED from, not the one it leaves behind.
        (self.temp / "installed").unlink()
        self.assertEqual((self.bundle / "toolchain.key").read_text(), self.key() + "\n")

    def restore_tail(self, root: str) -> str:
        """The `restore` invocation's last three arguments, for assertion."""
        return "restore --bundle %s --root %s" % (self.bundle, root)

    def test_a_second_invocation_restores_instead_of_installing(self):
        self.assertEqual(self.provision("--root", "/").returncode, 0)
        self.fresh_runner()
        result = self.provision("--root", "/")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(len(self.calls("restore")), 1, self.argv())
        self.assertIn(self.restore_tail("/"), self.calls("restore")[0])
        self.assertEqual(len(self.escalated("restore")), 1, self.argv())
        self.assertIn("cached bundle", result.stdout)
        self.assertFalse([line for line in self.argv() if line.startswith("install-gcc16")])

    def test_a_bundle_from_another_image_seeds_the_install_and_is_replaced(self):
        """A new runner image: nothing is unpacked, apt starts from the
        bundle's own package files, and the result is stamped for this image
        so that the cache action saves it under this image's key."""
        self.assertEqual(self.provision("--root", "/").returncode, 0)
        built_for = (self.bundle / "toolchain.key").read_text().strip()
        (self.archive / "gcc-16_16.2.0_amd64.deb").write_text("package file\n")
        self.fresh_runner()
        here = self.key(MOCK_BASELINE="9.5")
        self.assertNotEqual(here, built_for)
        result = self.provision("--root", "/", MOCK_BASELINE="9.5", GITHUB_ACTIONS="true")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn(f"cached bundle is for {built_for}, this runner is {here}", result.stdout)
        self.assertEqual(self.invocations("restore"), [], self.argv())
        lines = self.argv()
        self.assertEqual(lines[0], "install-gcc16 --multilib")
        self.assertIn(f"gcc16-archive {self.archive}", lines)
        self.assertEqual(len([line for line in lines if line.startswith("bundle export")]), 1)
        self.assertEqual((self.bundle / "toolchain.key").read_text().strip(), here)
        self.assertTrue((self.archive / "gcc-16_16.2.0_amd64.deb").is_file())
        # Expected on a new image, so a notice -- but visible without the log.
        self.assertIn("::notice title=Toolchain bundle rebuilt for this runner image::",
                      result.stdout)
        self.assertNotIn("::warning", result.stdout)

    def test_a_machine_that_kept_its_packages_is_not_the_baseline(self):
        """A persistent host after its first provisioning: the bundle is a
        delta against a baseline it no longer has, and there is nothing left
        to install -- so nothing is unpacked and nothing stale is kept."""
        self.assertEqual(self.provision("--root", "/").returncode, 0)
        self.log.unlink()
        result = self.provision("--root", "/")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(self.invocations("restore"), [], self.argv())
        self.assertIn("no package delta after install", result.stdout)
        self.assertFalse((self.bundle / "toolchain.tar.gz").exists())
        self.assertFalse((self.bundle / "toolchain.key").exists())
        self.assertEqual(len(self.calls("verify")), 1, self.argv())

    def test_annotations_stay_out_of_a_terminal(self):
        self.assertEqual(self.provision("--root", "/").returncode, 0)
        self.fresh_runner()
        result = self.provision("--root", "/", MOCK_BASELINE="9.5")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertNotIn("::", result.stdout)

    def test_the_archive_keeps_exactly_the_installed_package_files(self):
        """A seeded archive also holds the previous image's versions; an
        archive that only grew would hand every later image all of them."""
        self.archive.mkdir(parents=True)
        (self.archive / "lists").mkdir()
        (self.archive / "partial").mkdir()
        keep = {"gcc-16_16.2.0_amd64.deb",
                # apt writes the epoch's colon as %3a.
                "valgrind_1%3a3.22.0-0ubuntu3_amd64.deb"}
        drop = {"gcc-16_16.1.0_amd64.deb",                 # superseded version
                "libc6-dbg_2.39-0ubuntu8.11_amd64.deb",    # the previous image's glibc
                "qemu-user_8.2_i386.deb",                  # another architecture
                "mold_2.30_amd64.deb"}                     # not installed at all
        for name in keep | drop:
            (self.archive / name).write_text("package file\n")
        (self.archive / "lists/ppa_Packages").write_text("index\n")
        (self.archive / "partial/half_1_amd64.deb").write_text("truncated\n")
        result = self.provision("--root", "/")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual({path.name for path in self.archive.glob("*.deb")}, keep)
        self.assertTrue((self.archive / "lists/ppa_Packages").is_file())
        self.assertFalse((self.archive / "partial").exists())

    def test_an_unreadable_package_database_does_not_empty_the_archive(self):
        self.archive.mkdir(parents=True)
        names = {"gcc-16_16.2.0_amd64.deb", "gcc-16_16.1.0_amd64.deb"}
        for name in names:
            (self.archive / name).write_text("package file\n")
        result = self.provision("--root", "/", MOCK_DPKG_BROKEN="1")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual({path.name for path in self.archive.glob("*.deb")}, names)

    def test_a_second_archive_is_adopted_when_the_bundle_has_none(self):
        """The weekly `lccc-apt-debs` cache of a lane that starts with no
        bundle at all: its files become the bundle's archive, and the
        installers are pointed at that one."""
        weekly = self.temp / "weekly"
        (weekly / "lists").mkdir(parents=True)
        (weekly / "partial").mkdir()
        (weekly / "gcc-16_16.2.0_amd64.deb").write_text("package file\n")
        (weekly / "lists/ppa_Packages").write_text("index\n")
        result = self.provision("--root", "/", LCCC_CI_APT_ARCHIVE_DIR=str(weekly))
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn(f"gcc16-archive {self.archive}", self.argv())
        self.assertEqual((self.archive / "gcc-16_16.2.0_amd64.deb").read_text(),
                         "package file\n")
        self.assertEqual((self.archive / "lists/ppa_Packages").read_text(), "index\n")
        # Adopted, not moved: that cache has its own key and its own save.
        self.assertTrue((weekly / "gcc-16_16.2.0_amd64.deb").is_file())

    def test_without_a_bundle_the_callers_archive_is_used_as_given(self):
        weekly = self.temp / "weekly"
        weekly.mkdir()
        result = self.provision("--root", "/", "--no-bundle",
                                LCCC_CI_APT_ARCHIVE_DIR=str(weekly))
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn(f"gcc16-archive {weekly}", self.argv())
        apt = [line for line in self.argv() if line.startswith("apt-get")]
        self.assertIn(f"Dir::Cache::archives={weekly}", apt[0])
        self.assertFalse(self.archive.exists())
        self.assertFalse((self.bundle / "toolchain.key").exists())

    def test_the_live_root_restore_is_escalated_and_a_prefix_is_not(self):
        self.assertEqual(self.provision("--root", "/").returncode, 0)
        self.fresh_runner()
        self.assertEqual(self.provision("--root", "/").returncode, 0)
        self.assertEqual(len(self.calls("restore")), 1, self.argv())
        self.assertEqual(len(self.escalated("restore")), 1, self.argv())
        self.assertIn(self.restore_tail("/"), self.calls("restore")[0])
        self.fresh_runner()
        prefix = self.temp / "prefix"
        prefix.mkdir()
        self.assertEqual(self.provision("--root", str(prefix)).returncode, 0)
        self.assertEqual(len(self.calls("restore")), 1, self.argv())
        self.assertEqual(self.escalated("restore"), [], self.argv())
        self.assertIn(self.restore_tail(str(prefix)), self.calls("restore")[0])

    def test_a_failing_restore_falls_through_to_the_installer(self):
        self.assertEqual(self.provision("--root", "/").returncode, 0)
        self.fresh_runner()
        result = self.provision("--root", "/", MOCK_RESTORE_STATUS="1",
                                GITHUB_ACTIONS="true")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("failed verification", result.stdout)
        self.assertEqual(len(self.escalated("restore")), 1, self.argv())
        self.assertIn(self.restore_tail("/"), self.escalated("restore")[0])
        self.assertIn("install-gcc16 --multilib", self.argv())
        # This bundle claimed to be this image's, and its cache entry cannot
        # be replaced: that is a warning, not the notice a new image gets.
        self.assertIn("::warning title=Cached toolchain bundle rejected::", result.stdout)
        self.assertNotIn("::notice", result.stdout)

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


class WorkflowCacheKeys(unittest.TestCase):
    """Every lane's bundle cache is keyed by the runner image.

    The preamble is repeated in each job, and the two lanes that were added
    without the package archive their thirteen siblings had are how a rejected
    bundle became a timeout.  A key is the same kind of line: one copy that
    still names only the operating system restores another image's files.
    """

    PATH = "~/.cache/lccc-toolchain"
    KEY_STEP = 'bash scripts/ci_toolchain_key.sh --github-output >> "$GITHUB_OUTPUT"'
    CHANGES_PACKAGES = re.compile(r"\b(apt-get|apt|dpkg|add-apt-repository)\b|ci_install_|ci_provision_")

    def lanes(self):
        for path in sorted((ROOT / ".github/workflows").glob("*.yml")):
            workflow = yaml.safe_load(path.read_text())
            for name, job in (workflow.get("jobs") or {}).items():
                steps = job.get("steps") or []
                caches = [index for index, step in enumerate(steps)
                          if (step.get("with") or {}).get("path") == self.PATH]
                provisions = [index for index, step in enumerate(steps)
                              if "ci_provision_toolchain.sh" in str(step.get("run") or "")]
                if caches or provisions:
                    yield f"{path.name}:{name}", steps, caches, provisions

    def test_there_are_lanes_to_check(self):
        self.assertGreaterEqual(len(list(self.lanes())), 18)

    def test_every_provisioning_lane_restores_the_bundle_for_its_own_image(self):
        for lane, steps, caches, provisions in self.lanes():
            with self.subTest(lane=lane):
                self.assertEqual(len(caches), 1, "exactly one bundle cache step")
                self.assertEqual(len(provisions), 1, "exactly one provisioning step")
                cache, provision = caches[0], provisions[0]
                self.assertLess(cache, provision, "the bundle is restored before it is used")
                settings = steps[cache]["with"]
                self.assertEqual(settings["key"],
                                 "lccc-toolchain-v2-${{ steps.toolchain-key.outputs.key }}")
                # The newest bundle of any image is the seed of a new one.
                self.assertEqual(settings.get("restore-keys", "").split(),
                                 ["lccc-toolchain-v2-"])
                identify = [index for index, step in enumerate(steps)
                            if step.get("id") == "toolchain-key"]
                self.assertEqual(len(identify), 1, "exactly one identity step")
                self.assertLess(identify[0], cache)
                self.assertEqual(steps[identify[0]].get("run"), self.KEY_STEP)
                # The identity is the image's: nothing may install a package
                # before it is taken.
                for step in steps[:identify[0]]:
                    self.assertIsNone(self.CHANGES_PACKAGES.search(str(step.get("run") or "")),
                                      f"`{step.get('name')}` runs before the image is identified")

    def test_no_key_names_only_the_operating_system(self):
        for path in sorted((ROOT / ".github/workflows").glob("*.yml")):
            with self.subTest(workflow=path.name):
                self.assertNotIn("lccc-toolchain-v1", path.read_text())


if __name__ == "__main__":
    unittest.main()
