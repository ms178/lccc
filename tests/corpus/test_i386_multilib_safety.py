"""Offline contract tests preventing the -m32 libc container-harness trap."""

import importlib.util
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
