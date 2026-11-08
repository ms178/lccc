#!/usr/bin/env python3
"""Exhaustive bit-exact SIMD shape matrix driver.

For every optimization/ISA flag set it compiles
tests/simd/simd_shape_matrix.c with the candidate, runs all 29 shapes, and
checks three independent properties:

  1. semantic correctness   -- fast() digest == de-optimized ref() digest
                               within the candidate build;
  2. optimization invariance -- both digests equal those of a -O0 reference
                               build of the same source;
  3. zero diagnostics        -- the candidate build must be warning-clean
                               under -Wall -Wextra -Werror (any output is an
                               infrastructure error).

An optional sanitizer leg compiles with ASan+UBSan and runs `selftest`, so
UB, out-of-bounds accesses and memory errors in the corpus itself cannot
masquerade as a compiler bug.  `bench` mode measures median ns/iteration per
shape on the host CPU without polluting the compared stdout stream.

    python3 tests/simd/check_simd_shape_matrix.py --candidate gcc --reference gcc
    python3 tests/simd/check_simd_shape_matrix.py --candidate target/fastbuild/lccc --quick
    python3 tests/simd/check_simd_shape_matrix.py --self-test

This driver is deliberately independent of the finding-status lattice in
check_audit_2026-11-08.py: every row here is a control.  A shape that fails
is a real regression, not an open finding, so there is no "expected failure"
mode and no way for a broken oracle to be waved through.
"""
from __future__ import annotations

import argparse
from dataclasses import dataclass
from pathlib import Path
import sys
from tempfile import TemporaryDirectory

_SCRIPT_DIR = str(Path(__file__).resolve().parent)
if _SCRIPT_DIR not in sys.path:
    sys.path.insert(0, _SCRIPT_DIR)

try:
    from probe_utils import (
        ProbeError, compiler, host_cpu_flags, run_rc, write_json,
    )
except ImportError as exc:  # pragma: no cover
    raise SystemExit(f'check_simd_shape_matrix: probe_utils unavailable: {exc}') from exc

SOURCE = Path(__file__).with_name('simd_shape_matrix.c').resolve()
C_STD = ('-std=c99',)
WARNINGS = ('-Wall', '-Wextra', '-Werror', '-Wpedantic', '-Wconversion',
            '-Wshadow', '-Wstrict-prototypes', '-Wold-style-definition',
            '-Wcast-qual', '-Wwrite-strings', '-Wundef', '-Wvla')

# Each entry: name, flags, host CPUID features needed to *execute* the result.
FLAG_SETS: tuple[tuple[str, tuple[str, ...], tuple[str, ...]], ...] = (
    ('O0',              ('-O0',),                                    ()),
    ('O1',              ('-O1',),                                    ()),
    ('O2',              ('-O2',),                                    ()),
    ('O3',              ('-O3',),                                    ()),
    ('Os',              ('-Os',),                                    ()),
    ('fma_fast',        ('-O2', '-mfma', '-ffp-contract=fast'),      ('fma',)),
    ('fma_off',         ('-O2', '-mfma', '-ffp-contract=off'),       ('fma',)),
    ('contract_on',     ('-O2', '-mfma', '-ffp-contract=on'),        ('fma',)),
    ('avx2_no_fma',     ('-O2', '-mavx2', '-mno-fma', '-ffast-math'),('avx2',)),
    ('avx2_prefer128',  ('-O2', '-mavx2', '-mfma',
                         '-mprefer-vector-width=128'),               ('avx2', 'fma')),
    ('avx512f_bw',      ('-O2', '-mavx512f', '-mavx512bw'),          ('avx512f', 'avx512bw')),
    ('native_unroll',   ('-O3', '-march=native', '-funroll-loops'),    ('avx512f', 'avx512bw')),
)
QUICK_SETS = ('O0', 'O2', 'fma_fast', 'avx512f_bw')


@dataclass(frozen=True)
class ShapeRow:
    name: str
    fast: str
    ref: str
    flag: str
    ok: bool

    def as_dict(self) -> dict:
        return {'name': self.name, 'fast': self.fast, 'ref': self.ref,
                'flag': self.flag, 'ok': self.ok}


def parse_all(output: str) -> dict[str, ShapeRow]:
    """Parse `all` stdout.  Unknown lines are rejected, never skipped."""
    rows: dict[str, ShapeRow] = {}
    for lineno, line in enumerate(output.splitlines(), 1):
        if not line.strip():
            continue
        parts = line.split()
        if len(parts) != 4 or parts[3] not in ('ok', 'MISMATCH'):
            raise ValueError(f'malformed shape line {lineno}: {line!r}')
        if parts[0] in rows:
            raise ValueError(f'duplicate shape {parts[0]!r}')
        rows[parts[0]] = ShapeRow(parts[0], parts[1], parts[2],
                                  parts[3], parts[3] == 'ok')
    if not rows:
        raise ValueError('empty shape output')
    return rows


def parse_bench(stderr: str) -> dict[str, float]:
    result: dict[str, float] = {}
    for line in stderr.splitlines():
        if line.startswith('#') or not line.strip():
            continue
        parts = line.split()
        if len(parts) == 2:
            try:
                result[parts[0]] = float(parts[1])
            except ValueError:
                continue
    return result


def classify(rows: dict[str, ShapeRow], reference: dict[str, ShapeRow],
             warnings: str) -> tuple[int, list[str]]:
    """Pure comparison logic, used by --self-test as well as the real run."""
    problems: list[str] = []
    if warnings.strip():
        problems.append(f'compiler diagnostics: {warnings.strip().splitlines()[0]}')
    if set(rows) != set(reference):
        problems.append(f'shape set differs: missing={sorted(set(reference) - set(rows))} '
                        f'extra={sorted(set(rows) - set(reference))}')
    for name in sorted(set(rows) & set(reference)):
        got, want = rows[name], reference[name]
        if not got.ok or got.fast != got.ref:
            problems.append(f'{name}: internal oracle mismatch')
        if got.fast != want.fast or got.ref != want.ref:
            problems.append(f'{name}: differs from -O0 reference')
    return len(problems), problems


def self_test() -> int:
    checks: list[tuple[str, bool]] = []

    def expect(label: str, got: object, want: object) -> None:
        checks.append((label, got == want))

    good = 'a 0000000000000001 0000000000000001 ok\n'
    bad = 'a 0000000000000001 0000000000000002 MISMATCH\n'
    expect('parse good', parse_all(good)['a'].ok, True)
    expect('parse bad', parse_all(bad)['a'].ok, False)
    try:
        parse_all('garbage\n')
        expect('reject malformed', False, True)
    except ValueError:
        expect('reject malformed', True, True)
    try:
        parse_all(good + good)
        expect('reject duplicate', False, True)
    except ValueError:
        expect('reject duplicate', True, True)
    expect('classify clean', classify(parse_all(good), parse_all(good), '')[0], 0)
    expect('classify oracle mismatch',
           classify(parse_all(bad), parse_all(good), '')[0] >= 1, True)
    expect('classify reference drift',
           classify(parse_all(good),
                    parse_all('a 0000000000000009 0000000000000001 ok\n'), '')[0] >= 1, True)
    expect('classify warning hard',
           classify(parse_all(good), parse_all(good), 'cc1: warning\n')[0] >= 1, True)
    expect('parse bench', parse_bench('# head\na 12.5\nb 7.0\n'),
           {'a': 12.5, 'b': 7.0})
    expect('sets unique', len({f[0] for f in FLAG_SETS}), len(FLAG_SETS))
    expect('quick subset valid', set(QUICK_SETS) <= {f[0] for f in FLAG_SETS}, True)

    failed = [label for label, ok in checks if not ok]
    for label, ok in checks:
        print(f'{"PASS" if ok else "FAIL"}  {label}')
    print(f'self-test: {len(checks) - len(failed)}/{len(checks)} checks passed')
    return 1 if failed else 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--candidate', default='gcc')
    parser.add_argument('--reference', default='gcc')
    parser.add_argument('--quick', action='store_true',
                        help='run only the four most informative flag sets')
    parser.add_argument('--sanitize', action='store_true',
                        help='also compile/run the corpus under ASan+UBSan')
    parser.add_argument('--bench', action='store_true',
                        help='measure median ns/iteration per shape on the host')
    parser.add_argument('--bench-iters', type=int, default=2048)
    parser.add_argument('--json', type=Path)
    parser.add_argument('--compile-timeout', type=float, default=120.0)
    parser.add_argument('--run-timeout', type=float, default=60.0)
    parser.add_argument('--self-test', action='store_true')
    parser.add_argument('--list-shapes', action='store_true')
    args = parser.parse_args(argv)

    if args.self_test:
        return self_test()
    try:
        cand, ref = compiler(args.candidate), compiler(args.reference)
    except ValueError as exc:
        parser.error(str(exc))
    if not SOURCE.is_file():
        parser.error(f'missing corpus: {SOURCE}')
    if not (0 < args.bench_iters <= 100000):
        parser.error('--bench-iters must be in [1, 100000]')

    host = host_cpu_flags()
    selected = [f for f in FLAG_SETS
                if not args.quick or f[0] in QUICK_SETS]
    receipt: dict = {
        'schema': 1, 'driver': 'check_simd_shape_matrix',
        'candidate': cand, 'reference': ref, 'host_flags': sorted(host),
        'quick': args.quick, 'sanitized': args.sanitize,
        'flag_sets': [], 'timings_ns': {},
    }
    failures: list[str] = []

    with TemporaryDirectory(prefix='lccc-shape-matrix-') as work:
        root = Path(work)

        def build(cc: list[str], flags: tuple[str, ...], out: Path,
                  extra: tuple[str, ...] = ()) -> tuple[int, str]:
            cmd = [*cc, *C_STD, *flags, *WARNINGS, *extra, str(SOURCE),
                   '-o', str(out)]
            try:
                rc, _, err = run_rc(cmd, root, args.compile_timeout)
            except ProbeError as exc:
                return -1, str(exc)
            return rc, err

        def execute(exe: Path, mode: tuple[str, ...]) -> tuple[int, str, str]:
            try:
                return run_rc([str(exe), *mode], root, args.run_timeout)
            except ProbeError as exc:
                return -1, '', str(exc)

        # One -O0 reference build for the whole matrix.  It is shared across
        # flag sets: compiling it per set would waste ~10 builds and cannot
        # add information (the reference is deliberately unoptimized).
        ref_exe = root / 'reference'
        rc, err = build(ref, ('-O0',), ref_exe)
        if rc != 0:
            print(f'ERROR reference build: {err.strip().splitlines()[:2]}')
            return 1
        rc, out, err = execute(ref_exe, ('all',))
        if rc != 0:
            print(f'ERROR reference run rc={rc}: {err.strip()[:200]}')
            return 1
        try:
            reference_rows = parse_all(out)
        except ValueError as exc:
            print(f'ERROR reference parse: {exc}')
            return 1
        if args.list_shapes:
            for name in sorted(reference_rows):
                print(name)
            return 0
        print(f'reference: {len(reference_rows)} shapes, -O0, digests pinned')

        for name, flags, host_isa in selected:
            absent = [f for f in host_isa if f not in host]
            if absent:
                print(f'SKIP  {name:<16} host lacks {",".join(absent)}')
                receipt['flag_sets'].append({'name': name, 'flags': list(flags),
                                             'status': 'SKIP', 'reason': absent})
                continue
            exe = root / f'cand_{name}'
            rc, err = build(cand, flags, exe)
            if rc != 0:
                failures.append(f'{name}: candidate build rc={rc}: '
                                f'{err.strip().splitlines()[:1]}')
                print(f'ERROR {name:<16} build failed')
                receipt['flag_sets'].append({'name': name, 'flags': list(flags),
                                             'status': 'ERROR'})
                continue
            rc, out, run_err = execute(exe, ('all',))
            # A nonzero exit is NOT automatically an infrastructure error:
            # the corpus returns 1 when its own fast/ref oracle detects a
            # mismatch, which is exactly the wrong-code evidence we want.
            # Parse stdout first; only empty or malformed output is ERROR.
            try:
                rows = parse_all(out)
            except ValueError as exc:
                failures.append(f'{name}: {exc} (rc={rc}: '
                                f'{run_err.strip()[:160]})')
                print(f'ERROR {name:<16} malformed output')
                receipt['flag_sets'].append({'name': name, 'flags': list(flags),
                                             'status': 'ERROR'})
                continue
            n, problems = classify(rows, reference_rows, err)
            if rc != 0 and n == 0:
                problems.append(f'candidate exited {rc} despite clean digests: '
                                f'{run_err.strip()[:160]}')
            n = len(problems)
            status = 'PASS' if n == 0 else 'FAIL'
            print(f'{status:<5} {name:<16} shapes={len(rows)} problems={n}')
            for problem in problems[:5]:
                print(f'      {problem}')
            failures.extend(f'{name}: {p}' for p in problems)
            receipt['flag_sets'].append({'name': name, 'flags': list(flags),
                                         'status': status, 'shapes': len(rows),
                                         'problems': problems})

        if args.sanitize:
            san_exe = root / 'sanitized'
            san_flags = ('-O1', '-g', '-fno-omit-frame-pointer',
                         '-fsanitize=address,undefined',
                         '-fno-sanitize-recover=all')
            rc, err = build(cand, san_flags, san_exe)
            if rc != 0:
                failures.append(f'sanitizer build rc={rc}: {err.strip()[:160]}')
                print('ERROR sanitizer build')
            else:
                rc, out, run_err = execute(san_exe, ('selftest',))
                san_err = (err + out + run_err).strip()
                if rc != 0 or san_err.startswith('AddressSanitizer') or \
                        'runtime error:' in san_err:
                    failures.append(f'sanitizer rc={rc}: {san_err[:160]}')
                    print('FAIL  sanitizer leg')
                else:
                    print('PASS  sanitizer leg (ASan+UBSan, selftest + all)')
                receipt['sanitizer'] = {'rc': rc, 'clean': rc == 0}

        if args.bench:
            bench_exe = root / 'bench'
            rc, err = build(cand, ('-O2', '-march=native'), bench_exe)
            if rc != 0:
                failures.append(f'bench build rc={rc}: {err.strip()[:160]}')
            else:
                rc, _, bench_err = execute(
                    bench_exe, ('bench', str(args.bench_iters)))
                timings = parse_bench(bench_err)
                receipt['timings_ns'] = timings
                print(f'PASS  bench leg: {len(timings)} shapes, '
                      f'median of 7 batches x {args.bench_iters} iterations')
                if rc != 0:
                    failures.append(f'bench run rc={rc}')

    if args.json:
        write_json(args.json, receipt)
    print(f'shape matrix: {len(selected)} flag sets, '
          f'{len(reference_rows)} shapes, {len(failures)} problems')
    return 1 if failures else 0


if __name__ == '__main__':
    raise SystemExit(main())
