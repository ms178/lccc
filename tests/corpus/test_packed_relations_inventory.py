"""Keep four scoped follow-ups distinct from historical audit dispositions."""
import json
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[2]
BASE = ROOT / 'engineering/llvm-missed'


class PackedRelationsInventory(unittest.TestCase):
    def test_scoped_provenance_and_followups(self):
        manifest = json.loads((BASE / 'packed-relations-sources.json').read_text())
        reports = {row['number']: row for row in manifest['reports']}
        self.assertEqual(set(reports), {192094, 219334, 202576, 205005})
        ledger = json.loads((BASE / 'coverage.json').read_text())
        cases = {row['number']: row for row in ledger['cases']}
        for number, report in reports.items():
            with self.subTest(number=number):
                self.assertEqual(report['comment_count'], report['reported_comment_count'])
                for field in ('body_sha256', 'issue_json_sha256', 'comments_sha256'):
                    self.assertRegex(report[field], r'^[0-9a-f]{64}$')
                case = cases[number]
                followup = [item for item in case['followups'] if item['cohort'] == 'packed-relations']
                self.assertEqual(len(followup), 1)
                self.assertNotEqual(followup[0]['previous_status'], case['status'])
                self.assertEqual(case['status'], report['disposition'])
                self.assertEqual(case['scope'], report['scope'])
                for path in followup[0]['evidence']:
                    self.assertTrue((ROOT / path).is_file(), path)
        self.assertIn('does not implement', reports[205005]['scope'])


if __name__ == '__main__':
    unittest.main()
