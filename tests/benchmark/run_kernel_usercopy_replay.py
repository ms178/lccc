#!/usr/bin/env python3
"""Paired x86-64 userspace replay of two LCCC-built lib/usercopy.o objects.

This is NOT a kernel boot, privileged-SMAP test or whole-kernel benchmark.
Alternative instructions are not patched; a shared test-only return thunk is
used. Faulting pointers are excluded. The harness checks 4,488 valid-buffer
alignment/length/nonzero cases before every timed run. Only LCCC compiles the
harness; callers supply the two kernel objects and their build provenance.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import random
import shutil
import statistics
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
FIXTURES = Path(__file__).resolve().parent / 'fixtures'


def run(argv, timeout=60):
    result = subprocess.run(list(map(str, argv)), capture_output=True, text=True,
                            timeout=timeout, check=False)
    if result.returncode:
        raise RuntimeError(f'command failed ({result.returncode}): {list(map(str, argv))!r}; '
                           f'stdout={result.stdout!r}; stderr={result.stderr!r}')
    if result.stderr:
        raise RuntimeError(f'unexpected diagnostic: {result.stderr}')
    return result.stdout


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def function_size(obj):
    sizes = [int(fields[1], 16) for line in run(['nm', '-S', '--defined-only', obj]).splitlines()
             if len(fields := line.split()) == 4 and fields[3] == 'check_zeroed_user']
    if len(sizes) != 1 or sizes[0] <= 0:
        raise RuntimeError(f'expected one sized check_zeroed_user in {str(obj)!r}')
    return sizes[0]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--before-object', required=True, type=Path)
    parser.add_argument('--after-object', required=True, type=Path)
    parser.add_argument('--lccc', default=str(ROOT / 'target/fastbuild/lccc'))
    parser.add_argument('--samples', type=int, default=21)
    parser.add_argument('--warmup', type=int, default=3)
    parser.add_argument('--iterations', type=int, default=10_000_000)
    parser.add_argument('--cpu', type=int)
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    if platform.system() != 'Linux' or platform.machine() != 'x86_64':
        parser.error('this replay requires a native x86-64 Linux host')
    if not (5 <= args.samples <= 1000 and 0 <= args.warmup <= 100
            and 1 <= args.iterations <= 1_000_000_000):
        parser.error('require 5..1000 samples, 0..100 warmups and 1..1e9 iterations')
    compiler = Path(shutil.which(args.lccc) or args.lccc).resolve(strict=True)
    version = run([compiler, '--version']).splitlines()[0]
    if 'lccc' not in version.lower():
        parser.error('only LCCC may compile this harness; GCC kernel builds are forbidden')
    objects = {'before': args.before_object.resolve(strict=True),
               'after': args.after_object.resolve(strict=True)}
    affinity = os.sched_getaffinity(0)
    cpu = min(affinity) if args.cpu is None else args.cpu
    if cpu not in affinity:
        parser.error('selected CPU is outside the process affinity set')
    rows = {mode: [] for mode in objects}
    with tempfile.TemporaryDirectory(prefix='lccc-usercopy-replay-') as directory:
        work = Path(directory)
        for mode, obj in objects.items():
            run([compiler, '-O2', '-fno-pie', '-nostdlib', '-static',
                 f'-DREPLAY_ITERATIONS={args.iterations}UL',
                 FIXTURES / 'kernel_usercopy_replay.c',
                 FIXTURES / 'kernel_usercopy_replay_start.S', obj, '-o', work / mode])
        try:
            os.sched_setaffinity(0, {cpu})
            for i in range(args.warmup + args.samples):
                # Alternate order to reduce one-sided drift; pair adjacent runs.
                for mode in (('before', 'after') if i % 2 == 0 else ('after', 'before')):
                    output = run([work / mode]).strip()
                    if not output.isdecimal() or int(output) <= 0:
                        raise RuntimeError(f'invalid timing output: {output!r}')
                    if i >= args.warmup:
                        rows[mode].append(int(output))
        finally:
            os.sched_setaffinity(0, affinity)
    ratios = [a / b for a, b in zip(rows['after'], rows['before'])]
    rng = random.Random(178)
    bootstrap = sorted(statistics.median(rng.choices(ratios, k=len(ratios)))
                       for _ in range(10_000))
    report = {
        'scope': 'unprivileged kernel-function replay, not whole-kernel runtime',
        'compiler': version, 'compiler_sha256': digest(compiler),
        'object_sha256': {k: digest(v) for k, v in objects.items()},
        'function_bytes': {k: function_size(v) for k, v in objects.items()},
        'cpu': cpu, 'platform': platform.platform(),
        'cpu_model': next((line.split(':', 1)[1].strip()
                           for line in Path('/proc/cpuinfo').read_text().splitlines()
                           if line.startswith('model name')), 'unknown'),
        'harness_sha256': {p.name: digest(p) for p in
                           (FIXTURES / 'kernel_usercopy_replay.c',
                            FIXTURES / 'kernel_usercopy_replay_start.S')},
        'warmup_pairs': args.warmup, 'sample_pairs': args.samples,
        'iterations_per_run': args.iterations, 'correctness_cases_per_run': 4488,
        'samples_ns': rows,
        'median_ns': {k: statistics.median(v) for k, v in rows.items()},
        'ratio_of_medians': statistics.median(rows['after']) / statistics.median(rows['before']),
        'paired_median_ratio': statistics.median(ratios),
        'faster_pairs': sum(r < 1 for r in ratios),
        'exploratory_bootstrap_paired_ratio_95pct': [bootstrap[250], bootstrap[9750]],
        'limits': ['no privileged SMAP patching or fault recovery',
                   'VM scheduling/frequency noise remains; bootstrap assumes independent pairs',
                   'object provenance must be recorded separately; no other compiler is invoked'],
    }
    text = json.dumps(report, indent=2) + '\n'
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(text)
    print(text, end='')


if __name__ == '__main__':
    main()
