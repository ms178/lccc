"""Guarantees of the host-run oracles in scripts/godbolt.py (LOCAL_ORACLES).

Run: python3 -m pytest -q scripts/test_local_oracles.py

A local oracle is opt-in (never a default, which must run on GitHub runners),
is identified by its own macro probe, and refuses any binary whose probe does
not match the pinned release. The real ICX check is skipped unless a verified
binary is supplied through LCCC_ICX_BIN or scripts/ensure_icx_2026.sh works.
"""
from __future__ import annotations

import os
import stat
import sys
import tempfile
import unittest
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
        godbolt._LOCAL_BINARIES.pop("icx2026.1.1", None)
        self._tmp = tempfile.TemporaryDirectory(prefix="local-oracle-test-")

    def tearDown(self) -> None:
        if self._env is not None:
            os.environ["LCCC_ICX_BIN"] = self._env
        godbolt._LOCAL_BINARIES.pop("icx2026.1.1", None)
        self._tmp.cleanup()

    def test_wrong_release_is_refused_whatever_its_path(self) -> None:
        wrong = _fake_compiler(Path(self._tmp.name), "20240000")
        os.environ["LCCC_ICX_BIN"] = str(wrong)
        with self.assertRaisesRegex(godbolt.GodboltError, "probes"):
            godbolt.local_oracle_binary("icx2026.1.1")

    def test_matching_probe_is_accepted_and_digested(self) -> None:
        right = _fake_compiler(Path(self._tmp.name), "20260101")
        os.environ["LCCC_ICX_BIN"] = str(right)
        meta = godbolt.local_oracle_metadata("icx2026.1.1")
        self.assertEqual(meta["semver"], "2026.1.1")
        self.assertEqual(len(meta["binary_sha256"]), 64)
        self.assertEqual(meta["probe"], "__INTEL_LLVM_COMPILER=20260101")

    def test_local_oracle_is_refused_for_non_x86_arch(self) -> None:
        req = codegen_oracle.Request(
            source=Path(__file__), function="f", flags="-O3", local_flags=None, arch="aarch64",
        )
        with self.assertRaisesRegex(godbolt.GodboltError, "x86-64 oracle"):
            codegen_oracle._record_local_oracle("icx2026.1.1", req)


@unittest.skipUnless(
    os.environ.get("LCCC_ICX_BIN") or (Path(__file__).resolve().parent.parent / "scripts").exists(),
    "needs LCCC_ICX_BIN or a provisionable ICX",
)
class RealIcx(unittest.TestCase):
    def test_real_icx_probes_as_2026_1_1(self) -> None:
        try:
            meta = godbolt.local_oracle_metadata("icx2026.1.1")
        except godbolt.GodboltError as error:
            self.skipTest(f"ICX unavailable on this host: {error}")
        self.assertEqual(meta["probe"], "__INTEL_LLVM_COMPILER=20260101")


if __name__ == "__main__":
    unittest.main()
