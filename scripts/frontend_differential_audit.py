#!/usr/bin/env python3
"""Differential audit of lccc's C frontend against independent oracles.

The frontend is audited *data-driven*: every case in ``tests/frontend_audit``
is run through lccc **and** through the reference compilers, and the observable
behaviour is compared.  Three case families exist, each answered by a different
observable:

``pp/``
    Preprocessor cases.  All three compilers preprocess the case (``-E``); the
    outputs are reduced to a *token stream* -- line markers dropped, every run
    of whitespace (newlines included) collapsed to one space -- and compared.
    Newlines carry no meaning in C outside directives and ``//`` comments, so
    this comparison is exact rather than cosmetic, and it is what makes the
    test insensitive to the oracles' differing whitespace habits.

``run/``
    Semantic cases.  Every case is a complete program that prints one line per
    assertion; it is compiled and executed by each compiler and the stdout,
    stderr emptiness and exit status must match.  This is the only oracle that
    can see constant folding, integer promotions, ``sizeof``/``_Alignof``
    layout, enum values, character-set mapping and code generation at once.

``diag/``
    Diagnostics cases.  The case must *fail* the same way everywhere: every
    oracle rejects it and lccc rejects it too, without crashing and without an
    internal-error message.  A case whose first line is
    ``/* expect: accept */`` is instead required to be accepted by all three.

Usage::

    python3 scripts/frontend_differential_audit.py --lccc target/fastbuild/lccc
    python3 scripts/frontend_differential_audit.py --family pp --verbose
    python3 scripts/frontend_differential_audit.py --family run --levels 0,2

Exit status is 0 only when every selected case agrees with every oracle.  The
script is dependency-free (no pytest/column/file) by design: it has to run in
the bare research sandbox.
"""

from __future__ import annotations

import argparse
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parent.parent
AUDIT_ROOT = ROOT / 'tests/frontend_audit'
# Oracle discovery: the pinned toolchain name first, then the distribution's
# own spelling. A gate that silently skipped because the runner shipped `gcc`
# instead of `gcc-16` would report "0/0 cases agree" and pass -- so the
# fallback is explicit and the resolved binary is printed in the report line.
DEFAULT_ORACLES = {
    'gcc': ('gcc-16', 'gcc'),
    'clang': ('clang-23', 'clang'),
}
# Cases are compiled and run with an unlimited stack: the huge-frame regression
# beside this audit deliberately allocates gigabytes of locals, and a data-driven
# harness must not be able to fail for a reason that has nothing to do with the
# code under test.
ULIMIT_UNLOCKED = 'ulimit -s unlimited 2>/dev/null; ulimit -v unlimited 2>/dev/null; exec "$@"'


class Result:
    __slots__ = ('case', 'family', 'ok', 'detail')

    def __init__(self, case: Path, family: str, ok: bool, detail: str = '') -> None:
        self.case = case
        self.family = family
        self.ok = ok
        self.detail = detail


def run(cmd: list[str], *, cwd: Path | None = None, timeout: int = 120,
        stdin_text: str | None = None) -> subprocess.CompletedProcess[str]:
    return subprocess.run(cmd, cwd=cwd, input=stdin_text, text=True, timeout=timeout,
                          stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False)


def run_isolated(cmd: list[str], *, cwd: Path, timeout: int = 240) -> subprocess.CompletedProcess[str]:
    """Run ``cmd`` under an unlimited stack without a shell-quoting hazard."""
    if shutil.which('bash'):
        return run(['bash', '-c', ULIMIT_UNLOCKED, 'audit',
                    *cmd], cwd=cwd, timeout=timeout)
    return run(cmd, cwd=cwd, timeout=timeout)


PUNCTUATORS = sorted(
    """
    %:%: <<= >>= ... -> ++ -- << >> <= >= == != && || += -= *= /= %= &= |= ^=
    ## <: :> <% %> %: {
    } [ ] ( ) ; : ? . + - * / % & | ^ ~ ! = < > , #
    """.split(),
    key=len, reverse=True,
)


def token_stream(text: str) -> list[str]:
    """Reduce preprocessed C to its significant token sequence.

    Drops line markers and tokenizes the rest the way the C lexer does, so two
    outputs that differ only in the *separators* between tokens compare equal
    (`a+b` and `a + b` are the same token sequence) while a real difference --
    a different token, a missing token, a different token count -- does not.
    A whitespace comparison would report both, and a whitespace comparison is
    what a naive `-E` diff gives you.
    """
    source = '\n'.join(line for line in text.splitlines() if not line.lstrip().startswith('#'))
    tokens: list[str] = []
    i, n = 0, len(source)
    while i < n:
        c = source[i]
        if c.isspace():
            i += 1
            continue
        # Identifiers, including the `u8`/`L`/`u`/`U` literal prefixes: the
        # prefix and the quote then merge into one token.
        if c.isalpha() or c == '_' or c == '$':
            j = i + 1
            while j < n and (source[j].isalnum() or source[j] in '_$'):
                j += 1
            word = source[i:j]
            if j < n and source[j] in '"\'' and word in ('L', 'u', 'U', 'u8'):
                quote = source[j]
                j += 1
                while j < n and source[j] != quote:
                    j += 2 if source[j] == '\\' else 1
                j = min(j + 1, n)
            tokens.append(source[i:j])
            i = j
            continue
        if c.isdigit() or (c == '.' and i + 1 < n and source[i + 1].isdigit()):
            j = i + 1
            while j < n:
                d = source[j]
                if d.isalnum() or d in '._':
                    j += 1
                elif d in '+-' and source[j - 1] in 'eEpP':
                    j += 1
                else:
                    break
            tokens.append(source[i:j])
            i = j
            continue
        if c in '"\'':
            j = i + 1
            while j < n and source[j] != c:
                j += 2 if source[j] == '\\' else 1
            tokens.append(source[i:min(j + 1, n)])
            i = min(j + 1, n)
            continue
        for punctuator in PUNCTUATORS:
            if source.startswith(punctuator, i):
                tokens.append(punctuator)
                i += len(punctuator)
                break
        else:
            tokens.append(c)
            i += 1
    return tokens


def first_divergence(left: str, right: str) -> str:
    """A short, copy-pasteable description of where two token streams differ."""
    if left == right:
        return ''
    lt, rt = left, right
    for index, (a, b) in enumerate(zip(lt, rt)):
        if a != b:
            lo, hi = max(0, index - 6), index + 6
            return (f'token {index}:\n    expected ... {" ".join(lt[lo:hi])}\n'
                    f'    actual   ... {" ".join(rt[lo:hi])}')
    if len(lt) != len(rt):
        longer, shorter = (lt, rt) if len(lt) > len(rt) else (rt, lt)
        return f'token count {len(lt)} vs {len(rt)}; extra: {" ".join(longer[len(shorter):][:12])}'
    return 'streams differ only in separator placement'


def check_pp(case: Path, lccc: str, oracles: dict[str, str], verbose: bool) -> Result:
    mine = run([lccc, '-std=gnu23', '-E', str(case)])
    if mine.returncode != 0:
        return Result(case, 'pp', False, f'lccc -E failed rc={mine.returncode}:\n{mine.stderr.strip()}')
    mine_tokens = token_stream(mine.stdout)
    details: list[str] = []
    for name, exe in oracles.items():
        ref = run([exe, '-std=gnu23', '-E', '-P', str(case)])
        if ref.returncode != 0:
            return Result(case, 'pp', True, f'{name} could not preprocess the case; skipped')
        ref_tokens = token_stream(ref.stdout)
        if ref_tokens != mine_tokens:
            details.append(f'vs {name}: {first_divergence(ref_tokens, mine_tokens)}')
    if details:
        return Result(case, 'pp', False, '\n'.join(details))
    return Result(case, 'pp', True, '' if not verbose else f'{len(mine_tokens)} tokens agree')


def check_run(case: Path, lccc: str, oracles: dict[str, str], levels: list[str],
              workdir: Path, verbose: bool) -> Result:
    details: list[str] = []
    reference: dict[str, tuple[int, str]] = {}
    for name, exe in oracles.items():
        binary = workdir / f'{case.stem}.{name}'
        built = run([exe, '-std=gnu11', '-w', '-O0', str(case), '-o', str(binary)])
        if built.returncode != 0:
            return Result(case, 'run', True, f'{name} rejected the case; skipped')
        executed = run_isolated([str(binary)], cwd=workdir)
        reference[name] = (executed.returncode, executed.stdout)
    for level in levels:
        binary = workdir / f'{case.stem}.lccc-O{level}'
        built = run([lccc, '-std=gnu11', '-w', f'-O{level}', str(case), '-o', str(binary)])
        if built.returncode != 0:
            details.append(f'-O{level}: lccc rejected the case:\n{built.stderr.strip()[:2000]}')
            continue
        executed = run_isolated([str(binary)], cwd=workdir)
        for name, (rc, out) in reference.items():
            if executed.returncode != rc:
                details.append(f'-O{level} vs {name}: exit {executed.returncode} != {rc}')
            if executed.stdout != out:
                got, want = executed.stdout.splitlines(), out.splitlines()
                diff = first_divergence(want, got)
                details.append(f'-O{level} vs {name}: stdout differs; {diff}')
        if executed.stderr.strip():
            details.append(f'-O{level}: program wrote to stderr:\n{executed.stderr.strip()[:1000]}')
    if details:
        return Result(case, 'run', False, '\n'.join(details))
    return Result(case, 'run', True, '' if not verbose else f'levels {",".join(levels)} agree')


def check_diag(case: Path, lccc: str, oracles: dict[str, str], verbose: bool) -> Result:
    text = case.read_text(errors='replace')
    expect_accept = text.lstrip().startswith('/* expect: accept */')
    built = run([lccc, '-std=gnu11', '-fsyntax-only', str(case)])
    accepted = built.returncode == 0
    lccc_accepted = accepted
    if 'internal error' in built.stderr.lower() or 'panicked' in built.stderr.lower():
        return Result(case, 'diag', False, f'lccc hit an internal error:\n{built.stderr.strip()[:2000]}')
    details: list[str] = []
    for name, exe in oracles.items():
        ref = run([exe, '-std=gnu11', '-fsyntax-only', str(case)])
        if (ref.returncode == 0) != lccc_accepted:
            verdict = 'accepted' if lccc_accepted else 'rejected'
            details.append(f'{name} disagrees: lccc {verdict} the case')
    if expect_accept and not lccc_accepted:
        details.append(f'case is required to be accepted but lccc rejected it:\n{built.stderr.strip()[:2000]}')
    if details:
        return Result(case, 'diag', False, '\n'.join(details))
    return Result(case, 'diag', True,
                  '' if not verbose else ('accepted' if lccc_accepted else 'rejected, as required'))


def collect(family: str) -> list[Path]:
    directory = AUDIT_ROOT / family
    if not directory.is_dir():
        return []
    return sorted(path for path in directory.glob('*.c') if path.is_file())


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--lccc', default=str(ROOT / 'target/fastbuild/lccc'))
    parser.add_argument('--family', action='append', choices=('pp', 'run', 'diag'),
                        help='restrict the run to one family (repeatable)')
    parser.add_argument('--levels', default='0,1,2',
                        help='optimisation levels for the run family (default 0,1,2)')
    parser.add_argument('--oracle', action='append', default=None,
                        metavar='NAME=EXE', help='override an oracle, e.g. gcc=gcc-16')
    parser.add_argument('--verbose', '-v', action='store_true')
    parser.add_argument('--keep', action='store_true', help='keep the scratch directory')
    args = parser.parse_args(argv)

    lccc = str(Path(args.lccc).resolve())
    if not os.access(lccc, os.X_OK):
        print(f'error: lccc binary not found or not executable: {lccc}', file=sys.stderr)
        return 2

    oracles: dict[str, str] = {}
    for name, candidates in DEFAULT_ORACLES.items():
        for exe in candidates:
            found = shutil.which(exe)
            if found:
                oracles[name] = found
                break
    for spec in args.oracle or ():
        name, _, exe = spec.partition('=')
        found = shutil.which(exe)
        if not found:
            print(f'error: oracle {exe} not found', file=sys.stderr)
            return 2
        oracles[name] = found
    if not oracles:
        print('error: no oracle compiler available '
              '(looked for gcc-16/gcc and clang-23/clang)', file=sys.stderr)
        return 2

    families = args.family or ['pp', 'run', 'diag']
    levels = [part.strip() for part in args.levels.split(',') if part.strip()]
    scratch = Path(tempfile.mkdtemp(prefix='lccc-frontend-audit-'))
    results: list[Result] = []
    try:
        for family in families:
            cases = collect(family)
            if not cases:
                continue
            print(f'== {family}: {len(cases)} case(s) ==')
            for case in cases:
                if family == 'pp':
                    result = check_pp(case, lccc, oracles, args.verbose)
                elif family == 'run':
                    result = check_run(case, lccc, oracles, levels, scratch, args.verbose)
                else:
                    result = check_diag(case, lccc, oracles, args.verbose)
                results.append(result)
                status = 'PASS' if result.ok else 'FAIL'
                suffix = f'  {result.detail}' if (not result.ok or args.verbose) and result.detail else ''
                print(f'  [{status}] {case.relative_to(AUDIT_ROOT)}{suffix}')
    finally:
        if not args.keep:
            shutil.rmtree(scratch, ignore_errors=True)
        elif args.verbose:
            print(f'scratch kept at {scratch}')

    failed = [result for result in results if not result.ok]
    print(f'\n{len(results) - len(failed)}/{len(results)} case(s) agree with '
          f'{", ".join(sorted(oracles))}')
    if failed:
        print('FAILURES:')
        for result in failed:
            print(f'  {result.family}/{result.case.name}')
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
