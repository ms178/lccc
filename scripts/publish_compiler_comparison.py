#!/usr/bin/env python3
"""Publish one complete evidence section within Actions' ten-notice step limit.

One invocation must occupy its own workflow step. Full raw evidence remains in
artifacts; a missing, malformed, incomplete or required-failing section is not
successful validation. Historical semantic failures remain visible without
pretending that they are failures of the required compiler.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
from benchmark_comparison_report import (
    COMPACT_RATIO_COLUMNS, bench, bounded_notices, codegen_rows,
    compact_rows, notice, notice_rows, summarize,
)
from compare_compiler_witnesses import cases

SECTIONS = ('semantic', 'native', 'correctness-o3', 'codegen-final',
            'codegen-main', 'codegen-pr835', 'codegen-prior',
            'callgrind', 'callgrind-gcc-control')


def load(path):
    raw = path.read_bytes()
    return json.loads(raw), hashlib.sha256(raw).hexdigest()


def publish(root, section):
    if section == 'semantic':
        sources = {p.stem: p.read_text().strip() for p in sorted(root.glob('*.commit'))}
        source_complete = (set(sources) == {'final', 'main', 'pr835', 'prior'}
                           and all(len(value) == 40 and set(value) <= set('0123456789abcdef')
                                   for value in sources.values()))
        notice('compiler comparison source identity', sources)
        data, digest = load(root / 'witnesses/index.json')
        notice('native witness comparison', {
            'complete': data['complete'], 'control_policy': data['control_policy'],
            'summary': data['summary'], 'compilers': data['compilers'], 'sha256': digest})
        failures = []
        for row in data['cases']:
            failed = {key: {'build': arm['build']['status'],
                            'run': arm.get('run', {}).get('status')}
                      for key, arm in row['arms'].items() if not arm['pass']}
            if failed:
                failures.append({'case': row['id'], 'failures': failed})
        notice_rows('semantic witness failures', failures)
        expected = {item[0] for item in cases()}
        found = [row['id'] for row in data['cases']]
        complete = data['complete'] and len(found) == len(expected) and set(found) == expected
        required = {'gcc', *data['required']}
        return int(not complete or not source_complete or any(
            not row['arms'][label]['pass'] for row in data['cases'] for label in required))

    if section in ('native', 'correctness-o3'):
        data, digest = load(root / (section + '.json'))
        result = summarize(data)
        if section == 'native':
            if result['skip_perf']:
                raise ValueError('native timing section contains a correctness-only report')
            notice('compiler comparison native', {
                'sha256': digest, 'comparisons': result['comparisons'],
                'ratio_columns': COMPACT_RATIO_COLUMNS})
            notice_rows('compiler comparison rows native', compact_rows(result))
        else:
            if not result['skip_perf']:
                raise ValueError('O3 inventory unexpectedly contains a timing claim')
            notice('compiler comparison correctness-o3', {
                'sha256': digest, 'timing_claim': False,
                'correct': {arm: sum(row['correct'][arm] for row in result['rows'])
                            for arm in result['rows'][0]['correct']}})
            notice_rows('compiler comparison rows correctness-o3', [
                {'name': row['name'], 'correct': row['correct']} for row in result['rows']])
        return int(any(not all(row['correct'].values()) for row in result['rows']))

    data, digest = load(root / section / 'index.json')
    expected = {item.name for item in bench.BENCHMARKS}
    complete = (data['complete'] and set(data['selected']) == expected
                and set(data['results']) == expected and not data['failures'])
    notice(section + ' audit', {
        'complete': data['complete'], 'full_registry_valid': complete,
        'shapes': len(data['results']), 'failure_count': len(data['failures']),
        'failures': data['failures'][:2], 'metric': data['metric'], 'sha256': digest})
    if section.startswith('codegen-'):
        notice_rows('static whole-TU rows ' + section, codegen_rows(data))
    else:
        notice_rows(section + ' rows', [
            {'name': name, **row['metrics']} for name, row in data['results'].items()])
    return int(not complete)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument('--root', type=Path, required=True)
    parser.add_argument('--section', choices=SECTIONS, required=True)
    args = parser.parse_args(argv)
    try:
        return bounded_notices(lambda: publish(args.root, args.section))
    except (OSError, ValueError, KeyError, TypeError) as error:
        text = str(error).replace('%', '%25').replace('\r', '%0D').replace('\n', '%0A')
        print('::error title=Compiler evidence publication failed::' + text)
        return 1


if __name__ == '__main__':
    raise SystemExit(main())
