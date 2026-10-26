"""Every declared encoder row must be a legal Intel-SDM encoding.

``scripts/sdm_isa_crossref.py`` answers a question the as-2.47 differential
cannot: for *every* mnemonic the encoders declare -- not just the ones a corpus
happens to exercise -- is the byte sequence a legal form of that instruction?
The reference is a vendored catalogue derived from Intel's own ISA descriptions
(intelxed/xed ``datafiles/*.txt``), so the check is hermetic and runs in CI.

These tests pin the properties that make the gate trustworthy:

* it reaches every row of both backends and reports *nothing* (no finding, no
  unmodelled row) -- a gate that quietly skips rows is worse than no gate;
* the model is derived from the helper bodies, so a wrong group digit, a wrong
  operand size or a wrong opcode map is reported instead of being waved through;
* the reference carries provenance for every source file, and the one class XED
  does not model (`prefetchw`) is supplied from the SDM by hand and is *not*
  smuggled into the generated catalogue.

The catalogue is regenerated from upstream with::

    gh api repos/intelxed/xed/contents/<path> --jq .content | base64 -d > /tmp/xed/ref/<name>
    python3 scripts/sdm_isa_crossref.py --build-catalogue scripts/isa/catalogue.json \
        --reference-dir /tmp/xed/ref --manifest scripts/isa/MANIFEST.json
"""

from __future__ import annotations

import importlib.util
import json
import subprocess
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))

SDM_PY = ROOT / "scripts" / "sdm_isa_crossref.py"
CATALOGUE = ROOT / "scripts" / "isa" / "catalogue.json"
MANIFEST = ROOT / "scripts" / "isa" / "MANIFEST.json"


def _load(path: Path, name: str):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


def _row(body: str, names: list[str], arrays: list[list[int]] | None = None) -> dict:
    return {"file": "src/backend/x86/assembler/encoder/mod.rs", "line": 1,
            "names": names, "body": body, "arrays": arrays or []}


class SdmCrossrefTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.sdm = _load(SDM_PY, "sdm_isa_crossref")
        cls.catalogue = cls.sdm.with_sdm_only_classes(json.loads(CATALOGUE.read_text()))

    # -- the gate itself ---------------------------------------------------

    def test_every_row_of_both_backends_is_verified(self):
        rows = []
        for path in self.sdm.encoder_files("all"):
            rows.extend(self.sdm.extract_rows(Path(path).resolve()))
        findings, unmodeLLed = self.sdm.verify(
            self.catalogue, [Path(ROOT / f) for f in self.sdm.encoder_files("all")],
            verbose=False, b64=True)
        self.assertGreater(len(rows), 1400)
        self.assertEqual(
            [(r["file"], r["line"], r["body"]) for r in unmodeLLed], [],
            "a row the model cannot read is exactly the row that hides a bug")
        report = "\n".join(
            f"{kind} {name} at {row['file']}:{row['line']} ({row['body'][:60]})"
            for row, name, kind, _ in findings[:20])
        self.assertEqual(findings, [], report)

    def test_strict_mode_exits_zero(self):
        proc = subprocess.run(
            [sys.executable, str(SDM_PY), "--backend", "all", "--strict"],
            cwd=ROOT, capture_output=True, text=True)
        self.assertEqual(proc.returncode, 0, proc.stdout[-2000:] + proc.stderr[-2000:])

    def test_catalogue_provenance(self):
        manifest = json.loads(MANIFEST.read_text())
        self.assertIn("Regenerate, do not hand-edit", manifest["note"])
        self.assertGreaterEqual(len(manifest["sources"]), 30)
        for source in manifest["sources"]:
            self.assertEqual(len(source["sha256"]), 64, source["path"])
            self.assertGreater(source["bytes"], 0, source["path"])
            self.assertTrue(source["license"], source["path"])
        self.assertEqual(manifest["classes"], len(json.loads(CATALOGUE.read_text())))
        self.assertGreaterEqual(manifest["classes"], 1400)
        self.assertGreaterEqual(manifest["encodings"], 4900)

    # -- the reference is parsed, not guessed ------------------------------

    def test_register_only_modrm_is_not_a_missing_modrm_byte(self):
        enc = self.sdm.parse_pattern("0x0F 0x78 osz_refining_prefix REFINING66() MOD[0b11] MOD=3 REG[0b000] RM[nnn] UIMM8() UIMM8_1()")
        self.assertTrue(enc['modrm'])
        self.assertEqual(enc['modrm_mode'], 'register')
        self.assertEqual(enc['reg'], 0)
        mem = self.sdm.parse_pattern("0x0F 0x2B f2_refining_prefix MOD[mm] MOD!=3 REG[rrr] RM[nnn] MODRM()")
        self.assertTrue(mem['modrm'])
        self.assertEqual(mem['modrm_mode'], 'memory')

    def test_one_iclass_can_carry_several_patterns(self):
        block = (
            "\n{\nICLASS    : MOVUPS\n"
            "PATTERN   : 0x0F 0x10 MOD[mm] MOD!=3 REG[rrr] RM[nnn] MODRM()\n}\n{\n"
            "ICLASS    : MOVUPS\n"
            "PATTERN   : 0x0F 0x11 MOD[mm] MOD!=3 REG[rrr] RM[nnn] MODRM()\n}\n")
        tmp = Path(self.id().replace(".", "_") + ".txt")
        tmp.write_text(block)
        try:
            table = self.sdm.parse_reference([tmp])
        finally:
            tmp.unlink()
        self.assertEqual(
            sorted(e["opcode"] for e in table["MOVUPS"]), [0x10, 0x11])

    def test_register_in_opcode_patterns_are_parsed(self):
        # `0b1011_0 SRM[rrr]` is `mov r8, imm8` = 0xB0; missing it left the
        # catalogue with `opcode: null` and every register-immediate MOV row
        # unmatched.
        enc = self.sdm.parse_pattern(
            "0b1011_0 SRM[rrr] UIMM8()")
        self.assertEqual(enc["opcode"], 0xB0)
        enc = self.sdm.parse_pattern(
            "0b0101_1 SRM[rrr] REX2=0 DF64()")
        self.assertEqual(enc["opcode"], 0x58)

    def test_evex_patterns_become_evex_encodings(self):
        enc = self.sdm.parse_pattern(
            "EVV 0xEF V66 V0F MOD[0b11] MOD=3 REG[rrr] RM[nnn] VL512 W1")
        self.assertEqual((enc["family"], enc["map"], enc["pp"], enc["w"],
                          enc["opcode"]), ("evex", "0f", "66", 1, 0xEF))

    def test_sdm_only_class_is_not_in_the_generated_catalogue(self):
        self.assertNotIn("PREFETCHW", json.loads(CATALOGUE.read_text()))
        extra = self.sdm.SDM_ONLY_CLASSES["PREFETCHW"][0]
        self.assertEqual((extra["map"], extra["opcode"], extra["reg"]),
                         ("0f", 0x0D, 1))
        self.assertIn("PREFETCHW", self.catalogue)

    # -- modelling rules that used to be wrong -----------------------------

    def test_group_digit_comes_from_the_helper_argument(self):
        neg = self.sdm.model_row(_row("self.encode_unary_rm(ops, 3, 4)",
                                      ["negl"]), True, "negl")
        self.assertEqual([(e["opcode"], e["reg"]) for e in neg], [(0xF7, 3)])
        # `inc`/`dec` are group 5, not group 3: the same helper picks the table
        # from the digit, so a wrong digit shows up as an illegal encoding.
        inc = self.sdm.model_row(_row("self.encode_unary_rm(ops, 0, 4)",
                                      ["incl"]), True, "incl")
        self.assertEqual([(e["opcode"], e["reg"]) for e in inc], [(0xFF, 0)])

    def test_size_suffix_must_match_the_helper_argument(self):
        row = _row("self.encode_unary_rm(ops, 3, 4)", ["negw"], [[0xF7]])
        self.assertTrue(self.sdm.size_mismatch(row, "negw"))
        self.assertFalse(self.sdm.size_mismatch(row, "negl"))

    def test_avx_move_is_a_vex_encoding(self):
        encs = self.sdm.model_row(
            _row("self.encode_avx_mov_np(ops, 0x10, 0x11, true)", ["vmovups"]),
            True, "vmovups")
        self.assertEqual({(e["family"], e["map"], e["pp"], e["opcode"]) for e in encs},
                         {("vex", "0f", "66", 0x10), ("vex", "0f", "66", 0x11)})

    def test_vmovdqu_takes_the_f3_prefix_not_none(self):
        encs = self.sdm.model_row(
            _row("self.encode_avx_mov(ops, 0x6F, 0x7F, false)", ["vmovdqu"]),
            True, "vmovdqu")
        self.assertEqual({e["pp"] for e in encs}, {"f3"})
        self.assertEqual({e["opcode"] for e in encs}, {0x6F, 0x7F})

    def test_broadcast_and_ldmxcsr_are_vex_forms(self):
        bcast = self.sdm.model_row(
            _row("self.encode_avx_broadcast(ops, &[0x18])", ["vbroadcastss"],
                 [[0x18]]), True, "vbroadcastss")
        self.assertEqual((bcast[0]["family"], bcast[0]["map"], bcast[0]["pp"],
                          bcast[0]["opcode"]), ("vex", "0f38", "66", 0x18))
        ldmxcsr = self.sdm.model_row(
            _row("self.encode_vex_mem_only(ops, &[0x0F, 0xAE], 2)", ["vldmxcsr"],
                 [[0x0F, 0xAE]]), True, "vldmxcsr")
        self.assertEqual((ldmxcsr[0]["family"], ldmxcsr[0]["map"],
                          ldmxcsr[0]["opcode"], ldmxcsr[0]["reg"]),
                         ("vex", "0f", 0xAE, 2))

    def test_multi_line_row_is_modelled_by_its_encoding_call(self):
        rows = self.sdm.extract_rows(
            (ROOT / "src/backend/x86/assembler/encoder/mod.rs").resolve())
        vround = next(r for r in rows if r["names"] == ["vroundss"])
        encs = self.sdm.model_row(vround, True, "vroundss")
        self.assertEqual((encs[0]["family"], encs[0]["map"], encs[0]["opcode"]),
                         ("vex", "0f3a", 0x0A))

    def test_movslq_is_the_base_map_movsxd(self):
        encs = self.sdm.model_row(
            _row("self.encode_movsx(ops, 4, 8)", ["movslq"]), True, "movslq")
        self.assertEqual((encs[0]["map"], encs[0]["opcode"]), ("none", 0x63))
        literal = self.sdm._upper_class("movslq")
        self.assertIn("MOVSXD", literal)

    def test_att_x87_pop_forms_are_swapped(self):
        # as 2.47: `fsubp` = de e1 (SDM FSUBRP = DE /4), `fsubrp` = de e9
        # (SDM FSUBP = DE /5), and likewise for the divide pair.
        self.assertEqual(self.sdm.ALIASES["fsubp"], "FSUBRP")
        self.assertEqual(self.sdm.ALIASES["fdivp"], "FDIVRP")
        pops = {name: {(e["opcode"], e["reg"]) for e in self.catalogue[name]}
                for name in ("FSUBP", "FSUBRP", "FDIVP", "FDIVRP")}
        self.assertEqual(pops["FSUBRP"], {(0xDE, 4)})
        self.assertEqual(pops["FSUBP"], {(0xDE, 5)})
        self.assertEqual(pops["FDIVRP"], {(0xDE, 6)})
        self.assertEqual(pops["FDIVP"], {(0xDE, 7)})

    def test_x87_arith_uses_the_base_byte_digit(self):
        # `fsub` (base 0xE0) is /4; `fsubr` (base 0xE8) is /5.  A remap table
        # once swapped them, which encoded `fsub` as `fsubr`.
        for base, digit in ((0xE0, 4), (0xE8, 5)):
            encs = self.sdm.model_row(
                _row(f"self.encode_x87_arith_reg(ops, 0xD8, 0xDC, {base:#04x})",
                     ["fsub"]), True, "fsub")
            self.assertEqual({e["reg"] for e in encs}, {digit})
            self.assertEqual({e["opcode"] for e in encs}, {0xD8, 0xDC})

    # -- helpers added to close the last unmodelled rows -------------------

    def test_imul_and_mov_cover_every_operand_shape(self):
        imul = self.sdm.model_row(_row("self.encode_imul(ops, 4)", ["imull"]),
                                  True, "imull")
        self.assertEqual({(e["map"], e["opcode"], e["reg"]) for e in imul},
                         {("none", 0xF7, 5), ("0f", 0xAF, None),
                          ("none", 0x6B, None), ("none", 0x69, None)})
        mov = self.sdm.model_row(_row("self.encode_mov(ops, 8)", ["movq"]),
                                 True, "movq")
        self.assertEqual({(e["opcode"], e["reg"]) for e in mov},
                         {(0xB8, None), (0x89, None), (0x8B, None), (0xC7, 0)})

    def test_apx_push2_pop2_are_evex_map4(self):
        push2 = self.sdm.model_row(
            _row("self.encode_push2(ops, false)", ["push2"]), True, "push2")
        self.assertEqual((push2[0]["family"], push2[0]["map"], push2[0]["w"],
                          push2[0]["opcode"], push2[0]["reg"]),
                         ("evex", "map4", 0, 0xFF, 6))
        push2p = self.sdm.model_row(
            _row("self.encode_push2(ops, true)", ["push2p"]), True, "push2p")
        self.assertEqual(push2p[0]["w"], 1)

    def test_user_msr_prefix_follows_the_mnemonic(self):
        urd = self.sdm.model_row(
            _row('self.encode_user_msr(ops, mnemonic)', ["urdmsr", "uwrmsr"]),
            True, "urdmsr")
        uwr = self.sdm.model_row(
            _row('self.encode_user_msr(ops, mnemonic)', ["urdmsr", "uwrmsr"]),
            True, "uwrmsr")
        self.assertEqual((urd[0]["map"], urd[0]["pp"]), ("map7", "f2"))
        self.assertEqual((uwr[0]["map"], uwr[0]["pp"]), ("map7", "f3"))

    def test_vcvt_table_is_read_from_the_encoder(self):
        table = self.sdm._vcvt_table()
        self.assertGreaterEqual(len(table), 40)
        convert = self.sdm.model_row(
            _row('self.encode_vcvt(ops, "vcvtps2pd")', ["vcvtps2pd"]),
            True, "vcvtps2pd")
        self.assertEqual((convert[0]["family"], convert[0]["pp"], convert[0]["opcode"]),
                         ("vex", "none", 0x5A))
        narrowing = self.sdm.model_row(
            _row('self.encode_vcvt(ops, "vcvtpd2ps")', ["vcvtpd2ps"]),
            True, "vcvtpd2ps")
        self.assertEqual({e["family"] for e in narrowing}, {"vex", "evex"})

    def test_setcc_and_string_ops_use_the_mnemonic(self):
        setcc = self.sdm.model_row(_row('self.encode_setcc(ops, "setc")',
                                        ["setcc"]), True, "setcc")
        self.assertEqual(setcc[0]["opcode"], 0x92)
        self.assertIn("SETB", self.sdm._upper_class("setcc"))
        movsb = self.sdm.model_row(
            _row('self.encode_string_op(ops, 0xA4, string_op_size(mnemonic)?, '
                 'Some(0), Some(1), "movs")', ["movsb"]), True, "movsb")
        self.assertEqual([e["opcode"] for e in movsb], [0xA4])
        movsq = self.sdm.model_row(
            _row('self.encode_string_op(ops, 0xA4, string_op_size(mnemonic)?, '
                 'Some(0), Some(1), "movs")', ["movsq"]), True, "movsq")
        self.assertEqual([e["opcode"] for e in movsq], [0xA5])

    def test_relocation_rows_are_not_mnemonics(self):
        for name in ("gotpcrel", "gottpoff", "tlsdesc", "es", "fs"):
            self.assertIn(name, self.sdm.NOT_A_MNEMONIC)


if __name__ == "__main__":
    unittest.main()
