"""Renaming historical measurements must not silently rewrite their bytes."""
import hashlib
import json
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[2]
EVIDENCE = ROOT / 'engineering/evidence'
# Anchors checked against the reviewed commit before the filenames changed.
ANCHORS = {
    'perf14-memory-dataflow-2026-10-11.json':
        '1a47c5469f0868e643c086f8ce5d14a343adf15f5f7ed2b888e106e2812bc23f',
    'perf14-vector-address-2026-10-11.json':
        'be3c93ab7aefba83281783bf1d5a0429f3a2539b7c43c86f1e9d52dc8e5106dc',
}


class RetainedEvidenceTests(unittest.TestCase):
    def test_manifest_keeps_the_independently_recorded_anchors(self):
        manifest = json.loads((EVIDENCE / 'retained-evidence-sha256.json').read_text())
        self.assertEqual(manifest, ANCHORS)

    def test_historical_payloads_are_unchanged(self):
        for name, digest in ANCHORS.items():
            with self.subTest(name=name):
                self.assertEqual(hashlib.sha256((EVIDENCE / name).read_bytes()).hexdigest(), digest)


if __name__ == '__main__':
    unittest.main()
