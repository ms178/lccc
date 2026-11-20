"""Exercise the boot verdict on serial bytes without building a kernel."""
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class SerialSentinel(unittest.TestCase):
    def test_only_one_literal_record_passes(self):
        script = (ROOT / 'scripts/qemu_boot_test.sh').read_text()
        function = 'expect_contiguous() {' + script.split('expect_contiguous() {', 1)[1].split('\n}', 1)[0] + '\n}'
        with tempfile.TemporaryDirectory() as td:
            log = Path(td) / 'serial'
            for content, expected in [(b'a b\n', 0), (b'ab\n', 1), (b'a  b\n', 1),
                                      (b'a b\na b\n', 1), (b'b a\n', 1), (b'', 1)]:
                with self.subTest(content=content):
                    log.write_bytes(content)
                    result = subprocess.run(['bash', '-c', function +
                                             '\nLOG=$1; fail=0; expect_contiguous probe "a b"; exit "$fail"',
                                             'probe', str(log)], capture_output=True)
                    self.assertEqual(result.returncode, expected)


if __name__ == '__main__':
    unittest.main()
