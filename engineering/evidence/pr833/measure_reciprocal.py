#!/usr/bin/env python3
"""Reproduce the isolated S48/S49 matmul comparison from verified saved ELFs.

The two build directories must be restored under --research. This deliberately
uses the SAME fresh-child driver as the all45 runner, not a second C benchmark.
No compiler executable is needed: complete timed executable hashes are checked.
"""
import argparse
import json
from pathlib import Path
import random
import statistics
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'scripts'))
from benchmark_native_batched import bench, calibrated_count, execute, publish, sha


def main():
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--research',type=Path,required=True)
    ap.add_argument('--out',type=Path,required=True)
    args=ap.parse_args();args.out.mkdir(parents=True,exist_ok=True)
    paths={'s48':args.research/'unfused-focused','s49':args.research/'native-S49'}
    manifests={k:json.loads((p/'build.json').read_text()) for k,p in paths.items()}
    for m in manifests.values():
        if not m['complete']:raise ValueError('incomplete build')
    a,b=[m['identity'] for m in manifests.values()]
    for key in ['flags','driver_sha256','object_protocol_sha256','header_provider']:
        if a[key]!=b[key]:raise ValueError('experiment mismatch: '+key)
    if a['sources']['matmul']!=b['sources']['matmul']:raise ValueError('source mismatch')
    programs={k:p/'matmul/lccc/program' for k,p in paths.items()}
    expected={k:(p/'matmul/lccc/expected.stdout').read_text() for k,p in paths.items()}
    if len(set(expected.values()))!=1:raise ValueError('original output mismatch')
    for k,p in programs.items():
        if sha(p)!=manifests[k]['binaries']['matmul']['lccc']:raise ValueError('executable mismatch')
    affinity,pinning,warnings=bench.choose_affinity('0')
    report=dict(complete=False,source=a['sources']['matmul'],flags=a['flags'],pinning=pinning,warnings=warnings,
                build_sha256={k:sha(p/'build.json') for k,p in paths.items()},
                program_sha256={k:sha(p) for k,p in programs.items()},
                driver_sha256=a['driver_sha256'],samples=[],metric='fresh-child entry time, lower s49/s48 is better')
    def sample(k,n,label):
        data=execute([*affinity,programs[k],str(n)],args.out/f'{k}-{label}.json')
        if data['stdout']!=expected[k]*n:raise ValueError('stdout mismatch')
        prefix='BATCH_RESULT '
        if not data['stderr'].startswith(prefix):raise ValueError('missing driver result')
        value=json.loads(data['stderr'][len(prefix):])
        if value['iterations']!=n or value['ns']<=0:raise ValueError('invalid driver result')
        return value
    try:
        cal={k:sample(k,5,'calibration') for k in programs}
        count=calibrated_count(cal,.2,1.)
        report.update(calibration=cal,iterations=count)
        rng=random.Random(8334948)
        for r in range(-1,7):
            order=list(programs);rng.shuffle(order)
            values={k:sample(k,count,f'round{r}') for k in order}
            if r>=0:report['samples'].append(dict(order=order,values=values))
        ratios=[r['values']['s49']['ns']/r['values']['s48']['ns'] for r in report['samples']]
        report.update(complete=True,ratios=ratios,median=statistics.median(ratios),
                      exploratory_ci95=bench.bootstrap_median_ci(ratios,8334948))
        print(report['median'],report['exploratory_ci95'])
    finally:publish(args.out/'results.json',report)


if __name__=='__main__':main()
