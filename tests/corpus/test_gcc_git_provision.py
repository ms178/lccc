"""Offline contracts for the pinned GCC development-corpus provisioner."""
import importlib.util
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("ensure_gcc_torture_git", ROOT / "scripts/ensure_gcc_torture_git.py")
provisioner = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = provisioner
spec.loader.exec_module(provisioner)


class ProvisionContract(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.upstream = self.root / "upstream"
        self.upstream.mkdir()
        self.destination = self.root / "cache"
        self.git("init", "-q", "-b", "master")
        self.git("config", "user.name", "Fixture")
        self.git("config", "user.email", "fixture@example.invalid")
        for name, text in {
            "gcc/BASE-VER": "17.0.0\n",
            "gcc/testsuite/gcc.c-torture/execute/ok.c": "int main(void) { return 0; }\n",
            "gcc/testsuite/gcc.c-torture/compile/ok.c": "int f(void) { return 1; }\n",
            "gcc/testsuite/gcc.dg/support.h": "/* fixture */\n",
            "gcc/testsuite/lib/gcc-dg.exp": "# fixture\n",
        }.items():
            path = self.upstream / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text)
        self.git("add", ".")
        self.git("commit", "-qm", "fixture")
        self.commit = self.git("rev-parse", "HEAD").strip()

    def git(self, *args):
        return subprocess.check_output(["git", "-C", str(self.upstream), *args], text=True)

    def provision(self, ref="refs/heads/master"):
        return provisioner.provision(str(self.upstream), ref, self.destination)

    def test_option_shaped_ref_never_reaches_git(self):
        # `--ref` is threaded into `git ls-remote ... <ref>`, so a value that starts
        # with a dash is an *option* to git, not a name: `--upload-pack=<cmd>` is the
        # classic way a "ref" becomes a subprocess.  The guard has to reject it
        # before anything is spawned, and it must not merely fail -- a run that
        # crashed after invoking git would leave the marker below behind.
        marker = self.root / "spawned"
        real = subprocess.check_output

        def spy(cmd, *args, **kwargs):
            if cmd and cmd[0] == "git":
                marker.write_text("git was invoked")
            return real(cmd, *args, **kwargs)

        with mock.patch.object(subprocess, "check_output", side_effect=spy):
            for hostile in ("--upload-pack=touch /tmp/nope", "-x", "", "refs/heads/m ast"):
                with self.subTest(ref=hostile):
                    with self.assertRaises(ValueError) as ctx:
                        provisioner.resolve(str(self.upstream), hostile)
                    self.assertIn("--ref", str(ctx.exception))
            # A repository spelled as an option is refused for the same reason.
            with self.assertRaises(ValueError):
                provisioner.resolve("--upload-pack=echo", "refs/heads/master")
        self.assertFalse(marker.exists(), "a refused value still reached git")
        # Positive control: the shapes the guard must accept still resolve.
        self.assertEqual(provisioner.resolve(str(self.upstream), "refs/heads/master"), self.commit)
        self.assertEqual(provisioner.resolve(str(self.upstream), self.commit.upper()), self.commit)

    def test_pinned_checkout_stamp_and_idempotence(self):
        first = self.provision()
        self.assertEqual(first["commit"], self.commit)
        self.assertEqual(first["counts"], dict(compile=1, execute=1))
        stamp = self.destination / "gcc/testsuite/.lccc-provisioned"
        self.assertIn(self.commit, stamp.read_text())
        with mock.patch.object(provisioner.publication, "publish_directory") as publish:
            self.assertEqual(self.provision(self.commit), first)
            publish.assert_not_called()

    def test_missing_changed_and_extra_sources_repair(self):
        initial = self.provision()
        source = self.destination / "gcc/testsuite/gcc.c-torture/execute/ok.c"
        source.write_text("truncated")
        self.assertEqual(self.provision(), initial)
        source.unlink()
        self.assertEqual(self.provision(), initial)
        extra = source.with_name("stray.c")
        extra.write_text("int wrong;\n")
        self.assertEqual(self.provision(), initial)
        self.assertFalse(extra.exists())

    def test_failed_fetch_preserves_previous_tree(self):
        initial = self.provision()
        with mock.patch.object(provisioner, "resolve", return_value="1" * 40):
            with self.assertRaises(subprocess.SubprocessError):
                self.provision()
        self.assertEqual(provisioner.verify(self.destination, self.commit, 17)["manifest_sha256"],
                         initial["manifest_sha256"])
        self.assertFalse(list(self.root.glob(".gcc17-stage-*")))

    def test_wrong_major_and_symlink_are_rejected(self):
        self.provision()
        with self.assertRaisesRegex(ValueError, "expected GCC 18"):
            provisioner.verify(self.destination, self.commit, 18)
        alias = self.root / "alias"
        alias.symlink_to(self.destination, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, "symlink"):
            provisioner.provision(str(self.upstream), self.commit, alias)

    def test_ambiguous_ref_rejected(self):
        with mock.patch.object(provisioner.subprocess, "check_output", return_value=(
                "a" * 40 + "\trefs/heads/master\n" + "b" * 40 + "\trefs/heads/other\n").encode()):
            with self.assertRaisesRegex(ValueError, "unambiguously"):
                provisioner.resolve("unused", "refs/heads/*")


if __name__ == "__main__":
    unittest.main()
