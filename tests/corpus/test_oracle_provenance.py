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


def assert_provenance(data: dict, pin: str) -> None:
    """The one provenance contract, used both on the live baseline and on
    mutated negative controls.  A matrix measured with a superseded oracle
    must be rejected even if every other key looks plausible."""
    provenance = data.get("provenance", {})
    if provenance.get("recorded_with") != data.get("toolchains"):
        raise AssertionError("recorded_with is not the toolchain set that measured the rows")
    note = provenance.get("note", "")
    if "--update-baseline" not in note or pin not in note:
        raise AssertionError(f"note does not name the live pin {pin} and the regeneration command")
    if provenance.get("rederived_with", {}).get("clang") != pin:
        raise AssertionError(f"rederived_with.clang is not the live pin {pin}")
    clang = provenance.get("recorded_with", {}).get("clang", "")
    if f"clang version {pin}" not in clang:
        raise AssertionError(f"recorded Clang banner is not the live pin {pin}")
    if "Measured with Clang 23.1.4" in note:
        raise AssertionError("a stale Clang 23.1.4 measurement was relabelled as current")


class OracleProvenance(unittest.TestCase):
    def test_baseline_records_the_oracles_that_measured_it(self) -> None:
        data = json.loads(BASELINE.read_text())
        assert_provenance(data, live_clang_pin())

    def test_a_superseded_measurement_is_rejected(self) -> None:
        """Negative controls: each way a stale matrix could masquerade as a
        re-derived one must be caught by the same contract."""
        pin = live_clang_pin()
        data = json.loads(BASELINE.read_text())
        for name, mutate in (
            ("old note", lambda d: d["provenance"].update(note="Measured with Clang 23.1.4.")),
            ("old rederived_with", lambda d: d["provenance"].update(rederived_with={"clang": "23.1.4"})),
            ("old banner", lambda d: d["provenance"]["recorded_with"].update(clang="clang version 23.1.4")),
            ("dropped note", lambda d: d["provenance"].pop("note")),
            ("dropped rederived_with", lambda d: d["provenance"].pop("rederived_with")),
        ):
            with self.subTest(name=name):
                mutated = json.loads(json.dumps(data))
                mutate(mutated)
                with self.assertRaises(AssertionError):
                    assert_provenance(mutated, pin)

    def test_generator_keeps_provenance_on_regeneration(self) -> None:
        # --update-baseline rewrites the whole file; if it dropped the note
        # or the machine-readable pin, a re-derived baseline would again be
        # indistinguishable from an old measurement.  This is a source-level
        # contract, not a data assertion.
        text = MATRIX_GATE.read_text()
        for needle in ('base["provenance"]', '"recorded_with"', '"rederived_with"',
                       "clang version ([0-9]", '"note"'):
            self.assertIn(needle, text)

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
