#!/usr/bin/env python3
"""Per-name ISA gating probe for the SSE/AVX/AVX2 intrinsic surface.

Scope: every `_mm_*` / `_mm256_*` intrinsic declared by GCC 16.2's headers
under the union of the ISA flags in UNION_FLAGS, excluding AVX-512 names.
For each name and each tier (`-mavx`, `-mavx2`) the probe asks GCC and lccc
whether one translation unit containing a call to that name compiles with
`-c`.  Gating diagnostics come from inlining the call into a function that
lacks the ISA, so `-fsyntax-only` cannot measure gating.

Soundness: one translation unit per (name, compiler, flag set).  GCC reports
only the first always_inline target mismatch per translation unit, so any
batching would let later failing calls pass silently.  A verdict is
"accepted" iff the compile exits 0, and "rejected" otherwise; nothing is
inferred from which diagnostics appeared.

Speed: GCC spends ~0.45 s per translation unit parsing immintrin.h, so GCC
gets a precompiled header per flag set (a speed-only change: the PCH is built
with the probe's own flags and GCC silently ignores a mismatched PCH).  lccc
parses the same header in ~0.1 s and needs none.

Verdict populations per tier:
  gap       GCC accepts, lccc rejects       (surface missing or gated too hard)
  failopen  lccc accepts, GCC rejects       (ISA leak)

Argument synthesis: function parameter types come from GCC's preprocessed
prototypes.  Macro parameters carry no types, so each is typed by its body (a
parameter cast to a vector type is a vector; otherwise an integer) and the
vector type is the first candidate that GCC accepts; lccc then gets the same
argument tuple, so an argument-type error is never counted as a gating verdict.

Usage: scripts/probe_sse_avx_gating.py --gcc gcc-16 --lccc target/fastbuild/lccc
       [--out PATH] [--jobs 2] [--limit N]
Evidence tooling, not part of CI.  Writes JSON to --out (stdout by default).
"""
import argparse
import concurrent.futures
import itertools
import json
import os
import re
import subprocess
import sys
import tempfile

UNION_FLAGS = ['-mavx2', '-mfma', '-mf16c', '-msse4.2', '-mpclmul', '-maes',
               '-mbmi', '-mbmi2', '-mlzcnt', '-mpopcnt', '-mmovbe', '-mcx16',
               '-mxsave', '-mxsaveopt']
TIERS = {'avx': ['-mavx'], 'avx2': ['-mavx2']}
OPT = '-O2'
SCOPE_RE = re.compile(r'^_mm(?:256)?_\w+$')
# AVX-512 names carry 512, _mask_/_maskz_ or _round/_sae.  AVX names such as
# _mm256_maskload_ps and the SHA intrinsics must stay in scope.
# AVX-512 (512, masks, rounding/SAE forms), FP16 standalone tokens (_ph/_pbh/_sh
# as a word), AVX-NE-CONVERT (cvtne*), and the rounding-control forms `_xxx_round`
# that are not SSE4.1 `_mm[256]_round_*`.
EXCLUDE_RE = re.compile(r'512|_mask_|_maskz_|_mask$|_sae|cvtne|neeph|neobf|_(?:ph|pbh|sh)(?:_|$)|(?<!_mm)(?<!_mm256)(?<!_mm128)_round')
# Not intrinsics: mm_malloc.h helpers that share the _mm prefix.
NOT_INTRINSICS = {'_mm_malloc', '_mm_free'}
TYPE_KEYWORDS = {'int', 'char', 'short', 'long', 'float', 'double', 'void',
                 'unsigned', 'signed', 'const', 'volatile', '_Bool'}
VECTOR_TYPE_RE = re.compile(r'^__m(?:64|128|256)[dfi]?$')


def run(cmd, timeout=300):
    try:
        p = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout,
                           errors='replace')
        return p.returncode, p.stderr
    except subprocess.TimeoutExpired:
        return 124, 'timeout'


class Probe:
    """Owns the scratch directory, GCC precompiled headers, and per-name compiles."""

    def __init__(self, gcc, lccc, workdir):
        self.gcc, self.lccc, self.workdir = gcc, lccc, workdir
        self.pch_dirs = {}

    def pch_dir(self, flags):
        key = tuple(flags)
        if key not in self.pch_dirs:
            d = os.path.join(self.workdir, 'pch_' + '_'.join(f.lstrip('-').replace('.', '') for f in flags))
            os.makedirs(d, exist_ok=True)
            with open(os.path.join(d, 'probe_pch.h'), 'w') as f:
                f.write('#include <immintrin.h>\n')
            rc, err = run([self.gcc, OPT, *flags, '-x', 'c-header', os.path.join(d, 'probe_pch.h'),
                           '-o', os.path.join(d, 'probe_pch.h.gch')])
            if rc != 0:
                sys.exit(f'PCH build failed for {flags}: {err[:300]}')
            self.pch_dirs[key] = d
        return self.pch_dirs[key]

    def compile(self, compiler, flags, name, args, tag, void_form=False):
        """Return (accepted, first error line).  Accepted iff the compile exits 0.

        The call's result is consumed through an opaque sink.  Without that, GCC
        deletes an unused builtin call before its ISA check (which runs at
        expansion, after dead-code elimination), so the probe would pass names
        that GCC rejects.  Always-inline functions are checked earlier, which is
        why the defect is silent for them and visible only for builtins."""
        src = os.path.join(self.workdir, f'{tag}.c')
        obj = os.path.join(self.workdir, f'{tag}.o')
        cmd = [compiler, OPT, *flags]
        # The explicit include is always present: the PCH only saves parse time,
        # so a PCH that GCC silently ignores can never make every name vanish.
        if compiler == self.gcc:
            cmd += ['-include', 'probe_pch.h', '-I', self.pch_dir(flags)]
        call = f'{name}({", ".join(args)})'
        if void_form:
            body = f'    {call};\n'
        else:
            body = (f'    __typeof__({call}) probe_r = {call};\n'
                    '    probe_use(&probe_r);\n')
        text = ('#include <immintrin.h>\n'
                'extern void probe_use(const void *);\n'
                f'void probe_fn(void)\n{{\n{body}}}\n')
        with open(src, 'w') as f:
            f.write(text)
        rc, err = run(cmd + ['-c', src, '-o', obj])
        if os.path.exists(obj):
            os.remove(obj)
        if rc == 0:
            return True, ''
        first = next((ln.strip()[:240] for ln in err.splitlines() if ': error' in ln),
                     f'rc={rc}')
        return False, first


def param_type(decl):
    """Strip the trailing parameter name, keeping qualifiers and pointers."""
    tokens = re.findall(r'\w+|\*', decl)
    if len(tokens) >= 2 and re.fullmatch(r'\w+', tokens[-1]):
        last = tokens[-1]
        is_type = (last in TYPE_KEYWORDS or VECTOR_TYPE_RE.match(last)
                   or last.endswith('_t') or last.startswith('__mmask'))
        if not is_type:
            tokens = tokens[:-1]
    return ' '.join(tokens)


def split_params(text):
    text = text.strip()
    if text in ('', 'void'):
        return []
    parts, depth, cur = [], 0, ''
    for ch in text:
        if ch == ',' and depth == 0:
            parts.append(cur.strip())
            cur = ''
            continue
        depth += ch == '('
        depth -= ch == ')'
        cur += ch
    parts.append(cur.strip())
    return parts


def arg_for_type(ptype):
    """A well-typed argument value for one parameter type."""
    base = re.sub(r'\b(const|volatile)\b', '', ptype).strip()
    if '*' in base:
        return f'({ptype})0'
    if VECTOR_TYPE_RE.match(base):
        return f'({base}){{0}}'
    if base in ('float', 'double'):
        return '1.0'
    return '1'


def return_type(lines, index):
    """GCC writes `extern __inline <RET>` within the few lines above a prototype."""
    for j in range(index - 1, max(index - 5, -1), -1):
        m = re.match(r'extern __inline (\S+)', lines[j])
        if m:
            return m.group(1)
    return None


def collect_declarations(header_text):
    """{name: ('fn', [param types], return type) | ('macro', [param names], body)}."""
    decls = {}
    lines = header_text.split('\n')
    for m in re.finditer(r'^(_mm\w*) \(([^\n]*)\)\s*$', header_text, re.M):
        name = m.group(1)
        if name in decls or not SCOPE_RE.match(name) or EXCLUDE_RE.search(name) \
                or name in NOT_INTRINSICS:
            continue
        index = header_text.count('\n', 0, m.start())
        decls[name] = ('fn', [param_type(p) for p in split_params(m.group(2))],
                       return_type(lines, index))
    for m in re.finditer(r'^#define (_mm\w*)\(([^)]*)\) (.*)$', header_text, re.M):
        name = m.group(1)
        if name in decls or not SCOPE_RE.match(name) or EXCLUDE_RE.search(name) \
                or name in NOT_INTRINSICS:
            continue
        params = [p.strip() for p in m.group(2).split(',') if p.strip()]
        decls[name] = ('macro', params, m.group(3))
    return decls


def top_level_args(text, open_index):
    """Argument strings of the call whose '(' is at open_index, or None if unbalanced."""
    depth, args, cur = 0, [], ''
    for ch in text[open_index:]:
        if ch == '(':
            depth += 1
            if depth == 1:
                continue
        elif ch == ')':
            depth -= 1
            if depth == 0:
                args.append(cur.strip())
                return args
        elif ch == ',' and depth == 1:
            args.append(cur.strip())
            cur = ''
            continue
        if depth >= 1:
            cur += ch
    return None


def strip_parens(text):
    """`((V))` -> `V`: drop parentheses that enclose the whole argument."""
    text = text.strip()
    while text.startswith('(') and text.endswith(')'):
        depth = 0
        for i, ch in enumerate(text):
            depth += (ch == '(') - (ch == ')')
            if depth == 0 and i != len(text) - 1:
                return text
        text = text[1:-1].strip()
    return text


# GCC's internal vector typedefs: __v<count><elem>, elem = qi/hi/si/di (integers),
# sf (float), df (double).  Mapped to the public __m<bits>[i|d|] type of equal width.
VECTOR_ELEM = {'qi': (1, 'i'), 'hi': (2, 'i'), 'si': (4, 'i'), 'di': (8, 'i'),
               'sf': (4, ''), 'df': (8, 'd')}


def vector_type_for_cast(text):
    """`__v8si` -> `__m256i`, `__v4df` -> `__m256d`, `__v8sf` -> `__m256`; public
    `__m` names are returned unchanged."""
    m = re.fullmatch(r'__v(\d+)(qi|hi|si|di|sf|df)', text)
    if not m:
        return text
    size, suffix = VECTOR_ELEM[m.group(2)]
    return f'__m{int(m.group(1)) * size * 8}{suffix}'


def macro_param_types(params, body, decls, depth=0):
    """{param: C type} taken from the declared parameter types of the callees the
    body forwards each parameter to, followed through macro-to-macro forwarding.
    A parameter that is only cast, as in `(__m256d)V`, takes that cast type."""
    types = {}
    if depth > 4:
        return types
    for m in re.finditer(r'\b(\w+)\s*\(', body):
        callee = m.group(1)
        if callee not in decls:
            continue
        args = top_level_args(body, m.end() - 1)
        if not args:
            continue
        decl = decls[callee]
        if decl[0] == 'fn':
            ptypes = decl[1]
        else:
            inner = macro_param_types(decl[1], decl[2], decls, depth + 1)
            ptypes = [inner.get(q) for q in decl[1]]
        for k, a in enumerate(args):
            a = strip_parens(a)
            if a in params and k < len(ptypes) and ptypes[k]:
                types.setdefault(a, ptypes[k])
    for p in params:
        if p in types:
            continue
        cast = re.search(r'\(\s*(__m\d+[dfi]?|__v\d+\w+)\s*\)\s*\(?\s*' + re.escape(p) + r'\b', body)
        if cast:
            types[p] = vector_type_for_cast(cast.group(1))
    return types


def resolve_macro(probe, name, params, body, decls):
    """(argument tuple, void_form) that GCC accepts under the union flags, or (None, False)."""
    types = macro_param_types(params, body, decls)
    args = [arg_for_type(types[p]) if p in types else '1' for p in params]
    ok, msg = probe.compile(probe.gcc, UNION_FLAGS, name, args, f'mres_{name}', False)
    if ok:
        return args, False
    # The void form is only meaningful for a macro that really returns void.  Its
    # result-discarding call is dead-code-eliminated before builtin ISA checks, so
    # using it for a non-void macro would accept names GCC rejects.
    if re.search(r'void', msg):
        ok, _ = probe.compile(probe.gcc, UNION_FLAGS, name, args, f'mres_{name}', True)
        if ok:
            return args, True
    return None, False


def probe_name(task):
    probe, name, args, void_form = task
    row = {'args': args, 'void_form': void_form, 'verdicts': {}}
    if args is None:
        row['unresolved'] = True
        return name, row
    ok, msg = probe.compile(probe.gcc, UNION_FLAGS, name, args, f'u_{name}', void_form)
    row['gcc_union'] = ok
    row['gcc_union_msg'] = msg
    for tier, flags in TIERS.items():
        g_ok, g_msg = probe.compile(probe.gcc, flags, name, args, f'g{tier}_{name}', void_form)
        l_ok, l_msg = probe.compile(probe.lccc, flags, name, args, f'l{tier}_{name}', void_form)
        row['verdicts'][tier] = {'gcc': g_ok, 'lccc': l_ok,
                                 'gcc_msg': g_msg, 'lccc_msg': l_msg}
    return name, row


def main():
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[1])
    ap.add_argument('--gcc', default='gcc-16')
    ap.add_argument('--lccc', default='target/fastbuild/lccc')
    ap.add_argument('--out', default='-')
    ap.add_argument('--jobs', type=int, default=2)
    ap.add_argument('--limit', type=int, default=0)
    opts = ap.parse_args()

    with tempfile.TemporaryDirectory(prefix='gating_probe_') as workdir:
        probe = Probe(opts.gcc, opts.lccc, workdir)
        src = os.path.join(workdir, 'hdr.c')
        with open(src, 'w') as f:
            f.write('#include <immintrin.h>\n')
        rc, err = run([opts.gcc, '-E', '-dD', *UNION_FLAGS, src,
                       '-o', os.path.join(workdir, 'pp.h')])
        if rc != 0:
            sys.exit(f'preprocess failed: {err[:300]}')
        with open(os.path.join(workdir, 'pp.h')) as f:
            header_text = f.read()
        decls = collect_declarations(header_text)
        names = sorted(decls)
        if opts.limit:
            names = names[:opts.limit]

        tasks = []
        for name in names:
            decl = decls[name]
            if decl[0] == 'fn':
                args = [arg_for_type(t) for t in decl[1]]
                void_form = decl[2] == 'void'
            else:
                args, void_form = resolve_macro(probe, name, decl[1], decl[2], decls)
            tasks.append((probe, name, args, void_form))
        # Build every precompiled header on this thread before any worker starts:
        # two workers racing to write the same .gch is a hang, not a slowdown.
        for flags in [UNION_FLAGS, *TIERS.values()]:
            probe.pch_dir(flags)
        with concurrent.futures.ThreadPoolExecutor(max_workers=opts.jobs) as pool:
            rows = dict(pool.map(probe_name, tasks))

    summary, lists = {}, {}
    in_scope = sorted(n for n, r in rows.items() if r.get('gcc_union'))
    for tier in TIERS:
        gap = sorted(n for n, r in rows.items() if tier in r.get('verdicts', {})
                     and r['verdicts'][tier]['gcc'] and not r['verdicts'][tier]['lccc'])
        leak = sorted(n for n, r in rows.items() if tier in r.get('verdicts', {})
                      and r['verdicts'][tier]['lccc'] and not r['verdicts'][tier]['gcc'])
        lists[tier] = {'gap': gap, 'failopen': leak}
        summary[tier] = {'gap': len(gap), 'failopen': len(leak)}
    out = {'gcc': opts.gcc, 'lccc': opts.lccc, 'opt': OPT, 'union_flags': UNION_FLAGS,
           'names_probed': len(names), 'in_scope_gcc_union': len(in_scope),
           'unresolved': sorted(n for n, r in rows.items() if r.get('unresolved')),
           'summary': summary, 'lists': lists, 'results': rows}
    text = json.dumps(out, indent=1, sort_keys=True)
    if opts.out == '-':
        print(text)
    else:
        with open(opts.out, 'w') as f:
            f.write(text + '\n')
    print(json.dumps({'names': len(names), 'in_scope': len(in_scope), **summary}),
          file=sys.stderr)


if __name__ == '__main__':
    main()
