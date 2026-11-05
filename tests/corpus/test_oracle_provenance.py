#!/usr/bin/env python3
"""Generated oracle artifacts must state which oracle produced them.

A version bump that relabels an old measurement as current is a silent lie:
the numbers stay valid only while the recorded oracle is named. These guards
make provenance machine-readable and tie the expectation headers to the one
live Clang pin, so the next bump cannot forget either file.
"""

from __future__ import annotations

import json
import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
BASELINE = ROOT / "tests/regression/sse2_codegen_matrix.json"
GRID = ROOT / "tests/regression/ext_vector_abi/grid_expectations.txt"
INSTALLER = ROOT / "scripts/ci_install_clang_oracle.sh"
MATRIX_GATE = ROOT / "tests/regression/check_sse2_codegen_matrix.py"


def live_clang_pin() -> str:
    match = re.search(r"^VERSION=([0-9.]+)$", INSTALLER.read_text(), re.MULTILINE)
    assert match, "ci_install_clang_oracle.sh has no single VERSION pin"
    return match.group(1)


class OracleProvenance(unittest.TestCase):
    def test_baseline_records_the_oracles_that_measured_it(self) -> None:
        data = json.loads(BASELINE.read_text())
        self.assertIn("provenance", data)
        self.assertEqual(data["provenance"]["recorded_with"], data["toolchains"])
        self.assertIn("23.1.4", data["provenance"]["note"])
        self.assertIn("--update-baseline", data["provenance"]["note"])

    def test_generator_keeps_provenance_on_regeneration(self) -> None:
        # --update-baseline rewrites the whole file; if it dropped the field,
        # a re-derived baseline would again be indistinguishable from an old
        # measurement. This is a source-level contract, not a data assertion.
        text = MATRIX_GATE.read_text()
        self.assertIn('base["provenance"]', text)
        self.assertIn('"recorded_with"', text)

    def test_grid_expectations_name_the_live_oracle(self) -> None:
        pin = live_clang_pin()
        header = "\n".join(GRID.read_text().splitlines()[:16])
        self.assertIn(pin, header)
        self.assertIn("Re-validated", header)
        self.assertIn("23.1.4", header)
        self.assertIn("tracking references stay stable", header)

    def test_grid_expectation_rows_are_still_parsable(self) -> None:
        # Comments may be edited freely; data rows must not be touched.
        for line in GRID.read_text().splitlines():
            if not line or line.lstrip().startswith("#"):
                continue
            fields = line.split()
            self.assertGreaterEqual(len(fields), 7, line)
            self.assertIn(fields[5], ("oracle", "lccc", "flaky"), line)

    def test_baseline_schema_keys_are_preserved(self) -> None:
        data = json.loads(BASELINE.read_text())
        for key in ("configs", "debt", "verdicts", "toolchains", "mnemonics"):
            self.assertIn(key, data)


if __name__ == "__main__":
    unittest.main()
