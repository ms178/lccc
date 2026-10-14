#!/usr/bin/env python3
"""Fail-closed nbody pair-loop mass-read census, not a runtime speed metric.

The pinned workload has seven doubles per Body (56-byte stride, mass at 48).
Track address residues from the actual `bodies` symbol through full-width
copies, LEAs, stride updates and frame pointer spills. Join facts over the CFG;
unknown/clobbered facts cannot certify a read as non-mass. Count memory reads,
including folded arithmetic operands, in the natural velocity-update loop.
This deliberately refuses new/unmodelled instruction/address representations.
"""
import argparse
import json
from pathlib import Path
import re
import sys

import hot_loop_metric as cfg

GP = {}
for root, aliases in {
    'rax': ('rax','eax','ax','al','ah'), 'rbx': ('rbx','ebx','bx','bl','bh'),
    'rcx': ('rcx','ecx','cx','cl','ch'), 'rdx': ('rdx','edx','dx','dl','dh'),
    'rsi': ('rsi','esi','si','sil'), 'rdi': ('rdi','edi','di','dil'),
    'rbp': ('rbp','ebp','bp','bpl'), 'rsp': ('rsp','esp','sp','spl'),
}.items():
    GP.update({alias: root for alias in aliases})
for number in range(8, 16):
    GP.update({f'r{number}{suffix}': f'r{number}' for suffix in ('','d','w','b')})
GP_OPS = {prefix + suffix for prefix in ('mov','lea','add','sub','cmp','test','xor','and','or','inc','dec','neg','not','imul','shl','shr','sar','sal') for suffix in ('b','w','l','q')}
SIMD = {prefix + suffix for prefix in ('add','sub','mul','div','sqrt','min','max','xor','and','or','ucomi','comi','cvt','round') for suffix in ('ss','sd','ps','pd')}
SIMD |= {prefix + suffix for prefix in ('fmadd132','fmadd213','fmadd231','fmsub132','fmsub213','fmsub231','fnmadd132','fnmadd213','fnmadd231','fnmsub132','fnmsub213','fnmsub231') for suffix in ('ss','sd','ps','pd')}
SIMD |= {'pshufd','pshufhw','pshuflw','shufpd','shufps','unpcklpd','unpckhpd','punpcklqdq','punpckhqdq','pxor','pand','por','paddq','psubq','paddd','psubd'}
MEM = re.compile(r"^([^()]*)\(%(\w+)(?:,\s*%(\w+)(?:,\s*([1248]))?)?\)$")
MOV = re.compile(r"^v?mov(?:sd|ss|upd|aps|apd|ups|dqu|dqa|q|l|w|b|d|ddup|dqu[0-9]+|dqa[0-9]+)$")
CC = r'(?:a|ae|b|be|c|e|z|g|ge|l|le|na|nae|nb|nbe|nc|ne|nz|ng|nge|nl|nle|no|np|ns|o|p|pe|po|s)'
PURE_FLOW = re.compile(rf'^(?:j{CC}|j(?:mp|cxz|ecxz|rcxz)|cmov{CC}[wlq]?|set{CC}|nop[lwq]?)$')
CALLER_SAVED = {'rax', 'rcx', 'rdx', 'rsi', 'rdi', 'r8', 'r9', 'r10', 'r11'}


def census_operation_known(mn, ops):
    # Calls may hide Body reads in a callee; ABI clobber propagation alone
    # cannot certify their memory effects. Likewise implicit string/stack
    # operations and unknown opcodes must fail even if the loop header later
    # re-establishes every address fact. This is an intraprocedural census.
    if mn.startswith(('call', 'push', 'pop', 'ret')):
        return False
    return bool(PURE_FLOW.fullmatch(mn) or ops and (
        mn in GP_OPS or mn in ('movslq', 'movzwl', 'movzbl', 'movswl')
        or mn.removeprefix('v') in SIMD or MOV.fullmatch(mn)))


def operands(text):
    return [s.strip() for s in re.split(r",(?![^()]*\))", text)]


def reg(text):
    return GP.get(text[1:]) if text.startswith('%') else None


def retire_slots(facts, slot, size):
    # Frame keys are normalized to the incoming ABI stack pointer. Retire
    # overlapping eight-byte pointer spills, including writes through RSP.
    for key in list(facts):
        if not isinstance(key, tuple):
            continue
        if key[0] != slot[0] or max(key[1], slot[1]) < min(key[1] + 8, slot[1] + size):
            facts.pop(key)


def address(text, facts, constants=frozenset()):
    m = MEM.fullmatch(text)
    if not m:
        return ('unknown', None)
    disp, base, index, _ = m.groups()
    if base == 'rip':
        body = re.fullmatch(r'bodies(?:\+(-?\d+))?', disp)
        if body:
            return ('body', int(body[1] or 0) % 56)
        return ('constant', None) if disp in constants else ('unknown', None)
    if index:
        return ('unknown', None)
    try:
        offset = int(disp or '0', 0)
    except ValueError:
        return ('unknown', None)
    value = facts.get(base)
    if isinstance(value, int):
        return ('body', (value + offset) % 56)
    if isinstance(value, tuple) and value[0] == 'frame':
        return ('stack', ('frame', value[1] + offset))
    return ('unknown', None)


def transfer(line, facts, constants=frozenset()):
    facts = dict(facts)
    parts = line.split(None, 1)
    mn = parts[0]
    ops = operands(parts[1]) if len(parts) > 1 else []
    if mn.startswith('call'):
        return {k: v for k, v in facts.items() if isinstance(k, str) and k not in CALLER_SAVED}
    if mn.startswith(('push', 'pop')):
        facts = {k: v for k, v in facts.items() if isinstance(k, str)}
        sp = facts.get('rsp')
        if isinstance(sp, tuple) and sp[0] == 'frame':
            facts['rsp'] = ('frame', sp[1] + (-8 if mn.startswith('push') else 8))
        else:
            facts.pop('rsp', None)
    base = mn.removeprefix('v')
    if not (mn in GP_OPS or mn in ('leaq','movslq','movzwl','movzbl','movswl')
            or mn.startswith(('j','call','ret','push','pop','cmov','set','nop'))
            or base in SIMD or MOV.fullmatch(mn)):
        return {}
    if mn in ('movsb','movsw','movsl','movsd','movsq') and not ops:
        return {}
    if not ops or mn.startswith('j') or mn in ('ret', 'retq', 'nop', 'nopl', 'nopw'):
        return facts
    destination = reg(ops[-1])
    value = None
    if mn == 'leaq' and len(ops) == 2:
        kind, offset = address(ops[0], facts, constants)
        if kind == 'body':
            value = offset
        elif kind == 'stack':
            value = offset
    elif mn == 'movq' and len(ops) == 2:
        source = reg(ops[0])
        if source and ops[0] == '%' + source:
            value = facts.get(source)
        else:
            kind, slot = address(ops[0], facts, constants)
            if kind == 'stack':
                value = facts.get(slot)
        if destination is None:
            kind, slot = address(ops[1], facts, constants)
            if kind == 'stack':
                retire_slots(facts, slot, 8)
                if value is not None:
                    facts[slot] = value
            else:
                facts = {k: v for k, v in facts.items() if isinstance(k, str)}
    elif mn in ('addq', 'subq') and len(ops) == 2 and ops[0].startswith('$'):
        try:
            old = facts.get(destination)
            delta = (1 if mn == 'addq' else -1) * int(ops[0][1:], 0)
            if isinstance(old, int):
                value = (old + delta) % 56
            elif isinstance(old, tuple) and old[0] == 'frame':
                value = ('frame', old[1] + delta)
        except ValueError:
            pass
    # Read-only GP instructions never define their final operand.
    if mn in {prefix + suffix for prefix in ('cmp','test') for suffix in ('b','w','l','q')}:
        return facts
    if destination:
        facts.pop(destination, None)
        if ops[-1] == '%' + destination and value is not None:
            facts[destination] = value
    if mn.startswith('xchg'):
        for op in ops:
            facts.pop(reg(op), None)
    if mn in ('syscall', 'sysenter') or mn.startswith(('rep', 'lods', 'stos')) and not MOV.fullmatch(mn):
        return {}
    # Explicit frame stores retire the corresponding pointer-spill fact.
    if ops and '(' in ops[-1] and mn != 'leaq' and mn != 'movq':
        kind, slot = address(ops[-1], facts, constants)
        if kind == 'stack':
            try:
                size = width(mn, ops)
            except ValueError:
                size = 1 << 64
            retire_slots(facts, slot, size)
        elif kind == 'unknown':
            facts = {k: v for k, v in facts.items() if isinstance(k, str)}
    return facts


def width(mn, ops):
    if mn.endswith('sd') or mn in ('movlpd', 'movhpd', 'vmovlpd', 'vmovhpd', 'movq', 'vmovq', 'movddup', 'vmovddup', 'vbroadcastsd', 'vpbroadcastq'):
        return 8
    if mn.endswith('ss') or mn in ('movl', 'movd', 'vmovd'):
        return 4
    if mn.endswith('q') and not any('%xmm' in op or '%ymm' in op or '%zmm' in op for op in ops):
        return 8
    if mn == 'movw':
        return 2
    if mn == 'movb':
        return 1
    if any('%zmm' in op for op in ops):
        return 64
    if any('%ymm' in op for op in ops):
        return 32
    if any('%xmm' in op for op in ops):
        return 16
    raise ValueError(f'unmodelled memory footprint: {mn}')


def census(text):
    if re.search(r'(?m)^\s*\.(?:set|equ|equiv|weakref)\b', text):
        raise ValueError('unmodelled symbol aliases')
    if '#APP' in text or re.search(r'(?m)^\s*jmp\s+\*', text):
        raise ValueError('opaque/indirect control flow')
    if not re.search(r'(?m)^\s*\.size\s+bodies,\s*280\s*$', text):
        raise ValueError('missing pinned five-body object layout')
    constants = set()
    readonly = False
    for line in text.splitlines():
        section = re.match(r'^\s*\.section\s+([^,\s]+)', line)
        if section:
            readonly = section[1].startswith('.rodata')
        elif re.match(r'^\s*\.(?:text|data|bss)\b', line):
            readonly = False
        label = cfg._LABEL.match(line)
        if readonly and label:
            constants.add(label[1])
    body = cfg.extract_function([line.split('#', 1)[0] for line in text.splitlines()], 'main')
    if not body:
        raise ValueError('missing main workload')
    blocks = cfg.basic_blocks(body)
    entries, exits = [None] * len(blocks), [None] * len(blocks)
    for _ in range(len(blocks) * 8 + 1):
        changed = False
        for block in blocks:
            incoming = [exits[p] for p in block.pred if exits[p] is not None]
            if block.index == 0:
                entry = {'rsp': ('frame', 0)}
            elif not incoming:
                continue
            else:
                entry = {k: v for k, v in incoming[0].items() if all(x.get(k) == v for x in incoming[1:])}
            state = entry
            for line in block.insns:
                state = transfer(line, state, constants)
            if entry != entries[block.index] or state != exits[block.index]:
                changed = True
                entries[block.index], exits[block.index] = entry, state
        if not changed:
            break
    else:
        raise ValueError('address proof did not converge')
    records = {}
    for block in blocks:
        if entries[block.index] is None:
            continue
        state = entries[block.index]
        rows = []
        for line in block.insns:
            parts = line.split(None, 1)
            mn = parts[0]
            ops = operands(parts[1]) if len(parts) > 1 else []
            if not census_operation_known(mn, ops):
                rows.append({'line': line, 'unknown': True})
            for i, op in enumerate(ops):
                if '(' not in op or mn == 'leaq' or mn.startswith('nop'):
                    continue
                kind, offset = address(op, state, constants)
                store = bool(MOV.fullmatch(mn) and i == len(ops) - 1)
                if kind == 'body':
                    size = width(mn, ops)
                    touches_mass = any(48 <= (offset + j) % 56 <= 55 for j in range(size))
                    rows.append({'line': line, 'mass_read': touches_mass and not store,
                                 'velocity_store': store and offset == 24 and size == 16})
                elif kind == 'unknown':
                    rows.append({'line': line, 'unknown': True})
            state = transfer(line, state, constants)
        records[block.index] = rows
    candidates = []
    for loop in cfg.natural_loops(blocks):
        if loop.composite:
            continue
        rows = [r for b in loop.blocks for r in records.get(b, [])]
        if sum(r.get('velocity_store', False) for r in rows) >= 2:
            candidates.append((loop, rows))
    if len(candidates) != 1:
        raise ValueError(f'expected one recognized pair-update natural loop, found {len(candidates)}')
    loop, rows = candidates[0]
    unknown = [r['line'] for r in rows if r.get('unknown')]
    if unknown:
        raise ValueError('unproved pair-loop effects/addresses: ' + '; '.join(unknown))
    reads = [r['line'] for r in rows if r.get('mass_read')]
    if not 1 <= len(reads) <= 2:
        raise ValueError(f'expected 1..2 semantic mass reads, got {len(reads)}: {reads}')
    return {'loop': loop.header_label, 'mass_reads': len(reads), 'instructions': reads,
            'metric': 'workload-specific proven mass-read operands; NOT runtime'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('assembly', type=Path)
    args = parser.parse_args()
    try:
        print(json.dumps(census(args.assembly.read_text()), sort_keys=True))
    except (OSError, ValueError) as error:
        print(f'nbody mass census: FAIL: {error}', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
