"""The scalar-logic cohort counts distinct reports and native contracts honestly."""
import csv
import json
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[2]
DIRECTORY = ROOT / "engineering/llvm-missed"


class ScalarLogicInventory(unittest.TestCase):
    def test_fifty_distinct_retrieved_reports_have_explicit_limited_dispositions(self):
        source = json.loads((DIRECTORY / "scalar-logic-sources.json").read_text())
        reports = source["reports"]
        numbers = [row["number"] for row in reports]
        self.assertEqual(len(numbers), 50)
        self.assertEqual(len(set(numbers)), len(numbers))
        ledger = json.loads((DIRECTORY / "coverage.json").read_text())["cases"]
        cohort = [row for row in ledger if row.get("cohort") == "scalar-logic"]
        self.assertEqual({row["number"] for row in cohort}, set(numbers))
        self.assertEqual(len(cohort), len(numbers))
        with (DIRECTORY / "inventory.csv").open(newline="") as stream:
            index = {int(row["number"]): row for row in csv.DictReader(stream)}
        for row in reports:
            with self.subTest(number=row["number"]):
                self.assertEqual(row["url"], f'https://github.com/llvm/llvm-project/issues/{row["number"]}')
                current = next(case for case in ledger if case["number"] == row["number"])
                self.assertEqual(index[row["number"]]["review_status"], current["status"])
                if current["status"] != row["disposition"]:
                    self.assertTrue(current.get("followups"), "historical dispositions require an explicit follow-up")
                self.assertIn(row["disposition"], ("covered-limited", "reviewed-deferred"))
                self.assertTrue(row["scope"])
                self.assertGreaterEqual(row["comment_count"], 0)
                for field in ("body_sha256", "comments_sha256", "issue_json_sha256"):
                    self.assertRegex(row[field], r"^[0-9a-f]{64}$")
        implemented = {row["number"] for row in reports if row["disposition"] == "covered-limited"}
        self.assertEqual(implemented, {44992, 45820, 55739, 56124, 58313, 61120, 83699})

    def test_native_forms_are_twenty_five_named_contracts_at_two_widths(self):
        fixture = (ROOT / "tests/regression/llvm_scalar_logic.c").read_text()
        names = re.findall(r"uint(?:32|64)_t logic_(\w+)_(32|64)\(", fixture)
        self.assertEqual(len(names), 50)
        self.assertEqual(len(set(names)), len(names))
        self.assertEqual({name for name, width in names if width == "32"},
                         {name for name, width in names if width == "64"})
        self.assertEqual(len({name for name, _ in names}), 25)
        for name, width in names:
            self.assertIn(f"reference_{name}_{width}(", fixture)


if __name__ == "__main__":
    unittest.main()
