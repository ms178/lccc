"""Exercise mold cache/source switching without building or fetching a linker."""

import hashlib
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
RELEASE_STUB = "#!/bin/sh\necho 'mold 3.0.0 release-cache'\n"


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
        self.release = self.root / "mold-release"
        self.release.write_text(RELEASE_STUB)
        # Exercise the real byte check against a small, offline release fixture.
        versions = self.root / "tools/linker/oracle_versions.sh"
        digest = "ada05c08a5686fdc8cf047f946f6c9b5163b7181ee7c500f7a904bcb750642a3"
        versions.write_text(versions.read_text().replace(digest, hashlib.sha256(self.release.read_bytes()).hexdigest()))
        self.script(self.root / "scripts/ensure_mold_300_binary.sh", """
set -eu
[ "$1" = --prefix ]
mkdir -p "$2/bin"
install -m755 "$MOLD_TEST_RELEASE" "$2/bin/mold"
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
        self.script(self.stub_bin / "cargo", """
set -eu
printf '%s\\n' "$*" >> "$MOLD_TEST_CARGO_LOG"
printf '%s\\n' "$RUSTFLAGS" >> "$MOLD_TEST_CARGO_LOG.rustflags"
""")
        self.cargo_log = self.root / "cargo.log"
        self.env = dict(os.environ, PATH=f"{self.stub_bin}{os.pathsep}{os.environ['PATH']}",
                        MOLD_REF="v3.0.0", MOLD_TEST_CARGO_LOG=str(self.cargo_log),
                        MOLD_TEST_INSTALL=str(self.root / "install-mold.sh"),
                        MOLD_TEST_RELEASE=str(self.release))

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
        self.assertEqual(self.cargo_log.with_suffix(".log.rustflags").read_text(),
                         "-C target-cpu=native\n")
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

    def test_warm_releases_are_verified_before_execution(self):
        lld = "LLD 23.1.5 (compatible with GNU linkers)"
        self.setup_mold("cache")
        self.setup_tools("cache", lld)
        invoked = self.root / "altered-release-ran"
        for setup, executable in (
            (lambda: self.setup_mold("cache"), self.prefix / "bin/mold-v3_0_0-release"),
            (lambda: self.setup_tools("cache", lld), self.root / "oracle outputs/mold-3.0.0-release/bin/mold"),
        ):
            with self.subTest(executable=executable):
                invoked.unlink(missing_ok=True)
                # Same version and the same banner, but not the restored bytes.
                self.script(executable, f"touch '{invoked}'\necho 'mold 3.0.0 release-cache'\n")
                setup()
                self.assertFalse(invoked.exists())
                self.assertEqual(executable.read_text(), RELEASE_STUB)
        self.assertFalse(self.cargo_log.exists())

    def test_tools_do_not_accept_a_prerelease_or_wrong_lld_branch(self):
        for banner in ("LLD 23.1.0-rc1", "LLD 23.10.0", "LLD 23.1.old",
                       "Debian LLD 22.0.1 (LLVM tools 23.1.5 installed)"):
            with self.subTest(banner=banner):
                self.setup_tools("cache", banner, expect_ok=False)
