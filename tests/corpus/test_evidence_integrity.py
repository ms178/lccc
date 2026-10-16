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
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('verify_evidence', ROOT/'scripts/verify_evidence_hashes.py')
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)


class EvidenceIntegrity(unittest.TestCase):
    def test_real_inventory(self):
        self.assertGreater(m.verify(ROOT, ROOT/m.DEFAULT)["artifacts"], 0)

    def test_restored_capture_keeps_original_bytes_and_closed_scope(self):
        # Original PR832 capture at 63e783b, not a hash refreshed from the
        # rewritten current review document. Verified against that Git blob.
        retained = 'engineering/PR831-AUDIT.md'
        expected = 'e3af0932bb02500031d3289d6aaff7979a3e389bdffa27e6baf0571811961956'
        self.assertEqual(hashlib.sha256((ROOT/retained).read_bytes()).hexdigest(), expected)
        data = json.loads((ROOT/m.DEFAULT).read_text())
        row = next(row for row in data['files'] if row['path'] == retained)
        self.assertEqual(row['sha256'], expected)
        scope = json.loads((ROOT/m.SCOPE).read_text())
        self.assertIn(retained, scope['paths'])
        self.assertNotIn('engineering/PR831-COMPILER-AUDIT.md', scope['paths'])
        self.assertEqual(m.verify(ROOT, ROOT/m.DEFAULT)["artifacts"], 69)

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

    def test_default_cli_checks_all_inventories_and_propagates_followup_failure(self):
        for failure in (False, True):
            outcomes = [{"artifacts": 69}, {"artifacts": 61}, ValueError("followup corruption") if failure else {"artifacts": 130}]
            with self.subTest(failure=failure), patch.object(m, 'contained', side_effect=lambda root, name: root/name), \
                    patch.object(m, 'verify', side_effect=outcomes) as verify, redirect_stdout(io.StringIO()):
                self.assertEqual(m.main([]), int(failure))
                self.assertEqual([call.args[1] for call in verify.call_args_list],
                                 [ROOT/m.DEFAULT, ROOT/m.ADDITIONAL, ROOT/m.FOLLOWUPS])

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
            (root/'RAW.json').write_text(json.dumps([{'original': 'e', 'gzip': 'nested/e.gz',
                'raw_sha256': hashlib.sha256(b'valid').hexdigest()}]))
            rows = [{'path': name, 'bytes': (root/name).stat().st_size,
                     'sha256': hashlib.sha256((root/name).read_bytes()).hexdigest()}
                    for name in ('RAW.json', 'nested/e.gz')]
            rows[1]['raw_sha256'] = hashlib.sha256(b'valid').hexdigest()
            scope = root/m.SCOPE
            scope.parent.mkdir(parents=True)
            scope.write_text(json.dumps({'schema_version': 1, 'description': 'fixture',
                                        'paths': ['RAW.json', 'nested/e.gz']}))
            for envelope in ({'files': rows, 'schema_version': 1,
                              'capture_commit': 'a'*40, 'scope': 'fixture'},
                             {'schema': 1, 'artifacts': rows}):
                manifest = root/'manifest.json'
                manifest.write_text(json.dumps({**envelope, 'raw_log_manifests': ['RAW.json']}))
                with self.subTest(envelope=list(envelope)), self.assertRaisesRegex(ValueError, 'filename'):
                    m.verify(root, manifest)


    def test_followup_inventory_covers_real_scopes(self):
        result = m.verify(ROOT, ROOT/m.FOLLOWUPS)
        self.assertGreaterEqual(result['artifacts'], 112)
        self.assertEqual(result['canonical_aliases'], 7)

    def test_scope_and_alias_mutations_fail_closed(self):
        for mutation in ('unlisted', 'omitted', 'extra', 'missing_target', 'alias_hash',
                         'duplicate_alias', 'reintroduced', 'symlink_root', 'escape_root'):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp); folder = root/'data'; folder.mkdir()
                (folder/'a').write_bytes(b'original')
                row = {'path':'data/a','bytes':8,'sha256':hashlib.sha256(b'original').hexdigest()}
                alias = {'former_path':'data/old','canonical_path':'data/a','bytes':8,'sha256':row['sha256']}
                data = {'schema':1,'artifacts':[row], 'coverage_roots':['data'], 'canonical_aliases':[alias]}
                if mutation == 'unlisted': (folder/'new').write_bytes(b'new')
                elif mutation == 'omitted': data['artifacts']=[]
                elif mutation == 'extra': data['coverage_roots']=[]
                elif mutation == 'missing_target': alias['canonical_path']='missing'
                elif mutation == 'alias_hash': alias['sha256']='0'*64
                elif mutation == 'duplicate_alias': data['canonical_aliases'].append(dict(alias))
                elif mutation == 'reintroduced': (folder/'old').write_bytes(b'original')
                elif mutation == 'symlink_root': (root/'link').symlink_to(folder);data['coverage_roots']=['link']
                elif mutation == 'escape_root': data['coverage_roots']=['../escape']
                manifest=root/'manifest.json';manifest.write_text(json.dumps(data))
                with self.assertRaises((ValueError,OSError)):
                    m.verify(root,manifest)


class StrictEvidenceInventory(unittest.TestCase):
    def fixture(self,root,raw=b'original bytes'):
        compressed=gzip.compress(raw,mtime=0);(root/'e.gz').write_bytes(compressed)
        (root/'keep.txt').write_bytes(b'keep')
        rows=[{'path':'e.gz','bytes':len(compressed),'sha256':hashlib.sha256(compressed).hexdigest(),'raw_sha256':hashlib.sha256(raw).hexdigest()},
              {'path':'keep.txt','bytes':4,'sha256':hashlib.sha256(b'keep').hexdigest()}]
        data={'schema_version':1,'capture_commit':'a'*40,'scope':'fixture','raw_log_manifests':[],'files':rows}
        scope=root/'engineering/evidence/pr832-integrity.scope.json';scope.parent.mkdir(parents=True)
        scope.write_text(json.dumps({'schema_version':1,'description':'independent fixture inventory','paths':['e.gz','keep.txt']}))
        manifest=root/'manifest.json';manifest.write_text(json.dumps(data))
        return manifest,data

    def test_strict_schema_and_independent_completeness(self):
        for mutation in ('version','unknown-root','unknown-row','boolean-bytes','bad-sha','missing-schema','delete-file-and-row','duplicate-json-key'):
            with self.subTest(mutation=mutation),tempfile.TemporaryDirectory() as tmp:
                root=Path(tmp);manifest,data=self.fixture(root)
                if mutation=='version':data['schema_version']=2
                elif mutation=='unknown-root':data['invented']=True
                elif mutation=='unknown-row':data['files'][0]['invented']=True
                elif mutation=='boolean-bytes':data['files'][1]['bytes']=True;(root/'keep.txt').write_bytes(b'x');data['files'][1]['sha256']=hashlib.sha256(b'x').hexdigest()
                elif mutation=='bad-sha':data['files'][0]['raw_sha256']='x'*64
                elif mutation=='missing-schema':del data['schema_version']
                elif mutation=='delete-file-and-row':(root/'e.gz').unlink();data['files'].pop(0)
                manifest.write_text(json.dumps(data))
                if mutation=='duplicate-json-key':manifest.write_text(manifest.read_text().replace('"files":','"files":[],"files":',1))
                with self.assertRaises((ValueError,OSError)):m.verify(root,manifest)

    def test_symlink_ancestor_and_corrupt_gzip(self):
        for mutation in ('ancestor','corrupt'):
            with self.subTest(mutation=mutation),tempfile.TemporaryDirectory() as tmp:
                root=Path(tmp);manifest,data=self.fixture(root)
                if mutation=='ancestor':
                    (root/'real').mkdir();(root/'e.gz').rename(root/'real/e.gz');(root/'alias').symlink_to(root/'real',target_is_directory=True)
                    data['files'][0]['path']='alias/e.gz'
                    scope=root/'engineering/evidence/pr832-integrity.scope.json'
                    spec=json.loads(scope.read_text());spec['paths'][0]='alias/e.gz';scope.write_text(json.dumps(spec))
                else:
                    bad=(root/'e.gz').read_bytes()[:-4]+b'BAD!';(root/'e.gz').write_bytes(bad)
                    data['files'][0]['sha256']=hashlib.sha256(bad).hexdigest()
                manifest.write_text(json.dumps(data))
                with self.assertRaises((ValueError,OSError,EOFError)):m.verify(root,manifest)

    def test_streaming_and_decoded_cap(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);manifest,_=self.fixture(root,b'x'*(2*1024*1024+7))
            with mock.patch.object(gzip,'decompress',side_effect=AssertionError('whole-buffer decompression')),mock.patch.object(Path,'read_bytes',side_effect=AssertionError('whole-buffer data read')):
                self.assertEqual(m.verify(root,manifest)["artifacts"],2)
    def test_decoded_cap(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);manifest,_=self.fixture(root,b'x'*17)
            with mock.patch.object(m,'MAX_DECODED',16,create=True):
                with self.assertRaises(ValueError):m.verify(root,manifest)

    def test_raw_log_traversal_and_leaf_spelling(self):
        for leaf in ('../e.gz','/e.gz','./e.gz','a/../e.gz','..\\e.gz'):
            with self.subTest(leaf=leaf),tempfile.TemporaryDirectory() as tmp:
                root=Path(tmp);manifest,data=self.fixture(root)
                records=[{'original':'e','gzip':leaf,'raw_sha256':data['files'][0]['raw_sha256']}]
                content=json.dumps(records).encode();(root/'RAW.json').write_bytes(content)
                data['files'].append({'path':'RAW.json','bytes':len(content),'sha256':hashlib.sha256(content).hexdigest()});data['raw_log_manifests']=['RAW.json']
                scope=root/'engineering/evidence/pr832-integrity.scope.json';spec=json.loads(scope.read_text());spec['paths'].append('RAW.json');scope.write_text(json.dumps(spec))
                manifest.write_text(json.dumps(data))
                with self.assertRaises((ValueError,OSError)):m.verify(root,manifest)
