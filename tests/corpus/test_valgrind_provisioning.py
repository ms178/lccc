"""Private current-Valgrind cache receipts: no network or real compiler use."""
import hashlib
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
INSTALLER = ROOT / "scripts/ci_install_valgrind_current.sh"
COMMIT = "5a4657de12a8ed16b07175e1769df4dad31f80f2"
DIGEST = "3e6c1aa8c8bbf14cbc8c70a0eb1c7bb40b8515c8f93e23191c8ceffeadab91fa"


class ValgrindProvisioning(unittest.TestCase):
    def invoke(self, prefix, *args, **env):
        return subprocess.run(
            ["bash", str(INSTALLER), "--prefix", str(prefix), *args],
            env={**os.environ, **env}, text=True, capture_output=True, check=False,
        )

    def cache(self, root):
        prefix = root / "prefix"
        for directory in ("bin", "libexec/valgrind"):
            (prefix / directory).mkdir(parents=True, exist_ok=True)
        launcher = prefix / "bin/valgrind"
        launcher.write_text("#!/bin/sh\nprintf '%s\\n' valgrind-3.28.0\n")
        files = [launcher]
        for arch in ("amd64", "x86"):
            tool = prefix / f"libexec/valgrind/callgrind-{arch}-linux"
            tool.write_text("#!/bin/sh\nexit 0\n")
            files.append(tool)
        for path in files:
            path.chmod(0o755)
        (prefix / ".lccc-source").write_text(f"{COMMIT} {DIGEST}\n")
        (prefix / ".lccc-tools.sha256").write_text("".join(
            f"{hashlib.sha256(p.read_bytes()).hexdigest()}  {p.relative_to(prefix)}\n"
            for p in files
        ))
        return prefix

    def test_verified_dual_mode_cache_is_reused_and_exported(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            prefix = self.cache(root)
            exported = root / "github-path"
            result = self.invoke(prefix, "--print-bin", GITHUB_PATH=str(exported))
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout.strip(), str(prefix / "bin"))
            self.assertEqual(exported.read_text().strip(), str(prefix / "bin"))
            self.assertIn("reusing verified", result.stderr)

    def test_changed_tool_cannot_be_accepted_by_version_string(self):
        with tempfile.TemporaryDirectory() as directory:
            prefix = self.cache(Path(directory))
            (prefix / "libexec/valgrind/callgrind-amd64-linux").write_text("changed\n")
            result = self.invoke(prefix)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("unverified nonempty prefix", result.stderr)

    def test_old_source_receipt_is_not_a_cache_hit(self):
        with tempfile.TemporaryDirectory() as directory:
            prefix = self.cache(Path(directory))
            (prefix / ".lccc-source").write_text("old-source old-digest\n")
            result = self.invoke(prefix)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("unverified nonempty prefix", result.stderr)

    def test_missing_i386_tool_is_not_accepted(self):
        with tempfile.TemporaryDirectory() as directory:
            prefix = self.cache(Path(directory))
            (prefix / "libexec/valgrind/callgrind-x86-linux").unlink()
            result = self.invoke(prefix)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("unverified nonempty prefix", result.stderr)

    def test_foreign_prefix_is_never_deleted_or_overlaid(self):
        with tempfile.TemporaryDirectory() as directory:
            prefix = Path(directory) / "foreign"
            prefix.mkdir()
            sentinel = prefix / "keep"
            sentinel.write_text("untouched\n")
            result = self.invoke(prefix)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(sentinel.read_text(), "untouched\n")

    def test_unknown_option_is_a_usage_error(self):
        with tempfile.TemporaryDirectory() as directory:
            result = self.invoke(Path(directory) / "prefix", "--not-an-option")
            self.assertEqual(result.returncode, 2)


if __name__ == "__main__":
    unittest.main()
