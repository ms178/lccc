#!/usr/bin/env python3
"""Per-function instruction counts from an assembly dump (GAS, AT&T syntax).

A function starts at a `<name>:` label whose name is one of the probe
functions; every later non-blank line that is not a label, not a directive
(a leading `.`) and not a comment (a leading `#`) is one instruction.  Counts
include the framing (`push`/`mov`/`pop`/`ret`) for all three compilers, so the
differences are instruction-selection differences, not counting artefacts.
"""
import re
import sys

LABEL = re.compile(r'^([A-Za-z_][\w.$]*):')

def counts(path, names):
    out, cur = {}, None
    for raw in open(path, errors='replace'):
        line = raw.strip()
        if not line:
            continue
        m = LABEL.match(line)
        if m:
            cur = m.group(1) if m.group(1) in names else None
            if cur:
                out.setdefault(cur, 0)
            continue
        if cur and not line.startswith(('.', '#')):
            out[cur] += 1
    return out

def main():
    names = [a for a in sys.argv[1:] if not a.endswith('.s')]
    files = [a for a in sys.argv[1:] if a.endswith('.s')]
    columns = [counts(path, set(names)) for path in files]
    width = max(len(n) for n in names) + 2
    print(''.ljust(width) + ''.join(p.ljust(10) for p in files))
    for name in names:
        row = ''.join(str(c.get(name, 0)).ljust(10) for c in columns)
        print(name.ljust(width) + row)

if __name__ == '__main__':
    main()
