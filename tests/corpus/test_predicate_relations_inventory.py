"""Every selected report and old deferral remains individually accountable."""
import csv
import json
import importlib.util
import sys
from pathlib import Path
import re
import unittest
ROOT = Path(__file__).resolve().parents[2]
DIRECTORY = ROOT / "engineering/llvm-missed"


EXPECTED_NEW = {
    33576, 45821, 51748, 57635, 58589, 59266, 59393, 60167,
    61365, 63321, 63472, 63756, 64583, 64884, 66101, 69123,
    71792, 72651, 73904, 75004, 75784, 79266, 81288, 82350,
    83225, 85833, 86176, 88107, 88481, 95994, 96765, 97044,
    98800, 102355, 104772, 110919, 118108, 167173, 172888, 174937,
    180492, 183584, 186509, 186558, 192094, 202066, 202576, 205005,
    223249, 232620,
}
EXPECTED_PRIOR_DEFERRALS = {
    1884, 3007, 6509, 10156, 13134, 13687, 35970, 38139,
    38926, 41333, 41757, 41801, 41908, 42219, 45243, 45478,
    46405, 47868, 47946, 49562, 51090, 54856, 55016, 55468,
    57666, 58624, 59555, 59773, 61671, 62514, 62898, 63749,
    63848, 64238, 64859, 69333, 73417, 78038, 79696, 79823,
    80100, 82075, 84038, 84656, 84763, 85823, 85939, 86874,
    96455, 96690, 98848, 102946, 108559, 150954, 213877, 219334,
}


class PredicateRelationsInventory(unittest.TestCase):
    def test_exact_debian_gate_executes_the_fixture_and_retains_its_receipt(self):
        workflow = (ROOT / ".github/workflows/debian-oracle.yml").read_text()
        self.assertIn("python3 tests/regression/check_llvm_predicate_relations.py --gcc gcc-16 --json debian-predicate-relations.json", workflow)
        self.assertNotIn("--allow-gcc16-snapshot", workflow)
        self.assertIn("gcc-16-multilib", workflow)
        self.assertIn("libc6-dev-i386", workflow)
        self.assertIn("assert data['gcc_16_2_normative']", workflow)
        self.assertGreaterEqual(workflow.count("debian-predicate-relations.json"), 3)

    def test_exact_and_supplemental_gcc_oracles_are_distinguished(self):
        directory = ROOT / "tests/regression"
        sys.path.insert(0, str(directory))
        try:
            spec = importlib.util.spec_from_file_location("predicate_relations_checker", directory / "check_llvm_predicate_relations.py")
            checker = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(checker)
        finally:
            sys.path.pop(0)
        for version in ("16.2", "16.2.0"):
            self.assertTrue(checker.gcc_oracle_identity(version))
        for version in ("16.0.1", "16.1.0"):
            with self.assertRaises(ValueError):
                checker.gcc_oracle_identity(version)
            self.assertFalse(checker.gcc_oracle_identity(version, allow_snapshot=True))
        for version in ("15.2.0", "17.0.1", "unknown"):
            with self.assertRaises(ValueError):
                checker.gcc_oracle_identity(version, allow_snapshot=True)

    def test_identity_sets_followups_and_current_dispositions(self):
        source = json.loads((DIRECTORY / "predicate-relations-sources.json").read_text())
        reports = source["reports"]
        new = {r["number"] for r in reports if r["review_kind"] == "new"}
        old = {r["number"] for r in reports if r["review_kind"] == "prior-deferral"}
        self.assertEqual(new, EXPECTED_NEW)
        self.assertEqual(old, EXPECTED_PRIOR_DEFERRALS)
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
                self.assertEqual(ledger[number]["status"], index[number]["review_status"])
                if row["disposition"] != ledger[number]["status"]:
                    self.assertTrue(any(f["previous_status"] == row["disposition"] and f["status"] == ledger[number]["status"] for f in ledger[number].get("followups", [])), "historical cohort requires a distinct follow-up")
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
