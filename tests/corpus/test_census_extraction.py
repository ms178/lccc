"""The census extractor must be blind to source layout.

Regression cover for a false alarm the drift ratchet produced against a tree
that had not changed semantically: `cargo fmt` (a required gate) splits probe
entries that do not fit on one line *before* the name, and the extraction
pattern required the opening paren immediately before the quoted name, so 19
AVX-512 names disappeared from the census and the CI gate failed with
"CENSUS DRIFT ... only in census: avx512bw ...".

The extractor now reads each table's body and matches on the entry's own
shape, so both layouts yield the same name set.
"""

from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO / "scripts"))

import cpu_supports_name_census as census  # noqa: E402

INLINE = """\
pub const PROBE_TABLE: &[(&str, Probe)] = &[
    ("cmov", Probe::new(CpuWord::Leaf1Edx, 15, Xstate::None)),
    ("avx512bw", Probe::new(CpuWord::Leaf7Ebx, 30, Xstate::Avx512)),
];

pub const NAME_ALIASES: &[(&str, &str)] = &[
    ("pni", "sse3"),
];
"""

REFLOWED = """\
pub const PROBE_TABLE: &[(&str, Probe)] = &[
    ("cmov", Probe::new(CpuWord::Leaf1Edx, 15, Xstate::None)),
    (
        "avx512bw",
        Probe::new(CpuWord::Leaf7Ebx, 30, Xstate::Avx512),
    ),
];

pub const NAME_ALIASES: &[(&str, &str)] = &[
    ("pni", "sse3"),
];
"""

# A name that only appears in prose must not enter the set, and neither may a
# name in a second table that is not PROBE_TABLE.
DECOY = """\
//! `fictional` is mentioned in a doc comment only.
pub const PROBE_TABLE: &[(&str, Probe)] = &[
    ("sse2", Probe::new(CpuWord::Leaf1Edx, 26, Xstate::None)),
];

pub const OTHER: &[(&str, Probe)] = &[
    ("decoy", Probe::new(CpuWord::Leaf1Edx, 0, Xstate::None)),
];
"""


def names_for(source: str) -> set[str]:
    with tempfile.TemporaryDirectory() as td:
        root = Path(td) / "src" / "common"
        root.mkdir(parents=True)
        (root / "cpu_probe.rs").write_text(source, encoding="utf-8")
        return census.lccc_names(Path(td))


class CensusExtractionTest(unittest.TestCase):
    def test_inline_and_reflowed_sources_agree(self) -> None:
        inline = names_for(INLINE)
        reflowed = names_for(REFLOWED)
        self.assertEqual(inline, reflowed)
        self.assertEqual(inline, {"cmov", "avx512bw", "pni"})

    def test_reflowed_entry_is_not_lost(self) -> None:
        reflowed = names_for(REFLOWED)
        self.assertIn("avx512bw", reflowed, "the rustfmt layout must not hide a name")

    def test_prose_and_other_tables_do_not_leak(self) -> None:
        found = names_for(DECOY)
        self.assertEqual(found, {"sse2"})
        self.assertNotIn("fictional", found)
        self.assertNotIn("decoy", found)

    def test_gcc_isa_names_table_is_parsed(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            header = Path(td) / "i386-isas.h"
            header.write_text(
                'ISA_NAMES_TABLE_ENTRY("x86-64-v2", FEATURE_X86_64_V2, P_X86_64_V2, NULL)\n'
                '  ISA_NAMES_TABLE_ENTRY ( "amx-fp8", FEATURE_AMX_FP8, P_NONE, "-mamx-fp8")\n'
                '/* ISA_NAMES_TABLE_START */\n',
                encoding="utf-8",
            )
            self.assertEqual(census.gcc_isa_names(header), ["x86-64-v2", "amx-fp8"])


if __name__ == "__main__":
    unittest.main()
