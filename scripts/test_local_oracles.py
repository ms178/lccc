"""Guarantees of the host-run oracles in scripts/godbolt.py (LOCAL_ORACLES).

Run: python3 -m pytest -q scripts/test_local_oracles.py

A local oracle is opt-in (never a default, which must run on GitHub runners),
is identified by its own macro probe, and refuses any binary whose probe does
not match the pinned release. The real ICX check requires LCCC_TEST_REAL_ICX=1 and an explicit
LCCC_ICX_BIN. Ordinary test discovery never provisions or invokes a compiler.
"""
from __future__ import annotations

import os
import stat
import sys
import tempfile
import unittest
from unittest.mock import patch
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import codegen_oracle  # noqa: E402
import godbolt  # noqa: E402


def _fake_compiler(directory: Path, macro_value: str) -> Path:
    """An executable that answers `-E -dM` like a compiler with that macro."""
    script = directory / "fake-icx"
    script.write_text(
        "#!/bin/sh\n"
        f"echo '#define __INTEL_LLVM_COMPILER {macro_value}'\n"
    )
    script.chmod(script.stat().st_mode | stat.S_IXUSR)
    return script


class LocalOracleRegistry(unittest.TestCase):
    def test_local_oracles_are_never_defaults(self) -> None:
        for name in godbolt.LOCAL_ORACLES:
            self.assertNotIn(name, godbolt.DEFAULT_ORACLES)
            self.assertNotIn(name, codegen_oracle.DEFAULT_ORACLES)

    def test_local_names_do_not_alias_remote_channels(self) -> None:
        for name in godbolt.LOCAL_ORACLES:
            self.assertNotIn(name, godbolt.COMPILER_ALIASES)


class ProbeRefusal(unittest.TestCase):
    def setUp(self) -> None:
        self._env = os.environ.pop("LCCC_ICX_BIN", None)
        godbolt._LOCAL_BINARIES.pop("icx2026.2", None)
        self._tmp = tempfile.TemporaryDirectory(prefix="local-oracle-test-")

    def tearDown(self) -> None:
        if self._env is not None:
            os.environ["LCCC_ICX_BIN"] = self._env
        godbolt._LOCAL_BINARIES.pop("icx2026.2", None)
        self._tmp.cleanup()

    def test_wrong_release_is_refused_whatever_its_path(self) -> None:
        wrong = _fake_compiler(Path(self._tmp.name), "20240000")
        os.environ["LCCC_ICX_BIN"] = str(wrong)
        with self.assertRaisesRegex(godbolt.GodboltError, "probes"):
            godbolt.local_oracle_binary("icx2026.2")

    def test_matching_probe_is_accepted_and_digested(self) -> None:
        right = _fake_compiler(Path(self._tmp.name), "20260200")
        os.environ["LCCC_ICX_BIN"] = str(right)
        meta = godbolt.local_oracle_metadata("icx2026.2")
        self.assertEqual(meta["semver"], "2026.2.0")
        self.assertEqual(len(meta["binary_sha256"]), 64)
        self.assertEqual(meta["probe"], "__INTEL_LLVM_COMPILER=20260200")

    def test_local_oracle_is_refused_for_non_x86_arch(self) -> None:
        req = codegen_oracle.Request(
            source=Path(__file__), function="f", flags="-O3", local_flags=None, arch="aarch64",
        )
        with self.assertRaisesRegex(godbolt.GodboltError, "x86-64 oracle"):
            codegen_oracle._record_local_oracle("icx2026.2", req)

    def test_newer_patch_reports_actual_identity(self) -> None:
        binary = _fake_compiler(Path(self._tmp.name), "20260201")
        os.environ["LCCC_ICX_BIN"] = str(binary)
        self.assertEqual(godbolt.local_oracle_metadata("icx2026.2")["semver"], "2026.2.1")

    def test_replaced_binary_is_reprobed_and_refused(self) -> None:
        binary = _fake_compiler(Path(self._tmp.name), "20260200")
        os.environ["LCCC_ICX_BIN"] = str(binary)
        godbolt.local_oracle_binary("icx2026.2")
        _fake_compiler(Path(self._tmp.name), "20240000")
        with self.assertRaises(godbolt.GodboltError):
            godbolt.local_oracle_metadata("icx2026.2")

    def test_changed_environment_selects_new_binary(self) -> None:
        binary = _fake_compiler(Path(self._tmp.name), "20260200")
        os.environ["LCCC_ICX_BIN"] = str(binary)
        godbolt.local_oracle_binary("icx2026.2")
        other = Path(self._tmp.name) / 'other'
        other.mkdir()
        os.environ["LCCC_ICX_BIN"] = str(_fake_compiler(other, "20260101"))
        with self.assertRaises(godbolt.GodboltError):
            godbolt.local_oracle_binary("icx2026.2")

    def test_retargeted_symlink_is_not_cached(self) -> None:
        first = Path(self._tmp.name) / 'first'
        second = Path(self._tmp.name) / 'second'
        first.mkdir()
        second.mkdir()
        link = Path(self._tmp.name) / 'icx-link'
        link.symlink_to(_fake_compiler(first, "20260200"))
        os.environ["LCCC_ICX_BIN"] = str(link)
        godbolt.local_oracle_binary("icx2026.2")
        link.unlink()
        link.symlink_to(_fake_compiler(second, "20240000"))
        with self.assertRaises(godbolt.GodboltError):
            godbolt.local_oracle_binary("icx2026.2")

    def test_local_only_catalogue_fetch_is_offline(self) -> None:
        with patch.object(godbolt, 'list_compilers', side_effect=AssertionError('network')):
            self.assertEqual(codegen_oracle._fetch_compiler_list(['icx2026.2']), [])

    def test_empty_provisioner_output_is_a_structured_error(self) -> None:
        with patch.object(godbolt.subprocess, 'run') as run:
            run.return_value.returncode = 0
            run.return_value.stdout = ''
            with self.assertRaisesRegex(godbolt.GodboltError, 'exactly one'):
                godbolt.local_oracle_binary('icx2026.2')


@unittest.skipUnless(
    os.environ.get("LCCC_TEST_REAL_ICX") == "1" and bool(os.environ.get("LCCC_ICX_BIN")),
    "real ICX probes require LCCC_TEST_REAL_ICX=1 and LCCC_ICX_BIN",
)
class RealIcx(unittest.TestCase):
    def test_real_icx_probes_as_2026_2(self) -> None:
        meta = godbolt.local_oracle_metadata("icx2026.2")
        self.assertTrue(meta["semver"].startswith("2026.2."))


if __name__ == "__main__":
    unittest.main()
