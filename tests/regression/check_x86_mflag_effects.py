#!/usr/bin/env python3
"""The x86 machine options that are a rule about the finished text.

`-mharden-sls=`, `-mcld`, the `notrack` of a `switch` under
`-fcf-protection=branch`, and the `-pg` family's names and negatives were
either refused, swallowed, or (the jump of a `switch`) wrong: a tracked
indirect jump to case labels that carry no `endbr`, which a processor with
indirect-branch tracking stops with a control-protection fault.

What is checked, in both modes:

1. `-mharden-sls=`: in the compiler's own code every `ret` and every
   indirect jump is followed by `int3` (per value of the option), nothing
   else is, an `asm` statement's body is untouched, and the same holds
   function by function for GCC 16.2 (the two returns where lccc is
   stricter are named in `src/backend/x86_late_text.rs`). The program
   runs: the traps are never executed.
2. `-mcld`: every string instruction the compiler writes has `cld` in
   front of it; without the option there is none on x86-64.
3. `-fcf-protection=branch`: the dispatch of a `switch` is `notrack` in
   both shapes (through the table's name, and through a register in
   position-independent code), a computed goto and a sibling call are not,
   and their targets carry the `endbr`.
4. `-pg`: `-mfentry-name=`, `-mfentry-section=`, and the `-mno-` forms that
   used to be swallowed (`-mfentry -mno-fentry` called `__fentry__`).
5. `-mapx-inline-asm-use-gpr32` defines GCC's macro and its `-mno-` form
   takes it back.

Each rule is also run against the option switched off, so that a check
which cannot fail is seen.
"""
from __future__ import annotations

import argparse
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ENV = {**os.environ, 'LC_ALL': 'C', 'LANG': 'C'}

FIXTURE = r'''
#include <string.h>
extern int ext(int);
extern int (*fp)(int);
struct ops { int (*fn)(int); };
struct big { long a[64]; };
int plain(int a) { return a + 1; }
int tail_reg(struct ops *o) { return o->fn(1); }
int tail_mem(int a) { return fp(a); }
int tail_direct(int a) { return ext(a); }
int call_ind(int a) { return fp(a) + 1; }
int sw(int a) {
    switch (a) {
    case 0: return ext(1);
    case 1: return ext(22);
    case 2: return ext(5);
    case 3: return ext(99);
    case 4: return ext(7);
    case 5: return ext(13);
    case 6: return ext(77);
    default: return 0;
    }
}
void *cgoto(int a) {
    static void *tab[] = { &&l0, &&l1, &&l2 };
    goto *tab[a];
l0: ext(0);
l1: ext(1);
l2: return tab[0];
}
int with_asm(int a) { __asm__("jmp 1f\n\tret\n1:"); return a; }
/* What a person wrote outside a function, and the body of a naked function:
   a return, an indirect jump and a string instruction, never called. A
   trap or a `cld` put in here would move bytes somebody counted (the
   kernel's static-call trampolines are `ret; int3; nop; nop; nop`). */
__asm__(".pushsection .text\n"
        ".globl hand\n"
        ".type hand, @function\n"
        "hand:\n"
        "\trep movsb\n"
        "\tjmp *(%esp)\n"
        "\tret\n"
        ".size hand, .-hand\n"
        ".popsection");
__attribute__((naked)) void bare(void) { __asm__("rep stosb\n\tjmp *(%esp)\n\tret"); }
struct big retbig(const struct big *p) { return *p; }
void copy(struct big *d, const struct big *s) { *d = *s; }
void fill(char *d, unsigned long n) { memset(d, 0, n); }
/* An argument of 32 KiB is copied with a string instruction. */
struct huge { long a[4096]; };
extern void byval(struct huge h);
void pass(const struct huge *p) { byval(*p); }
'''

MAIN = r'''
#include <stdio.h>
struct ops { int (*fn)(int); };
struct big { long a[64]; };
int plain(int), tail_reg(struct ops *), tail_mem(int), tail_direct(int), call_ind(int), sw(int), with_asm(int);
void *cgoto(int);
struct big retbig(const struct big *);
void copy(struct big *, const struct big *);
struct huge { long a[4096]; };
void pass(const struct huge *);
static long seen;
void byval(struct huge h) { seen = h.a[0] * 3 + h.a[2048] * 5 + h.a[4095] * 7; }
static struct huge source;
int ext(int a) { return a * 3 + 1; }
static int twice(int a) { return a * 2; }
int (*fp)(int) = twice;
int main(void) {
    struct ops o = { twice };
    struct big b, c, d;
    long sum = 0;
    for (int i = 0; i < 64; i++) b.a[i] = i * 7 + 1;
    copy(&c, &b);
    d = retbig(&c);
    for (int i = 0; i < 64; i++) sum += d.a[i];
    for (int i = 0; i < 4096; i++) source.a[i] = i + 11;
    pass(&source);
    sum += seen;
    printf("%d %d %d %d %d %d %ld", plain(1), tail_reg(&o), tail_mem(5), tail_direct(6), call_ind(7),
           with_asm(8), sum);
    for (int i = -1; i < 9; i++) printf(" %d", sw(i));
    printf(" %d\n", cgoto(0) != 0);
    return 0;
}
'''

FAILURES: list[str] = []


def fail(message: str) -> None:
    FAILURES.append(message)


def run(cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, env=ENV, **kw)


def compile_asm(cc, flags, src) -> str:
    proc = run([cc, *flags, '-w', '-S', '-o', '-', src])
    if proc.returncode != 0:
        fail(f'{Path(cc).name} {" ".join(flags)}: {proc.stderr.strip()[:200]}')
        return ''
    return proc.stdout


def instructions(asm: str):
    """(function, statement, inside an asm statement) for every instruction."""
    function, in_asm = None, False
    for line in asm.splitlines():
        text = line.split('#')[0].strip() if not line.strip().startswith('#') else line.strip()
        if text == '#APP':
            in_asm = True
            continue
        if text == '#NO_APP':
            in_asm = False
            continue
        if not text or text.startswith('#') or text.startswith('.'):
            continue
        label = re.match(r'^([A-Za-z_][\w.]*):', text)
        if label:
            function = label.group(1)
            continue
        if text.endswith(':'):
            continue
        yield function, re.sub(r'\s+', ' ', text), in_asm


HAND_WRITTEN = sorted(['jmp 1f', 'ret', 'rep movsb', 'jmp *(%esp)', 'ret', 'rep stosb', 'jmp *(%esp)', 'ret'])
RETURN = re.compile(r'^ret[ql]?( \$\d+)?$')
INDIRECT = re.compile(r'^(notrack )?jmp[ql]? (\*|__x86_indirect_thunk_)')


def traps(asm: str):
    """For each return and indirect jump of the compiler's own code:
    (function, kind, followed by int3)."""
    out, items = [], list(instructions(asm))
    for at, (function, text, in_asm) in enumerate(items):
        if in_asm:
            continue
        kind = 'ret' if RETURN.match(text) else 'jmp' if INDIRECT.match(text) else None
        if kind:
            trapped = at + 1 < len(items) and items[at + 1][1] == 'int3'
            out.append((function, kind, trapped, text))
    return out


def check_sls(lccc, gcc, src, mode):
    values = {'none': set(), 'return': {'ret'}, 'indirect-jmp': {'jmp'}, 'all': {'ret', 'jmp'}}
    for value, want in values.items():
        for extra in ([], ['-fPIC'], ['-fcf-protection=branch'], ['-mfunction-return=thunk-extern'],
                      ['-mindirect-branch=thunk-extern', '-mindirect-branch-register']):
            flags = [f'-m{mode}', '-O2', f'-mharden-sls={value}', *extra]
            asm = compile_asm(lccc, flags, src)
            found = traps(asm)
            if not any(kind == 'ret' for _, kind, _, _ in found) and '-mfunction-return=thunk-extern' not in extra:
                fail(f'{" ".join(flags)}: no return found (the reader is blind)')
            for function, kind, trapped, text in found:
                if trapped != (kind in want):
                    fail(f'{" ".join(flags)}: {function}: `{text}` is {"" if trapped else "not "}followed by int3')
            count = sum(1 for _, text, in_asm in instructions(asm) if text == 'int3' and not in_asm)
            if count != sum(1 for _, kind, _, _ in found if kind in want):
                fail(f'{" ".join(flags)}: {count} int3, expected one behind each of the '
                     f'{sum(1 for _, kind, _, _ in found if kind in want)} returns and jumps asked for')
            # What a person wrote is as it was written: the asm statement,
            # the asm outside any function, the naked function's body.
            body = sorted(text for _, text, in_asm in instructions(asm) if in_asm)
            if body != HAND_WRITTEN:
                fail(f'{" ".join(flags)}: the hand-written assembly reads {body}')
    if not gcc:
        return
    # GCC 16.2: the same kinds of statement are trapped. `ret $n` is the
    # exception named in x86_late_text.rs.
    for value, want in values.items():
        flags = [f'-m{mode}', '-O2', '-fno-pic', f'-mharden-sls={value}']
        for function, kind, trapped, text in traps(compile_asm(gcc, flags, src)):
            if ' $' in text:
                continue
            if trapped != (kind in want):
                fail(f'GCC {" ".join(flags)}: {function}: `{text}` '
                     f'{"is" if trapped else "is not"} followed by int3; the rule here says otherwise')


def check_cld(lccc, src, mode):
    for flags in (['-O2'], ['-Os'], ['-O0']):
        base = [f'-m{mode}', *flags]
        for on in (False, True):
            asm = compile_asm(lccc, base + (['-mcld'] if on else ['-mno-cld']), src)
            items = [text for _, text, in_asm in instructions(asm) if not in_asm]
            strings = [at for at, text in enumerate(items) if re.match(r'^rep(e|z|ne|nz)? ', text)]
            if not strings:
                fail(f'-m{mode} {" ".join(flags)}: the fixture has no string instruction (the check is blind)')
            for at in strings:
                cleared = at > 0 and items[at - 1] == 'cld'
                if on and not cleared:
                    fail(f'-m{mode} {" ".join(flags)} -mcld: `{items[at]}` without cld in front')
                if not on and cleared:
                    fail(f'-m{mode} {" ".join(flags)} -mno-cld: cld in front of `{items[at]}`')
            if on and any(items[at] == 'cld' and items[at + 1] == 'cld' for at in range(len(items) - 1)):
                fail(f'-m{mode} {" ".join(flags)} -mcld: cld twice in a row')
            written = sorted(text for _, text, in_asm in instructions(asm) if in_asm)
            if written != HAND_WRITTEN:
                fail(f'-m{mode} {" ".join(flags)}: the hand-written assembly reads {written}')


def check_notrack(lccc, gcc, src, mode):
    for pic in (['-fno-pic'], ['-fPIC']):
        for cet in (False, True):
            flags = [f'-m{mode}', '-O2', *pic] + (['-fcf-protection=branch'] if cet else [])
            asm = compile_asm(lccc, flags, src)
            per: dict[str, list[str]] = {}
            for function, text, in_asm in instructions(asm):
                if not in_asm and INDIRECT.match(text):
                    per.setdefault(function, []).append(text)
            dispatch = per.get('sw', [])
            if not dispatch:
                fail(f'{" ".join(flags)}: `sw` has no indirect jump (the check is blind)')
            for text in dispatch:
                if text.startswith('notrack ') != cet:
                    fail(f'{" ".join(flags)}: the dispatch of `sw` is `{text}`')
            for function in ('cgoto', 'tail_reg', 'tail_mem'):
                for text in per.get(function, []):
                    if text.startswith('notrack '):
                        fail(f'{" ".join(flags)}: `{function}` jumps with `{text}`: its target has an endbr')
            if cet:
                endbr = 'endbr64' if mode == '64' else 'endbr32'
                labels = sum(1 for function, text, _ in instructions(asm) if function == 'cgoto' and text == endbr)
                if labels < 4:
                    fail(f'{" ".join(flags)}: `cgoto` has {labels} {endbr}, expected the entry and three labels')
    if gcc:
        asm = compile_asm(gcc, [f'-m{mode}', '-O2', '-fno-pic', '-fcf-protection=branch'], src)
        dispatch = [text for function, text, _ in instructions(asm) if function == 'sw' and INDIRECT.match(text)]
        if not dispatch or not all(text.startswith('notrack ') for text in dispatch):
            fail(f'GCC -m{mode}: the dispatch of `sw` is {dispatch}; the rule here follows GCC\'s notrack')


def check_run(lccc, gcc, src, main, tmp, mode):
    """The program's output is the same with every rule on."""
    def build(cc, flags, name):
        exe = os.path.join(tmp, name)
        proc = run([cc, f'-m{mode}', '-O2', '-w', *flags, src, main, '-o', exe])
        if proc.returncode != 0:
            return None, proc.stderr.strip()[:200]
        out = run([exe])
        return (out.returncode, out.stdout), None

    reference, error = build(lccc, [], f'plain{mode}')
    if reference is None:
        if mode == '32':
            print(f'  -m32: cannot link here ({error}); the run is skipped')
            return
        fail(f'-m{mode}: the fixture does not link: {error}')
        return
    if reference[0] != 0 or not reference[1].strip():
        fail(f'-m{mode}: the fixture exits {reference[0]}')
    for flags in (['-mharden-sls=all'], ['-mcld'], ['-mharden-sls=all', '-mcld', '-fcf-protection=branch'],
                  ['-mharden-sls=all', '-mcld', '-fPIC'], ['-Os', '-mharden-sls=all', '-mcld']):
        got, error = build(lccc, flags, 'on' + mode)
        if got != reference:
            fail(f'-m{mode} {" ".join(flags)}: output {got or error}, without the options {reference}')
    if gcc:
        got, error = build(gcc, [], 'gcc' + mode)
        if got is not None and got != reference:
            fail(f'-m{mode}: GCC prints {got}, lccc {reference}')


def check_mcount(lccc, src):
    def first(flags):
        asm = compile_asm(lccc, ['-O2', '-pg', *flags], src)
        body = [text for function, text, _ in instructions(asm) if function == 'plain']
        sections = re.findall(r'^\s*\.section\s+([^,\s]+)', asm, re.M)
        return body, sections

    cases = [
        (['-mfentry'], 'call __fentry__', None),
        (['-mfentry', '-mno-fentry'], 'call mcount', None),
        (['-mno-fentry', '-mfentry'], 'call __fentry__', None),
        (['-mfentry', '-mfentry-name=my_entry'], 'call my_entry', None),
        (['-mfentry-name=my_entry', '-mfentry', '-mno-fentry'], 'call mcount', None),
        (['-mfentry', '-mrecord-mcount'], 'call __fentry__', '__mcount_loc'),
        (['-mfentry', '-mrecord-mcount', '-mfentry-section=.my_records'], 'call __fentry__', '.my_records'),
        (['-mfentry', '-mrecord-mcount', '-mno-record-mcount'], 'call __fentry__', None),
        (['-mfentry', '-mfentry-section=.my_records'], 'call __fentry__', None),
        (['-mfentry', '-mnop-mcount', '-mno-nop-mcount'], 'call __fentry__', None),
    ]
    for flags, call, section in cases:
        body, sections = first(flags)
        if call not in body:
            fail(f'-pg {" ".join(flags)}: `plain` has no `{call}`: {body[:4]}')
        for other in ('call __fentry__', 'call mcount', 'call my_entry'):
            if other != call and other in body:
                fail(f'-pg {" ".join(flags)}: `plain` has `{other}`')
        records = [name for name in sections if name in ('__mcount_loc', '.my_records')]
        if (section is None and records) or (section is not None and set(records) != {section}):
            fail(f'-pg {" ".join(flags)}: record sections {sorted(set(records))}, expected {section}')
    for flags, message in ((['-mfentry-name='], "missing argument to '-mfentry-name='"),
                           (['-mfentry-section='], "missing argument to '-mfentry-section='")):
        proc = run([lccc, '-O2', '-pg', *flags, '-S', '-o', os.devnull, src])
        if proc.returncode == 0 or message not in proc.stderr:
            fail(f'{" ".join(flags)}: exit {proc.returncode}, {proc.stderr.strip()[:120]!r}')


def check_macro(lccc, gcc, src):
    for mode in ('64', '32'):
        for flags, want in ((['-mapx-inline-asm-use-gpr32'], True), ([], False),
                            (['-mapx-inline-asm-use-gpr32', '-mno-apx-inline-asm-use-gpr32'], False),
                            (['-mno-apx-inline-asm-use-gpr32', '-mapx-inline-asm-use-gpr32'], True)):
            for cc in filter(None, (lccc, gcc)):
                proc = run([cc, f'-m{mode}', *flags, '-dM', '-E', src])
                got = '#define __APX_INLINE_ASM_USE_GPR32__ 1' in proc.stdout
                if proc.returncode != 0 or got != want:
                    fail(f'{Path(cc).name} -m{mode} {" ".join(flags)}: __APX_INLINE_ASM_USE_GPR32__ '
                         f'{"defined" if got else "not defined"} (exit {proc.returncode})')


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    parser.add_argument('--lccc', default=os.environ.get('CCC', 'target/fastbuild/lccc'))
    parser.add_argument('--gcc', default=os.environ.get('GCC'),
                        help='GCC 16.2, to compare the rules with (optional)')
    args = parser.parse_args(argv)
    gcc = args.gcc if args.gcc and shutil.which(args.gcc) else None
    if gcc and '16.2' not in run([gcc, '-dumpfullversion']).stdout:
        print(f'  {gcc} is not 16.2: the comparison with GCC is skipped')
        gcc = None
    with tempfile.TemporaryDirectory() as tmp:
        src, main_src = os.path.join(tmp, 'rules.c'), os.path.join(tmp, 'main.c')
        Path(src).write_text(FIXTURE)
        Path(main_src).write_text(MAIN)
        for mode in ('64', '32'):
            check_sls(args.lccc, gcc, src, mode)
            check_cld(args.lccc, src, mode)
            check_notrack(args.lccc, gcc, src, mode)
            check_run(args.lccc, gcc, src, main_src, tmp, mode)
        check_mcount(args.lccc, src)
        check_macro(args.lccc, gcc, src)
    if FAILURES:
        for message in FAILURES[:40]:
            print(f'check_x86_mflag_effects: {message}')
        print(f'check_x86_mflag_effects: FAIL ({len(FAILURES)} problem(s))')
        return 1
    print('check_x86_mflag_effects: PASS (-mharden-sls=, -mcld, notrack switch dispatch, -pg names and '
          f'negatives, the APX asm macro; -m64 and -m32{", against GCC 16.2" if gcc else ""})')
    return 0


if __name__ == '__main__':
    sys.exit(main())
