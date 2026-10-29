#!/usr/bin/env python3
"""Run the canonical corpus and collect per-program, per-core-class perf stat evidence.

See docs/hybrid-perf-benchmark.md. This is intentionally a local, opt-in
experiment; counters never replace the runner's differential correctness gate.
"""
from __future__ import annotations

import argparse
import csv
import hashlib
import json
import math
import os
from pathlib import Path
import random
import shlex
import shutil
import subprocess
import sys
import tempfile
from datetime import datetime, timezone

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'tests/benchmark'))
import run_benchmarks as bench


def cpulist(text: str) -> set[int]:
    result: set[int] = set()
    for part in text.strip().split(','):
        bounds = part.split('-')
        if len(bounds) not in (1, 2) or any(not x.isdigit() for x in bounds):
            raise ValueError(f'invalid CPU list {text!r}')
        lo, hi = int(bounds[0]), int(bounds[-1])
        if hi < lo or hi > 1048575:
            raise ValueError(f'invalid CPU range {part!r}')
        result.update(range(lo, hi + 1))
    return result


def choose_cpus(sysfs: Path, allowed: set[int], p_cpu: int | None, e_cpu: int | None):
    if not allowed:
        raise ValueError('empty CPU affinity set')
    paths = {name: sysfs / pmu for name, pmu in [('P', 'cpu_core'), ('E', 'cpu_atom')]}
    hybrid = any(p.exists() for p in paths.values())
    if not hybrid:
        if p_cpu is not None or e_cpu is not None:
            raise ValueError('P/E override requires kernel hybrid PMU cpu_core/cpu_atom')
        return [{'class': 'generic (P/E unknown)', 'cpu': min(allowed), 'pmu': None}]
    if not all(p.is_dir() for p in paths.values()):
        raise ValueError('incomplete hybrid PMU topology; refusing to guess core type')
    classes = {}
    for name, path in paths.items():
        classes[name] = cpulist((path / 'cpus').read_text())
    if classes['P'] & classes['E']:
        raise ValueError('overlapping hybrid PMU CPU lists')
    result = []
    for name, override in [('P', p_cpu), ('E', e_cpu)]:
        candidates = sorted(classes[name] & allowed)
        if override is not None and override not in candidates:
            raise ValueError(f'{name}-CPU {override} not in allowed {name}-core set {candidates}')
        result.append({'class': name, 'cpu': override if override is not None else (candidates[0] if candidates else None),
                       'pmu': paths[name].name, 'allowed_class_cpus': candidates})
    return result


def cpu_metadata(cpu: int) -> dict:
    """Record topology and frequency state; never mutate host policy."""
    base = Path('/sys/devices/system/cpu') / f'cpu{cpu}'
    paths = ('topology/core_id', 'topology/physical_package_id',
             'topology/thread_siblings_list', 'cpufreq/scaling_governor',
             'cpufreq/scaling_driver', 'cpufreq/scaling_cur_freq',
             'cpufreq/scaling_min_freq', 'cpufreq/scaling_max_freq')
    data = {p: bench.read_first(str(base / p)) for p in paths}
    cpuinfo = bench.read_first('/proc/cpuinfo') or ''
    for block in cpuinfo.split('\n\n'):
        fields = dict(line.split(':', 1) for line in block.splitlines() if ':' in line)
        fields = {key.strip(): value.strip() for key, value in fields.items()}
        if fields.get('processor') == str(cpu):
            data['cpuinfo'] = {key: fields.get(key) for key in ('vendor_id', 'cpu family',
                'model', 'model name', 'stepping', 'microcode', 'flags')}
            break
    return data


def event_args(pmu: str | None) -> list[str]:
    def event(name: str) -> str:
        return f'{pmu}/{name}/' if pmu else name
    # Group cycles+instructions to make an IPC ratio meaningful. Do not use
    # -C: that turns this into per-CPU/system-wide rather than per-task stats.
    return ['-e', '{' + event('cycles') + ',' + event('instructions') + '}',
            '-e', event('branches') + ',' + event('branch-misses')]


def counter_status(text: str, pmu: str | None) -> tuple[bool, str]:
    """Refuse unavailable, duplicated or heavily multiplexed core counters."""
    found = {}
    required = {f'{pmu}/{n}/' if pmu else n for n in ('cycles', 'instructions', 'branches', 'branch-misses')}
    for row in csv.reader(text.splitlines(), delimiter=';'):
        if len(row) < 3:
            continue
        event = row[2].strip()
        # perf may append privilege modifiers; event itself is exact otherwise.
        event = event.removesuffix(':u').removesuffix(':k')
        if event.endswith('/u') or event.endswith('/k'):
            event = event[:-1]
        if event not in required:
            continue
        if event in found:
            return False, f'duplicate event {event}'
        try:
            value = float(row[0].strip())
            running = float(row[4].strip()) if len(row) > 4 and row[4].strip() else None
        except ValueError:
            return False, f'{event} unsupported/not counted: {row[0]}'
        if not math.isfinite(value) or value < 0 or (
                event in {f'{pmu}/{n}/' if pmu else n for n in ('cycles', 'instructions')} and value == 0):
            return False, f'invalid {event} value: {value}'
        if running is None or not math.isfinite(running) or running < 95 or running > 100.01:
            return False, f'{event} running percentage unavailable or below 95%: {running}'
        found[event] = value
    missing = required - found.keys()
    if missing:
        return False, 'missing events: ' + ', '.join(sorted(missing))
    return True, 'all four counters present; running >=95%'


def publish(path: Path, data: str):
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, name = tempfile.mkstemp(prefix='.' + path.name + '.', dir=path.parent)
    try:
        with os.fdopen(fd, 'w', encoding='utf-8') as out:
            out.write(data)
            out.flush()
            os.fsync(out.fileno())
        os.replace(name, path)
    finally:
        if os.path.exists(name):
            os.unlink(name)


def run_perf(perf: str, cpu: int, pmu: str | None, binary: Path, timeout: int, expected: str | None):
    # Dedicated output file prevents benchmark stderr from corrupting perf CSV.
    with tempfile.TemporaryDirectory(prefix='lccc-perf-') as tmp:
        csvfile = Path(tmp) / 'stat.csv'
        command = ['taskset', '-c', str(cpu), perf, 'stat', '--all-user', '--no-big-num', '-x', ';',
                   '-o', str(csvfile), *event_args(pmu), '--', str(binary)]
        try:
            p = subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                               timeout=timeout, env=bench.controlled_environment(), cwd=ROOT)
            stdout_hash = hashlib.sha256(p.stdout).hexdigest()
            text = csvfile.read_text(errors='replace') if csvfile.exists() else ''
            ok, why = counter_status(text, pmu)
            if p.returncode != 0:
                ok, why = False, f'perf/program exit {p.returncode}; {why}'
            if expected is not None and stdout_hash != expected:
                ok, why = False, f'benchmark stdout differs from correctness run; {why}'
            return {'ok': ok, 'reason': why, 'command': command, 'returncode': p.returncode,
                    'stdout_sha256': stdout_hash, 'stderr': p.stderr.decode(errors='replace'), 'perf_csv': text}
        except subprocess.TimeoutExpired:
            return {'ok': False, 'reason': f'timeout after {timeout}s', 'command': command}


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--lccc', required=True)
    ap.add_argument('--gcc', default='gcc')
    ap.add_argument('--output', type=Path, required=True, help='human-readable .txt report')
    ap.add_argument('--artifacts', type=Path, required=True, help='persistent evidence directory (must be new)')
    ap.add_argument('--reps', type=int, default=9)
    ap.add_argument('--perf-reps', type=int, default=3)
    ap.add_argument('--warmup', type=int, default=2)
    ap.add_argument('--p-cpu', type=int)
    ap.add_argument('--e-cpu', type=int)
    ap.add_argument('--only', help='development: comma-separated corpus IDs; omit for full corpus')
    args = ap.parse_args(argv)
    if args.reps < 1 or args.perf_reps < 1 or args.warmup < 0:
        ap.error('reps and perf-reps must be positive; warmup must be nonnegative')
    if not shutil.which('taskset'):
        ap.error('taskset is required for reliable core-class pinning')
    perf = shutil.which('perf')
    if not perf:
        ap.error('perf is required (install the Linux perf tools)')
    try:
        classes = choose_cpus(Path('/sys/bus/event_source/devices'), set(os.sched_getaffinity(0)), args.p_cpu, args.e_cpu)
        args.artifacts.mkdir(parents=True, exist_ok=False)
    except (OSError, ValueError) as exc:
        ap.error(str(exc))
    output = args.output.resolve()
    artifacts = args.artifacts.resolve()
    header = ['LCCC vs GCC: ' + ('selected corpus' if args.only else 'full corpus') + ' / hybrid perf evidence',
              'Started UTC: ' + datetime.now(timezone.utc).isoformat(),
              'Evidence directory: ' + str(artifacts),
              'CPU classes (kernel PMU topology intersected with affinity): ' + json.dumps(classes),
              'Times include program startup; short tests and shared machines can be noisy.',
              'Counters are per-task, user-space only, may be scaled; raw CSV includes running percentages.',
              'No cross-class aggregation; counters and times cannot prove whole-application speedups.', '']
    lines = header + ['RUN IN PROGRESS; NOT COMPLETE\n']
    publish(output, '\n'.join(lines))
    failures = []
    perf_version = subprocess.run([perf, '--version'], capture_output=True, text=True)
    lines = header + ['perf version: ' + perf_version.stdout.strip(), '']
    for cls in classes:
        label, cpu, pmu = cls['class'], cls['cpu'], cls['pmu']
        lines.append(f'========== {label} / CPU {cpu} / PMU {pmu or "generic"} ==========')
        if cpu is not None:
            lines.append('CPU topology/frequency/feature snapshot: ' + json.dumps(cpu_metadata(cpu), sort_keys=True))
        if cpu is None:
            lines.append('UNAVAILABLE: no CPU of this class is in the current affinity set.\n')
            failures.append(f'{label}: no CPU allowed for this core class')
            publish(output, '\n'.join(lines) + '\nRUN IN PROGRESS; NOT COMPLETE\n')
            continue
        directory = artifacts / label.replace(' ', '_').replace('/', '_')
        directory.mkdir()
        report = directory / 'corpus.json'
        command = [sys.executable, str(ROOT / 'tests/benchmark/run_benchmarks.py'),
                   '--compilers', 'lccc,gcc', '--lccc', args.lccc, '--gcc', args.gcc,
                   '--cpu', str(cpu), '--reps', str(args.reps), '--warmup', str(args.warmup),
                   '--strict', '--artifact-dir', str(directory / 'artifacts'), '--json', str(report),
                   '--markdown', str(directory / 'corpus.md')]
        if args.only:
            command += ['--only', args.only]
        lines += ['Corpus command: ' + shlex.join(command), 'Corpus running...']
        publish(output, '\n'.join(lines) + '\nRUN IN PROGRESS; NOT COMPLETE\n')
        with (directory / 'corpus.log').open('w') as log:
            rc = subprocess.run(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT,
                                env=bench.controlled_environment()).returncode
        lines.pop()
        lines.append(f'Corpus exit: {rc}; log: {directory / "corpus.log"}')
        if rc:
            failures.append(f'{label}: corpus exit {rc}')
        if not report.is_file():
            failures.append(f'{label}: corpus report missing')
            continue
        data = json.loads(report.read_text())
        if not data.get('complete'):
            failures.append(f'{label}: incomplete corpus')
        lines += ['Corpus metadata and settings:', json.dumps({k: data.get(k) for k in
                  ('metadata', 'settings', 'compilers', 'unavailable_compilers')}, indent=2),
                  'Corpus summary:', bench.render_terminal(data), '']
        probe = run_perf(perf, cpu, pmu, Path('/bin/true'), 15, None)
        lines += ['PMU preflight: ' + json.dumps(probe, indent=2), '']
        if not probe['ok']:
            failures.append(f'{label}: PMU preflight failed; counters not collected')
            publish(output, '\n'.join(lines) + '\nRUN IN PROGRESS; NOT COMPLETE\n')
            continue
        by_name = {b.name: b for b in bench.BENCHMARKS}
        for entry in data['benchmarks']:
            name = entry['name']
            lines.append(f'--- {label} {name} ---')
            if any(entry.get('correctness', {}).get(k, {}).get('matches_baseline') is not True for k in ('gcc', 'lccc')):
                lines.append('SKIPPED: correctness failed or unavailable; see corpus JSON.')
                failures.append(f'{label}/{name}: correctness')
                continue
            rng = random.Random(bench.stable_seed(label, name))
            for round_no in range(args.perf_reps):
                order = ['lccc', 'gcc']
                rng.shuffle(order)
                for key in order:
                    binary = directory / 'artifacts' / name / key / 'benchmark.bin'
                    runs = entry['runs'][key]
                    expected = runs[0]['stdout_sha256']
                    obs = run_perf(perf, cpu, pmu, binary, by_name[name].timeout_s + 15, expected)
                    lines.append(json.dumps({'round': round_no, 'compiler': key, **obs}, sort_keys=True))
                    if not obs['ok']:
                        failures.append(f'{label}/{name}/{key}/round{round_no}: {obs["reason"]}')
            publish(output, '\n'.join(lines) + '\nRUN IN PROGRESS; NOT COMPLETE\n')
    lines += ['', 'RESULT: ' + ('FAILED/INCOMPLETE' if failures else 'COMPLETE'),
              'Failures: ' + json.dumps(failures, indent=2),
              'Finished UTC: ' + datetime.now(timezone.utc).isoformat()]
    publish(output, '\n'.join(lines) + '\n')
    return 1 if failures else 0


if __name__ == '__main__':
    sys.exit(main())
