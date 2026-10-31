#!/usr/bin/env python3
"""Deterministic adversarial -Wdiv-by-zero condition matrix against GCC.

The constants exercise signed overflow, unsigned wrapping, mixed widths and
integer promotions. This compares warning verdicts, not generated code.
"""
import argparse
import itertools
import random
import re
import subprocess
import tempfile
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument('--lccc', required=True)
parser.add_argument('--gcc', required=True)
parser.add_argument('--verify-batching', action='store_true',
                    help='compare every batched verdict with a separate translation unit')
args = parser.parse_args()
rng = random.Random(0x6C636363)
values = ['0', '1', '2', '3u', '0xffffffffu', '2147483647', '0x80000000u',
          '-1', '-2', '0UL', '1UL', '18446744073709551615UL',
          '(unsigned char)256', '(signed char)-1']
ops = ['<', '<=', '==', '!=', '>', '>=']
cases = []
for i in range(120):
    a, b, c = (rng.choice(values) for _ in range(3))
    op = rng.choice(ops)
    if i % 3 == 0:
        condition = f'(({a}){op}({b}))'
    elif i % 3 == 1:
        condition = f'((({a})+({b})){op}({c}))'
    else:
        condition = f'((({a})-({b})){op}({c}))'
    cases.append(f'int f(void){{return ({condition}) ? (1/0) : 1;}}\n')
# A second fixed seed adds multiply/divide/remainder and skips containing
# otherwise-overflowing arithmetic. GCC must diagnose only evaluated paths.
rng = random.Random(0x21C0)
values = ['0', '1', '-1', '0u', '1u', '0xffffffffu', '2147483647',
          '-2147483647', '(unsigned char)255', '(signed char)-128',
          '0x7fffffffffffffffLL', '0xffffffffffffffffULL']
for i in range(220):
    a, b, c = (rng.choice(values) for _ in range(3))
    op, cmp = rng.choice(['+', '-', '*', '/', '%']), rng.choice(ops)
    condition = f'((({a}){op}({b})){cmp}({c}))'
    if i % 7 == 0:
        condition = f'(1 || {condition})'
    elif i % 7 == 1:
        condition = f'(0 && {condition})'
    elif i % 7 == 2:
        condition = f'(0 ? {condition} : 1)'
    elif i % 7 == 3:
        condition = f'(1 ? {condition} : 0)'
    cases.append(f'int f(void){{return ({condition}) ? (1/0) : 1;}}\n')
# A non-ICE selected arm may have a known nonzero source value which becomes
# zero after a narrowing cast. Exercise positive and negative values, both
# condition arms and GNU ?:, boolean conversion, and the negated result.
cast_types = ['unsigned char', 'signed char', 'unsigned short', 'short',
              'unsigned int', 'int', 'long', '_Bool']
cast_values = ['0', '1', '-1', '255', '256', '257', '65535', '65536',
               '-256', '4294967296LL']
cast_forms = ['(1 ? ({value}) : x)', '(0 ? x : ({value}))', '(({value}) ?: x)']
for cast_type, value, form, negate in itertools.product(
        cast_types, cast_values, cast_forms, (False, True)):
    expr = f'({cast_type})({form.format(value=value)})'
    if negate:
        expr = f'!({expr})'
    cases.append(f'int f(int x){{return x/({expr});}}\n')
# Differentiate value folding (divisors) from branch-warning suppression.
# A conditional inherits the selected arm's overflow state, not its condition's.
overflows = ['(2147483647+1)', '(2147483647+2147483647)', '(-2147483647-2)',
             '(100000*100000)', '(-2147483647-1)/-1']
overflow_cases = []
for ov in overflows:
    conditions = [ov, f'({ov}==0)', f'({ov}!=0)', f'({ov}?0:0)',
                  f'({ov}?0:1)', f'({ov}?1:0)', f'({ov}?1:1)',
                  f'(0?{ov}:0)', f'(1?{ov}:0)', f'({ov}&&0)',
                  f'({ov}||1)', f'(0&&{ov})', f'(1||{ov})']
    for cond in conditions:
        for expression in (f'({cond}) ? 1/0 : 1', f'({cond}) ? 1 : 1/0',
                           f'x/({cond})'):
            overflow_cases.append(f'int f(int x){{return {expression};}}\n')
runs = [(cases, ()), (overflow_cases, ()), (overflow_cases, ('-fwrapv',))]
def diagnostic_verdicts(path, count, status, stderr):
    """Extract one warning verdict per source line, failing closed on anomalies.

    Chunks are capped well below LCCC's 200-error rendering limit. Multiple
    division errors on a single line still produce the same verdict as an
    isolated translation unit. Any other error, crash or truncated diagnostic
    stream is a harness failure, never a passing comparison.
    """
    if status not in (0, 1) or any(marker in stderr.lower() for marker in (
            'too many errors', 'internal compiler error', 'panicked at', 'fatal error')):
        raise ValueError(f'compiler crashed or diagnostics were truncated (exit {status})')
    line_pattern = re.compile(r'^' + re.escape(str(path)) + r':(\d+):\d+: error: (.*)$')
    summary_pattern = re.compile(r'^ccc: error: \d+ error\(s\) during semantic analysis$')
    verdicts = set()
    for line in stderr.splitlines():
        match = line_pattern.match(line)
        if match:
            number, message = int(match.group(1)), match.group(2)
            if not 1 <= number <= count or 'division by zero' not in message:
                raise ValueError(f'unexpected diagnostic: {line}')
            verdicts.add(number - 1)
        elif 'error:' in line and not summary_pattern.match(line):
            raise ValueError(f'unattributed error: {line}')
    if (status != 0) != bool(verdicts):
        raise ValueError('exit status and attributed division diagnostics disagree')
    return verdicts


def self_test():
    path = Path('/tmp/divzero-selftest.c')
    good = f'{path}:2:7: error: division by zero [-Werror=div-by-zero]\n'
    assert diagnostic_verdicts(path, 3, 1, good) == {1}
    assert diagnostic_verdicts(path, 3, 0, '') == set()
    bad = [
        (1, ''), (0, good), (-9, good), (2, good),
        (1, good + 'too many errors emitted\n'),
        (1, f'{path}:9:7: error: division by zero\n'),
        (1, f'{path}:1:7: error: undeclared identifier\n'),
        (1, '/other/file.c:1:2: error: division by zero\n'),
        (1, good + 'thread panicked at compiler.rs\n'),
    ]
    for status, stderr in bad:
        try:
            diagnostic_verdicts(path, 3, status, stderr)
        except ValueError:
            continue
        raise AssertionError(f'diagnostic parser accepted invalid stream: {status}, {stderr!r}')


def run(compiler, bits, flags, src, count):
    command = [compiler, '-std=gnu11', f'-m{bits}', *flags,
               '-Werror=div-by-zero', '-fsyntax-only', str(src)]
    result = subprocess.run(command, capture_output=True, text=True, timeout=20)
    if result.stdout.strip():
        raise SystemExit(f'{compiler}: unexpected stdout: {result.stdout[:1000]}')
    try:
        return diagnostic_verdicts(src, count, result.returncode, result.stderr)
    except ValueError as error:
        raise SystemExit(f'{compiler} -m{bits} {flags}: {error}\n{result.stderr[:4000]}') from error


self_test()
with tempfile.TemporaryDirectory(prefix='lccc-divzero-matrix-') as tmp:
    src = Path(tmp) / 'case.c'
    for bits, (group, extra_flags) in itertools.product((32, 64), runs):
        for start in range(0, len(group), 40):
            chunk = group[start:start + 40]
            # Each distinct function occupies exactly one physical line, so
            # its error location directly identifies the corresponding case.
            unique = []
            for index, source in enumerate(chunk):
                if source.count('\n') != 1 or source.count('int f(') != 1:
                    raise SystemExit(f'malformed matrix case {start + index}: {source!r}')
                unique.append(source.replace('int f(', f'int f_{index}(', 1))
            src.write_text(''.join(unique))
            results = [run(cc, bits, extra_flags, src, len(chunk))
                       for cc in (args.gcc, args.lccc)]
            if results[0] != results[1]:
                bad = sorted(results[0] ^ results[1])
                details = ''.join(f'  case {start + i}: {chunk[i]}' for i in bad[:10])
                raise SystemExit(f'div-zero matrix mismatch -m{bits} {extra_flags}:\n{details}')
            # During validation, compare every batch verdict with the old
            # one-translation-unit-per-case semantics for both compilers.
            if args.verify_batching:
                for index, source in enumerate(chunk):
                    src.write_text(source)
                    for cc, verdicts in zip((args.gcc, args.lccc), results):
                        isolated = bool(run(cc, bits, extra_flags, src, 1))
                        if isolated != (index in verdicts):
                            raise SystemExit(f'batch/isolation mismatch for {cc} -m{bits} '
                                             f'{extra_flags} case {start + index}: {source}')
print(f'div-zero constant matrix: PASS ({sum(len(group) for group, _ in runs) * 2} '
      'GCC comparisons; fail-closed batch parser self-test)')
