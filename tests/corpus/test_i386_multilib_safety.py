"""Offline contract tests preventing the -m32 libc container-harness trap."""

import importlib.util
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SAFE_SCRIPT = ROOT / "scripts/ensure_i386_multilib_safe.sh"
RESTORE_PY = ROOT / "scripts/restore_agent_toolchain_cache.py"
ARENA_RESTORE = ROOT / "scripts/arena_session_restore.sh"


def _load_restore_module():
    spec = importlib.util.spec_from_file_location("restore_agent_toolchain_cache", RESTORE_PY)
    assert spec and spec.loader
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


class I386MultilibSafetyTests(unittest.TestCase):
    def test_safe_script_self_test_passes(self):
        proc = subprocess.run(
            ["bash", str(SAFE_SCRIPT), "--self-test"],
            cwd=ROOT,
            text=True,
            capture_output=True,
            timeout=30,
        )
        self.assertEqual(proc.returncode, 0, proc.stderr + proc.stdout)
        self.assertIn("self-test PASSED", proc.stdout)

    def test_audit_apt_sim_rejects_live_libc_upgrades_and_foreign_i386(self):
        with tempfile.TemporaryDirectory() as td:
            tmp = Path(td)
            cases = {
                "libc6_upgrade": (
                    "Inst libc6 [2.41-12] (2.41-12+deb13u4 Debian-Security:13 [amd64])\n"
                    "Inst libc6-i386 (2.41-12+deb13u4 Debian-Security:13 [amd64])\n",
                    False,
                ),
                "foreign_i386": (
                    "Inst linux-libc-dev:i386 (6.12.111-1 Debian:13 [i386])\n",
                    False,
                ),
                "cross_removal": (
                    "Remv gcc-aarch64-linux-gnu [4:14.2.0-1]\n"
                    "Inst gcc-multilib (4:14.2.0-1 Debian:13 [amd64])\n",
                    False,
                ),
                "pure_biarch_install": (
                    "Inst libc6-i386 (2.41-12+deb13u4 Debian:13 [amd64])\n"
                    "Inst libc6-dev-i386 (2.41-12+deb13u4 Debian:13 [amd64])\n"
                    "Inst lib32gcc-14-dev (14.2.0-19 Debian:13 [amd64])\n"
                    "Inst gcc-multilib (4:14.2.0-1 Debian:13 [amd64])\n",
                    True,
                ),
            }
            for name, (content, should_pass) in cases.items():
                sim = tmp / f"{name}.sim"
                sim.write_text(content)
                proc = subprocess.run(
                    ["bash", str(SAFE_SCRIPT), "--audit-apt-sim", str(sim)],
                    cwd=ROOT,
                    text=True,
                    capture_output=True,
                    timeout=15,
                )
                if should_pass:
                    self.assertEqual(proc.returncode, 0, f"{name}: {proc.stderr}")
                else:
                    self.assertNotEqual(proc.returncode, 0, f"{name} unexpectedly passed")

    def test_validate_sysroot_path_rejects_os_root_and_snapshot_workspace(self):
        for unsafe in ("/", "/usr", "/lib", "/lib32", "/lib64", "/etc", "/home/user", "/home/user/sysroot"):
            proc = subprocess.run(
                ["bash", str(SAFE_SCRIPT), "--validate-sysroot-path", unsafe],
                cwd=ROOT,
                text=True,
                capture_output=True,
                timeout=15,
            )
            self.assertNotEqual(proc.returncode, 0, f"accepted unsafe sysroot {unsafe}")

    def test_validate_sysroot_path_rejects_every_shared_top_level_tree(self):
        """Staging *replaces* the sysroot, so a shared tree must be refused.

        The first implementation used a denylist of the obvious system paths
        and then `rm -rf "$sysroot"`.  `LCCC_I686_SYSROOT=/tmp` (or `/var`, or
        `/opt`) passed that list and would have deleted the directory wholesale.
        A dedicated sysroot is at least two components deep; every legitimate
        path satisfies that, and no shared top-level tree does.
        """
        unsafe = ("/tmp", "/var", "/opt", "/srv", "/root", "/home", "/mnt",
                  "/media", "/dev", "/proc", "/sys", "/boot", "/run",
                  "/dev/shm", "/proc/self", "/sys/kernel", "/run/user")
        for path in unsafe:
            proc = subprocess.run(
                ["bash", str(SAFE_SCRIPT), "--validate-sysroot-path", path],
                cwd=ROOT, text=True, capture_output=True, timeout=15)
            self.assertNotEqual(proc.returncode, 0, f"accepted shared tree {path}")
        for path in ("/var/tmp/lccc-i686-sysroot", "/opt/lccc-sysroot",
                     "/var/tmp/nested/deep/sysroot"):
            proc = subprocess.run(
                ["bash", str(SAFE_SCRIPT), "--validate-sysroot-path", path],
                cwd=ROOT, text=True, capture_output=True, timeout=15)
            self.assertEqual(proc.returncode, 0,
                             f"refused legitimate sysroot {path}: {proc.stderr}")

    def test_staging_never_replaces_a_directory_it_did_not_create(self):
        """`rm -rf` may only ever run on a tree proven to be ours."""
        script = SAFE_SCRIPT.read_text(encoding="utf-8")
        self.assertIn("assert_replaceable_sysroot()", script)
        self.assertIn('assert_replaceable_sysroot "$sysroot"', script)
        # The guard must run BEFORE the removal, not after it.
        self.assertLess(script.index('assert_replaceable_sysroot "$sysroot"'),
                        script.index('rm -rf "$sysroot"'))
        with tempfile.TemporaryDirectory(dir="/var/tmp") as tmp:
            root = Path(tmp)
            populated = root / "someone-elses-data"
            populated.mkdir()
            (populated / "precious").write_text("do not delete")
            probe = (
                f'source "{SAFE_SCRIPT}" --print-env >/dev/null 2>&1; '
                f'assert_replaceable_sysroot "{populated}"'
            )
            proc = subprocess.run(["bash", "-c", probe], cwd=ROOT, text=True,
                                  capture_output=True, timeout=20)
            self.assertNotEqual(proc.returncode, 0, "accepted a populated foreign tree")
            self.assertTrue((populated / "precious").exists(), "the guard deleted data")
            # Empty, absent and marker-carrying trees are all replaceable.
            for name, marker in (("empty", False), ("ours", True)):
                candidate = root / name
                candidate.mkdir()
                if marker:
                    (candidate / "README.DO_NOT_COPY_TO_ROOT.txt").write_text("ours")
                proc = subprocess.run(
                    ["bash", "-c", f'source "{SAFE_SCRIPT}" --print-env >/dev/null 2>&1; '
                                   f'assert_replaceable_sysroot "{candidate}"'],
                    cwd=ROOT, text=True, capture_output=True, timeout=20)
                self.assertEqual(proc.returncode, 0, f"{name}: {proc.stderr}")
            proc = subprocess.run(
                ["bash", "-c", f'source "{SAFE_SCRIPT}" --print-env >/dev/null 2>&1; '
                               f'assert_replaceable_sysroot "{root / "absent"}"'],
                cwd=ROOT, text=True, capture_output=True, timeout=20)
            self.assertEqual(proc.returncode, 0, f"absent: {proc.stderr}")

    def test_generated_wrappers_embed_an_absolute_driver_path(self):
        """A wrapper must work outside the environment that created it.

        `exec gcc-16` resolved only while the toolchain's bin dir was on PATH;
        a gate running the wrapper from its own environment failed with
        `exec: gcc-16: not found` (measured).  The driver is now resolved to an
        absolute path at generation time.
        """
        script = SAFE_SCRIPT.read_text(encoding="utf-8")
        self.assertIn('resolved=$(type -P "$real"', script)
        self.assertIn("compiler not found on PATH", script)
        with tempfile.TemporaryDirectory(dir="/var/tmp") as tmp:
            root = Path(tmp)
            sysroot = root / "sysroot"
            (sysroot / "lib32").mkdir(parents=True)
            (sysroot / "lib32" / "ld-linux.so.2").write_text("")
            bindir = root / "toolchain" / "bin"
            bindir.mkdir(parents=True)
            driver = bindir / "lccc-wrapper-probe-cc"
            marker = root / "driver-ran"
            driver.write_text(
                "#!/bin/sh\n"
                'if [ "${1:-}" = -print-multiarch ]; then echo x86_64-linux-gnu; exit 0; fi\n'
                f'printf "%s\\n" "$@" > "{marker}"\n'
            )
            driver.chmod(0o755)
            # A *bare* driver name is the regression: passing an already
            # absolute path would pass even if resolution were removed.
            probe = (
                f'source "{SAFE_SCRIPT}" --print-env >/dev/null 2>&1; '
                # A shell function must not masquerade as the driver file.
                'lccc-wrapper-probe-cc() { :; }; '
                f'write_m32_wrappers "{sysroot}" lccc-wrapper-probe-cc lccc-wrapper-probe-cc; '
                f'sed -n "4p" "{sysroot}/bin/m32-cc"'
            )
            env = dict(os.environ, PATH=f"{bindir}:{os.environ['PATH']}")
            proc = subprocess.run(["bash", "-c", probe], cwd=ROOT, text=True,
                                  capture_output=True, timeout=20, env=env)
            self.assertEqual(proc.returncode, 0, proc.stderr + proc.stdout)
            self.assertTrue(proc.stdout.startswith(f"exec {driver} "),
                            f"wrapper does not exec the resolved driver: {proc.stdout}")
            # Execute the wrapper with the generating toolchain removed from
            # PATH.  Inspecting the text alone would not catch a broken exec.
            clean = {"PATH": "/usr/bin:/bin"}
            run = subprocess.run([str(sysroot / "bin" / "m32-cc"), "a file.c"],
                                 cwd=root, text=True, capture_output=True,
                                 timeout=20, env=clean)
            self.assertEqual(run.returncode, 0, run.stderr + run.stdout)
            argv = marker.read_text().splitlines()
            self.assertIn("-m32", argv)
            self.assertEqual(argv[-1], "a file.c")

    def test_sourcing_exposes_helpers_without_running_the_cli(self):
        """A helper import must not provision, consult apt, or need env.sh."""
        with tempfile.TemporaryDirectory(dir="/var/tmp") as tmp:
            absent = Path(tmp) / "absent" / "sysroot"
            env = dict(os.environ, LCCC_I686_SYSROOT=str(absent))
            proc = subprocess.run(
                ["bash", "-c", f'source "{SAFE_SCRIPT}" --print-env; '
                               'type assert_replaceable_sysroot >/dev/null; '
                               'type write_m32_wrappers >/dev/null'],
                cwd=ROOT, text=True, capture_output=True, timeout=20, env=env)
            self.assertEqual(proc.returncode, 0, proc.stderr + proc.stdout)
            self.assertEqual(proc.stdout, "")
            self.assertFalse(absent.exists())

    def test_restore_agent_toolchain_cache_refuses_live_root_prefixes(self):
        mod = _load_restore_module()
        for bad in (Path("/"), Path("/usr"), Path("/lib"), Path("/lib64"), Path("/usr/local")):
            with self.assertRaises(RuntimeError, msg=f"accepted {bad}"):
                mod.validate_restore_prefix(bad)

    def test_arena_session_restore_never_runs_unconstrained_multilib_apt(self):
        text = ARENA_RESTORE.read_text(encoding="utf-8")
        self.assertIn("ensure_i386_multilib_safe.sh", text)
        self.assertIn("--no-upgrade", text)
        self.assertNotIn("dpkg --add-architecture i386", text)


if __name__ == "__main__":
    unittest.main()
