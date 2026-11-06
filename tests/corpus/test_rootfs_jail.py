#!/usr/bin/env python3
"""Empirical regression tests for the live-root write catastrophe.

Three agent sessions destroyed a usr-merged sandbox by extracting a Debian
biarch payload onto ``/``: ``/lib -> usr/lib`` was replaced by an i386 directory
tree, ``/lib64/ld-linux-x86-64.so.2`` disappeared, and every subsequent
``execve`` failed -- including the tools needed to repair the machine.

These tests prove the structural fix instead of asserting it:

* the jail refuses the write and the host root is bit-for-bit unchanged;
* the jail fails *closed* (payload never runs) when it cannot protect the host;
* the static policy gate rejects the exact historical command and accepts the
  safe replacements, with a mutation-verified detector self-test;
* the "environment recovery" script -- the one an agent runs first after a
  harness wipe, and the actual recurrence vector -- no longer contains an
  unaudited multilib transaction.

Safety of the tests themselves: the dynamic jail probe extracts a payload whose
only member is a *benign canary* at the live root (``/lccc-jail-canary-<pid>``),
never ``lib32/`` or ``lib64/``.  A hypothetical jail regression therefore leaves
a stray file and a red test instead of an unbootable sandbox.  The real
catastrophe string is covered statically by the policy gate.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
JAIL = ROOT / "scripts/rootfs_jail.sh"
POLICY = ROOT / "scripts/check_rootfs_write_policy.py"
SAFE = ROOT / "scripts/ensure_i386_multilib_safe.sh"
RECOVER = ROOT / "tools/linker/recover_env.sh"


def run(argv: list[str], **kw) -> subprocess.CompletedProcess:
    return subprocess.run(argv, capture_output=True, text=True, timeout=180, **kw)


def root_state() -> dict[str, str]:
    """Identity of every load-bearing live-root entry, before and after."""
    state = {}
    for name in ("lib", "lib64", "lib32", "libx32", "bin", "sbin", "usr"):
        p = Path("/") / name
        if p.is_symlink():
            state[name] = f"symlink:{os.readlink(p)}"
        elif p.exists():
            st = p.stat()
            state[name] = f"dir:{st.st_ino}:{st.st_mode}"
        else:
            state[name] = "absent"
    return state


class RootfsJailTests(unittest.TestCase):
    def test_jail_is_wired_into_the_provisioner(self):
        text = SAFE.read_text()
        self.assertIn('rootfs_jail.sh', text)
        self.assertIn('jail_available', text)
        self.assertIn('jail_run -- dpkg-deb -x', text)
        # The jail must have no off-switch: an env var that disables it would be
        # the first thing a future "make it work" change reaches for.
        self.assertNotIn("LCCC_JAIL_DISABLE", text)
        self.assertNotIn("LCCC_JAIL_DISABLE", JAIL.read_text())

    def test_jail_self_test_passes(self):
        proc = run(["bash", str(JAIL), "--self-test"])
        if proc.returncode == 3:
            avail = run(["bash", str(JAIL), "--available"])
            self.assertEqual(avail.returncode, 3,
                             "self-test reported unavailable but --available disagreed")
            self.skipTest("user namespaces unavailable: jail correctly fails closed")
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        self.assertIn("PASS", proc.stdout + proc.stderr)

    def test_jail_blocks_a_live_root_write_and_host_is_unchanged(self):
        if not shutil.which("dpkg-deb"):
            self.skipTest("dpkg-deb not installed")
        avail = run(["bash", str(JAIL), "--available"])
        if avail.returncode != 0:
            self.skipTest("user namespaces unavailable: covered by fail-closed test")
        # Prove the guard before exercising it.
        self.assertEqual(run(["bash", str(JAIL), "--self-test"]).returncode, 0)

        before = root_state()
        canary = f"lccc-jail-canary-{os.getpid()}"
        with tempfile.TemporaryDirectory(dir="/var/tmp") as tmp:
            payload = Path(tmp) / "payload"
            (payload / "DEBIAN").mkdir(parents=True)
            (payload / "DEBIAN" / "control").write_text(
                "Package: lccc-jail-canary\nVersion: 1.0\nArchitecture: amd64\n"
                "Maintainer: lccc <lccc@example.invalid>\n"
                "Description: benign live-root write probe\n")
            (payload / canary).write_text("must never reach the live root\n")
            deb = Path(tmp) / "canary.deb"
            built = run(["dpkg-deb", "--build", "-Znone", str(payload), str(deb)])
            self.assertEqual(built.returncode, 0, built.stderr)

            proc = run(["bash", str(JAIL), "--", "exec",
                        "dpkg-deb", "-x", str(deb), "/"])
            self.assertNotEqual(proc.returncode, 0,
                                "a live-root extraction succeeded inside the jail")
            self.assertFalse((Path("/") / canary).exists(),
                             "jail leaked a write to the live root")
        self.assertEqual(root_state(), before, "live root identity changed")
        self.assertEqual(run(["/bin/sh", "-c", "true"]).returncode, 0,
                         "execve is broken -- the sandbox is damaged")

    def test_jail_fails_closed_without_unshare(self):
        with tempfile.TemporaryDirectory(dir="/var/tmp") as tmp:
            bindir = Path(tmp) / "bin"
            bindir.mkdir()
            # A PATH with the payload tool but no unshare: the jail must refuse
            # to run the payload at all rather than "best effort" it.
            for tool in ("dpkg-deb", "sh", "cat", "rm", "mktemp", "dirname",
                         "readlink", "id", "touch", "sudo"):
                real = shutil.which(tool)
                if real:
                    (bindir / tool).symlink_to(real)
            marker = Path(tmp) / "payload-ran"
            env = dict(os.environ, PATH=str(bindir))
            env.pop("LCCC_JAIL_CANARY_PARENT", None)
            bash = shutil.which("bash") or "/bin/bash"
            proc = run([bash, str(JAIL), "--", "exec", "sh", "-c",
                        f"touch {marker}"], env=env)
            self.assertEqual(proc.returncode, 3, proc.stdout + proc.stderr)
            self.assertFalse(marker.exists(),
                             "payload ran even though the jail was unavailable")

    def test_guard_and_payload_share_one_namespace(self):
        """The guard must verify the namespace the payload actually runs in.

        A guard that proves the properties in one `unshare` and then opens a
        second one for the payload proves nothing about the payload.  This
        counts `unshare` invocations through a logging shim: the happy path
        must build exactly one namespace.
        """
        if not shutil.which("unshare"):
            self.skipTest("unshare not installed")
        real = shutil.which("unshare")
        with tempfile.TemporaryDirectory(dir="/var/tmp") as tmp:
            bindir = Path(tmp) / "bin"
            bindir.mkdir()
            log = Path(tmp) / "invocations"
            shim = bindir / "unshare"
            # One marker line per invocation: the guard script itself contains
            # newlines, so logging "$@" would count lines of shell, not calls.
            shim.write_text(f'#!/bin/sh\nprintf "invoked\\n" >> "{log}"\nexec "{real}" "$@"\n')
            shim.chmod(0o755)
            env = dict(os.environ, PATH=str(bindir) + os.pathsep + os.environ["PATH"])
            env.pop("LCCC_JAIL_CANARY_PARENT", None)
            bash = shutil.which("bash") or "/bin/bash"
            proc = run([bash, str(JAIL), "--", "exec", "true"], env=env)
            if proc.returncode == 3:
                self.skipTest("user namespaces unavailable")
            self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
            self.assertTrue(log.exists(), "the shim never ran")
            self.assertEqual(len(log.read_text().splitlines()), 1,
                             "guard and payload used different namespaces:\n" + log.read_text())

    def test_payload_exit_status_and_argv_are_preserved(self):
        avail = run(["bash", str(JAIL), "--available"])
        if avail.returncode != 0:
            self.skipTest("user namespaces unavailable")
        proc = run(["bash", str(JAIL), "--", "exec", "sh", "-c", "exit 7"])
        self.assertEqual(proc.returncode, 7, proc.stdout + proc.stderr)
        with tempfile.TemporaryDirectory(dir="/var/tmp") as tmp:
            target = Path(tmp) / "argv.txt"
            hostile = "a b;touch " + str(Path(tmp) / "injected")
            proc = run(["bash", str(JAIL), "--", "exec", "sh", "-c",
                        'printf "%s" "$1" > "$2"', "_", hostile, str(target)])
            self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
            self.assertEqual(target.read_text(), hostile)
            self.assertFalse((Path(tmp) / "injected").exists(),
                             "the payload argv was re-interpreted by a shell")

    def test_nonexistent_canary_parent_is_refused_not_certified(self):
        """A write that fails because the directory is missing proves nothing."""
        if not shutil.which("unshare"):
            self.skipTest("unshare not installed")
        with tempfile.TemporaryDirectory(dir="/var/tmp") as tmp:
            env = dict(os.environ, LCCC_JAIL_CANARY_PARENT=str(Path(tmp) / "absent"))
            proc = run(["bash", str(JAIL), "--available"], env=env)
            self.assertEqual(proc.returncode, 3,
                             "a missing canary parent must fail closed: "
                             + proc.stdout + proc.stderr)
            marker = Path(tmp) / "ran"
            proc = run(["bash", str(JAIL), "--", "exec", "sh", "-c",
                        f"touch {marker}"], env=env)
            self.assertEqual(proc.returncode, 3, proc.stdout + proc.stderr)
            self.assertFalse(marker.exists(), "payload ran under an unverified guard")

    def test_jail_keeps_caller_owned_trees_writable(self):
        avail = run(["bash", str(JAIL), "--available"])
        if avail.returncode != 0:
            self.skipTest("user namespaces unavailable")
        with tempfile.TemporaryDirectory(dir="/var/tmp") as tmp:
            target = Path(tmp) / "staged" / "lib32"
            proc = run(["bash", str(JAIL), "--", "exec", "sh", "-c",
                        f'mkdir -p "{target}" && echo ok > "{target}/ld-linux.so.2"'])
            self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
            self.assertEqual((target / "ld-linux.so.2").read_text().strip(), "ok")


class WritePolicyGateTests(unittest.TestCase):
    def test_detector_self_test_passes(self):
        proc = run(["python3", str(POLICY), "--self-test"])
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        self.assertIn("PASSED", proc.stdout)

    def test_repository_has_no_unguarded_live_root_writes(self):
        proc = run(["python3", str(POLICY)])
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        self.assertIn("PASS", proc.stdout)

    def test_gate_rejects_the_exact_historical_catastrophe(self):
        program = (
            "import sys; sys.path.insert(0, sys.argv[1]);"
            "import check_rootfs_write_policy as m;"
            "f = m.scan_line('x.sh', 1, sys.argv[2]);"
            "print(len(f));"
            "sys.exit(0 if len(f) == 1 and f[0].rule == 'live-root-extraction' else 1)"
        )
        catastrophe = ('sudo -n dpkg-deb -x "$D"/libc6-i386_2.36-9+deb12u14_amd64.deb /')
        proc = run(["python3", "-c", program, str(POLICY.parent), catastrophe])
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        self.assertEqual(proc.stdout.strip(), "1")


class RecoveryScriptTests(unittest.TestCase):
    def test_recovery_script_routes_32bit_through_the_audited_provisioner(self):
        text = RECOVER.read_text()
        code = "\n".join(line for line in text.splitlines()
                         if not line.strip().startswith("#"))
        self.assertIn("scripts/ensure_i386_multilib_safe.sh", code)
        self.assertNotIn("add-architecture", code)
        self.assertNotIn("apt-get install", code)
        self.assertNotIn("lib32gcc-14-dev", code)

    def test_recovery_script_uses_the_audited_4gib_swap_helper(self):
        code = "\n".join(line for line in RECOVER.read_text().splitlines()
                         if not line.strip().startswith("#"))
        self.assertIn("scripts/ci_enable_swap.sh", code)
        self.assertNotIn("8G", code)
        self.assertNotIn("count=8192", code)


if __name__ == "__main__":
    unittest.main()
