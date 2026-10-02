#!/usr/bin/env bash
# Native CLI evidence for mixed lex/parse errors on every built backend driver.
# Imported corpus bodies are never modified. All source inputs are temporary.
set -euo pipefail
repo=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$repo"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
printf 'int f(void) { @ return missing + ; }\n' > "$tmp/mixed.c"
printf 'int f(void) { return 0; } @\n' > "$tmp/lex-only.c"
python3 - "$repo" "$tmp" <<'PY'
import re
import subprocess
import sys
from pathlib import Path
repo, tmp = map(Path, sys.argv[1:])
sys.path.insert(0, str(repo))
from tools.corpus import diagnostics, process
for name in ('lccc', 'lccc-x86', 'lccc-i686', 'lccc-arm', 'lccc-riscv'):
    exe = repo / 'target/fastbuild' / name
    if not exe.is_file():
        raise SystemExit('FAIL: required backend driver missing: ' + name)
    for source in (tmp / 'mixed.c', tmp / 'lex-only.c'):
        result = process.run([str(exe), '-std=c99', '-fsyntax-only', str(source)], 15)
        assert result['returncode'] == 1 and not result['failure'], (name, source, result)
        text = result['stderr'].decode()
        raw = diagnostics.parse(text)
        errors = [d for d in raw if d['kind'] == 'error' and d['file'] is not None]
        match = re.search(r'^ccc: error: .+: (\d+) frontend error\(s\)$', text, re.M)
        assert match and int(match[1]) == len(errors), (name, source, text)
        assert 'unexpected character byte 0x40' in text, (name, source, text)
        assert diagnostics.classify(1, text, compiler_family='lccc') == 'REJECT'
        print('PASS', name, source.name, 'counted errors:', len(errors))
PY
