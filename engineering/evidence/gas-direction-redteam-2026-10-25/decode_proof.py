#!/usr/bin/env python3
"""Re-run the snapshot-vs-objdump decode proof and write it as JSON.

`tests/corpus/test_gas_snapshot_decode.py` asserts agreement; this script emits
the evidence: every direction-sensitive encoding of `scripts/isa/gas-2.47.json`
as bytes, the mnemonic `objdump` prints for it, and the snapshot's own record.

    ASMDIFF_TEST_AS=<gas-2.47>/as ASMDIFF_TEST_OBJDUMP=<gas-2.47>/objdump \
        python3 engineering/evidence/gas-direction-redteam-2026-10-25/decode_proof.py \
        > decode-proof.json
"""

from __future__ import annotations

import importlib.util
import json
import os
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
TEST = ROOT / "tests" / "corpus" / "test_gas_snapshot_decode.py"


def main() -> int:
    spec = importlib.util.spec_from_file_location("decode_test", TEST)
    mod = importlib.util.module_from_spec(spec)
    sys.modules["decode_test"] = mod
    spec.loader.exec_module(mod)
    if mod._objdump() is None:
        print("ASMDIFF_TEST_AS / ASMDIFF_TEST_OBJDUMP must point at the "
              "pinned binutils 2.47 pair", file=sys.stderr)
        return 2
    case = mod.GasSnapshotDecodeTests("test_the_snapshot_agrees_with_objdump")
    case.setUpClass()
    slots = case._slots()
    decoded = case._decode_all(slots)
    rows = []
    for index, (name, enc, payload) in enumerate(slots):
        rows.append({
            "mnemonic": name,
            "bytes": payload.hex(),
            "encoding": {k: enc[k] for k in ("family", "map", "pp", "w",
                                             "opcode", "reg")},
            "objdump": decoded.get(index),
        })
    print(json.dumps({
        "tool": os.environ.get(mod.OBJDUMP_ENV),
        "slots": len(rows),
        "exempt": {f"{k[0]} 0x{k[1]:02x}": v for k, v in mod.EXEMPT.items()},
        "rows": rows,
    }, indent=1, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
