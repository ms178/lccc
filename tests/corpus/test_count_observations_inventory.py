"""Audit receipts must not turn deferred count reports into solved coverage."""
import csv
import hashlib
import json
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[2]
BASE = ROOT / "engineering/llvm-missed"


class CountObservationsInventory(unittest.TestCase):
    def test_source_records_and_scoped_ledger(self):
        manifest = json.loads((BASE / "count-observations-sources.json").read_text())
        records = {r["number"]: r for r in manifest["records"]}
        self.assertEqual(set(records), {47250, 76810, 83772, 115751, 131444, 210673, 213877, 231124})
        cases = json.loads((BASE / "coverage.json").read_text())["cases"]
        self.assertEqual(len(cases), len({r["number"] for r in cases}))
        cases = {r["number"]: r for r in cases}
        with (BASE / "inventory.csv").open() as f:
            rows = {int(r["number"]): r for r in csv.DictReader(f)}
        for number, record in records.items():
            with self.subTest(number=number):
                self.assertEqual(record["reported_comments"], record["accessible_comments"])
                self.assertRegex(record["sha256"], r"^[0-9a-f]{64}$")
                self.assertEqual(record["url"], f"https://github.com/llvm/llvm-project/issues/{number}")
                case = cases[number]
                self.assertEqual(case["status"], "covered-limited" if number in {115751, 210673} else "reviewed-deferred")
                self.assertEqual(rows[number]["review_status"], case["status"])
                if number == 213877:
                    followups = [r for r in case["followups"] if r["cohort"] == "count-observations"]
                    self.assertEqual(len(followups), 1)
                    case = followups[0]
                for path in case["evidence"]:
                    self.assertTrue((ROOT / path).is_file(), path)
        snapshot = json.loads((BASE / "snapshot.json").read_text())
        for field, path in [("csv_sha256", "inventory.csv"), ("ledger_sha256", "coverage.json")]:
            self.assertEqual(snapshot[field], hashlib.sha256((BASE / path).read_bytes()).hexdigest())


if __name__ == "__main__":
    unittest.main()
