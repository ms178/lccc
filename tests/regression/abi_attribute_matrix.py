#!/usr/bin/env python3
"""The ABI-selecting attribute contract, measured in both directions.

GP-02 (risk R1 of `docs/GCC16-PARITY-ASSESSMENT-2026-10-27.md`) is the one
defect in the campaign that produces *silently wrong executables*: a
convention attribute that one compiler honours and the other ignores still
links, still starts, and returns garbage. This module is the machine-checked
form of the contract that replaces that failure mode, and it is imported by
`check_attribute_diagnostics.py` so a single CI step owns both the wording and
the convention.

Two independent directions are asserted, because either one alone can lie:

1. **The table versus gcc.** `src/frontend/parser/attribute_registry.rs`
   declares one verdict per (name, target). This module re-derives that verdict
   from a live gcc by *observing* what gcc does — not by trusting the comment:
   compile three twin bodies (integer, floating, aggregate-return) and a record
   layout with and without the attribute, then decide between

       rejects      gcc diagnoses the attribute as unusable here
       only32       gcc says `'x' attribute only available for 32-bit`
       ignored      gcc warns `'x' attribute ignored` and emits identical code
       honours      gcc is silent and the emitted code changes
       inert        gcc is silent and the emitted code does not change

   A gcc that changes its mind about a name therefore fails the gate instead of
   silently redefining what "parity" means.

2. **The table versus lccc.** For every cell, lccc must be silent on `Default`,
   reproduce gcc's sentence on `GccIgnored`/`OnlyAvailable32Bit`, and *refuse*
   `Unsupported` — with `CCC_ALLOW_UNSUPPORTED_ABI` as the documented escape
   hatch, which must downgrade the refusal to a warning. The two conventions
   lccc genuinely implements (`regparm`, `fastcall` on i686) are proved by a
   cross-compiler runtime sentinel: gcc builds one side and lccc the other, in
   both directions, and the program must exit with the value only an agreeing
   ABI can produce.
"""
from __future__ import annotations

import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
REGISTRY = ROOT / 'src/frontend/parser/attribute_registry.rs'
ENV = {**os.environ, 'LC_ALL': 'C'}

# Probes. Each one is two twins that differ only in the attribute, so any
# difference in the emitted code is attributable to the attribute alone.
INT_PROBE = """\
int {attr}f(int a, int b, int c, int d) {{ return a * 1000 + b * 100 + c * 10 + d; }}
int g(int a, int b, int c, int d) {{ return a * 1000 + b * 100 + c * 10 + d; }}
"""
FP_PROBE = """\
double {attr}h(double a, double b) {{ return a + b; }}
double k(double a, double b) {{ return a + b; }}
"""
# Aggregate return is where several i386 conventions differ (hidden pointer
# popped by the callee or not) even though the argument registers are the same.
AGG_PROBE = """\
typedef struct {{ int a, b, c, d; }} Big;
Big {attr}ha(Big s, int x) {{ s.a += x; return s; }}
Big ka(Big s, int x) {{ s.a += x; return s; }}
"""
# Bit-field layout: the only probe that can see `ms_struct`/`gcc_struct`, which
# never touch code and are therefore invisible to the three probes above.
REC_PROBE = """\
struct {attr}R {{ char a; int b : 5; int c : 3; }};
struct Q {{ char a; int b : 5; int c : 3; }};
int rs(void) {{ return (int)sizeof(struct R); }}
int qs(void) {{ return (int)sizeof(struct Q); }}
"""
BODIES = {'int': INT_PROBE, 'fp': FP_PROBE, 'agg': AGG_PROBE, 'rec': REC_PROBE}
RECORD_ONLY = {'ms_struct', 'gcc_struct'}

# Attributes that take a mandatory integer argument, spelled the way a user
# writes them; a bare name is a different diagnostic and a different test.
CALL_FORMS = {
    'regparm': 'regparm(3)',
    'callee_pop_aggregate_return': 'callee_pop_aggregate_return(0)',
}


def attribute_spelling(name: str) -> str:
    return f'__attribute__(({CALL_FORMS.get(name, name)})) '


def _strip_local_symbols(text: str) -> str:
    """Normalise the parts of an asm listing that legitimately differ."""
    text = re.sub(r'\.L(FB|FE|BB|BE|VL)\d+', r'.L\1x', text)
    text = re.sub(r'\.LC\d+', '.LCx', text)
    return '\n'.join(line.strip() for line in text.splitlines() if line.strip())


def function_bodies(asm: str) -> dict[str, str]:
    """Split a gcc/lccc `-S` listing into normalised per-function bodies."""
    bodies: dict[str, str] = {}
    current = None
    for line in asm.splitlines():
        match = re.match(r'^([A-Za-z_]\w*):$', line)
        if match and not line.startswith('.L'):
            current = match.group(1)
            bodies[current] = ''
            continue
        # `.size f, .-f` ends the function and is the only place the body
        # contains its own name, so stopping here makes twin bodies comparable
        # without any name rewriting.
        if current is not None and re.match(r'^\s*\.size\s+[\w.]+\s*,', line):
            current = None
            continue
        if current is not None:
            bodies[current] += line + '\n'
    return {name: _strip_local_symbols(body) for name, body in bodies.items()}


_REG_ALIASES = {
    'rax': 'rax', 'eax': 'rax', 'ax': 'rax', 'al': 'rax', 'ah': 'rax',
    'rbx': 'rbx', 'ebx': 'rbx', 'bx': 'rbx', 'bl': 'rbx', 'bh': 'rbx',
    'rcx': 'rcx', 'ecx': 'rcx', 'cx': 'rcx', 'cl': 'rcx', 'ch': 'rcx',
    'rdx': 'rdx', 'edx': 'rdx', 'dx': 'rdx', 'dl': 'rdx', 'dh': 'rdx',
    'rsi': 'rsi', 'esi': 'rsi', 'si': 'rsi', 'sil': 'rsi',
    'rdi': 'rdi', 'edi': 'rdi', 'di': 'rdi', 'dil': 'rdi',
    'rbp': 'rbp', 'ebp': 'rbp', 'bpl': 'rbp',
    'rsp': 'rsp', 'esp': 'rsp', 'spl': 'rsp',
}


def _canon_reg(name: str) -> str:
    """Fold the 8/16/32/64-bit spellings of one register into one name.

    Without this, two bodies that differ only in whether gcc picked `%eax` or
    `%rax` for the same value would look like an ABI change.
    """
    if name in _REG_ALIASES:
        return _REG_ALIASES[name]
    match = re.fullmatch(r'r(\d+)([dwb])?', name)
    if match:
        return f'r{match.group(1)}'
    return name


def abi_signature(probe_id: str, body: str) -> tuple:
    """The part of a function body that an ABI decision can change.

    Deliberately *not* the instruction stream: gcc emits equivalent but
    different code for two identical twin functions in the same translation
    unit (`combine` sees them independently), so a byte comparison would report
    a false "the attribute changed the ABI" for every name. What a convention
    actually decides is which registers carry the arguments and who pops the
    outgoing argument area, plus — for the layout probe — the record's size.
    """
    body = re.sub(r'\$[A-Za-z_][\w.]*', '$SYM', body)
    if probe_id == 'rec':
        return tuple(sorted(set(re.findall(r'\$(\d+)', body))))
    regs = tuple(sorted({_canon_reg(r) for r in re.findall(r'%([a-z][a-z0-9]*)', body)}))
    ret = tuple(re.findall(r'\b(ret|jmp)\b\s*(\$?\d*)', body))
    return (regs, ret)


def compile_to(cc: str, source: str, flags: list[str]) -> tuple[int, str, str]:
    with tempfile.NamedTemporaryFile('w', suffix='.c', delete=False) as handle:
        handle.write(source)
        path = handle.name
    try:
        result = subprocess.run([cc, *flags, '-S', '-o', '-', '-x', 'c', path],
                                capture_output=True, text=True, env=ENV)
        return result.returncode, result.stdout, result.stderr
    finally:
        os.unlink(path)


def gcc_observation(gcc: str, name: str, target: str) -> tuple[str, str]:
    """What gcc actually does with `name` on `target`: (observation, evidence)."""
    attr = attribute_spelling(name)
    base = ['-O2'] + (['-m32'] if target == 'i686' else [])
    diagnostics: list[str] = []
    errors: list[str] = []
    changed = False
    probe_ids = ['rec'] if name in RECORD_ONLY else ['int', 'fp', 'agg']
    for probe_id in probe_ids:
        source = BODIES[probe_id].format(attr=attr)
        rc, out, err = compile_to(gcc, source, base)
        for line in err.splitlines():
            if 'attribute' in line and ('warning:' in line or 'error:' in line):
                diagnostics.append(line.split(': ', 2)[-1].strip())
        if rc != 0:
            errors.append(err.strip().splitlines()[0] if err.strip() else 'compile failure')
            continue
        bodies = function_bodies(out)
        first, second = _pair(probe_id)
        if first not in bodies or second not in bodies:
            errors.append(f'{probe_id} probe bodies missing from the listing')
            continue
        if abi_signature(probe_id, bodies[first]) != abi_signature(probe_id, bodies[second]):
            changed = True
    evidence = ' | '.join(errors or diagnostics) or 'silent'
    if errors:
        return 'rejects', evidence
    joined = ' '.join(diagnostics)
    if 'only available for 32-bit' in joined:
        return 'only32', evidence
    if 'ignored' in joined:
        return ('ignored' if not changed else 'ignored-changed'), evidence
    return ('honours' if changed else 'inert'), evidence


def _pair(probe_id: str) -> tuple[str, str]:
    return {
        'int': ('f', 'g'),
        'fp': ('h', 'k'),
        'agg': ('ha', 'ka'),
        'rec': ('rs', 'qs'),
    }[probe_id]


# What lccc must do with each kind of gcc observation.
EXPECTED_VERDICT = {
    'rejects': 'Unsupported',
    'honours': 'Unsupported',
    'only32': 'OnlyAvailable32Bit',
    'ignored': 'GccIgnored',
    'ignored-changed': 'Unsupported',
    'inert': 'Default',
}

# Measured exceptions, each justified in the registry's ABI section:
#  * lccc *implements* these two conventions on i686, so gcc observing
#    `honours` is the reason to keep them, not to refuse them.
LCCC_IMPLEMENTS = {('regparm', 'i686'), ('fastcall', 'i686')}


def registry_verdicts() -> dict[tuple[str, str], str]:
    """Parse `ABI_VERDICTS` out of the registry: (name, target) -> verdict."""
    source = REGISTRY.read_text()
    match = re.search(
        r'const ABI_VERDICTS: &\[\(&str, AbiEffect, AbiEffect\)\] = &\[(.*?)\n\];',
        source, re.S)
    if not match:
        raise ValueError('missing ABI_VERDICTS table in attribute_registry.rs')
    rows = re.findall(
        r'\(\s*"([a-z0-9_]+)",\s*AbiEffect::(\w+),\s*AbiEffect::(\w+)\s*,?\s*\)',
        match.group(1))
    if not rows:
        raise ValueError('ABI_VERDICTS is empty')
    names = [row[0] for row in rows]
    if len(names) != len(set(names)):
        raise ValueError('duplicate name in ABI_VERDICTS')
    table: dict[tuple[str, str], str] = {}
    for name, x86_64, i686 in rows:
        table[(name, 'x86-64')] = x86_64
        table[(name, 'i686')] = i686
    return table


def check_table_against_gcc(gcc: str, verdicts: dict[tuple[str, str], str],
                            has_i686: bool) -> tuple[list[tuple], list[str]]:
    """Re-derive every cell from gcc and compare with the table."""
    failures: list[tuple] = []
    notes: list[str] = []
    for (name, target), verdict in sorted(verdicts.items()):
        if target == 'i686' and not has_i686:
            notes.append(f'skipped {name}/i686 (gcc has no 32-bit multilib)')
            continue
        observation, evidence = gcc_observation(gcc, name, target)
        expected = EXPECTED_VERDICT.get(observation)
        if expected is None:
            failures.append((f'abi:{name}/{target}', 'unknown gcc observation',
                             'rejects|only32|ignored|honours|inert',
                             (observation, evidence)))
            continue
        if (name, target) in LCCC_IMPLEMENTS:
            if observation != 'honours':
                failures.append((f'abi:{name}/{target}',
                                 'lccc implements it but gcc no longer honours it',
                                 'honours', (observation, evidence)))
            elif verdict == 'Unsupported':
                failures.append((f'abi:{name}/{target}',
                                 'table refuses a convention lccc implements',
                                 'not Unsupported', verdict))
            continue
        if expected != verdict:
            failures.append((f'abi:{name}/{target}', 'verdict disagrees with gcc',
                             (expected, observation), (verdict, evidence)))
    return failures, notes


def _driver() -> Path:
    """The compiler under test; `-m32` selects the i686 back-end.

    The same binary serves both targets (`check_asm_goto_outputs.sh` drives
    `-m32` through it), so the gate needs no second artifact to be meaningful.
    """
    return ROOT / 'target/fastbuild/lccc'


def _arch_flags(target: str) -> list[str]:
    return ['-m32'] if target == 'i686' else []


def _syntax_only(driver: Path, source: str, flags: list[str],
                 env: dict[str, str]) -> tuple[subprocess.CompletedProcess, str]:
    """Compile one probe with lccc; returns the run and its first diagnostic."""
    with tempfile.NamedTemporaryFile('w', suffix='.c', delete=False) as handle:
        handle.write(source)
        path = handle.name
    try:
        proc = subprocess.run([str(driver), *flags, '-fsyntax-only', path],
                              capture_output=True, text=True, env=env)
    finally:
        os.unlink(path)
    return proc, first_diag(proc.stderr)


def first_diag(stderr: str) -> str:
    for line in stderr.splitlines():
        if ': error: ' in line or ': warning: ' in line:
            return line.split(': ', 2)[-1] if line.count(': ') >= 2 else line
    return ''


def check_lccc(verdicts: dict[tuple[str, str], str]) -> tuple[list[tuple], list[str]]:
    """Prove lccc's side of every cell, including the runtime sentinel."""
    failures: list[tuple] = []
    notes: list[str] = []
    checked = 0
    for (name, target), verdict in sorted(verdicts.items()):
        driver = _driver()
        if not driver.exists():
            notes.append(f'skipped lccc {name}/{target} ({driver.name} not built)')
            continue
        if (name, target) in LCCC_IMPLEMENTS:
            continue  # proved end-to-end by the sentinel below
        source = BODIES['rec' if name in RECORD_ONLY else 'int'].format(
            attr=attribute_spelling(name))
        flags = ['-O2', *_arch_flags(target)]
        proc, diag = _syntax_only(driver, source, flags, ENV)
        checked += 1
        if verdict == 'Default':
            if proc.returncode != 0 or 'attribute' in proc.stderr:
                failures.append((f'abi:{name}/{target}', 'Default must be silent',
                                 '', (proc.returncode, diag)))
        elif verdict == 'GccIgnored':
            want = (f"'{name}' attribute directive ignored"
                    if name in ('preserve_none', 'no_callee_saved_registers')
                    else f"'{name}' attribute ignored")
            if proc.returncode != 0 or want not in proc.stderr:
                failures.append((f'abi:{name}/{target}', 'gcc wording not reproduced',
                                 want, (proc.returncode, diag)))
        elif verdict == 'OnlyAvailable32Bit':
            want = f"'{name}' attribute only available for 32-bit"
            if proc.returncode != 0 or want not in proc.stderr:
                failures.append((f'abi:{name}/{target}', 'gcc wording not reproduced',
                                 want, (proc.returncode, diag)))
        else:  # Unsupported
            if proc.returncode == 0:
                failures.append((f'abi:{name}/{target}',
                                 'a foreign convention was accepted silently',
                                 'nonzero exit', (proc.returncode, diag)))
            elif name not in proc.stderr:
                failures.append((f'abi:{name}/{target}',
                                 'refusal does not name the attribute',
                                 name, proc.stderr[:200]))
            relaxed = {**ENV, 'CCC_ALLOW_UNSUPPORTED_ABI': '1'}
            again, _ = _syntax_only(driver, source, flags, relaxed)
            if again.returncode != 0 or 'CCC_ALLOW_UNSUPPORTED_ABI' not in again.stderr:
                failures.append((f'abi:{name}/{target}',
                                 'escape hatch does not downgrade the refusal',
                                 'warning + success',
                                 (again.returncode, first_diag(again.stderr))))
    sentinel_failures, sentinel_notes = check_sentinels()
    return failures + sentinel_failures, notes + sentinel_notes


# The cross-compiler sentinel: one side built by gcc, the other by lccc, both
# directions, no libc (raw `_start` + `int $0x80`), so the only thing that can
# make the program exit 123 is an identical argument-passing convention.
CALLEE = 'int __attribute__(({attr}))callee(int a, int b, int c) ' \
         '{{ return a * 100 + b * 10 + c; }}\n'
CALLER = """\
int __attribute__(({attr}))callee(int a, int b, int c);
void _start(void) {{
    int r = callee(1, 2, 3);
    __asm__ __volatile__("mov %0, %%ebx\\n\\tmov $1, %%eax\\n\\tint $0x80"
                         :: "r"(r) : "eax", "ebx", "memory");
    __builtin_unreachable();
}}
"""
SENTINELS = ('fastcall', 'regparm(3)')


def check_sentinels() -> tuple[list[tuple], list[str]]:
    failures: list[tuple] = []
    notes: list[str] = []
    lccc = _driver()
    gcc = shutil.which('gcc')
    if not lccc.exists() or not gcc:
        return failures, ['skipped i686 ABI runtime sentinels '
                          f'({lccc.name} or gcc missing)']
    tools = {'gcc': gcc, 'lccc': str(lccc)}
    for spelling in SENTINELS:
        attr = f'{spelling} '
        with tempfile.TemporaryDirectory(prefix='lccc-abi-') as tmp:
            tmpdir = Path(tmp)
            (tmpdir / 'callee.c').write_text(CALLEE.format(attr=attr))
            (tmpdir / 'caller.c').write_text(CALLER.format(attr=attr))
            for label, callee_side, caller_side in (
                    ('gcc-callee/lccc-caller', 'gcc', 'lccc'),
                    ('lccc-callee/gcc-caller', 'lccc', 'gcc')):
                objects = []
                built = True
                for stem, side in (('callee', callee_side), ('caller', caller_side)):
                    cc = tools[side]
                    obj = tmpdir / f'{stem}.o'
                    # `-ffreestanding -fno-pie` keeps the object bare so the
                    # freestanding `-static -no-pie` link below cannot see a
                    # GOT reference either compiler is otherwise free to
                    # create, and neither one pulls in a libc call.
                    cmd = [cc, '-m32', '-O2', '-ffreestanding', '-fno-pie', '-c',
                           str(tmpdir / f'{stem}.c'), '-o', str(obj)]
                    proc = subprocess.run(cmd, capture_output=True, text=True, env=ENV)
                    if proc.returncode != 0:
                        failures.append((f'sentinel:{spelling}',
                                         f'{label} {stem} did not build',
                                         '', proc.stderr.strip()[:300]))
                        built = False
                        break
                    objects.append(str(obj))
                if not built:
                    continue
                exe = tmpdir / 'prog'
                link = subprocess.run(
                    [gcc, '-m32', '-nostdlib', '-static', '-no-pie', '-Wl,-e,_start',
                     *objects, '-o', str(exe)],
                    capture_output=True, text=True, env=ENV)
                if link.returncode != 0:
                    notes.append(f'skipped sentinel {spelling} ({label}): 32-bit link '
                                 f'unavailable ({link.stderr.strip().splitlines()[:1]})')
                    continue
                code = subprocess.run([str(exe)], capture_output=True).returncode
                if code != 123:
                    failures.append((f'sentinel:{spelling}', f'{label} disagrees on the ABI',
                                     123, code))
    return failures, notes


def main() -> int:
    gcc = shutil.which(os.environ.get('GCC') or 'gcc')
    if not gcc:
        print('SKIP: no gcc found')
        return 1
    has_i686 = subprocess.run([gcc, '-m32', '-E', '-x', 'c', '-', '-o', os.devnull],
                              input='int x;\n', capture_output=True,
                              text=True).returncode == 0
    verdicts = registry_verdicts()
    failures, notes = check_table_against_gcc(gcc, verdicts, has_i686)
    lccc_failures, lccc_notes = check_lccc(verdicts)
    failures += lccc_failures
    for note in notes + lccc_notes:
        print(f'  note: {note}')
    if failures:
        print(f'FAIL: {len(failures)} ABI attribute contract mismatches')
        for failure in failures[:20]:
            print('  ', failure)
        return 1
    print(f'PASS: ABI attribute contract holds ({len(verdicts)} cells re-measured '
          f'against {gcc})')
    return 0


if __name__ == '__main__':
    sys.exit(main())
