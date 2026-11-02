#!/usr/bin/env python3
"""Print one function's instructions from a .s file (no directives/comments).
Handles GAS output from LCCC, GCC, Clang and ICX: a function label may carry a
trailing `# @name` comment (Clang/ICX), operands may be tab-separated, and
local `.L*` labels inside the body do not end it.
usage: asmfn.py FILE.s FUNC [FUNC...]"""
import re, sys

LABEL = re.compile(r'^([A-Za-z_.$][\w.$]*):\s*(#.*)?$')
text = open(sys.argv[1]).read().splitlines()
for fn in sys.argv[2:]:
    out, on = [], False
    for line in text:
        s = line.strip()
        m = LABEL.match(s)
        if not on:
            on = bool(m and m.group(1) == fn)
            continue
        if m and not m.group(1).startswith('.L'):
            break
        if s.startswith('.size') or s == '.cfi_endproc':
            break
        if not s or m or s.startswith(('.', '#')):
            continue
        s = s.split('#', 1)[0].strip()  # trailing comment (`# kill:` etc.)
        s = ' '.join(s.replace('\t', ' ').split())
        out.append('    ' + s)
    print(f'{fn}: {len(out)} insns')
    print('\n'.join(out))
