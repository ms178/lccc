#!/usr/bin/env python3
"""Controlled, checksum-gated generator A/B; not a whole-compiler ranking.

All arms use identical flags/source/repetitions, one pinned allowed CPU, warmups,
and balanced rotating order. Internal monotonic timing excludes process startup
and checksum work. Artifacts retain commands, samples, assembly and identities.
"""
from __future__ import annotations

import argparse
from dataclasses import asdict
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import re
import shutil
import statistics
import subprocess

import codegen_oracle
import godbolt

SOURCE = r'''#include <stdio.h>
#include <stdlib.h>
#include <time.h>
#define N 262144
static unsigned char bytes[N + 2];
__attribute__((noinline)) void generate(unsigned char *dst) {
    for (unsigned i = 0; i < N; ++i)
        dst[i] = (unsigned char)(((i * 37u) + 19u) ^ ((i + 3u) * 11u));
}
int main(int argc, char **argv) {
    unsigned reps = argc > 1 ? (unsigned)strtoul(argv[1], 0, 10) : 10000;
    bytes[0] = 0xa5; bytes[N + 1] = 0x5a;
    struct timespec start, end;
    if (clock_gettime(CLOCK_MONOTONIC, &start)) return 2;
    for (unsigned r = 0; r < reps; ++r) {
        generate(bytes + 1);
        __asm__ __volatile__("" : : "r"(bytes) : "memory");
    }
    if (clock_gettime(CLOCK_MONOTONIC, &end)) return 2;
    if (bytes[0] != 0xa5 || bytes[N + 1] != 0x5a) return 3;
    unsigned long long checksum = 1469598103934665603ULL;
    for (unsigned i = 0; i < N; ++i) {
        unsigned char want = (unsigned char)(((i * 37u) + 19u) ^ ((i + 3u) * 11u));
        if (bytes[i + 1] != want) return 4;
        checksum = (checksum ^ bytes[i + 1]) * 1099511628211ULL;
    }
    double seconds = (double)(end.tv_sec - start.tv_sec)
                   + (double)(end.tv_nsec - start.tv_nsec) * 1e-9;
    printf("%llu %.9f\n", checksum, seconds);
    return 0;
}
'''
FLAGS = ['-std=gnu17', '-O2', '-march=haswell', '-fwrapv']


def run(command, env):
    result = subprocess.run(command, env=env, text=True, capture_output=True, timeout=180)
    if result.returncode:
        raise RuntimeError(f'{command!r}: exit {result.returncode}\n{result.stdout}\n{result.stderr}')
    return result


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--lccc', default='target/fastbuild/lccc')
    parser.add_argument('--gcc', default='gcc-16')
    parser.add_argument('--rounds', type=int, default=9)
    parser.add_argument('--repetitions', type=int, default=10000)
    parser.add_argument('--out', type=Path, default=Path('results/generator-map-measurement'))
    args = parser.parse_args()
    if args.rounds < 3 or args.rounds % 3 or args.repetitions < 1:
        parser.error('rounds must be a positive multiple of three; repetitions must be positive')
    args.out.mkdir(parents=True, exist_ok=True)
    output = args.out.resolve()
    source = output / 'generator.c'
    source.write_text(SOURCE)
    env = {k: v for k, v in os.environ.items()
           if k not in ('CCC_NO_GENERATOR_MAP', 'LCCC_FORCE_SSE2', 'LCCC_DEBUG_VECTORIZE')}
    allowed = sorted(os.sched_getaffinity(0))
    os.sched_setaffinity(0, {allowed[0]})
    compilers = {}
    for key, executable in [('lccc', args.lccc), ('gcc', args.gcc)]:
        resolved = shutil.which(executable) or executable
        binary = Path(resolved).resolve(strict=True)
        compilers[key] = {'path': str(binary), 'sha256': digest(binary),
                          'version': run([str(binary), '--version'], env).stdout.strip()}
    report = {'schema': 1, 'source_sha256': digest(source), 'flags': FLAGS,
              'metric': 'one hot-cache byte-generator kernel, internal monotonic seconds; not a corpus ranking',
              'system': platform.platform(), 'cpu': platform.processor(), 'cpu_affinity': [allowed[0]],
              'allowed_cpus': allowed, 'rounds': args.rounds, 'repetitions': args.repetitions,
              'compilers': compilers, 'arms': {}, 'run_order': []}
    cpuinfo = Path('/proc/cpuinfo').read_text()
    report['cpu_model'] = next((s.split(':', 1)[1].strip() for s in cpuinfo.splitlines()
                                if s.startswith('model name')), 'unknown')
    for label in ['default', 'kill-switch', 'gcc']:
        arm_env = dict(env)
        if label == 'kill-switch':
            arm_env['CCC_NO_GENERATOR_MAP'] = '1'
        compiler = compilers['gcc' if label == 'gcc' else 'lccc']['path']
        binary = output / label
        command = [compiler, *FLAGS, str(source), '-o', str(binary)]
        run(command, arm_env)
        assembly = output / f'{label}.s'
        if label != 'gcc':
            arm_env['LCCC_DEBUG_VECTORIZE'] = '1'
        trace = run([compiler, *FLAGS, '-S', str(source), '-o', str(assembly)], arm_env).stderr
        (output / f'{label}.trace').write_text(trace)
        observed = re.findall(r'^\[VEC-GEN\] fn=(\S+) ', trace, re.MULTILINE)
        if label != 'gcc' and observed != (['generate'] if label == 'default' else []):
            raise RuntimeError(f'{label}: unexpected generator firings: {observed}')
        body = godbolt._function_body(assembly.read_text().splitlines(), 'generate')
        if body is None:
            raise RuntimeError(f'{label}: missing generate assembly')
        report['arms'][label] = {'compile': command, 'binary_sha256': digest(binary),
            'assembly_sha256': digest(assembly), 'generate_stats': asdict(codegen_oracle._stats(body, 'x86_64')),
            'samples_seconds': [], 'generator_firings': observed}
    checksum = None

    def sample(label):
        nonlocal checksum
        result = run([str(output / label), str(args.repetitions)], env)
        words = result.stdout.split()
        if len(words) != 2:
            raise RuntimeError(f'{label}: invalid output {result.stdout!r}')
        value, seconds = int(words[0]), float(words[1])
        if not math.isfinite(seconds) or seconds <= 0:
            raise RuntimeError(f'{label}: invalid timing {seconds}')
        if checksum is None:
            checksum = value
        if value != checksum:
            raise RuntimeError(f'{label}: checksum differs')
        return seconds

    labels = list(report['arms'])
    for label in labels:
        sample(label)  # Full-size, excluded warmup per arm.
    for round_index in range(args.rounds):
        offset = round_index % len(labels)
        order = labels[offset:] + labels[:offset]
        report['run_order'].append(order)
        for label in order:
            report['arms'][label]['samples_seconds'].append(sample(label))
    report['checksum'] = checksum
    for label, arm in report['arms'].items():
        arm['median_seconds'] = statistics.median(arm['samples_seconds'])
        print(f"{label}: median {arm['median_seconds']:.6f} seconds")
    current = report['arms']['default']['samples_seconds']
    report['paired_speedups'] = {
        label: [baseline / enabled for baseline, enabled in zip(arm['samples_seconds'], current)]
        for label, arm in report['arms'].items() if label != 'default'
    }
    (output / 'measurement.json').write_text(json.dumps(report, indent=2) + '\n')
    print(f"wrote {output / 'measurement.json'}")


if __name__ == '__main__':
    main()
