#!/usr/bin/env python3
"""Offline publisher retained independently of the removed hosted research job."""
import hashlib
import json
import os
from pathlib import Path
root = Path('results/research')
root.mkdir(parents=True, exist_ok=True)
summary = {'metric': 'bounded simulated Ir; NOT native cycles or universal superiority',
           'head': (root / 'candidate.sha').read_text().strip() if (root / 'candidate.sha').exists() else None,
           'baseline': (root / 'baseline.sha').read_text().strip() if (root / 'baseline.sha').exists() else None,
           'artifact': 'compiler-research', 'callgrind': {}}
selected = set(os.environ['RESEARCH_CASES'].split(','))
for arm in ('main', 'gcc'):
    path = root / f'callgrind-{arm}' / 'index.json'
    if not path.exists():
        summary['callgrind'][arm] = {'available': False}
        continue
    raw = path.read_bytes()
    data = json.loads(raw)
    complete = data['complete'] and not data['failures'] and set(data['results']) == selected
    summary['callgrind'][arm] = {
        'complete': complete,
        'report_sha256': hashlib.sha256(raw).hexdigest(),
        'ratios': {name: row['metrics']['ir_ratio'] for name, row in data['results'].items()},
        'failed_cases': [row['name'] for row in data['failures']],
    }
text = json.dumps(summary, separators=(',', ':'))
assert len(text.encode()) <= 3900, 'checks API annotation budget exceeded'
escaped = text.replace('%', '%25').replace('\r', '%0D').replace('\n', '%0A')
print(f'::notice title=Compiler research evidence::{escaped}')
(root / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')
with open(os.environ['GITHUB_STEP_SUMMARY'], 'a') as stream:
    stream.write('```json\n' + json.dumps(summary, indent=2) + '\n```\n')
