"""Hermetic negative controls for evidence integrity and ordering censuses."""
import gzip
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts'))
import verify_evidence_hashes as evidence
import check_licm_ordering as ordering
import expand_asm_fixture as fixture


class EvidenceTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        (self.root / 'run.gz').write_bytes(gzip.compress(b'measured\n', mtime=0))
        (self.root / 'RAW-LOGS.json').write_text(json.dumps([{
            'gzip': 'run.gz', 'raw_sha256': hashlib.sha256(b'measured\n').hexdigest()}]))
        self.manifest = self.root / 'inventory.json'
        self.data = {'schema': 1, 'artifacts': [], 'raw_log_manifests': ['RAW-LOGS.json']}
        self.refresh()

    def refresh(self):
        self.data['artifacts'] = [{'path': name, 'bytes': (self.root / name).stat().st_size,
                                  'sha256': hashlib.sha256((self.root / name).read_bytes()).hexdigest()}
                                 for name in ('run.gz', 'RAW-LOGS.json')]
        self.save()

    def save(self):
        self.manifest.write_text(json.dumps(self.data))

    def verify(self):
        return evidence.verify(self.root, self.manifest)

    def test_positive(self):
        self.assertEqual(self.verify(), {'artifacts': 2, 'decompressed_raw_logs': 1, 'gzip_streams': 1, 'canonical_aliases': 0})

    def test_malformed_inventory_types(self):
        for value in [[], None, 1, "inventory", {'schema': True},
                      {'schema': 1, 'artifacts': {}},
                      {'schema': 1, 'artifacts': [None]}]:
            self.manifest.write_text(json.dumps(value))
            with self.subTest(value=value), self.assertRaises(ValueError): self.verify()

    def test_duplicate_raw_manifest(self):
        self.data['raw_log_manifests'] *= 2
        self.save()
        with self.assertRaisesRegex(ValueError, 'duplicate'): self.verify()

    def test_gzip_crc_without_raw_manifest(self):
        damaged = bytearray((self.root / 'run.gz').read_bytes())
        damaged[-8] ^= 1
        (self.root / 'run.gz').write_bytes(damaged)
        self.data['raw_log_manifests'] = []
        self.refresh()
        with self.assertRaises((OSError, ValueError)): self.verify()

    def test_compressed_corruption(self):
        (self.root / 'run.gz').write_bytes(b'broken')
        with self.assertRaises(ValueError): self.verify()

    def test_rehashed_wrapper_cannot_hide_raw_corruption(self):
        (self.root / 'run.gz').write_bytes(gzip.compress(b'forged\n', mtime=0))
        self.refresh()
        with self.assertRaisesRegex(ValueError, 'decompressed'): self.verify()

    def test_empty_inventory(self):
        self.data['artifacts'] = []
        self.save()
        with self.assertRaises(ValueError): self.verify()

    def test_duplicate_artifact(self):
        self.data['artifacts'].append(self.data['artifacts'][0])
        self.save()
        with self.assertRaises(ValueError): self.verify()

    def test_duplicate_json_key(self):
        self.manifest.write_text('{"schema":1,"schema":1}')
        with self.assertRaises(ValueError): self.verify()

    def test_missing_file(self):
        (self.root / 'run.gz').unlink()
        with self.assertRaises(OSError): self.verify()

    def test_size_mismatch(self):
        self.data['artifacts'][0]['bytes'] += 1
        self.save()
        with self.assertRaises(ValueError): self.verify()

    def test_bad_hash(self):
        self.data['artifacts'][0]['sha256'] = 'invalid'
        self.save()
        with self.assertRaises(ValueError): self.verify()

    def test_traversal_and_absolute_path(self):
        for name in ('../run.gz', '/run.gz'):
            with self.subTest(name=name), self.assertRaises(ValueError):
                evidence.contained(self.root, name)

    def test_raw_manifest_must_be_authenticated(self):
        self.data['artifacts'] = self.data['artifacts'][:1]
        self.save()
        with self.assertRaisesRegex(ValueError, 'not authenticated'): self.verify()


class OrderingCensusTests(unittest.TestCase):
    def assembly(self, inside=True):
        reads = '\n'.join(f' movl ordering_g{i}(%rip), %eax' for i in range(4))
        return 'probe:\n' + ('.loop:\n' + reads if inside else reads + '\n.loop:') + '\n jne .loop\n ret\n.size probe, .-probe\n'

    def test_inside_and_hoisted(self):
        self.assertEqual(ordering.census(self.assembly(), 'probe')['inside_backedges'], 4)
        self.assertEqual(ordering.census(self.assembly(False), 'probe')['inside_backedges'], 0)

    def test_missing_function_is_not_zero_reads(self):
        with self.assertRaises(ValueError): ordering.census(self.assembly(), 'absent')

    def test_missing_symbol_and_lea_are_not_loads(self):
        for old, new in [('ordering_g0', 'other'), ('movl ordering_g0', 'leaq ordering_g0')]:
            with self.subTest(new=new), self.assertRaises(ValueError):
                ordering.census(self.assembly().replace(old, new), 'probe')

    def test_comments_do_not_add_reads(self):
        self.assertEqual(ordering.census(self.assembly().replace(' ret', ' # movl ordering_g0(%rip), %eax\n ret'), 'probe')['inside_backedges'], 4)


class FixtureExpansionTests(unittest.TestCase):
    def test_both_branches_and_substitution(self):
        source = r""".macro PAIR name, keep
\name:
.if \keep
movl $1, %eax
.else
xorl %eax, %eax
.endif
ret
.endm
PAIR kept, 1
PAIR removed, 0
"""
        self.assertEqual(fixture.expand(source),
                         'kept:\nmovl $1, %eax\nret\nremoved:\nxorl %eax, %eax\nret\n')

    def test_rejects_unbalanced_and_unknown_substitutions(self):
        for source in ['.else', '.endif', '.if 1', '.macro A name',
                       '.macro A name\n\\unknown:\n.endm\nA x',
                       '.macro A name\nret\n.endm\nA x, y']:
            with self.subTest(source=source), self.assertRaises(ValueError):
                fixture.expand(source)

    def test_parameter_tokens_not_prefixes(self):
        self.assertEqual(fixture.expand('.macro A x, xy\n\\x \\xy\n.endm\nA first, second'),
                         'first second\n')

    def test_duplicate_else_rejected(self):
        with self.assertRaises(ValueError):
            fixture.expand('.macro A x\n.if 1\n.else\n.else\n.endif\n.endm\nA value')

    def test_all_implicit_functions_are_exposed(self):
        source = (ROOT / 'tests/regression/fixtures/peephole-vex-flags/implicit-witness.s').read_text()
        expanded = fixture.expand(source)
        self.assertEqual(expanded.count('.type '), 18)
        self.assertNotIn('.if ', expanded)
        self.assertNotIn('\\', expanded)


if __name__ == '__main__':
    unittest.main()
