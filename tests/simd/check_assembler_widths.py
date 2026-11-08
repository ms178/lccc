#!/usr/bin/env python3
"""Byte-exact integrated-assembler / GNU-as width-parity regression.

SA-13 was a REAL assembler defect: `lccc` accepted
`vinserti128 $1, %xmm1, %xmm2, %xmm3`, an all-XMM spelling that GNU as 2.47
rejects with `operand size mismatch for `vinserti128'`.  The integrated
assembler silently emitted VEX.L=0 bytes, i.e. machine code no CPU decodes as
VINSERTI128.  That is worse than a diagnostic: an assembly source could look
correct and produce a garbage instruction.

This driver checks the six VEX.L=1 half-width families
(vinsert{i,f}128 / vextract{i,f}128 / vperm2{i,f}128) against the *authority*,
GNU as 2.47.20260726, on a table of 14 byte-probed spellings:

  1. accept/reject PARITY for every case;
  2. for every accepted case, the emitted .text section must be BYTE IDENTICAL
     to GNU as (via objcopy --only-section=.text);
  3. a rejected case must be rejected with a diagnostic naming the mnemonic
     (a crash, an empty failure or an unrelated error is a hard failure).

A future GAS version may change a verdict; the table records the exact
binutils version the verdicts were probed against and `--require-gas-247`
makes that an explicit precondition instead of a silent assumption.

    python3 tests/simd/check_assembler_widths.py                # real run
    python3 tests/simd/check_assembler_widths.py --self-test
    python3 tests/simd/check_assembler_widths.py --json receipt.json
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
    from probe_utils import ProbeError, compiler, run_rc, write_json
except ImportError as exc:  # pragma: no cover
    raise SystemExit(f'check_assembler_widths: probe_utils unavailable: {exc}') from exc

GAS_VERSION = '2.47.20260726'

# (name, assembly body instruction, accepted by GNU as 2.47)
@dataclass(frozen=True)
class Case:
    name: str
    line: str
    valid: bool

    def as_dict(self) -> dict:
        return {'name': self.name, 'line': self.line, 'valid': self.valid}


CASES: tuple[Case, ...] = (
    Case('inserti_all_xmm',   'vinserti128 $1, %xmm1, %xmm2, %xmm3', False),
    Case('inserti_dst_xmm',   'vinserti128 $1, %xmm1, %ymm2, %xmm3', False),
    Case('inserti_src_ymm',   'vinserti128 $1, %ymm1, %ymm2, %ymm3', False),
    Case('inserti_valid',     'vinserti128 $1, %xmm1, %ymm2, %ymm3', True),
    Case('inserti_valid_mem', 'vinserti128 $1, (%rax), %ymm2, %ymm3', True),
    Case('extracti_src_xmm',  'vextracti128 $1, %xmm1, %xmm2', False),
    Case('extracti_dst_ymm',  'vextracti128 $1, %ymm1, %ymm2', False),
    Case('extracti_valid',    'vextracti128 $1, %ymm1, %xmm2', True),
    Case('extracti_valid_mem','vextracti128 $1, %ymm1, (%rax)', True),
    Case('insertf_all_xmm',   'vinsertf128 $1, %xmm1, %xmm2, %xmm3', False),
    Case('insertf_valid',     'vinsertf128 $1, %xmm1, %ymm2, %ymm3', True),
    Case('extractf_valid',    'vextractf128 $1, %ymm1, %xmm2', True),
    Case('perm2f_src_xmm',    'vperm2f128 $1, %xmm1, %ymm2, %ymm3', False),
    Case('perm2f_valid',      'vperm2f128 $1, %ymm1, %ymm2, %ymm3', True),
    Case('perm2i_all_xmm',    'vperm2i128 $1, %xmm1, %xmm2, %xmm3', False),
    Case('perm2i_valid',      'vperm2i128 $1, %ymm1, %ymm2, %ymm3', True),
)


def mnemonic_of(line: str) -> str:
    """First token of the instruction, used to validate diagnostics."""
    return line.split()[0].strip()


def classify_case(case: Case, lccc_rc: int, lccc_err: str, gas_rc: int,
                  bytes_equal: bool | None) -> tuple[int, list[str]]:
    """Pure verdict. Used by --self-test and by the real run."""
    problems: list[str] = []
    if (lccc_rc == 0) != (gas_rc == 0) or (lccc_rc == 0) != case.valid:
        problems.append(f'{case.name}: accept/reject mismatch '
                        f'lccc_rc={lccc_rc} gas_rc={gas_rc} expected_valid={case.valid}')
    if lccc_rc != 0:
        m = mnemonic_of(case.line)
        if m not in lccc_err:
            problems.append(f'{case.name}: rejection lacks mnemonic {m!r}: '
                            f'{lccc_err.strip()[:120]!r}')
        if not lccc_err.strip():
            problems.append(f'{case.name}: rejection produced no diagnostic')
    elif bytes_equal is False:
        problems.append(f'{case.name}: .text differs from GNU as {GAS_VERSION}')
    elif bytes_equal is None and case.valid:
        problems.append(f'{case.name}: no byte comparison performed')
    return len(problems), problems


def self_test() -> int:
    checks: list[tuple[str, bool]] = []

    def expect(label: str, got: object, want: object) -> None:
        checks.append((label, got == want))

    expect('cases unique', len({c.name for c in CASES}), len(CASES))
    expect('valid cases exist', sum(c.valid for c in CASES) >= 8, True)
    expect('invalid cases exist', sum(not c.valid for c in CASES) >= 6, True)
    expect('all six families covered',
           {mnemonic_of(c.line) for c in CASES},
           {'vinserti128', 'vextracti128', 'vinsertf128', 'vextractf128',
            'vperm2f128', 'vperm2i128'})
    expect('mnemonic extraction', mnemonic_of('vinserti128 $1, %xmm1, %xmm2, %xmm3'),
           'vinserti128')
    good = next(c for c in CASES if c.name == 'inserti_valid')
    expect('clean valid case', classify_case(good, 0, '', 0, True)[0], 0)
    expect('wrong code on valid case',
           classify_case(good, 0, '', 0, False)[0] >= 1, True)
    expect('missing byte compare',
           classify_case(good, 0, '', 0, None)[0] >= 1, True)
    bad = next(c for c in CASES if c.name == 'inserti_all_xmm')
    expect('clean rejected case',
           classify_case(bad, 1, 'operand size mismatch for `vinserti128\'', 1,
                         None)[0], 0)
    expect('assembler wrongly accepts',
           classify_case(bad, 0, '', 1, None)[0] >= 1, True)
    expect('unrelated diagnostic rejected',
           classify_case(bad, 1, 'error: bad flag', 1, None)[0] >= 1, True)
    expect('silent rejection rejected',
           classify_case(bad, 1, '', 1, None)[0] >= 1, True)
    # Mutation: flipping a verdict must make the classifier complain.
    flipped = Case(good.name, good.line, not good.valid)
    expect('verdict mutation detected',
           classify_case(flipped, 0, '', 0, True)[0] >= 1, True)

    failed = [label for label, ok in checks if not ok]
    for label, ok in checks:
        print(f'{"PASS" if ok else "FAIL"}  {label}')
    print(f'self-test: {len(checks) - len(failed)}/{len(checks)} checks passed')
    return 1 if failed else 0


def resolve_tools(args: argparse.Namespace) -> tuple[list[str], str, str | None, str]:
    """(lccc command, gas path, objcopy path or None, note)."""
    import os
    pfx = os.environ.get('LCCC_AGENT_CACHE_PREFIX', '')
    as_path = args.gas or (str(Path(pfx) / 'bin' / 'as') if pfx else '') \
        or shutil.which('as') or ''
    objcopy = args.objcopy or (str(Path(pfx) / 'bin' / 'objcopy') if pfx else '') \
        or shutil.which('objcopy') or ''
    if not as_path or not Path(as_path).is_file():
        raise ValueError('GNU as not found; pass --gas or set LCCC_AGENT_CACHE_PREFIX')
    cand = compiler(args.lccc)
    if not Path(cand[0]).is_file():
        raise ValueError(f'lccc binary not found: {cand[0]} (build --profile fastbuild)')
    return cand, as_path, objcopy or None, ''


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--lccc', default='target/fastbuild/lccc')
    parser.add_argument('--gas', default='')
    parser.add_argument('--objcopy', default='')
    parser.add_argument('--require-gas-247', action='store_true',
                        help='hard-fail unless GNU as reports the probed version')
    parser.add_argument('--json', type=Path)
    parser.add_argument('--timeout', type=float, default=60.0)
    parser.add_argument('--self-test', action='store_true')
    args = parser.parse_args(argv)
    if args.self_test:
        return self_test()
    try:
        lccc, gas, objcopy, note = resolve_tools(args)
    except ValueError as exc:
        parser.error(str(exc))

    receipt: dict = {'schema': 1, 'driver': 'check_assembler_widths',
                     'gas': gas, 'gas_version_expected': GAS_VERSION,
                     'lccc': lccc, 'cases': [], 'problems': []}
    failures: list[str] = []
    with TemporaryDirectory(prefix='lccc-asm-width-') as work:
        root = Path(work)
        # Version of the authority, recorded rather than assumed.
        vrc, vout, verr = run_rc([gas, '--version'], root, args.timeout)
        version = (vout or verr).splitlines()[0] if (vout or verr) else ''
        receipt['gas_version_actual'] = version
        if args.require_gas_247 and GAS_VERSION not in version:
            print(f'ERROR GNU as version {version!r} != probed {GAS_VERSION!r}')
            return 1
        for case in CASES:
            src = root / f'{case.name}.s'
            src.write_text('.text\n.globl p\np:\n    ' + case.line + '\n    ret\n',
                           encoding='utf-8')
            lo, go = root / f'{case.name}.lccc.o', root / f'{case.name}.gas.o'
            try:
                lrc, _, lerr = run_rc([*lccc, '-c', str(src), '-o', str(lo)],
                                      root, args.timeout)
            except ProbeError as exc:
                lrc, lerr = -1, str(exc)
            try:
                grc, _, gerr = run_rc([gas, str(src), '-o', str(go)],
                                      root, args.timeout)
            except ProbeError as exc:
                grc, gerr = -1, str(exc)
            bytes_equal: bool | None = None
            if lrc == 0 and grc == 0 and objcopy and Path(objcopy).is_file():
                lb, gb = root / f'{case.name}.lccc.bin', root / f'{case.name}.gas.bin'
                ok = True
                for exe, obj, out in ((objcopy, lo, lb), (objcopy, go, gb)):
                    rc, _, _ = run_rc([exe, '-O', 'binary', '--only-section=.text',
                                       str(obj), str(out)], root, args.timeout)
                    ok &= rc == 0 and out.is_file()
                if ok:
                    bytes_equal = lb.read_bytes() == gb.read_bytes()
            n, problems = classify_case(case, lrc, lerr, grc, bytes_equal)
            for prob in problems:
                failures.append(prob)
                print(f'FAIL  {prob}')
            status = 'PASS' if n == 0 else 'FAIL'
            if n == 0:
                extra = 'byte-identical' if bytes_equal else 'rejected'
                print(f'PASS  {case.name}: {extra}')
            receipt['cases'].append({'name': case.name, 'line': case.line,
                                     'valid': case.valid, 'lccc_rc': lrc,
                                     'gas_rc': grc, 'byte_identical': bytes_equal,
                                     'status': status, 'problems': problems})
    receipt['summary'] = {'cases': len(CASES), 'problems': len(failures),
                          'byte_compared': sum(1 for c in receipt['cases']
                                               if c['byte_identical'] is True)}
    receipt['problems'] = failures
    if args.json:
        write_json(args.json, receipt)
    print(f'assembler widths: {len(CASES)} cases, '
          f'{receipt["summary"]["byte_compared"]} byte-identical encodings, '
          f'{len(failures)} problems')
    return 1 if failures else 0


if __name__ == '__main__':
    sys.exit(main())
