#!/usr/bin/env python3
"""Non-kernel GCC oracle for explicit include-path diagnostics in every driver mode."""
from __future__ import annotations
import argparse
import re
import shlex
import subprocess
import tempfile
from pathlib import Path


def run(lccc: list[str], gcc: list[str]) -> None:
    cases = [
        ([], None), (['-Wall', '-Wextra'], None),
        (['-Wmissing-include-dirs'], 'warning'),
        (['-Werror=missing-include-dirs'], 'error'),
        (['-Werror', '-Wmissing-include-dirs', '-Wno-error=missing-include-dirs'], 'warning'),
        (['-Wmissing-include-dirs', '-Wno-missing-include-dirs'], None),
        (['-w', '-Werror=missing-include-dirs'], None),
    ]
    checks = 0
    with tempfile.TemporaryDirectory(prefix='lccc-include-dirs-') as td:
        root = Path(td)
        source = root / 'fixture.c'
        source.write_text('#pragma GCC diagnostic ignored "-Wmissing-include-dirs"\nint sentinel;\n')
        asm = root / 'fixture.S'
        asm.write_text('.text\n')
        existing = root / 'existing'
        existing.mkdir()
        (root / 'directory-link').symlink_to(existing, target_is_directory=True)
        (root / 'dangling-link').symlink_to(root / 'absent')
        path_cases = [('absent', 1), ('existing', 0), ('directory-link', 0), ('dangling-link', 1)]
        for search in ['-I', '-iquote', '-isystem', '-idirafter']:
            for path_name, count in path_cases:
                for flags, severity in cases:
                    # Two missing entries are two independent CLI occurrences.
                    for mode in [['-E'], ['-fsyntax-only'], ['-c'], ['-M'], ['-MM'], ['-E', '-dM']]:
                        expected = [severity] * count if severity and mode not in (['-M'], ['-MM'], ['-E', '-dM']) else []
                        for name, compiler in [('GCC', gcc), ('LCCC', lccc)]:
                            result = subprocess.run(compiler + flags + mode + [search,
                                str(root / path_name), str(source), '-o', str(root / 'out')],
                                text=True, capture_output=True, timeout=30)
                            hits = re.findall(r'(warning|error):[^\n]*\[-W(?:error=)?missing-include-dirs\]', result.stderr)
                            if hits != expected or bool(result.returncode) != ('error' in expected):
                                raise RuntimeError(f'{name} {search} {path_name} {flags} {mode}: '
                                    f'expected {expected}, got {hits}, exit {result.returncode}\n{result.stderr}')
                        checks += 1
            # File-valued search paths diagnose even without the optional leaf.
            # GCC deliberately prints no -W category for this diagnostic.
            for flags, severity in [([], 'warning'), (['-Wno-missing-include-dirs'], 'warning'),
                                    (['-Werror'], 'error'), (['-w', '-Werror'], None)]:
                for name, compiler in [('GCC', gcc), ('LCCC', lccc)]:
                    result = subprocess.run(compiler + flags + ['-fsyntax-only', search,
                        str(source), str(source), '-o', str(root / 'out')],
                        text=True, capture_output=True, timeout=30)
                    hits = re.findall(r'(warning|error):[^\n]*not a directory', result.stderr)
                    expected = [severity] if severity else []
                    if hits != expected or bool(result.returncode) != (severity == 'error'):
                        raise RuntimeError(f'{name} non-directory {search} {flags}: {result.stderr}')
                checks += 1
            for mode in ['-E', '-c']:
                for name, compiler in [('GCC', gcc), ('LCCC', lccc)]:
                    result = subprocess.run(compiler + ['-Werror=missing-include-dirs', search,
                        str(root / 'absent'), mode, str(asm), '-o', str(root / 'out')],
                        text=True, capture_output=True, timeout=30)
                    if not result.returncode or '[-Werror=missing-include-dirs]' not in result.stderr:
                        raise RuntimeError(f'{name} assembly {search} {mode}: {result.stderr}')
                checks += 1
        for name, compiler in [('GCC', gcc), ('LCCC', lccc)]:
            result = subprocess.run(compiler + ['-Wmissing-include-dirs', '-E', '-I', str(root / 'absent'),
                '-I', str(root / 'absent'), str(source)], text=True, capture_output=True, timeout=30)
            if result.returncode or result.stderr.count('[-Wmissing-include-dirs]') != 2:
                raise RuntimeError(f'{name} duplicate directory occurrences: {result.stderr}')
        checks += 1
    print(f'include-path diagnostics: PASS ({checks} checks against non-kernel GCC)')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--lccc', default='target/fastbuild/lccc')
    parser.add_argument('--gcc', default='gcc-16')
    args = parser.parse_args()
    run(shlex.split(args.lccc), shlex.split(args.gcc))
