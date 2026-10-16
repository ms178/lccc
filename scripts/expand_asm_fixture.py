#!/usr/bin/env python3
"""Expand the deliberately small GAS-macro subset in architectural fixtures.

This is a test adapter, NOT a general assembler preprocessor. The compiler's
peephole consumes instructions, not unevaluated GAS .if/.else branches. Native
comparison against the original GAS fixture independently checks this adapter.
Only positional macros and integer .if/.else/.endif are supported; unsupported
substitution or unbalanced conditionals fail rather than silently skip tests.
"""
from pathlib import Path
import sys
import re


def expand(text):
    macros = {}
    output = []
    lines = iter(text.splitlines())

    def select(body):
        conditions = []
        else_seen = []
        selected = []
        for line in body:
            token = line.strip()
            if token.startswith('.if '):
                conditions.append(bool(int(token[4:].strip(), 0)))
                else_seen.append(False)
            elif token == '.else':
                if not conditions or else_seen[-1]:
                    raise ValueError('unmatched or repeated .else')
                else_seen[-1] = True
                conditions[-1] = not conditions[-1]
            elif token == '.endif':
                if not conditions:
                    raise ValueError('unmatched .endif')
                conditions.pop()
                else_seen.pop()
            elif all(conditions):
                if '\\' in line or token.startswith(('.macro ', '.if', '.endm')):
                    raise ValueError('unsupported fixture macro syntax: ' + line)
                selected.append(line)
        if conditions:
            raise ValueError('unterminated .if')
        return selected

    for line in lines:
        token = line.strip()
        if token.startswith('.macro '):
            name, parameters = token[7:].split(None, 1)
            if name in macros:
                raise ValueError('duplicate macro: ' + name)
            parameters = [p.strip() for p in parameters.split(',')]
            if len(set(parameters)) != len(parameters) or not all(p.isidentifier() for p in parameters):
                raise ValueError('invalid macro parameters')
            body = []
            for definition in lines:
                if definition.strip() == '.endm':
                    break
                body.append(definition)
            else:
                raise ValueError('unterminated .macro')
            macros[name] = parameters, body
            continue
        words = token.split(None, 1)
        if words and words[0] in macros:
            parameters, body = macros[words[0]]
            arguments = [a.strip() for a in words[1].split(',')] if len(words) == 2 else []
            if len(arguments) != len(parameters):
                raise ValueError('macro argument count mismatch')
            bindings = dict(zip(parameters, arguments))
            def substitute(match):
                if match[1] not in bindings:
                    raise ValueError('unknown macro parameter: ' + match[1])
                return bindings[match[1]]
            expanded = re.sub(r'\\([A-Za-z_]\w*)', substitute, '\n'.join(body))
            output.extend(select(expanded.splitlines()))
        else:
            output.extend(select([line]))
    return '\n'.join(output) + '\n'


if __name__ == '__main__':
    if len(sys.argv) != 2:
        raise SystemExit('usage: expand_asm_fixture.py fixture.s')
    print(expand(Path(sys.argv[1]).read_text()), end='')
