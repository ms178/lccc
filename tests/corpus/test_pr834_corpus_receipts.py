"""Real capture + post-decoding semantic mutations; no benchmark executions."""
import copy
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock

ROOT=Path(__file__).resolve().parents[2]
spec=importlib.util.spec_from_file_location('corpus_receipts',ROOT/'scripts/verify_corpus_receipts.py')
verifier=importlib.util.module_from_spec(spec)
spec.loader.exec_module(verifier)


class CorpusReceipts(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with mock.patch.object(verifier,'_verify_dataset',wraps=verifier._verify_dataset) as linked:
            cls.result=verifier.verify(ROOT,ROOT/verifier.DEFAULT,False)
            cls.decoded=linked.call_args.args

    def test_exact_complete_capture(self):
        self.assertEqual(self.result['program_pairs'],225)
        self.assertEqual(self.result['linked_process_receipts'],3600)
        self.assertEqual(self.result['program_Ir_range'],[1,1])
        self.assertEqual(self.result['ELF_equal_all_rounds'],44)
        self.assertFalse(self.result['compiler_inputs_checked'])
        self.assertFalse(self.result['compiler_profiles_included'])

    def reject(self,mutator):
        # Archive hashing is exercised by the real capture above. These mutations
        # test the semantic boundary AFTER decoding, including removal of both
        # a member and its inventory row (ordinary checksums alone permit that).
        args=list(copy.deepcopy(self.decoded))
        mutator(args)
        with self.assertRaises((ValueError,KeyError)):
            verifier._verify_dataset(*args)

    def test_reject_derived_metric_lie(self):
        self.reject(lambda args:args[0]['bounded-01/arith_loop/manifest.json']['results']['arith_loop']['metrics'].update(ir_ratio=0.5))

    def test_reject_uncovered_raw_path(self):
        def mutate(args):
            entry=args[0]['bounded-01/arith_loop/manifest.json']['results']['arith_loop']
            entry['native']['mine']['raw_outputs']['stdout']=args[5]['original_root']+'/../outside'
        self.reject(mutate)

    def test_reject_missing_sidecar_and_inventory_row(self):
        def mutate(args):
            name='default-05/zstd_count/bb/repeat-04/program.compile.json'
            del args[0][name]
            del args[3][name]
        self.reject(mutate)

    def test_reject_wrong_compiler_identity(self):
        self.reject(lambda args:args[0]['bounded-01/arith_loop/manifest.json']['tools']['mine'].update(sha256='0'*64))

    def test_reject_wrong_counter_geometry(self):
        self.reject(lambda args:args[2]['bounded-01/arith_loop/aa/arith_loop.cg'].update(LL='1024,1,64'))

    def test_reject_modified_archive_digest(self):
        config=json.loads((ROOT/verifier.DEFAULT).read_text());config['archive_sha256']='0'*64
        with tempfile.TemporaryDirectory() as tmp:
            path=Path(tmp)/'config.json';path.write_text(json.dumps(config))
            with self.assertRaisesRegex(ValueError,'archive digest mismatch'):
                verifier.verify(ROOT,path)

    def test_reject_unknown_schema_field(self):
        config=json.loads((ROOT/verifier.DEFAULT).read_text());config['unchecked']='not allowed'
        with tempfile.TemporaryDirectory() as tmp:
            path=Path(tmp)/'config.json';path.write_text(json.dumps(config))
            with self.assertRaisesRegex(ValueError,'invalid object keys'):
                verifier.verify(ROOT,path)


if __name__=='__main__':unittest.main()
