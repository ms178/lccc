#!/usr/bin/env python3
"""The i686 differential generator must emit valid C for the reference too."""
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import m32_differential_fuzz


class M32GeneratorTests(unittest.TestCase):
    def test_generation_is_deterministic_and_seed_sensitive(self):
        self.assertEqual(m32_differential_fuzz.gen_c(4800),
                         m32_differential_fuzz.gen_c(4800))
        self.assertNotEqual(m32_differential_fuzz.gen_c(4800),
                            m32_differential_fuzz.gen_c(4801))

    def test_reference_accepts_generated_translation_units(self):
        cc = shutil.which("gcc")
        if cc is None:
            self.skipTest("no host C compiler")
        for seed in (0, 1, 2, 3, 116, 4799, 4800, 4801):
            with self.subTest(seed=seed), tempfile.TemporaryDirectory() as tmp:
                src = Path(tmp) / "case.c"
                src.write_text(m32_differential_fuzz.gen_c(seed))
                result = subprocess.run(
                    [cc, "-m32", "-std=gnu11", "-w", "-fsyntax-only", str(src)],
                    text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                    timeout=30)
                self.assertEqual(
                    result.returncode, 0,
                    f"generator emitted invalid C for seed {seed}: "
                    f"{result.stderr[-800:]}")


if __name__ == "__main__":
    unittest.main()
