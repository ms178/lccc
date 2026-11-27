#!/usr/bin/env python3
"""The x86 `-m` options GCC 16.2 has and lccc has no code generation for.

Why this exists
---------------
The driver has an arm for every machine option it implements.  Everything
else went to two catch-alls: an option was an error ("unsupported machine
option"), its `-mno-` form was accepted without a word.  Measured against
GCC 16.2's own list (`gcc --completion=-m`, 666 spellings), that refused 119
spellings GCC accepts and swallowed 81 `-mno-` spellings, and neither
half was right for all of them:

* `-mlong-double-80`, `-masm=att`, `-mtls-direct-seg-refs`, `-mfpmath=sse`,
  `-mieee-fp`, `-mglibc`, ... name the state lccc is in.  Refusing them stops
  a build that asks for what it would get.
* `-mbranch-cost=`, `-mtune-ctrl=`, `-mstringop-strategy=`, `-mstv`, ... choose
  among ways of generating the same code.  No object, ABI or language rule
  depends on them.
* `-mno-direct-extern-access`, `-mno-tls-direct-seg-refs`,
  `-mno-fancy-math-387` (IA-32) ask for a state lccc is *not* in.  Accepting
  them in silence is the thing the driver's policy forbids.
* `-msoft-float -mno-soft-float`, `-mfentry -mno-fentry`,
  `-mskip-rax-setup -mno-skip-rax-setup`: the `-mno-` form of an option the
  driver implements was swallowed, so the first one stayed in force.

This file is the decision for every one of those spellings, in both modes
(`-m64`, `-m32`), with the reason.  It generates

* `src/driver/x86_mflag_policy.rs`: the table the driver consults for a
  machine option none of its arms took, and
* `docs/x86-machine-options.md`: the same table for people,

and checks both against the compilers:

    gen_x86_mflag_policy.py --write            regenerate the two files
    gen_x86_mflag_policy.py --check            the committed files are current
    gen_x86_mflag_policy.py --check --lccc BIN [--gcc gcc-16]
        every row behaves as the table says under BIN, and, with --gcc:
        every spelling of GCC's list is decided (none reaches the driver's
        last resort), nothing GCC refuses is accepted, and the warnings
        taken over from GCC are GCC's words.

The verdicts
------------
state     accepted in silence: what is asked is what lccc does, always
hint      accepted with a warning that nothing acts on it (a preference
          among equivalent code; `LCCC_STRICT_OPTIONS=1` makes it an error)
ignored   GCC 16.2 accepts and ignores it with a warning; lccc prints the
          same warning and ignores it
effect    implemented; the driver has an arm for it (the row is here for
          the document and the check)
depends   decided by the rest of the command line (see `DEPENDS`)
refused   an error with the reason
invalid   GCC 16.2 refuses it in this mode; lccc refuses it in GCC's words
"""
from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
import tempfile
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
RUST = ROOT / 'src/driver/x86_mflag_policy.rs'
DOC = ROOT / 'docs/x86-machine-options.md'
EXPECTED_GCC = '16.2.0'
ENV = {**os.environ, 'LC_ALL': 'C', 'LANG': 'C'}


# --------------------------------------------------------------------------- verdicts
def S(why):
    return ('state', why)


def H(why):
    return ('hint', why)


def W(text):
    return ('ignored', text)


def E(why):
    return ('effect', why)


def D(why):
    return ('depends', why)


def X(why):
    return ('refused', why)


def G(text):
    return ('invalid', text)


def both(verdict):
    return {'64': verdict, '32': verdict}


def modes(x86_64, ia32):
    return {'64': x86_64, '32': ia32}


# Reasons used more than once.
TUNING = 'a choice among ways of generating the same code; nothing in the object, the ABI or the language depends on it'
NEVER = 'the instructions are never generated'
NO_INTRINSICS = ('its intrinsics are not provided, and the macro GCC defines for it is a promise of them '
                 '(docs/x86-isa-permissions-and-simd-abi.md, section 1.6)')
NO_REALIGN = ('functions are compiled for a stack that is 16-byte aligned when they are entered, '
              'and cannot realign it')
LIBC = 'it selects another C library (headers, start files, dynamic linker); LCCC builds for glibc'


def tuning():
    """A switch between equivalent code: a hint either way."""
    return both(H(TUNING)), both(H(TUNING))


# --------------------------------------------------------------------------- boolean options
# name -> (on, off); each a {'64': verdict, '32': verdict}, or None where GCC
# 16.2 has no such spelling (`-mno-glibc` does not exist).
BOOLEAN: dict[str, tuple] = {}


def B(name, on, off):
    assert name not in BOOLEAN, name
    BOOLEAN[name] = (on, off)


for _name in ('8bit-idiv', 'avx256-split-unaligned-load', 'avx256-split-unaligned-store',
              'call-ms2sysv-xlogues', 'gather', 'scatter', 'inline-all-stringops',
              'inline-stringops-dynamically', 'noreturn-no-callee-saved-registers',
              'omit-leaf-frame-pointer', 'push-args', 'recip', 'relax-cmpxchg-loop', 'stv',
              'unroll-only-small-loops', 'accumulate-outgoing-args', 'align-stringops'):
    B(_name, *tuning())
B('dispatch-scheduler', both(H(TUNING)), None)
B('default', None, both(H('`-mno-default` clears GCC\'s tuning features: ' + TUNING)))
B('vzeroupper',
  both(S('`vzeroupper` is written before a call or return whenever a 256-bit register was written')),
  both(H('`vzeroupper` stays where it is written: it avoids a transition penalty and changes no value '
         'the ABI defines')))
B('hle', both(H('the XACQUIRE and XRELEASE prefixes are hints to elide a lock; an atomic operation '
                'without them does the same thing on every processor')),
  both(S('no XACQUIRE or XRELEASE prefix is ever generated')))
B('ieee-fp', both(S('floating-point comparisons treat unordered operands as IEEE 754 has them')),
  both(H('a permission to compare without regard to unordered operands; comparisons stay IEEE')))
B('fancy-math-387', both(S('a permission, and GCC\'s default: nothing is promised by it')),
  modes(S('no `fsin`, `fcos` or `fsqrt` is generated for x86-64'),
        X('the IA-32 back end expands `sqrt` to `fsqrt`; generating a call instead is not implemented')))
B('partial-vector-fp-math',
  both(S('a permission, and GCC\'s default: nothing is promised by it')),
  both(X('floating-point operations on vectors narrower than the register are not audited for the '
         'exceptions their unused lanes can raise, so the restriction cannot be promised')))

# ---- ABI, layout and conventions --------------------------------------------------------
B('128bit-long-double',
  modes(S('`long double` is 16 bytes on x86-64 with or without it'),
        X('it makes `long double` 16 bytes on IA-32; LCCC implements the 12-byte psABI layout only')),
  None)
B('96bit-long-double',
  modes(G("'-m96bit-long-double' is not compatible with this target"),
        S('`long double` is 12 bytes on IA-32')),
  None)
B('long-double-80', both(S('`long double` is the x87 80-bit format')), None)
B('long-double-64', both(X('it makes `long double` a 64-bit `double`; LCCC implements the x87 80-bit '
                           'format only, and so does the C library it links')), None)
B('long-double-128', both(X('it makes `long double` IEEE binary128; LCCC implements the x87 80-bit '
                            'format only, and so does the C library it links')), None)
B('align-double',
  modes(S('`double` and `long long` members are 8-byte aligned on x86-64 with or without it '
          '(GCC\'s output is identical)'),
        X('it aligns `double` and `long long` members to 8 bytes on IA-32, which changes the layout '
          'of every structure that has one; LCCC implements the psABI layout only')),
  both(S('structure members have their psABI alignment')))
B('ms-bitfields', both(X('it lays bit-fields out as the Microsoft compiler does; LCCC implements the '
                         'System V layout only')),
  both(S('bit-fields have the System V layout')))
B('rtd',
  modes(W("'-mrtd' is ignored in 64bit mode"),
        X('it makes every function pop its own arguments (`ret $n`): another calling convention '
          'for the whole translation unit')),
  both(S('the caller pops the arguments')))
B('sseregparm',
  modes(S('`float` and `double` arguments travel in SSE registers on x86-64 with or without it'),
        X('it passes `float` and `double` arguments in SSE registers on IA-32: another calling '
          'convention for the whole translation unit')),
  None)
B('vect8-ret-in-mem',
  modes(S('it changes nothing on x86-64 (GCC\'s output is identical)'),
        X('it returns 8-byte vectors in memory on IA-32: another return convention')),
  both(S('8-byte vectors are returned as the psABI has it')))
B('x32', both(X('the x32 ABI (ILP32 on x86-64) is not implemented')), None)
B('iamcu',
  modes(G("Intel MCU psABI isn't supported in 64-bit mode"),
        X('the Intel MCU psABI is not implemented')),
  both(S('the code is not built for the Intel MCU psABI')))
B('glibc', both(S('LCCC builds for glibc')), None)
for _name in ('bionic', 'musl', 'uclibc'):
    B(_name, both(X(LIBC)), None)
B('android', both(X(LIBC)), both(S('LCCC builds for glibc')))

# ---- the stack ---------------------------------------------------------------------------
B('stackrealign', both(X('no dynamic stack realignment: a function entered with a misaligned stack '
                         'would run with misaligned locals')),
  both(S('no function realigns its stack')))
B('force-drap', both(X('no dynamic stack realignment, so there is no argument pointer to force')),
  both(S('no function realigns its stack')))
B('stack-arg-probe', both(X('stack probing through a call to `___chkstk_ms` is not implemented; '
                            '`-fstack-clash-protection` probes inline')),
  both(S('no stack probe is called')))

# ---- hardening and instrumentation ---------------------------------------------------------
B('cet-switch', both(X('under `-fcf-protection=branch` the jump of a `switch` is `notrack`, as in GCC '
                       'without this option; a tracked jump with an `endbr` at every case label is '
                       'not implemented')),
  both(S('the jump of a `switch` is `notrack` under `-fcf-protection=branch`')))
B('manual-endbr', both(X('under `-fcf-protection=branch` every function begins with `endbr`; limiting '
                         'that to functions marked `cf_check` is not implemented')),
  both(S('every function begins with `endbr` under `-fcf-protection=branch`')))
B('force-indirect-call', both(X('making every call through a register or the GOT is not implemented')),
  both(S('a direct call is a direct call')))
B('record-return', both(X('there are no `__return_loc` records: `-minstrument-return=` is not '
                          'implemented')),
  both(S('returns are not recorded')))
B('needed', both(X('it records the ISA level the object needs in a GNU property note '
                   '(GNU_PROPERTY_X86_ISA_1_NEEDED); the note is not written')),
  both(S('no ISA level is recorded in the object')))
B('direct-extern-access',
  both(S('an external object is addressed directly (a copy relocation in an executable)')),
  both(X('it routes every access to an external object or function address through the GOT and marks '
         'the object with GNU_PROPERTY_1_NEEDED_INDIRECT_EXTERN_ACCESS; neither is implemented')))
B('tls-direct-seg-refs',
  both(S('a thread-local object is addressed as an offset from `%fs` or `%gs`')),
  both(X('loading the thread pointer and adding the offset, for systems where a segment-relative '
         'access is not usable, is not implemented')))
B('cld', both(E('`cld` in front of every string instruction the compiler writes')),
  both(S('the direction flag is taken to be clear, as the ABI has it')))
# The `-mno-` forms of options the driver implements. They went to the
# catch-all and changed nothing, so the option given first stayed in force:
# `-mfentry -mno-fentry -pg` called `__fentry__`.
B('fentry', both(E('`-pg` calls `__fentry__` in front of the prologue')),
  both(E('`-pg` calls `mcount` behind the frame')))
B('record-mcount', both(E('each profiling call is recorded in `__mcount_loc`')),
  both(E('no records')))
B('nop-mcount', both(E('a five-byte nop in place of the profiling call')),
  both(E('the profiling call is a call')))
B('skip-rax-setup', both(E('`%al` is not set for a variadic call (with `-mno-sse`)')),
  both(E('`%al` is set for a variadic call')))
B('soft-float', both(E('no x87 instruction')), both(E('as `-mhard-float`: the x87 is used')))
B('prefer-avx128', both(E('as `-mprefer-vector-width=128`')),
  both(E('as `-mprefer-vector-width=256`: GCC 16.2 gives the negative of this alias that meaning')))
B('indirect-branch-cs-prefix', both(E('a CS prefix before a call or jump to an indirect-branch thunk')),
  both(E('no such prefix')))
B('indirect-branch-register',
  both(E('nothing to do: a call or tail call through a pointer goes through a register')),
  both(S('a permission to branch through memory')))

# ---- startup objects ----------------------------------------------------------------------
B('daz-ftz', both(X('it links `crtfastmath.o`, which sets FTZ and DAZ in MXCSR at startup; LCCC does '
                    'not link that object')),
  both(S('`crtfastmath.o` is never linked, not under `-ffast-math` either')))
B('pc80', both(S('the x87 precision control is left as the system sets it, which is 80-bit')), None)
B('pc64', both(X('it links `crtprec64.o`, which sets the x87 to 53-bit precision at startup; LCCC does '
                 'not link that object')), None)
B('pc32', both(X('it links `crtprec32.o`, which sets the x87 to 24-bit precision at startup; LCCC does '
                 'not link that object')), None)

# ---- the assembler ------------------------------------------------------------------------
B('sse2avx', both(X('it has the assembler encode SSE instructions with VEX; LCCC\'s assembler has no '
                    'such mode')),
  both(S('an SSE instruction is encoded as written')))

# ---- ISA extensions without intrinsics ----------------------------------------------------
for _name in ('3dnowa', 'amx-avx512', 'amx-complex', 'amx-fp16', 'amx-fp8', 'amx-movrs', 'amx-tf32',
              'fma4', 'kl', 'widekl', 'lwp', 'prefetchi', 'raoint', 'sse4a', 'tbm', 'xop'):
    B(_name, both(X(NO_INTRINSICS)), both(S(NEVER)))
B('crc32', both(D('the `crc32` instructions are permitted with SSE4.2 and not on their own')),
  both(D('`__CRC32__` and the `crc32` intrinsics follow SSE4.2 and cannot be withdrawn alone')))
B('mwait', both(D('MONITOR and MWAIT are permitted with SSE3 and not on their own')),
  both(D('MONITOR and MWAIT follow SSE3 and cannot be withdrawn alone')))
B('apx-inline-asm-use-gpr32',
  both(E('defines `__APX_INLINE_ASM_USE_GPR32__`; the permission itself, r16 to r31 for `asm` '
         'operands, is not used')),
  both(S('no `asm` operand is given one of r16 to r31')))

# ---- what GCC 16.2 itself only warns about ---------------------------------------------------
for _name in ('mitigate-rop', 'mpx', 'pcommit'):
    B(_name, both(W(f"switch '-m{_name}' is no longer supported")),
      both(S('GCC 16.2 has no such feature, nor has LCCC')))
B('ain', both(W("command-line option '-main' is valid for Cobol but not for C")),
  both(W("command-line option '-mno-ain' is valid for Cobol but not for C")))
B('dump-tune-features', both(X('it prints the names `-mtune-ctrl=` takes; LCCC has no such features')),
  None)

# --------------------------------------------------------------------------- aliases
# option -> (GCC's warning, what it stands for). Rewritten before anything
# else reads the command line, so the replacement is ordered as the alias was.
ALIASES = {
    '-mfused-madd': ("'-mfused-madd' is deprecated; use '-ffp-contract=' instead", '-ffp-contract=fast'),
    '-mno-fused-madd': ("'-mfused-madd' is deprecated; use '-ffp-contract=' instead", '-ffp-contract=off'),
    '-mintel-syntax': ("'-mintel-syntax' and '-mno-intel-syntax' are deprecated; use '-masm=intel' and "
                       "'-masm=att' instead", '-masm=intel'),
    '-mno-intel-syntax': ("'-mintel-syntax' and '-mno-intel-syntax' are deprecated; use '-masm=intel' and "
                          "'-masm=att' instead", '-masm=att'),
    '-msse5': ("'-msse5' was removed", '-mavx'),
    '-mno-sse5': ("'-msse5' was removed", '-mno-avx'),
}
# prefix -> (GCC's warning, the prefix it stands for)
PREFIX_ALIASES = {
    '-mstore-max=': ("'-mstore-max=' is deprecated; use '-mmove-max=' instead", '-mmove-max='),
}
# `-malign-NAME=N` is `-falign-NAME=2^N` where no `-falign-NAME` is given
# (measured: an explicit `-falign-functions=64` wins on either side of it).
OBSOLETE_ALIGN = ('functions', 'jumps', 'loops')

# --------------------------------------------------------------------------- valued options
# prefix -> ({value: modes}, rule for any other value)
#   'invalid'  GCC's "unrecognized argument in option"
#   a name     a checker in the driver (`Driver::x86_mflag_free_value`)
VALUED: dict[str, tuple] = {}


def V(prefix, values, other='invalid'):
    assert prefix not in VALUED, prefix
    VALUED[prefix] = (values, other)


V('-mabi=', {
    'sysv': both(S('functions have the System V calling convention unless they are declared otherwise')),
    'ms': both(X('it makes the Microsoft calling convention the default for every function; LCCC '
                 'implements that convention for functions declared `ms_abi` only')),
})
V('-malign-data=', {
    'compat': both(S('an object has at least its psABI alignment; GCC\'s default')),
    'abi': both(S('an object has at least its psABI alignment')),
    'cacheline': both(H('more alignment than the psABI asks for is a preference, and no object depends on '
                        'another translation unit having it')),
})
V('-maddress-mode=', {
    'long': modes(S('addresses are 64-bit'), G("address mode 'long' not supported in the 32 bit mode")),
    'short': modes(G("address mode 'short' not supported in the 64 bit mode"), S('addresses are 32-bit')),
})
V('-masm=', {
    'att': both(S('the output and the `asm` templates are in AT&T syntax')),
    'intel': both(X('it switches the output and the dialect of `asm` templates to Intel syntax; LCCC '
                    'writes and reads AT&T syntax only')),
})
V('-mfpmath=', {
    'sse': modes(S('`float` and `double` are computed in SSE registers'),
                 X('IA-32 code computes `float` in SSE registers and `double` in the x87 here; '
                   'computing both in SSE is not implemented')),
    '387': modes(X('computing `float` and `double` in the x87 on x86-64 is not implemented (it would '
                   'change results: the x87 keeps excess precision)'),
                 X('IA-32 code computes `float` in SSE registers and `double` in the x87 here; '
                   'computing both in the x87 is not implemented')),
    **{v: both(H('GCC calls this choice experimental and promises nothing by it: ' + TUNING))
       for v in ('sse+387', '387+sse', 'sse,387', '387,sse', 'both')},
})
V('-mfunction-return=', {
    'keep': both(E('a return is a `ret`')),
    'thunk': both(X('only `thunk-extern` is implemented: the return thunk is the program\'s to supply')),
    'thunk-inline': both(X('only `thunk-extern` is implemented: the return thunk is the program\'s to '
                           'supply')),
})
V('-mindirect-branch=', {
    'keep': both(E('an indirect branch is an indirect branch')),
    'thunk': both(X('`thunk-extern` and `thunk-inline` are implemented; a thunk defined in the object '
                    'itself is not')),
})
V('-minstrument-return=', {
    'none': both(S('returns are not instrumented')),
    'call': both(X('a call to `__return__` before each return is not implemented')),
    'nop5': both(X('a five-byte nop before each return is not implemented')),
})
V('-mlam=', {
    'none': both(S('no pointer bit is treated as a tag')),
    'u48': modes(X('linear address masking (tag bits in pointers) is not implemented'),
                 G("'-mlam=' option: [u48|u57] not supported for 32-bit code")),
    'u57': modes(X('linear address masking (tag bits in pointers) is not implemented'),
                 G("'-mlam=' option: [u48|u57] not supported for 32-bit code")),
}, other='lam')
V('-mmove-max=', {v: both(H(TUNING)) for v in ('128', '256', '512', 'none')})
V('-mstringop-strategy=', {
    **{v: both(H(TUNING)) for v in ('byte_loop', 'libcall', 'loop', 'rep_4byte', 'rep_byte',
                                    'unrolled_loop', 'vector_loop')},
    'rep_8byte': modes(H(TUNING), G("'-mstringop-strategy=rep_8byte' not supported for 32-bit code")),
})
V('-mveclibabi=', {v: both(H('a permission to call a vector math library; no such call is generated'))
                   for v in ('svml', 'acml', 'aocl')})
V('-mcmodel=', {
    '32': modes(G("code model '32' not supported in the 64 bit mode"),
                S('IA-32 code has one code model')),
}, other='elsewhere')
V('-mapx-features=', {}, other='apx_features')
V('-mno-apx-features=', {}, other='apx_features')
V('-mbranch-cost=', {}, other='branch_cost')
V('-mincoming-stack-boundary=', {}, other='incoming_stack_boundary')
V('-mlarge-data-threshold=', {}, other='large_data_threshold')
V('-mfentry-name=', {}, other='fentry_name')
V('-mfentry-section=', {}, other='fentry_section')
V('-mrecip=', {}, other='recip')
V('-mtune-ctrl=', {}, other='free_hint')
V('-mmemcpy-strategy=', {}, other='free_hint')
V('-mmemset-strategy=', {}, other='free_hint')
V('-main=', {}, other='cobol')
V('-mno-ain=', {}, other='cobol')

# What the checkers named above decide, for the document.
CHECKERS = {
    'apx_features': ('`none`, or any `-mno-apx-features=` list, is accepted while APX is off (`-mapxf` not in '
                     'force); otherwise refused: APX code generation is selected as a whole by `-mapxf`, '
                     'its features cannot be chosen one by one. Any list but `none` is an error on IA-32, '
                     'in GCC\'s words'),
    'branch_cost': '0 to 5: a hint. Anything else is GCC\'s error',
    'incoming_stack_boundary': ('4 (16 bytes): the state. 5 to 12: a hint (a promise of more alignment that '
                                'nothing uses). 3 on x86-64, 2 or 3 on IA-32: refused, ' + NO_REALIGN +
                                '. Anything else is GCC\'s error'),
    'large_data_threshold': ('refused: no object is placed in the large data sections (`-mcmodel=medium` is '
                             'compiled as the small model), so there is no threshold to set'),
    'fentry_name': 'implemented: the function a `-pg -mfentry` site calls. An empty name is GCC\'s error',
    'fentry_section': 'implemented: the section of the `-mrecord-mcount` records',
    'recip': ('a list of `all`, `default`, `none`, `div`, `sqrt`, `vec-div`, `vec-sqrt`, each with or without '
              '`!`: a hint (reciprocal estimates are never generated). Anything else is GCC\'s error'),
    'free_hint': 'any value: a hint. GCC checks the value against its tuning tables; LCCC has none',
    'cobol': 'GCC\'s warning that the option is for Cobol, and nothing else',
    'lam': 'GCC\'s error ("unknown lam type")',
    'elsewhere': 'the other code models have arms of their own in the driver',
}

# The rules of `depends` rows, for the document.
DEPENDS = {
    'crc32': ('`-mcrc32` is accepted where SSE4.2 is enabled and refused where it is not; `-mno-crc32` is '
              'accepted where SSE4.2 is not enabled and refused where it is. GCC has a CRC32 bit of its '
              'own; here `__CRC32__` and the permission to call the intrinsics are SSE4.2\'s'),
    'mwait': ('`-mmwait` is accepted where SSE3 is enabled and refused where it is not; `-mno-mwait` the '
              'other way round. GCC has a MWAIT bit of its own; here `_mm_monitor` and `_mm_mwait` are '
              'SSE3\'s'),
}

VERDICTS = ('state', 'hint', 'ignored', 'effect', 'depends', 'refused', 'invalid')
RUST_VARIANT = {'state': 'State', 'hint': 'Hint', 'effect': 'Effect', 'depends': 'Depends'}


# --------------------------------------------------------------------------- rendering: Rust
def rust_str(text: str) -> str:
    return '"' + text.replace('\\', '\\\\').replace('"', '\\"') + '"'


def rust_verdict(verdict) -> str:
    kind, text = verdict
    if kind in RUST_VARIANT:
        return f'Verdict::{RUST_VARIANT[kind]}'
    variant = {'ignored': 'Ignored', 'refused': 'Refused', 'invalid': 'Invalid'}[kind]
    return f'Verdict::{variant}({rust_str(text)})'


def rust_modes(m) -> str:
    if m is None:
        return 'None'
    return f'Some(Modes {{ x86_64: {rust_verdict(m["64"])}, ia32: {rust_verdict(m["32"])} }})'


def render_rust() -> str:
    out = [
        '// @generated by scripts/gen_x86_mflag_policy.py --write; do not edit.',
        '//',
        '// The x86 machine options of GCC 16.2 that no arm of the driver takes:',
        '// what is done with each, in each mode, and why. The reasons and the',
        '// measurements behind them are in the script.',
        '',
        '/// What is done with one spelling in one mode.',
        '#[derive(Clone, Copy, Debug, PartialEq, Eq)]',
        'pub(crate) enum Verdict {',
        '    /// Accepted in silence: what is asked is what LCCC does, always.',
        '    State,',
        '    /// Accepted with a warning that nothing acts on it: a preference',
        '    /// among equivalent code.',
        '    Hint,',
        '    /// GCC 16.2 accepts and ignores it with this warning; so does LCCC.',
        '    Ignored(&\'static str),',
        '    /// Implemented by an arm of the driver.',
        '    Effect,',
        '    /// Decided by the rest of the command line.',
        '    Depends,',
        '    /// An error, with the reason.',
        '    Refused(&\'static str),',
        '    /// GCC 16.2 refuses it in this mode, in these words.',
        '    Invalid(&\'static str),',
        '}',
        '',
        '/// The verdicts of one spelling.',
        '#[derive(Clone, Copy, Debug, PartialEq, Eq)]',
        'pub(crate) struct Modes {',
        '    pub x86_64: Verdict,',
        '    pub ia32: Verdict,',
        '}',
        '',
        '/// `-mNAME` and `-mno-NAME`; `None` where GCC 16.2 has no such spelling.',
        'pub(crate) struct Boolean {',
        '    pub name: &\'static str,',
        '    pub on: Option<Modes>,',
        '    pub off: Option<Modes>,',
        '}',
        '',
        '/// What is done with a value the table does not list.',
        '#[derive(Clone, Copy, Debug, PartialEq, Eq)]',
        'pub(crate) enum OtherValue {',
        '    /// GCC\'s "unrecognized argument in option".',
        '    Invalid,',
        '    /// The checker of this name in the driver.',
        '    Checked(&\'static str),',
        '}',
        '',
        '/// `-mNAME=VALUE`.',
        'pub(crate) struct Valued {',
        '    pub prefix: &\'static str,',
        '    pub values: &\'static [(&\'static str, Modes)],',
        '    pub other: OtherValue,',
        '}',
        '',
        '/// Sorted by name.',
        'pub(crate) static BOOLEAN: &[Boolean] = &[',
    ]
    for name in sorted(BOOLEAN):
        on, off = BOOLEAN[name]
        out += [
            '    Boolean {',
            f'        name: {rust_str(name)},',
            f'        on: {rust_modes(on)},',
            f'        off: {rust_modes(off)},',
            '    },',
        ]
    out += ['];', '', '/// Sorted by prefix (without its `=`).', 'pub(crate) static VALUED: &[Valued] = &[']
    for prefix in sorted(VALUED, key=lambda p: p[:-1]):
        values, other = VALUED[prefix]
        out += ['    Valued {', f'        prefix: {rust_str(prefix)},', '        values: &[']
        for value in sorted(values):
            m = values[value]
            out.append(f'            ({rust_str(value)}, Modes {{ x86_64: {rust_verdict(m["64"])}, '
                       f'ia32: {rust_verdict(m["32"])} }}),')
        other_rust = 'OtherValue::Invalid' if other == 'invalid' else f'OtherValue::Checked({rust_str(other)})'
        out += ['        ],', f'        other: {other_rust},', '    },']
    out += ['];', '', '/// An option GCC 16.2 keeps as another name for an option: its warning',
            '/// and what it stands for. Sorted by option.',
            'pub(crate) static ALIASES: &[(&str, &str, &str)] = &[']
    for option in sorted(ALIASES):
        warning, replacement = ALIASES[option]
        out.append(f'    ({rust_str(option)}, {rust_str(warning)}, {rust_str(replacement)}),')
    out += ['];', '', '/// The same for a prefix: `-mstore-max=V` is `-mmove-max=V`.',
            'pub(crate) static PREFIX_ALIASES: &[(&str, &str, &str)] = &[']
    for prefix in sorted(PREFIX_ALIASES):
        warning, replacement = PREFIX_ALIASES[prefix]
        out.append(f'    ({rust_str(prefix)}, {rust_str(warning)}, {rust_str(replacement)}),')
    out += ['];', '', '/// `-malign-NAME=N`: obsolete in GCC 16.2 for `-falign-NAME=2^N`.',
            'pub(crate) static OBSOLETE_ALIGN: &[&str] = &[' + ', '.join(rust_str(n) for n in OBSOLETE_ALIGN) + '];',
            '']
    return '\n'.join(out)


def rustfmt(text: str) -> str:
    proc = subprocess.run(['rustfmt', '--edition', '2024', '--emit', 'stdout'], input=text,
                          capture_output=True, text=True, env=ENV)
    if proc.returncode != 0:
        sys.exit(f'rustfmt failed:\n{proc.stderr}')
    return proc.stdout


# --------------------------------------------------------------------------- rendering: document
WORD = {'state': 'accepted', 'hint': 'hint', 'ignored': 'ignored, with GCC\'s warning',
        'effect': 'implemented', 'depends': 'depends', 'refused': '**refused**', 'invalid': 'error, as in GCC'}


def doc_verdict(verdict) -> str:
    kind, text = verdict
    text = text.replace('|', '\\|')
    if kind in ('ignored', 'invalid'):
        return f'{WORD[kind]}: "{text}"'
    return f'{WORD[kind]}: {text}'


def doc_cell(m) -> str:
    if m is None:
        return 'not an option of GCC 16.2'
    if m['64'] == m['32']:
        return doc_verdict(m['64'])
    return f'x86-64: {doc_verdict(m["64"])}<br>IA-32: {doc_verdict(m["32"])}'


def census() -> dict[str, int]:
    count = dict.fromkeys(VERDICTS, 0)
    for on, off in BOOLEAN.values():
        for m in (on, off):
            if m is not None:
                for mode in ('64', '32'):
                    count[m[mode][0]] += 1
    for values, _ in VALUED.values():
        for m in values.values():
            for mode in ('64', '32'):
                count[m[mode][0]] += 1
    return count


def render_doc() -> str:
    count = census()
    out = [
        '<!-- Generated by scripts/gen_x86_mflag_policy.py --write; do not edit by hand. -->',
        '# x86 machine options without code generation of their own',
        '',
        'GCC 16.2 has 666 spellings of x86 `-m` options (`gcc --completion=-m`). The driver has an arm for',
        'the ones it implements: the ISA options and `-march=`/`-mtune=` (docs/x86-isa-permissions-and-simd-abi.md),',
        'the code models, the stack-protector guard, the retpoline and return thunks, `-mharden-sls=`,',
        '`-mregparm=`, `-mpreferred-stack-boundary=`, the `-pg` family, `-mno-red-zone`, `-mno-80387`,',
        '`-mgeneral-regs-only` and so on. This table is the rest: every spelling none of those arms takes,',
        'with what the driver does with it in each mode and why.',
        '',
        'The words:',
        '',
        '* **accepted**: in silence. What is asked is what the compiler does, with or without the option.',
        '* **hint**: accepted with a warning that nothing acts on it. The option chooses among ways of',
        '  generating equivalent code. `LCCC_STRICT_OPTIONS=1` makes it an error, so that a capability probe',
        '  can tell it from an option that is implemented.',
        '* **ignored, with GCC\'s warning**: GCC 16.2 accepts the option, warns and does nothing; so does lccc,',
        '  in the same words.',
        '* **implemented**: the option has its effect (an arm of the driver; listed here so that the table is',
        '  the whole list).',
        '* **depends**: decided by the rest of the command line; the rule is given below the table.',
        '* **refused**: an error that says why. Accepting the option would produce something other than what',
        '  was asked for: another layout or calling convention, a mitigation that is not there, an object',
        '  without the record it should carry.',
        '* **error, as in GCC**: GCC 16.2 refuses the spelling in that mode; lccc refuses it in GCC\'s words.',
        '',
        'An `-mno-NAME` whose name GCC 16.2 does not have is accepted as before (nothing by that name exists',
        'here to switch off) and refused under `LCCC_STRICT_MFLAGS`; an `-mNAME` GCC does not have is an error.',
        '',
        'Verdicts in the table, counted per spelling and mode: '
        + ', '.join(f'{count[k]} {k}' for k in VERDICTS if count[k]) + '.',
        '',
        '`scripts/gen_x86_mflag_policy.py --check --lccc BIN --gcc gcc-16` holds every row to the compiler',
        'and the whole of GCC\'s list to the table.',
        '',
        '## `-mNAME` and `-mno-NAME`',
        '',
        '| name | `-mNAME` | `-mno-NAME` |',
        '|---|---|---|',
    ]
    for name in sorted(BOOLEAN):
        on, off = BOOLEAN[name]
        out.append(f'| `{name}` | {doc_cell(on)} | {doc_cell(off)} |')
    out += ['', '### The rows that depend on the command line', '']
    for name in sorted(DEPENDS):
        out.append(f'* `{name}`: {DEPENDS[name]}.')
    out += ['', '## `-mNAME=VALUE`', '', '| option | value | |', '|---|---|---|']
    for prefix in sorted(VALUED):
        values, other = VALUED[prefix]
        for value in sorted(values):
            out.append(f'| `{prefix}` | `{value}` | {doc_cell(values[value])} |')
        if other == 'invalid':
            out.append(f'| `{prefix}` | any other | error, as in GCC: "unrecognized argument in option" |')
        else:
            out.append(f'| `{prefix}` | {"any" if not values else "any other"} | {CHECKERS[other]} |')
    out += ['', '## Other names for options', '',
            'GCC 16.2 keeps these as names for another option and warns; lccc prints the same warning and reads',
            'the other option in their place.',
            '', '| option | stands for | warning |', '|---|---|---|']
    for option in sorted(ALIASES):
        warning, replacement = ALIASES[option]
        out.append(f'| `{option}` | `{replacement}` | "{warning}" |')
    for prefix in sorted(PREFIX_ALIASES):
        warning, replacement = PREFIX_ALIASES[prefix]
        out.append(f'| `{prefix}V` | `{replacement}V` | "{warning}" |')
    for name in OBSOLETE_ALIGN:
        out.append(f'| `-malign-{name}=N` | `-falign-{name}=2^N`, where no `-falign-{name}` is given; '
                   f'N from 0 to 16, 0 for the default | "\'-malign-{name}\' is obsolete, use '
                   f'\'-falign-{name}\'" |')
    return '\n'.join(out).rstrip() + '\n'


# --------------------------------------------------------------------------- measuring
SOURCE = 'int f(void) { return 0; }\n'


def run(cmd, env=None):
    proc = subprocess.run(cmd, capture_output=True, text=True, env={**ENV, **(env or {})})
    return proc.returncode, proc.stderr


def spellings():
    """Every (argument, mode, verdict) the table decides."""
    for name, (on, off) in sorted(BOOLEAN.items()):
        for arg, m in ((f'-m{name}', on), (f'-mno-{name}', off)):
            if m is not None:
                for mode in ('64', '32'):
                    yield arg, mode, m[mode]
    for prefix, (values, _) in sorted(VALUED.items()):
        for value, m in sorted(values.items()):
            for mode in ('64', '32'):
                yield prefix + value, mode, m[mode]


# Values for the checkers: (argument, mode or None for both, expected kind).
CHECKED_VALUES = [
    ('-mbranch-cost=0', None, 'hint'), ('-mbranch-cost=5', None, 'hint'),
    ('-mbranch-cost=6', None, 'invalid'), ('-mbranch-cost=x', None, 'invalid'),
    ('-mincoming-stack-boundary=4', None, 'state'), ('-mincoming-stack-boundary=5', None, 'hint'),
    ('-mincoming-stack-boundary=12', None, 'hint'), ('-mincoming-stack-boundary=3', '64', 'refused'),
    ('-mincoming-stack-boundary=3', '32', 'refused'), ('-mincoming-stack-boundary=2', '32', 'refused'),
    ('-mincoming-stack-boundary=2', '64', 'invalid'), ('-mincoming-stack-boundary=13', None, 'invalid'),
    ('-mincoming-stack-boundary=x', None, 'invalid'),
    ('-mlarge-data-threshold=65536', None, 'refused'), ('-mlarge-data-threshold=x', None, 'invalid'),
    ('-mfentry-name=foo', None, 'effect'), ('-mfentry-name=', None, 'invalid'),
    ('-mfentry-section=.sec', None, 'effect'),
    ('-mrecip=all', None, 'hint'), ('-mrecip=!sqrt,vec-div', None, 'hint'), ('-mrecip=bogus', None, 'invalid'),
    ('-mtune-ctrl=use_leave', None, 'hint'), ('-mmemcpy-strategy=libcall:-1:align', None, 'hint'),
    ('-mmemset-strategy=libcall:-1:align', None, 'hint'),
    ('-mapx-features=none', None, 'state'), ('-mapx-features=egpr', '64', 'refused'),
    ('-mapx-features=egpr', '32', 'invalid'), ('-mapx-features=bogus', None, 'invalid'),
    ('-mno-apx-features=egpr', None, 'state'),
    ('-main=x', None, 'ignored'), ('-mno-ain=x', None, 'ignored'),
    ('-mmove-max=64', None, 'invalid'), ('-masm=bogus', None, 'invalid'), ('-mfpmath=bogus', None, 'invalid'),
    ('-mveclibabi=x', None, 'invalid'), ('-mlam=u64', None, 'invalid'),
]

GENERIC = 'LCCC refuses to silently ignore'
HINT = re.compile(r"warning: '(.+)' is tolerated but not implemented")


def classify_lccc(lccc, arg, mode, src):
    """How `lccc` treats `arg`: (kind, text) in the table's words."""
    rc, err = run([lccc, f'-m{mode}', '-S', '-o', os.devnull, arg, src])
    lines = [line for line in err.splitlines() if line.strip()]
    if rc != 0:
        text = next((line for line in lines if 'error:' in line), lines[0] if lines else '')
        if GENERIC in text:
            return 'undecided', text
        match = re.search(r'unsupported machine option \S+: (.*)', text)
        if match:
            return 'refused', match.group(1)
        return 'invalid', re.sub(r'^\S*: error: ', '', text)
    if not lines:
        return 'state', ''
    if HINT.search(lines[0]):
        return 'hint', lines[0]
    return 'ignored', re.sub(r'^\S*: warning: ', '', lines[0])


def check_lccc(lccc, src) -> list[str]:
    problems = []

    def one(item):
        arg, mode, (kind, text) = item
        got, got_text = classify_lccc(lccc, arg, mode, src)
        want = 'state' if kind == 'effect' else kind
        if kind == 'depends':
            # Without the extension enabled, the option is refused and its
            # `-mno-` form is the state.
            want = 'state' if arg.startswith('-mno-') else 'refused'
        if got != want:
            return f'{arg} -m{mode}: the table says {kind}, lccc: {got} ({got_text[:100]!r})'
        if kind in ('refused', 'ignored', 'invalid') and text not in got_text:
            return f'{arg} -m{mode}: {kind} with other words: {got_text[:140]!r}, expected {text[:100]!r}'
        # A row must hold under the strict policy as well: what is not
        # implemented is an error there, what is a state is not.
        rc, _ = run([lccc, f'-m{mode}', '-S', '-o', os.devnull, arg, src],
                    {'LCCC_STRICT_OPTIONS': '1', 'LCCC_STRICT_MFLAGS': '1'})
        if (rc == 0) != (want == 'state'):
            return f'{arg} -m{mode}: under the strict policy exit {rc}, the table says {kind}'
        return None

    items = list(spellings())
    for arg, mode, kind in CHECKED_VALUES:
        for m in ((mode,) if mode else ('64', '32')):
            items.append((arg, m, (kind, '')))
    with ThreadPoolExecutor(os.cpu_count() or 1) as pool:
        problems += [p for p in pool.map(one, items) if p]

    # Aliases print GCC's warning and act as what they stand for.
    for option, (warning, replacement) in sorted(ALIASES.items()):
        for mode in ('64', '32'):
            rc, err = run([lccc, f'-m{mode}', '-S', '-o', os.devnull, option, src])
            rc2, err2 = run([lccc, f'-m{mode}', '-S', '-o', os.devnull, replacement, src])
            if warning not in err:
                problems.append(f'{option} -m{mode}: GCC\'s warning is missing: {err.strip()[:120]!r}')
            if rc != rc2:
                problems.append(f'{option} -m{mode}: exit {rc}, its replacement {replacement}: exit {rc2}')
    return problems


def gcc_vocabulary(gcc) -> list[str]:
    proc = subprocess.run([gcc, '--completion=-m'], capture_output=True, text=True, env=ENV)
    return sorted({line.strip() for line in proc.stdout.splitlines() if line.strip()})


# One value for each option whose completion ends in `=`.
FREE_VALUE = {
    '-malign-functions=': '4', '-malign-jumps=': '4', '-malign-loops=': '4', '-mbranch-cost=': '3',
    '-mcpu=': 'haswell', '-mfentry-name=': 'foo', '-mfentry-section=': '.sec',
    '-mincoming-stack-boundary=': '4', '-mlarge-data-threshold=': '65536',
    '-mmemcpy-strategy=': 'libcall:-1:align', '-mmemset-strategy=': 'libcall:-1:align',
    '-mpreferred-stack-boundary=': '4', '-mrecip=': 'all', '-mregparm=': '3',
    '-mstack-protector-guard-offset=': '40', '-mstack-protector-guard-reg=': 'fs',
    '-mstack-protector-guard-symbol=': 'sym', '-mtune-ctrl=': 'use_leave',
}


def check_gcc(lccc, gcc, src) -> list[str]:
    version = subprocess.run([gcc, '-dumpfullversion'], capture_output=True, text=True, env=ENV).stdout.strip()
    if version != EXPECTED_GCC:
        return [f'{gcc} is {version}; the table is measured against {EXPECTED_GCC}']
    problems = []

    def one(spelling):
        arg = spelling + FREE_VALUE[spelling] if spelling in FREE_VALUE else spelling
        found = []
        for mode in ('64', '32'):
            g_rc, g_err = run([gcc, f'-m{mode}', '-S', '-o', os.devnull, arg, src])
            l_kind, l_text = classify_lccc(lccc, arg, mode, src)
            if g_rc == 0 and l_kind == 'undecided':
                found.append(f'{arg} -m{mode}: GCC accepts it and the driver has no decision for it')
            if g_rc != 0 and l_kind in ('state', 'hint', 'ignored'):
                found.append(f'{arg} -m{mode}: GCC refuses it ({g_err.strip().splitlines()[0][:90]!r}) '
                             f'and lccc accepts it')
            if l_kind == 'ignored':
                g_first = re.sub(r'^\S*: warning: ', '', (g_err.strip().splitlines() or [''])[0])
                if g_first != l_text:
                    found.append(f'{arg} -m{mode}: warning {l_text!r}, GCC\'s is {g_first!r}')
        return found

    with ThreadPoolExecutor(os.cpu_count() or 1) as pool:
        for found in pool.map(one, gcc_vocabulary(gcc)):
            problems += found

    # The table's own premises: a `state` or `hint` row is a spelling GCC
    # accepts; an `invalid` row is one GCC refuses, in the words quoted.
    def premise(item):
        arg, mode, (kind, text) = item
        rc, err = run([gcc, f'-m{mode}', '-S', '-o', os.devnull, arg, src])
        if kind == 'invalid':
            if rc == 0:
                return f'{arg} -m{mode}: the table says GCC refuses it; GCC accepts it'
            if text not in err:
                return f'{arg} -m{mode}: GCC\'s words are {err.strip().splitlines()[0][:140]!r}, the table has {text!r}'
        elif rc != 0:
            return f'{arg} -m{mode}: the table decides it ({kind}) and GCC refuses it: {err.strip().splitlines()[0][:100]!r}'
        elif kind == 'ignored' and text not in err:
            return f'{arg} -m{mode}: GCC\'s warning is {err.strip()[:140]!r}, the table has {text!r}'
        return None

    with ThreadPoolExecutor(os.cpu_count() or 1) as pool:
        problems += [p for p in pool.map(premise, list(spellings())) if p]
    for option, (warning, _) in sorted(ALIASES.items()):
        rc, err = run([gcc, '-S', '-o', os.devnull, option, src])
        if warning not in err:
            problems.append(f'{option}: GCC\'s warning is {err.strip()[:140]!r}, the table has {warning!r}')
    return problems


# --------------------------------------------------------------------------- main
def stale(path: Path, text: str) -> bool:
    return not path.exists() or path.read_text() != text


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    parser.add_argument('--write', action='store_true', help='regenerate the Rust table and the document')
    parser.add_argument('--check', action='store_true', help='fail when a generated file is stale')
    parser.add_argument('--lccc', help='hold every row to this compiler')
    parser.add_argument('--gcc', help='hold the table to this GCC (16.2.0)')
    args = parser.parse_args(argv)
    if not (args.write or args.check):
        parser.error('one of --write, --check')
    if args.gcc and not args.lccc:
        parser.error('--gcc needs --lccc')

    rust, doc = rustfmt(render_rust()), render_doc()
    if args.write:
        RUST.write_text(rust)
        DOC.write_text(doc)
        print(f'wrote {RUST.relative_to(ROOT)} and {DOC.relative_to(ROOT)}')
    problems = []
    if args.check:
        for path, text in ((RUST, rust), (DOC, doc)):
            if stale(path, text):
                problems.append(f'{path.relative_to(ROOT)} is stale: run scripts/gen_x86_mflag_policy.py --write')
    if args.lccc:
        with tempfile.TemporaryDirectory() as tmp:
            src = os.path.join(tmp, 'c.c')
            Path(src).write_text(SOURCE)
            problems += check_lccc(args.lccc, src)
            if args.gcc:
                problems += check_gcc(args.lccc, args.gcc, src)
    count = census()
    rows = sum(count.values())
    if problems:
        for problem in problems:
            print(f'gen_x86_mflag_policy: {problem}')
        print(f'gen_x86_mflag_policy: FAIL ({len(problems)} problem(s), {rows} verdicts)')
        return 1
    measured = ' against ' + ' and '.join(x for x in (args.lccc and 'lccc', args.gcc and 'GCC') if x) \
        if args.lccc else ''
    print(f'gen_x86_mflag_policy: PASS ({rows} verdicts over {len(BOOLEAN)} names and {len(VALUED)} '
          f'valued options, {len(ALIASES) + len(PREFIX_ALIASES) + len(OBSOLETE_ALIGN)} aliases{measured})')
    return 0


if __name__ == '__main__':
    sys.exit(main())
