"""Recorded build requests must not be changed except compiler/output slots."""
import sys
from pathlib import Path
import unittest
sys.path.insert(0,str(Path(__file__).resolve().parents[2]/'scripts'))
from replay_batched_codegen import output_command

class ReplayCommandTests(unittest.TestCase):
    def test_preserves_original_and_all_macro_flags(self):
        old=['old-cc','-O2','-DPASSES=65536','source.c','-S','-o','old.s']
        new=output_command(old,Path('new.s'),Path('new-cc'))
        self.assertEqual(new,['new-cc','-O2','-DPASSES=65536','source.c','-S','-o','new.s'])
        self.assertEqual(old[-1],'old.s')
    def test_preserves_driver_when_not_replaced(self):
        self.assertEqual(output_command(['gcc-16','driver.o','work.o','-o','old'], 'new'),
                         ['gcc-16','driver.o','work.o','-o','new'])
    def test_rejects_absent_duplicate_or_truncated_output(self):
        for command in [['cc','x.c'],['cc','-o'],['cc','-o','a','-o','b']]:
            with self.assertRaises(ValueError):output_command(command,'new')

if __name__=='__main__':unittest.main()
