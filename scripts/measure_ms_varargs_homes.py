#!/usr/bin/env python3
"""Measure MS variadic home saves; synthetic ABI fixture, NOT a kernel benchmark.

Both target objects use LCCC. GCC builds only the non-kernel timing/oracle
harness. Alternating runs and raw wall/CPU samples expose noisy results; a
smaller function is not evidence of a whole-kernel runtime improvement.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import statistics
import subprocess
import tempfile


def run(command, **kwargs):
    return subprocess.check_output(command, text=True, **kwargs)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--before', required=True, type=Path)
    parser.add_argument('--after', required=True, type=Path)
    parser.add_argument('--out', required=True, type=Path)
    parser.add_argument('--gcc', default=os.environ.get('GCC', 'gcc'))
    parser.add_argument('--iterations', type=int, default=5000000)
    parser.add_argument('--rounds', type=int, default=7)
    args = parser.parse_args()
    if args.iterations < 1 or args.rounds < 3:
        parser.error('positive iteration count and at least three rounds required')
    source = '''
#define MS __attribute__((ms_abi,noinline))
MS long four(long a,long b,long c,long d,...) {
 __builtin_ms_va_list ap; __builtin_ms_va_start(ap,d);
 long v=__builtin_va_arg(ap,long); __builtin_ms_va_end(ap);
 return a+b+c+d+v;
}
MS long eight(long a,long b,long c,long d,long e,long f,long g,long h,...) {
 __builtin_ms_va_list ap; __builtin_ms_va_start(ap,h);
 long v=__builtin_va_arg(ap,long); __builtin_ms_va_end(ap);
 return a+b+c+d+e+f+g+h+v;
}
'''
    harness = r'''
#include <time.h>
#include <stdio.h>
#include <stdlib.h>
extern __attribute__((ms_abi)) long four(long,long,long,long,...);
extern __attribute__((ms_abi)) long eight(long,long,long,long,long,long,long,long,...);
static double now(clockid_t id) {
 struct timespec t; if(clock_gettime(id,&t)) exit(3);
 return t.tv_sec+t.tv_nsec*1e-9;
}
int main(int argc,char **argv) {
 if(argc!=2)return 4;
 unsigned long n=strtoul(argv[1],0,10), sum=0;
 double wall=now(CLOCK_MONOTONIC),cpu=now(CLOCK_PROCESS_CPUTIME_ID);
 for(unsigned long i=0;i<n;++i) {
  sum+=four(1,2,3,4,5L);
  sum+=eight(1,2,3,4,5,6,7,8,9L);
 }
 cpu=now(CLOCK_PROCESS_CPUTIME_ID)-cpu;wall=now(CLOCK_MONOTONIC)-wall;
 if(sum!=n*60UL)return 5;
 printf("%.9f %.9f %lu\n",wall,cpu,sum);return 0;
}
'''
    report = {'scope': 'synthetic non-kernel Microsoft variadic ABI fixture',
              'limits': 'Not a kernel benchmark; raw samples are not a statistical significance claim.',
              'iterations': args.iterations, 'rounds': args.rounds,
              'flags': ['-O2', '-mno-sse', '-mno-sse2'],
              'source_sha256': hashlib.sha256(source.encode()).hexdigest(),
              'harness_sha256': hashlib.sha256(harness.encode()).hexdigest(),
              'gcc_version': run([args.gcc, '--version']).splitlines()[0],
              'variants': {}}
    with tempfile.TemporaryDirectory(prefix='ms-va-homes-') as directory:
        tmp = Path(directory)
        (tmp / 'target.c').write_text(source)
        (tmp / 'harness.c').write_text(harness)
        subprocess.run([args.gcc, '-O2', '-c', str(tmp / 'harness.c'), '-o', str(tmp / 'harness.o')], check=True)
        for label, compiler in [('before', args.before), ('after', args.after)]:
            compiler = compiler.resolve(strict=True)
            obj = tmp / (label + '.o')
            subprocess.run([str(compiler), *report['flags'], '-c', str(tmp / 'target.c'), '-o', str(obj)], check=True)
            subprocess.run([args.gcc, str(obj), str(tmp / 'harness.o'), '-o', str(tmp / label)], check=True)
            sizes = {}
            for line in run(['nm', '-S', '--defined-only', str(obj)]).splitlines():
                row = line.split()
                if len(row) == 4 and row[3] in ('four', 'eight'):
                    sizes[row[3]] = int(row[1], 16)
            if set(sizes) != {'four', 'eight'}:
                raise RuntimeError('incomplete function-size coverage')
            report['variants'][label] = {
                'compiler_sha256': hashlib.sha256(compiler.read_bytes()).hexdigest(),
                'function_bytes': sizes, 'samples': []}
        for round_index in range(args.rounds):
            for label in (('before', 'after') if round_index % 2 == 0 else ('after', 'before')):
                wall, cpu, checksum = run([str(tmp / label), str(args.iterations)]).split()
                report['variants'][label]['samples'].append({'wall_s': float(wall), 'cpu_s': float(cpu), 'checksum': int(checksum)})
    for variant in report['variants'].values():
        variant['median_wall_s'] = statistics.median(s['wall_s'] for s in variant['samples'])
        variant['median_cpu_s'] = statistics.median(s['cpu_s'] for s in variant['samples'])
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({k: {key: value for key, value in v.items() if key != 'samples'}
                      for k, v in report['variants'].items()}, indent=2))


if __name__ == '__main__':
    main()
