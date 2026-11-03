#!/usr/bin/env python3
"""Hash and lexically screen every C/header/assembly file in a kernel tree.

This is a review inventory, NOT semantic analysis, a profitability model or a
benchmark. Counts include inactive #if branches and macro definitions. Assembly
is hashed but not interpreted. No kernel code is compiled with any compiler.
Reports belong outside Git; they include one record per source file, including
files with no recognized patterns, so absence of matches is not absence of
coverage. Follow-up correctness and runtime/code-size measurements are owed.
"""
import argparse
from collections import Counter
import hashlib
import json
import os
from pathlib import Path
import re

PATTERNS = {
    'loops': r'\b(?:for|while|do)\b',
    'bulk_memory': r'\b(?:memcpy|memmove|memset|memchr|memcmp|memchr_inv)\s*\(',
    'string_operations': r'\b(?:strlen|strnlen|strcmp|strncmp|strcpy|strncpy)\s*\(',
    'bit_operations': r'\b(?:hweight\w*|__builtin_(?:popcount\w*|clz\w*|ctz\w*)|find_\w*bit|rol\d+|ror\d+|swab\d+)\s*\(',
    'overflow_checks': r'\b(?:check_(?:add|sub|mul)_overflow|__builtin_(?:add|sub|mul)_overflow)\s*\(',
    'wide_integer': r'\b(?:__int128|__uint128_t|u128|s128)\b',
    'wide_float': r'\b(?:_Float128|__float128|long\s+double|_Complex)\b',
    'ms_abi': r'\b(?:ms_abi|__efiapi)\b',
    'vector_intrinsics': r'\b(?:_mm\w*|__builtin_ia32_\w*)\s*\(',
    'volatile': r'\b(?:volatile|__volatile__)\b',
    'single_access': r'\b(?:READ_ONCE|WRITE_ONCE|read_once|write_once)\s*\(',
    'atomics': r'\b(?:atomic\w*|cmpxchg\w*|xchg\w*)\s*\(',
    'barriers': r'\b(?:barrier\w*|smp_[rw]?mb|[rw]?mb)\s*\(',
    'locking': r'\b(?:spin_\w*|raw_spin_\w*|mutex_\w*|rcu_\w*)\s*\(',
    'branch_hints': r'\b(?:likely|unlikely|static_branch_\w*|__builtin_expect)\s*\(',
    'inline_assembly': r'\b(?:asm|__asm|__asm__)\b',
}
# A lexical approximation only. Replacing with spaces keeps source locations;
# preprocessing, token pasting and escaped newlines still require real parsing.
NONCODE = re.compile(r'//[^\n]*|/\*[\s\S]*?\*/|"(?:\\[\s\S]|[^"\\])*"|\'(?:\\[\s\S]|[^\'\\\n])*\'')
REGEXES = {name: re.compile(pattern) for name, pattern in PATTERNS.items()}


def mask_noncode(text):
    return NONCODE.sub(lambda match: ''.join('\n' if c == '\n' else ' ' for c in match.group()), text)


def digest(path):
    hasher = hashlib.sha256()
    with path.open('rb') as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b''):
            hasher.update(chunk)
    return hasher.hexdigest()


def inventory(root):
    records, totals = [], Counter()
    for directory, dirs, files in os.walk(root, followlinks=False):
        dirs[:] = sorted(d for d in dirs if d != '.git')
        for name in sorted(files):
            path = Path(directory) / name
            if path.suffix not in ('.c', '.h', '.S', '.s'):
                continue
            record = {'path': path.relative_to(root).as_posix()}
            if path.is_symlink():
                record.update(symlink=os.readlink(path), lexically_screened=False)
                records.append(record)
                continue
            data = path.read_bytes()
            record.update(bytes=len(data), sha256=hashlib.sha256(data).hexdigest(),
                          physical_lines=data.count(b'\n') + int(bool(data) and not data.endswith(b'\n')),
                          lexically_screened=path.suffix in ('.c', '.h'), patterns={})
            if record['lexically_screened']:
                text = mask_noncode(data.decode('utf-8', errors='replace'))
                for label, pattern in REGEXES.items():
                    count, samples = 0, []
                    for match in pattern.finditer(text):
                        count += 1
                        if len(samples) < 6:
                            samples.append(text.count('\n', 0, match.start()) + 1)
                    if count:
                        record['patterns'][label] = {'count': count, 'sample_lines': samples}
                        totals[label] += count
            records.append(record)
    return records, dict(totals)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--kernel', required=True, type=Path)
    parser.add_argument('--out', required=True, type=Path)
    args = parser.parse_args()
    root = args.kernel.resolve(strict=True)
    if not (root / 'Makefile').is_file() or not (root / 'include/linux').is_dir():
        parser.error('--kernel must name a Linux source tree')
    records, totals = inventory(root)
    report = {'schema': 1, 'kernel_root': str(root),
              'scope': 'All .c/.h/.S/.s files, all architectures, including zero-match files',
              'limits': 'Lexical screening, not exhaustive semantic/manual review. Inactive branches included; assembly hashed only. No performance benefit inferred.',
              'patterns': PATTERNS, 'files': records, 'file_count': len(records),
              'screened_file_count': sum(r['lexically_screened'] for r in records),
              'totals': totals,
              'identity': {name: digest(root / name) for name in
                           ('Makefile', '.config', '.lccc-prepared', '.lccc-full-build-config')
                           if (root / name).is_file()}}
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({k: report[k] for k in ('file_count', 'screened_file_count', 'totals', 'identity')}, indent=2))


if __name__ == '__main__':
    main()
