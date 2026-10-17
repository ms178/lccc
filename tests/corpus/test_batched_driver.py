"""Fresh-state driver integration tests (system C compiler; no LCCC build)."""
from pathlib import Path
import json
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class DriverTests(unittest.TestCase):
    def compile(self, directory, entry):
        cc = shutil.which('gcc-16') or shutil.which('cc')
        self.assertIsNotNone(cc, 'a host C compiler is required')
        source = directory / 'entry.c'
        source.write_text('#include <stdio.h>\n' + entry)
        binary = directory / 'driver'
        subprocess.run([cc, '-O2', '-fno-lto', str(ROOT / 'tests/benchmark/batched_driver.c'),
                        str(source), '-o', str(binary)], check=True, capture_output=True)
        return binary

    def test_fresh_globals_and_count_validation(self):
        with tempfile.TemporaryDirectory() as temp:
            binary = self.compile(Path(temp), 'static int count; int benchmark_entry(void) { printf("fresh %d\\n", ++count); return count != 1; }')
            result = subprocess.run([binary, '3'], text=True, capture_output=True, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout, 'fresh 1\n' * 3)
            timing = json.loads(result.stderr.removeprefix('BATCH_RESULT '))
            self.assertEqual(timing['iterations'], 3)
            self.assertGreater(timing['ns'], 0)
            for count in ('0', '-1', 'junk', '1000001'):
                with self.subTest(count=count):
                    result = subprocess.run([binary, count], capture_output=True, timeout=10)
                    self.assertEqual(result.returncode, 99)
                    self.assertEqual(result.stdout, b'')

    def test_failure_is_not_a_timing_sample(self):
        with tempfile.TemporaryDirectory() as temp:
            binary = self.compile(Path(temp), 'int benchmark_entry(void) { return 1; }')
            result = subprocess.run([binary, '3'], capture_output=True, timeout=10)
            self.assertEqual(result.returncode, 97)
            self.assertNotIn(b'BATCH_RESULT', result.stderr)

if __name__ == '__main__':
    unittest.main()
