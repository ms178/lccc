"""The census must compile ISA-specific TUs without raising the global ISA."""
from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))
from scripts.loop_invert_census import default_corpus, target_flags_for


class CensusTargetFlags(unittest.TestCase):
    def test_avx_stress_remains_in_census_with_registered_target(self):
        source = ROOT / "tests/benchmark/programs/avx_fp_private.c"
        self.assertIn(source, default_corpus(ROOT))
        self.assertEqual(target_flags_for(ROOT, source), ["-mavx2"])
        self.assertEqual(target_flags_for(ROOT, source.relative_to(ROOT)), ["-mavx2"])

    def test_generic_sources_do_not_inherit_avx(self):
        for name in ["linux_find_bit.c", "fp_product_map.c", "lz4_compress.c"]:
            self.assertEqual(target_flags_for(ROOT, Path("tests/benchmark/programs") / name), [])

    def test_sse1_source_preserves_negative_isa_requirement(self):
        self.assertEqual(
            target_flags_for(ROOT, Path("tests/benchmark/programs/f32_sse1_map.c")),
            ["-mno-sse2", "-msse"],
        )

    def test_unregistered_sources_keep_default_target(self):
        self.assertEqual(target_flags_for(ROOT, Path("unregistered.c")), [])
