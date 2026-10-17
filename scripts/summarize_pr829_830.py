#!/usr/bin/env python3
"""Reproduce the PR829/830 report from complete raw experiments, without reranking away failures."""
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
from paired_ab import sign_test_p

LABELS = dict(ccc='PR829', lev='PR830', gcc='GCC16', clang='Clang23')
WORST15 = set('byte_load_reassembly spectral_norm expat_siphash24 sha256_transform expat_xml_scan linux_find_bit_scaled linux_find_bit sieve binary_trees lz4_match_extend mandelbrot sqlite_varint zlib_ng_adler32_combine strlen_bench switch_dispatch'.split())


def load(path):
    return json.loads(path.read_text())


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def geomean(values):
    return math.exp(statistics.fmean(math.log(v) for v in values))


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--cache', type=Path, required=True)
    p.add_argument('--logs', type=Path, required=True)
    p.add_argument('--out', type=Path, required=True)
    p.add_argument('--compiler', type=Path, required=True)
    args = p.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    initial = load(args.logs / 'native45.json')
    tls = load(args.logs / 'native-tls-v4.json')
    replay = load(args.cache / 'native-replay-v4/replay.json')
    original_hashes = load(args.logs / 'native45-strict-revalidation.json')['binary_sha256']
    expected = {b.name for b in bench.BENCHMARKS}
    assert replay['complete'] and not replay['failures']
    assert replay['changed'] == ['tls_seg_access']
    assert replay['compiler_sha256'] == sha(args.compiler)
    assert {r['name'] for r in initial['benchmarks']} == expected
    assert [r['name'] for r in tls['benchmarks']] == ['tls_seg_access']
    assert initial['settings'] == tls['settings'], 'native settings changed'
    current_hashes = {r['name']: r['current_sha256'] for r in replay['checks']}
    summary = dict(compiler_sha256=sha(args.compiler), native={}, callgrind={}, confirmations={})
    rows = []
    for old in initial['benchmarks']:
        name = old['name']
        row = tls['benchmarks'][0] if name == 'tls_seg_access' else old
        bench.evaluate_correctness(row, ['lccc', *LABELS])
        assert all(r['matches_baseline'] is True for r in row['correctness'].values()), name
        hashes = ({k: row['compile'][k]['binary_sha256'] for k in ['lccc', *LABELS]}
                  if name == 'tls_seg_access' else dict(original_hashes[name], lccc=current_hashes[name]))
        assert hashes['lccc'] == current_hashes[name]
        record = dict(name=name, tags=row['tags'], source_sha256=row['source_sha256'],
                      original_epyc_screen15=name in WORST15,
                      epoch='native-tls-v4.json' if name == 'tls_seg_access' else 'native45.json',
                      binary_sha256=hashes, comparisons={})
        for key, label in LABELS.items():
            ratio = row['ratios_from_lccc'][key]
            record['comparisons'][label] = dict(median=ratio['median'],
                ci95_exploratory=ratio['median_ci95_bootstrap'],
                identical_binary=hashes['lccc'] == hashes[key],
                candidate_quality=row['runtime']['lccc']['quality_warnings'],
                reference_quality=row['runtime'][key]['quality_warnings'])
        rows.append(record)
    summary['native']['rows'] = rows
    summary['native']['method'] = ('45 workloads, randomized interleaved 7 rounds + 1 warmup; '
        'TLS remeasured after its repair, other 44 binaries proven unchanged. Ratios <1 favor '
        'candidate. Identical binaries are uninformative about compiler improvement. '
        'Intervals are exploratory, not multiplicity corrected. No samples removed.')
    for group, selected in [('all45', rows), ('workload18', [r for r in rows if 'workload' in r['tags']]),
                            ('synthetic27', [r for r in rows if 'synthetic' in r['tags']]),
                            ('original_screen15', [r for r in rows if r['original_epyc_screen15']])]:
        summary['native'][group] = dict(n=len(selected), geomean={label: geomean(
            r['comparisons'][label]['median'] for r in selected) for label in LABELS.values()})
    for tag in ['pr829', 'pr830', 'gcc16', 'clang23']:
        matrix = load(args.cache / ('v4-corpus-' + tag) / 'index.json')
        assert matrix['complete'] and not matrix['failures']
        assert set(matrix['results']) == expected
        summary['callgrind'][tag] = dict(n=len(matrix['results']),
            geomean_ir=geomean(r['metrics']['ir_ratio'] for r in matrix['results'].values()),
            rows=matrix['results'])
    for name in ['struct-native-paired', 'tls-native-paired']:
        raw = load(args.logs / (name + '.json'))
        # Historical files used a rounded normal approximation. Retain those
        # originals and recompute an exact test from the recorded pair signs.
        assert raw['rounds'] == len(raw['paired_diffs_ms']) == 25
        summary['confirmations'][name] = dict(
            median_a_ms=raw['arm_a_timings']['median_ms'],
            median_b_ms=raw['arm_b_timings']['median_ms'],
            elapsed_ratio_a_over_b=raw['arm_a_timings']['median_ms']/raw['arm_b_timings']['median_ms'],
            exact_sign_test_p=sign_test_p(raw['paired_diffs_ms']),
            historical_rounded_normal_p=raw['sign_test_p'])
    summary['compiler_work'] = load(args.cache / 'v4-compiler-work/index.json')
    assert summary['compiler_work']['complete']
    (args.out / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')
    with (args.out / 'native-all45.csv').open('w', newline='') as stream:
        writer = csv.writer(stream)
        writer.writerow(['workload', 'group', 'original_screen15', *LABELS.values(),
                         *[label + '_identical_binary' for label in LABELS.values()]])
        for row in rows:
            writer.writerow([row['name'], 'workload' if 'workload' in row['tags'] else 'synthetic',
                             row['original_epyc_screen15'],
                             *[row['comparisons'][label]['median'] for label in LABELS.values()],
                             *[row['comparisons'][label]['identical_binary'] for label in LABELS.values()]])
    for name in ['native45.json', 'native-tls-v4.json', 'native45-strict-revalidation.json',
                 'struct-native-paired.json', 'tls-native-paired.json', 'compiler-identities.json',
                 'intel-executed-identities.json', 'fp-recurrence-controls.json',
                 'fp-recurrence-ablations-2.json', 'shared-pr830.json', 'shared-synthesis.json',
                 'shared-loader-synthesis.json', 'shared-loader-pr830.json', 'linker-v4.json']:
        data = (args.logs / name).read_bytes()
        (args.out / (name + '.gz')).write_bytes(gzip.compress(data, mtime=0))
    (args.out / 'native-replay-v4.json').write_text(json.dumps(replay, indent=2) + '\n')
    print(json.dumps({k: v for k, v in summary['native'].items() if k != 'rows'}, indent=2))


if __name__ == '__main__':
    main()
