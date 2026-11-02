"""Strict ELF64/x86-64 relocatable identity for benchmark entry-symbol renaming.

Compare every non-string/symbol section byte, section layout semantics, and
resolved symbol-table entry. The sole permitted change is one defined global
function symbol's name. No recompilation, instruction normalization, or change
to data/relocations is permitted. Section file offsets may move with strings.
"""
import struct


def object_identity(data, entry):
    def unpack(fmt, offset):
        size = struct.calcsize(fmt)
        if offset < 0 or offset + size > len(data):
            raise ValueError('truncated ELF object')
        return struct.unpack_from(fmt, data, offset)

    def region(offset, size):
        if offset < 0 or size < 0 or offset + size > len(data):
            raise ValueError('out-of-bounds ELF section')
        return data[offset:offset + size]

    def string(table, offset):
        if offset < 0 or offset >= len(table):
            raise ValueError('out-of-bounds ELF string')
        end = table.find(b'\0', offset)
        if end < 0:
            raise ValueError('unterminated ELF string')
        return table[offset:end].decode('utf-8', errors='strict')

    if len(data) < 64 or data[:7] != b'\x7fELF\x02\x01\x01':
        raise ValueError('requires little-endian ELF64')
    typ, machine, version, address, phoff, shoff, flags, ehsize, phentsize, phnum, shentsize, count, names = unpack('<HHIQQQIHHHHHH', 16)
    if (typ != 1 or machine != 62 or version != 1 or address or phoff or phnum
            or ehsize != 64 or shentsize != 64 or not 0 < names < count or count > 65534):
        raise ValueError('unsupported ELF relocatable layout')
    headers = [unpack('<IIQQQQIIQQ', shoff + i * shentsize) for i in range(count)]
    if headers[names][1] != 3:
        raise ValueError('invalid section-name table')
    strings = region(headers[names][4], headers[names][5])
    labels = [string(strings, h[0]) for h in headers]
    if len(set(labels)) != len(labels):
        raise ValueError('ambiguous duplicate section names')
    def section_name(index):
        if 0 <= index < count:
            return ('section', labels[index])
        if index >= 0xff00:
            return ('special', index)
        raise ValueError('invalid section reference')
    result = {'header': (data[:16], flags, phentsize),
              'allocated_order': [labels[i] for i,h in enumerate(headers) if h[2] & 2],
              'sections': {}}
    entries = 0
    for index, h in enumerate(headers):
        name, kind, attrs, addr, offset, size, link, info, align, entsize = h
        label = string(strings, name)
        # Offsets and string-table lengths may change when the name grows.
        if link >= count:
            raise ValueError('invalid linked section')
        metadata = (kind, attrs, addr, section_name(link), section_name(info) if kind in (4,9) else info, align, entsize)
        result['sections'][label] = [metadata]
        if kind in (2, 3) and attrs & 2:
            raise ValueError('allocated symbol/string tables are not rename-only metadata')
        if kind == 3:
            continue
        if kind == 2:
            if entsize != 24 or size % 24 or link >= count or headers[link][1] != 3:
                raise ValueError('invalid ELF symbol table')
            symbol_strings = region(headers[link][4], headers[link][5])
            region(offset, size)
            symbols = []
            for pos in range(offset, offset + size, 24):
                strid, bindtype, other, section, value, extent = unpack('<IBBHQQ', pos)
                symbol = string(symbol_strings, strid)
                if symbol == entry:
                    if bindtype != 0x12 or not 0 < section < count or not headers[section][2] & 4:
                        raise ValueError('entry must be a defined global function in executable code')
                    entries += 1
                    symbol = 'BENCHMARK_ENTRY'
                symbols.append((symbol, bindtype, other, section_name(section), value, extent))
            result['sections'][label].append(('symbols', symbols))
        else:
            result['sections'][label].append(('contents', size, b'' if kind == 8 else region(offset, size)))
    if entries != 1:
        raise ValueError('entry function missing or ambiguous')
    return result


def rename_entry(data, old='main', new='benchmark_entry'):
    """Append a string table and change one st_name, preserving all other bytes.

    Unlike GNU objcopy this does not rewrite LLVM's .llvm_addrsig linkage or
    split a shared section/symbol string table. Existing offsets remain valid;
    the string-table header points to its appended copy plus the new name.
    The full semantic identity check below is mandatory, not advisory.
    """
    before = object_identity(data, old)
    if not new or '\0' in new:
        raise ValueError('invalid new entry name')
    result = bytearray(data)
    shoff, = struct.unpack_from('<Q', data, 40)
    count, = struct.unpack_from('<H', data, 60)
    renamed = False
    for index in range(count):
        h = struct.unpack_from('<IIQQQQIIQQ', data, shoff + 64 * index)
        if h[1] != 2:
            continue
        table_header = shoff + 64 * h[6]
        table = struct.unpack_from('<IIQQQQIIQQ', data, table_header)
        strings = data[table[4]:table[4] + table[5]]
        for offset in range(h[4], h[4] + h[5], 24):
            name, = struct.unpack_from('<I', data, offset)
            if strings[name:strings.index(b'\0', name)].decode('utf-8') != old:
                continue
            if renamed:
                raise ValueError('ambiguous entry rename')
            renamed = True
            replacement = strings + new.encode('utf-8') + b'\0'
            struct.pack_into('<QQ', result, table_header + 24, len(result), len(replacement))
            struct.pack_into('<I', result, offset, len(strings))
            result.extend(replacement)
    if not renamed or object_identity(bytes(result), new) != before:
        raise ValueError('entry rename changed object identity')
    return bytes(result)
