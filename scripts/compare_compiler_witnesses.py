#!/usr/bin/env python3
"""Bounded native semantic comparison of explicitly named compiler candidates.

This is correctness evidence, never a timing ranking. Every candidate uses the
same current-tree fixtures and semantic flags as an independent GCC -O0
control. Candidate optimization levels are explicit; control optimization is
always -O0, not a performance comparison. No self-check is disabled. Historical
candidates may fail; --require identifies the arms whose failures fail the job.
Missing tools, control failures, timeouts and missing executables are failures,
not successful skips. Full command/output artifacts accompany the compact index.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools.corpus import publication


def cases():
    for opt in ('-O0', '-O1', '-O2', '-O3', '-Os'):
        for name, flags in (
            ('macro_literal_payload', ()),
            ('macro_token_boundary_stringification', ()),
            ('enumerator_attribute_aliases', ()),
            ('wrapv_byte_address', ('-fwrapv',)),
            ('readonly_reduction_guard', ('-fwrapv', '-march=x86-64-v3')),
            ('fenv_constant_rounding', ('-frounding-math', '-lm')),
            ('fenv_rounding_freestanding', ('-frounding-math',)),
            ('strict_recip_traps', ('-ftrapping-math', '-ffp-contract=off', '-march=x86-64-v3', '-lm')),
            ('strict_packed_recip_bits', ('-fwrapv', '-ffp-contract=off', '-march=x86-64-v3')),
        ):
            yield name + '/' + opt[1:], name, (opt, *flags), ()
        for which in range(4):
            yield f'wrapv_domains/{opt[1:]}/{which}', 'wrapv_domains', (opt, '-fwrapv'), (str(which),)


def execute(command, directory, environment, timeout):
    try:
        run = subprocess.run(command, capture_output=True, env=environment, timeout=timeout)
        status, stdout, stderr = run.returncode, run.stdout, run.stderr
    except subprocess.TimeoutExpired as error:
        status = 'error'
        stdout = error.stdout or b''
        stderr = (error.stderr or b'') + b'\n' + str(error).encode()
    except OSError as error:
        status, stdout, stderr = 'error', b'', str(error).encode()
    directory.mkdir(parents=True, exist_ok=True)
    (directory / 'stdout').write_bytes(stdout)
    (directory / 'stderr').write_bytes(stderr)
    return {'command': [str(value) for value in command], 'status': status,
            'stdout_sha256': hashlib.sha256(stdout).hexdigest(),
            'stderr_sha256': hashlib.sha256(stderr).hexdigest(),
            'stdout_bytes': len(stdout), 'stderr_tail': stderr[-1500:].decode(errors='replace')}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument('--compiler', action='append', required=True, metavar='LABEL=EXECUTABLE')
    parser.add_argument('--require', action='append', required=True, help='candidate that must pass every case')
    parser.add_argument('--gcc', default='gcc-16', help='independent native control and builtin-header provider')
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args(argv)
    compilers = {'gcc': shutil.which(args.gcc)}
    for item in args.compiler:
        label, sep, executable = item.partition('=')
        if not sep or not re.fullmatch('[a-z][a-z0-9_-]*', label) or label in compilers:
            parser.error('invalid/repeated candidate: ' + item)
        compilers[label] = shutil.which(executable)
    if set(args.require) - compilers.keys():
        parser.error('--require names an unregistered compiler')
    args.out.mkdir(parents=True, exist_ok=True)
    report = {'complete': False, 'metric': 'native correctness; NOT performance',
              'control_policy': 'independent GCC at -O0; identical language, FP and target flags; candidates retain their requested optimization levels',
              'compilers': {}, 'cases': [], 'summary': {}, 'required': args.require}

    def save():
        publication.atomic_bytes(args.out / 'index.json', (json.dumps(report, indent=2) + '\n').encode())

    with publication.lock(args.out / '.comparison.lock'):
        for label, executable in compilers.items():
            report['compilers'][label] = {'path': executable, 'sha256':
                hashlib.sha256(Path(executable).read_bytes()).hexdigest() if executable else None}
        save()
        if not all(compilers.values()):
            report['error'] = 'missing compiler; no semantic comparison is admissible'
            save()
            return 1
        try:
            header = subprocess.run([compilers['gcc'], '-print-file-name=include'],
                                    capture_output=True, text=True, timeout=10)
        except (OSError, subprocess.TimeoutExpired) as error:
            report['error'] = 'GCC builtin-header discovery failed: ' + str(error)
            save()
            return 1
        include = Path(header.stdout.strip())
        if header.returncode or not include.is_dir():
            report['error'] = 'GCC builtin-header discovery failed'
            save()
            return 1
        environment = {key: value for key, value in os.environ.items() if not key.startswith('CCC_')}
        environment.update(LC_ALL='C', CCC_VALIDATE_SSA='1')
        for case, fixture, flags, parameters in cases():
            source = ROOT / 'tests/regression' / (fixture + '.c')
            control_flags = ('-O0', *(flag for flag in flags if not flag.startswith('-O')))
            row = {'id': case, 'source': str(source.relative_to(ROOT)),
                   'source_sha256': hashlib.sha256(source.read_bytes()).hexdigest(),
                   'flags': flags, 'gcc_control_flags': control_flags,
                   'arguments': parameters, 'arms': {}}
            for label, executable in compilers.items():
                directory = (args.out / case / label).resolve()
                directory.mkdir(parents=True, exist_ok=True)
                binary = directory / 'witness'
                binary.unlink(missing_ok=True)
                selected_flags = control_flags if label == 'gcc' else flags
                libraries = [flag for flag in selected_flags if flag.startswith('-l')]
                frontend = [flag for flag in selected_flags if not flag.startswith('-l')]
                build = execute([executable, '-I' + str(include), *frontend, str(source),
                                 *libraries, '-o', str(binary)], directory / 'build', environment, 40)
                arm = {'build': build, 'pass': False}
                if build['status'] == 0 and binary.is_file() and os.access(binary, os.X_OK):
                    arm['binary_sha256'] = hashlib.sha256(binary.read_bytes()).hexdigest()
                    arm['run'] = execute([str(binary), *parameters], directory / 'run', environment, 5)
                row['arms'][label] = arm
                binary.unlink(missing_ok=True)
            control = row['arms']['gcc'].get('run', {})
            for arm in row['arms'].values():
                result = arm.get('run', {})
                arm['pass'] = bool(control.get('status') == 0 and result.get('status') == 0
                                   and result['stdout_sha256'] == control['stdout_sha256'])
            report['cases'].append(row)
            save()
        report['summary'] = {label: {'pass': sum(row['arms'][label]['pass'] for row in report['cases']),
                                    'fail': sum(not row['arms'][label]['pass'] for row in report['cases'])}
                             for label in compilers}
        report['complete'] = True
        save()
    print(json.dumps(report['summary'], sort_keys=True))
    return int(any(report['summary'][label]['fail'] for label in ['gcc', *args.require]))


if __name__ == '__main__':
    raise SystemExit(main())
