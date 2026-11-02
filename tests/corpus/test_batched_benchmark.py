"""The calibrated runner must reject real changes hidden by entry renaming."""
import importlib.util
from pathlib import Path
import unittest
import copy
import json
import tempfile
ROOT=Path(__file__).resolve().parents[2]
spec=importlib.util.spec_from_file_location('batched',ROOT/'scripts/benchmark_native_batched.py')
runner=importlib.util.module_from_spec(spec);spec.loader.exec_module(runner)

class CalibrationTests(unittest.TestCase):
    def test_fastest_sets_common_duration(self):
        self.assertEqual(runner.calibrated_count({'a': {'ns': 20000000, 'iterations': 2}, 'b': {'ns': 40000000, 'iterations': 2}}, .2, 3), 20)
    def test_slow_arm_caps_extreme_speed_difference(self):
        self.assertEqual(runner.calibrated_count({'a': {'ns': 20, 'iterations': 2}, 'b': {'ns': 200000000, 'iterations': 2}}, .2, 3), 30)
    def test_one_iteration_floor(self):
        self.assertEqual(runner.calibrated_count({'a': {'ns': 10000000000, 'iterations': 2}}, .2, 3), 1)


class ResumeTests(unittest.TestCase):
    def fixture(self):
        identity={'compilers': {'a': {'sha256': 'aaa'}, 'b': {'sha256': 'bbb'}}, 'flags': ['-O2']}
        row=dict(name='first', target_seconds=.2, max_batch_seconds=1., iterations=2,
                 samples=[dict(values={'a':dict(ns=20,iterations=2),'b':dict(ns=10,iterations=2)}) for _ in range(3)],
                 comparisons={'a':dict(paired_ratios=[2.,2.,2.],median=2.),
                              'b':dict(paired_ratios=[1.,1.,1.],median=1.)})
        old=dict(identity=copy.deepcopy(identity),rounds=3,pinning={'cpu':0},rows=[row])
        return old, identity
    def validate(self, old, identity, **changes):
        args=dict(old=old,identity=identity,names=['first','second'],reference='b',rounds=3,
                  seconds=.2,max_seconds=1.,pinning={'cpu':0})
        args.update(changes)
        return runner.validate_resume(**args)
    def test_exact_prefix_survives(self):
        old,identity=self.fixture()
        self.assertEqual(self.validate(old,identity),old['rows'])
    def test_changed_compiler_is_rejected(self):
        old,identity=self.fixture();identity['compilers']['a']['sha256']='new'
        with self.assertRaises(ValueError):self.validate(old,identity)
    def test_changed_flags_or_cpu_are_rejected(self):
        old,identity=self.fixture()
        with self.assertRaises(ValueError):self.validate(old,identity,pinning={'cpu':1})
        identity['flags']=['-O3']
        with self.assertRaises(ValueError):self.validate(old,identity)
    def test_reference_is_verified_from_raw_pairs(self):
        old,identity=self.fixture()
        with self.assertRaises(ValueError):self.validate(old,identity,reference='a')
    def test_incomplete_or_misordered_rows_are_rejected(self):
        old,identity=self.fixture();old['rows'][0]['samples'].pop()
        with self.assertRaises(ValueError):self.validate(old,identity)
        old,identity=self.fixture();old['rows'][0]['name']='second'
        with self.assertRaises(ValueError):self.validate(old,identity)
    def test_timing_policy_cannot_change(self):
        old,identity=self.fixture()
        with self.assertRaises(ValueError):self.validate(old,identity,seconds=.1)
        with self.assertRaises(ValueError):self.validate(old,identity,max_seconds=2.)
    def test_bad_counts_and_forged_medians_are_rejected(self):
        old,identity=self.fixture();old['rows'][0]['comparisons']['a']['median']=.5
        with self.assertRaises(ValueError):self.validate(old,identity)
        old,identity=self.fixture();old['rows'][0]['samples'][0]['values']['a']['ns']=True
        with self.assertRaises(ValueError):self.validate(old,identity)
        old,identity=self.fixture();old['rows'][0]['iterations']=0
        with self.assertRaises(ValueError):self.validate(old,identity)
    def test_atomic_report_publication(self):
        old,_=self.fixture()
        with tempfile.TemporaryDirectory() as directory:
            path=Path(directory)/'report.json';runner.publish(path,old)
            self.assertEqual(json.loads(path.read_text()),old)
            self.assertFalse(Path(str(path)+'.tmp').exists())

if __name__=='__main__':unittest.main()


class MeasuredObjectCommandTests(unittest.TestCase):
    def test_direct_mode_measures_the_production_compile(self):
        emit, build = runner.measured_object_commands(['cc', '-O2'], 'cc', 'w.c', 'w.s', 'w.o', 'direct')
        self.assertEqual(emit, ['cc', '-O2', '-S', 'w.c', '-o', 'w.s'])
        self.assertEqual(build, ['cc', '-O2', '-c', 'w.c', '-o', 'w.o'])

    def test_assembly_mode_keeps_the_historical_round_trip(self):
        emit, build = runner.measured_object_commands(['cc', '-O2'], 'cc', 'w.c', 'w.s', 'w.o', 'assembly')
        self.assertEqual(emit[2:4], ['-S', 'w.c'])
        self.assertEqual(build, ['cc', '-c', 'w.s', '-o', 'w.o'])

    def test_unknown_mode_is_rejected(self):
        with self.assertRaises(ValueError):
            runner.measured_object_commands([], 'cc', 'w.c', 'w.s', 'w.o', 'objcopy')
