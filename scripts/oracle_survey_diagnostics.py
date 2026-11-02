#!/usr/bin/env python3
"""Publish bounded CE survey diagnostics even after a failed producer step.

This is observability, not a gate or a fallback: the producer's nonzero exit
and missing vendors remain failures. No instruction-count ranks are published
here. Full assembly/reports remain in the uploaded artifact.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

BUDGET = 3900
MAX_DETAILS = 16


def read_json(path):
    path = Path(path)
    if path.stat().st_size > 16 * 1024 * 1024:
        raise ValueError('report exceeds diagnostic reader budget')
    return json.loads(path.read_text())


def emit(data):
    text = json.dumps(data, separators=(',', ':'), ensure_ascii=True)
    # Check the encoded command too; % escaping can expand the wire payload.
    text = text.replace('%', '%25').replace('\r', '%0D').replace('\n', '%0A')
    if len(text.encode()) > BUDGET:
        # A bounded prefix remains useful even for hostile/control-heavy
        # diagnostics. JSON-encode it again rather than slicing a command.
        text = json.dumps({'detail_truncated': True, 'excerpt':
                           json.dumps(data, ensure_ascii=True)[:900]}, ensure_ascii=True)
        text = text.replace('%', '%25').replace('\r', '%0D').replace('\n', '%0A')
        assert len(text.encode()) <= BUDGET
    print('::notice title=Oracle survey diagnostics::' + text)


def publish(index_path):
    path = Path(index_path)
    try:
        index = read_json(path)
        results = index['results']
        failures = index['failures']
        if not isinstance(results, dict) or not isinstance(failures, list):
            raise ValueError('malformed index')
    except (OSError, ValueError, KeyError, TypeError) as error:
        emit({'available': False, 'error': str(error)[:500], 'comparison_pass': False})
        return
    emit({'available': True, 'complete': index.get('complete') is True,
          'failed_requests': len(failures), 'recorded_requests': len(results),
          'index_sha256': hashlib.sha256(path.read_bytes()).hexdigest(),
          'detail_limit': MAX_DETAILS, 'comparison_pass': False,
          'note': 'diagnostics only; producer status and complete vendor checks remain authoritative'})
    for failure in failures[:MAX_DETAILS]:
        name = str(failure.get('name', 'unknown'))[:100] if isinstance(failure, dict) else 'unknown'
        row = results.get(name, {})
        detail = {'workload': name, 'failure': str(failure)[:300],
                  'requested_oracles': None, 'requested_flags': None}
        arguments = row.get('arguments', []) if isinstance(row, dict) else []
        for option, key in (('--oracles', 'requested_oracles'), ('--flags', 'requested_flags')):
            if isinstance(arguments, list) and option in arguments:
                i = arguments.index(option) + 1
                if i < len(arguments):
                    detail[key] = str(arguments[i])[:300]
        try:
            reports = read_json(row['report'])
            if not isinstance(reports, list) or len(reports) != 1:
                raise ValueError('expected exactly one source report')
            report = reports[0]
            detail['errors'] = [str(error)[:400] for error in report['errors'][:4]]
            detail['available_compilers'] = [str(record['key'])[:60] for record in report['records'][:5]]
        except (OSError, ValueError, KeyError, TypeError) as error:
            detail['report_error'] = str(error)[:400]
        emit(detail)
    if len(failures) > MAX_DETAILS:
        emit({'omitted_failure_details': len(failures) - MAX_DETAILS,
              'note': 'complete errors retained in uploaded corpus index and per-workload reports'})


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument('index', type=Path)
    args = parser.parse_args(argv)
    publish(args.index)
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
