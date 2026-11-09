"""Fast content-identity and hostile-cache tests for the GCC oracle prefix."""
import importlib.util
import io
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("gcc_oracle_tree", ROOT / "scripts/gcc_oracle_tree.py")
tree = importlib.util.module_from_spec(spec)
spec.loader.exec_module(tree)


class OracleTree(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name) / "gcc-16"
        (self.root / "bin").mkdir(parents=True)
        (self.root / "bin/gcc").write_bytes(b"driver\n")
        (self.root / "bin/gcc").chmod(0o755)
        os.link(self.root / "bin/gcc", self.root / "bin/gcc-16")
        (self.root / "bin/g++").symlink_to("gcc")
        (self.root / "lib").mkdir()
        (self.root / "lib/cc1").write_bytes(b"front end\n")
        (self.root / "lib/cc1").chmod(0o644)
        self.archive = Path(self.tmp.name) / "oracle.tar.xz"
        with tarfile.open(self.archive, "w:xz") as tar:
            tar.add(self.root, arcname="gcc-16")
        self.baseline = tree.tree_digest(tree.archive_records(self.archive))

    def current(self):
        return tree.tree_digest(tree.directory_records(self.root))

    def test_archive_and_extracted_tree_match_including_hardlinks(self):
        self.assertEqual(self.current(), self.baseline)
        # A copied hardlink has the same semantics and is deliberately accepted.
        (self.root / "bin/gcc-16").unlink()
        (self.root / "bin/gcc-16").write_bytes(b"driver\n")
        (self.root / "bin/gcc-16").chmod(0o755)
        self.assertEqual(self.current(), self.baseline)

    def test_mutated_file_mode_new_path_and_symlink_each_change_identity(self):
        cc1 = self.root / "lib/cc1"
        cc1.write_bytes(b"front end changed\n")
        self.assertNotEqual(self.current(), self.baseline)
        cc1.write_bytes(b"front end\n")
        cc1.chmod(0o600)
        self.assertNotEqual(self.current(), self.baseline)
        cc1.chmod(0o644)
        (self.root / "lib/injected").write_text("extra")
        self.assertNotEqual(self.current(), self.baseline)
        (self.root / "lib/injected").unlink()
        (self.root / "bin/g++").unlink()
        (self.root / "bin/g++").symlink_to("/bin/true")
        self.assertNotEqual(self.current(), self.baseline)

    def test_refuses_root_symlink_and_fifo(self):
        link = self.root.parent / "link"
        link.symlink_to(self.root, target_is_directory=True)
        with self.assertRaises(ValueError):
            tree.directory_records(link)
        os.mkfifo(self.root / "fifo")
        with self.assertRaises(ValueError):
            tree.directory_records(self.root)

    def test_untrusted_archive_paths_and_duplicates_rejected(self):
        for names in (("gcc-16/../escape",), ("/gcc-16/absolute",),
                      ("gcc-16/same", "gcc-16/same")):
            with self.subTest(names=names):
                archive = self.root.parent / "bad.tar.xz"
                with tarfile.open(archive, "w:xz") as tar:
                    for name in names:
                        member = tarfile.TarInfo(name)
                        member.size = 1
                        tar.addfile(member, io.BytesIO(b"x"))
                with self.assertRaises(ValueError):
                    tree.archive_records(archive)

    def test_cli_fails_closed_on_mismatch(self):
        proc = subprocess.run([sys.executable, str(ROOT / "scripts/gcc_oracle_tree.py"),
                               "--directory", str(self.root), "--expect", "0" * 64],
                              capture_output=True, text=True, timeout=5)
        self.assertNotEqual(proc.returncode, 0)
        self.assertIn("mismatch", proc.stderr)


if __name__ == "__main__":
    unittest.main()
