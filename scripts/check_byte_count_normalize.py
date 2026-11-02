#!/usr/bin/env python3
"""Execute byte-count offsets/tails/wraps with tight heap buffers and ISA controls.

Native assertions and vendor output comparisons are correctness checks. The
packed-op check only establishes that the optimization was exercised; it is not
performance evidence. Use callgrind_corpus.py for paired instruction counts.
"""
from __future__ import annotations
import argparse
import hashlib
import json
import os
import re
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / 'tests/optimizer/byte_count_normalize.c'


def main():
    p = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    p.add_argument('--lccc', type=Path, required=True)
    p.add_argument('--gcc', default='gcc-16')
    p.add_argument('--clang')
    p.add_argument('--memcheck', action='store_true')
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--json', type=Path, required=True)
    a = p.parse_args()
    a.output.mkdir(parents=True, exist_ok=True)
    a.json.parent.mkdir(parents=True, exist_ok=True)
    report = {'complete': False, 'source_sha256': hashlib.sha256(SOURCE.read_bytes()).hexdigest(),
              'candidate_sha256': hashlib.sha256(a.lccc.resolve().read_bytes()).hexdigest(), 'rows': []}
    def save():
        temp = a.json.with_suffix('.tmp')
        temp.write_text(json.dumps(report, indent=2) + '\n')
        temp.replace(a.json)
    def run(cmd, env):
        result = subprocess.run([str(x) for x in cmd], env=env, text=True, capture_output=True, timeout=120)
        if result.returncode:
            raise RuntimeError(f'{cmd}: exit {result.returncode}\n{result.stdout}\n{result.stderr}')
        return result.stdout
    save()
    common = ['-O2', '-march=x86-64-v3', '-fwrapv']
    cc = str(a.lccc.resolve())
    modes = [('gcc', a.gcc, {}, common)]
    if a.clang:
        modes.append(('clang', a.clang, {}, common))
    modes.extend([
        ('normalized', cc, {}, common),
        ('disabled', cc, {'CCC_DISABLE_PASSES': 'byte_count_normalize'}, common),
        ('baseline-isa', cc, {}, ['-O2', '-march=x86-64', '-fwrapv']),
        ('disabled-baseline-isa', cc, {'CCC_DISABLE_PASSES': 'byte_count_normalize'},
         ['-O2', '-march=x86-64', '-fwrapv']),
        ('no-avx', cc, {}, [*common, '-mno-avx']),
        ('O3', cc, {}, ['-O3', '-march=x86-64-v3', '-fwrapv']),
    ])
    expected = None
    try:
        for label, compiler, extra, flags in modes:
            env = {k:v for k,v in os.environ.items() if not k.startswith(('CCC_', 'LCCC_DEBUG_'))}
            env.update(extra)
            if compiler == cc:
                env['CCC_VALIDATE_SSA'] = '1'
            binary, assembly = a.output / label, a.output / (label+'.s')
            binary.unlink(missing_ok=True)
            assembly.unlink(missing_ok=True)
            cmd = [compiler, *flags, SOURCE, '-o', binary]
            run(cmd, env)
            output = run([binary], env)
            if expected is None:
                expected = output
            if not output.startswith('byte-count boundaries: ') or output != expected:
                raise ValueError('output mismatch: '+label)
            run([compiler, *flags, SOURCE, '-S', '-o', assembly], env)
            text = assembly.read_text()
            # AVX2 counts with VEX `vpsadbw` on ymm; below the AVX ceiling the
            # same reduction is the legacy-SSE2 `psadbw` on xmm (16 lanes).
            packed = 'vpsadbw' in text
            legacy = re.compile(r'(?m)^\s*psadbw\s')
            focused = focused_sse2 = None
            if compiler == cc:
                begin = re.search(r'(?m)^count_2_4097:\s*$', text)
                end = re.search(r'(?m)^\s*\.size\s+count_2_4097\s*,', text)
                if begin is None or end is None or end.start() <= begin.end():
                    raise ValueError('missing focused count function: '+label)
                body = text[begin.end():end.start()]
                focused = 'vpsadbw' in body
                focused_sse2 = legacy.search(body) is not None
            if label == 'normalized' and not focused:
                raise ValueError('positive arm did not exercise normalized packed counting')
            if label in ('disabled', 'disabled-baseline-isa') and (focused or focused_sse2):
                raise ValueError('normalization kill switch did not preserve the scalar witness')
            if label in ('baseline-isa', 'disabled-baseline-isa', 'no-avx') and packed:
                raise ValueError('AVX2 instruction escaped the ISA ceiling')
            if label in ('baseline-isa', 'no-avx') and not focused_sse2:
                raise ValueError('SSE2 arm did not exercise normalized psadbw counting: '+label)
            row = {'arm': label, 'command': [str(x) for x in cmd], 'environment': extra,
                   'compiler_version': run([compiler, '--version'], env), 'stdout': output,
                   'packed_count_present': packed, 'focused_count_packed': focused,
                   'focused_count_packed_sse2': focused_sse2, 'assembly_sha256': hashlib.sha256(assembly.read_bytes()).hexdigest()}
            if a.memcheck:
                log = a.output / (label+'.memcheck')
                checked = run(['valgrind', '--error-exitcode=97', '--leak-check=full', f'--log-file={log}', binary], env)
                if checked != expected:
                    raise ValueError('Memcheck output mismatch: '+label)
                row['memcheck'] = log.read_text()
            report['rows'].append(row)
            save()
            print(label+': PASS'+(' + Memcheck' if a.memcheck else ''), flush=True)
        report['complete'] = True
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        report['error'] = str(error)
        print('FAIL:', error)
    save()
    return int(not report['complete'])


if __name__ == '__main__':
    raise SystemExit(main())
