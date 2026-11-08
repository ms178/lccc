#!/usr/bin/env python3
"""Fail-closed driver for the 2026-11-08 SIMD implementation audit.

Every row is tied to an SA item in engineering/SIMD-IMPLEMENTATION-AUDIT-
2026-11-08.md and carries the status that register claims.  Rows tagged
``open`` (finding unfixed) are expected to FAIL.  Rows tagged ``pass`` are
controls; rows tagged ``fixed`` are FORMER findings that this patchset
repaired — they are hard regression controls (a `fixed` row that starts
failing is a REGRESSION, never an "expected open" success).  Keeping the
history in the row tag prevents a silently reverted fix from looking like a
green audit.  The status lattice:

    OPEN-CONFIRMED  open row failed  -> report in default mode, hard in strict
    STALE-OPEN      open row passed  -> hard in default mode (reconcile the
                                        register or flip the row tag)
    PASS            control passed   -> soft
    REGRESSION      control failed   -> hard (the harness or the compiler broke)
    ERROR            infrastructure  -> always hard
    SKIP            host lacks ISA   -> soft, with an explicit reason

Default mode exits nonzero only for REGRESSION, ERROR or STALE-OPEN.  A
compiler with known open defects can therefore pass; a fix that lands
silently cannot.  ``--strict`` inverts the open rows into required passes, so
the same command becomes the post-fix gate.  A compiler without the findings
(e.g. ``--lccc $(which gcc)``) is strict-green: that is the harness's own
end-to-end oracle.

    python3 tests/simd/check_audit_2026-11-08.py --lccc target/fastbuild/lccc --reference gcc-16
    python3 tests/simd/check_audit_2026-11-08.py --lccc $(which gcc) --strict
    python3 tests/simd/check_audit_2026-11-08.py --self-test
    python3 tests/simd/check_audit_2026-11-08.py --list-rows

Design notes (these fix the defects of the first revision of this pack):

* Controls are real rows, not prose.  ``--list-rows`` is the machine-readable
  contract; a static test asserts every row token in the audit resolves here.
* A failed compile is never a pass unless stderr is a *diagnostic*.  ICEs,
  timeouts, missing toolchains and unclassified failures are ERROR.
* Every asm oracle proves the kernel was actually emitted (label + multiply)
  before asserting the absence or presence of an instruction.
* Runtime rows gate on host CPUID, not on target predefined macros.
* ``-ffp-contract=fast`` is pinned for the SA-02 rows, whose trigger *is*
  contraction; a second ``-ffp-contract=off`` row is a control.
* ``-std=c99`` is pinned, so candidate and reference see the same language.
"""
from __future__ import annotations

import argparse
from dataclasses import dataclass
from pathlib import Path
import sys

# Make sibling imports work regardless of cwd or invocation style.
_SCRIPT_DIR = str(Path(__file__).resolve().parent)
if _SCRIPT_DIR not in sys.path:
    sys.path.insert(0, _SCRIPT_DIR)

try:
    from probe_utils import (
        ProbeError, compiler, compiler_crash, host_cpu_flags, is_crash,
        is_diagnostic, run_rc, run_rc_safe, write_json,
    )
except ImportError as exc:  # pragma: no cover - repository integrity guard
    raise SystemExit(
        f'check_audit: tests/simd/probe_utils.py is unavailable ({exc}); '
        'the driver cannot run with an unverified process layer'
    ) from exc

SOURCE = Path(__file__).with_name('audit_repro_2026-11-08.c').resolve()
C_STD = ['-std=c99']
WARNINGS = ['-Wall', '-Werror']

# ---------------------------------------------------------------------------
# Standalone translation units for asm-shape / diagnostic / assembler rows.
# Global (non-static) symbols plus a caller: an oracle that only asserts the
# *absence* of an instruction can be satisfied by a compiler emitting nothing.
# ---------------------------------------------------------------------------

SA01_TU = r'''#include <stdint.h>
float sa01_sink1, sa01_sink2;
__attribute__((noinline)) void sa01_dot2(int n, const float *restrict a,
    const float *restrict b, const float *restrict c, const float *restrict d,
    float *restrict r1, float *restrict r2) {
    float s1 = 0.0f, s2 = 0.0f;
    int i;
    for (i = 0; i < n; i++) { s1 += a[i] * b[i]; s2 += c[i] * d[i]; }
    *r1 = s1; *r2 = s2;
}
void sa01_probe(const float *a, const float *b, const float *c,
                const float *d) { sa01_dot2(64, a, b, c, d,
                &sa01_sink1, &sa01_sink2); }
'''

SA01_CTL_TU = r'''#include <stdint.h>
float sa01_ctl_sink;
__attribute__((noinline)) void sa01_ctl_dot1(int n, const float *restrict a,
    const float *restrict b, float *restrict r) {
    float s = 0.0f;
    int i;
    for (i = 0; i < n; i++) s += a[i] * b[i];
    *r = s;
}
void sa01_ctl_probe(const float *a, const float *b) {
    sa01_ctl_dot1(64, a, b, &sa01_ctl_sink);
}
'''

SA03_TU = r'''#include <immintrin.h>
__attribute__((noinline)) __m512i sa03_load512(const void *p) {
    return _mm512_loadu_si512(p);
}
__attribute__((noinline)) void sa03_store512(void *p, __m512i v) {
    _mm512_storeu_si512(p, v);
}
'''

SA04_TU = r'''#include <immintrin.h>
__attribute__((noinline)) __m512i sa04_shuflo(__m512i v) {
    return _mm512_shufflelo_epi16(v, 0x4E);
}
'''

SA33_TU = r'''#include <immintrin.h>
void sa33_sink(void);
__attribute__((noinline)) __m512i sa33_ins(__m512i a, __m128i b) {
    return _mm512_inserti32x4(a, b, 1);
}
__attribute__((noinline)) void sa33_f(__m512i a, __m128i b) {
    sa33_sink();
    (void)sa33_ins(a, b);
    sa33_sink();
}
'''

SA13_ASM = '''\
.text
.globl sa13_probe
sa13_probe:
    vinserti128 $1, %xmm1, %xmm2, %xmm3
    ret
'''


@dataclass(frozen=True)
class Row:
    name: str
    kind: str                 # runtime | diff | asm | diag | assembler
    flags: tuple[str, ...]
    expected: str             # open | pass | fixed
    item: str
    ice_ok: bool
    host_isa: tuple[str, ...] = ()
    entry: str = ''          # argv[1] of the C reproducer; defaults to name


ROWS: tuple[Row, ...] = (
    Row('sa02',         'runtime', ('-O2', '-mfma', '-ffp-contract=fast'), 'fixed', 'SA-02', False, ('fma',)),
    Row('sa02_ctl',     'runtime', ('-O2', '-mfma', '-ffp-contract=fast'), 'pass', 'SA-02', False, ('fma',)),
    Row('sa02_nofuse',  'runtime', ('-O2', '-mfma', '-ffp-contract=off'),  'pass', 'SA-02', False, ('fma',), 'sa02'),
    Row('sa05',         'runtime', ('-O2', '-mavx512bw'),                  'fixed', 'SA-05', False, ('avx512f', 'avx512bw')),
    Row('sa05_ctl',     'runtime', ('-O2',),                               'pass', 'SA-05', False),
    Row('sa06',         'diff',    ('-O2',),                               'fixed', 'SA-06', False),
    Row('sa07',         'diff',    ('-O2',),                               'fixed', 'SA-07', True),
    Row('sa01',         'runtime', ('-O2',),                               'pass', 'SA-01', False),
    Row('sa01_ctl',     'runtime', ('-O2',),                               'pass', 'SA-01', False),
    Row('sa01_asm',     'asm',     ('-O2', '-mavx2', '-mno-fma', '-ffast-math'), 'fixed', 'SA-01', False),
    Row('sa01_ctl_asm', 'asm',     ('-O2', '-mavx2', '-mno-fma', '-ffast-math'), 'pass', 'SA-01', False),
    Row('sa03_asm',     'asm',     ('-O2', '-mavx'),                      'fixed', 'SA-03', False),
    Row('sa04_diag',    'diag',    ('-O2', '-mavx512f'),                   'fixed', 'SA-04', False),
    Row('sa13_asm',     'assembler', (),                                   'fixed', 'SA-13', False),
    Row('sa33_asm',     'asm',     ('-O2', '-mavx512f'),                   'fixed', 'SA-33', False),
)

ROW_BY_NAME: dict[str, Row] = {row.name: row for row in ROWS}

# One -D per row isolates the C reproducer into a per-finding translation
# unit.  Without this, a single compiler ICE in SA-07's word-phi kernel made
# all nine runtime/diff rows fail to compile, turning eight independent
# measurements into one ERROR (measured against a real fastbuild lccc).
ROW_GROUP: dict[str, str] = {
    'sa02': 'AUDIT_SA02', 'sa02_ctl': 'AUDIT_SA02',
    'sa02_nofuse': 'AUDIT_SA02',
    'sa05': 'AUDIT_SA05', 'sa05_ctl': 'AUDIT_SA05',
    'sa06': 'AUDIT_SA06', 'sa07': 'AUDIT_SA07',
    'sa01': 'AUDIT_SA01', 'sa01_ctl': 'AUDIT_SA01',
}
assert set(ROW_GROUP) == {r.name for r in ROWS
                          if r.kind in ('runtime', 'diff')}


def group_flags(row: Row) -> tuple[str, ...]:
    return ('-D' + ROW_GROUP[row.name],)


@dataclass
class Result:
    kind: str                 # pass | fail | skip | error
    detail: str = ''

    def __str__(self) -> str:
        return self.kind if not self.detail else f'{self.kind}: {self.detail}'


def _trim(text: str, limit: int = 160) -> str:
    stripped = text.strip()
    first = stripped.splitlines()[0] if stripped else ''
    return first[:limit]


def check_asm_text(
    text: str,
    must_contain: tuple[str, ...] = (),
    must_contain_any: tuple[tuple[str, ...], ...] = (),
    must_not_contain: tuple[str, ...] = (),
    ordered_pairs: tuple[tuple[object, str], ...] = (),
) -> Result:
    """Pure assembly oracle, shared by probe_asm() and --self-test.

    ``must_contain_any`` accepts a group of equivalent spellings: GCC emits
    ``vinserti32x4`` for _mm512_inserti32x4 while Clang emits the bitwise
    equivalent ``vinsertf32x4``.  Hard-coding one spelling would turn a
    different-but-correct front end into a fake infrastructure ERROR.
    A missing ordered pair is a *finding* (fail), not an infrastructure
    error; absence of the kernel itself is an ERROR.
    """
    if not text.strip():
        return Result('error', 'empty assembly: oracle cannot prove emission')
    for needle in must_contain:
        if needle not in text:
            return Result('error', f'kernel not emitted: missing {needle!r}')
    for group in must_contain_any:
        if not any(needle in text for needle in group):
            return Result('error', f'kernel not emitted: none of {group!r}')
    for first, second in ordered_pairs:
        alternatives = (first,) if isinstance(first, str) else tuple(first)  # type: ignore[arg-type]
        positions = [text.find(a) for a in alternatives if a in text]
        if not positions:
            return Result('fail',
                          f'ordered-pair oracle cannot prove {alternatives!r}')
        a = min(positions)
        b = text.find(second)
        if b < 0 or b <= a:
            return Result('fail',
                          f'{second!r} not emitted after {alternatives!r}')
    for needle in must_not_contain:
        if needle in text:
            return Result('fail', f'contains {needle!r}')
    return Result('pass')


def classify(result: Result, expected: str, strict: bool) -> tuple[str, bool]:
    """Pure status lattice.  Returns (label, is_hard)."""
    if result.kind == 'error':
        return 'ERROR', True
    if result.kind == 'skip':
        return 'SKIP', False
    if expected == 'open':
        if result.kind == 'fail':
            return 'OPEN-CONFIRMED', bool(strict)
        return 'STALE-OPEN', not strict
    if result.kind == 'pass':
        return 'PASS', False
    return 'REGRESSION', True


def missing_isa(required: tuple[str, ...], host: frozenset[str]) -> tuple[str, ...]:
    """Pure helper: CPUID features required to *execute* a compiled probe."""
    return tuple(sorted(f for f in required if f not in host))


def self_test() -> int:
    """Prove the checker itself can fail.  No compiler is invoked."""
    checks: list[tuple[str, bool]] = []

    def expect(label: str, got: object, want: object) -> None:
        checks.append((label, got == want))

    expect('error hard default', classify(Result('error'), 'open', False), ('ERROR', True))
    expect('error hard strict', classify(Result('error'), 'open', True), ('ERROR', True))
    expect('skip soft', classify(Result('skip'), 'open', False), ('SKIP', False))
    expect('open fail default', classify(Result('fail'), 'open', False), ('OPEN-CONFIRMED', False))
    expect('open fail strict', classify(Result('fail'), 'open', True), ('OPEN-CONFIRMED', True))
    expect('open pass default', classify(Result('pass'), 'open', False), ('STALE-OPEN', True))
    expect('open pass strict', classify(Result('pass'), 'open', True), ('STALE-OPEN', False))
    expect('control pass', classify(Result('pass'), 'pass', False), ('PASS', False))
    expect('control regression', classify(Result('fail'), 'pass', False), ('REGRESSION', True))
    expect('control error', classify(Result('error'), 'pass', True), ('ERROR', True))

    expect('empty stderr not diagnostic', is_diagnostic('')[0], False)
    expect('ICE not diagnostic', is_diagnostic('internal compiler error: oops')[0], False)
    expect('timeout not diagnostic', is_diagnostic('command timed out after 1s')[0], False)
    expect('missing toolchain not diagnostic', is_diagnostic('cc: No such file or directory')[0], False)
    expect('feature diagnostic accepted', is_diagnostic("error: target specific option mismatch\nrequires -mavx512bw")[0], True)
    expect('hint accepted', is_diagnostic('bizarre wording', ('bizarre',))[0], True)
    expect('unclassified rejected', is_diagnostic('xyzzy')[0], False)
    expect('infrastructure stderr not a diagnostic',
           is_diagnostic('probe infrastructure failure: timed out')[0], False)

    expect('ICE text is a crash',
           is_crash('cc1: internal compiler error: in expand'),
           'internal compiler error')
    expect('panic is a crash', bool(is_crash("thread 'main' panicked at foo.rs")), True)
    expect('feature diagnostic is not a crash',
           is_crash("error: unrecognized command-line option '-mavx512bw'"), '')
    expect('signal is a crash', compiler_crash(-11, ''), 'signal 11')
    expect('bad flag is not a crash', compiler_crash(1, 'error: bad flag'), '')
    expect('infrastructure sentinel is not a crash',
           compiler_crash(-1, 'probe infrastructure failure: timed out'), '')

    gcc_asm = 'sa33_f:\n\tvinserti32x4 $1, %xmm1, %zmm0, %zmm0\n\tvzeroupper\n'
    clang_asm = 'sa33_f:\n\tvinsertf32x4 $1, %xmm1, %zmm0, %zmm0\n\tvzeroupper\n'
    no_vz = 'sa33_f:\n\tvinsertf32x4 $1, %xmm1, %zmm0, %zmm0\n'
    inserts = ('vinserti32x4', 'vinsertf32x4', 'vinserti64x4', 'vinsertf64x4')
    asm_args = dict(must_contain=('sa33_f',), must_contain_any=(inserts,),
                    ordered_pairs=((inserts, 'vzeroupper'),))
    expect('asm oracle GCC spelling', check_asm_text(gcc_asm, **asm_args).kind, 'pass')
    expect('asm oracle Clang spelling', check_asm_text(clang_asm, **asm_args).kind, 'pass')
    expect('missing vzeroupper is a finding',
           check_asm_text(no_vz, **asm_args).kind, 'fail')
    expect('missing kernel is ERROR',
           check_asm_text('nothing relevant\n', **asm_args).kind, 'error')
    expect('empty asm is ERROR', check_asm_text('', **asm_args).kind, 'error')
    expect('forbidden mnemonic is a finding',
           check_asm_text('k:\n\tvfmadd213ps\n',
                          must_contain=('k',),
                          must_not_contain=('vfmadd',)).kind, 'fail')

    expect('host gating exact', missing_isa(('fma', 'avx512bw'), frozenset({'fma'})), ('avx512bw',))
    expect('host gating complete', missing_isa(('fma',), frozenset({'fma', 'avx2'})), ())
    expect('host gating empty host fails closed', missing_isa(('fma',), frozenset()), ('fma',))

    expect('rows unique', len({r.name for r in ROWS}), len(ROWS))
    expect('controls exist', sum(r.expected == 'pass' for r in ROWS) >= 3, True)
    expect('fixed regression controls exist',
           sum(r.expected == 'fixed' for r in ROWS) >= 9, True)
    expect('expected values are known',
           all(r.expected in ('open', 'pass', 'fixed') for r in ROWS), True)
    expect('fixed rows are hard controls',
           classify(Result('fail'), 'fixed', False), ('REGRESSION', True))
    expect('every row names an SA item', all(r.item.startswith('SA-') for r in ROWS), True)

    bad = [label for label, ok in checks if not ok]
    for label, ok in checks:
        print(f'{"PASS" if ok else "FAIL"}  {label}')
    print(f'self-test: {len(checks) - len(bad)}/{len(checks)} checks passed')
    return 1 if bad else 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--lccc', default='target/fastbuild/lccc')
    parser.add_argument('--reference', default='gcc')
    parser.add_argument('--strict', action='store_true',
                        help='open rows must now pass (post-fix gate)')
    parser.add_argument('--json', type=Path)
    parser.add_argument('--compile-timeout', type=float, default=120.0)
    parser.add_argument('--run-timeout', type=float, default=60.0)
    parser.add_argument('--list-rows', action='store_true',
                        help='print the machine-readable row contract and exit')
    parser.add_argument('--self-test', action='store_true',
                        help='verify the status lattice and diagnostic classifier')
    args = parser.parse_args(argv)

    if args.self_test:
        return self_test()
    if args.list_rows:
        for row in ROWS:
            print(f'{row.name}\t{row.expected}\t{row.item}\t{row.kind}\t'
                  f'{" ".join(row.flags)}\t{",".join(row.host_isa)}')
        return 0
    for name, value in (('--compile-timeout', args.compile_timeout),
                        ('--run-timeout', args.run_timeout)):
        if not (0 < value <= 1800):
            parser.error(f'{name} must be in (0, 1800] seconds')

    try:
        cand, ref = compiler(args.lccc), compiler(args.reference)
    except ValueError as exc:
        parser.error(str(exc))
    if not Path(cand[0]).exists() and '/' in cand[0]:
        parser.error(f'lccc not found: {args.lccc} '
                     '(build it first: cargo build --profile fastbuild)')
    if not SOURCE.is_file():
        parser.error(f'missing reproducer source: {SOURCE}')

    host = host_cpu_flags()
    report: list[dict] = []
    hard = 0

    from tempfile import TemporaryDirectory
    with TemporaryDirectory(prefix='lccc-audit-1108-') as work:
        root = Path(work)

        def compile_step(cmd: list[str], ice_ok: bool) -> Result | None:
            """Compile once; return an ERROR/RESULT or None on success.

            `ice_ok` does NOT mean "any compile failure is a finding".  Only a
            genuine compiler crash (ICE text, panic, assertion or a POSIX
            signal) may be counted as SA-07's expected ICE.  A feature
            diagnostic, a bad flag, a missing toolchain or a timeout stays a
            hard infrastructure ERROR, so a broken toolchain cannot produce
            fake evidence.
            """
            rc, _, err = run_rc_safe(cmd, root, args.compile_timeout)
            if rc == 0:
                return None
            crash = compiler_crash(rc, err) if ice_ok else ''
            if crash:
                return Result('fail', f'compiler crash ({crash})')
            return Result('error', f'compile-fail rc={rc}: {_trim(err)}')

        def compile_and_run(row: Row) -> Result:
            exe = root / row.name
            entry = row.entry or row.name
            exe.unlink(missing_ok=True)
            cmd = [*cand, *C_STD, *row.flags, *group_flags(row), *WARNINGS,
                   str(SOURCE), '-o', str(exe)]
            failure = compile_step(cmd, row.ice_ok)
            if failure is not None:
                return failure
            if not exe.is_file():
                return Result('error', 'compiler returned 0 without an executable')
            try:
                rc, out, err = run_rc([str(exe), entry], root,
                                      args.run_timeout)
            except ProbeError as exc:
                return Result('error', f'run {_trim(str(exc))}')
            if 'MISMATCH' in out:
                return Result('fail', _trim(out))
            if 'skipped' in out:
                return Result('skip', _trim(out))
            if rc != 0:
                return Result('error', f'candidate exit {rc}: {_trim(err)}')
            if 'ok' not in out:
                return Result('error', f'unexpected output {_trim(out)!r}')
            return Result('pass')

        def probe_diff(row: Row) -> Result:
            exe = root / f'{row.name}_cand'
            ref_exe = root / f'{row.name}_ref'
            entry = row.entry or row.name
            for path in (exe, ref_exe):
                path.unlink(missing_ok=True)
            failure = compile_step(
                [*ref, '-O0', *C_STD, *group_flags(row), str(SOURCE),
                 '-o', str(ref_exe)], False)
            if failure is not None:
                return Result('error', f'reference-fail {failure.detail}')
            failure = compile_step(
                [*cand, *C_STD, *row.flags, *group_flags(row), *WARNINGS,
                 str(SOURCE), '-o', str(exe)], row.ice_ok)
            if failure is not None:
                return failure
            try:
                got_rc, got, got_err = run_rc([str(exe), entry], root,
                                              args.run_timeout)
                want_rc, want, want_err = run_rc([str(ref_exe), entry],
                                                 root, args.run_timeout)
            except ProbeError as exc:
                return Result('error', f'run {_trim(str(exc))}')
            if want_rc != 0:
                return Result('error', f'reference exit {want_rc}: {_trim(want_err)}')
            if got_rc != 0:
                return Result('fail', f'candidate exit {got_rc}: {_trim(got_err)}')
            if got != want:
                return Result('fail', f'{_trim(got)!r} != {_trim(want)!r}')
            return Result('pass')

        def probe_asm(name: str, source: str, flags: tuple[str, ...],
                      must_contain: tuple[str, ...] = (),
                      must_contain_any: tuple[tuple[str, ...], ...] = (),
                      must_not_contain: tuple[str, ...] = (),
                      ordered_pairs: tuple[tuple[object, str], ...] = (),
                      diag_is_pass: bool = False,
                      diag_hints: tuple[str, ...] = ()) -> Result:
            src = root / f'{name}.c'
            asm = root / f'{name}.s'
            src.write_text(source, encoding='utf-8')
            asm.unlink(missing_ok=True)
            # One compile only.  run_rc exposes the raw stderr, so diagnostic
            # classification cannot be fooled by the flags in the command
            # line (an earlier revision matched 'avx512' in the command).
            rc, _, err = run_rc_safe(
                [*cand, *C_STD, *flags, '-S', str(src), '-o', str(asm)],
                root, args.compile_timeout)
            if rc == -1:
                return Result('error', f'infrastructure {_trim(err)}')
            if rc != 0:
                if not diag_is_pass:
                    return Result('error', f'compile-fail {_trim(err)}')
                ok, why = is_diagnostic(err, diag_hints)
                return (Result('pass', why) if ok
                        else Result('error', f'non-diagnostic failure: {why}'))
            if not asm.is_file():
                return Result('error', 'compiler returned 0 without assembly')
            text = asm.read_text(encoding='utf-8', errors='replace')
            return check_asm_text(text, must_contain, must_contain_any,
                                  must_not_contain, ordered_pairs)

        def probe_diag(name: str, source: str, flags: tuple[str, ...],
                       diag_hints: tuple[str, ...]) -> Result:
            src = root / f'{name}.c'
            obj = root / f'{name}.o'
            src.write_text(source, encoding='utf-8')
            obj.unlink(missing_ok=True)
            rc, _, err = run_rc_safe([*cand, *C_STD, *flags, '-c', str(src),
                                      '-o', str(obj)], root, args.compile_timeout)
            if rc == -1:
                return Result('error', f'infrastructure {_trim(err)}')
            if rc == 0:
                return Result('fail', 'compiled cleanly, no diagnostic')
            ok, why = is_diagnostic(err, diag_hints)
            return (Result('pass', why) if ok
                    else Result('error', f'non-diagnostic failure: {why}'))

        def probe_assembler(name: str, text: str,
                            diag_hints: tuple[str, ...]) -> Result:
            src = root / f'{name}.s'
            obj = root / f'{name}.o'
            src.write_text(text, encoding='utf-8')
            obj.unlink(missing_ok=True)
            rc, _, err = run_rc_safe([*cand, '-c', str(src), '-o', str(obj)],
                                     root, args.compile_timeout)
            if rc == -1:
                return Result('error', f'infrastructure {_trim(err)}')
            if rc == 0:
                return Result('fail', 'assembler accepted an invalid xmm spelling')
            ok, why = is_diagnostic(err, diag_hints)
            return (Result('pass', why) if ok
                    else Result('error', f'non-diagnostic failure: {why}'))

        def record(row: Row, result: Result) -> None:
            nonlocal hard
            label, is_hard = classify(result, row.expected, args.strict)
            hard += int(is_hard)
            report.append({
                'row': row.name, 'item': row.item, 'kind': row.kind,
                'flags': ' '.join(row.flags), 'expected': row.expected,
                'actual': str(result), 'status': label,
                'host_isa': list(row.host_isa),
            })

        # --- runtime / differential rows ----------------------------------
        for row in ROWS:
            if row.kind not in ('runtime', 'diff'):
                continue
            absent = missing_isa(row.host_isa, host)
            if absent:
                record(row, Result('skip', f'host lacks {",".join(absent)}'))
                continue
            record(row, probe_diff(row) if row.kind == 'diff'
                   else compile_and_run(row))

        # --- SA-01: two accumulators must not emit FMA without -mfma ------
        row = ROW_BY_NAME['sa01_asm']
        record(row, probe_asm(
            'sa01', SA01_TU, row.flags,
            must_contain=('sa01_dot2', 'vmul'),
            must_not_contain=('vfmadd',)))
        # --- SA-01 control: the single-accumulator arm is already clean ---
        row = ROW_BY_NAME['sa01_ctl_asm']
        record(row, probe_asm(
            'sa01_ctl', SA01_CTL_TU, row.flags,
            must_contain=('sa01_ctl_dot1', 'vmul'),
            must_not_contain=('vfmadd',)))
        # --- SA-03: 512-bit load/store under plain -mavx ------------------
        row = ROW_BY_NAME['sa03_asm']
        record(row, probe_asm(
            'sa03', SA03_TU, row.flags,
            must_not_contain=('%zmm', 'vmovdqu64'), diag_is_pass=True,
            diag_hints=('avx512', 'target specific', 'always_inline',
                        'inlining failed', 'immintrin', 'error:')))
        # --- SA-04: shufflelo_epi16 must require AVX-512BW ----------------
        row = ROW_BY_NAME['sa04_diag']
        record(row, probe_diag(
            'sa04', SA04_TU, row.flags,
            ('avx512bw', 'target specific', 'always_inline',
             'inlining failed', 'error:')))
        # --- SA-13: integrated assembler rejects xmm vinserti128 ---------
        row = ROW_BY_NAME['sa13_asm']
        record(row, probe_assembler(
            'sa13', SA13_ASM,
            ('vinserti128', 'operand', 'mismatch', 'invalid', 'unsupported',
             'error:')))
        # --- SA-33: vzeroupper after digit-less EVEX ops ------------------
        row = ROW_BY_NAME['sa33_asm']
        record(row, probe_asm(
            'sa33', SA33_TU, row.flags,
            must_contain=('sa33_f',),
            must_contain_any=(('vinserti32x4', 'vinsertf32x4',
                              'vinserti64x4', 'vinsertf64x4'),),
            ordered_pairs=((('vinserti32x4', 'vinsertf32x4',
                             'vinserti64x4', 'vinsertf64x4'),
                            'vzeroupper'),)))

    for row in report:
        print(f"{row['status']:<15} {row['row']:<14} [{row['item']}] "
              f"flags={row['flags']} actual={row['actual']}")
    if args.json:
        write_json(args.json, {
            'schema': 1, 'driver': 'check_audit_2026-11-08',
            'candidate': cand, 'reference': ref, 'strict': args.strict,
            'host_flags': sorted(host), 'rows': report,
        })
    if hard:
        print(f'{hard} row(s) require action '
              f'({"strict" if args.strict else "regression/error/stale-open"})')
        return 1
    print('audit pack green '
          f'({"strict" if args.strict else "all controls and former findings green"})')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
