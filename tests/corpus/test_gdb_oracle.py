#!/usr/bin/env python3
"""Regression tests for the pinned GDB oracle policy.

The DWARF gates used to resolve a debugger opportunistically and return
success when it was absent -- a green run that never asked a debugger
anything. `tests/regression/lib/gdb_oracle.sh` replaces that with a single
fail-closed policy shared by shell and Python consumers. These tests drive
every branch of that policy with synthetic debuggers, so they are
deterministic on hosts with any (or no) gdb installed, and they assert the
static wiring that keeps the two consumers and all three CI invocations from
drifting back to `shutil.which("gdb")`.
"""

from __future__ import annotations

import importlib.util
import os
import shlex
import shutil
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
LIB = ROOT / "tests/regression/lib/gdb_oracle.sh"
DWARF_GATE = ROOT / "tests/regression/check_dwarf_line_tables.py"
BYVAL_GATE = ROOT / "tests/regression/check_incoming_byval_homes.sh"
CI_WORKFLOW = ROOT / ".github/workflows/ci.yml"
LANE_PLAN = ROOT / ".github/ci-lane-plan.json"
LOCAL_MIRROR = ROOT / "scripts/ci_local.sh"

BANNER = "GNU gdb (GDB) 18.1"
BASH = shutil.which("bash") or "/bin/bash"
HOSTED_COMMAND = (
    "python3 tests/regression/check_dwarf_line_tables.py "
    "--lccc target/fastbuild/lccc --lccc-i686 target/fastbuild/lccc-i686 "
    "--gcc gcc-16 --allow-missing-debugger"
)


def stub_debugger(directory: Path, first_line: str, name: str = "gdb",
                  *, executable: bool = True) -> Path:
    """A fake `gdb --version`: an exact first line plus a long copyright tail.

    The tail matters: a resolver built on a truncated pipe (`| head -n1`)
    can die from SIGPIPE under `set -o pipefail`. The library uses command
    substitution + parameter expansion instead, which must handle it.
    """
    path = directory / name
    tail = "\n".join(f"Copyright (C) 2026 line {i}" for i in range(40))
    path.write_text(
        "#!/bin/sh\n"
        f"printf '%s\\n' {shlex.quote(first_line)}\n"
        f"printf '%s\\n' {shlex.quote(tail)}\n",
        encoding="utf-8",
    )
    if executable:
        path.chmod(path.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
    return path


class GdbOracleCli(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory(prefix="lccc-gdb-oracle-")
        self.addCleanup(self._tmp.cleanup)
        self.dir = Path(self._tmp.name)

    def cli(self, *args: str, env_extra: dict[str, str] | None = None,
            include_system_path: bool = False) -> subprocess.CompletedProcess[str]:
        env = {k: v for k, v in os.environ.items() if not k.startswith("GDB")}
        # A PATH containing only the fixture dir makes "no debugger on PATH"
        # deterministic even on CI images that ship a distribution gdb.
        parts = [str(self.dir)]
        if include_system_path:
            parts += ["/usr/bin", "/bin"]
        env["PATH"] = os.pathsep.join(parts)
        env.update(env_extra or {})
        return subprocess.run(
            [BASH, str(LIB), "--print", *args],
            env=env, text=True, capture_output=True, check=False,
        )

    # ── resolution: the happy paths ──────────────────────────────────────
    def test_exact_banner_on_path_is_resolved(self) -> None:
        stub = stub_debugger(self.dir, BANNER)
        done = self.cli()
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertEqual(done.stdout.strip(), str(stub))
        self.assertTrue(Path(done.stdout.strip()).is_absolute())
        self.assertEqual(done.stderr, "")

    def test_explicit_gdb_variable_is_resolved(self) -> None:
        stub = stub_debugger(self.dir, BANNER, name="my-pinned-gdb")
        done = self.cli(env_extra={"GDB": str(stub)})
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertEqual(done.stdout.strip(), str(stub))

    def test_long_version_tail_does_not_break_banner_parsing(self) -> None:
        stub_debugger(self.dir, BANNER)
        done = self.cli()
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertNotIn("Copyright", done.stdout)

    # ── fail closed, explicit skip ───────────────────────────────────────
    def test_missing_debugger_fails_closed(self) -> None:
        done = self.cli()
        self.assertEqual(done.returncode, 1, done.stderr)
        self.assertEqual(done.stdout, "")
        self.assertIn("FAIL:", done.stderr)
        self.assertIn("18.1", done.stderr)
        self.assertNotIn("SKIP:", done.stderr)

    def test_missing_debugger_skips_only_when_explicitly_allowed(self) -> None:
        done = self.cli("--allow-missing-debugger")
        self.assertEqual(done.returncode, 3, done.stderr)
        self.assertEqual(done.stdout, "")
        self.assertIn("SKIP:", done.stderr)
        self.assertIn("--allow-missing-debugger", done.stderr)
        self.assertIn("will NOT run", done.stderr)

    def test_unpinned_debugger_is_never_a_substitute(self) -> None:
        stub_debugger(self.dir, "GNU gdb (Debian 15.2-1) 15.2")
        done = self.cli()
        self.assertEqual(done.returncode, 1, done.stderr)
        self.assertEqual(done.stdout, "")
        self.assertIn("unpinned", done.stderr)

    def test_unpinned_debugger_with_escape_is_an_explicit_skip(self) -> None:
        stub_debugger(self.dir, "GNU gdb (Debian 15.2-1) 15.2")
        done = self.cli("--allow-missing-debugger")
        self.assertEqual(done.returncode, 3, done.stderr)
        self.assertIn("SKIP:", done.stderr)
        self.assertIn("unpinned", done.stderr)

    # ── an explicit choice is never replaced or downgraded ───────────────
    def test_explicit_wrong_gdb_cannot_be_downgraded_to_a_skip(self) -> None:
        stub = stub_debugger(self.dir, "GNU gdb (GDB) 17.1", name="wrong")
        done = self.cli("--allow-missing-debugger", env_extra={"GDB": str(stub)})
        self.assertEqual(done.returncode, 1, done.stderr)
        self.assertIn("never substituted", done.stderr)
        self.assertNotIn("SKIP:", done.stderr)

    def test_explicit_nonexistent_gdb_fails(self) -> None:
        done = self.cli("--allow-missing-debugger",
                        env_extra={"GDB": str(self.dir / "does-not-exist")})
        self.assertEqual(done.returncode, 1, done.stderr)
        self.assertIn("not an executable", done.stderr)

    def test_non_executable_explicit_gdb_fails(self) -> None:
        stub = stub_debugger(self.dir, BANNER, name="not-exec", executable=False)
        done = self.cli(env_extra={"GDB": str(stub)})
        self.assertEqual(done.returncode, 1, done.stderr)
        self.assertIn("not an executable", done.stderr)

    # ── exactness of the banner ──────────────────────────────────────────
    def test_banner_with_trailing_text_is_rejected(self) -> None:
        stub_debugger(self.dir, BANNER + " (Debian patched)")
        done = self.cli()
        self.assertEqual(done.returncode, 1, done.stderr)
        self.assertIn("unpinned", done.stderr)

    def test_empty_banner_is_rejected(self) -> None:
        stub_debugger(self.dir, "")
        done = self.cli()
        self.assertEqual(done.returncode, 1, done.stderr)

    def test_failing_version_command_is_rejected(self) -> None:
        path = self.dir / "gdb"
        path.write_text("#!/bin/sh\nexit 7\n", encoding="utf-8")
        path.chmod(0o755)
        done = self.cli()
        self.assertEqual(done.returncode, 1, done.stderr)

    # ── CLI contract ─────────────────────────────────────────────────────
    def test_unknown_argument_is_rejected(self) -> None:
        done = self.cli("--definitely-not-an-option")
        self.assertEqual(done.returncode, 2, done.stderr)

    def test_cli_without_print_is_rejected(self) -> None:
        env = {"PATH": os.pathsep.join([str(self.dir), "/usr/bin", "/bin"])}
        done = subprocess.run([BASH, str(LIB)], env=env, text=True,
                              capture_output=True, check=False)
        self.assertEqual(done.returncode, 2, done.stderr)

    def test_help_exits_zero(self) -> None:
        env = {"PATH": os.pathsep.join([str(self.dir), "/usr/bin", "/bin"])}
        done = subprocess.run([BASH, str(LIB), "--help"], env=env, text=True,
                              capture_output=True, check=False)
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertIn("18.1", done.stdout)

    # ── sourceable library contract ──────────────────────────────────────
    def sourced(self, banner: str | None, allow: bool) -> subprocess.CompletedProcess[str]:
        if banner is not None:
            stub_debugger(self.dir, banner)
        flag = " --allow-missing-debugger" if allow else ""
        script = (
            f'. {shlex.quote(str(LIB))}\n'
            f'rc=0\n'
            f'require_gdb_oracle probe-gate{flag} || rc=$?\n'
            f'printf "rc=%s bin=%s\\n" "$rc" "$GDB_ORACLE_BIN"\n'
        )
        env = {"PATH": str(self.dir)}
        return subprocess.run([BASH, "-c", script], env=env, text=True,
                              capture_output=True, check=False)

    def test_sourced_library_resolves(self) -> None:
        done = self.sourced(BANNER, allow=False)
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertIn("rc=0 bin=", done.stdout)
        self.assertNotIn("bin=\n", done.stdout)

    def test_sourced_library_fails_closed(self) -> None:
        done = self.sourced(None, allow=False)
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertIn("rc=1 bin=\n", done.stdout)

    def test_sourced_library_explicit_skip(self) -> None:
        done = self.sourced(None, allow=True)
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertIn("rc=3 bin=\n", done.stdout)


class GdbOracleWiring(unittest.TestCase):
    """A policy nobody checks is a policy that will be removed by accident."""

    @staticmethod
    def text(path: Path) -> str:
        return path.read_text(encoding="utf-8")

    def test_dwarf_gate_no_longer_resolves_gdb_opportunistically(self) -> None:
        text = self.text(DWARF_GATE)
        self.assertNotIn('shutil.which("gdb")', text)
        self.assertIn("GDB_ORACLE_LIB", text)
        self.assertIn("resolve_gdb_oracle(", text)
        self.assertIn("--allow-missing-debugger", text)
        self.assertNotIn("gdb not installed, debugger check skipped", text)

    def test_dwarf_gate_helpers_receive_the_resolved_oracle(self) -> None:
        text = self.text(DWARF_GATE)
        self.assertIn("def require_debugger_resolves(gdb: str | None", text)
        self.assertIn("def require_debugger_breaks_function(gdb: str | None", text)
        self.assertIn("resolve_gdb_oracle(allow_missing=", text)

    def test_byval_forensics_uses_the_pinned_oracle_only(self) -> None:
        text = self.text(BYVAL_GATE)
        self.assertIn("require_gdb_oracle", text)
        self.assertIn("GDB_ORACLE_BIN", text)
        self.assertNotIn("command -v gdb", text)

    def test_escape_is_explicit_in_hosted_workflow_plan_and_local_mirror(self) -> None:
        self.assertIn(HOSTED_COMMAND, self.text(CI_WORKFLOW))
        self.assertIn(HOSTED_COMMAND, self.text(LANE_PLAN))
        self.assertIn("--allow-missing-debugger", self.text(LOCAL_MIRROR))
        local = self.text(LOCAL_MIRROR)
        self.assertIn('--gcc "${GCC16:-gcc-16}" --allow-missing-debugger', local)


class GdbOraclePythonResolver(unittest.TestCase):
    """The Python adapter must fail closed even if the CLI misbehaves."""

    @classmethod
    def setUpClass(cls) -> None:
        spec = importlib.util.spec_from_file_location("lccc_dwarf_gate", DWARF_GATE)
        assert spec and spec.loader
        cls.mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cls.mod)

    def resolve(self, rc: int, stdout: str, stderr: str = "") -> object:
        fake = subprocess.CompletedProcess([], rc, stdout, stderr)
        with mock.patch.object(self.mod.subprocess, "run", return_value=fake) as run:
            result = self.mod.resolve_gdb_oracle(allow_missing=False)
        return result, run

    def test_success_returns_absolute_executable_path(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            exe = Path(tmp) / "gdb"
            exe.write_text("#!/bin/sh\n", encoding="utf-8")
            exe.chmod(0o755)
            result, run = self.resolve(0, f"{exe}\n")
            self.assertEqual(result, str(exe))
            self.assertIn("--print", run.call_args.args[0])

    def test_explicit_skip_returns_none(self) -> None:
        result, _ = self.resolve(3, "", "SKIP: explicit\n")
        self.assertIsNone(result)

    def test_hard_failure_raises(self) -> None:
        with self.assertRaises(self.mod.GateFailure):
            self.resolve(1, "", "FAIL: no oracle")

    def test_unusable_path_fails_closed(self) -> None:
        with self.assertRaises(self.mod.GateFailure):
            self.resolve(0, "relative/gdb\n")
        with self.assertRaises(self.mod.GateFailure):
            self.resolve(0, "\n")

    def test_escape_is_forwarded_to_policy(self) -> None:
        fake = subprocess.CompletedProcess([], 3, "", "SKIP: explicit\n")
        with mock.patch.object(self.mod.subprocess, "run", return_value=fake) as run:
            self.mod.resolve_gdb_oracle(allow_missing=True)
        self.assertIn("--allow-missing-debugger", run.call_args.args[0])


if __name__ == "__main__":
    unittest.main()
