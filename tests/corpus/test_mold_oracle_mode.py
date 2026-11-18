"""Exercise mold cache/source switching without building or fetching a linker."""

import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import unittest


INSTALL_SOURCE_STUB = """
set -eu
mkdir -p "$PREFIX/bin"
cat > "$PREFIX/bin/mold" <<'LINKER'
#!/bin/sh
echo 'mold 3.0.0 source-build'
LINKER
chmod +x "$PREFIX/bin/mold"
"""


ROOT = Path(__file__).resolve().parents[2]


class MoldOracleModeTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="mold-oracle-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.prefix = self.root / "install"
        self.stub_bin = self.root / "bin"
        self.stub_bin.mkdir()
        for name in ("tests/linker/setup_oracles.sh", "tools/linker/setup_oracles.sh",
                     "tools/linker/oracle_versions.sh", "tools/linker/native_bfd_wrapper.sh"):
            dest = self.root / name
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / name, dest)
        (self.root / "scripts").mkdir()
        self.script(self.root / "scripts/ensure_mold_300_binary.sh", """
set -eu
[ "$1" = --prefix ]
mkdir -p "$2/bin"
cat > "$2/bin/mold" <<'STUB'
#!/bin/sh
echo 'mold 3.0.0 release-cache'
STUB
chmod +x "$2/bin/mold"
""")
        self.script(self.root / "install-mold.sh", INSTALL_SOURCE_STUB)
        self.script(self.stub_bin / "git", """
set -eu
case " $* " in
  *' clone '*)
    for dst in "$@"; do :; done
    mkdir -p "$dst/.git"
    cp "$MOLD_TEST_INSTALL" "$dst/install-mold.sh"
    ;;
  *' rev-parse '*) echo 1234567890123456789012345678901234567890 ;;
  *' fetch '*|*' checkout '*) ;;
  *) echo "unexpected git $*" >&2; exit 1 ;;
esac
""")
        self.script(self.root / "scripts/ensure_binutils_247_binary.sh", """
set -eu
[ "$1" = --prefix ]
mkdir -p "$2/bin"
printf '#!/bin/sh\\necho "GNU ld 2.47.20260726"\\n' > "$2/bin/ld"
chmod +x "$2/bin/ld"
""")
        self.script(self.stub_bin / "cargo", """printf '%s\\n' "$*" >> "$MOLD_TEST_CARGO_LOG"
""")
        self.cargo_log = self.root / "cargo.log"
        self.env = dict(os.environ, PATH=f"{self.stub_bin}{os.pathsep}{os.environ['PATH']}",
                        MOLD_REF="v3.0.0", MOLD_TEST_CARGO_LOG=str(self.cargo_log),
                        MOLD_TEST_INSTALL=str(self.root / "install-mold.sh"))

    @staticmethod
    def script(path, text):
        path.write_text(f"#!/bin/sh\n{text}")
        path.chmod(0o755)

    def setup_mold(self, mode):
        run = subprocess.run(
            ["bash", str(self.root / "tests/linker/setup_oracles.sh"), "--only", "mold",
             "--prefix", str(self.prefix), "-j", "2"],
            cwd=self.root, env=dict(self.env, MOLD_PREFER=mode),
            capture_output=True, text=True, timeout=15,
        )
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        tools = self.prefix / "bin"
        for name in ("mold", "ld.mold", "mold-v3_0_0"):
            banner = subprocess.check_output([tools / name, "--version"], text=True)
            self.assertIn("source-build" if mode == "source" else "release-cache", banner)
        return (tools / "ORACLE_REVISIONS.txt").read_text()

    def test_source_request_after_cache_builds_the_x86_native_variant(self):
        cached = self.setup_mold("cache")
        self.assertFalse(self.cargo_log.exists())
        source = self.setup_mold("source")
        self.assertIn("source-build", source)
        self.assertIn("1234567890123456789012345678901234567890", source)
        args = self.cargo_log.read_text().splitlines()
        self.assertEqual(args, ["build --release --locked -j 2 -p mold-cli "
                                "--no-default-features --features x86_64,i386,system-allocator"])
        self.assertIn("release-cache", cached)
        self.assertNotIn("source-build", self.setup_mold("cache"))
        self.assertIn("source-build", self.setup_mold("source"))
        self.assertEqual(self.cargo_log.read_text().splitlines(), args)

    def setup_tools(self, mode, lld_banner, *, expect_ok=True):
        artifacts = self.root / "oracle outputs"
        sources = self.root / "sources"
        wrappers = self.root / "wrappers"
        sources.mkdir(exist_ok=True)
        lld = self.stub_bin / "ld.lld"
        self.script(lld, "echo '" + lld_banner + "'")
        archive = sources / "mold-3.0.0.tar.gz"
        if not archive.exists():
            with tarfile.open(archive, "w:gz") as tf:
                tf.add(self.root / "install-mold.sh", "mold-3.0.0/install-mold.sh")
        env = dict(self.env, ARTIFACTS=str(artifacts), PREFIX=str(artifacts),
                   SRC=str(sources), LINKDIR=str(wrappers), LLD_BIN=str(lld),
                   WITH_WILD="0", MOLD_PREFER=mode, ORACLE_CACHE="", JOBS="2")
        run = subprocess.run(["bash", str(self.root / "tools/linker/setup_oracles.sh")],
                             cwd=self.root, env=env, capture_output=True, text=True, timeout=15)
        if not expect_ok:
            self.assertEqual(run.returncode, 1, run.stdout + run.stderr)
            self.assertIn("version pin violated", run.stdout)
            self.assertFalse((artifacts / "ORACLES.lock").exists())
            return None
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        banner = subprocess.check_output([wrappers / "mold-3.0.0", "--version"], text=True)
        self.assertIn("source-build" if mode == "source" else "release-cache", banner)
        return (artifacts / "ORACLES.lock").read_text()

    def test_tools_wrapper_and_inventory_track_the_selected_mode(self):
        lld = "Debian LLD 23.1.5 (compatible with GNU linkers)"
        self.assertIn("release-cache", self.setup_tools("cache", lld))
        result = self.setup_tools("source", lld)
        self.assertIn("source-build", result)
        self.assertIn("features=x86_64,i386,system-allocator", result)
        self.assertIn(lld, result)
        self.assertIn("release-cache", self.setup_tools("cache", lld))
        self.assertIn("source-build", self.setup_tools("source", lld))
        self.assertEqual(len(self.cargo_log.read_text().splitlines()), 1)

    def test_tools_do_not_accept_a_prerelease_or_wrong_lld_branch(self):
        for banner in ("LLD 23.1.0-rc1", "LLD 23.10.0", "LLD 23.1.old"):
            with self.subTest(banner=banner):
                self.setup_tools("cache", banner, expect_ok=False)
