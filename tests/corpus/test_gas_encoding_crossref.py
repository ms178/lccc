"""Every declared encoder row must be a legal GNU as 2.47 encoding.

The differential harness compares LCCC against `as` 2.47 byte for byte, but only
for the instructions a corpus happens to contain.  ``scripts/gas_encoding_crossref.py``
closes that gap: it expands `opcodes/i386-opc.tbl` — GNU as 2.47's own
instruction table, including its template blocks — into a vendored snapshot, and
checks every declarative row of the i686 encoder against it.

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
