"""The existing whole-object assembler gates discover both generated matrices."""
import importlib.util
from pathlib import Path
import unittest
ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('sse34_matrix', ROOT/'tests/asm-diff/gen_sse34.py')
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)

class Matrix(unittest.TestCase):
    def test_both_modes_are_regenerated_exactly(self):
        for bits, path in [(64,'sse34-complete.casefile'),(32,'i686/sse34-complete.casefile')]:
            self.assertTrue((ROOT/'tests/asm-diff'/path).read_text()==m.generate(bits), path)
    def test_inventory_and_control_sweeps(self):
        self.assertEqual(len(m.SSSE3),15) # PALIGNR is the sixteenth, immediate form.
        for bits in (32,64):
            text=m.generate(bits)
            for instruction in ('phaddsw','phsubsw','pcmpestri','pcmpestrm','pcmpistri','pcmpistrm','extrq','insertq'):
                self.assertIn(';;; '+instruction+'\n',text)
            self.assertIn('pcmpistri $255,',text)
            self.assertIn('palignr $255,%mm7,%mm0',text)
            self.assertIn('extrq $255,$17,',text)
            self.assertIn('insertq $19,$255,',text)
        self.assertIn('rex64 pcmpestri $255,%xmm9,%xmm10',m.generate(64))
