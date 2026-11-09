#!/usr/bin/env python3
"""Regression tests for the SIMD implementation-audit pack.

These tests are picked up by the CI gate that already runs
``python3 -m unittest discover -s tests/simd -p 'test_*.py'``.  Adding a new
test file therefore changes no workflow text: the sharded lane plan
(.github/ci-lane-plan.json) stays byte-identical, so
tests/regression/check_ci_lane_plan.py remains green.  That is the reason the
pack is wired through unittest discovery instead of by adding a new CI step
to a sharded lane (which would require regenerating the whole plan).

Every test uses the reference compiler only.  No lccc build is needed, which
makes these tests deterministic and green on any runner with a C compiler.
ISA-gated runtime legs key off /proc/cpuinfo (never off target predefined
macros), so a runner without AVX-512 still proves compile-time cleanliness
and executes the portable path rather than faulting or silently passing.
The lccc-dependent driver remains a developer/CI-artifact tool until the
register's open findings are resolved; it is deliberately not a hard gate.
"""
from __future__ import annotations

import importlib.util
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
if str(HERE) not in sys.path:
    sys.path.insert(0, str(HERE))
ROOT = HERE.parents[1]
PY = sys.executable
GCC = os.environ.get('LCCC_TEST_CC', 'gcc')
DOC = ROOT / 'engineering' / 'SIMD-IMPLEMENTATION-AUDIT-2026-11-08.md'
README = ROOT / 'tests' / 'simd' / 'README.md'
REPRO_C = HERE / 'audit_repro_2026-11-08.c'
SHAPE_C = HERE / 'simd_shape_matrix.c'
DRIVER = HERE / 'check_audit_2026-11-08.py'
MATRIX = HERE / 'check_simd_shape_matrix.py'

WARN_FLAGS = ('-std=c99', '-O2', '-Wall', '-Wextra', '-Werror', '-Wpedantic',
              '-Wconversion', '-Wshadow', '-Wstrict-prototypes',
              '-Wold-style-definition', '-Wcast-qual', '-Wwrite-strings',
              '-Wundef', '-Wvla', '-fno-strict-aliasing')
SAN_FLAGS = ('-std=c99', '-O1', '-g', '-fno-omit-frame-pointer',
             '-fsanitize=address,undefined', '-fno-sanitize-recover=all',
             '-Wall', '-Wextra', '-Werror')


def load_driver_module():
    """Import the hyphenated driver by path (it is not a valid module name)."""
    spec = importlib.util.spec_from_file_location('audit_driver', DRIVER)
    if spec is None or spec.loader is None:
        raise RuntimeError(f'cannot load {DRIVER}')
    module = importlib.util.module_from_spec(spec)
    # dataclasses resolves annotations through sys.modules[cls.__module__];
    # a spec-loaded module must be registered before exec_module().
    sys.modules[spec.name] = module
    try:
        spec.loader.exec_module(module)
    finally:
        sys.modules.pop(spec.name, None)
    return module


def run(cmd: list[str], timeout: float = 120.0) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, cwd=ROOT, text=True, capture_output=True,
                          timeout=timeout, check=False)


class AuditPackTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory(prefix='lccc-audit-unittest-')
        cls.work = Path(cls.tmp.name)

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    # --- structural / traceability --------------------------------------

    def test_01_drivers_self_test(self):
        """The checkers must be able to fail; a green checker that cannot
        detect its own breakage is decoration."""
        # Every driver in this pack must be able to fail.  Adding a new
        # checker without wiring its --self-test here would leave an
        # unvalidated oracle in CI; enumerate them explicitly.
        drivers = (DRIVER, MATRIX,
                   HERE / 'check_simd_isa_matrix.py',
                   HERE / 'check_isa_permissions.py',
                   HERE / 'check_assembler_widths.py',
                   HERE / 'check_yarpgen.py',
                   HERE / 'check_lccc_capability.py')
        for script in drivers:
            with self.subTest(script=script.name):
                self.assertTrue(script.is_file(), f'missing driver {script}')
                proc = run([PY, str(script), '--self-test'])
                self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
                self.assertIn('checks passed', proc.stdout)

    def test_02_row_tokens_resolve(self):
        """Every `sa*`/`saNN*` token named in the audit must be a real row."""
        self.assertTrue(DOC.is_file(), f'missing audit document: {DOC}')
        tokens = set(re.findall(r'`(sa[0-9][0-9a-z_]*)`', DOC.read_text()))
        module = load_driver_module()
        rows = {r.name for r in module.ROWS}
        self.assertTrue(tokens, 'audit names no rows')
        self.assertEqual(tokens - rows, set(),
                         f'phantom row names in audit: {sorted(tokens - rows)}')
        self.assertTrue(any(r.expected == 'pass' for r in module.ROWS),
                        'REGRESSION path is unreachable without control rows')

    def test_03_every_c_entry_point_is_exercised(self):
        """No dead argv branches in the C reproducer."""
        exe = self.work / 'repro'
        proc = run([GCC, *WARN_FLAGS, str(REPRO_C), '-o', str(exe)])
        self.assertEqual(proc.returncode, 0, proc.stderr)
        list_proc = run([str(exe), 'list'])
        self.assertEqual(list_proc.returncode, 0, list_proc.stderr)
        entries = {line.split('\t')[0] for line in list_proc.stdout.splitlines()
                   if line.strip()}
        module = load_driver_module()
        used = {r.entry or r.name for r in module.ROWS
                if r.kind in ('runtime', 'diff')}
        self.assertEqual(entries, used,
                         f'dead C entry points: {sorted(entries - used)}; '
                         f'missing C entries: {sorted(used - entries)}')

    def test_04_readme_has_no_phantom_rows(self):
        text = README.read_text()
        for phantom in ('sa02_*', 'sa03_diag', 'sa05_run', 'sa06_run',
                        'sa07_run'):
            self.assertNotIn(phantom, text, f'README names {phantom}')

    def test_05_status_lattice_is_total(self):
        """All 4 result kinds x 2 expectations x 2 modes are classified."""
        module = load_driver_module()
        seen = set()
        for kind in ('pass', 'fail', 'skip', 'error'):
            for expected in ('open', 'pass'):
                for strict in (False, True):
                    label, hard = module.classify(
                        module.Result(kind), expected, strict)
                    self.assertIn(label, {'OPEN-CONFIRMED', 'STALE-OPEN',
                                          'PASS', 'REGRESSION', 'ERROR', 'SKIP'})
                    self.assertIsInstance(hard, bool)
                    seen.add((label, hard))
        self.assertGreaterEqual(len(seen), 8)

    def test_05b_crash_is_not_a_diagnostic(self):
        """SA-07's oracle may count only real compiler crashes.  A feature
        diagnostic, a bad flag or a missing toolchain must be hard ERROR."""
        import probe_utils as pu
        self.assertEqual(pu.is_crash('cc1: internal compiler error: boom'),
                         'internal compiler error')
        self.assertEqual(pu.is_crash("error: unrecognized option '-mavx512bw'"), '')
        self.assertEqual(pu.compiler_crash(-11, ''), 'signal 11')
        self.assertEqual(pu.compiler_crash(1, 'error: bad flag'), '')
        self.assertEqual(pu.compiler_crash(
            -1, 'probe infrastructure failure: timed out'), '')
        ok, _ = pu.is_diagnostic(
            'probe infrastructure failure: timed out -mavx512bw', ('avx512bw',))
        self.assertFalse(ok, 'infrastructure text must outrank a hint')
        rc, _, err = pu.run_rc_safe(['/definitely/not/a/compiler'], self.work, 1.0)
        self.assertEqual(rc, -1)
        self.assertIn('probe infrastructure failure', err)

    # --- C corpora: zero warnings, zero UB ------------------------------

    def test_06_c_corpora_warning_free(self):
        for src in (REPRO_C, SHAPE_C):
            for extra in ((), ('-mavx512f', '-mavx512bw')):
                with self.subTest(src=src.name, extra=extra):
                    exe = self.work / f'{src.stem}{len(extra)}.bin'
                    proc = run([GCC, *WARN_FLAGS, *extra, str(src),
                                '-o', str(exe)])
                    self.assertEqual(proc.returncode, 0, proc.stderr)
                    self.assertEqual(proc.stderr, '',
                                     f'{src.name}: compiler diagnostics')

    @staticmethod
    def _evex_probe(flags: tuple[str, ...]) -> tuple[bool, str]:
        """Return whether `flags` can compile AND EXECUTE a real zmm store.

        CPUID advertisement is not proof of executability: the 2026-11-08
        hosted runner advertises avx512f/avx512bw in /proc/cpuinfo yet its
        vCPU rejects EVEX with SIGILL (hosted log
        `42506426020/job/128810582710`: `probe run rc=-4`).  Two earlier
        probes were themselves unsound and were caught by hosted CI:

        * a constant `_mm512_storeu_si512` was constant-folded away at -O1,
          so it reported success on a CPU with no EVEX support;
        * an `asm volatile` with an `"x"` constraint was allowed by gcc-16
          16.0.1 to use a non-zmm register, so it also reported success
          without ever executing an EVEX instruction.

        The gate is therefore two-part and fails closed:
        (1) the probe's disassembly MUST contain a `%zmm` operand, and
        (2) the probe MUST exit 0.  The source uses a `volatile` input plus
        an opaque memory-clobber barrier so the store cannot be folded, and
        it uses the SAME `_mm512_storeu_si512` intrinsic as the corpus.
        """
        import shutil
        work = AuditPackTests.work
        src = work / 'evex_probe.c'
        src.write_text(
            '#include <immintrin.h>\n'
            'static volatile int evex_input = 7;\n'
            'int main(void) {\n'
            '  unsigned lanes[16];\n'
            '  int seed = evex_input;\n'
            '  __m512i v = _mm512_set1_epi32(seed);\n'
            '  _mm512_storeu_si512((void *)lanes, v);\n'
            '  __asm__ volatile ("" : : "r" (lanes) : "memory");\n'
            '  return lanes[0] == (unsigned)seed ? 0 : 1;\n'
            '}\n', encoding='utf-8')
        tag = 'san' if any('sanitize' in f for f in flags) else 'plain'
        exe = work / f'evex_probe_{tag}.bin'
        try:
            proc = run([GCC, *flags, '-mavx512f', '-mavx512bw',
                        str(src), '-o', str(exe)])
            if proc.returncode != 0:
                return False, (f'probe compile rc={proc.returncode}: '
                               f'{proc.stderr.strip()[:200]}')
        except (OSError, subprocess.SubprocessError) as exc:
            return False, f'probe infrastructure failure: {exc}'
        objdump = shutil.which('objdump')
        if objdump is None:
            return False, 'objdump unavailable: cannot prove the probe is EVEX'
        dis = run([objdump, '-d', str(exe)])
        if dis.returncode != 0 or not re.search(r'%zmm', dis.stdout):
            return False, ('probe emitted no %zmm store: the compiler '
                           'narrowed the vector and the gate must close')
        try:
            res = run([str(exe)], timeout=15)
        except (OSError, subprocess.SubprocessError) as exc:
            return False, f'probe infrastructure failure: {exc}'
        if res.returncode != 0:
            return False, (f'probe run rc={res.returncode}: '
                           f'{(res.stdout + res.stderr).strip()[:200]}')
        return True, 'real EVEX zmm store executed'

    def test_07_c_corpora_sanitizers(self):
        """ASan+UBSan clean on ANY host, with an executable ISA gate.

        Both variants are COMPILED on every runner (so the AVX-512 header
        path stays warning-free under -Werror).  The portable variant is
        always EXECUTED.  The AVX-512 variant is executed under sanitizers
        only when a tiny EVEX probe built with the identical flags really
        executes; otherwise the skip is printed with its reason and a
        non-sanitized AVX-512 build is executed as hard correctness
        evidence.  Nothing is silently downgraded to a pass.
        """
        san_ok, san_reason = self._evex_probe(SAN_FLAGS)
        plain_ok, plain_reason = self._evex_probe(('-std=c99', '-O1'))
        for src in (REPRO_C, SHAPE_C):
            modes = (('all',),) if src is REPRO_C else (('all',), ('selftest',))
            for label, extra in (('portable', ()),
                                 ('avx512', ('-mavx512f', '-mavx512bw'))):
                with self.subTest(src=src.name, isa=label):
                    exe = self.work / f'{src.stem}_{label}_san.bin'
                    proc = run([GCC, *SAN_FLAGS, *extra, str(src), '-o', str(exe)])
                    self.assertEqual(proc.returncode, 0, proc.stderr)
                    if label != 'avx512' or san_ok:
                        for mode in modes:
                            with self.subTest(mode=mode):
                                run_proc = run([str(exe), *mode], timeout=60)
                                self.assertEqual(run_proc.returncode, 0,
                                                 run_proc.stdout + run_proc.stderr)
                                self.assertNotIn('AddressSanitizer', run_proc.stderr)
                                self.assertNotIn('runtime error:', run_proc.stderr)
                        continue
                    print(f'SKIP  {src.name} AVX-512 ASan runtime: {san_reason}',
                          flush=True)
                    if not plain_ok:
                        print(f'SKIP  {src.name} AVX-512 plain runtime: '
                              f'{plain_reason}', flush=True)
                        continue
                    plain_exe = self.work / f'{src.stem}_{label}_plain.bin'
                    plain = run([GCC, '-std=c99', '-O1', *extra, str(src),
                                 '-o', str(plain_exe)])
                    self.assertEqual(plain.returncode, 0, plain.stderr)
                    for mode in modes:
                        with self.subTest(mode=mode, evidence='plain'):
                            run_proc = run([str(plain_exe), *mode], timeout=60)
                            self.assertEqual(run_proc.returncode, 0,
                                             run_proc.stdout + run_proc.stderr)

    def test_07b_evex_probe_gate_fails_closed(self):
        """The executable gate must report (False, reason) on a broken
        toolchain instead of silently advertising AVX-512 support."""
        from unittest import mock
        with mock.patch.object(sys.modules[__name__], 'GCC',
                               '/definitely/not/a/compiler'):
            ok, reason = self._evex_probe(SAN_FLAGS)
        self.assertFalse(ok)
        self.assertTrue(reason.startswith(('probe compile',
                                           'probe infrastructure failure')),
                        reason)

    def test_07c_evex_probe_is_not_optimized_away(self):
        """The ISA gate must prove a real zmm store or close explicitly.

        A constant `_mm512_storeu_si512` can be constant-folded to nothing,
        and an `"x"` asm constraint can be narrowed to ymm/xmm.  Either case
        must make the gate report `(False, <reason>)`, never a false success.
        """
        import shutil
        objdump = shutil.which('objdump')
        if objdump is None:
            self.skipTest('objdump not installed')
        for flags, name in ((SAN_FLAGS, 'evex_probe_san.bin'),
                            (('-std=c99', '-O1'), 'evex_probe_plain.bin')):
            with self.subTest(probe=name):
                ok, reason = self._evex_probe(flags)
                exe = self.work / name
                self.assertTrue(exe.is_file(), f'{name} was not compiled')
                dis = run([objdump, '-d', str(exe)])
                self.assertEqual(dis.returncode, 0, dis.stderr)
                if re.search(r'%zmm', dis.stdout):
                    self.assertNotIn('no %zmm store', reason)
                else:
                    self.assertFalse(ok, 'gate opened without a zmm store')
                    self.assertIn('no %zmm store', reason)

    def test_07d_evex_gate_rejects_narrowed_disassembly(self):
        """A compiler that narrows the probe to ymm must close the gate.

        This is the deterministic control for the gcc-16 16.0.1 behaviour on
        hosted CI; it does not depend on the host CPU or compiler version.
        """
        import shutil
        from unittest import mock
        if shutil.which('objdump') is None:
            self.skipTest('objdump not installed')
        real_run = run

        def fake(cmd, timeout=120.0):
            if Path(cmd[0]).name == 'objdump':
                return subprocess.CompletedProcess(
                    cmd, 0, 'vmovdqu  %ymm0,(%rsp)\n', '')
            return real_run(cmd, timeout=timeout)

        with mock.patch.object(sys.modules[__name__], 'run',
                               side_effect=fake):
            ok, reason = self._evex_probe(('-std=c99', '-O1'))
        self.assertFalse(ok)
        self.assertIn('no %zmm store', reason)

    # --- end-to-end harness checks --------------------------------------

    def test_08_driver_strict_green_with_reference_compiler(self):
        """A compiler without the findings must be strict-green.  This is the
        end-to-end oracle for the whole status/diagnostic/probe machinery."""
        receipt = self.work / 'nested' / 'audit.json'
        proc = run([PY, str(DRIVER), '--lccc', GCC, '--reference', GCC,
                    '--strict', '--json', str(receipt),
                    '--compile-timeout', '60', '--run-timeout', '30'],
                   timeout=300)
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        self.assertTrue(receipt.is_file(), '--json did not create parent dirs')
        import json
        data = json.loads(receipt.read_text())
        self.assertEqual(data['schema'], 1)
        self.assertEqual(len(data['rows']), 15)
        statuses = {r['status'] for r in data['rows']}
        self.assertNotIn('ERROR', statuses)
        self.assertNotIn('REGRESSION', statuses)
        self.assertIn('PASS', statuses)

    def test_09_shape_matrix_quick(self):
        proc = run([PY, str(MATRIX), '--candidate', GCC, '--reference', GCC,
                    '--quick', '--sanitize', '--compile-timeout', '60',
                    '--run-timeout', '30'], timeout=300)
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        self.assertIn('0 problems', proc.stdout)

    def test_10_shape_oracles_are_independent(self):
        """A deliberately wrong fast() must be caught by the oracle."""
        exe = self.work / 'shape.bin'
        proc = run([GCC, *WARN_FLAGS, str(SHAPE_C), '-o', str(exe)])
        self.assertEqual(proc.returncode, 0, proc.stderr)
        out = run([str(exe), 'all']).stdout
        rows = [line.split() for line in out.splitlines() if line.strip()]
        self.assertEqual(len(rows), 29)
        for name, fast, ref, flag in rows:
            self.assertEqual(fast, ref, f'{name}: oracle mismatch')
            self.assertEqual(flag, 'ok', f'{name}: not ok')

    # --- documentation claims -------------------------------------------

    def test_11_source_claims_still_true(self):
        """Spot-check headline claims that the register asserts as S-evidence.
        Snippets, not line numbers: line anchors drift, prose does not."""
        claims = (
            # SA-33 is FIXED (§9c.10.4): the obligation is armed at the
            # physical wide-register write, so the old opcode-name proxy is
            # gone.  Pin the replacement invariant instead of the defect, and
            # assert below that the proxy really was removed -- otherwise a
            # revert would restore the snippet and silently satisfy a test
            # that was only ever checking the bug still existed.
            ('src/backend/x86/codegen/intrinsics_simd.rs',
             'zmm.starts_with("ymm") || zmm.starts_with("zmm")'),
            ('src/backend/state.rs',
             'reg.starts_with("ymm") || reg.starts_with("zmm")'),
            ('src/passes/vectorize.rs', 'debug_assert!(false'),
            ('scripts/gen_lcccsimd.py', '"vmovdqu64_load"'),
            ('scripts/gen_lcccsimd.py', '"vpshuflw512"'),
            ('src/frontend/preprocessor/predefined_macros.rs', '__AVX512BW__'),
        )
        for rel, snippet in claims:
            with self.subTest(rel=rel, snippet=snippet):
                path = ROOT / rel
                self.assertTrue(path.is_file(), f'missing {rel}')
                self.assertIn(snippet, path.read_text(errors='replace'),
                              f'{rel}: claim {snippet!r} no longer holds')
        # The retired SA-33 proxy must NOT come back.
        self.assertNotIn('format!("{:?}", op)',
                         (ROOT / 'src/backend/x86/codegen/intrinsics_simd.rs').read_text(),
                         'SA-33 regressed: the opcode-name vzeroupper proxy is back')
        # SA-05 must stay non-commutative at 512 bits.
        self.assertIn('"vpmaddubsw", false',
                      (ROOT / 'src/backend/x86/codegen/intrinsics_simd.rs').read_text(),
                      'SA-05 regressed: vpmaddubsw is commutative again')

    def test_12_register_is_complete(self):
        text = DOC.read_text()
        ids = re.findall(r'^\| (SA-\d\d) \|', text, re.M)
        self.assertEqual(len(ids), 54)
        self.assertEqual(len(set(ids)), 54)
        for i in range(1, 55):
            self.assertIn(f'SA-{i:02d}', set(ids))

    # --- mutation proofs: the harness must be able to fail ---------------

    def test_14_mutation_wrong_code_is_detected(self):
        """Inject a real wrong-code bug into each corpus and prove its own
        oracle catches it.  A green checker that cannot fail is decoration."""
        repro = REPRO_C.read_text()
        needle = '    r[0] = x[0] * k - z[0];\n    r[1] = x[1] * k - z[1];'
        self.assertEqual(repro.count(needle), 1)
        bad_repro = self.work / 'repro_bad.c'
        bad_repro.write_text(repro.replace(
            needle, '    r[0] = x[0] * k + z[0];\n'
                    '    r[1] = x[1] * k - z[1];', 1))
        exe = self.work / 'repro_bad.bin'
        proc = run([GCC, *WARN_FLAGS, str(bad_repro), '-o', str(exe)])
        self.assertEqual(proc.returncode, 0, proc.stderr)
        out = run([str(exe), 'sa02'])
        self.assertNotEqual(out.returncode, 0,
                            'mutated SA-02 kernel was NOT detected')
        self.assertIn('MISMATCH', out.stdout + out.stderr)

        shape = SHAPE_C.read_text()
        needle2 = '    for (i = 0; i < N; i++) acc += g_fa[i] * g_fb[i];'
        self.assertEqual(shape.count(needle2), 1)
        bad_shape = self.work / 'shape_bad.c'
        bad_shape.write_text(shape.replace(
            needle2, '    for (i = 0; i < N; i++) acc += 1.0f + g_fa[i] * g_fb[i];', 1))
        exe2 = self.work / 'shape_bad.bin'
        proc = run([GCC, *WARN_FLAGS, str(bad_shape), '-o', str(exe2)])
        self.assertEqual(proc.returncode, 0, proc.stderr)
        out2 = run([str(exe2), 'all'])
        self.assertNotEqual(out2.returncode, 0)
        lines = [line for line in out2.stdout.splitlines() if line.strip()]
        self.assertEqual(len(lines), 29,
                         'all shapes must be reported, not just the first failure')
        self.assertEqual(sum('MISMATCH' in line for line in lines), 1)

    def test_15_crash_oracle_is_not_fooled_by_diagnostics(self):
        """A genuine crash and a feature diagnostic must be distinguished.

        SA-07 is now a *fixed* hard control (the ARX terminator bug was
        repaired), so a real ICE is REGRESSION evidence -- never a silent
        PASS and never an expected OPEN-CONFIRMED.  A feature diagnostic
        from a broken toolchain must stay a hard ERROR.  Both are hard
        failures; only the classification differs.
        """
        scripts = {
            'ice': 'echo "lccc: internal compiler error: in expand_expr" >&2\nexit 1\n',
            'diag': ("echo \"lccc: error: unrecognized command-line option "
                     "'-mavx512bw'\" >&2\nexit 1\n"),
        }
        for name, body in scripts.items():
            path = self.work / f'fake-{name}-cc'
            path.write_text('#!/bin/sh\n' + body)
            path.chmod(0o755)
            with self.subTest(compiler=name):
                proc = run([PY, str(DRIVER), '--lccc', str(path),
                            '--reference', GCC, '--strict',
                            '--compile-timeout', '20', '--run-timeout', '10'],
                           timeout=120)
                sa07 = [line for line in proc.stdout.splitlines()
                        if len(line.split()) >= 2 and line.split()[1] == 'sa07']
                self.assertEqual(len(sa07), 1, proc.stdout)
                if name == 'ice':
                    self.assertIn('REGRESSION', sa07[0],
                                  'a real ICE must fail a fixed control hard')
                    self.assertIn('compiler crash', sa07[0])
                    self.assertNotIn('OPEN-CONFIRMED', sa07[0],
                                     'SA-07 is fixed; an ICE is no longer '
                                     'expected evidence')
                else:
                    self.assertIn('ERROR', sa07[0],
                                  'a feature diagnostic must not be ICE evidence')
                    self.assertNotIn('compiler crash', sa07[0])
                self.assertNotEqual(proc.returncode, 0)

    @unittest.skipUnless(importlib.util.find_spec('yaml') is not None,
                         'PyYAML not installed')
    def test_13_ci_lane_plan_unchanged(self):
        proc = run([PY, str(ROOT / 'tests' / 'regression' /
                            'check_ci_lane_plan.py')])
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)


if __name__ == '__main__':
    raise unittest.main()
