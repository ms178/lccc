#!/usr/bin/env python3
"""Differential LightPixel gate; timings never determine correctness success."""
from __future__ import annotations
import argparse
import json
from pathlib import Path
import re
import tempfile
from aos_pair_shapes import ProbeError, compiler, run

ROOT = Path(__file__).resolve().parent
NAMES = ('lightpixel', 'lightpixel_separate', 'lightpixel_value',
         'lightpixel_wide4', 'umin4', 'umax4', 'lightpixel_scattered',
         'lightpixel_volatile', 'lightpixel_interference',
         'lightpixel_wide8', 'umin8', 'umax8')
EXPECTED = 'PASS lightpixel 65536 cases 12 kernels guarded3/4 alias0..3 full-u32\n'


def body(assembly: str, name: str) -> str:
    start = re.search(r'^' + re.escape(name) + r':.*$', assembly, re.M)
    if start is None:
        raise ProbeError(f'missing assembly symbol {name}')
    tail = assembly[start.end():]
    end = re.search(r'^\s*\.size\s+' + re.escape(name) + r'\s*,', tail, re.M)
    if end is None:
        raise ProbeError(f'missing assembly size {name}')
    return tail[:end.start()]


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--lccc', default='target/fastbuild/lccc')
    p.add_argument('--reference', default='gcc-16')
    p.add_argument('--json', type=Path)
    p.add_argument('--require-native', action='store_true')
    args = p.parse_args(argv)
    try:
        cc, ref = compiler(args.lccc), compiler(args.reference)
    except ValueError as exc:
        p.error(str(exc))
    rows = []
    with tempfile.TemporaryDirectory(prefix='lccc-lightpixel-') as work:
        root = Path(work)
        try:
            # Harness (including independent wide-multiply oracle) always
            # comes from the reference at O0, not the compiler under test.
            harness = root / 'harness.o'
            run([*ref, '-O0', '-Wall', '-Wextra', '-Werror', '-c',
                 str(ROOT / 'lightpixel_harness.c'), '-o', str(harness)], root, 60)
            for arch, flags in [('sse2', ['-msse2', '-mno-sse4.1', '-mno-avx']),
                                ('sse41', ['-msse4.1', '-mno-avx']),
                                ('avx2', ['-mavx2'])]:
                for opt in ('-O2', '-O3'):
                    row = dict(arch=arch, opt=opt, ok=False)
                    try:
                        obj, asm, exe = root / 'kernel.o', root / 'kernel.s', root / 'probe'
                        for path in (obj, asm, exe):
                            path.unlink(missing_ok=True)
                        base = [*cc, opt, *flags, '-Wall', '-Wextra', '-Werror']
                        source = str(ROOT / 'lightpixel_kernels.c')
                        run([*base, '-c', source, '-o', str(obj)], root, 60)
                        run([*base, '-S', source, '-o', str(asm)], root, 60)
                        assembly = asm.read_text()
                        functions = {n: body(assembly, n) for n in NAMES}
                        if args.require_native and arch != 'sse2':
                            for n, mnemonic in [('umin4', 'pminud'), ('umax4', 'pmaxud'),
                                                ('lightpixel_wide4', 'pminud')]:
                                if not re.search(r'\bv?' + mnemonic + r'\b', functions[n]):
                                    raise ProbeError(f'{n}: missing unsigned native {mnemonic}')
                        if args.require_native and arch == 'avx2':
                            for n, mnemonic in [('umin8', 'vpminud'), ('umax8', 'vpmaxud'),
                                                ('lightpixel_wide8', 'vpminud')]:
                                if not re.search(r'\b' + mnemonic + r'\b', functions[n]):
                                    raise ProbeError(f'{n}: missing {mnemonic}')
                        if args.require_native:
                            volatile = functions['lightpixel_volatile']
                            reads = re.findall(r'\bmovzbl\s+[^,\n]*\([^)\n]+\),', volatile)
                            writes = re.findall(r'\bmovb\s+[^,\n]+,\s*[^#\n]*\(', volatile)
                            if len(reads) != 6 or len(writes) != 4:
                                raise ProbeError('volatile: expected six byte reads and four byte writes')
                        if arch == 'sse2' and re.search(r'\b(?:v?pminud|v?pmaxud)\b', assembly):
                            raise ProbeError('SSE4.1 instruction leaked into SSE2 target')
                        run([*ref, str(harness), str(obj), '-o', str(exe)], root, 60)
                        output = run([str(exe)], root, 60)
                        if output != EXPECTED:
                            raise ProbeError(f'invalid/incomplete receipt: {output!r}')
                        row.update(ok=True, receipt=output.strip(), assembly=functions)
                    except ProbeError as exc:
                        row['error'] = str(exc)
                    rows.append(row)
        except ProbeError as exc:
            rows.append(dict(stage='reference', ok=False, error=str(exc)))
    report = dict(candidate=cc, reference=ref, cases=rows)
    if args.json:
        args.json.parent.mkdir(parents=True, exist_ok=True)
        args.json.write_text(json.dumps(report, indent=2) + '\n')
    for row in rows:
        print(('PASS' if row['ok'] else 'FAIL'), row.get('arch', 'reference'),
              row.get('opt', ''), row.get('error', ''))
    return 0 if rows and all(row['ok'] for row in rows) else 1


if __name__ == '__main__':
    raise SystemExit(main())
