"""Every declared encoder row must be a legal GNU as 2.47 encoding.

The differential harness compares LCCC against `as` 2.47 byte for byte, but only
for the instructions a corpus happens to contain.  ``scripts/gas_encoding_crossref.py``
closes that gap: it expands `opcodes/i386-opc.tbl` — GNU as 2.47's own
instruction table, including its template blocks — into a vendored snapshot, and
checks every declarative row of *both* encoder backends (`i686` and `x86`)
against it.

This test runs that check hermetically (no network, no reference file) so a row
that is added with a wrong opcode, prefix, map or `/digit` fails in CI even when
no corpus exercises it, and so the claim "cross-referenced with GNU as 2.47"
stays true as the encoder grows.

The snapshot is regenerated from upstream with::

    gh api repos/RTEMS/sourceware-mirror-binutils-gdb/contents/opcodes/i386-opc.tbl \\
        ?ref=binutils-2_47-branch --jq .content | tr -d '\\n' | base64 -d > /tmp/i386-opc.tbl
    python3 scripts/gas_encoding_crossref.py --build /tmp/i386-opc.tbl

``scripts/isa/gas-2.47.manifest.json`` records the upstream path, ref and blob
digest of the table the snapshot was built from.

The two backends differ in one thing the model cannot infer from a row: `40..4F`
is a REX prefix in 64-bit mode and an `inc`/`dec` opcode in 32-bit mode, so the
mode is taken from the row's own path (`sdm.model_any`).
"""

from __future__ import annotations

import importlib.util
import json
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))

CROSSREF = ROOT / "scripts" / "gas_encoding_crossref.py"
SDM = ROOT / "scripts" / "sdm_isa_crossref.py"


def _load(path: Path, name: str):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


class GasEncodingCrossrefTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.gas = _load(CROSSREF, "gas_encoding_crossref")
        cls.sdm = _load(SDM, "sdm_isa_crossref")
        cls.table = cls.gas.load_snapshot()
        cls.rows = []
        for path in cls.sdm.ENCODER_FILES:
            cls.rows.extend(cls.sdm.extract_rows(Path(path).resolve()))

    def test_snapshot_has_provenance(self):
        manifest = json.loads((ROOT / "scripts" / "isa" / "gas-2.47.manifest.json").read_text())
        upstream = manifest["upstream"]
        self.assertEqual(upstream["ref"], "binutils-2_47-branch")
        self.assertEqual(upstream["path"], "opcodes/i386-opc.tbl")
        self.assertEqual(len(upstream["sha256"]), 64)
        # The table is generated from template blocks; a snapshot built without
        # expanding them loses the largest families (the ALU, the <sd>/<bw>
        # pairs), which would silently shrink the gate instead of failing it.
        self.assertGreater(manifest["stats"]["declarations"], 60)
        self.assertGreater(manifest["mnemonics"], 1500)
        self.assertGreater(manifest["encodings"], 3000)

    def test_snapshot_pins_the_upstream_table_it_was_built_from(self):
        """The snapshot is only an oracle if it came from *that* table.

        `upstream.sha256` is the digest of `opcodes/i386-opc.tbl` at
        `binutils-2_47-branch`; a regeneration from another branch or a docored
        copy would change it.  The digest below was re-fetched from upstream and
        a rebuild from those bytes reproduces `scripts/isa/gas-2.47.json`
        byte-for-byte.
        """
        manifest = json.loads(
            (ROOT / "scripts" / "isa" / "gas-2.47.manifest.json").read_text())
        upstream = manifest["upstream"]
        self.assertEqual(upstream["repo"], "RTEMS/sourceware-mirror-binutils-gdb")
        self.assertEqual(
            upstream["sha256"],
            "a48171c05930cbe120a26fdf802689a4865607b3fe42eca784541dbf7ba81179")
        self.assertEqual(upstream["bytes"], 288187)

    def test_every_row_is_a_legal_encoding(self):
        findings, unmodeLLed, checked, _ = self.gas.verify(
            self.rows, self.table,
            lambda row, name=None: self.sdm.model_row(row, False, name))
        self.assertEqual([r["body"] for r in unmodeLLed], [])
        self.assertGreater(checked, 400)
        self.assertEqual(
            findings,
            [],
            "\n".join(
                f"{f['kind']} {f['name']} at {f['file']}:{f['line']}: {f['body']}"
                for f in findings[:20]
            ),
        )

    def test_both_backends_are_legal_encodings(self):
        rows = []
        for path in self.sdm.encoder_files("all"):
            rows.extend(self.sdm.extract_rows(Path(path).resolve()))
        findings, unmodeLLed, checked, _ = self.gas.verify(
            rows, self.table, self.sdm.model_any)
        self.assertGreater(checked, 900)
        self.assertLess(len(unmodeLLed), len(rows))
        self.assertEqual(
            findings,
            [],
            "\n".join(
                f"{f['kind']} {f['name']} at {f['file']}:{f['line']}: {f['body']}"
                for f in findings[:20]
            ),
        )

    def test_32_bit_inc_dec_is_not_read_as_a_rex_prefix(self):
        # `40` is `inc %eax` in 32-bit mode, so a 32-bit row must keep it; the
        # 64-bit backend reads the same byte as a REX prefix.
        row = {"file": "src/backend/i686/assembler/encoder/mod.rs", "line": 1,
               "names": ["incl"], "body": "self.encode_inc_dec(ops, 0x40)", "arrays": [[0x40]]}
        self.assertFalse(self.sdm.row_is_64bit(row))
        self.assertEqual(self.sdm.model_any(row)[0]["opcode"], 0x40)
        self.assertTrue(self.sdm.row_is_64bit({**row, "file": row["file"].replace("i686", "x86")}))

    def test_coverage_lists_only_mnemonics_without_a_row(self):
        table = {
            # Implemented: must not appear in the worklist.
            "addl": [{"family": "legacy", "map": "none", "pp": "none",
                      "opcode": 0x01, "w": None, "reg": None}],
            # VEX-encoded and unimplemented, and EVEX-only (out of scope for a
            # VEX/legacy backend), which the worklist drops.
            "vmptrld": [{"family": "vex", "map": "0f38", "pp": "none",
                         "opcode": 0x06, "w": None, "reg": 6}],
            "vpaddd": [{"family": "evex", "map": "0f", "pp": "66",
                        "opcode": 0xFE, "w": None, "reg": None}],
        }
        rows = [{"file": "x", "line": 1, "names": ["addl"],
                 "body": "self.encode_alu(ops)", "arrays": []}]
        self.assertEqual(sorted(self.gas.coverage(rows, table)), ["vmptrld"])

    def test_single_byte_opcodes_are_not_read_as_prefixes(self):
        """The BMI rows spell their opcode as a lone `F2`/`F3` byte.

        `opcodes/i386-opc.tbl` packs `[mandatory prefix] opcode` into one field,
        and a three-byte VEX or EVEX row leaves the opcode alone in it: `andn
        0xf2` is VEX.NDS.LZ.0F38.W0 F2, `blsi 0xf3/3` is ...F3 /3.  A distiller
        that reads that byte as a prefix has no opcode left and silently drops
        the row, which is how `andn`/`blsi`/`blsr`/`blsmsk` used to be missing
        from the snapshot while being perfectly legal encodings.
        """

        def encs(name):
            return {(e["family"], e["map"], e["pp"], e["opcode"], e["reg"])
                    for e in self.table.get(name, [])}

        self.assertIn(("vex", "0f38", "none", 0xF2, None), encs("andn"))
        self.assertIn(("vex", "0f38", "none", 0xF3, 3), encs("blsi"))
        self.assertIn(("vex", "0f38", "none", 0xF3, 1), encs("blsr"))
        self.assertIn(("vex", "0f38", "none", 0xF3, 2), encs("blsmsk"))
        # Same shape, EVEX-only encoding: EVEX.66.0F3A.W0 66 /r ib.
        self.assertIn(("vex", "0f3a", "66", 0x66, None), encs("vfpclassps"))

    def test_prefix_and_opcode_in_one_field_still_split(self):
        """`shrx` is `0xf2f7`: mandatory prefix F2, opcode F7."""
        forms = {(e["family"], e["map"], e["pp"], e["opcode"], e["reg"])
                 for e in self.table["shrx"]}
        self.assertIn(("vex", "0f38", "f2", 0xF7, None), forms)

    def test_three_byte_opcode_rows_keep_their_modrm_digit(self):
        """`0F 01 <ModRM>`: the digit the row encodes is ModRM's `/reg` field.

        The table spells the whole thing in one field (`wrmsrns 0x0f01c6`,
        `sgdt 0x0f010/0`), so the distiller has to split escape, opcode and
        ModRM.  Verified against GNU as 2.47: `swapgs` is `0F 01 F8` (=/7),
        `monitor` `0F 01 C8` (=/1), `xgetbv` `0F 01 D0` (=/2), `rdpkru` `0F 01
        EE` (=/5), `wrmsrns` `0F 01 C6` (=/0).
        """

        def encs(name):
            return {(e["family"], e["map"], e["opcode"], e["reg"])
                    for e in self.table.get(name, [])}

        for name, digit in (("wrmsrns", 0), ("sgdt", 0), ("monitor", 1),
                            ("mwait", 1), ("xgetbv", 2), ("serialize", 5),
                            ("rdpkru", 5), ("swapgs", 7), ("rdtscp", 7)):
            self.assertIn(("legacy", "0f", 0x01, digit), encs(name), name)

    def test_no_encoder_row_is_uncovered(self):
        """The gate's worklist must stay empty (MB-43).

        `andn`, `blsi`, `blsr`, `blsmsk` and the parameterised `setcc` used to
        appear here; the first four are now distilled from the table and
        `setcc` resolves through `SNAPSHOT_ALIASES` to the `setc` row it emits.
        """
        rows = []
        for path in self.sdm.encoder_files("all"):
            rows.extend(self.sdm.extract_rows(Path(path).resolve()))
        findings, _, checked, uncovered = self.gas.verify(
            rows, self.table, self.sdm.model_any)
        self.assertEqual(uncovered, [])
        self.assertEqual(findings, [])
        self.assertGreater(checked, 1600)

    def test_setcc_alias_resolves_to_the_row_it_encodes(self):
        row = {"file": "src/backend/x86/assembler/encoder/mod.rs", "line": 1,
               "names": ["setcc"], "body": 'self.encode_setcc(ops, "setc")',
               "arrays": []}
        setc = {"family": "legacy", "map": "0f", "pp": "none", "opcode": 0x92,
                "reg": 0, "w": None}
        findings, unmodeLLed, checked, uncovered = self.gas.verify(
            [row], self.table, lambda r, name=None: [dict(setc)])
        self.assertEqual((findings, unmodeLLed, uncovered), ([], [], []))
        self.assertEqual(checked, 1)

    def test_core_isa_encodings_are_pinned(self):
        """A spot check so a snapshot regenerated from the wrong branch fails."""
        def has(name, family, map_, pp, opcode):
            return any(
                e["family"] == family
                and e["map"] == map_
                and e["pp"] == pp
                and e["opcode"] == opcode
                for e in self.table.get(name, [])
            )

        self.assertTrue(has("add", "legacy", "none", "none", 0x00))
        self.assertTrue(has("add", "legacy", "none", "none", 0x83))
        self.assertTrue(has("mov", "legacy", "none", "none", 0xA0))
        self.assertTrue(has("imul", "legacy", "none", "none", 0x6B))
        self.assertTrue(has("crc32", "legacy", "0f38", "f2", 0xF0))
        self.assertTrue(has("fcmovb", "legacy", "none", "none", 0xDA))


class GasAbsentTriageTests(unittest.TestCase):
    """``--triage`` splits the declarative-row gap list by source spelling.

    The split is a reading aid, and these tests pin exactly the two ways it can
    mislead so the help text keeps saying so: a name that is only a *prefix* of
    another mnemonic must not count as a mention, and a family that composes its
    mnemonic has no literal to find.
    """

    @classmethod
    def setUpClass(cls):
        cls.gas = _load(CROSSREF, "gas_encoding_crossref")

    def _triage(self, source: str, names: list[str]):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "encoder.rs"
            path.write_text(source)
            return self.gas.triage_absent(names, [str(path)])

    def test_a_prefix_of_a_longer_mnemonic_is_not_a_mention(self):
        unmentioned, mentioned = self._triage(
            'fn f() { match m { "vaddps" | "addn" | "blsi" => {} } }',
            ["add", "vaddps", "addn"],
        )
        self.assertEqual(mentioned, ["vaddps", "addn"])
        self.assertEqual(unmentioned, ["add"])

    def test_a_quoted_or_commented_spelling_counts_as_a_mention(self):
        unmentioned, mentioned = self._triage(
            '// movbe is handled by encode_movbe\nlet n = "rorx";\n',
            ["movbe", "rorx", "pdep"],
        )
        self.assertEqual(mentioned, ["movbe", "rorx"])
        self.assertEqual(unmentioned, ["pdep"])

    def test_a_composed_mnemonic_has_no_literal_and_lands_unmentioned(self):
        # The documented limitation: `format!("vfmadd{op}132pd")` covers the
        # whole family, so the UNMENTIONED half is not a coverage verdict.
        unmentioned, mentioned = self._triage(
            'let m = format!("vfmadd{}132pd", op);\n', ["vfmadd132pd"]
        )
        self.assertEqual(unmentioned, ["vfmadd132pd"])
        self.assertEqual(mentioned, [])


if __name__ == "__main__":
    unittest.main()
