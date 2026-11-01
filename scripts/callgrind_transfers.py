#!/usr/bin/env python3
"""Count owned-object taken transfers, respecting Callgrind name compression."""
import argparse
from pathlib import Path
import re

OBJECT = re.compile(r'^(c?ob)=(?:\((\d+)\))?(.*)$')
TRANSFER = re.compile(r'^(?:jcnd|jump)=(\d+)(?:[/ ]|$)')


def count_transfers(lines, own):
    names = {}
    current = None
    total = 0
    seen = False
    for line in lines:
        line = line.rstrip('\r\n')
        record = OBJECT.match(line)
        if record:
            kind, identifier, name = record.groups()
            name = name.lstrip(' ')
            if identifier is not None:
                if name:
                    names[identifier] = name
                elif identifier not in names:
                    raise ValueError(f'undefined object-name reference: {identifier}')
                name = names[identifier]
            # cob= binds the SAME object-name namespace but describes the
            # called object. It must not redirect the current cost context.
            if kind == 'ob':
                current = name
                seen |= current == own
        elif current == own and line.startswith(('jcnd=', 'jump=')):
            transfer = TRANSFER.match(line)
            if transfer is None:
                raise ValueError(f'malformed transfer record: {line}')
            total += int(transfer[1])
    if not seen:
        raise ValueError(f'owned object missing from profile: {own}')
    return total


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('binary')
    p.add_argument('profile', type=Path)
    args = p.parse_args()
    with args.profile.open(errors='surrogateescape') as stream:
        print(count_transfers(stream, args.binary))


if __name__ == '__main__':
    main()
