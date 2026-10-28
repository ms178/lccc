#!/usr/bin/env python3
"""Generate src/backend/x86/arch_table.rs from GCC 16.2.

Two tables, neither transcribed by hand:

* Every CPU name GCC accepts in ``-march=`` and in
  ``__attribute__((target("arch=NAME")))`` (the two lists are verified to
  be the same), with every ``X86Feature`` and ``X86ExtraFeature`` it
  enables, read from GCC's predefined macros (``gcc -march=NAME -dM -E``)
  in the CPU's widest mode, and CMOV, which has no macro, from GCC's code
  generation.
* Every ISA option of ``target("...")``: the features ``NAME`` enables
  and disables and the features ``no-NAME`` disables, each with everything
  GCC's implication graph attaches to it. Measured with ``#pragma GCC
  target``, which applies the attribute's option handler and updates the
  predefined macros in place: ``set`` from a base with no ISA at all,
  ``clear`` and ``unset`` from a base with every feature. CMPXCHG16B has
  no pragma-updated macro and is read from code generation (an inline
  ``cmpxchg16b``). GCC applies a target string's options in order, each
  with these fixed masks, so the table reproduces any option sequence.

Run without arguments to regenerate the file, or with ``--check`` to
re-measure and reject drift. ``GCC`` selects the oracle (default
``gcc-16``); it must be GCC 16.2, the version the table records.
"""
import difflib
import os
import re
import shutil
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUTPUT = ROOT / 'src/backend/x86/arch_table.rs'
GCC = os.environ.get('GCC', 'gcc-16')
# How many diff lines `--check` prints before truncating: enough to name the
# drifted rows, short enough to stay readable in a CI annotation.
DIFF_LIMIT = 60
EXPECTED_VERSION = '16.2.0'

# `X86Feature` variant -> the GCC macro that announces it. CMOV has no
# macro; it is read from code generation (a `cmov` in `max()`).
FEATURE_MACROS = [
    ('Fxsr', '__FXSR__'),
    ('Mmx', '__MMX__'),
    ('Sse', '__SSE__'),
    ('Sse2', '__SSE2__'),
    ('Sse3', '__SSE3__'),
    ('Ssse3', '__SSSE3__'),
    ('Sse41', '__SSE4_1__'),
    ('Sse42', '__SSE4_2__'),
    ('Popcnt', '__POPCNT__'),
    ('Cx16', '__GCC_HAVE_SYNC_COMPARE_AND_SWAP_16'),
    ('Avx', '__AVX__'),
    ('Avx2', '__AVX2__'),
    ('Fma', '__FMA__'),
    ('F16c', '__F16C__'),
    ('Bmi', '__BMI__'),
    ('Bmi2', '__BMI2__'),
    ('Lzcnt', '__LZCNT__'),
    ('Movbe', '__MOVBE__'),
    ('Aes', '__AES__'),
    ('Pclmul', '__PCLMUL__'),
    ('Rdrnd', '__RDRND__'),
    ('Vaes', '__VAES__'),
    ('Vpclmulqdq', '__VPCLMULQDQ__'),
    ('AvxVnni', '__AVXVNNI__'),
    ('AvxVnniInt8', '__AVXVNNIINT8__'),
    ('AvxVnniInt16', '__AVXVNNIINT16__'),
    ('Gfni', '__GFNI__'),
    ('Avx512F', '__AVX512F__'),
    ('Avx512Bw', '__AVX512BW__'),
    ('Avx512Dq', '__AVX512DQ__'),
    ('Avx512Vl', '__AVX512VL__'),
    ('Avx512Vnni', '__AVX512VNNI__'),
    ('Avx512BitAlg', '__AVX512BITALG__'),
    ('Avx512Vpopcntdq', '__AVX512VPOPCNTDQ__'),
    ('Avx512Cd', '__AVX512CD__'),
    ('Avx512Ifma', '__AVX512IFMA__'),
    ('Avx512Vbmi', '__AVX512VBMI__'),
    ('Avx512Vbmi2', '__AVX512VBMI2__'),
    ('Avx512Bf16', '__AVX512BF16__'),
    ('Avx512Fp16', '__AVX512FP16__'),
    ('Avx512Bmm', '__AVX512BMM__'),
    ('Avx512Vp2intersect', '__AVX512VP2INTERSECT__'),
]
# `X86Feature` variant -> its `target("...")` option name (GCC's spelling).
OPTION_NAMES = {
    'Fxsr': 'fxsr', 'Mmx': 'mmx', 'Sse': 'sse', 'Sse2': 'sse2', 'Sse3': 'sse3',
    'Ssse3': 'ssse3', 'Sse41': 'sse4.1', 'Sse42': 'sse4.2', 'Popcnt': 'popcnt',
    'Cx16': 'cx16', 'Avx': 'avx', 'Avx2': 'avx2', 'Fma': 'fma', 'F16c': 'f16c',
    'Bmi': 'bmi', 'Bmi2': 'bmi2', 'Lzcnt': 'lzcnt', 'Movbe': 'movbe', 'Aes': 'aes',
    'Pclmul': 'pclmul', 'Rdrnd': 'rdrnd', 'Vaes': 'vaes', 'Vpclmulqdq': 'vpclmulqdq',
    'AvxVnni': 'avxvnni', 'AvxVnniInt8': 'avxvnniint8', 'AvxVnniInt16': 'avxvnniint16',
    'Gfni': 'gfni', 'Avx512F': 'avx512f', 'Avx512Bw': 'avx512bw', 'Avx512Dq': 'avx512dq',
    'Avx512Vl': 'avx512vl', 'Avx512Vnni': 'avx512vnni', 'Avx512BitAlg': 'avx512bitalg',
    'Avx512Vpopcntdq': 'avx512vpopcntdq', 'Avx512Cd': 'avx512cd',
    'Avx512Ifma': 'avx512ifma', 'Avx512Vbmi': 'avx512vbmi', 'Avx512Vbmi2': 'avx512vbmi2',
    'Avx512Bf16': 'avx512bf16', 'Avx512Fp16': 'avx512fp16', 'Avx512Bmm': 'avx512bmm',
    'Avx512Vp2intersect': 'avx512vp2intersect',
}
# Option state that is not an `X86Feature`, kept in the high bits of the
# same mask: ABM (`target("abm")`) is GCC's AMD spelling that grants POPCNT
# and, through the option override, LZCNT. (variant, macro, option, bit)
PSEUDO = [('Abm', '__ABM__', 'abm', 63)]
# Options that are not one feature's name: `sse4` (SSE4.2 when granted,
# SSE4.1 and up when denied) and `general-regs-only` (clears the vector
# register classes, keeps the integer extensions).
EXTRA_OPTIONS = ['sse4', 'general-regs-only']
# AVX-512ER/PF (Knights Landing) left GCC in 15, so GCC 16 rejects their
# options and no CPU row has them. Their columns follow the rule GCC 14.2
# implements (measured with `gcc-14` and `#pragma GCC target` for every
# option of this table): each grants AVX-512F's closure plus itself, its
# denial removes only itself, and every option that removes AVX-512F
# removes both. Frozen hardware, so the rule cannot drift.
LEGACY_AVX512 = ['Avx512Er', 'Avx512Pf']
LEGACY_OPTION_NAMES = {'Avx512Er': 'avx512er', 'Avx512Pf': 'avx512pf'}
# Driver-level extensions lccc does not generate code for but announces
# with GCC's macro (`X86ExtraFeature`), so `-march=` can grant them from
# the same measurement instead of a hand list.
EXTRA_MACROS = [
    ('Prfchw', '__PRFCHW__'),
    ('AvxIfma', '__AVXIFMA__'),
    ('AvxNeConvert', '__AVXNECONVERT__'),
    ('Cmpccxadd', '__CMPCCXADD__'),
    ('Sha512', '__SHA512__'),
    ('Sm3', '__SM3__'),
    ('Sm4', '__SM4__'),
    ('Movrs', '__MOVRS__'),
    ('UserMsr', '__USER_MSR__'),
    ('AmxTile', '__AMX_TILE__'),
    ('AmxInt8', '__AMX_INT8__'),
    ('AmxBf16', '__AMX_BF16__'),
    ('Avx10_1', '__AVX10_1__'),
    ('Avx10_2', '__AVX10_2__'),
    ('ApxF', '__APX_F__'),
]
# GCC's option for each macro-only extension.
EXTRA_OPTION_NAMES = {
    'Prfchw': 'prfchw', 'AvxIfma': 'avxifma', 'AvxNeConvert': 'avxneconvert',
    'Cmpccxadd': 'cmpccxadd', 'Sha512': 'sha512', 'Sm3': 'sm3', 'Sm4': 'sm4',
    'Movrs': 'movrs', 'UserMsr': 'usermsr', 'AmxTile': 'amx-tile',
    'AmxInt8': 'amx-int8', 'AmxBf16': 'amx-bf16', 'Avx10_1': 'avx10.1',
    'Avx10_2': 'avx10.2', 'ApxF': 'apxf',
}
CMOV_PROBE = 'int f(int a, int b) { return a < b ? b : a; }\n'


ENV = {**os.environ, 'LC_ALL': 'C', 'LANG': 'C'}


def run(args, stdin=''):
    return subprocess.run([GCC, *args], input=stdin, capture_output=True, text=True, env=ENV)


def gcc_version():
    proc = run(['-dumpfullversion'])
    if proc.returncode != 0:
        sys.exit(f'{GCC}: cannot run the oracle: {proc.stderr.strip()}')
    return proc.stdout.strip()


def valid_march_names(mode):
    """GCC's own list, from the note it prints for an invalid -march."""
    proc = run([mode, '-march=lccc-invalid-cpu', '-x', 'c', '-c', '-', '-o', os.devnull], 'int x;')
    match = re.search(r"valid arguments to '-march=' switch are: ([^\n]*)", proc.stderr)
    if not match:
        sys.exit(f'{GCC} {mode}: no list of valid -march= arguments in:\n{proc.stderr}')
    return [name for name in match.group(1).split() if name != 'native']


def valid_tune_names(mode):
    """GCC's `-mtune=` list for `mode` (without `native`), each name
    verified by compiling with it."""
    proc = run([mode, '-mtune=lccc-invalid-cpu', '-x', 'c', '-c', '-', '-o', os.devnull], 'int x;')
    match = re.search(r"valid arguments to '-mtune=' switch are: ([^\n]*)", proc.stderr)
    if not match:
        sys.exit(f'{GCC} {mode}: no list of valid -mtune= arguments in:\n{proc.stderr}')
    names = {name for name in match.group(1).split() if name != 'native'}
    for name in sorted(names):
        if tune_rejected(mode, name):
            sys.exit(f'{name}: listed for {mode} -mtune= but rejected by GCC')
    return names


def tune_rejected(mode, name):
    proc = run([mode, f'-mtune={name}', '-x', 'c', '-c', '-', '-o', os.devnull], 'int x;')
    return proc.returncode != 0


def measure_tune_vocabulary(rows):
    """(tune-only names, `-march=`-only names). GCC's `-mtune=` vocabulary
    is the `-march=` CPUs minus the `-march=`-only names plus the tune-only
    ones, a CPU being valid for `-m64` exactly when its `-march=` is; the
    rule is checked against GCC in both modes."""
    arch = {name for name, *_ in rows}
    tune32 = valid_tune_names('-m32')
    tune64 = valid_tune_names('-m64')
    tune_only = sorted(tune32 - arch)
    march_only = sorted(arch - tune32)
    derived64 = {name for name, x86_64, *_ in rows if x86_64} - set(march_only) | set(tune_only)
    if derived64 != tune64:
        sys.exit(f'-m64 -mtune= vocabulary breaks the row rule: {sorted(derived64 ^ tune64)}')
    for name in sorted(tune32 - tune64):
        if not tune_rejected('-m64', name):
            sys.exit(f'{name}: missing from the -m64 -mtune= list but accepted on -m64')
    return tune_only, march_only


def macros(mode, name, *flags):
    proc = run([mode, f'-march={name}', *flags, '-dM', '-E', '-x', 'c', '-'])
    if proc.returncode != 0:
        return None
    return {line.split()[1] for line in proc.stdout.splitlines() if line.startswith('#define ')}


def feature_variants(defined):
    """The `X86Feature` variants and option pseudo-bits a macro dump shows."""
    return [variant for variant, macro in FEATURE_MACROS + [(v, m) for v, m, _, _ in PSEUDO] if macro in defined]


def attribute_accepts(mode, name):
    src = f'__attribute__((target("arch={name}"))) int f(void) {{ return 0; }}\n'
    return run([mode, '-x', 'c', '-c', '-', '-o', os.devnull], src).returncode == 0


def has_cmov(mode, name):
    proc = run([mode, f'-march={name}', '-O2', '-S', '-x', 'c', '-', '-o', '-'], CMOV_PROBE)
    if proc.returncode != 0:
        sys.exit(f'{name}: the CMOV probe does not compile on {mode}:\n{proc.stderr}')
    return 'cmov' in proc.stdout


CPU_MACRO = re.compile(r'__[a-z][a-z0-9_]*')


def lowercase_macros(args):
    proc = run([*args, '-dM', '-E', '-x', 'c', '-'])
    if proc.returncode != 0:
        return None
    return {m for m in re.findall(r'^#define (\S+)', proc.stdout, re.M) if CPU_MACRO.fullmatch(m)}


def is_sse_companion(macro):
    """`__athlon_sse__`, `__tune_athlon_sse__`: GCC defines these only while
    SSE is enabled (`-march=athlon -msse` has them, `-march=athlon-4
    -mno-sse` does not); lccc applies the same condition."""
    return macro.endswith('_sse__')


def cpu_macros(mode, name, defined, base):
    """GCC's CPU-name macros, verbatim, as (-march= macros, -mtune= macros).

    -march= macros: `__haswell`, `__haswell__` and aliases such as
    `__core_avx2__` (not always in pairs: athlon-4 has `__athlon_sse__`
    alone). -mtune= macros: `__tune_haswell__`, ..., measured from
    `-mtune=NAME` (the default ISA has SSE, so SSE companions appear); []
    where GCC rejects the name for -mtune= (x86-64-v2..v4). `base` holds
    the target/OS macros every CPU of the mode shares.

    Also verifies the two rules lccc applies when no -mtune= is given: the
    -march= CPU tunes, except that `x86-64*` tunes generic (no macros)."""
    own = (defined & {m for m in defined if CPU_MACRO.fullmatch(m)}) - base
    march_tune = sorted(m for m in own if m.startswith('__tune_'))
    arch = sorted(own - set(march_tune))
    tuned = lowercase_macros([mode, f'-mtune={name}']) or set()
    tune = sorted(m for m in tuned if m.startswith('__tune_'))
    sse = '__SSE__' in defined
    implied = [] if name.startswith('x86-64') else [m for m in tune if sse or not is_sse_companion(m)]
    if march_tune != implied:
        sys.exit(f'{name}: -march= tunes {march_tune}, the lccc rule says {implied}')
    if any(is_sse_companion(m) for m in arch) and not sse:
        sys.exit(f'{name}: SSE companion macro without SSE: {arch}')
    return arch, tune


def measure():
    """(name, x86_64, avx512f, [X86Feature variant names], [arch macro
    names], [tune macro names]) for every CPU."""
    version = gcc_version()
    if version != EXPECTED_VERSION:
        sys.exit(f'{GCC} is {version}; the table records GCC {EXPECTED_VERSION}')
    names32 = valid_march_names('-m32')
    names64 = set(valid_march_names('-m64'))
    if not names64 <= set(names32):
        sys.exit(f'-m64 names missing from the -m32 list: {sorted(names64 - set(names32))}')
    rows = []
    for name in sorted(set(names32)):
        if macros('-m32', name) is None or not attribute_accepts('-m32', name):
            sys.exit(f'{name}: listed for -march= but rejected by GCC on -m32')
        x86_64 = name in names64
        # A CPU's features are those of its widest mode (CMPXCHG16B is only
        # announced on -m64; the SSE levels are the same in both).
        mode = '-m64' if x86_64 else '-m32'
        defined = macros(mode, name)
        if defined is None or not attribute_accepts(mode, name):
            sys.exit(f'{name}: listed for {mode} -march= but rejected on {mode}')
        features = (['Cmov'] if has_cmov(mode, name) else []) + feature_variants(defined)
        extras = [variant for variant, macro in EXTRA_MACROS if macro in defined]
        rows.append((name, x86_64, features, extras, mode, defined))
    base = {}
    for *_, mode, defined in rows:
        lower = {m for m in defined if CPU_MACRO.fullmatch(m)}
        base[mode] = base.get(mode, lower) & lower
    return [
        (name, x86_64, features, extras, *cpu_macros(mode, name, defined, base[mode]))
        for name, x86_64, features, extras, mode, defined in rows
    ]


# One `#pragma GCC target` probe: each feature macro becomes a global the
# assembly names, and CMPXCHG16B shows as an inline instruction.
CX16_PROBE = '''
__int128 lccc_cx16_v;
__int128 lccc_cx16(__int128 o, __int128 n) { return __sync_val_compare_and_swap(&lccc_cx16_v, o, n); }
'''


NOT_NEGATABLE = 'does not allow a negated form'


def option_state(base, option, may_refuse=False):
    src = f'#pragma GCC target("{option}")\n' if option else ''
    for variant, macro in FEATURE_MACROS + [(v, m) for v, m, _, _ in PSEUDO]:
        if variant != 'Cx16':
            src += f'#ifdef {macro}\nint lccc_have_{variant};\n#endif\n'
    src += CX16_PROBE
    proc = run(['-m64', *base, '-O2', '-S', '-x', 'c', '-', '-o', '-'], src)
    if proc.returncode != 0:
        if may_refuse and NOT_NEGATABLE in proc.stderr:
            return None
        sys.exit(f'target("{option}") does not compile:\n{proc.stderr}')
    tracked = [v for v, _ in FEATURE_MACROS] + [v for v, *_ in PSEUDO]
    state = {v for v in tracked if re.search(rf'\blccc_have_{v}\b', proc.stdout)}
    if 'cmpxchg16b' in proc.stdout:
        state.add('Cx16')
    return state


def macro_state(base, option):
    """The feature macros after `#pragma GCC target(option)` (no CMPXCHG16B)."""
    src = f'#pragma GCC target("{option}")\n' if option else ''
    for variant, macro in FEATURE_MACROS + [(v, m) for v, m, _, _ in PSEUDO]:
        if variant != 'Cx16':
            src += f'#ifdef {macro}\n@{variant}\n#endif\n'
    proc = run([*base, '-E', '-P', '-x', 'c', '-'], src)
    if proc.returncode != 0:
        sys.exit(f'target("{option}") does not preprocess:\n{proc.stderr}')
    return {line[1:] for line in proc.stdout.split() if line.startswith('@')}


def measure_implied(options):
    """GCC's option-override implications, as (trigger, implied) pairs:
    after the options, `implied` is enabled wherever `trigger` is, unless
    an option named `implied` explicitly (`TARGET_SSE4_2` enables POPCNT).
    Measured on a 32-bit `-march=i386` base, where nothing is enabled:
    each grant once with nothing explicit and once with every feature
    explicitly denied first; the difference is what the override added."""
    free = ['-m32', '-march=i386']
    pseudo = [o for _, _, o, _ in PSEUDO]
    names = [o[0] for o in options if o[0] in [*OPTION_NAMES.values(), *pseudo]]
    denied = free + [f'-mno-{n}' for n in names if n != 'cx16']
    if macro_state(free, None) or macro_state(denied, None):
        sys.exit('the 32-bit i386 base is not empty')
    added = {}
    for name in names:
        if name == 'cx16':
            continue
        extra = macro_state(free, name) - macro_state(denied, name)
        for implied in extra:
            added.setdefault(implied, set()).add(name)
    sets = {o[0]: o[1] for o in options}
    rules = []
    for implied, granting in sorted(added.items()):
        common = set.intersection(*(sets[n] for n in granting))
        trigger = [t for t in common if {n for n in names if n != 'cx16' and t in sets[n]} == granting]
        if len(trigger) != 1:
            sys.exit(f'{implied}: no single trigger explains {sorted(granting)} ({trigger})')
        rules.append((trigger[0], implied))
    return sorted(rules, key=lambda r: (r[1] in PSEUDO_CONST, ORDER.index(r[1]) if r[1] in ORDER else 0, r[0]))


def measure_extra_requires():
    """{extension: (features, extensions) whose `-mno-` removes it}, each
    measured on `-march=x86-64 -mEXTENSION`: AVXIFMA leaves with AVX2,
    SHA512 only with AVX (`-mno-avx2` keeps it), CMPCCXADD with nothing,
    AMX-INT8 with AMX-TILE. (The extension's own option sets the same.)"""
    feature_options = [(OPTION_NAMES[v], v) for v, _ in FEATURE_MACROS if v not in ('Fxsr', 'Mmx')]
    extra_options = [(EXTRA_OPTION_NAMES[v], v) for v, _ in EXTRA_MACROS]

    def requires(extra):
        variant, macro = extra
        base = ['-x', 'c', '-dM', '-E', '-', '-march=x86-64', f'-m{EXTRA_OPTION_NAMES[variant]}']
        if not re.search(rf'^#define {macro} ', run(base).stdout, re.M):
            sys.exit(f'-m{EXTRA_OPTION_NAMES[variant]} does not define {macro}')

        def removed_by(options):
            return [v for option, v in options if v != variant
                    and not re.search(rf'^#define {macro} ', run(base + [f'-mno-{option}']).stdout, re.M)]
        return variant, (removed_by(feature_options), removed_by(extra_options))

    with ThreadPoolExecutor(os.cpu_count() or 1) as pool:
        return dict(pool.map(requires, EXTRA_MACROS))


def measure_options():
    """(option, set, clear, unset) for every `target("...")` ISA option;
    `unset` is None where GCC refuses the `no-` form."""
    pseudo = [o for _, _, o, _ in PSEUDO]
    names = [OPTION_NAMES[v] for v, _ in FEATURE_MACROS] + pseudo + EXTRA_OPTIONS
    # Every option denied explicitly, so the override implications cannot
    # add to what a grant sets (they are measured apart: measure_implied).
    empty = ['-march=x86-64'] + [f'-mno-{n}' for n in names if n not in EXTRA_OPTIONS]
    full = ['-march=x86-64'] + [f'-m{n}' for n in names if n not in EXTRA_OPTIONS]
    nothing, everything = option_state(empty, None), option_state(full, None)
    tracked = {v for v, _ in FEATURE_MACROS} | {v for v, *_ in PSEUDO}
    if nothing:
        sys.exit(f'the empty base still has {sorted(nothing)}')
    if everything != tracked:
        sys.exit(f'the full base lacks {sorted(tracked - everything)}')
    options = []
    for name in names:
        denied = option_state(full, f'no-{name}', may_refuse=name in EXTRA_OPTIONS)
        unset = None if denied is None else everything - denied
        options.append([
            name,
            option_state(empty, name),
            everything - option_state(full, name),
            unset,
        ])
    for option in options:
        for column in (2, 3):
            if option[column] is not None and 'Avx512F' in option[column]:
                option[column] |= set(LEGACY_AVX512)
    f_set = next(o[1] for o in options if o[0] == 'avx512f')
    for variant in LEGACY_AVX512:
        options.append([LEGACY_OPTION_NAMES[variant], f_set | {variant}, set(), {variant}])
    return sorted(options)


def measure_abm_derived(rows):
    """{CPU: features it has only through ABM}. GCC grants a CPU's ABM-
    derived POPCNT and LZCNT only while ABM itself is not denied:
    `-march=amdfam10 -mno-abm` has neither, `-march=btver1 -mno-abm` keeps
    both (its CPU entry names them itself)."""
    derived = {}
    for name, x86_64, features, *_ in rows:
        if 'Abm' in features:
            mode = '-m64' if x86_64 else '-m32'
            lost = set(features) - set(feature_variants(macros(mode, name, '-mno-abm'))) - {'Abm', 'Cmov'}
            if lost:
                derived[name] = sorted(lost, key=ORDER.index)
    return derived


def verify_march_negation(rows, options, derived):
    """The driver's `-march=CPU -mno-N` is the CPU's features minus N's
    unset mask (and minus what the CPU has only through ABM for `-mno-abm`):
    check that model against GCC for every CPU, mode and negatable option."""
    skip = set(EXTRA_OPTIONS) | set(LEGACY_OPTION_NAMES.values())
    unset = {name: un for name, _, _, un in options if un is not None and name not in skip}
    jobs = []
    for name, x86_64, features, *_ in rows:
        for mode in ('-m64', '-m32') if x86_64 else ('-m32',):
            for option, removed in unset.items():
                lost = removed | (set(derived.get(name, ())) if option == 'abm' else set())
                expected = set(features) - lost - {'Cmov'}
                if mode == '-m32':
                    expected -= {'Cx16'}
                jobs.append((mode, name, option, expected))

    def deviation(job):
        mode, name, option, expected = job
        defined = macros(mode, name, f'-mno-{option}')
        if defined is None:
            return f'{mode} -march={name} -mno-{option}: rejected'
        got = set(feature_variants(defined)) - {'Cmov'}
        if mode == '-m32':
            got -= {'Cx16'}
        return None if got == expected else f'{mode} -march={name} -mno-{option}: {sorted(got ^ expected)}'

    with ThreadPoolExecutor(os.cpu_count() or 1) as pool:
        bad = [d for d in pool.map(deviation, jobs) if d]
    if bad:
        sys.exit('-march= with -mno- deviates from the row minus the unset mask:\n' + '\n'.join(bad))
    return len(jobs)


def group_by_value(mapping):
    """[(value, [keys])] in first-key order."""
    groups = {}
    for key, value in mapping.items():
        groups.setdefault(tuple(value), []).append(key)
    return list(groups.items())


ORDER = ['Cmov'] + [variant for variant, _ in FEATURE_MACROS] + LEGACY_AVX512
PSEUDO_CONST = {variant: variant.upper() for variant, *_ in PSEUDO}


def variants(items):
    """A mask expression: `bits(&[...])`, plus a pseudo-feature constant."""
    real = [i for i in items if i not in PSEUDO_CONST]
    expr = 'bits(&[' + ', '.join(sorted(real, key=ORDER.index)) + '])'
    return ' | '.join([expr] + [PSEUDO_CONST[i] for i in items if i in PSEUDO_CONST])


def rust_strs(names):
    return ', '.join(f'"{name}"' for name in names)


def bit(item):
    return PSEUDO_CONST.get(item, f'1 << {item} as u8')


def render(rows, options, implied, requires, derived, tunes):
    extras = [variant for variant, _ in EXTRA_MACROS]
    out = [
        '//! GCC 16.2\'s x86 CPU names (`-march=` and `target("arch=")`), the',
        '//! features each grants, and what every ISA option of `target("...")`',
        '//! does to a feature set, all measured from GCC.',
        '//!',
        '//! Generated by `scripts/gen_x86_arch_table.py` (GCC 16.2.0); do not edit.',
        '//! `--check` re-measures the oracle and rejects drift.',
        '',
        'use super::isa::X86Isa;',
        'use crate::common::x86_features::X86Feature::{self, *};',
        '',
        '/// An extension lccc generates no code for but announces with GCC\'s',
        '/// macro, so that `-march=` grants it from the same measurement.',
        '#[derive(Clone, Copy, Debug, PartialEq, Eq)]',
        '#[repr(u8)]',
        'pub(crate) enum X86ExtraFeature {',
        *[f'    {variant},' for variant in extras],
        '}',
        '',
        'impl X86ExtraFeature {',
        f'    pub(crate) const ALL: [X86ExtraFeature; {len(extras)}] = [',
        *[f'        X86ExtraFeature::{variant},' for variant in extras],
        '    ];',
        '',
        '    /// GCC\'s macro for the extension.',
        '    #[must_use]',
        '    pub(crate) const fn macro_name(self) -> &\'static str {',
        '        match self {',
        *[f'            X86ExtraFeature::{variant} => "{macro}",' for variant, macro in EXTRA_MACROS],
        '        }',
        '    }',
        '',
        '    /// The features whose removal removes this extension (GCC: AVXIFMA',
        '    /// leaves with `-mno-avx2`, SHA512 only with `-mno-avx`, CMPCCXADD',
        '    /// with nothing): it stays enabled only while all of them are.',
        '    #[must_use]',
        '    pub(crate) const fn requires(self) -> u64 {',
        '        match self {',
        *[f'            {" | ".join("X86ExtraFeature::" + v for v in variants_)} => {variants(list(key))},'
          for key, variants_ in group_by_value({v: r for v, (r, _) in requires.items() if r})],
        '            _ => 0,',
        '        }',
        '    }',
        '',
        '    /// The extensions whose removal removes this one (AMX-INT8 needs',
        '    /// AMX-TILE); its option enables them (`-mamx-int8` defines',
        '    /// `__AMX_TILE__`).',
        '    #[must_use]',
        '    pub(crate) const fn requires_extras(self) -> u32 {',
        '        match self {',
        *[f'            {" | ".join("X86ExtraFeature::" + v for v in variants_)} => '
          + ' | '.join(f'1 << X86ExtraFeature::{e} as u8' for e in key) + ','
          for key, variants_ in group_by_value({v: e for v, (_, e) in requires.items() if e})],
        '            _ => 0,',
        '        }',
        '    }',
        '',
        '    /// GCC\'s option for the extension (`-mNAME`).',
        '    #[must_use]',
        '    pub(crate) const fn option(self) -> &\'static str {',
        '        match self {',
        *[f'            X86ExtraFeature::{v} => "{o}",' for v, o in EXTRA_OPTION_NAMES.items()],
        '        }',
        '    }',
        '}',
        '',
        '/// One CPU name GCC accepts.',
        '#[derive(Clone, Copy, Debug, PartialEq, Eq)]',
        'pub(crate) struct X86ArchRow {',
        '    pub name: &\'static str,',
        '    /// GCC accepts the name for x86-64 code (`-m64`).',
        '    pub x86_64: bool,',
        '    features: u64,',
        '    extras: u32,',
        '    /// GCC\'s `-march=` CPU macros (`__haswell`, `__core_avx2__`, ...).',
        '    pub arch_macros: &\'static [&\'static str],',
        '    /// GCC\'s `-mtune=` macros (`__tune_haswell__`, ...); `-march=` alone',
        '    /// implies them, except for `x86-64*` (generic tuning).',
        '    pub tune_macros: &\'static [&\'static str],',
        '}',
        '',
        'impl X86ArchRow {',
        '    /// GCC enables `feature` for this CPU.',
        '    #[must_use]',
        '    pub const fn has(&self, feature: X86Feature) -> bool {',
        '        self.features & (1 << feature as u8) != 0',
        '    }',
        '',
        '    /// GCC enables the macro-only extension `extra` for this CPU.',
        '    #[must_use]',
        '    pub const fn has_extra(&self, extra: X86ExtraFeature) -> bool {',
        '        self.extras & (1 << extra as u8) != 0',
        '    }',
        '',
        '    /// The CPU\'s features as an `EffectiveFeatures` bitmask.',
        '    #[must_use]',
        '    pub const fn feature_bits(&self) -> u64 {',
        '        self.features',
        '    }',
        '',
        '    /// The x86-64 code-generation permission (`X86Isa::NONE` for a',
        '    /// 32-bit-only CPU, which is an error on x86-64).',
        '    #[must_use]',
        '    pub const fn isa(&self) -> X86Isa {',
        '        if !self.x86_64 {',
        '            return X86Isa::NONE;',
        '        }',
        '        X86Isa::from_feature_bits(self.features)',
        '    }',
        '',
        '    /// The features this CPU has only through ABM, which it therefore',
        '    /// does not grant while ABM is denied (GCC: `-march=amdfam10',
        '    /// -mno-abm` has neither POPCNT nor LZCNT, `-march=btver1 -mno-abm`',
        '    /// keeps both; measured for every CPU).',
        '    #[must_use]',
        '    pub fn abm_derived(&self) -> u64 {',
        '        match self.name {',
        *[f'            {" | ".join(chr(34) + n + chr(34) for n in names)} => {variants(list(key))},'
          for key, names in group_by_value(derived)],
        '            _ => 0,',
        '        }',
        '    }',
        '}',
        '',
        'const fn bits(list: &[X86Feature]) -> u64 {',
        '    let mut bits = 0;',
        '    let mut i = 0;',
        '    while i < list.len() {',
        '        bits |= 1 << list[i] as u8;',
        '        i += 1;',
        '    }',
        '    bits',
        '}',
        '',
        *[f'/// Option state outside `X86Feature`: GCC\'s `{o}` (`{m}`).\npub(crate) const {PSEUDO_CONST[v]}: u64 = 1 << {b};' for v, m, o, b in PSEUDO],
        '',
        'const fn row(',
        '    name: &\'static str,',
        '    x86_64: bool,',
        '    features: u64,',
        '    extra: &[X86ExtraFeature],',
        '    arch_macros: &\'static [&\'static str],',
        '    tune_macros: &\'static [&\'static str],',
        ') -> X86ArchRow {',
        '    let mut extras = 0;',
        '    let mut i = 0;',
        '    while i < extra.len() {',
        '        extras |= 1 << extra[i] as u8;',
        '        i += 1;',
        '    }',
        '    X86ArchRow { name, x86_64, features, extras, arch_macros, tune_macros }',
        '}',
        '',
        'const Y: bool = true;',
        'const N: bool = false;',
        '',
        '/// Sorted by name (binary search). Columns: name, x86-64, features,',
        '/// macro-only extensions, `-march=` macro names, `-mtune=` macro names.',
        '#[rustfmt::skip]',
        'pub(crate) const X86_ARCH_TABLE: &[X86ArchRow] = &[',
    ]
    width = max(len(name) for name, *_ in rows) + 3
    names = lambda items: '&[' + ', '.join(f'"{item}"' for item in items) + ']'
    use_extra = lambda items: '&[' + ', '.join(f'X86ExtraFeature::{e}' for e in items) + ']'
    for name, x86_64, features, extra, arch, tune in rows:
        quoted = f'"{name}",'
        out.append(
            f'    row({quoted.ljust(width)} {"Y" if x86_64 else "N"}, {variants(features)}, '
            f'{use_extra(extra)}, {names(arch)}, {names(tune)}),'
        )
    out += [
        '];',
        '',
        '/// One ISA option of `target("...")`: the features `NAME` enables and',
        '/// disables and the features `no-NAME` disables, each with everything',
        '/// GCC\'s implication graph attaches to it (`avx512bmm` enables AVX512BW,',
        '/// `no-avx2` disables every AVX-512 subset).',
        '#[derive(Clone, Copy, Debug, PartialEq, Eq)]',
        'pub(crate) struct X86TargetOption {',
        '    pub name: &\'static str,',
        '    set: u64,',
        '    clear: u64,',
        '    /// `None` where GCC refuses `no-NAME` (`general-regs-only`).',
        '    unset: Option<u64>,',
        '}',
        '',
        'impl X86TargetOption {',
        '    /// `features` (an `EffectiveFeatures` bitmask) after `NAME`, or after',
        '    /// `no-NAME` when `negated`; `None` for a `no-` form GCC refuses.',
        '    #[must_use]',
        '    pub const fn apply(&self, features: u64, negated: bool) -> Option<u64> {',
        '        if !negated {',
        '            return Some((features & !self.clear) | self.set);',
        '        }',
        '        match self.unset {',
        '            Some(unset) => Some(features & !unset),',
        '            None => None,',
        '        }',
        '    }',
        '',
        '    /// The features `NAME` (or `no-NAME`) makes explicit: those it sets or',
        '    /// clears, which the override implications (`X86_IMPLIED`) then leave',
        '    /// alone.',
        '    #[must_use]',
        '    pub const fn touched(&self, negated: bool) -> u64 {',
        '        if !negated {',
        '            return self.set | self.clear;',
        '        }',
        '        match self.unset {',
        '            Some(unset) => unset,',
        '            None => 0,',
        '        }',
        '    }',
        '}',
        '',
        'const fn option(name: &\'static str, set: u64, clear: u64, unset: Option<u64>) -> X86TargetOption {',
        '    X86TargetOption { name, set, clear, unset }',
        '}',
        '',
        '/// Sorted by name (binary search). Columns: name, set, clear, unset.',
        '#[rustfmt::skip]',
        'pub(crate) const X86_TARGET_OPTIONS: &[X86TargetOption] = &[',
    ]
    for name, set_, clear, unset in options:
        denial = 'None' if unset is None else f'Some({variants(unset)})'
        out.append(f'    option("{name}", {variants(set_)}, {variants(clear)}, {denial}),')
    out += [
        '];',
        '',
        '/// GCC\'s option-override implications `(trigger, implied)`: after a',
        '/// command line\'s or a `target("...")`\'s options, `implied` is enabled',
        '/// wherever `trigger` is, unless an option set or cleared `implied`',
        '/// explicitly (`-msse4.2 -mno-popcnt` has no POPCNT; an `arch=` forgets',
        '/// what the command line made explicit).',
        'pub(crate) const X86_IMPLIED: &[(u64, u64)] = &[',
        *[f'    ({bit(t)}, {bit(i)}),' for t, i in implied],
        '];',
        '',
        '/// GCC\'s row for the `-march=` / `target("arch=")` CPU `name`, or',
        '/// `None` for a name GCC rejects (`generic` and `native` included).',
        '#[must_use]',
        'pub(crate) fn x86_arch_row(name: &str) -> Option<&\'static X86ArchRow> {',
        '    X86_ARCH_TABLE',
        '        .binary_search_by(|row| row.name.cmp(name))',
        '        .ok()',
        '        .map(|i| &X86_ARCH_TABLE[i])',
        '}',
        '',
        '/// `-mtune=` names that are not `-march=` CPUs.',
        f'pub(crate) const X86_TUNE_ONLY_NAMES: &[&str] = &[{rust_strs(tunes[0])}];',
        '',
        '/// `-march=` CPUs GCC refuses as `-mtune=` names.',
        f'pub(crate) const X86_MARCH_ONLY_NAMES: &[&str] = &[{rust_strs(tunes[1])}];',
        '',
        '/// GCC accepts `-mtune=name` (`native` aside) for x86-64 code',
        '/// (`x86_64`) or for 32-bit code: the `-march=` CPUs valid in that mode',
        '/// without the `-march=`-only names, plus the tune-only names.',
        '#[must_use]',
        'pub(crate) fn x86_tune_name_valid(name: &str, x86_64: bool) -> bool {',
        '    X86_TUNE_ONLY_NAMES.contains(&name)',
        '        || (!X86_MARCH_ONLY_NAMES.contains(&name)',
        '            && x86_arch_row(name).is_some_and(|row| row.x86_64 || !x86_64))',
        '}',
        '',
        '/// GCC\'s handler for the `target("...")` / `-m` option `name` (without',
        '/// `no-`), or `None` for a name lccc does not model.',
        '#[must_use]',
        'pub(crate) fn x86_target_option(name: &str) -> Option<&\'static X86TargetOption> {',
        '    X86_TARGET_OPTIONS',
        '        .binary_search_by(|option| option.name.cmp(name))',
        '        .ok()',
        '        .map(|i| &X86_TARGET_OPTIONS[i])',
        '}',
        '',
    ]
    return '\n'.join(out)


def rustfmt(text):
    """The text as `cargo fmt` leaves it (the repository's rustfmt config)."""
    tool = shutil.which('rustfmt')
    if tool is None:
        sys.exit('rustfmt not found on PATH (the table is compared as `cargo fmt` leaves it)')
    proc = subprocess.run(
        [tool, '--edition', '2024', '--emit', 'stdout'],
        input=text, capture_output=True, text=True, cwd=ROOT,
    )
    if proc.returncode != 0:
        sys.exit(f'rustfmt failed:\n{proc.stderr}')
    return proc.stdout



def check_committed(text, current, rel, fmt):
    """`--check`: fail unless the committed table is what the oracle generates.

    ``text`` is this run's regenerated table and ``current`` the committed file.
    A byte difference is not necessarily drift: ``text`` went through *this*
    run's rustfmt and the committed file through whichever stable the author
    ran, so formatter churn alone can differ.  Both sides are therefore put
    through ``fmt`` before being compared -- table drift still fails, and the
    committed file's formatting stays owned by `cargo fmt --check`, which CI
    runs separately.  On drift the unified diff is printed (truncated to
    ``DIFF_LIMIT`` lines) because "regenerate it" without the offending rows is
    not actionable from a red check.
    """
    if current == text:
        print(f'PASS: {rel} matches {GCC} ({len(text.splitlines())} lines)')
        return
    normalised = fmt(current) if current else ''
    if normalised == text:
        print(f'PASS: {rel} matches {GCC} ({len(text.splitlines())} lines; '
              'committed copy is stale under this rustfmt, run `cargo fmt`)')
        return
    diff = list(difflib.unified_diff(
        normalised.splitlines(), text.splitlines(),
        fromfile=f'{rel} (committed)', tofile=f'{rel} (regenerated from {GCC})',
        lineterm=''))
    shown = '\n'.join(diff[:DIFF_LIMIT])
    more = f'\n... {len(diff) - DIFF_LIMIT} more diff lines' if len(diff) > DIFF_LIMIT else ''
    sys.exit(f'{rel} drifted from {GCC}; regenerate it\n{shown}{more}')


def main(argv):
    if argv not in ([], ['--check']):
        sys.exit('usage: gen_x86_arch_table.py [--check]')
    options = measure_options()
    rows = measure()
    derived = measure_abm_derived(rows)
    checked = verify_march_negation(rows, options, derived)
    print(f'-march= with -mno-: {checked} cases match the model ({GCC})')
    tunes = measure_tune_vocabulary(rows)
    text = rustfmt(
        render(rows, options, measure_implied(options), measure_extra_requires(), derived, tunes)
    )
    if argv == ['--check']:
        check_committed(text, OUTPUT.read_text() if OUTPUT.exists() else '',
                        str(OUTPUT.relative_to(ROOT)), rustfmt)
        return
    OUTPUT.write_text(text)
    print(f'wrote {OUTPUT.relative_to(ROOT)}')


if __name__ == '__main__':
    main(sys.argv[1:])
