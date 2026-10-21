"""Every selected report and old deferral remains individually accountable."""
import csv
import json
from pathlib import Path
import re
import unittest
ROOT = Path(__file__).resolve().parents[2]
DIRECTORY = ROOT / "engineering/llvm-missed"


class PredicateRelationsInventory(unittest.TestCase):
    def test_identity_sets_followups_and_current_dispositions(self):
        source = json.loads((DIRECTORY / "predicate-relations-sources.json").read_text())
        reports = source["reports"]
        new = {r["number"] for r in reports if r["review_kind"] == "new"}
        old = {r["number"] for r in reports if r["review_kind"] == "prior-deferral"}
        self.assertEqual(len(new), 50)
        self.assertEqual(len(old), 56)
        self.assertFalse(new & old)
        self.assertEqual(len(reports), 106)
        ledger = {r["number"]: r for r in json.loads((DIRECTORY / "coverage.json").read_text())["cases"]}
        with (DIRECTORY / "inventory.csv").open(newline="") as stream:
            index = {int(r["number"]): r for r in csv.DictReader(stream)}
        doc = (DIRECTORY / "PREDICATE-RELATIONS.md").read_text()
        for row in reports:
            with self.subTest(number=row["number"]):
                number = row["number"]
                self.assertEqual(row["disposition"], ledger[number]["status"])
                self.assertEqual(row["disposition"], index[number]["review_status"])
                self.assertIn(f"[{number}]({row['url']})", doc)
                self.assertTrue(row["scope"])
                for field in ("body_sha256", "comments_sha256", "issue_json_sha256"):
                    self.assertRegex(row[field], r"^[0-9a-f]{64}$")
                if number in old:
                    followup = next(f for f in ledger[number]["followups"] if f["cohort"] == "predicate-relations")
                    self.assertEqual(followup["previous_status"], "reviewed-deferred")
                    self.assertEqual(followup["scope"], row["scope"])
        self.assertEqual(sum(r["disposition"] == "covered-limited" for r in reports), 15)

    def test_sixty_two_native_forms_and_production_refusal_counterexamples(self):
        fixture = (ROOT / "tests/regression/llvm_predicate_relations.c").read_text()
        names = re.findall(r"uint(?:32|64)_t predicate_(\w+)_(32|64)\(", fixture)
        self.assertEqual(len(names), 62)
        self.assertEqual(len(set(names)), 62)
        self.assertEqual({n for n, w in names if w == "32"}, {n for n, w in names if w == "64"})
        # The select-bswap proposal cannot drop an independent false condition.
        self.assertNotEqual(0, int.from_bytes((1).to_bytes(4, "little"), "big"))
        # No assumed nuw: 15-x differs from XOR for an unbounded x.
        self.assertNotEqual((15 - 16) & 255, 16 ^ 15)
        # No exact division: unequal values have equal quotients.
        self.assertEqual(4 // 3, 5 // 3)
        self.assertNotEqual(4, 5)
        # Nonzero sparse masks do not supply a particular known-one bit.
        self.assertNotEqual(2 & 3, 0)
        self.assertEqual(2 & 1, 0)
        # Word truthiness and bitwise masks are not canonical Booleans.
        self.assertTrue(bool(2))
        self.assertNotEqual(2, 1)
        self.assertNotEqual(1 ^ 1, 1 | 1)
        # Moving XOR through OR without disjointness changes the result.
        self.assertNotEqual((1 | 1) ^ 1, (1 ^ 1) | 1)
        # Total byte CLZ differs from its nonzero contract at zero.
        total_clz = lambda value: 8 - value.bit_length()
        self.assertNotEqual(total_clz(0) ^ 7, 7 - total_clz(0))
        for n, width in names:
            self.assertIn(f"reference_{n}_{width}(", fixture)


if __name__ == "__main__":
    unittest.main()
