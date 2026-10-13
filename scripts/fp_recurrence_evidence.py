#!/usr/bin/env python3
"""Reproduce four-arm bounded Callgrind and benchmark .text/pass-time evidence.

Not a CI runner. Requires active swap and already-built compilers. Records
incomplete results on interruption; never labels simulated Ir as runtime.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shlex
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools.corpus import publication


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--baseline', type=Path, required=True)
    ap.add_argument('--candidate', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    args = ap.parse_args()
    if len(Path('/proc/swaps').read_text().splitlines()) < 2:
        ap.error('active swap is required for this bounded-memory experiment')
    baseline, candidate = args.baseline.resolve(), args.candidate.resolve()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    env = os.environ.copy()
    for key in ['CCC_DISABLE_PASSES', 'CCC_PEEPHOLE_SKIP', 'CCC_TIME_PASSES']:
        env.pop(key, None)
    result = {'complete': False, 'metric': 'simulated Ir; not cycles or speedup',
              'compiler_sha256': {'baseline': sha(baseline), 'candidate': sha(candidate)},
              'swap': Path('/proc/swaps').read_text(), 'callgrind': {}, 'sizes': []}

    def save():
        publication.atomic_bytes(out / 'summary.json', (json.dumps(result, indent=2)+'\n').encode())

    def run(command, log, extra=None):
        current = env | (extra or {})
        start = time.perf_counter()
        with log.open('w') as f:
            subprocess.run(list(map(str, command)), cwd=ROOT, env=current,
                           stdout=f, stderr=subprocess.STDOUT, check=True)
        return time.perf_counter() - start

    with publication.lock(out / '.evidence.lock'):
        save()
        for arm, variable in [('on', None), ('no_square', 'CCC_DISABLE_PASSES=loop_square'),
                              ('no_double', 'CCC_DISABLE_PASSES=fp_double'),
                              ('no_jump', 'CCC_PEEPHOLE_SKIP=jump_thread')]:
            mine = shlex.join(['env', variable, str(candidate)]) if variable else str(candidate)
            ref = str(baseline if arm == 'on' else candidate)
            directory = out / arm
            run([sys.executable, ROOT/'scripts/callgrind_corpus.py', '--mine', mine,
                 '--ref', ref, '--out', directory], out/(arm+'.log'))
            index = json.loads((directory/'index.json').read_text())
            assert index['complete'] and not index['failures'], index
            result['callgrind'][arm] = index
            save()
            print('PASS full bounded corpus:', arm, len(index['results']), flush=True)
        sizes = out / 'sizes'
        sizes.mkdir(exist_ok=True)
        for source in sorted((ROOT/'tests/benchmark/programs').glob('*.c')):
            for opt in ['-O2', '-Os']:
                row = {'source': source.name, 'source_sha256': sha(source), 'opt': opt}
                for name, compiler in [('baseline', baseline), ('candidate', candidate)]:
                    stem = f'{source.stem}-{opt[1:]}-{name}'
                    obj, text = sizes/(stem+'.o'), sizes/(stem+'.text')
                    command = [compiler, opt, '-march=x86-64-v3', '-c', source, '-o', obj]
                    elapsed = run(command, sizes/(stem+'.log'), {'CCC_TIME_PASSES': '1'})
                    subprocess.run(['objcopy', '-O', 'binary', '--only-section=.text',
                                    str(obj), str(text)], check=True)
                    row[name] = {'compile_seconds': elapsed, 'text_bytes': text.stat().st_size, 'text_sha256': sha(text),
                                 'command': list(map(str, command)), 'timings': stem+'.log'}
                result['sizes'].append(row)
                save()
        result['complete'] = True
        save()
        print('PASS .text and pass-time census:', len(result['sizes']), flush=True)


if __name__ == '__main__':
    main()
