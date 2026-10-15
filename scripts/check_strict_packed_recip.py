#!/usr/bin/env python3
"""Bitwise and structural gate for ordered FP with packed I32 denominators."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / 'tests/regression/strict_packed_recip_bits.c'
FLAGS = ['-O2', '-march=x86-64-v3', '-fwrapv', '-ffp-contract=off']


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--lccc', default=os.environ.get('CCC', str(ROOT / 'target/fastbuild/lccc')))
    p.add_argument('--gcc', default=os.environ.get('GCC', 'gcc'))
    p.add_argument('--clang', default=os.environ.get('CLANG'))
    p.add_argument('--memcheck', action='store_true')
    p.add_argument('--json', type=Path)
    args = p.parse_args()
    report = {'complete': False, 'source_sha256': hashlib.sha256(SOURCE.read_bytes()).hexdigest(), 'arms': []}

    def run(cmd, env):
        result = subprocess.run(cmd, env=env, text=True, capture_output=True, timeout=90)
        if result.returncode:
            raise RuntimeError(f'{cmd}: exit {result.returncode}\n{result.stderr}')
        return result.stdout

    modes = [('gcc', args.gcc, {}, FLAGS)]
    if args.clang:
        modes.append(('clang', args.clang, {}, FLAGS))
    for name, env, flags in [
        ('packed', {}, FLAGS),
        ('no-xmm', {'CCC_NO_XMM_REGALLOC': '1'}, FLAGS),
        ('no-vecreg', {'CCC_NO_VECREG': '1'}, FLAGS),
        ('no-promotion', {'CCC_NO_VEX_PROMOTE': '1'}, FLAGS),
        ('scalar-denominators', {'CCC_DISABLE_PASSES': 'strict_recip_pack'}, FLAGS),
        ('scalar-loop', {'CCC_DISABLE_PASSES': 'vectorize'}, FLAGS),
        ('baseline', {}, [f for f in FLAGS if not f.startswith('-march=')] + ['-march=x86-64']),
    ]:
        modes.append((name, args.lccc, env, flags))
    try:
        with tempfile.TemporaryDirectory(prefix='strict-packed-') as td:
            directory = Path(td)
            expected = None
            for name, compiler, extra_env, flags in modes:
                env = dict(os.environ, **extra_env)
                # Exercise SSA checks in the same runtime arms, not only in a
                # separate compile whose inputs/configuration might differ.
                if compiler == args.lccc:
                    env['CCC_VALIDATE_SSA'] = '1'
                binary, assembly = directory / name, directory / (name + '.s')
                cmd = [compiler, *flags, str(SOURCE), '-o', str(binary)]
                run(cmd, env)
                output = run([str(binary)], env)
                if expected is None:
                    expected = output
                if output != expected:
                    raise RuntimeError(f'{name}: bitwise mismatch against GCC')
                row = dict(name=name, compiler=run([compiler, '--version'], env), cmd=cmd,
                           env=extra_env, cases=len(output.splitlines()), stdout_sha256=hashlib.sha256(output.encode()).hexdigest())
                if args.memcheck:
                    log = directory / (name + '.memcheck')
                    checked = run(['valgrind', '--error-exitcode=97', '--leak-check=full', '--log-file=' + str(log), str(binary)], env)
                    if checked != expected:
                        raise RuntimeError(f'{name}: Memcheck output differs')
                    row['memcheck'] = log.read_text()
                if compiler == args.lccc:
                    run([compiler, *flags, '-S', str(SOURCE), '-o', str(assembly)], env)
                    text = assembly.read_text()
                    match = re.search(r'(?ms)^packed_sum:\n(.*?)^\.size packed_sum,', text)
                    if not match:
                        raise RuntimeError('missing packed_sum assembly')
                    body = match.group(1)
                    row['assembly'] = body
                    if name in ('packed', 'no-xmm', 'no-vecreg', 'no-promotion'):
                        for insn in ['pmulld', 'psrad', 'vcvtdq2pd', 'vdivpd', 'vmulpd']:
                            if insn not in body:
                                raise RuntimeError(f'{name}: missing {insn}; packing not exercised')
                        if len(re.findall(r'\bvaddsd\b', body)) < 4 or 'vaddpd' in body:
                            raise RuntimeError(f'{name}: source-ordered additions changed')
                        for operand in re.findall(r'vcvtdq2pd\s+([^,]+),', body):
                            if '%ymm' in operand:
                                raise RuntimeError(f'{name}: conversion reads more than 128 bits')
                    if name in ('packed', 'no-xmm', 'no-vecreg') and re.search(r'^\s*(?:movdqa|movdqu|movd|pmulld|paddd|psrad|pshufd)\s', body, re.M):
                        raise RuntimeError(f'{name}: legacy SIMD in the promoted AVX loop')
                    if name == 'baseline' and ('vdivpd' in body or 'vcvtdq2pd' in body):
                        raise RuntimeError('packed lowering escaped ISA gate')
                report['arms'].append(row)
                print(f'{name}: PASS {row["cases"]} bitwise cases' + (' + Memcheck' if args.memcheck else ''))
            report['complete'] = True
    finally:
        if args.json:
            args.json.parent.mkdir(parents=True, exist_ok=True)
            args.json.write_text(json.dumps(report, indent=2) + '\n')


if __name__ == '__main__':
    main()
