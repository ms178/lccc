"""Independent note-run oracle: payload bytes are never alignment padding."""
import importlib.util
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('linker_note_runs', ROOT / 'tests/linker/run_linker_tests.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


def section(name, kind, off, size, flags=2, delta=4096):
    return [name, kind, flags, off + delta, off, size, 0, 0, 4]


class NoteRuns(unittest.TestCase):
    def test_true_alignment_gaps_merge(self):
        for gap in range(9):
            with self.subTest(gap=gap):
                rows = [section('a', 7, 64, 32), section('b', 7, 96 + gap, 32)]
                self.assertEqual(len(module._allocated_note_runs(rows)), 1)

    def test_even_one_byte_payload_splits_the_run(self):
        for size in range(1, 9):
            with self.subTest(size=size):
                rows = [section('a', 7, 64, 32), section('.rodata', 1, 96, size),
                        section('b', 7, 104, 32)]
                self.assertEqual(len(module._allocated_note_runs(rows)), 2)

    def test_large_gaps_and_different_mappings_split(self):
        for off, delta in [(112, 4096), (96, 8192)]:
            rows = [section('a', 7, 64, 32), section('b', 7, off, 32, delta=delta)]
            self.assertEqual(len(module._allocated_note_runs(rows)), 2)

    def test_non_file_payload_does_not_split(self):
        for middle in [section('empty', 1, 96, 0), section('bss', 8, 96, 8),
                       section('debug', 1, 96, 8, flags=0)]:
            rows = [section('a', 7, 64, 32), middle, section('b', 7, 104, 32)]
            self.assertEqual(len(module._allocated_note_runs(rows)), 1)

    def test_note_span_payload_boundaries(self):
        occupied = [(0x120, 0x127)]
        for start, end, expected in [
            (0x100, 0x120, False), (0x127, 0x140, False),
            (0x120, 0x121, True), (0x126, 0x140, True),
            (0x100, 0x140, True), (0x123, 0x123, False),
            (0x124, 0x122, False),
        ]:
            with self.subTest(start=start, end=end):
                self.assertEqual(module._note_span_has_payload(occupied, start, end), expected)

    def test_registered_oracle_entry_point_executes_its_controls(self):
        self.assertEqual(module._note_run_oracle_test(None, None).status, 'PASS')


if __name__ == '__main__':
    unittest.main()
