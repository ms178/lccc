#!/usr/bin/env python3
"""Strict checksum-only gate for the exploratory corpus; never a timing gate."""
from __future__ import annotations

import argparse
import json
import math
from pathlib import Path
import re
import tempfile

from aos_pair_shapes import ProbeError, compiler, run

SOURCE = Path(__file__).with_name('scoreboard_kernels.c').resolve()
# Independent inventory: missing/duplicate rows cannot silently shrink coverage.
NAMES = '''add_f32 saxpy daxpy fma3 sqrt_f32 f2i i2f f2d stencil3
sum_i32 max_i32 dot_i32 dot_f32_fast sum_f64_fast sad_u8 popcnt count_eq_u8
add_i32 mul_i32 add_i16 add_u8 satadd_u8 widen_u8_u16 narrow_i16_u8 shift_i32
abs_f32 select_f32 min_i32 condstore clamp_i32 stride2 reverse gather scatter
cmul aos_soa alias_ptr restrict unknown_n alias_overlap recur prefix_i32
overlap_dist1 matmul'''.split()


def checksums(output: str) -> dict[str, str]:
    rows = {}
    for line in output.splitlines():
        fields = line.split()
        if len(fields) != 3:
            raise ProbeError(f'malformed scoreboard row: {line!r}')
        name, checksum, timing = fields
        if name not in NAMES or name in rows or not re.fullmatch('[0-9a-f]{16}', checksum):
            raise ProbeError(f'invalid name/checksum or duplicate row: {line!r}')
        try:
            ns = float(timing)
        except ValueError as exc:
            raise ProbeError(f'invalid timing: {line!r}') from exc
        if not math.isfinite(ns) or ns < 0:
            raise ProbeError(f'invalid timing: {line!r}')
        rows[name] = checksum
    if set(rows) != set(NAMES):
        raise ProbeError(f'missing rows: {sorted(set(NAMES) - set(rows))}')
    return rows


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--lccc', default='target/fastbuild/lccc')
    parser.add_argument('--reference', default='gcc-16')
    parser.add_argument('--reps', type=int, default=1)
    parser.add_argument('--json', type=Path)
    parser.add_argument('--check-clock-failure', action='store_true')
    args = parser.parse_args(argv)
    if args.reps < 1 or args.reps > 4000:
        parser.error('--reps must be in 1..4000')
    try:
        candidate, reference = compiler(args.lccc), compiler(args.reference)
    except ValueError as exc:
        parser.error(str(exc))
    rows = []
    with tempfile.TemporaryDirectory(prefix='lccc-scoreboard-') as work:
        root = Path(work)

        def evaluate(cc: list[str], opt: str) -> dict[str, str]:
            exe = root / 'probe'
            exe.unlink(missing_ok=True)
            run([*cc, opt, '-ffp-contract=off', f'-DREPS={args.reps}',
                 '-Wall', '-Wextra', '-Werror', str(SOURCE), '-lm', '-o', str(exe)], root, 60)
            return checksums(run([str(exe)], root, 60))

        try:
            expected = evaluate(reference, '-O0')
            for opt in ('-O1', '-O2', '-O3'):
                try:
                    got = evaluate(candidate, opt)
                    for name in NAMES:
                        rows.append(dict(opt=opt, kernel=name, ok=got[name] == expected[name],
                                         expected=expected[name], actual=got[name]))
                except ProbeError as exc:
                    rows.append(dict(opt=opt, ok=False, error=str(exc)))
        except ProbeError as exc:
            rows.append(dict(stage='reference', ok=False, error=str(exc)))
        if args.check_clock_failure:
            # Compilation failure is NOT the expected runtime clock failure.
            for role, cc, opt in [('reference', reference, '-O0'), ('candidate', candidate, '-O2')]:
                try:
                    exe = root / 'clock-failure'
                    exe.unlink(missing_ok=True)
                    fixture = SOURCE.with_name('clock_failure.c')
                    run([*cc, opt, '-DREPS=1', '-ffp-contract=off',
                         '-Wall', '-Wextra', '-Werror', str(SOURCE), str(fixture),
                         '-lm', '-o', str(exe)], root, 60)
                    try:
                        run([str(exe)], root, 60)
                    except ProbeError as exc:
                        if 'rc=70\n' not in str(exc) or 'clock_gettime:' not in str(exc):
                            raise
                    else:
                        raise ProbeError('injected clock failure did not terminate the probe')
                    rows.append(dict(stage='clock_failure', compiler=role, ok=True))
                except ProbeError as exc:
                    rows.append(dict(stage='clock_failure', compiler=role, ok=False, error=str(exc)))
    failed = [row for row in rows if not row['ok']]
    for row in failed:
        print(row)
    print(f'scoreboard: {len(rows)} checks, {len(failed)} failures (checksums only)')
    if args.json:
        args.json.parent.mkdir(parents=True, exist_ok=True)
        args.json.write_text(json.dumps(dict(schema=1, candidate=candidate, reference=reference,
                                             reps=args.reps, rows=rows), indent=2) + '\n')
    return int(bool(failed))


if __name__ == '__main__':
    raise SystemExit(main())
