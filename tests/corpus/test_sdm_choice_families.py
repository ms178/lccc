"""Every SDM encoding choice must be covered by the sweep, or be explained.

`gen_encoding_sweep.py` enumerates the instructions where the ISA offers more
than one legal encoding, and `encdiff.py` judges the compiler against the
shortest one any oracle produced.  The weak point is the family list itself: it
is hand-written, so a family the author forgot is a family nobody measures and
the sweep still passes, because it never generated a case.

`scripts/sdm_choice_families.py` closes that hole by holding the same families
as data, derived from the Intel SDM (via XED's ISA description) and re-checkable
against it with `--reparse`.  This test runs the hermetic half of that check in
CI: internal consistency plus coverage, with no network and no reference file,
because a gate that needs the internet is a gate that gets skipped.

The `--reparse` half is deliberately NOT run here: it needs a 448 KB reference
the tree does not vendor.  Run it by hand when the SDM is revised, and refresh
the pinned digest with the result.
"""

from __future__ import annotations

import importlib.util
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))

SDM = ROOT / "scripts" / "sdm_choice_families.py"
SWEEP = ROOT / "scripts" / "gen_encoding_sweep.py"


def _load(path: Path, name: str):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


class SdmChoiceFamilyCoverage(unittest.TestCase):
    """The family table and the sweep must not drift apart."""

    @classmethod
    def setUpClass(cls) -> None:
        cls.sdm = _load(SDM, "lccc_sdm_choice_families")

    def test_every_family_is_covered_or_documented(self) -> None:
        self.assertEqual(self.sdm.check_hermetic(), [])

    def test_the_table_is_not_empty(self) -> None:
        # A table that silently became empty would pass every check above.
        self.assertGreaterEqual(len(self.sdm.FAMILIES), 25)
        self.assertGreaterEqual(len({f[1] for f in self.sdm.FAMILIES}), 15)

    def test_each_family_names_a_short_and_a_different_long_form(self) -> None:
        for name, iclass, short, long_, _coverage in self.sdm.FAMILIES:
            self.assertNotEqual(short, long_, f"{name}/{iclass}")
            self.assertTrue(
                short.startswith("0x") or short[0].isdigit(),
                f"{name}/{iclass}: {short!r} is not an opcode",
            )

    def test_coverage_names_a_real_sweep_group_or_a_documented_mechanism(
        self,
    ) -> None:
        groups = self.sdm.sweep_groups()
        # The registry the check reads is the one the generator dispatches on.
        sweep = _load(SWEEP, "lccc_gen_encoding_sweep")
        self.assertEqual(groups, set(sweep.GROUPS))
        for name, iclass, _s, _l, coverage in self.sdm.FAMILIES:
            if coverage.startswith("relaxation:"):
                continue
            self.assertIn(coverage, groups, f"{name}/{iclass}")

    def test_the_two_families_the_sweep_used_to_miss_are_present(self) -> None:
        # `imul` imm8 is three bytes per multiply and a multiply is never cold;
        # branches are handled by relaxation rather than by the encoder. Both
        # were absent from the generator before this check existed.
        names = {f[0] for f in self.sdm.FAMILIES}
        self.assertIn("imul_imm8_sext", names)
        self.assertIn("jmp_rel8", names)
        sweep = _load(SWEEP, "lccc_gen_encoding_sweep_missed")
        self.assertIn("imul_imm", sweep.GROUPS)
        self.assertIn("jcc_rel", sweep.GROUPS)

        emitted: list[str] = []
        sweep.GROUPS["imul_imm"](emitted)
        self.assertTrue(
            any(i.startswith("imul $1,") for i in emitted),
            "the imul group must generate an immediate that fits a byte",
        )
        self.assertTrue(
            any(i.startswith("imul $128,") for i in emitted),
            "and one that does not, or the boundary is untested",
        )
        self.assertTrue(
            any("(%rdi), %ecx" in i for i in emitted),
            "and both a register and a memory source, which use different rows",
        )


class SdmReferenceParser(unittest.TestCase):
    """`--reparse` must actually read an XED-format ISA description."""

    def test_parses_classes_and_opcodes_from_a_synthetic_file(self) -> None:
        sdm = _load(SDM, "lccc_sdm_choice_families_parse")
        import tempfile

        text = (
            "#BEGIN_LEGAL\n# Copyright (c) 2026 Intel Corporation\n#END_LEGAL\n"
            "INSTRUCTIONS()::\n{\nICLASS    : ADD\n"
            "PATTERN   : 0x81 MOD[mm] MOD!=3 REG[0b000] RM[nnn] MODRM() SIMMz()\n"
            "OPERANDS  : MEM0:rw:v IMM0:r:z\n}\n"
            "{\nICLASS    : ADD\n"
            "PATTERN   : 0x83 MOD[mm] MOD!=3 REG[0b000] RM[nnn] MODRM() SIMM8()\n"
            "OPERANDS  : MEM0:rw:v IMM0:r:b\n}\n"
        )
        with tempfile.NamedTemporaryFile("w", suffix=".txt", delete=False) as fh:
            fh.write(text)
            path = Path(fh.name)
        try:
            table = sdm.parse_sdm_isa(path)
        finally:
            path.unlink()
        self.assertEqual(table["ADD"], {"0x81", "0x83"})

    def test_a_missing_opcode_is_reported(self) -> None:
        sdm = _load(SDM, "lccc_sdm_choice_families_missing")
        import tempfile

        # ADD with only the long form: the table's 0x83 claim must fail.
        text = (
            "INSTRUCTIONS()::\n{\nICLASS    : ADD\n"
            "PATTERN   : 0x81 MOD[mm] MOD!=3 REG[0b000] RM[nnn] MODRM()\n"
            "OPERANDS  : MEM0:rw:v IMM0:r:z\n}\n"
        )
        with tempfile.NamedTemporaryFile("w", suffix=".txt", delete=False) as fh:
            fh.write(text)
            path = Path(fh.name)
        try:
            problems = sdm.check_reparse(path)
        finally:
            path.unlink()
        self.assertTrue(
            any("0x83" in p and "ADD" in p for p in problems),
            f"expected a missing-opcode report, got {problems}",
        )


if __name__ == "__main__":
    unittest.main()
