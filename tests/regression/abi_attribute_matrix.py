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
   compile integer, floating, aggregate-return and register-preservation
   probes with and without the attribute, including a caller-side preservation
   probe, plus a separate record-layout probe; then decide between

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

# Every external process this gate starts is bounded. A hung or thrashing
# compiler would otherwise burn the job's multi-hour budget and the gate would
# report nothing at all -- the worst possible outcome for a check that exists to
# prove a contract. A timeout is turned into a *failure with a reason*, never a
# traceback: a gate that crashes names no cell.
COMPILE_TIMEOUT = 120.0
LINK_TIMEOUT = 60.0
RUN_TIMEOUT = 30.0

# The gcc release `ABI_VERDICTS` is pinned to. Cells are only *failed* on that
# release or newer: gcc changes its mind about convention attributes between
# releases (measured: `regparm` is silent on x86-64 in 12.2 and
# `'regparm' attribute ignored` in 16.2), so following whatever gcc happens to
# be on `$PATH` would make the contract flap. A newer gcc is still checked
# strictly -- that is how oracle drift is caught -- while an older one turns the
# disagreement into a note that names both verdicts.
ORACLE_GCC = (16, 2)

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
# Register preservation. The three probes above make no call, so nothing has
# to survive one and no compiler ever saves a callee-saved register: they are
# structurally blind to `no_callee_saved_registers`/`preserve_none`, which are
# pure prologue attributes (measured: with them GCC 16.2 keeps five arguments
# live across the call in stack slots instead of %rbx/%r12-%r15 and emits no
# push at all). Five live-across-call values force the reference compiler to
# use its callee-saved set, so the probe sees the convention the attribute
# selects.
REG_PROBE = """\
extern int sink(int);
int {attr}ps(int a, int b, int c, int d, int e, int f) {{
    int s = sink(a);
    return s + b * 3 + c * 5 + d * 7 + e * 11 + f * 13;
}}
int qs2(int a, int b, int c, int d, int e, int f) {{
    int s = sink(a);
    return s + b * 3 + c * 5 + d * 7 + e * 11 + f * 13;
}}
"""
# Caller-side preservation probe. The attributed callee is only a declaration,
# so the caller compiler must honor its clobber set at the call site. Keeping
# five computed values live across the call makes the ABI difference visible
# as callee-saved pushes and stack spills, rather than just an instruction-set
# difference in the attributed function.
CALLER_REG_PROBE = """\
extern int {attr}sink(int);
int caller(int a, int b, int c, int d, int e, int f) {{
    int x1=b*3, x2=c*5, x3=d*7, x4=e*11, x5=f*13;
    int y=sink(a);
    return y+x1+x2+x3+x4+x5;
}}
"""
PRESERVATION_ATTRS = {'no_callee_saved_registers', 'preserve_none'}
BODIES = {'int': INT_PROBE, 'fp': FP_PROBE, 'agg': AGG_PROBE, 'rec': REC_PROBE,
          'reg': REG_PROBE}
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

    Deliberately *not* the full instruction stream or the set of registers
    mentioned anywhere in it. The selected fields are limited to ABI-observable
    properties: input registers, callee-save actions, dynamic stack realignment,
    and return cleanup. The same function symbol is compiled separately with
    and without the attribute, preventing register allocation differences
    between differently named same-TU twins from masquerading as ABI changes.

    What a convention actually decides is:
      * `inputs`   -- which registers the callee reads before writing them,
                      i.e. where the arguments arrived;
      * `preserved`-- which callee-saved registers it pushes/pops, i.e. the
                      register-preservation contract (`preserve_none`,
                      `no_callee_saved_registers`);
      * `realign`  -- whether the prologue aligns `%rsp` itself
                      (`force_align_arg_pointer`);
      * `ret`      -- the epilogue form and any `ret $N` callee-pop count;
      * for the record probe, the record sizes only.
    """
    body = re.sub(r'\$[A-Za-z_][\w.]*', '$SYM', body)
    if probe_id == 'rec':
        return tuple(sorted(set(re.findall(r'\$(\d+)', body))))
    return (
        _input_registers(body),
        tuple(sorted(_saved_registers(body))),
        bool(re.search(r'\band[ql]\s+\$-?\d+,\s*%(?:rsp|esp)\b', body)),
        tuple(re.findall(r'\b(retq?|jmpq?)\b\s*(\$?\d*)', body)),
    )


#: Callee-saved registers on the two SysV targets. `%rbp` is deliberately
#: absent when it is the frame pointer: the reference compiler's choice to
#: omit a frame pointer is not an ABI decision this gate measures.
_CALLEE_SAVED = {
    'x86-64': {'rbx', 'r12', 'r13', 'r14', 'r15', 'rbp'},
    'i686': {'ebx', 'esi', 'edi', 'ebp'},
}

#: Mnemonics that only read their operands, so the last one is not a
#: destination.  Everything else in AT&T syntax writes its last operand.
_READ_ONLY = frozenset({
    'cmp', 'cmpl', 'cmpq', 'cmpw', 'cmpb', 'test', 'testl', 'testq', 'testw',
    'testb', 'push', 'pushq', 'pushl', 'pushw', 'call', 'callq', 'jmp', 'jmpq',
    'ret', 'retq', 'nop', 'nopw', 'nopl', 'vzeroupper', 'vzeroall', 'ud2',
})
#: Mnemonics whose equal-operand form is a pure write (`xor %eax,%eax`).
_SELF_WRITE = ('xor', 'pxor', 'vpxor', 'sub', 'psub')
#: `%rsp`/`%rbp` are the frame, not an argument channel, so their early reads
#: are not evidence of a convention: the pointer to the frame is always read
#: first, in a different order depending on whether a frame pointer is used.
_FRAME_REGS = frozenset({'rsp', 'rbp'})


def _saved_registers(body: str) -> list[tuple[str, str]]:
    """Every `push`/`pop` of a callee-saved register, in body order.

    Returns `(canonical_register, 'push' | 'pop')` pairs. Both SysV targets
    spell the mnemonic `push`/`pushq`/`pop`/`popq`, so the match is on the
    stem and the canonical register name decides membership.
    """
    saved = _CALLEE_SAVED['x86-64'] | _CALLEE_SAVED['i686']
    out = []
    for kind, reg in re.findall(r'\b(push|pop)[ql]?\s+%([a-z][a-z0-9]*)', body):
        name = _canon_reg(reg)
        if name in saved:
            out.append((name, kind))
    return out


def _input_registers(body: str) -> tuple:
    """Registers the body reads before it writes them, in first-read order.

    That is the machine-level definition of "where did the arguments arrive",
    and unlike the set of mentioned registers it is stable under the code
    generator's freedom to rematerialise values.
    """
    written: set[str] = set()
    inputs: list[str] = []
    for line in body.splitlines():
        text = line.split('#', 1)[0].strip()
        if not text or text.startswith('.') or text.endswith(':'):
            continue
        mnemonic, _, rest = text.partition('\t')
        if not rest:
            mnemonic, _, rest = text.partition(' ')
        mnemonic = mnemonic.strip()
        operands = [op.strip() for op in rest.split(',') if op.strip()]
        if not operands:
            continue
        if mnemonic.startswith(('push', 'pop')):
            # Save/restore traffic is an ABI consequence, not evidence that the
            # saved register carried an incoming argument.
            continue
        if mnemonic in _READ_ONLY:
            reads, dest = operands, None
        else:
            reads, dest = operands[:-1], operands[-1]
            if mnemonic.startswith(_SELF_WRITE) and len(operands) == 2 \
                    and operands[0] == operands[1]:
                reads = []
        for op in reads:
            for reg in re.findall(r'%([a-z][a-z0-9]*)', op):
                name = _canon_reg(reg)
                if name in _FRAME_REGS or name in written:
                    continue
                written.add(name)
                if name not in inputs:
                    inputs.append(name)
        if dest is None:
            continue
        if '(' in dest:
            # A memory destination reads its base/index registers.
            for reg in re.findall(r'%([a-z][a-z0-9]*)', dest):
                name = _canon_reg(reg)
                if name in _FRAME_REGS or name in written:
                    continue
                written.add(name)
                if name not in inputs:
                    inputs.append(name)
        else:
            for reg in re.findall(r'%([a-z][a-z0-9]*)', dest):
                written.add(_canon_reg(reg))
    return tuple(inputs)


def run_bounded(cmd: list[str], *, timeout: float, **kwargs) -> subprocess.CompletedProcess:
    """`subprocess.run` that reports a timeout as a 124 result, not a crash."""
    kwargs.setdefault('capture_output', True)
    kwargs.setdefault('text', True)
    kwargs.setdefault('env', ENV)
    try:
        return subprocess.run(cmd, timeout=timeout, **kwargs)
    except subprocess.TimeoutExpired:
        return subprocess.CompletedProcess(
            cmd, 124, '', f'{Path(cmd[0]).name} timed out after {timeout:g}s')
    except OSError as error:
        # A missing or non-executable compiler is a *result* here: the cell
        # reports it instead of the whole gate dying on an unrelated traceback.
        return subprocess.CompletedProcess(
            cmd, 127, '', f'{Path(cmd[0]).name}: {error.strerror or error}')


def compile_to(cc: str, source: str, flags: list[str]) -> tuple[int, str, str]:
    """Compile one probe to an assembly listing on stdout."""
    with tempfile.NamedTemporaryFile('w', suffix='.c', delete=False) as handle:
        handle.write(source)
        path = handle.name
    try:
        result = run_bounded([cc, *flags, '-S', '-o', '-', '-x', 'c', path],
                             timeout=COMPILE_TIMEOUT)
        return result.returncode, result.stdout, result.stderr
    finally:
        os.unlink(path)


def gcc_version(gcc: str) -> tuple[int, int]:
    """`(major, minor)` of `gcc`, or `(0, 0)` when it cannot be asked.

    `(0, 0)` sorts below every pin, so an unreadable gcc degrades to the lenient
    path instead of silently claiming the pinned release.
    """
    proc = run_bounded([gcc, '-dumpfullversion'], timeout=COMPILE_TIMEOUT)
    match = re.match(r'(\d+)\.(\d+)', (proc.stdout or '').strip())
    return (int(match.group(1)), int(match.group(2))) if match else (0, 0)


def gcc_observation(gcc: str, name: str, target: str) -> tuple[str, str]:
    """Measure attributed and plain ABI twins in isolated compilations."""
    attr = attribute_spelling(name)
    base = ['-O2'] + (['-m32'] if target == 'i686' else [])
    diagnostics: list[str] = []
    errors: list[str] = []
    diffs: list[str] = []
    changed = False
    probe_ids = ['rec'] if name in RECORD_ONLY else ['int', 'fp', 'agg', 'reg']

    def compare(probe_id: str, source: str, symbol: str) -> None:
        nonlocal changed
        reference_source = source.replace(attr, '', 1)
        rc, out, err = compile_to(gcc, source, base)
        ref_rc, ref_out, ref_err = compile_to(gcc, reference_source, base)
        for line in err.splitlines():
            if 'attribute' in line and ('warning:' in line or 'error:' in line):
                diagnostics.append(line.split(': ', 2)[-1].strip())
        if rc != 0:
            errors.append(err.strip().splitlines()[0] if err.strip() else
                          f'{probe_id} attributed compile failed ({rc})')
            return
        if ref_rc != 0:
            errors.append(ref_err.strip().splitlines()[0] if ref_err.strip() else
                          f'{probe_id} reference compile failed ({ref_rc})')
            return
        bodies, references = function_bodies(out), function_bodies(ref_out)
        if symbol not in bodies or symbol not in references:
            errors.append(f'{probe_id} same-symbol probe body missing from listing')
            return
        measured = abi_signature(probe_id, bodies[symbol])
        reference = abi_signature(probe_id, references[symbol])
        if measured != reference:
            changed = True
            diffs.append(f'{probe_id}: {attr_pair(measured, reference)}')

    for probe_id in probe_ids:
        source = BODIES[probe_id].format(attr=attr)
        first, second = _pair(probe_id)
        # Remove the un-attributed twin: compare one identically named body
        # compiled separately with and without the attribute.
        source = re.sub(
            rf'(?ms)^(?:void|int|double|Big)\s+{re.escape(second)}\s*'
            rf'\([^)]*\)\s*\{{.*?^\}}\s*', '', source)
        compare(probe_id, source, first)

    if name in PRESERVATION_ATTRS:
        compare('reg', CALLER_REG_PROBE.format(attr=attr), 'caller')

    unique = list(dict.fromkeys(errors or diagnostics))
    evidence = ' | '.join(unique) or 'silent'
    if changed and diffs:
        evidence += ' | ' + '; '.join(diffs[:2])
    if errors:
        return 'rejects', evidence
    joined = ' '.join(diagnostics)
    if 'only available for 32-bit' in joined:
        return 'only32', evidence
    if 'ignored' in joined:
        return ('ignored' if not changed else 'ignored-changed'), evidence
    return ('honours' if changed else 'inert'), evidence


def _show_signature(part) -> str:
    """Render one ABI signature component, however deeply nested."""
    if isinstance(part, (tuple, list)):
        return '[' + ' '.join(_show_signature(item) for item in part) + ']'
    return str(part)


def attr_pair(measured: tuple, reference: tuple) -> str:
    """A short `attr=<...> plain=<...>` description of two ABI signatures.

    `abi_signature` returns nested input-register, callee-save, realignment,
    and return-form data for code probes, plus a flat tuple of sizes for the
    record-layout probe; recursive rendering keeps diagnostics stable.
    An annotation that says only 'honours' names a cell but no cause.
    """
    return f'attr={_show_signature(measured)} plain={_show_signature(reference)}'


def _pair(probe_id: str) -> tuple[str, str]:
    return {
        'int': ('f', 'g'),
        'fp': ('h', 'k'),
        'agg': ('ha', 'ka'),
        'reg': ('ps', 'qs2'),
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
    known = {'Default', 'GccIgnored', 'OnlyAvailable32Bit', 'Unsupported',
             'Implemented'}
    for row in rows:
        for variant in row[1:]:
            if variant not in known:
                # A typo'd variant would otherwise travel through the gate as an
                # opaque 'verdict' string and fail on an unrelated cell.
                raise ValueError(f'unknown AbiEffect::{variant} for {row[0]}')
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
    version = gcc_version(gcc)
    strict = version >= ORACLE_GCC
    if version == ORACLE_GCC:
        kind = 'normative'
    elif strict:
        kind = 'newer than the pin'
    else:
        kind = 'predates the pin'
    notes.append(f'oracle: {gcc}: GCC {".".join(map(str, version))} ({kind}; '
                 f'table pinned to {ORACLE_GCC[0]}.{ORACLE_GCC[1]})')
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
            if strict:
                failures.append((f'abi:{name}/{target}', 'verdict disagrees with gcc',
                                 (expected, observation), (verdict, evidence)))
            else:
                notes.append(f'pre-oracle gcc observes {observation} for '
                             f'{name}/{target} (table: {verdict}): {evidence}')
    return failures, notes


def _driver() -> Path:
    """The compiler under test; `-m32` selects the i686 back-end.

    The same binary serves both targets (`check_asm_goto_outputs.sh` drives
    `-m32` through it), so the gate needs no second artifact to be meaningful.
    A caller that has already resolved the driver (the CI step, or `--lccc`)
    passes it in instead of letting this module guess a second time.
    """
    return Path(os.environ.get('LCCC') or ROOT / 'target/fastbuild/lccc')


def _arch_flags(target: str) -> list[str]:
    return ['-m32'] if target == 'i686' else []


def _syntax_only(driver: Path, source: str, flags: list[str],
                 env: dict[str, str]) -> tuple[subprocess.CompletedProcess, str]:
    """Compile one probe with lccc; returns the run and its first diagnostic."""
    with tempfile.NamedTemporaryFile('w', suffix='.c', delete=False) as handle:
        handle.write(source)
        path = handle.name
    try:
        proc = run_bounded([str(driver), *flags, '-fsyntax-only', path],
                           timeout=COMPILE_TIMEOUT, env=env)
    finally:
        os.unlink(path)
    return proc, first_diag(proc.stderr)


def first_diag(stderr: str) -> str:
    for line in stderr.splitlines():
        if ': error: ' in line or ': warning: ' in line:
            return line.split(': ', 2)[-1] if line.count(': ') >= 2 else line
    return ''


def check_lccc(verdicts: dict[tuple[str, str], str],
               driver: Path | None = None) -> tuple[list[tuple], list[str]]:
    """Prove lccc's side of every cell, including the runtime sentinel."""
    failures: list[tuple] = []
    notes: list[str] = []
    checked = 0
    driver = Path(driver) if driver is not None else _driver()
    built = driver.exists()
    if not built:
        notes.append(f'skipped lccc cells ({driver.name} not built)')
    for (name, target), verdict in sorted(verdicts.items()):
        if not built:
            break
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
            want = f"'{name}' attribute ignored"
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
    if not built:
        return failures, notes
    # The convention funnel must not swallow malformed argument lists, even
    # for an ignored/default convention. Exercise both GNU attribute spellings.
    for target in ('x86-64', 'i686'):
        for spelling in ('sysv_abi(1)', 'cdecl(1)', 'ms_abi(1)',
                         'regparm()', 'regparm(1,2)', 'regparm'):
            for attr in (f'__attribute__(({spelling}))', f'[[gnu::{spelling}]]'):
                proc, _ = _syntax_only(driver, f'int {attr} f(void);',
                                       _arch_flags(target), ENV)
                if proc.returncode == 0:
                    failures.append((f'abi:arity/{target}', 'malformed attribute accepted',
                                     'nonzero exit', attr))
    sentinel_failures, sentinel_notes = check_sentinels(driver)
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


def check_sentinels(driver: Path | None = None,
                    gcc: str | None = None) -> tuple[list[tuple], list[str]]:
    """Cross-compiler runtime sentinel for the two i686 conventions.

    `gcc` defaults to `$GCC` and only then to `gcc` on `$PATH`: the sentinel's
    claim is 'agrees with the *pinned oracle*', so resolving a different gcc
    than the one the table was measured against would weaken the claim.
    """
    failures: list[tuple] = []
    notes: list[str] = []
    lccc = Path(driver) if driver is not None else _driver()
    gcc = gcc or os.environ.get('GCC') or shutil.which('gcc')
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
                    proc = run_bounded(cmd, timeout=COMPILE_TIMEOUT)
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
                link = run_bounded(
                    [gcc, '-m32', '-nostdlib', '-static', '-no-pie', '-Wl,-e,_start',
                     *objects, '-o', str(exe)], timeout=LINK_TIMEOUT)
                if link.returncode != 0:
                    notes.append(f'skipped sentinel {spelling} ({label}): 32-bit link '
                                 f'unavailable ({link.stderr.strip().splitlines()[:1]})')
                    continue
                code = run_bounded([str(exe)], timeout=RUN_TIMEOUT).returncode
                if code != 123:
                    failures.append((f'sentinel:{spelling}', f'{label} disagrees on the ABI',
                                     123, code))
    return failures, notes


def check_probe_model() -> list[str]:
    """Red-team the assembly classifier independently of the live oracle."""
    body = """\
    pushq %rbx
    movq %r12, %rax
    movl %edi, %ecx
    andq $-16, %rsp
    popq %rbx
    ret $4
    """
    expected = (
        ('r12', 'rdi'),
        (('rbx', 'pop'), ('rbx', 'push')),
        True,
        (('ret', '$4'),),
    )
    actual = abi_signature('int', body)
    return [] if actual == expected else [f'probe parser expected {expected}, got {actual}']


def main() -> int:
    model_failures = check_probe_model()
    if model_failures:
        print('FAIL: ABI probe model self-test')
        for failure in model_failures:
            print('  ', failure)
        return 1
    gcc = shutil.which(os.environ.get('GCC') or 'gcc')
    if not gcc:
        print('SKIP: no gcc found')
        return 1
    has_i686 = run_bounded([gcc, '-m32', '-E', '-x', 'c', '-', '-o', os.devnull],
                           timeout=COMPILE_TIMEOUT,
                           input='int x;\n').returncode == 0
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
