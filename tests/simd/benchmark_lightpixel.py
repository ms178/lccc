#!/usr/bin/env python3
"""Order-balanced external-call microbenchmark; not a browser benchmark."""
from __future__ import annotations
import argparse
import json
import os
from pathlib import Path
import platform
import re
import statistics
import tempfile
from aos_pair_shapes import ProbeError, compiler, run

ROOT = Path(__file__).resolve().parent
NAMES = ('lightpixel_separate', 'lightpixel_value', 'lightpixel_wide4',
         'umin4', 'umax4', 'lightpixel_wide8', 'umin8', 'umax8')


def receipt(output: str, kernel: int) -> tuple[str, int]:
    match = re.fullmatch(r'(\d+) ([0-9a-f]{16}) ([0-9]+)\n', output)
    if match is None or int(match[1]) != kernel or int(match[3]) <= 0:
        raise ProbeError(f'invalid benchmark receipt: {output!r}')
    return match[2], int(match[3])


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--compiler', action='append', required=True, metavar='NAME=COMMAND')
    p.add_argument('--reference', default='gcc-16')
    p.add_argument('--rounds', type=int, default=12)
    p.add_argument('--json', type=Path, required=True)
    args = p.parse_args(argv)
    if args.rounds < 2 or args.rounds % 2:
        p.error('--rounds must be even and at least 2')
    commands = {}
    try:
        ref = compiler(args.reference)
        for spec in args.compiler:
            name, command = spec.split('=', 1)
            if not name or name == 'oracle' or name in commands:
                raise ValueError('compiler names must be nonempty and unique')
            commands[name] = compiler(command)
    except ValueError as exc:
        p.error(str(exc))
    cpu = None
    if hasattr(os, 'sched_getaffinity'):
        cpu = min(os.sched_getaffinity(0))
        os.sched_setaffinity(0, {cpu})
    rows = []
    report = dict(compilers=commands, reference=ref, rounds=args.rounds,
                  flags=['-O2', '-mavx2'], cpu=cpu, platform=platform.platform(),
                  versions={}, samples=rows, ok=False)
    try:
        for name, cc in commands.items():
            report['versions'][name] = run([*cc, '--version'], ROOT, 30)
        with tempfile.TemporaryDirectory(prefix='lccc-lightpixel-timing-') as work:
            root = Path(work)
            driver = root / 'driver.o'
            run([*ref, '-O2', '-Wall', '-Wextra', '-Werror', '-c',
                 str(ROOT / 'lightpixel_bench.c'), '-o', str(driver)], root, 60)
            executables = {}
            for index, (name, cc) in enumerate([('oracle', ref), *commands.items()]):
                obj, exe = root / f'{index}.o', root / f'{index}.exe'
                opt = '-O0' if index == 0 else '-O2'
                run([*cc, opt, '-mavx2', '-Wall', '-Wextra', '-Werror', '-c',
                     str(ROOT / 'lightpixel_kernels.c'), '-o', str(obj)], root, 60)
                run([*ref, str(driver), str(obj), '-o', str(exe)], root, 60)
                executables[name] = exe
            expected = [receipt(run([str(executables['oracle']), str(k)], root, 60), k)[0]
                        for k in range(len(NAMES))]
            # One untimed warm-up receipt per arm/kernel; never credit a bad hash.
            for name in commands:
                for k in range(len(NAMES)):
                    checksum, _ = receipt(run([str(executables[name]), str(k)], root, 60), k)
                    if checksum != expected[k]:
                        raise ProbeError(f'{name} warmup {NAMES[k]} checksum mismatch')
            for round_no in range(args.rounds):
                order = list(commands)
                if round_no % 2:
                    order.reverse()
                for k, kernel in enumerate(NAMES):
                    for name in order:
                        checksum, ns = receipt(run([str(executables[name]), str(k)], root, 60), k)
                        if checksum != expected[k]:
                            raise ProbeError(f'{name} {kernel} checksum mismatch')
                        rows.append(dict(round=round_no, compiler=name, kernel=kernel,
                                         checksum=checksum, ns=ns))
            report['median_ns'] = {
                name: {kernel: statistics.median(r['ns'] for r in rows
                       if r['compiler'] == name and r['kernel'] == kernel) for kernel in NAMES}
                for name in commands}
            report['ok'] = True
    except ProbeError as exc:
        report['error'] = str(exc)
    args.json.parent.mkdir(parents=True, exist_ok=True)
    args.json.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({k: v for k, v in report.items() if k in ('ok', 'error', 'median_ns')}, indent=2))
    return 0 if report['ok'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
