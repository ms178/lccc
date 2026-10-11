#!/usr/bin/env python3
"""Offline fixtures for the exact commit inventory, including adversarial paths."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).with_name("audit_levkropp_since.py").resolve()


class InventoryTest(unittest.TestCase):
    def test_exact_inventory(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            env = dict(os.environ, GIT_AUTHOR_NAME="test", GIT_COMMITTER_NAME="test",
                       GIT_AUTHOR_EMAIL="test@example.org", GIT_COMMITTER_EMAIL="test@example.org")

            def git(*args):
                return subprocess.check_output(["git", *args], cwd=root, env=env, text=True).strip()

            def write(name, text):
                (root / name).write_text(text)

            git("init", "-q")
            for name in ("same", "diverged", "parent", "deleted", "gone", "mode"):
                write(name, "before\n")
            git("add", ".")
            git("commit", "-qm", "parent")
            parent = git("rev-parse", "HEAD")
            for name in ("same", "diverged", "parent", "gone", "new\tpath\nname"):
                write(name, "incoming\n")
            (root / "deleted").unlink()
            (root / "mode").chmod(0o755)
            git("add", ".")
            git("commit", "-qm", "incoming")
            incoming = git("rev-parse", "HEAD")
            git("checkout", "-q", parent)
            write("same", "incoming\n")
            write("diverged", "local\n")
            (root / "deleted").unlink()
            (root / "gone").unlink()
            git("add", ".")
            git("commit", "-qm", "local")
            local = git("rev-parse", "HEAD")
            # Dirty checkout must not contaminate the committed comparison.
            write("same", "dirty\n")
            cmd = ["python3", str(SCRIPT), "--commit", incoming, "--against", local]
            out = subprocess.check_output(cmd, cwd=root, text=True)
            report = json.loads(out)
            self.assertEqual(report["parent"], parent)
            self.assertEqual(report["against"], local)
            self.assertEqual({r["path"]: r["status"] for r in report["paths"]}, {
                "same": "identical-entry", "diverged": "diverged-review-required",
                "parent": "parent-entry", "mode": "parent-entry",
                "deleted": "deletion-already-present", "gone": "absent-locally",
                "new\tpath\nname": "addition-absent"})
            self.assertEqual(sum(report["counts"].values()), 7)
            self.assertEqual(out, subprocess.check_output(cmd, cwd=root, text=True))
            bad = subprocess.run(["python3", str(SCRIPT), "--commit", "no-such-ref"],
                                 cwd=root, capture_output=True)
            self.assertNotEqual(bad.returncode, 0)
            self.assertEqual(bad.stdout, b"")


if __name__ == "__main__":
    unittest.main()
