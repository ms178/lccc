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

    def test_missing_i386_headers_fail_before_network_or_build(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            tools = root / "tools"
            tools.mkdir()
            marker = root / "network-attempted"
            mocks = {
                "compiler": """#!/bin/sh
if [ "$1" = -dumpfullversion ]; then echo 16.2.0; exit 0; fi
cat >/dev/null
echo 'asm/types.h: missing mock i386 header' >&2
exit 1
""",
                "as": "#!/bin/sh\necho 'GNU assembler 2.47'\n",
                "ld": "#!/bin/sh\necho 'mold 2.42.1 (mock)'\n",
                "curl": f"#!/bin/sh\ntouch '{marker}'\nexit 99\n",
            }
            for name, content in mocks.items():
                tool = tools / name
                tool.write_text(content)
                tool.chmod(0o755)
            result = self.invoke(root / "prefix", CC=str(tools / "compiler"),
                                 CXX=str(tools / "compiler"),
                                 PATH=str(tools) + os.pathsep + os.environ["PATH"])
            self.assertEqual(result.returncode, 1, result.stderr)
            self.assertIn("missing i386 Linux/libc development headers", result.stderr)
            self.assertFalse(marker.exists())
            self.assertFalse((root / "prefix").exists())

    def test_bootstrap_installs_i386_kernel_headers_without_old_compilers(self):
        script = (ROOT / "scripts/ci_prepare_research_tools.sh").read_text()
        self.assertLess(script.index("sudo dpkg --add-architecture i386"),
                        script.index("sudo apt-get update"))
        install = next(line for line in script.splitlines() if line.startswith("sudo apt-get install"))
        self.assertIn("linux-libc-dev:i386", install.split())
        self.assertIn("libc6-dev-i386", install.split())
        self.assertFalse(any(token.startswith(("gcc", "g++", "binutils")) for token in install.split()))

    def test_unknown_option_is_a_usage_error(self):
        with tempfile.TemporaryDirectory() as directory:
            result = self.invoke(Path(directory) / "prefix", "--not-an-option")
            self.assertEqual(result.returncode, 2)


if __name__ == "__main__":
    unittest.main()
