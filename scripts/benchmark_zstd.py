#!/usr/bin/env python3
"""Public-API zstd benchmark: same driver, real libraries, checked paired rounds.

Build each complete upstream static library with its compiler (two jobs), then
link the identical GCC-built driver object to each library. Assembly remains
ENABLED. Single-threaded static-library defaults are unmodified. Decompression
uses the SAME GCC-produced frame in every arm. All compression frames must be
cross-decoded by GCC; all decoded bytes must match input. Timed samples include
only API calls on resident data with reused contexts, excluding I/O and setup.

Inputs are supplied real corpus files; sizes are explicit prefix lengths (0 =
whole file). Do not present bounded Callgrind inputs as full-file timing. Native
rounds use common calibrated iteration counts, paired randomized execution and
excluded warm-up. Reports retain commands, versions, hashes and raw samples.
"""
from __future__ import annotations
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import random
import shutil
import statistics
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'tests/benchmark'))
import run_benchmarks as bench
sys.path.insert(0, str(ROOT / 'scripts'))
import callgrind_ab


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def write(path, value):
    tmp = Path(str(path) + '.tmp')
    tmp.write_text(json.dumps(value, indent=2) + '\n')
    os.replace(tmp, path)


def run(cmd, log, timeout=600):
    begin = time.monotonic()
    p = subprocess.run([str(x) for x in cmd], text=True, capture_output=True, timeout=timeout)
    record = dict(command=[str(x) for x in cmd], returncode=p.returncode,
                  wall_seconds=time.monotonic()-begin, stdout=p.stdout, stderr=p.stderr)
    write(log, record)
    if p.returncode:
        raise RuntimeError(f'command failed ({p.returncode}): {log}')
    return record


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--zstd', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--lccc', type=Path, required=True)
    ap.add_argument('--gcc', default='gcc-16')
    ap.add_argument('--clang', default='clang-23')
    ap.add_argument('--input', action='append', type=Path, required=True)
    ap.add_argument('--sizes', default='65536,1048576,0')
    ap.add_argument('--levels', default='1,3,9')
    ap.add_argument('--rounds', type=int, default=7)
    ap.add_argument('--seconds', type=float, default=.15)
    ap.add_argument('--cpu', default='auto')
    ap.add_argument('--profile', action='store_true', help='Memcheck and bounded Callgrind (smallest size only)')
    ap.add_argument('--build-only', action='store_true')
    ap.add_argument('--reuse-build', action='store_true')
    ap.add_argument('--baseline-driver', type=Path)
    ap.add_argument('--baseline-manifest', type=Path)
    args = ap.parse_args()
    if not __debug__:
        ap.error('assertions required')
    if bool(args.baseline_driver) != bool(args.baseline_manifest):
        ap.error('baseline driver requires its build manifest')
    sizes = list(dict.fromkeys(int(s) for s in args.sizes.split(',')))
    levels = list(dict.fromkeys(int(s) for s in args.levels.split(',')))
    if any(s < 0 for s in sizes) or any(l < 1 or l > 22 for l in levels):
        ap.error('invalid size or compression level')
    if args.profile and not any(s > 0 for s in sizes):
        ap.error('profiling requires an explicit bounded size')
    if args.rounds < 3 or not math.isfinite(args.seconds) or args.seconds <= 0:
        ap.error('require at least three rounds and positive finite duration')
    out = args.out.resolve(); out.mkdir(parents=True, exist_ok=True)
    source = args.zstd.resolve()
    compilers = dict(lccc=str(args.lccc.resolve()), gcc=shutil.which(args.gcc), clang=shutil.which(args.clang))
    if not all(compilers.values()):
        ap.error('requested compiler unavailable')
    driver = ROOT / 'tests/benchmark/zstd_driver.c'
    flags = '-O3 -march=x86-64-v3 -fno-lto'
    sources = {str(p.relative_to(source)): sha(p) for p in sorted(source.rglob('*'))
               if p.is_file() and p.suffix in {'.c', '.h', '.S'} and 'obj' not in p.parts}
    identity = dict(sources=sources, driver_sha256=sha(driver), flags=flags,
                    compiler_environment={k:v for k,v in os.environ.items() if k.startswith(('CCC_', 'LCCC_'))},
                    compilers={k: dict(path=v, sha256=sha(v), version=subprocess.check_output([v, '--version'], text=True))
                               for k,v in compilers.items()})
    binaries = {k: out/k/'zstd-driver' for k in compilers}
    if args.reuse_build:
        prior = json.loads((out/'build.json').read_text())
        assert prior['identity'] == identity, 'source/compiler/flags drift: rebuild required'
        assert prior['complete']
        assert all(sha(p) == prior['binaries'][k] for k,p in binaries.items())
    else:
        write(out/'build.json', dict(complete=False, identity=identity))
        run([compilers['gcc'], '-O2', '-I'+str(source/'lib'), '-c', driver, '-o', out/'driver.o'], out/'driver-build.json')
        for key, cc in compilers.items():
            directory = out/key
            if directory.exists(): shutil.rmtree(directory)
            directory.mkdir()
            shutil.copytree(source/'lib', directory/'lib', ignore=shutil.ignore_patterns('obj','*.o','*.a','*.so*'))
            run(['make', '-C', directory/'lib', '-j2', 'libzstd.a', 'CC='+cc, 'CFLAGS='+flags], out/(key+'-build.json'), timeout=1800)
            run([compilers['gcc'], out/'driver.o', directory/'lib/libzstd.a', '-pthread', '-no-pie', '-o', binaries[key]], out/(key+'-link.json'))
        write(out/'build.json', dict(complete=True, identity=identity, binaries={k:sha(p) for k,p in binaries.items()},
                                    libraries={k:sha(out/k/'lib/libzstd.a') for k in binaries}))
    if args.build_only: return 0
    baseline_identity = None
    if args.baseline_driver:
        baseline = json.loads(args.baseline_manifest.read_text())
        if not baseline['complete'] or sha(args.baseline_driver) != baseline['binaries']['lccc']:
            raise ValueError('unverified baseline driver')
        for field in ('sources', 'driver_sha256', 'flags'):
            if baseline['identity'][field] != identity[field]:
                raise ValueError('baseline workload/driver/flags mismatch: ' + field)
        if sha(out/'driver.o') != sha(args.baseline_manifest.parent/'driver.o'):
            raise ValueError('baseline must use the byte-identical common driver object')
        binaries['baseline'] = args.baseline_driver.resolve()
        baseline_identity = dict(driver_sha256=sha(args.baseline_driver),
                                 manifest_sha256=sha(args.baseline_manifest),
                                 compiler=baseline['identity']['compilers']['lccc'],
                                 compiler_environment=baseline['identity'].get('compiler_environment'))
    affinity, host, warnings = bench.choose_affinity(args.cpu)
    report = dict(complete=False, identity=identity, baseline=baseline_identity, host=host, affinity=affinity,
                  warnings=warnings, callgrind_model=dict(I1='32768,8,64',D1='32768,8,64',LL='33554432,16,64',scope='common simulation, not hardware counters'), rounds=args.rounds, seconds=args.seconds, rows=[], failures=[])
    write(out/'results.json',report)
    case_root=out/'cases'; case_root.mkdir(exist_ok=True)
    counter=0
    for original in args.input:
        raw=original.read_bytes(); assert raw, original
        for size in sizes:
            if size and size>len(raw): continue
            data=raw if size==0 else raw[:size]
            for level in levels:
                counter+=1
                directory=case_root/f'{counter:03d}-{original.name}-{len(data)}-L{level}'
                directory.mkdir(exist_ok=True)
                input_path=directory/'input'; input_path.write_bytes(data)
                reference=directory/'reference.zst'
                def command(key, mode, count, output='-'):
                    return [*affinity,str(binaries[key]),str(input_path),str(reference),str(output),str(level),mode,str(count)]
                run(command('gcc','c',1,reference),directory/'reference.json')
                for key in binaries:
                    frame=directory/(key+'.zst')
                    run(command(key,'c',1,frame),directory/(key+'-verify-c.json'))
                    cross=[*affinity,str(binaries['gcc']),str(input_path),str(frame),'-',str(level),'d','1']
                    run(cross,directory/(key+'-cross-decode.json'))
                    run(command(key,'d',1),directory/(key+'-verify-d.json'))
                for mode in ['c','d']:
                    probe=json.loads(run(command('gcc',mode,1),directory/(mode+'-calibration.json'))['stdout'])
                    count=max(1,min(1000000,math.ceil(args.seconds*1e9/max(1,probe['ns']))))
                    row=dict(input_name=original.name, original_sha256=sha(original), input_sha256=sha(input_path),
                             bytes=len(data), requested_size=size, level=level, mode=mode, iterations=count,
                             samples=[], comparisons={}, frames={k:sha(directory/(k+'.zst')) for k in binaries})
                    rng=random.Random(833+counter*101+(mode=='d'))
                    for round_id in range(-1,args.rounds):
                        order=list(binaries);rng.shuffle(order);samples={}
                        for key in order:
                            result=run(command(key,mode,count),directory/f'{mode}-round{round_id}-{key}.json')
                            sample=json.loads(result['stdout']);assert sample['input_bytes']==len(data) and sample['iterations']==count
                            samples[key]=sample
                        if round_id>=0:row['samples'].append(dict(order=order,values=samples))
                    for ref in ['gcc','clang',*(['baseline'] if args.baseline_driver else [])]:
                        ratios=[r['values']['lccc']['ns']/r['values'][ref]['ns'] for r in row['samples']]
                        row['comparisons'][ref]=dict(median=statistics.median(ratios),
                            exploratory_ci95=bench.bootstrap_median_ci(ratios,833+counter),paired_ratios=ratios)
                    if args.profile and len(data)==min(s for s in sizes if s>0):
                        row['instrumentation']={}
                        for key in binaries:
                            cg=directory/f'{mode}-{key}.cg'
                            run([*affinity,'valgrind','--tool=callgrind','--instr-atstart=no','--branch-sim=yes','--cache-sim=yes','--I1=32768,8,64','--D1=32768,8,64','--LL=33554432,16,64',
                                 '--callgrind-out-file='+str(cg),*command(key,mode,1)[len(affinity):]],directory/f'{mode}-{key}-callgrind.json')
                            run([*affinity,'valgrind','--error-exitcode=97','--leak-check=full',*command(key,mode,1)[len(affinity):]],directory/f'{mode}-{key}-memcheck.json')
                            events=callgrind_ab.parse_region_totals(cg)
                            assert events.get('Ir',0)>0, 'empty profiling region'
                            row['instrumentation'][key]=dict(events=events,sha256=sha(cg))
                    report['rows'].append(row);write(out/'results.json',report)
                    print(original.name,len(data),level,mode,{k:v['median'] for k,v in row['comparisons'].items()},flush=True)
    report['complete']=True;write(out/'results.json',report)
    return 0


if __name__=='__main__':
    raise SystemExit(main())
