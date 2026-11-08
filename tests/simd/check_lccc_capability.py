#!/usr/bin/env python3
"""Pinned LCCC capability-gap ratchet for the 42-shape SIMD ISA corpus.

`check_simd_isa_matrix.py` is a *reference-toolchain* gate: it proves the C
corpus is warning-free and bit-exact under GCC 16.2 / Clang 23.1.5 / QEMU
cross legs with a strict `-Werror` flag set.  Running that same flag set and
corpus against `lccc` currently produces diagnostics for capabilities LCCC
has not implemented yet.  Those are REAL defects; hiding them (dropping
flags, filtering stderr, or calling them SKIP) would be dishonest.

This driver makes the debt explicit, machine-readable and ratcheted:

  * it compiles `simd_isa_matrix.c` with the exact strict flag set under
    `lccc`, and classifies EVERY stderr line;
  * the observed gap set is compared to a pinned golden.  A NEW gap is a
    hard failure (regression).  A gap that DISAPPEARS is also a hard failure
    until the golden is deliberately updated -- so a fix cannot be credited
    without a reviewed receipt;
  * any diagnostic that is not a known gap shape is a hard failure.  Nothing
    is silently swallowed.

The three pinned gaps as of 2026-11-08 (measured with the fastbuild LCCC
16.2.0, GCC 16.2.0, Clang 23.1.5, GNU as 2.47.20260726):

  * four GCC warning groups LCCC has no implementation for
    (`-Wconversion`, `-Wshadow`, `-Wcast-qual`, `-Wwrite-strings`).  LCCC's
    GP-22 option-effect ledger deliberately refuses to accept an option
    inertly, so it warns by name; `LCCC_STRICT_OPTIONS=1` makes that fatal.
  * three intrinsics absent from `include/lcccsimd.h`
    (`_mm512_mask_add_epi32`, `_mm512_mullo_epi64`, `_mm_mask_add_epi32`).

The matching acceptance gates (all green in the receipt) are the GCC16 and
Clang23 `check_simd_isa_matrix.py` runs and `check_audit_2026-11-08.py
--strict`.

    python3 tests/simd/check_lccc_capability.py --self-test
    python3 tests/simd/check_lccc_capability.py --json receipt.json
"""
from __future__ import annotations

import argparse
from dataclasses import dataclass
from pathlib import Path
import re
import shutil
import sys
from tempfile import TemporaryDirectory

_SCRIPT_DIR = str(Path(__file__).resolve().parent)
if _SCRIPT_DIR not in sys.path:
    sys.path.insert(0, _SCRIPT_DIR)

try:
    from probe_utils import ProbeError, run_rc, write_json
except ImportError as exc:  # pragma: no cover
    raise SystemExit(f'check_lccc_capability: probe_utils unavailable: {exc}') from exc

ROOT = Path(__file__).resolve().parents[2]
CORPUS = ROOT / 'tests' / 'simd' / 'simd_isa_matrix.c'
GOLDEN = ROOT / 'tests' / 'simd' / 'lccc_capability_gaps.json'

# The exact strict flag set used by check_simd_isa_matrix.py (AVX-512 tier).
STRICT_FLAGS = (
    '-std=c99', '-O3', '-mavx512f', '-mavx512bw', '-mavx512dq',
    '-mavx512vl', '-mavx512vnni', '-mfma',
    '-Wall', '-Wextra', '-Werror', '-Wconversion', '-Wshadow',
    '-Wstrict-prototypes', '-Wold-style-definition', '-Wmissing-prototypes',
    '-Wcast-qual', '-Wwrite-strings', '-Wundef', '-Wpedantic',
    '-Wno-unused-parameter',
)

TOLERATED = re.compile(
    r"unrecognized command-line option '(-[^']+)' is tolerated but not implemented"
)
MISSING_INTRINSIC = re.compile(
    r"implicit declaration of function '([A-Za-z_][A-Za-z0-9_]*)'"
)
# The frontend's error-count trailer is not itself a capability gap.
TRAILER = re.compile(r'^\s*ccc: error: \d+ error\(s\) during semantic analysis\s*$')
# Diagnostic printers echo the offending source line and a caret row; those
# carry no `error:`/`warning:` token and are context, not separate findings.
DIAGNOSTIC_TOKEN = re.compile(r'(?:error|warning)\s*:')


@dataclass(frozen=True)
class Gaps:
    unsupported_options: frozenset[str]
    missing_intrinsics: frozenset[str]
    unexpected: tuple[str, ...]

    def as_dict(self) -> dict:
        return {
            'unsupported_options': sorted(self.unsupported_options),
            'missing_intrinsics': sorted(self.missing_intrinsics),
            'unexpected': list(self.unexpected),
        }

    def key(self) -> tuple[tuple[str, ...], tuple[str, ...]]:
        return (tuple(sorted(self.unsupported_options)),
                tuple(sorted(self.missing_intrinsics)))


def classify_diagnostics(stderr: str) -> Gaps:
    """Pure classifier.  Every non-gap diagnostic lands in `unexpected`.

    Deliberately strict: a new warning class (a real compiler warning about
    the corpus, a crash, a linker error) is NOT a capability gap and must
    never be absorbed into the golden.
    """
    opts: set[str] = set()
    missing: set[str] = set()
    unexpected: list[str] = []
    for raw in stderr.splitlines():
        line = raw.strip()
        if not line:
            continue
        m = TOLERATED.search(line)
        if m:
            opts.add(m.group(1))
            continue
        m = MISSING_INTRINSIC.search(line)
        if m:
            missing.add(m.group(1))
            continue
        if TRAILER.match(line) or not DIAGNOSTIC_TOKEN.search(line):
            continue
        unexpected.append(line)
    return Gaps(frozenset(opts), frozenset(missing), tuple(unexpected))


def evaluate(got: Gaps, golden: Gaps) -> tuple[int, list[str]]:
    """Pure ratchet.  Both directions are hard: no silent gaps in/out."""
    problems: list[str] = []
    for problem in got.unexpected:
        problems.append(f'unclassified diagnostic: {problem[:200]}')
    new_opts = sorted(got.unsupported_options - golden.unsupported_options)
    new_intr = sorted(got.missing_intrinsics - golden.missing_intrinsics)
    fixed_opts = sorted(golden.unsupported_options - got.unsupported_options)
    fixed_intr = sorted(golden.missing_intrinsics - got.missing_intrinsics)
    for name in new_opts:
        problems.append(f'NEW unsupported-option gap: {name}')
    for name in new_intr:
        problems.append(f'NEW missing intrinsic: {name}')
    for name in fixed_opts:
        problems.append(f'gap resolved ({name}): review and re-baseline the golden')
    for name in fixed_intr:
        problems.append(f'gap resolved ({name}): review and re-baseline the golden')
    return len(problems), problems


def self_test() -> int:
    checks: list[tuple[str, bool]] = []

    def expect(label: str, got: object, want: object) -> None:
        checks.append((label, got == want))

    empty = classify_diagnostics('')
    expect('empty diagnostics', empty.key(), ((), ()))
    sample = (
        "warning: unrecognized command-line option '-Wconversion' is tolerated "
        "but not implemented by LCCC (nothing will act on it)\n"
        "tests/simd/simd_isa_matrix.c:482:29: error: implicit declaration of "
        "function '_mm512_mask_add_epi32' [-Wimplicit-function-declaration]\n"
        "                             _mm512_mask_add_epi32(a, k, a, b));\n"
        "                             ^~~~~~~~~~~~~~~~~~~~~\n"
        "ccc: error: 1 error(s) during semantic analysis\n"
    )
    g = classify_diagnostics(sample)
    expect('option classified', sorted(g.unsupported_options), ['-Wconversion'])
    expect('intrinsic classified', sorted(g.missing_intrinsics),
           ['_mm512_mask_add_epi32'])
    expect('no false unexpected', g.unexpected, ())
    golden = Gaps(frozenset({'-Wconversion'}),
                  frozenset({'_mm512_mask_add_epi32'}), ())
    expect('exact golden passes', evaluate(g, golden)[0], 0)
    more = classify_diagnostics(sample.replace('-Wconversion', '-Wbogus'))
    # Renaming a gap is BOTH a new gap and an unexplained
    # resolution, so the count is >= 1 by construction.
    expect('new gap detected', evaluate(more, golden)[0] > 0, True)
    fewer = classify_diagnostics('')
    expect('disappeared gap detected', evaluate(fewer, golden)[0] > 0, True)
    crash = classify_diagnostics(sample + 'lccc: internal compiler error: panic\n')
    expect('crash is never a gap', evaluate(crash, golden)[0] > 0, True)
    expect('crash lands in unexpected', crash.unexpected[0],
           'lccc: internal compiler error: panic')
    expect('duplicate lines deduplicated',
           classify_diagnostics(sample + sample).key(), g.key())
    expect('warning about the corpus is not absorbed',
           classify_diagnostics('x.c:1:1: warning: unused variable [-Wunused]')
           .unexpected[0].startswith('x.c:'), True)

    failed = [label for label, ok in checks if not ok]
    for label, ok in checks:
        print(f'{"PASS" if ok else "FAIL"}  {label}')
    print(f'self-test: {len(checks) - len(failed)}/{len(checks)} checks passed')
    return 1 if failed else 0


def load_golden(path: Path) -> Gaps:
    import json
    data = json.loads(path.read_text())
    return Gaps(frozenset(data['unsupported_options']),
                frozenset(data['missing_intrinsics']), ())


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--lccc', default='target/fastbuild/lccc')
    ap.add_argument('--golden', type=Path, default=GOLDEN)
    ap.add_argument('--json', type=Path)
    ap.add_argument('--timeout', type=float, default=120.0)
    ap.add_argument('--self-test', action='store_true')
    args = ap.parse_args(argv)
    if args.self_test:
        return self_test()

    # probe_utils runs children with cwd=tempdir; a relative compiler path
    # must be resolved against the repository root first.
    raw_lccc = args.lccc
    lccc = str(Path(raw_lccc).resolve()) if '/' in raw_lccc else raw_lccc
    if '/' in lccc and not Path(lccc).is_file():
        print(f'FAIL  LCCC binary not found: {lccc}', file=sys.stderr)
        return 1
    if '/' not in lccc and not shutil.which(lccc):
        print(f'FAIL  LCCC binary not found on PATH: {lccc}', file=sys.stderr)
        return 1
    if not CORPUS.is_file():
        print(f'FAIL  corpus missing: {CORPUS}', file=sys.stderr)
        return 1
    try:
        golden = load_golden(args.golden)
    except (OSError, ValueError, KeyError) as exc:
        print(f'FAIL  golden unreadable: {exc}', file=sys.stderr)
        return 1

    receipt: dict = {
        'schema': 1, 'driver': 'check_lccc_capability', 'lccc': lccc,
        'corpus': str(CORPUS.relative_to(ROOT)), 'flags': list(STRICT_FLAGS),
    }
    with TemporaryDirectory(prefix='lccc-capability-') as work:
        root = Path(work)
        obj = root / 'matrix.o'
        cmd = [lccc, *STRICT_FLAGS, '-c', str(CORPUS), '-o', str(obj)]
        try:
            rc, _, err = run_rc(cmd, root, args.timeout)
        except ProbeError as exc:
            print(f'FAIL  compile probe crashed: {exc}', file=sys.stderr)
            return 1
        got = classify_diagnostics(err)
        receipt['compile_rc'] = rc
        receipt['observed'] = got.as_dict()
        n, problems = evaluate(got, golden)
        receipt['problems'] = problems
        receipt['golden'] = {
            'unsupported_options': sorted(golden.unsupported_options),
            'missing_intrinsics': sorted(golden.missing_intrinsics),
        }
        for problem in problems:
            print(f'FAIL  {problem}')
        if n == 0:
            print(f'PASS  pinned capability gaps unchanged: '
                  f'{len(got.unsupported_options)} options, '
                  f'{len(got.missing_intrinsics)} intrinsics, '
                  f'0 unclassified diagnostics')
            for name in sorted(got.unsupported_options):
                print(f'      unsupported option: {name}')
            for name in sorted(got.missing_intrinsics):
                print(f'      missing intrinsic: {name}')
    if args.json:
        write_json(args.json, receipt)
    return 1 if n else 0


if __name__ == '__main__':
    sys.exit(main())
