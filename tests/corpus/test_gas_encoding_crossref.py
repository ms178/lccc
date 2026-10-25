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

    def test_every_row_is_a_legal_encoding(self):
        findings, _, checked = self.gas.verify(self.rows, self.table, self.sdm.model_row)
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
        findings, unmodeLLed, checked = self.gas.verify(rows, self.table, self.sdm.model_any)
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


if __name__ == "__main__":
    unittest.main()
