#!/usr/bin/env python3
"""Recheck the fifteen measured candidate binaries and eight boundary fixtures."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[3]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--corpus', type=Path, required=True)
parser.add_argument('--compiler', type=Path, required=True)
parser.add_argument('--out', type=Path, required=True)
args = parser.parse_args()
args.out.mkdir(parents=True, exist_ok=True)
report = {'complete': False, 'results': [], 'errors': []}

def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def save():
    tmp = args.out / 'summary.tmp'
    tmp.write_text(json.dumps(report, indent=2) + '\n')
    tmp.replace(args.out / 'summary.json')

def check(name, binary, expected):
    log = args.out / (name + '.log')
    command = ['valgrind', '--tool=memcheck', '--error-exitcode=99', '--leak-check=full', '--errors-for-leak-kinds=definite,indirect', '--log-file=' + str(log), str(binary)]
    result = subprocess.run(command, capture_output=True, timeout=120)
    ok = result.returncode == 0 and hashlib.sha256(result.stdout).hexdigest() == expected
    report['results'].append({'name': name, 'command': command, 'binary_sha256': digest(binary), 'ok': ok, 'returncode': result.returncode, 'log': log.read_text(), 'stderr': result.stderr.decode(errors='replace')})
    if not ok:
        report['errors'].append(name)
    save()
    print(name, 'PASS' if ok else 'FAIL', flush=True)

save()
try:
    names = json.loads((ROOT / 'engineering/evidence/pr824-redteam/requested-worst15.json').read_text())['cases']
    for item in names:
        directory = args.corpus / item['name']
        manifest = json.loads((directory / 'manifest.json').read_text())
        if manifest['failures'] or manifest['comparable'] != 1:
            raise ValueError('corpus measurement was not comparable')
        stem, row = next(iter(manifest['results'].items()))
        binary = directory / 'aa' / stem
        if digest(binary) != row['binary_sha256']['mine']:
            raise ValueError('measured executable changed')
        check(item['name'], binary, row['correctness']['mine']['stdout_sha256'])
    fixtures = [('byte-bounds', 'byte_accessor_inline.c', b''),
                ('unsigned-affine', 'unsigned_affine_loop.c', b'unsigned-affine 33554432\n')]
    for label, source, expected in fixtures:
        fixture = ROOT / 'tests/regression' / source
        for mode in ('-m64', '-m32'):
            for opt in ('-O2', '-O3'):
                name = label + mode + opt
                binary = args.out / name
                command = [str(args.compiler.resolve()), mode, opt, str(fixture), '-o', str(binary)]
                subprocess.run(command, check=True, capture_output=True, timeout=60)
                check(name, binary, hashlib.sha256(expected).hexdigest())
    report['complete'] = True
except (OSError, ValueError, subprocess.SubprocessError) as error:
    report['errors'].append(str(error))
save()
raise SystemExit(int(not report['complete'] or bool(report['errors'])))
