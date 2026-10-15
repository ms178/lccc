import copy,sys,unittest
from pathlib import Path
ROOT=Path('/home/user/lccc');sys.path.insert(0,str(ROOT/'tests/corpus'))
from test_compiler_comparison_reports import summary

def fixture():
 arms=['lccc','gcc','prior'];rows=[]
 for case in summary.bench.BENCHMARKS:
  runs={a:[{'ok':True,'stdout':'correct\n','wall_s':.2 if a=='lccc' else .1} for _ in range(9)] for a in arms}
  rows.append({'name':case.name,'correctness':{a:{'matches_baseline':True,'stable_within_compiler':True} for a in arms},'runs':runs,'rounds':[{'round':i,'order':arms[:],'runs':{a:runs[a][i] for a in arms}} for i in range(9)],'runtime':{a:{'wall':{'median':.2 if a=='lccc' else .1}} for a in arms},'ratios_from_lccc':{a:{'n':9,'median':2.0} for a in ['gcc','prior']}})
 return {'complete':True,'benchmarks':rows,'compilers':[{'key':a} for a in arms],'settings':{'skip_perf':False}}

def admitted(data):return summary.summarize(data)['rows'][0]['ratios']['prior']['eligible']
class TimingProvenance(unittest.TestCase):
 def test_valid_raw_pairs_still_admitted(self):self.assertTrue(admitted(fixture()))
 def test_cached_ratio_cannot_invent_speedup(self):
  d=fixture();d['benchmarks'][0]['ratios_from_lccc']['prior']['median']=.01;self.assertFalse(admitted(d))
 def test_cached_floor_cannot_admit_short_raw_samples(self):
  d=fixture()
  for run in d['benchmarks'][0]['runs']['lccc']:run['wall_s']=.001
  self.assertFalse(admitted(d))
 def test_missing_duplicate_or_mismatched_pairs_are_not_evidence(self):
  for mode in ['missing','duplicate','mismatch','reorder','fake_order']:
   with self.subTest(mode=mode):
    d=fixture();r=d['benchmarks'][0]
    if mode=='missing':r['rounds']=[]
    elif mode=='duplicate':r['rounds'][1]['round']=0
    elif mode=='mismatch':r['rounds'][0]['runs']['prior']={**r['runs']['prior'][0],'wall_s':5}
    elif mode=='reorder':r['rounds'].reverse()
    else:r['rounds'][0]['order']=['lccc','gcc','gcc']
    self.assertFalse(admitted(d))
 def test_invalid_raw_timing_never_enters_aggregate(self):
  for value in [0,-1,float('nan'),float('inf'),None,True,'0.1']:
   with self.subTest(value=value):
    d=fixture();d['benchmarks'][0]['runs']['lccc'][0]['wall_s']=value;self.assertFalse(admitted(d))
 def test_incomplete_reports_cannot_be_certified(self):
  d=fixture();d['complete']=False
  with self.assertRaises(ValueError):summary.summarize(d)
if __name__=='__main__':unittest.main()
