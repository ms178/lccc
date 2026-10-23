#!/usr/bin/env python3
"""Matched-ISA SIMD probe, correctness-first; no performance acceptance threshold.

Use immutable --before/--before-include snapshots. Before defects are retained
as failures and never timed. After and vendor failures abort. Run only on an
x86-64-v4 host, without concurrent compiler builds or other timing jobs.
"""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import random
import statistics
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def execute(binary, count):
    begin = time.perf_counter_ns()
    r = subprocess.run([str(binary), str(count)], capture_output=True, timeout=120)
    elapsed = (time.perf_counter_ns() - begin) / 1e9
    return r, elapsed


def retain_correct_candidates(row, valid, expected):
    """Keep only binaries whose one-shot output matches GCC's receipt.

    The historical --before build is diagnostic, not a required oracle: an
    old implementation may compile and return successfully while producing
    the defect this probe is meant to measure. Record that mismatch and omit
    it from timing. A mismatch from after/GCC/Clang invalidates the run.
    """
    for compiler in tuple(valid):
        artifact = row['artifacts'][compiler]
        matches = artifact['stdout'] == expected
        artifact['output_matches_gcc'] = matches
        if matches:
            artifact['status'] = 'correct'
        elif compiler == 'before':
            artifact['status'] = 'correctness-mismatch'
            del valid[compiler]
        else:
            artifact['status'] = 'correctness-mismatch'
            raise RuntimeError(f"{row['name']}/{compiler}: vendor/candidate checksums differ")


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    for name in ('before', 'after', 'gcc', 'clang', 'before-include', 'output'):
        ap.add_argument('--' + name, required=True, type=Path)
    ap.add_argument('--rounds', type=int, default=9)
    ap.add_argument('--minimum-seconds', type=float, default=0.2)
    args = ap.parse_args()
    if args.rounds < 7 or not math.isfinite(args.minimum_seconds) or args.minimum_seconds < 0.2:
        ap.error('require at least seven rounds and 0.2 seconds per sample')
    flags_cpu = Path('/proc/cpuinfo').read_text().split('flags', 1)[1].splitlines()[0]
    if not all(x in flags_cpu.split() for x in ('avx2', 'avx512f', 'avx512dq', 'avx512cd', 'avx512bw', 'avx512vl')):
        ap.error('complete x86-64-v4 execution hardware is required')
    cpu = min(os.sched_getaffinity(0))
    os.sched_setaffinity(0, {cpu})
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=True)
    flags = ['-O2', '-std=gnu17', '-march=x86-64-v4', '-fno-fast-math', '-ffp-contract=off']
    source = ROOT / 'tests/bench/simd_conversion.c'
    compilers = {k: getattr(args, k).resolve() for k in ('before', 'after', 'gcc', 'clang')}
    receipt = dict(complete=False, cpu=cpu, flags=flags, rounds=args.rounds,
                   source_sha256=digest(source), header_sha256=digest(ROOT / 'include/lcccsimd.h'),
                   before_header_sha256=digest(args.before_include / 'lcccsimd.h'), compilers={k: dict(path=str(p), sha256=digest(p),
                   version=subprocess.check_output([str(p), '--version'], text=True).splitlines()[0])
                   for k, p in compilers.items()}, rows=[])
    rng = random.Random(73921)
    for kind, name in enumerate(('zero_extend', 'widen_integer', 'truncate_double', 'permute_double', 'copy_full_width')):
        row = dict(name=name, artifacts={}, samples=[])
        valid = {}
        for k, compiler in compilers.items():
            binary = out / f'{name}-{k}'
            include = args.before_include.resolve() if k == 'before' else ROOT / 'include'
            cmd = [str(compiler), *flags, f'-DKIND={kind}']
            if k in ('before', 'after'):
                cmd += ['-I' + str(include)]
            cmd += [str(source), '-o', str(binary)]
            r = subprocess.run(cmd, capture_output=True, text=True, timeout=120)
            artifact = dict(command=cmd, compile_exit=r.returncode, diagnostics=r.stderr)
            row['artifacts'][k] = artifact
            if r.returncode:
                artifact['status'] = 'compile-failure'
                if k != 'before':
                    raise RuntimeError(f'{name}/{k} failed to compile: {r.stderr}')
                continue
            result, _ = execute(binary, 1)
            artifact.update(binary_sha256=digest(binary), run_exit=result.returncode,
                            stdout=result.stdout.decode(), stderr=result.stderr.decode())
            if result.returncode:
                artifact['status'] = 'runtime-failure'
                if k != 'before':
                    raise RuntimeError(f'{name}/{k} failed independent lane checks')
                continue
            valid[k] = binary
        expected = row['artifacts']['gcc']['stdout']
        retain_correct_candidates(row, valid, expected)
        count = 1
        while True:
            durations = [execute(b, count)[1] for b in valid.values()]
            if min(durations) >= args.minimum_seconds:
                break
            count *= 2
            if count > (1 << 26):
                raise RuntimeError("calibration exceeded the bounded iteration budget")
        row['iterations'] = count
        # One discarded warm-up for each compiler, all measured samples retained.
        for b in valid.values():
            result, _ = execute(b, count)
            if result.returncode != 0 or result.stdout.decode() != expected:
                raise RuntimeError(f"{name}: incorrect warmup output")
        for _ in range(args.rounds):
            order = list(valid)
            rng.shuffle(order)
            sample = dict(order=order, wall_s={})
            for k in order:
                result, elapsed = execute(valid[k], count)
                if result.returncode != 0 or result.stdout.decode() != expected:
                    raise RuntimeError(f"{name}/{k}: incorrect timed output")
                sample['wall_s'][k] = elapsed
            row['samples'].append(sample)
        row['after_over'] = {k: statistics.median(s['wall_s']['after'] / s['wall_s'][k]
                                                 for s in row['samples']) for k in valid if k != 'after'}
        row['exploratory_ci95'] = {}
        bootstrap_rng = random.Random(83279)
        for k in row['after_over']:
            ratios = [s['wall_s']['after'] / s['wall_s'][k] for s in row['samples']]
            boots = sorted(statistics.median(bootstrap_rng.choices(ratios, k=len(ratios)))
                           for _ in range(4000))
            row['exploratory_ci95'][k] = [boots[100], boots[3899]]
        row['minimum_sample_s'] = min(t for s in row['samples'] for t in s['wall_s'].values())
        receipt['rows'].append(row)
        (out / 'results.json').write_text(json.dumps(receipt, indent=2) + '\n')
        print(name, row['after_over'], flush=True)
    receipt['complete'] = True
    (out / 'results.json').write_text(json.dumps(receipt, indent=2) + '\n')


if __name__ == '__main__':
    main()
