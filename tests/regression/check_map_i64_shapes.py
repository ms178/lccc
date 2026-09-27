#!/usr/bin/env python3
"""Check semantic assembly regions, never compiler-assigned block numbers."""
import re
import sys
from pathlib import Path


def function(text, name):
    match = re.search(r'(?ms)^' + name + r':\n(.*?)^\.size\s+' + name + ',', text)
    assert match, f'missing function {name}'
    return match[1]


def packed_loop(body):
    labels = {m.group(1): m.start() for m in re.finditer(r'(?m)^([.\w]+):', body)}
    loops = []
    for jump in re.finditer(r'(?m)^\s*j\w+\s+([.\w]+)\s*$', body):
        start = labels.get(jump.group(1), len(body))
        if start < jump.start():
            region = body[start:jump.end()]
            if re.search(r'\bv?p(?:sub|add)q\b', region):
                loops.append((start, region))
    assert len(loops) == 1, f'expected one packed backedge, found {len(loops)}'
    return loops[0]


def shape(text, name, copies, step, additions=0):
    body = function(text, name)
    start, loop = packed_loop(body)
    assert len(re.findall(r'\bv?psubq\b', loop)) == copies, name
    assert len(re.findall(r'\bv?paddq\b', loop)) == additions, name
    assert re.search(r'\baddq\s+\$' + str(step) + r'\s*,', loop), name
    assert '%ymm' not in body, name
    assert not re.search(r'\bleaq\s+(?:16|32|48)\(', loop), name
    for offset in range(16, step, 16):
        # Pin both a load and a store with the displacement: result tests
        # independently catch dropped offsets and incorrect stream bases.
        address = str(offset) + r'\(%\w+,\s*%\w+(?:,\s*1)?\)'
        assert re.search(r'\bv?movdqu\s+' + address + r',\s*%xmm', loop), name
        assert re.search(r'\bv?movdqu\s+%xmm\d+,\s*' + address, loop), name
    return body, start, loop


def main():
    vector, rolled, scalar, small = [Path(p).read_text() for p in sys.argv[1:]]
    shape(vector, 'sub64', 4, 64)
    shape(rolled, 'sub64', 1, 16)
    shape(vector, 'tier2', 2, 32, 2)  # Two sub AND two add instructions.
    body, start, loop = shape(vector, 'invariant64', 4, 64)
    assert len(re.findall(r'\b(?:unpcklpd|vpbroadcastq)\b', body[:start])) == 1
    assert not re.search(r'\b(?:unpcklpd|vpbroadcastq)\b', loop)
    assert not re.search(r'\bv?psubq\b', function(scalar, 'sub64'))
    for count in range(3, 17):
        assert re.search(r'\bv?psubq\b', function(small, f'small{count}')), count
    # The region finder must not depend on label numbering.
    renamed = re.sub(r'\.LBB(\d+)', lambda m: '.Lrenamed' + str(int(m[1])+901), vector)
    shape(renamed, 'sub64', 4, 64)
    # Negative controls: missing displacement or operation must be detected.
    for broken in (vector.replace('16(', '0('), vector.replace('psubq', 'paddq')):
        try:
            shape(broken, 'sub64', 4, 64)
        except AssertionError:
            pass
        else:
            raise AssertionError('shape checker accepted a mutation')


if __name__ == '__main__':
    main()
