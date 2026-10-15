#!/usr/bin/env python3
"""Summarize canonical named-arm benchmark evidence without timing false wins.

Keep every registered shape and every failure visible. Ratios below 1 mean
LCCC faster, not reference/LCCC. Launch-floor rows stay in the correctness
inventory but are excluded from the primary geometric-mean screening ratio.
No number from this script is a hardware-counter or bare-metal performance claim.
"""
from __future__ import annotations

import argparse
from contextlib import redirect_stdout
import hashlib
import io
import json
import math
from pathlib import Path
import statistics
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tests.benchmark import run_benchmarks as bench
from tools.corpus import publication


def summarize(data, floor=.020):
    expected = {item.name for item in bench.BENCHMARKS}
    rows = data['benchmarks']
    names = [row['name'] for row in rows]
    if len(names) != len(expected) or set(names) != expected:
        raise ValueError('report does not cover the exact complete benchmark registry')
    arms = [item['key'] for item in data['compilers']]
    if len(arms) != len(set(arms)) or 'lccc' not in arms or 'gcc' not in arms:
        raise ValueError('requires unique compiler arms, including LCCC and GCC')
    result = {'workloads': len(rows), 'skip_perf': data['settings']['skip_perf'],
              'floor_s': floor, 'identity': data['compilers'], 'rows': [], 'comparisons': {}}
    for row in rows:
        item = {'name': row['name'], 'correct': {}, 'ratios': {}}
        # Revalidate raw observations, including untimed warm-ups. Cached
        # success flags from an older or inconsistent report are not proof.
        validated = {'runs': row.get('runs', {}), 'warmups': row.get('warmups', {}),
                     'correctness': {}}
        bench.evaluate_correctness(validated, arms)
        for arm in arms:
            check = row.get('correctness', {}).get(arm, {})
            item['correct'][arm] = bool(check.get('matches_baseline') is True
                and check.get('stable_within_compiler') is True
                and validated['correctness'][arm]['matches_baseline'] is True)
        if not item['correct']['gcc']:
            item['correct'] = {arm: False for arm in arms}
        for arm in arms:
            if arm == 'lccc':
                continue
            ratio = row.get('ratios_from_lccc', {}).get(arm)
            wall_a = row.get('runtime', {}).get('lccc', {}).get('wall') or {}
            wall_b = row.get('runtime', {}).get(arm, {}).get('wall') or {}
            above_floor = wall_a.get('median', 0) >= floor and wall_b.get('median', 0) >= floor
            enough_samples = bool(ratio and 7 <= ratio.get('n', 0)
                                  <= min(len(row.get('runs', {}).get('lccc', [])),
                                         len(row.get('runs', {}).get(arm, []))))
            eligible = bool(not result['skip_perf'] and item['correct']['lccc']
                            and item['correct'][arm] and item['correct']['gcc']
                            and above_floor and enough_samples)
            item['ratios'][arm] = {'eligible': eligible, 'above_floor': above_floor,
                                   'enough_samples': enough_samples, 'paired': ratio}
        result['rows'].append(item)
    for arm in arms:
        if arm == 'lccc':
            continue
        eligible = [row['ratios'][arm]['paired']['median'] for row in result['rows']
                    if row['ratios'][arm]['eligible']]
        if any(not math.isfinite(value) or value <= 0 for value in eligible):
            raise ValueError('nonpositive/nonfinite timing ratio')
        result['comparisons'][arm] = {
            'correct': sum(row['correct'][arm] for row in result['rows']),
            'candidate_correct': sum(row['correct']['lccc'] for row in result['rows']),
            'timing_eligible': len(eligible), 'timing_excluded': len(rows) - len(eligible),
            'geometric_mean_ratio': math.exp(statistics.fmean(map(math.log, eligible))) if eligible else None,
            'arithmetic_mean_ratio': statistics.fmean(eligible) if eligible else None,
            'interpretation': 'LCCC/reference paired median ratio; screening only; <1 faster',
        }
    return result


COMPACT_RATIO_COLUMNS = ('eligible', 'median', 'ci95', 'n', 'above_floor', 'enough_samples')


def compact_rows(result):
    """Every workload and arm, without duplicating verbose raw paired samples."""
    rows = []
    for row in result['rows']:
        ratios = {}
        for label, arm in row['ratios'].items():
            pair = arm['paired'] or {}
            ratios[label] = [arm['eligible'], pair.get('median'),
                             pair.get('median_ci95_bootstrap'), pair.get('n'),
                             arm['above_floor'], arm['enough_samples']]
        rows.append({'name': row['name'], 'correct': row['correct'], 'ratios': ratios})
    return rows


def bounded_notices(produce, limit=10):
    """Actions retains only ten notices per step: fail before losing rows.

    Buffer the entire section first, so a future larger schema cannot silently
    publish a successful-looking prefix and discard the remainder.
    """
    buffer = io.StringIO()
    with redirect_stdout(buffer):
        status = produce()
    if len(buffer.getvalue().splitlines()) > limit:
        raise ValueError('evidence exceeds the per-step notice budget; split the section')
    print(buffer.getvalue(), end='')
    return status


def codegen_rows(index):
    """Compact whole-TU counts, retaining shared-source registry aliases.

    The oracle's legacy ``spills`` field counts stack references, not proven
    allocator spills. Keep that limitation explicit in the published schema.
    Failed rows remain in the index's failure inventory, never as zero counts.
    """
    rows = []
    for name, entry in index['results'].items():
        if entry.get('exit_code') != 0 or entry.get('error'):
            continue
        raw = Path(entry['report']).read_bytes()
        reports = json.loads(raw)
        if len(reports) != 1 or reports[0].get('errors'):
            raise ValueError('invalid successful codegen report: ' + name)
        local = [record for record in reports[0]['records'] if record['key'] == 'lccc']
        if len(local) != 1:
            raise ValueError('codegen report requires one local record: ' + name)
        metrics = {field: local[0][field] for field in
                   ('instructions', 'loads', 'stores', 'branches', 'vectors', 'calls', 'spills')}
        if any(type(value) is not int or value < 0 for value in metrics.values()):
            raise ValueError('invalid static count: ' + name)
        metrics['stack_refs'] = metrics.pop('spills')
        rows.append({'name': name, 'sha256': hashlib.sha256(raw).hexdigest(), **metrics})
    return rows


def notice(title, data):
    text = json.dumps(data, separators=(',', ':'), allow_nan=False)
    if len(text.encode()) > 3800:
        raise ValueError('notice exceeds checks-API evidence budget')
    escaped = text.replace('%', '%25').replace('\r', '%0D').replace('\n', '%0A')
    print(f'::notice title={title}::{escaped}')


def notice_rows(title, rows):
    """Pack rows into bounded notices instead of losing late rows to UI caps."""
    chunk = []
    for row in rows:
        if chunk and len(json.dumps([*chunk, row], separators=(',', ':'), allow_nan=False).encode()) > 3500:
            notice(title, chunk)
            chunk = []
        chunk.append(row)
    if chunk:
        notice(title, chunk)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument('report', type=Path)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--annotations', action='store_true')
    args = parser.parse_args(argv)
    data = json.loads(args.report.read_text())
    result = summarize(data)
    result['report_sha256'] = hashlib.sha256(args.report.read_bytes()).hexdigest()
    args.out.parent.mkdir(parents=True, exist_ok=True)
    publication.atomic_bytes(args.out, (json.dumps(result, indent=2, allow_nan=False) + '\n').encode())
    if args.annotations:
        def publish():
            notice('compiler comparison aggregate', {
                **{key: value for key, value in result.items() if key not in ('rows', 'identity')},
                'ratio_columns': COMPACT_RATIO_COLUMNS})
            notice_rows('compiler comparison rows', compact_rows(result))
        bounded_notices(publish)
    print(json.dumps(result['comparisons'], sort_keys=True))
    return int(any(not row['correct']['lccc'] or not row['correct']['gcc'] for row in result['rows']))


if __name__ == '__main__':
    raise SystemExit(main())
