import gzip
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('verify_evidence', ROOT/'scripts/verify_evidence_hashes.py')
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)


class EvidenceIntegrity(unittest.TestCase):
    def test_real_inventory(self):
        self.assertGreater(m.verify(ROOT, ROOT/m.DEFAULT), 0)

    def test_mutations_fail_closed(self):
        for mutation in ('bytes', 'missing', 'duplicate', 'raw', 'empty', 'escape', 'symlink'):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                path = root/'e.gz'
                raw = gzip.compress(b'original bytes', mtime=0)
                path.write_bytes(raw)
                row = {'path':'e.gz','bytes':len(raw),'sha256':hashlib.sha256(raw).hexdigest(),
                       'raw_sha256':hashlib.sha256(b'original bytes').hexdigest()}
                data = {'files':[row], 'raw_log_manifests':[]}
                if mutation == 'bytes': path.write_bytes(raw+b'X')
                elif mutation == 'missing': path.unlink()
                elif mutation == 'duplicate': data['files'].append(dict(row))
                elif mutation == 'raw': row['raw_sha256']='0'*64
                elif mutation == 'empty': data['files']=[]
                elif mutation == 'escape': row['path']='../e.gz'
                elif mutation == 'symlink':
                    real=root/'real.gz'; path.rename(real); path.symlink_to(real)
                manifest=root/'manifest.json'; manifest.write_text(json.dumps(data))
                with self.assertRaises((ValueError,OSError)):
                    m.verify(root, manifest)
