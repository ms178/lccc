"""Required workload semantics are shared by every caller, never inferred from GCC output."""
import os
import shlex
import shutil
import subprocess
import sys
from pathlib import Path
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts'))
sys.path.insert(0, str(ROOT))
import benchmark_codegen as codegen
from tests.benchmark import run_benchmarks as bench

REQUIRED_WRAP = {'arith_loop': '-DARITH_ITERS=64', 'loop_patterns': '-DSIZE=10000',
                 'switch_dispatch': '-DN=100000'}
def _gcc_with_ubsan_runtime():
    """Newest GCC that can actually LINK -fsanitize=undefined.

    A from-source oracle GCC is often configured --disable-libsanitizer to
    keep the bootstrap small, so selection by name alone would pick a
    compiler whose driver accepts the flag but whose link step dies with
    `cannot find -lubsan`. Probe the full compile+link pipeline instead.
    """
    for name in ('gcc-16', 'gcc'):
        path = shutil.which(name)
        if not path:
            continue
        try:
            probe = subprocess.run(
                [path, '-std=gnu17', '-fsanitize=undefined', '-x', 'c', '-',
                 '-o', os.devnull],
                input='int main(void){return 0;}',
                capture_output=True, text=True, timeout=30)
        except (OSError, subprocess.TimeoutExpired):
            continue
        if probe.returncode == 0:
            return path
    return None


GCC = _gcc_with_ubsan_runtime()


def signed(value, bits):
    value &= (1 << bits) - 1
    return value - (1 << bits) if value & (1 << (bits - 1)) else value


def arithmetic_expected():
    values = list(range(1, 33))
    for _ in range(64):
        for i in range(32):
            values[i] = signed(values[i] + values[(i + 1) % 32] * values[(i + 2) % 32], 32)
    result = 0
    for value in values:
        result ^= value
    return f'arith_loop result: {signed(result, 32)}\n'


def loops_expected():
    seed = 42
    values = []
    for _ in range(10000):
        seed = (seed * 1664525 + 1013904223) & 0xffffffff
        values.append((seed >> 1) - 1000000000)
    scaled = [signed(value * 3 + 7, 32) for value in values]
    dot = signed(sum(a * b for a, b in zip(values[:1000], scaled[:1000])), 64)
    seed = 42
    prefix = 0
    for _ in range(10000):
        seed = (seed * 1664525 + 1013904223) & 0xffffffff
        prefix += seed % 100
    return (f'sum={sum(values)} pos={sum(v for v in values if v > 0)} max={max(values)} '
            f'scaled={sum(scaled)} dot={dot} prefix={prefix}\n')


def switch_expected():
    seed = 777
    total = 0
    for a in range(100000):
        seed = (seed * 1664525 + 1013904223) & 0xffffffff
        op, b = (seed >> 16) & 15, seed & 0xffff
        # Apply the explicit 32-bit semantics independently of compiled C.
        results = (a + b, a - b, a * b, a ^ b, a | b, a & b,
                   a << (b & 7), a >> (b & 7), ~a + b, a - ~b,
                   (a + b) * 3, (a - b) * 5, (a ^ b) + 1,
                   (a | b) - 1, (a & b) + 2, a + b + 1)
        total = signed(total + signed(results[op], 32), 64)
    return f'switch_dispatch sum: {total}\n'


class BenchmarkInputContracts(unittest.TestCase):
    def test_overflowing_sources_declare_their_required_semantics(self):
        cases = {item.name: item for item in bench.BENCHMARKS}
        for name in REQUIRED_WRAP:
            with self.subTest(name=name):
                self.assertIn('-fwrapv', cases[name].extra_flags)

    def test_required_flags_reach_native_and_both_static_sides_after_user_options(self):
        with tempfile.TemporaryDirectory() as temp:
            for item in bench.BENCHMARKS:
                if item.name not in REQUIRED_WRAP:
                    continue
                with self.subTest(name=item.name):
                    compiler = bench.CompilerSpec('candidate', 'candidate', 'not-executed',
                                                  ('-O2', '-std=gnu17', '-fno-wrapv'))
                    with mock.patch.object(bench, 'invoke', return_value={
                            'ok': False, 'stdout': '', 'stderr': ''}) as invoke:
                        bench.compile_one(compiler, item, bench.source_for(item),
                                          Path(temp) / item.name, None)
                    command = invoke.call_args.args[0]
                    self.assertGreater(command.index('-fwrapv'), command.index('-fno-wrapv'))
                    oracle, local = codegen.compilation_flags(item, '-O2 -fno-wrapv', '/headers')
                    for flags in (oracle, local):
                        tokens = shlex.split(flags)
                        self.assertGreater(tokens.index('-fwrapv'), tokens.index('-fno-wrapv'))

    @unittest.skipUnless(GCC, 'native GCC required for the sanitizer and modular-oracle contract')
    def test_native_sanitizer_ablation_and_independent_modular_results(self):
        expected = {'arith_loop': arithmetic_expected(), 'loop_patterns': loops_expected(),
                    'switch_dispatch': switch_expected()}
        with tempfile.TemporaryDirectory() as temp:
            for item in bench.BENCHMARKS:
                if item.name not in REQUIRED_WRAP:
                    continue
                for required in (False, True):
                    with self.subTest(name=item.name, required_semantics=required):
                        binary = Path(temp) / 'workload'
                        # The negative arm intentionally removes the declared
                        # requirement, reproducing the invalid input contract.
                        flags = item.extra_flags if required else ()
                        build = subprocess.run([GCC, '-std=gnu17', '-O2',
                            '-fsanitize=undefined,float-cast-overflow', '-fno-sanitize-recover=all',
                            str(bench.source_for(item)), REQUIRED_WRAP[item.name], *flags,
                            '-o', str(binary)], capture_output=True, text=True, timeout=30)
                        self.assertEqual(build.returncode, 0, build.stderr)
                        run = subprocess.run([str(binary)], capture_output=True, text=True, timeout=10)
                        if required:
                            self.assertEqual(run.returncode, 0, run.stderr)
                            self.assertEqual(run.stderr, '')
                            self.assertEqual(run.stdout, expected[item.name])
                        else:
                            self.assertNotEqual(run.returncode, 0)
                            self.assertIn('signed integer overflow', run.stderr)




if __name__ == '__main__':
    unittest.main()
