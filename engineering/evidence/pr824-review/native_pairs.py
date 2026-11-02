"""Run from repo root. Reuse the canonical paired-median benchmark engine."""
import argparse, json, math, statistics, sys
from pathlib import Path
sys.path.insert(0, '.')
from tests.benchmark import run_benchmarks as b
root=Path('/home/user/.cache/pr824-review/native-pairs');root.mkdir(parents=True,exist_ok=True)
selected=set(Path('/home/user/.cache/worst15').read_text().strip().split(','))
flags=('-O2','-march=x86-64-v3','-fwrapv','-I'+b.detect_gcc_include('/home/user/.cache/toolchains/gcc-16.2.0/bin/gcc'))
compilers=[b.CompilerSpec('baseline','Prior S36 LCCC','/home/user/.cache/pr824-baseline/lccc',flags),b.CompilerSpec('lccc','PR824 reviewed candidate',str(Path('target/fastbuild/lccc').resolve()),flags)]
prefix,pinning,warnings=b.choose_affinity('auto')
work,work_meta=b.select_work_root(str(root/'work'));work.mkdir(parents=True,exist_ok=True)
report={'complete':False,'class':'VM screening, not EPYC 7763 certification','flags':flags,'environment':b.collect_environment(pinning,work_meta,b.probe_pmu(True),compilers),'compiler_sha256':{c.key:b.sha256_file(Path(c.executable)) for c in compilers},'warnings':warnings,'entries':[]}
def save():
 p=root/'report.json';q=p.with_suffix('.tmp');q.write_text(json.dumps(report,indent=2)+'\n');q.replace(p)
save()
args=argparse.Namespace(skip_perf=False,warmup=1,reps=9)
for item in b.BENCHMARKS:
 if item.name not in selected:continue
 print(item.name,flush=True)
 row=b.run_benchmark(item,compilers,args,work,root/'artifacts',prefix,b.controlled_environment(),20261014+b.stable_seed(item.name)%1000000)
 report['entries'].append(row);save()
 for key in ('baseline','lccc'):
  assert row['compile'][key]['ok']
  assert len(row['runs'][key])==9 and all(r['ok'] for r in row['runs'][key])
  assert row['correctness'][key]['matches_baseline']
ratios=[e['ratios_from_lccc']['baseline']['median'] for e in report['entries']]
assert len(ratios)==15
report['aggregate']={'arithmetic_mean':statistics.mean(ratios),'geomean':math.exp(statistics.mean(map(math.log,ratios))),'best':min(ratios),'worst':max(ratios)}
report['complete']=True;save();print(report['aggregate'])
