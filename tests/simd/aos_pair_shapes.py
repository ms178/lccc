#!/usr/bin/env python3
"""Fail-closed SV-01 differential reproducer (no shell, private scratch files).

LCCC=/path/to/lccc python3 tests/simd/aos_pair_shapes.py
Use --sweep for trip-count boundaries. Results are diagnostic checksums, not
performance measurements. A failed reference never becomes an empty oracle.
"""
from __future__ import annotations

import argparse
import json
import math
import os
import re
from pathlib import Path
import shlex
import signal
import subprocess
import tempfile

T = r'''#include <stdio.h>
#define N TRIP
static float x[3*N+8], y[3*N+8], o[3*N+8];
__attribute__((noinline)) void k(void){ for(int i=0;i<N;i++){ BODY } }
int main(void){ for(int i=0;i<3*N;i++){x[i]=i*0.5f;y[i]=i*0.25f+1;} k();
/* Compare every element, not a weighted sum that can hide compensating errors. */
for(int i=0;i<3*N+8;i++) { printf("%a\n",(double)o[i]); }
return 0; }
'''
GUARDED = r'''#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <sys/mman.h>
#include <unistd.h>
#define N TRIP
#define COUNT ELEMENTS
static float *x, *y, *o;
/* Each object has an inaccessible page on both sides. Placement exposes
 * either an under-read or an over-read at the exact object boundary. */
static float *alloc_guarded(void) {
    long page = sysconf(_SC_PAGESIZE);
    size_t bytes = COUNT * sizeof(float);
    if (page <= 0 || bytes > (size_t)page) exit(70);
    char *p = mmap(0, 3*(size_t)page, PROT_NONE,
                   MAP_PRIVATE|MAP_ANONYMOUS, -1, 0);
    if (p == MAP_FAILED || mprotect(p+page, (size_t)page, PROT_READ|PROT_WRITE)) exit(71);
    return (float *)(p + page + (AT_END ? (size_t)page-bytes : 0));
}
__attribute__((noinline)) void k(void){ for(int i=0;i<N;i++){ BODY } }
int main(void) {
    x=alloc_guarded(); y=alloc_guarded(); o=alloc_guarded();
    for(int i=0;i<COUNT;i++){x[i]=i*0.5f;y[i]=i*0.25f+1;o[i]=0;}
    k();
    for(int i=0;i<COUNT;i++) printf("%a\n",(double)o[i]);
    return 0;
}
'''


def source(name: str, body: str, n: int, layout: str, ctype: str = 'float',
           stride: int = 2, mixed: bool = False) -> tuple[str, int]:
    # The three-field control retains its own access shape; the pair shapes
    # vary their spacing without changing which fields they read/write.
    fields = 3 if name == 'e_stride3' else stride
    body = body.replace('2*i', f'{stride}*i')
    if layout == 'arrays':
        count = max(3, stride)*n+8
        text = T.replace('3*N+8', str(count)).replace('3*N', str(count))
    else:
        count = max(1, fields*n)
        text = (GUARDED.replace('ELEMENTS', str(count))
                .replace('AT_END', str(int(layout == 'guard-end'))))
    text = text.replace('TRIP', str(n)).replace('BODY', body)
    if mixed:
        text = text.replace('i*0.5f', '(i%17-8)*0.5f').replace('i*0.25f+1', '(i%13-6)*0.25f+1')
    text = '#include <stdint.h>\n' + re.sub(r'\bfloat\b', ctype, text)
    return text, count

BODIES = {
    "a_swapadd": "o[2*i]=x[2*i]+y[2*i+1]; o[2*i+1]=x[2*i+1]+y[2*i];",
    "b_mul2": "o[2*i]=x[2*i]*y[2*i]; o[2*i+1]=x[2*i+1]*y[2*i+1];",
    "c_cmul": "float ar=x[2*i],ai=x[2*i+1],br=y[2*i],bi=y[2*i+1]; o[2*i]=ar*br-ai*bi; o[2*i+1]=ar*bi+ai*br;",
    "d_split": "float ar=x[2*i],ai=x[2*i+1],br=y[2*i],bi=y[2*i+1]; o[2*i]=ar*br; o[2*i+1]=ai*bi;",
    "e_stride3": "o[3*i]=x[3*i]+y[3*i+2]; o[3*i+1]=x[3*i+1]; o[3*i+2]=y[3*i];",
    "f_copy2": "o[2*i]=x[2*i]; o[2*i+1]=x[2*i+1];",
    "g_add_aos": "o[2*i]=x[2*i]+y[2*i]; o[2*i+1]=x[2*i+1]+y[2*i+1];",
}
TRIPS = (0, 1, 2, 3, 4, 5, 7, 8, 9, 15, 16, 17, 31, 32, 33, 63, 64, 65)


class ProbeError(RuntimeError):
    """A process failed before it could supply a trustworthy result."""


def run(command: list[str], cwd: Path, timeout: float) -> str:
    try:
        p = subprocess.Popen(command, cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                             text=True, start_new_session=True)
    except OSError as exc:
        raise ProbeError(f"{shlex.join(command)}: {exc}") from exc
    try:
        output, errors = p.communicate(timeout=timeout)
    except BaseException as exc:
        # A compiler may spawn an assembler/linker that inherits its pipes.
        # Killing only the parent lets those children outlive the timeout and
        # can leave communicate() waiting on their still-open stdout/stderr.
        try:
            os.killpg(p.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        p.communicate()
        if isinstance(exc, subprocess.TimeoutExpired):
            raise ProbeError(f"{shlex.join(command)}: {exc}") from exc
        raise
    if p.returncode:
        raise ProbeError(f"{shlex.join(command)}: rc={p.returncode}\n{errors}")
    return output


def compiler(command: str) -> list[str]:
    args = shlex.split(command)
    if not args:
        raise ValueError("compiler command must not be empty")
    # Relative executable paths must survive the private working directory.
    if '/' in args[0]:
        args[0] = str(Path(args[0]).resolve())
    return args


def evaluate(cc: list[str], flags: list[str], src: Path, exe: Path,
             timeout: float, expected_lines: int) -> str:
    # Never execute an artifact left by a previous successful compilation.
    exe.unlink(missing_ok=True)
    run([*cc, *flags, '-ffp-contract=off', '-Wall', '-Wextra', '-Werror',
         str(src), '-o', str(exe)], src.parent, timeout)
    output = run([str(exe)], src.parent, timeout)
    if len(output.splitlines()) != expected_lines:
        raise ProbeError(f"{exe}: expected {expected_lines} output elements")
    return output


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--lccc', default=os.environ.get('LCCC', 'lccc'))
    parser.add_argument('--reference', default=os.environ.get('CC', 'gcc'))
    parser.add_argument('--sweep', action='store_true')
    parser.add_argument('--guarded', action='store_true', help='test exact page-start/end boundaries')
    parser.add_argument('--type', choices=('float', 'double', 'int32_t', 'int16_t'), default='float')
    parser.add_argument('--stride', type=int, choices=(2, 3, 4), default=2)
    parser.add_argument('--mixed', action='store_true', help='bounded mixed-sign inputs')
    parser.add_argument('--timeout', type=float, default=30)
    parser.add_argument('--json', type=Path, help='write machine-readable receipts')
    args = parser.parse_args(argv)
    if not math.isfinite(args.timeout) or args.timeout <= 0:
        parser.error('--timeout must be finite and positive')
    try:
        cc, ref = compiler(args.lccc), compiler(args.reference)
    except ValueError as exc:
        parser.error(str(exc))
    rows = []
    with tempfile.TemporaryDirectory(prefix='lccc-aos-') as work:
        root = Path(work)
        for n in TRIPS if args.sweep else (64,):
            for name, body in BODIES.items():
                for layout in ('guard-start', 'guard-end') if args.guarded else ('arrays',):
                    src = root / f'{name}_{n}_{layout}.c'
                    text, count = source(name, body, n, layout, args.type, args.stride, args.mixed)
                    src.write_text(text)
                    try:
                        expected = evaluate(ref, ['-O0'], src, root / 'reference', args.timeout, count)
                    except ProbeError as exc:
                        rows.append(dict(shape=name, trip=n, layout=layout, stage='reference',
                                         ok=False, error=str(exc)))
                        continue
                    for opt in ('-O1', '-O2', '-O3'):
                        row = dict(shape=name, trip=n, layout=layout, opt=opt, stage='candidate', ok=False)
                        try:
                            got = evaluate(cc, [opt], src, root / 'candidate', args.timeout, count)
                            row['ok'] = got == expected
                            if not row['ok']:
                                row['error'] = 'element output mismatch'
                        except ProbeError as exc:
                            row['error'] = str(exc)
                        rows.append(row)
    for row in rows:
        print(f"{row['shape']:12s} N={row['trip']:2d} {row.get('opt', 'ref'):3s} {row['layout']}: "
              + ('ok' if row['ok'] else row['error']))
    if args.json:
        args.json.parent.mkdir(parents=True, exist_ok=True)
        args.json.write_text(json.dumps({'schema': 1, 'candidate': cc, 'reference': ref,
                                         'type': args.type, 'stride': args.stride, 'mixed': args.mixed, 'rows': rows}, indent=2) + '\n')
    return int(any(not row['ok'] for row in rows))


if __name__ == '__main__':
    raise SystemExit(main())
