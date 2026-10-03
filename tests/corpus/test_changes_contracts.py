#!/usr/bin/env python3
"""Compiler-free generated Changes integrity/drift and parser contracts."""
import importlib.util
from pathlib import Path
import shutil
import sys
import tempfile
import unittest

REPO=Path(__file__).resolve().parents[2];sys.path.insert(0,str(REPO))
spec=importlib.util.spec_from_file_location('changes_contract',REPO/'scripts/edg_changes_mine.py')
changes=importlib.util.module_from_spec(spec);spec.loader.exec_module(changes)
from tools.corpus import schema


class ChangesContract(unittest.TestCase):
    def copied(self,td):
        p=Path(td)
        for name in changes.ARTIFACTS:shutil.copy2(REPO/'docs'/name,p/name)
        return p
    def test_shipped_integrity_and_both_eras(self):
        s=changes.verify_artifacts(REPO/'docs');self.assertEqual(s['entries_total'],13562);self.assertEqual(s['entries_kept'],1567)
        self.assertEqual(s['parsed_eras'],dict(bracketed=7415,unbracketed=6147))
    def test_output_drift_detected(self):
        with tempfile.TemporaryDirectory() as td:
            p=self.copied(td);(p/'edg_changes_c_extract.md').write_text('changed')
            with self.assertRaises(ValueError):changes.verify_artifacts(p)
    def test_miner_identity_drift_detected(self):
        with tempfile.TemporaryDirectory() as td:
            p=self.copied(td);f=p/'edg_changes_stats.json';s=schema.loads(f.read_text());s['miner_sha256']='0'*64;f.write_text(schema.dumps(s))
            with self.assertRaises(ValueError):changes.verify_artifacts(p)
    def test_denominator_drift_detected(self):
        with tempfile.TemporaryDirectory() as td:
            p=self.copied(td);f=p/'edg_changes_stats.json';s=schema.loads(f.read_text());s['entries_kept']+=1;f.write_text(schema.dumps(s))
            with self.assertRaises(ValueError):changes.verify_artifacts(p)
    def test_raw_source_pin_required(self):
        with self.assertRaises(ValueError):changes.render_artifacts(b'unpinned source')
    def test_unknown_year_typo_not_guessed(self):self.assertEqual(changes.normalize_year('11/20/111'),'111')
    def test_keyword_scoring_live_not_classification(self):
        score,_=changes.score_entry(dict(title='_Generic _Complex _Static_assert',body=''))
        self.assertEqual(score,7)
    def test_wrapped_reference_does_not_become_entry(self):
        source='8/7/08   C fix\n\nbody\n\n4/14/17 for EDGcpfe/12\nmore\n\n5/4/92   Another fix\n\nend\n'
        entries=changes.parse_entries(source);self.assertEqual(len(entries),2);self.assertIn('for EDGcpfe/12',entries[0]['body'])

if __name__=='__main__':unittest.main()
