import gzip
import hashlib
import importlib.util
import json
import io
from contextlib import redirect_stdout
from unittest.mock import patch
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('verify_evidence', ROOT/'scripts/verify_evidence_hashes.py')
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)


class EvidenceIntegrity(unittest.TestCase):
    def test_real_inventory(self):
        self.assertGreater(m.verify(ROOT, ROOT/m.DEFAULT)["artifacts"], 0)

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

    def test_default_cli_checks_both_inventories_and_propagates_second_failure(self):
        for failure in (False, True):
            outcomes = [{"artifacts": 69}, ValueError("secondary corruption") if failure else {"artifacts": 61}]
            with self.subTest(failure=failure), patch.object(m, 'contained', side_effect=lambda root, name: root/name), \
                    patch.object(m, 'verify', side_effect=outcomes) as verify, redirect_stdout(io.StringIO()):
                self.assertEqual(m.main([]), int(failure))
                self.assertEqual([call.args[1] for call in verify.call_args_list],
                                 [ROOT/m.DEFAULT, ROOT/m.ADDITIONAL])

    def test_ambiguous_legacy_envelope_is_not_silently_reinterpreted(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            manifest = root/'manifest.json'
            for extra in ({'schema': 1}, {'artifacts': []}):
                manifest.write_text(json.dumps({'files': [], **extra}))
                with self.subTest(extra=extra), self.assertRaisesRegex(ValueError, 'ambiguous'):
                    m.verify(root, manifest)

    def test_authenticated_raw_log_still_requires_a_leaf_filename(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root/'nested').mkdir()
            payload = gzip.compress(b'valid', mtime=0)
            (root/'nested/e.gz').write_bytes(payload)
            (root/'RAW.json').write_text(json.dumps([{'gzip': 'nested/e.gz',
                'raw_sha256': hashlib.sha256(b'valid').hexdigest()}]))
            rows = [{'path': name, 'bytes': (root/name).stat().st_size,
                     'sha256': hashlib.sha256((root/name).read_bytes()).hexdigest()}
                    for name in ('RAW.json', 'nested/e.gz')]
            for envelope in ({'files': rows}, {'schema': 1, 'artifacts': rows}):
                manifest = root/'manifest.json'
                manifest.write_text(json.dumps({**envelope, 'raw_log_manifests': ['RAW.json']}))
                with self.subTest(envelope=list(envelope)), self.assertRaisesRegex(ValueError, 'filename'):
                    m.verify(root, manifest)
