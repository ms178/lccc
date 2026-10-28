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
from contextlib import ExitStack, redirect_stdout, redirect_stderr
from io import StringIO
import subprocess
import json
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
        "echo '#define __x86_64__ 1'\n"
        "echo '#define __SIZEOF_POINTER__ 8'\n"
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


class LocalDispatchContract(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.source = Path(self.temp.name) / 'input.c'
        self.source.write_text('int f(void) { return 1; }')
        self.lines = ['f:', '  movl $1, %eax', '  ret', '.size f, .-f']
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        for name in ('list_compilers', 'compiler_metadata', 'compile_on_godbolt'):
            self.stack.enter_context(patch.object(godbolt, name, side_effect=AssertionError('remote API used')))
        self.compile = self.stack.enter_context(patch.object(godbolt, 'compile_local_oracle', return_value=self.lines))
        self.stack.enter_context(patch.object(godbolt, 'local_oracle_metadata', return_value={
            'id': 'icx2026.2', 'name': 'ICX', 'semver': '2026.2.0', 'binary_sha256': 'a' * 64,
        }))
        self.stack.enter_context(redirect_stdout(StringIO()))
        self.stack.enter_context(redirect_stderr(StringIO()))

    def test_compile_cli_dispatches_locally(self):
        self.assertEqual(godbolt.main(['compile', 'icx2026.2', str(self.source), '--function', 'f']), 0)
        self.compile.assert_called_once()

    def test_compile_cli_explicitly_refuses_execution(self):
        self.assertEqual(godbolt.main(['compile', 'icx2026.2', str(self.source), '--execute']), 1)
        self.compile.assert_not_called()

    def test_compare_cli_dispatches_locally_and_records_identity(self):
        output = Path(self.temp.name) / 'manifest.json'
        launcher = Path(self.temp.name) / 'lccc'
        launcher.write_text('#!/bin/sh\necho fixture-compiler\n')
        launcher.chmod(0o755)
        with patch.object(godbolt, '_compile_local', return_value=self.lines):
            self.assertEqual(godbolt.main(['compare', str(self.source), '--local', str(launcher),
                '--oracles', 'icx2026.2', '--function', 'f', '--json', str(output)]), 0)
        manifest = json.loads(output.read_text())
        self.assertEqual(len(manifest['oracles'][0]['launcher_sha256']), 64)
        self.assertEqual(manifest['oracles'][1]['binary_sha256'], 'a' * 64)

    def test_local_listing_is_discoverable_without_network_or_provisioning(self):
        self.assertEqual(godbolt.main(['list', '--local-only']), 0)
        self.compile.assert_not_called()

    def test_codegen_function_whole_tu_and_rank_are_offline(self):
        for mode in (['--function', 'f'], ['--all-functions'], ['--rank']):
            with self.subTest(mode=mode):
                self.assertEqual(codegen_oracle.main([str(self.source), '--no-local',
                    '--oracles', 'icx2026.2', *mode]), 0)
        self.assertEqual(self.compile.call_count, 3)

    def test_profile_uses_local_dispatch_for_isa_and_contraction_probes(self):
        import oracle_profile
        self.compile.return_value = [' vpaddd %ymm0, %ymm1, %ymm2', ' vfmadd132ps',
            ' vcvtph2ps', ' andnq', ' pdepq', ' lzcntq', ' popcntq', ' vpminsd',
            ' crc32q', ' mulsd', ' addsd']
        self.compile.side_effect = [self.compile.return_value, [' mulsd', ' addsd']]
        report = oracle_profile.verify(['icx2026.2'], '-O3 -march=haswell -ffp-contract=off')
        self.assertTrue(report['complete'])
        self.assertEqual(self.compile.call_count, 2)


class LocalTargetContract(unittest.TestCase):
    def test_accepts_native_isa_and_language_flags(self):
        godbolt.validate_local_oracle_flags('icx2026.2', '-O3 -march=haswell -m64 -std=c17 -I"some path"')

    def test_conflicting_and_opaque_flags_fail_before_provisioning(self):
        for flags in ('-m32', '-mx32', '-m16', '--target=aarch64-linux-gnu', '-target aarch64',
                      '-arch arm64', '-mabi=ilp32', '@response', '--config=file',
                      '-Xclang -triple', '-Xarch_arm64 -O3', '-Wp,@response', '--'):
            with self.subTest(flags=flags), patch.object(godbolt, 'local_oracle_binary') as binary:
                with self.assertRaises(godbolt.GodboltError):
                    godbolt.compile_local_oracle('icx2026.2', Path('unused.c'), flags)
                binary.assert_not_called()

    def test_native_probe_rejects_a_non_lp64_compiler(self):
        with tempfile.TemporaryDirectory() as td, patch.dict(os.environ):
            binary = _fake_compiler(Path(td), '20260200')
            binary.write_text(binary.read_text() + "echo '#define __SIZEOF_POINTER__ 4'\n")
            os.environ['LCCC_ICX_BIN'] = str(binary)
            with self.assertRaisesRegex(godbolt.GodboltError, 'LP64'):
                godbolt.local_oracle_binary('icx2026.2')
            godbolt._LOCAL_BINARIES.clear()

    def test_probe_timeout_is_a_structured_error(self):
        with tempfile.TemporaryDirectory() as td, patch.dict(os.environ):
            binary = _fake_compiler(Path(td), '20260200')
            os.environ['LCCC_ICX_BIN'] = str(binary)
            with patch.object(godbolt.subprocess, 'run', side_effect=subprocess.TimeoutExpired('probe', 60)):
                with self.assertRaises(godbolt.GodboltError):
                    godbolt.local_oracle_binary('icx2026.2')
            godbolt._LOCAL_BINARIES.clear()

    def test_malformed_provisioning_manifest_is_a_structured_error(self):
        with tempfile.TemporaryDirectory() as td, patch.dict(os.environ):
            binary = _fake_compiler(Path(td), '20260200')
            os.environ['LCCC_ICX_BIN'] = str(binary)
            (Path(td) / '.lccc-icx.json').write_text('{invalid')
            with self.assertRaisesRegex(godbolt.GodboltError, 'invalid provisioning manifest'):
                godbolt.local_oracle_metadata('icx2026.2')
            godbolt._LOCAL_BINARIES.clear()


@unittest.skipUnless(
    os.environ.get("LCCC_TEST_REAL_ICX") == "1" and bool(os.environ.get("LCCC_ICX_BIN")),
    "real ICX probes require LCCC_TEST_REAL_ICX=1 and LCCC_ICX_BIN",
)
class RealIcx(unittest.TestCase):
    def test_real_icx_probes_as_2026_2(self) -> None:
        meta = godbolt.local_oracle_metadata("icx2026.2")
        self.assertTrue(meta["semver"].startswith("2026.2."))

    def test_real_compile_runtime_dispatch_and_profile(self) -> None:
        import oracle_profile
        with tempfile.TemporaryDirectory() as td:
            src = Path(td) / 'real.c'
            src.write_text('#include <stdio.h>\n'
                '__attribute__((noinline)) int f(int n) { int r=0; for(int i=0;i<n;++i) r+=i*3+1; return r; }\n'
                'int main(void) { printf("%d\\n", f(1234)); return 0; }\n')
            flags = '-std=gnu17 -O2 -march=haswell -fwrapv'
            icx = str(godbolt.local_oracle_binary('icx2026.2'))
            binary = Path(td) / 'real'
            subprocess.run([icx, *flags.split(), str(src), '-o', str(binary)], check=True, timeout=180)
            expected = str(sum(i * 3 + 1 for i in range(1234)))
            got = subprocess.run([str(binary)], check=True, capture_output=True, text=True, timeout=30)
            self.assertEqual(got.stdout.strip(), expected)
            # A real vendor process must still never trigger the CE paths.
            with ExitStack() as stack:
                for name in ('list_compilers', 'compiler_metadata', 'compile_on_godbolt'):
                    stack.enter_context(patch.object(godbolt, name, side_effect=AssertionError('remote API used')))
                stack.enter_context(redirect_stdout(StringIO()))
                self.assertEqual(godbolt.main(['compile', 'icx2026.2', str(src), '--function', 'f', '--flags', flags]), 0)
                for mode in (['--function', 'f'], ['--all-functions'], ['--rank']):
                    self.assertEqual(codegen_oracle.main([str(src), '--no-local', '--oracles', 'icx2026.2',
                        '--flags', flags, *mode]), 0)
                if os.environ.get('CCC'):
                    self.assertEqual(godbolt.main(['compare', str(src), '--local', os.environ['CCC'],
                        '--oracles', 'icx2026.2', '--function', 'f', '--flags', flags]), 0)
                report = oracle_profile.verify(['icx2026.2'], flags + ' -ffp-contract=off')
                self.assertTrue(report['complete'], report)


if __name__ == "__main__":
    unittest.main()
