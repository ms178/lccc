#!/usr/bin/env python3
"""PR831 replay: reuse the canonical runner, preserve named compiler arms and each completed case.
Run only after compiler builds, CI and provisioning have finished. This is VM timing, not a physical-CPU claim.
"""
import argparse
import dataclasses
import json
import math
from pathlib import Path
import statistics
import subprocess
import sys
import tempfile

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--repo', type=Path, required=True)
p.add_argument('--candidate', required=True)
p.add_argument('--reference', required=True)
p.add_argument('--reference-key', default='pr831')
p.add_argument('--reference-label', default='PR831 control')
p.add_argument('--cflag', action='append', default=[])
p.add_argument('--gcc', default='/usr/bin/gcc-16')
p.add_argument('--clang')
p.add_argument('--only', help='Comma-separated canonical registry IDs; default all')
p.add_argument('--reps', type=int, default=9)
p.add_argument('--out', type=Path, required=True)
p.add_argument('--artifacts', type=Path, help='Keep the actual measured ELFs, not a later reconstruction')
a = p.parse_args()
sys.path.insert(0, str(a.repo.resolve()))
from tests.benchmark import run_benchmarks as b
from tools.corpus import publication
selected = set(a.only.split(',')) if a.only else {v.name for v in b.BENCHMARKS}
if not selected or not selected <= {v.name for v in b.BENCHMARKS}:
    p.error('empty or unknown registry selection')
if a.reps < 1 or a.reference_key in ('lccc', 'gcc', 'clang'):
    p.error('invalid repetitions or reference key')
include = subprocess.check_output([a.gcc, '-print-file-name=include'], text=True).strip()
if not Path(include).is_dir():
    p.error('missing GCC builtin headers')
flags = ('-O2', '-march=x86-64-v3', '-fwrapv', '-Wl,--no-as-needed', '-lm', '-I'+include, *a.cflag)
compilers = [b.CompilerSpec('lccc', 'Candidate', a.candidate, flags),
             b.CompilerSpec(a.reference_key, a.reference_label, a.reference, flags),
             b.CompilerSpec('gcc', 'GCC reference', a.gcc, flags)]
if a.clang:
    compilers.append(b.CompilerSpec('clang', 'Clang reference', a.clang, flags))
prefix, pin, warnings = b.choose_affinity('auto')
if not pin['applied']:
    p.error('CPU affinity unavailable: '+str(pin))
work_root, work_metadata = b.select_work_root(str(a.repo / 'target'))
args = argparse.Namespace(skip_perf=False, reps=a.reps, warmup=2)
report = {'complete': False, 'metric': 'paired native wall time, VM proxy',
          'selected': sorted(selected), 'reps': a.reps, 'warmups': 2,
          'metadata': b.collect_environment(pin, work_metadata, b.probe_pmu(True), compilers),
          'compilers': [dict(dataclasses.asdict(c), binary_sha256=b.sha256_file(Path(c.executable))) for c in compilers],
          'benchmarks': [], 'failures': [], 'warnings': warnings,
          'policy': 'canonical default sizes and per-workload flags; same flags for every arm; seeded randomized paired order; warmups excluded; no outlier removal'}
a.out.parent.mkdir(parents=True, exist_ok=True)
def save():
    publication.atomic_bytes(a.out, (json.dumps(report, indent=2)+'\n').encode())
save()
with tempfile.TemporaryDirectory(dir=work_root, prefix='pr831-native-') as temp:
    for index, item in enumerate(b.BENCHMARKS):
        if item.name not in selected:
            continue
        print(item.name, flush=True)
        entry = b.run_benchmark(item, compilers, args, Path(temp), a.artifacts,
                                prefix, b.controlled_environment(), 20261015 + index*100003)
        report['benchmarks'].append(entry)
        for c in compilers:
            runs = entry['runs'].get(c.key, [])
            if (not entry['compile'].get(c.key, {}).get('ok') or len(runs) != a.reps
                or any(not r['ok'] for r in runs)
                or entry['correctness'].get(c.key, {}).get('matches_baseline') is not True):
                report['failures'].append(item.name+'/'+c.key)
        save()
report['aggregates'] = {}
for key in (a.reference_key, 'gcc', 'clang'):
    ratios = {e['name']: e['ratios_from_lccc'][key]['median'] for e in report['benchmarks']
              if e.get('ratios_from_lccc', {}).get(key)
              and all(e['correctness'].get(k, {}).get('matches_baseline') is True for k in ('lccc', key))}
    if ratios:
        report['aggregates'][key] = {'n': len(ratios), 'gm': math.exp(statistics.fmean(map(math.log, ratios.values()))),
                                    'am': statistics.fmean(ratios.values()),
                                    'worst_first': sorted(ratios.items(), key=lambda x: x[1], reverse=True)}
report['complete'] = True
save()
print(json.dumps(report['aggregates'], indent=2))
sys.exit(bool(report['failures']))
