#!/usr/bin/env python3
"""Note-run oracle boundary tests: data bytes are never inferred to be padding."""
import unittest
from run_linker_tests import _note_run_count

class NoteRuns(unittest.TestCase):
    def test_alignment_gap(self):
        self.assertEqual(_note_run_count([(0x100,0x100,32,8),(0x128,0x128,32,8)],[]),1)
    def test_short_rodata_is_not_padding(self):
        self.assertEqual(_note_run_count([(0x100,0x100,32,8),(0x128,0x128,32,8)],[(0x120,0x127)]),2)
    def test_other_data_does_not_break_a_span(self):
        self.assertEqual(_note_run_count([(0x100,0x100,32,8),(0x120,0x120,32,8)],[(0,0x100),(0x140,0x200)]),1)
    def test_virtual_discontinuity(self):
        self.assertEqual(_note_run_count([(0x100,0x100,32,8),(0x1120,0x120,32,8)],[]),2)
    def test_large_gap_and_empty(self):
        self.assertEqual(_note_run_count([(0x100,0x100,32,8),(0x140,0x140,32,8)],[]),2)
        self.assertEqual(_note_run_count([],[]),0)

if __name__ == '__main__':
    unittest.main()
