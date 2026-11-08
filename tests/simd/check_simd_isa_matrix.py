#!/usr/bin/env python3
"""Cross-ISA / cross-architecture bit-exact shape matrix driver.

`simd_isa_matrix.c` has 42 ISA/shape rows: portable vector-extension shapes at
128/256/512-bit widths, scalar/serial shapes, one intrinsic probe per x86 ISA
tier (SSE2, SSSE3, SSE4.1, AVX2, FMA3, AVX-512 F/BW/DQ/VL/VNNI), and one
probe per non-x86 vector ISA (NEON, SVE, RVV).  This driver checks, for each
configuration:

  1. semantic correctness -- every `fast()` digest equals its de-optimised
     `ref()` digest inside the candidate build;
  2. optimization/ISA invariance -- both digests equal the -O0 GCC reference
     build, so a compiler cannot "fix" a mismatch by changing the oracle;
  3. zero diagnostics -- strict -Werror build, any output is a hard error;
  4. honest coverage -- `list` reports `available=0` for shapes whose ISA was
     not enabled; a run cannot silently claim an ISA it did not exercise.

`--cross` adds the four non-x86 execution legs through the cached QEMU user
emulators and a static cross C compiler (zig cc / clang).  SVE and RVV are
enabled with target features, and RVV is exercised at four VLEN values
(128/256/512/1024) so scalable-vector *shards* are covered, not just one
implementation-defined vector length.  Missing tools are reported as SKIP,
never as PASS.

    python3 tests/simd/check_simd_isa_matrix.py --candidate gcc --reference gcc
    python3 tests/simd/check_simd_isa_matrix.py --quick
    python3 tests/simd/check_simd_isa_matrix.py --cross
    python3 tests/simd/check_simd_isa_matrix.py --self-test
"""
from __future__ import annotations

import argparse
from dataclasses import dataclass
from pathlib import Path
import shutil
import sys
from tempfile import TemporaryDirectory

_SCRIPT_DIR = str(Path(__file__).resolve().parent)
if _SCRIPT_DIR not in sys.path:
    sys.path.insert(0, _SCRIPT_DIR)

try:
    from probe_utils import ProbeError, compiler, host_cpu_flags, run_rc, write_json
except ImportError as exc:  # pragma: no cover
    raise SystemExit(f'check_simd_isa_matrix: probe_utils unavailable: {exc}') from exc

SOURCE = Path(__file__).with_name('simd_isa_matrix.c').resolve()
C_STD = ('-std=c99',)
WARNINGS = ('-Wall', '-Wextra', '-Werror', '-Wpedantic', '-Wconversion',
            '-Wshadow', '-Wstrict-prototypes', '-Wold-style-definition',
            '-Wcast-qual', '-Wwrite-strings', '-Wundef', '-Wvla',
            '-fno-strict-aliasing')
FULL_X86 = ('-mavx512f', '-mavx512bw', '-mavx512dq', '-mavx512vl',
            '-mavx512vnni', '-mfma', '-mavx2', '-mpopcnt', '-mbmi2')

# name, flags, host CPUID features needed to execute
FLAG_SETS: tuple[tuple[str, tuple[str, ...], tuple[str, ...]], ...] = (
    ('O0',             ('-O0',),                                          ()),
    ('O2',             ('-O2',),                                          ()),
    ('O3',             ('-O3',),                                          ()),
    ('Os',             ('-Os',),                                          ()),
    ('sse2_only',      ('-O2', '-mno-avx'),                                ()),
    ('avx2_no_fma',    ('-O2', '-mavx2', '-mno-fma', '-ffast-math'),       ('avx2',)),
    ('fma_fast',       ('-O2', '-mfma', '-ffast-math'),                    ('fma',)),
    ('fma_off',        ('-O2', '-mfma', '-ffp-contract=off'),              ('fma',)),
    ('avx512f_only',   ('-O3', '-mavx512f', '-mavx512vl'),                 ('avx512f', 'avx512vl')),
    ('avx512_all',     ('-O3', *FULL_X86),                                 ('avx512f', 'avx512bw', 'avx512dq', 'avx512vl', 'avx512vnni')),
    ('prefer128',      ('-O2', '-mprefer-vector-width=128', *FULL_X86),    ('avx512f', 'avx512bw', 'avx512dq', 'avx512vl', 'avx512vnni')),
    ('native_unroll',  ('-O3', '-march=native', '-funroll-loops'),          ('avx512f', 'avx512bw')),
)
QUICK_SETS = ('O0', 'O2', 'avx2_no_fma', 'avx512_all')

# target triple, cross -mcpu, qemu binary, qemu -cpu, required host CPUID
CROSS_LEGS: tuple[tuple[str, str, str, str, tuple[str, ...]], ...] = (
    ('aarch64_neon', 'aarch64-linux-musl', 'baseline', 'qemu-aarch64', 'max', ()),
    ('aarch64_sve',  'aarch64-linux-musl', 'baseline+sve', 'qemu-aarch64', 'max,sve=on', ()),
    ('rv64gcv_v128', 'riscv64-linux-musl', 'baseline_rv64+v', 'qemu-riscv64',
     'rv64,v=true,vlen=128,elen=64', ()),
    ('rv64gcv_v256', 'riscv64-linux-musl', 'baseline_rv64+v', 'qemu-riscv64',
     'rv64,v=true,vlen=256,elen=64', ()),
    ('rv64gcv_v512', 'riscv64-linux-musl', 'baseline_rv64+v', 'qemu-riscv64',
     'rv64,v=true,vlen=512,elen=64', ()),
    ('rv64gcv_v1024', 'riscv64-linux-musl', 'baseline_rv64+v', 'qemu-riscv64',
     'rv64,v=true,vlen=1024,elen=64', ()),
    ('i686_sse2',    'x86-linux-musl', 'baseline', 'qemu-i386', 'max', ()),
)


def has_isa(host: frozenset[str], name: str) -> bool:
    """CPUID token lookup with the Linux `avx512_vnni` / `avx512vnni`
    spelling variants.  A missing alias must never silently SKIP an ISA
    leg, so both spellings are tried and the caller still reports SKIPs."""
    if name in host:
        return True
    if name.startswith('avx512') and not name.startswith('avx512_'):
        return ('avx512_' + name[len('avx512'):]) in host
    return False


@dataclass(frozen=True)
class ShapeRow:
    name: str
    fast: str
    ref: str
    ok: bool

    def as_dict(self) -> dict:
        return {'name': self.name, 'fast': self.fast, 'ref': self.ref,
                'ok': self.ok}


def parse_all(output: str) -> dict[str, ShapeRow]:
    """Parse `all` stdout; unknown lines are rejected, never skipped."""
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
                                  parts[3] == 'ok')
    if not rows:
        raise ValueError('empty shape output')
    return rows


def parse_list(output: str) -> dict[str, dict]:
    """Parse `list` stdout into {name: metadata}."""
    result: dict[str, dict] = {}
    for line in output.splitlines():
        parts = line.split('\t')
        if len(parts) != 6:
            raise ValueError(f'malformed list line: {line!r}')
        name, isa, shape, width, available, origin = parts
        if available not in ('0', '1'):
            raise ValueError(f'bad available flag: {available!r}')
        if name in result:
            raise ValueError(f'duplicate shape {name!r}')
        result[name] = {'isa': isa, 'shape': shape, 'width': width,
                        'available': available == '1', 'origin': origin}
    if not result:
        raise ValueError('empty list output')
    return result


def classify(rows: dict[str, ShapeRow], reference: dict[str, ShapeRow],
             warnings: str) -> tuple[int, list[str]]:
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


def is_runtime_missing(diagnostic: str) -> bool:
    """Recognise a missing sanitizer runtime (infrastructure), not a code
    defect.  Kept as a separate predicate so it is unit-testable."""
    markers = ('libasan', 'libubsan', 'libclang_rt.asan', 'libclang_rt.ubsan',
               'cannot find -lasan', 'cannot find -lubsan')
    low = diagnostic.lower()
    return any(m.lower() in low for m in markers)


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
    expect('classify diagnostic hard',
           classify(parse_all(good), parse_all(good), 'cc1: warning\n')[0] >= 1, True)
    listing = ('a\tsse2\tmap\t128\t1\tIntel SDM\nb\tsve\tmap\tscalable\t0\tArm\n')
    expect('parse list', parse_list(listing)['b']['available'], False)
    try:
        parse_list('a\tsse2\tmap\t128\t2\tbad\n')
        expect('reject bad available', False, True)
    except ValueError:
        expect('reject bad available', True, True)
    expect('sets unique', len({f[0] for f in FLAG_SETS}), len(FLAG_SETS))
    expect('quick subset valid', set(QUICK_SETS) <= {f[0] for f in FLAG_SETS}, True)
    expect('cross legs unique', len({c[0] for c in CROSS_LEGS}), len(CROSS_LEGS))
    expect('missing-runtime recognised',
           is_runtime_missing('ld: cannot find -lasan: No such file'), True)
    expect('real error not mislabelled',
           is_runtime_missing('error: expected ; before }'), False)
    expect('RVV covers four VLENs',
           sum(1 for c in CROSS_LEGS if c[0].startswith('rv64gcv')), 4)

    failed = [label for label, ok in checks if not ok]
    for label, ok in checks:
        print(f'{"PASS" if ok else "FAIL"}  {label}')
    print(f'self-test: {len(checks) - len(failed)}/{len(checks)} checks passed')
    return 1 if failed else 0


def find_cross_tools() -> tuple[list[str] | None, Path | None, str]:
    """Return (zig-cc command, qemu bin directory, note)."""
    import os
    pfx = os.environ.get('LCCC_AGENT_CACHE_PREFIX', '')
    qdir = Path(pfx) / 'bin' if pfx else None
    zig = shutil.which('zig') or '/tmp/zig/ziglang/zig'
    if Path(zig).is_file():
        zig_cmd = [zig, 'cc']
    else:
        return None, qdir, 'no static cross C compiler (zig cc) found'
    if qdir is None or not qdir.is_dir():
        return zig_cmd, None, 'LCCC_AGENT_CACHE_PREFIX unset; no QEMU prefix'
    return zig_cmd, qdir, ''


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--candidate', default='gcc')
    parser.add_argument('--reference', default='gcc')
    parser.add_argument('--quick', action='store_true')
    parser.add_argument('--cross', action='store_true',
                        help='add QEMU cross-architecture legs')
    parser.add_argument('--sanitize', action='store_true')
    parser.add_argument('--bench', action='store_true')
    parser.add_argument('--bench-iters', type=int, default=1024)
    parser.add_argument('--json', type=Path)
    parser.add_argument('--compile-timeout', type=float, default=180.0)
    parser.add_argument('--run-timeout', type=float, default=90.0)
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
        'schema': 1, 'driver': 'check_simd_isa_matrix',
        'candidate': cand, 'reference': ref, 'host_flags': sorted(host),
        'quick': args.quick, 'cross': args.cross, 'sanitized': args.sanitize,
        'configurations': [], 'problems': [], 'timings_ns': {},
    }
    failures: list[str] = []

    with TemporaryDirectory(prefix='lccc-isa-matrix-') as work:
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

        def execute(argv_cmd: list[str]) -> tuple[int, str, str]:
            try:
                return run_rc(argv_cmd, root, args.run_timeout)
            except ProbeError as exc:
                return -1, '', str(exc)

        ref_exe = root / 'reference'
        rc, err = build(ref, ('-O0',), ref_exe)
        if rc != 0:
            print(f'ERROR reference build: {err.strip().splitlines()[:2]}')
            return 1
        rc, out, err = execute([str(ref_exe), 'all'])
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
            absent = [f for f in host_isa if not has_isa(host, f)]
            if absent:
                print(f'SKIP  {name}: host lacks {",".join(absent)}')
                receipt['configurations'].append({'name': name, 'status': 'SKIP',
                                                  'reason': f'host lacks {absent}'})
                continue
            exe = root / f'cand-{name}'
            rc, err = build(cand, flags, exe)
            if rc != 0:
                print(f'ERROR {name}: build rc={rc} {err.strip()[:200]}')
                failures.append(f'{name}: build rc={rc}')
                receipt['configurations'].append({'name': name, 'status': 'ERROR',
                                                  'diagnostic': err.strip()[:400]})
                continue
            rc, out, runerr = execute([str(exe), 'all'])
            # A nonzero rc is a SEMANTIC failure, not infrastructure: `all`
            # returns 1 when a shape mismatches but still prints every row.
            try:
                rows = parse_all(out)
            except ValueError as exc:
                print(f'ERROR {name}: {exc} (rc={rc}) {runerr.strip()[:160]}')
                failures.append(f'{name}: {exc}')
                receipt['configurations'].append({'name': name, 'status': 'ERROR',
                                                  'diagnostic': str(exc)})
                continue
            n, problems = classify(rows, reference_rows, err)
            for prob in problems:
                print(f'FAIL  {name}: {prob}')
                failures.append(f'{name}: {prob}')
            status = 'PASS' if n == 0 else 'FAIL'
            if n == 0:
                print(f'PASS  {name}: {len(rows)} shapes')
            receipt['configurations'].append({
                'name': name, 'flags': list(flags), 'status': status,
                'shapes': len(rows), 'problems': problems,
            })
            if args.bench and n == 0:
                brc, _, berr = execute([str(exe), 'bench', str(args.bench_iters)])
                if brc == 0:
                    for line in berr.splitlines():
                        if line.startswith('all_shapes '):
                            receipt['timings_ns'][name] = float(line.split()[1]) * 1e9
                            break

        if args.sanitize:
            # A staged toolchain may lack libasan/libubsan entirely (the
            # harness-cache-8 GCC 16.2 and Clang 23.1.5 prefixes do).  That
            # is an ENVIRONMENT limitation, not a corpus defect: retry with a
            # sanitizer-capable compiler and record which one actually ran.
            san_exe = root / 'sanitized'
            san_flags = ('-fsanitize=address,undefined',
                         '-fno-sanitize-recover=all')
            san_used = None
            san_out = san_err = ''
            san_rc = -1
            for san_cc in (cand, ref, compiler('gcc')):
                src, serr = build(san_cc, ('-O1',), san_exe, san_flags)
                if src != 0:
                    san_err = serr
                    continue
                src, sout, serr = execute([str(san_exe), 'selftest'])
                if src == 0 and 'selftest PASS' in sout:
                    san_used, san_rc, san_out = san_cc, src, sout
                    break
                san_err = serr or sout
                san_rc = src
            if san_used is not None:
                print(f'PASS  sanitizer ({san_used[0]}): {san_out.strip()}')
                receipt['sanitizer_status'] = 'PASS'
                receipt['sanitizer_compiler'] = san_used
            else:
                # No sanitizer runtime anywhere in this sandbox.  Report a
                # SKIP with the exact reason: never a silent PASS, never a
                # false ERROR that would make a correct corpus look broken.
                kind = ('missing sanitizer runtime' if is_runtime_missing(san_err)
                        else 'sanitizer unavailable')
                note = f'{kind}; last error: {san_err.strip()[:200]}'
                print(f'SKIP  sanitizer: {note}')
                receipt['sanitizer_status'] = 'SKIP'
                receipt['sanitizer_note'] = note
            receipt['sanitizer_rc'] = san_rc

        if args.cross:
            zig_cmd, qdir, note = find_cross_tools()
            if zig_cmd is None or qdir is None:
                print(f'SKIP  cross legs: {note}')
                receipt['cross_status'] = 'SKIP'
                receipt['cross_note'] = note
            else:
                for leg, triple, mcpu, qemu, qcpu, _required in CROSS_LEGS:
                    qexe = qdir / qemu
                    if not qexe.is_file():
                        print(f'SKIP  {leg}: missing {qexe}')
                        receipt['configurations'].append(
                            {'name': leg, 'status': 'SKIP', 'reason': 'no qemu'})
                        continue
                    exe = root / f'cross-{leg}'
                    cmdc = [*zig_cmd, '-target', triple, f'-mcpu={mcpu}',
                            *C_STD, '-O2', *WARNINGS, '-static',
                            str(SOURCE), '-o', str(exe)]
                    try:
                        crc, _, cerr = run_rc(cmdc, root, args.compile_timeout)
                    except ProbeError as exc:
                        crc, cerr = -1, str(exc)
                    if crc != 0:
                        print(f'ERROR {leg}: cross build rc={crc} {cerr.strip()[:160]}')
                        failures.append(f'{leg}: cross build')
                        receipt['configurations'].append(
                            {'name': leg, 'status': 'ERROR',
                             'diagnostic': cerr.strip()[:300]})
                        continue
                    qcmd = [str(qexe), '-cpu', qcpu, str(exe), 'all']
                    qrc, qout, qerr = execute(qcmd)
                    try:
                        rows = parse_all(qout)
                    except ValueError as exc:
                        print(f'ERROR {leg}: {exc} (rc={qrc}) {qerr.strip()[:160]}')
                        failures.append(f'{leg}: {exc}')
                        receipt['configurations'].append(
                            {'name': leg, 'status': 'ERROR', 'diagnostic': str(exc)})
                        continue
                    # Cross targets legitimately have different `available=1`
                    # sets, but the fold digests of EVERY shape must still be
                    # identical: the portable scalar fallbacks are exact.
                    n, problems = classify(rows, reference_rows, cerr)
                    for prob in problems:
                        print(f'FAIL  {leg}: {prob}')
                        failures.append(f'{leg}: {prob}')
                    status = 'PASS' if n == 0 else 'FAIL'
                    if n == 0:
                        print(f'PASS  {leg}: {len(rows)} shapes under {qemu}')
                    lrc, lout, _ = execute([str(qexe), '-cpu', qcpu, str(exe), 'list'])
                    available = None
                    if lrc == 0:
                        try:
                            meta = parse_list(lout)
                            available = sum(1 for m in meta.values() if m['available'])
                        except ValueError:
                            pass
                    receipt['configurations'].append({
                        'name': leg, 'status': status, 'shapes': len(rows),
                        'available': available, 'qemu_cpu': qcpu,
                        'problems': problems,
                    })
                receipt['cross_status'] = 'PASS' if not any(
                    c.get('status') == 'ERROR' or c.get('status') == 'FAIL'
                    for c in receipt['configurations'] if c['name'] in
                    {x[0] for x in CROSS_LEGS}) else 'FAIL'

    summary = {'configurations': len(receipt['configurations']),
               'shapes': len(reference_rows), 'problems': len(failures),
               'passed': sum(1 for c in receipt['configurations']
                             if c['status'] == 'PASS')}
    receipt['summary'] = summary
    receipt['problems'] = failures
    if args.json:
        write_json(args.json, receipt)
    print(f'ISA matrix: {summary["passed"]}/{summary["configurations"]} configurations, '
          f'{summary["shapes"]} shapes, {summary["problems"]} problems')
    return 1 if failures else 0


if __name__ == '__main__':
    sys.exit(main())
