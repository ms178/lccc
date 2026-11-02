"""Minimal little-endian ELF32/ELF64 relocatable reader for the regression gates.

Only what the assembler-parity gates compare: section headers (type, size,
alignment, contents), the symbol table and relocations.  `struct` only, so the gates need
no binutils.
"""

import struct

SHT_SYMTAB, SHT_RELA, SHT_NOBITS, SHT_REL = 2, 4, 8, 9
SHN_UNDEF, SHN_ABS, SHN_COMMON = 0, 0xFFF1, 0xFFF2
BIND = {0: "LOCAL", 1: "GLOBAL", 2: "WEAK"}
VIS = {0: "DEFAULT", 1: "INTERNAL", 2: "HIDDEN", 3: "PROTECTED"}
TYPE = {0: "NOTYPE", 1: "OBJECT", 2: "FUNC", 3: "SECTION", 4: "FILE", 6: "TLS", 10: "IFUNC"}


class Elf:
    """`sections`: name -> (sh_type, bytes or None for NOBITS, sh_size,
    sh_addralign), in header order.  `symbols`: list of
    (name, bind, type, section name | "UND" | "ABS" | "COM", value, size,
    visibility).  `relocs`: list of (target section name, offset, type,
    symbol index into `symbols` or None, addend or None for SHT_REL, where
    the addend is implicit in the target bytes).  `machine`: e_machine."""

    def __init__(self, path):
        with open(path, "rb") as f:
            d = f.read()
        is64 = d[4] == 2
        self.machine, = struct.unpack_from("<H", d, 0x12)
        if is64:
            shoff, = struct.unpack_from("<Q", d, 0x28)
            shentsize, shnum, shstrndx = struct.unpack_from("<HHH", d, 0x3A)
        else:
            shoff, = struct.unpack_from("<I", d, 0x20)
            shentsize, shnum, shstrndx = struct.unpack_from("<HHH", d, 0x2E)

        def hdr(i):
            o = shoff + i * shentsize
            if is64:
                name, typ, _flags, _addr, off, size, link, _info, align, entsize = \
                    struct.unpack_from("<IIQQQQIIQQ", d, o)
            else:
                name, typ, _flags, _addr, off, size, link, _info, align, entsize = \
                    struct.unpack_from("<IIIIIIIIII", d, o)
            return name, typ, off, size, link, align, entsize

        def cstr(base, off):
            return d[base + off:d.index(b"\0", base + off)].decode()

        headers = [hdr(i) for i in range(shnum)]
        shstr = headers[shstrndx][2]
        names = [cstr(shstr, h[0]) for h in headers]
        self.sections = {}
        for i in range(1, shnum):
            _, t, off, size, _, align, _ = headers[i]
            self.sections[names[i]] = (t, None if t == SHT_NOBITS else d[off:off + size],
                                       size, align)
        self.symbols = []
        for _, t, off, size, link, _, entsize in headers:
            if t != SHT_SYMTAB:
                continue
            strbase = headers[link][2]
            for k in range(1, size // entsize):
                o = off + k * entsize
                if is64:
                    nm, info, other, shndx, value, sz = struct.unpack_from("<IBBHQQ", d, o)
                else:
                    nm, value, sz, info, other, shndx = struct.unpack_from("<IIIBBH", d, o)
                if shndx == SHN_UNDEF:
                    sec = "UND"
                elif shndx == SHN_ABS:
                    sec = "ABS"
                elif shndx == SHN_COMMON:
                    sec = "COM"
                else:
                    sec = names[shndx]
                self.symbols.append((cstr(strbase, nm), BIND.get(info >> 4, str(info >> 4)),
                                     TYPE.get(info & 15, str(info & 15)), sec, value, sz,
                                     VIS[other & 3]))
        self.relocs = []
        for i, (_, t, off, size, _link, _, entsize) in enumerate(headers):
            if t not in (SHT_RELA, SHT_REL):
                continue
            target = names[i][len(".rela" if t == SHT_RELA else ".rel"):]
            for k in range(size // entsize):
                o = off + k * entsize
                if is64:
                    r_off, info = struct.unpack_from("<QQ", d, o)
                    sym, typ = info >> 32, info & 0xFFFFFFFF
                    add = struct.unpack_from("<q", d, o + 16)[0] if t == SHT_RELA else None
                else:
                    r_off, info = struct.unpack_from("<II", d, o)
                    sym, typ = info >> 8, info & 0xFF
                    add = struct.unpack_from("<i", d, o + 8)[0] if t == SHT_RELA else None
                self.relocs.append((target, r_off, typ, sym - 1 if sym else None, add))
