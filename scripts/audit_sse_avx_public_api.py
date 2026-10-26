#!/usr/bin/env python3
"""Inventory every public SSE/AVX wrapper/macro; inventory is not manual sign-off.

Use --write to refresh the reviewed inventory, --check to reject drift, or
--json to print it. Both conditional definitions and wrapper dependencies are
retained. Instruction semantics, permissions and runtime evidence are separate
review obligations, never inferred from a header's presence.
"""
import argparse
import tempfile
import json
import hashlib
import subprocess
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
OUTPUT = ROOT / 'engineering/isa/sse-avx-public-api.json'
REFERENCE = ROOT / 'engineering/isa/gcc16-sse-avx-reference.json'
HEADERS = ('xmmintrin.h', 'emmintrin.h', 'pmmintrin.h', 'tmmintrin.h',
           'smmintrin.h', 'nmmintrin.h', 'avxintrin.h', 'avx2intrin.h')
COMMENT = re.compile(r'/\*.*?\*/|//[^\n]*', re.S)
DEFINITION = re.compile(r'^[^\n;#={}]*?\b(_mm(?:256)?_\w+)\s*\(', re.M)
MACRO = re.compile(r'^\s*#\s*define\s+(_mm(?:256)?_\w+)\(', re.M)
OBJECT_ALIAS = re.compile(r'^\s*#\s*define\s+(_mm(?:256)?_\w+)[ \t]+(_mm(?:256)?_\w+)\s*$', re.M)
CALL = re.compile(r'\b((?:__builtin_ia32_|__lccc_simd|_mm(?:256)?_)\w*)\s*\(')
CANONICAL = re.compile(r'm\.insert\(\s*"(_mm(?:256)?_\w+)"\s*,\s*BuiltinInfo::intrinsic\(', re.S)


def closing(text, start, left='(', right=')'):
    depth = 0
    for i in range(start, len(text)):
        if text[i] == left:
            depth += 1
        elif text[i] == right:
            depth -= 1
            if depth == 0:
                return i
    raise ValueError(f'unclosed {left} at offset {start}')


def parameter_count(text):
    if text.strip() in ('', 'void'):
        return 0
    depth = 0
    count = 1
    for c in text:
        if c in '([{':
            depth += 1
        elif c in ')]}':
            depth -= 1
        elif c == ',' and depth == 0:
            count += 1
    return count


def self_test():
    # Run the actual inventory pipeline, including the shared header. Pin
    # conditional alternatives, comment hiding, nesting and both macro forms.
    with tempfile.TemporaryDirectory() as d:
        root = Path(d)
        (root / 'src/frontend/sema').mkdir(parents=True)
        (root / 'src/frontend/sema/builtins.rs').write_text('')
        (root / 'include').mkdir()
        for h in HEADERS:
            (root / 'include' / h).write_text('')
        (root / 'include/xmmintrin.h').write_text(r'''/* _mm_fake(void) { } */
static inline V _mm_test(V a,
    V b) { return __lccc_simd128_ps_minps128(0, a, nested(b, 1)); }
#if FOO
#define _mm_macro(a, b) \
    __builtin_ia32_example((a), (b))
#else
static inline V _mm_macro(V a, V b) { return _mm_test(a, b); }
#endif
#define _mm_alias _mm_test
V _mm_prototype(V a);
static inline V _mm_null(void) { return _mm_test(0, 0); }
static inline V _mm_callback(void (*f)(int, int), V a) __attribute__((unused)) { return a; }
''')
        (root / 'include/lcccsimd.h').write_text('static inline V _mm256_shared(V a) { return a; }')
        rows = {r['name']: r for r in inventory(root)['apis']}
        assert set(rows) == {'_mm_test', '_mm_macro', '_mm_alias', '_mm_null', '_mm_callback', '_mm256_shared'}, rows
        assert rows['_mm_test']['arities'] == [2]
        assert rows['_mm_test']['generic_helpers'] == ['__lccc_simd128_ps_minps128']
        assert rows['_mm_macro']['definitions'] == 2
        assert rows['_mm_macro']['raw_builtins'] == ['__builtin_ia32_example']
        assert rows['_mm_macro']['wrapper_dependencies'] == ['_mm_test']
        assert rows['_mm_alias']['wrapper_dependencies'] == ['_mm_test']
        assert rows['_mm_null']['arities'] == [0]
        assert rows['_mm_callback']['arities'] == [2]
    print('PASS: public inventory parser controls')


def inventory(root=ROOT, header_root=None):
    header_root = header_root or root / 'include'
    aliases = set(CANONICAL.findall((root / 'src/frontend/sema/builtins.rs').read_text()))
    rows = []
    headers = HEADERS + (('lcccsimd.h',) if (header_root / 'lcccsimd.h').exists() else ())
    for header in headers:
        path = header_root / header
        raw = path.read_text()
        # Preserve line numbers while hiding comments, not instruction bodies.
        text = COMMENT.sub(lambda m: ''.join('\n' if c == '\n' else ' ' for c in m[0]), raw)
        definitions = {}
        for match in DEFINITION.finditer(text):
            end = closing(text, match.end() - 1)
            body = end + 1
            while body < len(text) and text[body].isspace():
                body += 1
            while True:
                attr = re.match(r'__attribute__?\s*\(', text[body:])
                if not attr:
                    break
                body = closing(text, body + attr.end() - 1) + 1
                while body < len(text) and text[body].isspace():
                    body += 1
            if body >= len(text) or text[body] != '{':
                continue  # A call/prototype, not a definition.
            stop = closing(text, body, '{', '}') + 1
            definitions.setdefault(match[1], []).append((match.start(), stop, 'function', match.end() - 1, end))
        for match in MACRO.finditer(text):
            end = closing(text, match.end() - 1)
            stop = text.find('\n', end)
            while stop >= 0 and text[stop - 1] == '\\':
                stop = text.find('\n', stop + 1)
            if stop < 0:
                stop = len(text)
            definitions.setdefault(match[1], []).append((match.start(), stop, 'macro', match.end() - 1, end))
        for match in OBJECT_ALIAS.finditer(text):
            definitions.setdefault(match[1], []).append((match.start(), match.end(), 'alias', -1, match.start()))
        for name, variants in sorted(definitions.items()):
            calls = set()
            arities = set()
            for start, stop, kind, opening, end in variants:
                if opening >= 0:
                    parameters = text[opening + 1:end].strip()
                    arities.add(parameter_count(parameters))
                calls.update(CALL.findall(text[end + 1:stop]))
            rows.append({'header': header, 'name': name,
                         'definitions': len(variants),
                         'forms': sorted({v[2] for v in variants}),
                         'arities': sorted(arities),
                         'canonical_override': name in aliases,
                         'generic_helpers': sorted(c for c in calls if c.startswith('__lccc_simd')),
                         'raw_builtins': sorted(c for c in calls if c.startswith('__builtin_ia32_')),
                         'wrapper_dependencies': sorted(c for c in calls if c.startswith('_mm') and c != name)})
            if any(v[2] == 'alias' for v in variants):
                rows[-1]['wrapper_dependencies'] = sorted(set(rows[-1]['wrapper_dependencies']) |
                    {m[2] for m in OBJECT_ALIAS.finditer(text) if m[1] == name})
    return {'schema': 1, 'scope': list(headers),
            'status': 'inventory-only; no instruction/manual/runtime sign-off implied',
            'declarations': len(rows), 'unique_names': len({r['name'] for r in rows}),
            'apis': rows}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument('--write', action='store_true')
    group.add_argument('--check', action='store_true')
    group.add_argument('--json', action='store_true')
    group.add_argument('--write-reference', action='store_true')
    group.add_argument('--self-test', action='store_true')
    parser.add_argument('--gcc', default='gcc-16')
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return
    if args.write_reference:
        def gcc(*flags):
            return subprocess.check_output([args.gcc, *flags], text=True, timeout=30).strip()
        if gcc('-dumpfullversion') != '16.2.0':
            raise SystemExit('GCC 16.2.0 resource headers required')
        include = Path(gcc('-print-file-name=include'))
        reference = inventory(header_root=include)
        data = {'schema': 1, 'compiler': gcc('--version').splitlines()[0],
                'headers_sha256': {h: hashlib.sha256((include / h).read_bytes()).hexdigest() for h in HEADERS},
                'names': sorted({r['name'] for r in reference['apis']})}
        REFERENCE.write_text(json.dumps(data, indent=2, sort_keys=True) + '\n')
        print(f"GCC reference: {len(data['names'])} public names")
        return
    result = inventory()
    if REFERENCE.exists():
        names = set(json.loads(REFERENCE.read_text())['names'])
        present = {r['name'] for r in result['apis']}
        result['reference_names'] = len(names)
        result['missing_reference_names'] = sorted(names - present)
        result['bundled_only_names'] = sorted(present - names)
    rendered = json.dumps(result, indent=2, sort_keys=True) + '\n'
    if args.write:
        OUTPUT.write_text(rendered)
    elif args.check:
        if not OUTPUT.exists() or OUTPUT.read_text() != rendered:
            raise SystemExit('public SSE/AVX inventory drift: review then run --write')
    else:
        print(rendered, end='')
    if not args.json:
        print(f"PASS: {result['declarations']} public header entries, {result['unique_names']} unique names; inventory only")


if __name__ == '__main__':
    main()
