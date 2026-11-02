#!/usr/bin/env python3
"""asm.py — count and show the instruction bodies of a compiler's assembly.

usage: asm.py count FILE...            one row per function, one column per file
       asm.py show FUNC FILE           that function's body, one line
A "body" is the instructions between `func:` and the `.size`/next function
label; frame setup is included (it is real text size) unless the compiler was
told to omit it, so pass the same flags to every compiler.
"""
import re, sys

LABEL = re.compile(r'^([A-Za-z_.$][\w.$]*):\s*(?:#.*)?$')
DIRECTIVE = re.compile(r'^\s*\.')
INSTR = re.compile(r'^\s*[a-z][a-z0-9]*(?:\.[a-z0-9]+)?(?:\s|$)')

def bodies(path):
    out, cur, name = {}, [], None
    for line in open(path, errors='replace'):
        s = line.rstrip('\n')
        m = LABEL.match(s)
        if m and not m.group(1).startswith('.L'):
            if name is not None:
                out.setdefault(name, cur)
            name, cur = m.group(1), []
            continue
        if name is None:
            continue
        if DIRECTIVE.match(s):
            if re.match(r'\s*\.size\s+%s,' % re.escape(name), s):
                out.setdefault(name, cur); name, cur = None, []
            continue
        # `#` starts a comment only when it is not an immediate (`#0`, `#-4`,
        # `#0x1f`); AArch64 and RISC-V immediates are spelled with it.
        t = re.sub(r'(?<![\w\])])[ \t]#(?![0-9-]).*$', '', s)
        t = re.sub(r'\s+[;@/]{1,2}.*$', '', t)
        if INSTR.match(t):
            cur.append(' '.join(t.split()))
    if name is not None:
        out.setdefault(name, cur)
    return out

def main(argv):
    cmd = argv[1]
    if cmd == 'count':
        files = argv[2:]
        data = [bodies(f) for f in files]
        names = [n for n in data[0] if not n.startswith('.')]
        for n in names:
            print('%-26s %s' % (n, '  '.join((str(len(d[n])) if n in d else '-').rjust(3) for d in data)))
    elif cmd == 'show':
        fn, path = argv[2], argv[3]
        for ins in bodies(path).get(fn, ['<no such function>']):
            print('   ' + ins)
    else:
        print(__doc__); return 2
    return 0

if __name__ == '__main__':
    sys.exit(main(sys.argv))
