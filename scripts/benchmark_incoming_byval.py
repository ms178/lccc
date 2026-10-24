#!/usr/bin/env python3
"""CPU-pinned, alternating A/B measurement of incoming MEMORY parameters.

Select sum_big(struct { long a,b,c; }) or the matrix cf_wrap_phi wrapper.
Retains kernels,
checksums and raw samples. This is a call-inclusive microbenchmark, not evidence
for general application speed or other architectures. Run without build load.
"""
import argparse
import hashlib
import json
import os
import re
from pathlib import Path
import statistics
import subprocess
import tempfile

DRIVER = r'''
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <time.h>
struct Big { long a,b,c; };
extern long original(struct Big);
extern long optimized(struct Big);
static uint64_t ns(void) {
    struct timespec t;
    if (clock_gettime(CLOCK_MONOTONIC, &t)) exit(2);
    return (uint64_t)t.tv_sec*1000000000u+t.tv_nsec;
}
int main(int argc, char **argv) {
    if (argc != 3) return 2;
    unsigned n = (unsigned)strtoul(argv[2],0,10);
    long a=1;
    uint64_t start=ns();
    if (argv[1][0]=='o') for(unsigned i=0;i<n;i++) a=original((struct Big){a,2,3});
    else for(unsigned i=0;i<n;i++) a=optimized((struct Big){a,2,3});
    uint64_t elapsed=ns()-start;
    if (a != 1+5L*n) return 3;
    printf("%llu %ld\n",(unsigned long long)elapsed,a);
    return 0;
}
'''


VECTOR_DRIVER = r'''
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
typedef float V __attribute__((vector_size(32)));
struct W { V v; int tag; };
_Static_assert(sizeof(struct W)==64, "matrix wrapper ABI");
extern V original(struct W, V, int);
extern V optimized(struct W, V, int);
static uint64_t ns(void) {
    struct timespec t;
    if (clock_gettime(CLOCK_MONOTONIC, &t)) exit(2);
    return (uint64_t)t.tv_sec*1000000000u+t.tv_nsec;
}
int main(int argc, char **argv) {
    if (argc != 3) return 2;
    unsigned n = (unsigned)strtoul(argv[2],0,10);
    V a={1,2,3,4,5,6,7,8}, b={.5,.5,.5,.5,.5,.5,.5,.5};
    uint64_t start=ns();
    if (argv[1][0]=='o') for(unsigned i=0;i<n;i++) a=original((struct W){a,7},b,0);
    else for(unsigned i=0;i<n;i++) a=optimized((struct W){a,7},b,0);
    uint64_t elapsed=ns()-start;
    uint32_t bits[8]; memcpy(bits,&a,sizeof a);
    for(unsigned i=0;i<8;i++) {
        float want=(n&1) ? .5f-(float)(i+1) : (float)(i+1);
        uint32_t w; memcpy(&w,&want,sizeof w);
        if (bits[i] != w) return 3;
    }
    printf("%llu 1\n",(unsigned long long)elapsed);
    return 0;
}
'''


def extract(path, name, kernel="sum_big"):
    lines = path.read_text().splitlines()
    start = lines.index(kernel+':')
    end = next(i for i in range(start+1, len(lines))
               if lines[i].strip().startswith('.size '+kernel+','))
    body = '\n'.join(lines[start:end+1]).replace(kernel, name)
    # A/B functions from the same source reuse basic-block label IDs.
    body = re.sub(r'\.L[\w.$]+', lambda m: m[0]+'_'+name, body)
    return f'.globl {name}\n.type {name},@function\n'+body+'\n'



def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--before', type=Path, required=True)
    parser.add_argument('--after', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--cc', default='gcc')
    parser.add_argument('--kernel', choices=['sum_big', 'cf_wrap_phi'], default='sum_big')
    parser.add_argument('--samples', type=int, default=21)
    parser.add_argument('--iterations', type=int, default=30_000_000)
    args = parser.parse_args()
    if not 3 <= args.samples <= 101 or not 1 <= args.iterations <= 1_000_000_000:
        parser.error('samples must be 3..101; iterations must be 1..1000000000')
    cpu = min(os.sched_getaffinity(0))
    os.sched_setaffinity(0, {cpu})
    vector = args.kernel == 'cf_wrap_phi'
    if vector and 'avx2' not in Path('/proc/cpuinfo').read_text().split():
        parser.error('cf_wrap_phi requires an AVX2 host')
    before, after = (extract(path, name, args.kernel) for path, name in
                     [(args.before, 'original'), (args.after, 'optimized')])
    rows = []
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        (tmp/'driver.c').write_text(VECTOR_DRIVER if vector else DRIVER)
        (tmp/'kernels.s').write_text('.text\n'+before+after+'.section .note.GNU-stack,"",@progbits\n')
        flags = ['-O2', '-Wall', '-Wextra', '-Werror'] + (['-mavx2', '-mfma'] if vector else [])
        subprocess.run([args.cc, *flags,
                        str(tmp/'driver.c'), str(tmp/'kernels.s'), '-o', str(tmp/'run')],
                       check=True, timeout=120)
        for sample in range(args.samples+1):
            row = {}
            order = ['original', 'new'] if sample % 2 == 0 else ['new', 'original']
            for mode in order:
                output = subprocess.check_output([str(tmp/'run'), mode, str(args.iterations)],
                                                 text=True, timeout=120).split()
                if len(output) != 2 or int(output[1]) != (1 if vector else 1+5*args.iterations):
                    raise RuntimeError('wrong benchmark checksum')
                row[mode] = int(output[0])
            if sample:
                rows.append(row)
    ratios = [r['original']/r['new'] for r in rows]
    median = statistics.median(ratios)
    report = dict(metric='call-inclusive dependent '+args.kernel, cpu=cpu, driver_flags=flags,
                  iterations_per_sample=args.iterations, samples_ns=rows,
                  median_before_over_after=median,
                  ratio_median_absolute_deviation=statistics.median(abs(r-median) for r in ratios),
                  before_kernel=before, after_kernel=after,
                  driver_sha256=hashlib.sha256((VECTOR_DRIVER if vector else DRIVER).encode()).hexdigest(),
                  harness_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                  before_sha256=hashlib.sha256(args.before.read_bytes()).hexdigest(),
                  after_sha256=hashlib.sha256(args.after.read_bytes()).hexdigest(),
                  compiler=subprocess.check_output([args.cc, '--version'], text=True).splitlines()[0],
                  cpu_model=next(line.split(':', 1)[1].strip() for line in
                                 Path('/proc/cpuinfo').read_text().splitlines()
                                 if line.startswith('model name')),
                  caveat='Virtualized single host; no PMU or general workload speed claim.')
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(report, indent=2)+'\n')
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()
