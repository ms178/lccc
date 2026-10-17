#!/usr/bin/env python3
"""Reproduce fresh PR832 comparisons; incomplete/incorrect runs are not ranks."""
import argparse
import csv
import gzip
import hashlib
import json
import math
from pathlib import Path
import statistics
import sys
ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'tests/benchmark'))
import run_benchmarks as bench
LABELS = dict(ccc='PR832', gcc='GCC16', clang='Clang23')

def load(path):
    return json.loads(path.read_text())

def geometric(values):
    return math.exp(statistics.fmean(math.log(x) for x in values))

def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--research', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--compiler', type=Path, required=True)
    args = ap.parse_args()
    if not __debug__:
        ap.error('assertions are required')
    original = load(args.research / 'native.json')
    refresh = load(args.research / 'native-dynamic.json')
    replay = load(args.research / 'native-replay.json')
    assert replay['complete']
    assert original['settings'] == refresh['settings']
    assert set(replay['changed']) == {b['name'] for b in refresh['benchmarks']}
    native = dict(original)
    replacements = {b['name']: b for b in refresh['benchmarks']}
    native['benchmarks'] = [replacements.get(b['name'], b) for b in original['benchmarks']]
    native['compilers'] = [c for c in refresh['compilers'] if c['key'] != 'lev']
    native['replay_proof'] = replay
    native['epochs'] = {b['name']: ('native-dynamic.json' if b['name'] in replacements else 'native.json')
                        for b in original['benchmarks']}
    checks = {c['name']: c for c in replay['checks']}
    for b in native['benchmarks']:
        assert b['compile']['lccc']['binary_sha256'] == checks[b['name']]['new_sha256']
        if b['name'] not in replacements: assert checks[b['name']]['identical']
    expected = {b.name for b in bench.BENCHMARKS}
    assert {b['name'] for b in native['benchmarks']} == expected
    compiler_hash = hashlib.sha256(args.compiler.read_bytes()).hexdigest()
    assert replay['compiler_sha256'] == compiler_hash
    rows = []
    for b in native['benchmarks']:
        bench.evaluate_correctness(b, ['lccc', *LABELS])
        assert all(b['correctness'][k]['matches_baseline'] is True for k in ['lccc', *LABELS]), b['name']
        hashes = {k: b['compile'][k]['binary_sha256'] for k in ['lccc', *LABELS]}
        rows.append(dict(name=b['name'], tags=b['tags'], hashes=hashes,
            comparisons={v: dict(median=b['ratios_from_lccc'][k]['median'],
                ci95_exploratory=b['ratios_from_lccc'][k]['median_ci95_bootstrap'],
                identical_binary=hashes[k] == hashes['lccc'],
                candidate_quality=b['runtime']['lccc']['quality_warnings'],
                reference_quality=b['runtime'][k]['quality_warnings']) for k,v in LABELS.items()}))
    summary = dict(compiler_sha256=compiler_hash, compilers=native['compilers'],
        settings=native['settings'], native_rows=rows, native_groups={}, callgrind={},
        interpretation='Native paired VM screening; identical ELFs cannot establish compiler wins. '
                       'Intervals exploratory, no multiplicity correction. Ir is simulation, not time.')
    for group, selected in [('all45',rows),('workload18',[r for r in rows if 'workload' in r['tags']]),
                            ('synthetic27',[r for r in rows if 'synthetic' in r['tags']])]:
        summary['native_groups'][group] = dict(n=len(selected),
            geometric_ratios={v:geometric(r['comparisons'][v]['median'] for r in selected) for v in LABELS.values()},
            identical={v:sum(r['comparisons'][v]['identical_binary'] for r in selected) for v in LABELS.values()})
    cg_replay = load(args.research / 'callgrind-replay.json')
    assert cg_replay['complete'] and cg_replay['compiler_sha256'] == compiler_hash
    cg_checks = {(r['arm'], r['name']): r['sha256'] for r in cg_replay['checks']}
    observed = set()
    for tag in ['main','gcc-16','clang-23']:
        d=load(args.research/('callgrind-'+tag)/'index.json')
        assert d['complete'] and not d['failures'], (tag,d['failures'])
        assert set(d['results']) == set(d['selected'])
        if tag=='main': assert set(d['results']) == expected
        for name in d['selected']:
            manifest = load(args.research / ('callgrind-' + tag) / name / 'manifest.json')
            item = next(b for b in bench.BENCHMARKS if b.name == name)
            record = manifest['results'][bench.source_for(item).stem]
            assert cg_checks[(tag, name)] == record['binary_sha256']['mine']
            observed.add((tag, name))
        summary['callgrind'][tag] = dict(n=len(d['results']),
            geometric_ir=geometric(r['metrics']['ir_ratio'] for r in d['results'].values()), rows=d['results'])
    assert observed == set(cg_checks)
    summary['callgrind_replay_proof'] = cg_replay
    original_dir=args.research/'codegen-final'
    reports={}
    for item in bench.BENCHMARKS:
        path=original_dir/item.name/'result.json'
        reports[item.name]=load(path)
    summary['codegen'] = dict(coverage={}, failures=[])
    for name,d in reports.items():
        for result in d:
            for record in result['records']:
                key=record['key']; summary['codegen']['coverage'][key]=summary['codegen']['coverage'].get(key,0)+1
            for error in result.get('errors',[]):summary['codegen']['failures'].append(dict(workload=name,**error))
    args.out.mkdir(parents=True,exist_ok=True)
    (args.out/'summary.json').write_text(json.dumps(summary,indent=2)+'\n')
    with (args.out/'native45.csv').open('w',newline='') as f:
        w=csv.writer(f);w.writerow(['workload',*LABELS.values(),'identical_to_PR832'])
        for r in rows:w.writerow([r['name'],*[r['comparisons'][v]['median'] for v in LABELS.values()],r['comparisons']['PR832']['identical_binary']])
    payloads={'native-final.json':native,'native-original.json':original,'native-dynamic.json':refresh,'codegen-combined.json':reports,
              'profile.json':load(original_dir/'profile.json')}
    for tag in ['main','gcc-16','clang-23']:
        folder=args.research/('callgrind-'+tag)
        payloads['callgrind-'+tag+'.json']={str(p.relative_to(folder)):load(p) for p in folder.rglob('*.json')}
    for name,data in payloads.items():
        (args.out/(name+'.gz')).write_bytes(gzip.compress((json.dumps(data,indent=2)+'\n').encode(),mtime=0))
    print(json.dumps({k:v for k,v in summary.items() if k in ('native_groups',)},indent=2))
    print('Callgrind:',{k:(v['n'],v['geometric_ir']) for k,v in summary['callgrind'].items()})

if __name__=='__main__':
    main()
