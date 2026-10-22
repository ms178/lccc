#!/usr/bin/env python3
"""Fail-closed contract for `scripts/oracle_toolchains_from_tags.sh`.

The script restores the pinned reference compilers from the repository's own
transport tags. Its whole reason to exist is that a provisioning step must
never produce a binary of the right name and the wrong version (or bytes), so
the tests below drive it against a mock repository with mock tags and pin
every refusal path, not just the happy one:

* a clean restore installs, probes versions, link-tests and is idempotent;
* a manifest that disagrees with the script's in-tree pins is refused before
  any payload is trusted (a rewritten tag must not vouch for itself);
* a payload whose digest does not match the pin is refused;
* a toolchain that answers `--version` with the wrong version is refused;
* a missing member is refused with the tag and member named;
* a payload split into `.partNN` chunks restores exactly like a whole one --
  the manifest crosscheck must not compare a chunk digest against the
  whole-member pin (a guaranteed false mismatch).

The mock toolchains are shell scripts: `--version` prints the pinned line,
any other invocation copies /bin/true to the `-o` target, which is all the
script's link probe (compile a trivial TU, check exit status) requires.
Only the four `*_SHA=` pins are rewritten in the copied script; the test
asserts those pins exist, so a future script that stopped pinning digests
in-tree fails here first.
"""
from __future__ import annotations

import hashlib
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts/oracle_toolchains_from_tags.sh"

GCC_VERSION = "16.2.0"
CLANG_VERSION = "23.1.3"

FAKE_DRIVER = """#!/bin/sh
for a in "$@"; do
    if [ "$a" = "--version" ]; then
        echo "{version_line}"
        exit 0
    fi
done
out=
while [ $# -gt 0 ]; do
    if [ "$1" = "-o" ]; then out=$2; fi
    shift
done
[ -n "$out" ] && cp /bin/true "$out"
exit 0
"""

FAKE_MOLD = """#!/bin/sh
echo "mold 2.42.1 (mock; mock)"
"""


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def make_tarball(path: Path, rootname: str, files: dict[str, tuple[str, int]]) -> bytes:
    """files: name -> (content, mode). Returns the tarball bytes."""
    with tempfile.TemporaryDirectory() as td:
        root = Path(td) / rootname
        for name, (content, mode) in files.items():
            target = root / name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(content)
            target.chmod(mode)
        with tarfile.open(path, "w:xz") as tf:
            tf.add(root, arcname=rootname)
    return path.read_bytes()


class OracleScriptTests(unittest.TestCase):
    def setUp(self):
        self.td = Path(tempfile.mkdtemp(prefix="oracle-mock-"))
        self.addCleanup(shutil.rmtree, self.td, True)
        self.repo = self.td / "repo"
        self.repo.mkdir()
        self.prefix = self.td / "prefix"
        self.bindir = self.td / "bin"
        self.bindir.mkdir()
        self.git("init", "-q")
        self.git("config", "user.email", "test@example.invalid")
        self.git("config", "user.name", "Oracle Mock")

        gcc_line = f"gcc-16 (GCC) {GCC_VERSION}"
        clang_line = f"clang version {CLANG_VERSION}"
        self.members: dict[str, tuple[str, bytes]] = {}  # member -> (tag, bytes)
        gcc_tar = self.td / "gcc.tar.xz"
        self.members["gcc-16-bookworm-x86_64.tar.xz"] = (
            "harness-cache-1",
            make_tarball(gcc_tar, "gcc-root", {
                "bin/gcc-16": (FAKE_DRIVER.format(version_line=gcc_line), 0o755),
                "bin/g++-16": (FAKE_DRIVER.format(version_line=gcc_line), 0o755),
                "bin/cpp-16": (FAKE_DRIVER.format(version_line=gcc_line), 0o755),
            }),
        )
        clang_tar = self.td / "clang.tar.xz"
        self.members["clang-23.1.3-linux-x64.tar.xz"] = (
            "harness-cache-4",
            make_tarball(clang_tar, "clang-root", {
                "bin/clang-23": (FAKE_DRIVER.format(version_line=clang_line), 0o755),
                "bin/clang": (FAKE_DRIVER.format(version_line=clang_line), 0o755),
            }),
        )
        self.members["mold-2.42.1-x86_64"] = ("harness-cache-1", FAKE_MOLD.encode())
        bu_tar = self.td / "binutils.tar.xz"
        self.members["binutils-2.47.tar.xz"] = (
            "harness-cache-1",
            make_tarball(bu_tar, "binutils-src", {"README": ("mock\n", 0o644)}),
        )
        self.pin_override: dict[str, str] = {}
        self.manifest_override: dict[str, str] = {}
        self.split_members: set[str] = set()
        self.omit_members: set[str] = set()

    # -- helpers -------------------------------------------------------------
    def git(self, *args: str) -> subprocess.CompletedProcess:
        return subprocess.run(
            ["git", "-C", str(self.repo), *args],
            check=True, capture_output=True, text=True,
        )

    def commit_tags(self):
        """(Re)create both transport tags from the current member table."""
        for tag in ("harness-cache-1", "harness-cache-4"):
            files = {m: data for m, (t, data) in self.members.items()
                     if t == tag and m not in self.omit_members}
            manifest = []
            for member, data in sorted(files.items()):
                if member in self.split_members:
                    half = len(data) // 2
                    for idx, chunk in enumerate((data[:half], data[half:])):
                        name = f"{member}.part{idx:02d}"
                        (self.repo / name).write_bytes(chunk)
                        self.git("add", name)
                        manifest.append(f"{sha256(chunk)}  {name}")
                else:
                    (self.repo / member).write_bytes(data)
                    self.git("add", member)
                    digest = self.manifest_override.get(member, sha256(data))
                    manifest.append(f"{digest}  {member}")
            (self.repo / "MANIFEST.sha256").write_text("\n".join(manifest) + "\n")
            self.git("add", "MANIFEST.sha256")
            self.git("commit", "-q", "--allow-empty", "-m", f"mock {tag}")
            # A fresh tag object each time: -f so reruns replace it.
            self.git("tag", "-f", tag)
            self.git("rm", "-q", "-r", "--cached", ".")
            for p in self.repo.iterdir():
                if p.name != ".git":
                    p.unlink()

    def install_script(self):
        text = SCRIPT.read_text()
        pins = {
            "GCC_SHA": "gcc-16-bookworm-x86_64.tar.xz",
            "CLANG_SHA": "clang-23.1.3-linux-x64.tar.xz",
            "MOLD_SHA": "mold-2.42.1-x86_64",
            "BINUTILS_SHA": "binutils-2.47.tar.xz",
        }
        for var, member in pins.items():
            assert re.search(rf"^{var}=[0-9a-f]{{64}}$", text, re.M), (
                f"script no longer pins {var} in-tree; the crosscheck design "
                "depends on it")
            digest = self.pin_override.get(member, sha256(self.members[member][1]))
            text = re.sub(rf"^{var}=[0-9a-f]{{64}}$", f"{var}={digest}", text, flags=re.M)
        scripts = self.repo / "scripts"
        scripts.mkdir(exist_ok=True)
        target = scripts / "oracle_toolchains_from_tags.sh"
        target.write_text(text)
        target.chmod(0o755)
        return target

    def run_script(self, *extra: str) -> subprocess.CompletedProcess:
        return subprocess.run(
            [str(self.script), "--prefix", str(self.prefix),
             "--bin-dir", str(self.bindir), *extra],
            capture_output=True, text=True, cwd=self.repo,
        )

    # -- the contract ---------------------------------------------------------
    def test_clean_restore_installs_probes_and_is_idempotent(self):
        self.commit_tags()
        self.script = self.install_script()
        proc = self.run_script()
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        gcc = self.prefix / f"gcc-{GCC_VERSION}/bin/gcc-16"
        clang = self.prefix / f"clang-{CLANG_VERSION}/bin/clang-23"
        self.assertTrue(gcc.exists() and clang.exists())
        self.assertTrue((self.bindir / "gcc-16").exists())
        again = self.run_script()
        self.assertEqual(again.returncode, 0, again.stdout + again.stderr)
        self.assertIn("reused", again.stdout)
        verify = self.run_script("--verify-only")
        self.assertEqual(verify.returncode, 0, verify.stdout + verify.stderr)

    def test_manifest_disagreement_is_refused(self):
        self.manifest_override["gcc-16-bookworm-x86_64.tar.xz"] = "0" * 64
        self.commit_tags()
        self.script = self.install_script()
        proc = self.run_script()
        self.assertEqual(proc.returncode, 1)
        self.assertIn("disagrees with this script", proc.stdout + proc.stderr)

    def test_payload_digest_mismatch_is_refused(self):
        # Pin and manifest agree with each other but not with the bytes:
        # the post-materialise digest check is what must refuse this.
        wrong = "f" * 64
        self.pin_override["clang-23.1.3-linux-x64.tar.xz"] = wrong
        self.manifest_override["clang-23.1.3-linux-x64.tar.xz"] = wrong
        self.commit_tags()
        self.script = self.install_script()
        proc = self.run_script()
        self.assertEqual(proc.returncode, 1)
        self.assertIn("digest mismatch", proc.stdout + proc.stderr)

    def test_wrong_version_is_refused(self):
        clang_tar = self.td / "clang22.tar.xz"
        self.members["clang-23.1.3-linux-x64.tar.xz"] = (
            "harness-cache-4",
            make_tarball(clang_tar, "clang-root", {
                "bin/clang-23": (FAKE_DRIVER.format(
                    version_line="clang version 22.9.9"), 0o755),
                "bin/clang": (FAKE_DRIVER.format(
                    version_line="clang version 22.9.9"), 0o755),
            }),
        )
        self.commit_tags()
        self.script = self.install_script()
        proc = self.run_script()
        self.assertEqual(proc.returncode, 1)
        self.assertIn("22.9.9", proc.stdout + proc.stderr)

    def test_missing_member_is_refused_and_named(self):
        self.omit_members.add("mold-2.42.1-x86_64")
        self.commit_tags()
        self.script = self.install_script()
        proc = self.run_script()
        self.assertEqual(proc.returncode, 1)
        self.assertIn("mold-2.42.1-x86_64", proc.stdout + proc.stderr)

    def test_split_payload_restores_like_a_whole_one(self):
        self.split_members.add("gcc-16-bookworm-x86_64.tar.xz")
        self.commit_tags()
        self.script = self.install_script()
        proc = self.run_script()
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        self.assertTrue((self.prefix / f"gcc-{GCC_VERSION}/bin/gcc-16").exists())


if __name__ == "__main__":
    sys.exit(unittest.main(verbosity=2) or 0)
