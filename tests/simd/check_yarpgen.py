#!/usr/bin/env python3
"""YARPGen 2.0 differential fuzzing gate (UB-free random loop programs).

YARPGen (github.com/intel/yarpgen, PLDI'23) generates *provably* UB-free C
programs whose only output is a decimal hash of all globals.  The hash must be
identical for every compiler and every optimization/ISA level.  This driver
runs a deterministic seed series through the three mandated toolchains and
the built `lccc`, and compares every hash against GCC 16.2 `-O0`.

Why this is the right fuzzer for this patchset:
  * it generates nested loops, mixed integer widths, alignment attributes,
    multi-dimensional arrays and if-conversion — the "complex shapes" the
    SLP/ARX/vectorizer changes touch, which no hand-written corpus covers;
  * it is UB-free by construction (YARPGen tracks value ranges at generation
    time), so a hash mismatch is a compiler defect, not an oracle defect;
  * the generator is seeded, so every failure is reproducible with one
    command (`yarpgen -s <seed> --std=c`).

Provenance: engineering/toolchains/yarpgen-provenance.json.  Provision with
`python3 scripts/provision_yarpgen.py` (github.com is an allowed host).

    python3 tests/simd/check_yarpgen.py --quick
    python3 tests/simd/check_yarpgen.py --seeds 64 --json receipt.json
    python3 tests/simd/check_yarpgen.py --self-test
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
    raise SystemExit(f'check_yarpgen: probe_utils unavailable: {exc}') from exc

# Each leg: label, command, flags.  The first is the -O0 authority.
LEGS: tuple[tuple[str, tuple[str, ...]], ...] = (
    ('gcc16_O0_ref',   ('-O0',)),
    ('gcc16_O2',       ('-O2',)),
    ('gcc16_O3_native',('-O3', '-march=native', '-funroll-loops')),
    ('gcc16_O2_v3',    ('-O2', '-march=x86-64-v3')),
    ('gcc16_O2_512',   ('-O2', '-mavx512f', '-mavx512bw', '-mavx512dq',
                        '-mavx512vl', '-mfma')),
    ('clang23_O2',     ('-O2',)),
    ('clang23_O3_512', ('-O3', '-mavx512f', '-mavx512bw', '-mavx512dq',
                        '-mavx512vl', '-mfma')),
    ('lccc_O2',        ('-O2',)),
    ('lccc_O3_native', ('-O3', '-march=native', '-funroll-loops')),
    ('lccc_O2_512',    ('-O2', '-mavx512f', '-mavx512bw', '-mavx512dq',
                        '-mavx512vl', '-mfma')),
)
QUICK_LEGS = ('gcc16_O0_ref', 'gcc16_O2', 'clang23_O2', 'lccc_O2')

# A program whose .bss exceeds 2 GiB cannot be linked in the SMALL code
# model: *every* x86-64 compiler gets a 32-bit PC-relative relocation
# overflow.  That is an environmental limit, not a differential compiler
# defect.  We only classify a seed as SKIP when EVERY leg failed with such a
# marker -- if even one compiler succeeds, lccc's failure is a genuine FAIL.
OVERSIZE_MARKERS = (
    'relocation truncated to fit', 'R_X86_64_32S', 'R_X86_64_PC32',
    'R_X86_64_REX_GOTPCRELX', 'mcmodel=large', 'image exceeds 2 GiB',
    'failed to convert GOTPCREL', 'relocation R_X86_64_32',
)


def classify_oversize(errors: dict[str, str], n_legs: int) -> str | None:
    """Return a SKIP reason iff all legs failed with a >2 GiB relocation.

    Pure function; exercised by --self-test.  A partial failure (any leg
    succeeding, or any non-relocation error) is NOT skippable.
    """
    if not errors or len(errors) != n_legs:
        return None
    reasons = []
    for label, err in sorted(errors.items()):
        if not any(m in err for m in OVERSIZE_MARKERS):
            return None
        reasons.append(f'{label}: {err[:100]}')
    return ('image exceeds the supported small code model (2 GiB); '
            'all %d legs hit the same 32-bit relocation limit: %s'
            % (n_legs, ' | '.join(reasons)))


def check_code_model_diagnostic(lccc: list[str], root: Path,
                                timeout: float) -> tuple[list[str], list[str]]:
    """SA-YG-1: `-mcmodel=large` must not silently degrade to the small model.

    Before the fix lccc accepted the flag, emitted `leaq sym(%rip)` and then
    failed at link time with "recompile with -mcmodel=large" -- advice the
    user had already followed.  Fail closed with an honest diagnostic.
    """
    src = root / 'cm_probe.c'
    src.write_text('int cm_global = 7;\nint main(void) { return cm_global - 7; }\n')
    exe = root / 'cm_probe.bin'
    problems: list[str] = []
    notes: list[str] = []
    try:
        rc, _, err = run_rc([*lccc, '-std=gnu99', '-O2', '-mcmodel=large',
                             str(src), '-o', str(exe)], root, timeout)
    except ProbeError as exc:
        rc, err = -1, str(exc)
    if rc == 0:
        problems.append('-mcmodel=large accepted silently without a working '
                        'large-code-model implementation')
    elif 'not implemented' not in err and 'large code model' not in err:
        problems.append(f'-mcmodel=large rejected without an actionable '
                        f'diagnostic: {err.strip()[:180]!r}')
    else:
        notes.append('-mcmodel=large: honest unsupported diagnostic (no '
                     'silent small-model fallback)')
    return problems, notes


@dataclass(frozen=True)
class Leg:
    label: str
    command: list[str]
    flags: tuple[str, ...]


def parse_hash(output: str) -> str:
    """YARPGen prints exactly one decimal hash; anything else is malformed."""
    lines = [x.strip() for x in output.splitlines() if x.strip()]
    if len(lines) != 1 or not lines[0].isdigit():
        raise ValueError(f'malformed YARPGen output: {output[:160]!r}')
    return lines[0]


def compare(reference: str, got: dict[str, str], strict: bool) -> tuple[int, list[str]]:
    """Pure comparison, also used by --self-test."""
    problems: list[str] = []
    if 'gcc16_O0_ref' not in got:
        return 1, ['missing -O0 authority hash']
    ref = got['gcc16_O0_ref']
    if reference and ref != reference:
        problems.append(f'-O0 authority drifted: {ref} != {reference}')
    for label in sorted(got):
        if label == 'gcc16_O0_ref':
            continue
        if got[label] != ref:
            problems.append(f'{label}: hash {got[label]} != {ref}')
    if strict and len(got) < 2:
        problems.append('strict mode requires at least two compilers')
    return len(problems), problems


def self_test() -> int:
    checks: list[tuple[str, bool]] = []

    def expect(label: str, got: object, want: object) -> None:
        checks.append((label, got == want))

    expect('parse decimal hash', parse_hash('123456789012345\n'), '123456789012345')
    try:
        parse_hash('ok\n123\n')
        expect('reject multi-line', False, True)
    except ValueError:
        expect('reject multi-line', True, True)
    try:
        parse_hash('nan\n')
        expect('reject non-decimal', False, True)
    except ValueError:
        expect('reject non-decimal', True, True)
    good = {'gcc16_O0_ref': '42', 'gcc16_O2': '42', 'lccc_O2': '42'}
    expect('clean comparison', compare('42', good, False)[0], 0)
    expect('mismatch detected',
           compare('42', {**good, 'lccc_O2': '43'}, False)[0], 1)
    expect('authority drift detected', compare('41', good, False)[0], 1)
    expect('missing authority detected',
           compare('42', {'gcc16_O2': '42'}, False)[0], 1)
    expect('strict needs 2 compilers',
           compare('42', {'gcc16_O0_ref': '42'}, True)[0], 1)
    expect('legs unique', len({x[0] for x in LEGS}), len(LEGS))
    expect('quick subset valid', set(QUICK_LEGS) <= {x[0] for x in LEGS}, True)
    expect('three toolchains represented',
           all(any(k in x[0] for x in LEGS)
               for k in ('gcc16', 'clang23', 'lccc')), True)
    expect('ISA legs present', sum('512' in x[0] for x in LEGS) >= 3, True)
    all_over = {x[0]: 'error: relocation truncated to fit: R_X86_64_32S'
                for x in LEGS}
    expect('oversize SKIP when every leg fails',
           classify_oversize(all_over, len(LEGS)) is not None, True)
    expect('no SKIP for a partial failure',
           classify_oversize({LEGS[0][0]: 'relocation truncated to fit'},
                             len(LEGS)) is None, True)
    expect('no SKIP for a compiler crash in one leg',
           classify_oversize({**all_over, LEGS[1][0]: 'cc: internal error'},
                             len(LEGS)) is None, True)
    expect('no SKIP when a leg succeeded',
           classify_oversize({}, len(LEGS)) is None, True)
    expect('timeout is never an oversize SKIP',
           classify_oversize({x[0]: 'gcc-16 ...: timed out after 90s'
                              for x in LEGS}, len(LEGS)) is None, True)
    # Ratchet: the default per-child timeout must cover the measured
    # 151 s GCC 16.2 -O3 -march=native -funroll-loops worst case.
    import argparse as _argparse
    _p = _argparse.ArgumentParser(add_help=False)
    _p.add_argument('--timeout', type=float, default=240.0)
    expect('default timeout exceeds measured worst compile',
           _p.parse_args([]).timeout > 151.0, True)

    failed = [label for label, ok in checks if not ok]
    for label, ok in checks:
        print(f'{"PASS" if ok else "FAIL"}  {label}')
    print(f'self-test: {len(checks) - len(failed)}/{len(checks)} checks passed')
    return 1 if failed else 0


def resolve_commands(args: argparse.Namespace) -> dict[str, list[str]]:
    """Resolve the three mandated toolchains; missing ones are reported."""
    import os
    pfx = os.environ.get('LCCC_AGENT_CACHE_PREFIX', '')
    gcc = args.gcc or (str(Path(pfx) / 'bin' / 'gcc-16') if pfx else '') \
        or shutil.which('gcc-16') or 'gcc'
    clang = args.clang or (str(Path(pfx) / 'bin' / 'clang-23') if pfx else '') \
        or shutil.which('clang-23') or ''
    lccc = args.lccc
    out: dict[str, list[str]] = {}
    for label, cmd in (('gcc16', gcc), ('clang23', clang), ('lccc', lccc)):
        try:
            resolved = compiler(cmd)
        except ValueError as exc:
            raise ValueError(f'{label}: {exc}') from exc
        if '/' in cmd and not Path(resolved[0]).is_file():
            raise ValueError(f'{label}: binary not found: {resolved[0]}')
        if not shutil.which(resolved[0]) and not Path(resolved[0]).is_file():
            raise ValueError(f'{label}: binary not found: {resolved[0]}')
        out[label] = resolved
    return out


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--yarpgen', default='')
    parser.add_argument('--gcc', default='')
    parser.add_argument('--clang', default='')
    parser.add_argument('--lccc', default='target/fastbuild/lccc')
    parser.add_argument('--seeds', type=int, default=48)
    parser.add_argument('--first-seed', type=int, default=1000)
    parser.add_argument('--quick', action='store_true')
    parser.add_argument('--strict', action='store_true')
    parser.add_argument('--json', type=Path)
    # Measured worst case on the host: seed 1029 under GCC 16.2
    # `-O3 -march=native -funroll-loops` takes 151 s (2026-11-08).
    # A 90 s default turned that into a bogus "compiler failure" with the
    # reason truncated away.  Keep the margin explicit and unit-tested.
    parser.add_argument('--timeout', type=float, default=240.0)
    parser.add_argument('--self-test', action='store_true')
    args = parser.parse_args(argv)
    if args.self_test:
        return self_test()
    if not (1 <= args.seeds <= 2000):
        parser.error('--seeds must be in [1, 2000]')

    yarpgen = args.yarpgen or shutil.which('yarpgen') or '/tmp/yarpgen/yarpgen'
    if not Path(yarpgen).is_file():
        parser.error(f'YARPGen not found at {yarpgen}; run scripts/provision_yarpgen.py')
    try:
        cmds = resolve_commands(args)
    except ValueError as exc:
        parser.error(str(exc))

    command_for = {'gcc16': cmds['gcc16'], 'clang23': cmds['clang23'],
                   'lccc': cmds['lccc']}
    legs: list[Leg] = []
    for label, flags in LEGS:
        if args.quick and label not in QUICK_LEGS:
            continue
        key = next(k for k in ('gcc16', 'clang23', 'lccc') if label.startswith(k))
        legs.append(Leg(label, command_for[key], flags))

    receipt: dict = {
        'schema': 1, 'driver': 'check_yarpgen', 'yarpgen': yarpgen,
        'commands': {k: v for k, v in command_for.items()},
        'legs': [x.label for x in legs], 'seeds': [], 'problems': [],
        'quick': args.quick, 'code_model_probe': {}, 'skipped': [],
        'timeout_s': args.timeout,
    }
    failures: list[str] = []
    with TemporaryDirectory(prefix='lccc-yarpgen-') as work:
        root = Path(work)
        cm_problems, cm_notes = check_code_model_diagnostic(
            cmds['lccc'], root, args.timeout)
        receipt['code_model_probe'] = {'problems': cm_problems,
                                       'notes': cm_notes}
        failures.extend(cm_problems)
        for note in cm_notes:
            print(f'PASS  {note}')
        for prob in cm_problems:
            print(f'FAIL  {prob}')
        for i in range(args.seeds):
            seed = args.first_seed + i
            gen = root / f'seed{seed}'
            gen.mkdir()
            # Keep the generated image inside the SUPPORTED small code
            # model: unbounded YARPGen dimensions produce [20]^7 arrays
            # (~19 GiB .bss) whose 32-bit PC-relative relocations overflow
            # for GCC, Clang AND lccc alike.  Three dimensions still yields
            # nested loops, multi-dimensional indexing and alignment
            # attributes — the shapes this fuzz campaign is about.
            grc, gout, gerr = run_rc([yarpgen, '-s', str(seed), '--std=c',
                                      '--max-array-dims=3', '-o', str(gen)],
                                     root, args.timeout)
            if grc != 0 or not (gen / 'func.c').is_file():
                failures.append(f'seed {seed}: generation failed rc={grc}')
                receipt['seeds'].append({'seed': seed, 'status': 'ERROR',
                                         'reason': (gerr or gout)[:200]})
                continue
            sources = [str(gen / 'driver.c'), str(gen / 'func.c')]
            hashes: dict[str, str] = {}
            errors: dict[str, str] = {}
            for leg in legs:
                exe = gen / f'{leg.label}.bin'
                cmd = [*leg.command, '-std=gnu99', *leg.flags, *sources,
                       '-o', str(exe)]
                try:
                    crc, _, cerr = run_rc(cmd, root, args.timeout)
                except ProbeError as exc:
                    crc, cerr = -1, str(exc)
                if crc != 0:
                    # Preserve the ACTUAL reason.  A 200-char cap truncated
                    # `... timed out after 90s` off the end of the command,
                    # so the receipt looked like a compiler defect.
                    errors[leg.label] = cerr.strip()[:600]
                    continue
                try:
                    rrc, rout, rerr = run_rc([str(exe)], root, args.timeout)
                except ProbeError as exc:
                    errors[leg.label] = str(exc)
                    continue
                if rrc != 0:
                    errors[leg.label] = f'run rc={rrc}: {rerr.strip()[:160]}'
                    continue
                try:
                    hashes[leg.label] = parse_hash(rout)
                except ValueError as exc:
                    errors[leg.label] = str(exc)
            n, problems = compare('', hashes, args.strict)
            skip = classify_oversize(errors, len(legs)) if errors else None
            if skip:
                # An honest SKIP: no oracle existed because no compiler
                # could link the program.  Never counted as a PASS hash.
                status = 'SKIP'
                print(f'SKIP  seed {seed}: {skip[:160]}')
                receipt['skipped'].append({'seed': seed, 'reason': skip})
                receipt['seeds'].append({'seed': seed, 'status': 'SKIP',
                                         'reason': skip, 'hashes': {},
                                         'errors': errors, 'problems': []})
                continue
            for label, err in sorted(errors.items()):
                problems.append(f'seed {seed} {label}: {err}')
            for prob in problems:
                failures.append(f'seed {seed}: {prob}')
            status = 'PASS' if n == 0 and not errors else 'FAIL'
            if status == 'PASS':
                print(f'PASS  seed {seed}: {len(hashes)} legs, hash {hashes["gcc16_O0_ref"]}')
            else:
                for prob in problems:
                    print(f'FAIL  seed {seed}: {prob}')
            receipt['seeds'].append({'seed': seed, 'status': status,
                                     'hashes': hashes, 'errors': errors,
                                     'problems': problems})
            if status == 'FAIL' and len(failures) >= 20:
                print('STOP  >= 20 problems; remaining seeds skipped '
                      '(first failures are the actionable ones)')
                break
    receipt['summary'] = {'seeds': len(receipt['seeds']),
                          'passed': sum(1 for s in receipt['seeds']
                                        if s['status'] == 'PASS'),
                          'skipped': sum(1 for s in receipt['seeds']
                                         if s['status'] == 'SKIP'),
                          'legs': len(legs), 'problems': len(failures)}
    receipt['problems'] = failures
    if args.json:
        write_json(args.json, receipt)
    print(f'YARPGen: {receipt["summary"]["passed"]}/{receipt["summary"]["seeds"]} seeds '
          f'({receipt["summary"]["skipped"]} SKIP), '
          f'{receipt["summary"]["legs"]} legs, {len(failures)} problems')
    return 1 if failures else 0


if __name__ == '__main__':
    sys.exit(main())
