"""Current summary must not present an unfinished historical diary as current."""
import hashlib
from pathlib import Path
import unittest
ROOT=Path(__file__).resolve().parents[2]

class ReviewDocumentation(unittest.TestCase):
    def test_historical_log_is_preserved_and_summary_is_explicit(self):
        summary=(ROOT/'engineering/PR832-REVIEW-FOLLOWUP.md').read_text()
        self.assertIn('Historical',summary.splitlines()[0])
        self.assertNotIn('execution evidence to follow',summary)
        self.assertLess(len(summary.splitlines()),80)
        history=ROOT/'engineering/history/PR832-REVIEW-LOG.md'
        # Intentional history-integrity tripwire: update the pin together with
        # the reviewed source document, never in an unrelated cleanup.
        self.assertEqual(hashlib.sha256(history.read_bytes()).hexdigest(),'268b65ea7e29fa5c29f42efd976019afdc76e4a0c0022f2e4990635336d01aa5')
        self.assertTrue((ROOT/'engineering/pr834/README.md').is_file())
