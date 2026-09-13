#!/usr/bin/env python3
"""Minimal ELF64 section/symbol-table prober.

Parses the ELF header directly instead of scraping `readelf -SW`, whose output
wraps each section header over two lines when names are long -- which silently
shifts every column and produces wrong answers (this has already caused two
false conclusions in this project).

Used by scripts/FOLLOWUP_LD_MODERN.md to compare .symtab size, symbol counts,
local/global partitioning (sh_info) and string-table size across linkers:

    python3 scripts/elfprobe.py out_bfd out_lld out_mold out_lccc
"""
import struct, sys
def probe(path):
    d = open(path,'rb').read()
    assert d[:4]==b'\x7fELF'
    is64 = d[4]==2
    e_shoff, = struct.unpack_from('<Q', d, 0x28)
    e_shentsize, e_shnum, e_shstrndx = struct.unpack_from('<HHH', d, 0x3A)
    secs=[]
    for i in range(e_shnum):
        o=e_shoff+i*e_shentsize
        name,typ,flags,addr,off,size,link,info,align,entsize = struct.unpack_from('<IIQQQQIIQQ', d, o)
        secs.append(dict(name=name,typ=typ,off=off,size=size,link=link,info=info,entsize=entsize))
    sh=secs[e_shstrndx]; stroff=sh['off']
    def nm(i):
        e=d.index(b'\0',stroff+i); return d[stroff+i:e].decode()
    for s in secs: s['n']=nm(s['name'])
    out={}
    for s in secs:
        if s['typ']==2:  # SHT_SYMTAB
            n=s['size']//24
            locals_=0
            strtab=secs[s['link']]
            sd=d[strtab['off']:strtab['off']+strtab['size']]
            types={}
            for k in range(n):
                o=s['off']+k*24
                st_name,st_info,st_other,st_shndx,st_value,st_size = struct.unpack_from('<IBBHQQ', d, o)
                bind=st_info>>4; typ=st_info&0xf
                if bind==0: locals_+=1
                types[typ]=types.get(typ,0)+1
            out.update(symtab_bytes=s['size'], nsyms=n, sh_info=s['info'], nlocals=locals_,
                       types=types, strtab_bytes=strtab['size'])
    out['size']=len(d)
    return out
for p in sys.argv[1:]:
    r=probe(p)
    print(f"  {p:12} file={r['size']:>8} symtab={r['symtab_bytes']:>7}B nsyms={r['nsyms']:>6} "
          f"nlocals={r['nlocals']:>6} sh_info={r['sh_info']:>5} "
          f"({'ok' if r['sh_info'] == r['nlocals'] else 'SPEC-VIOLATION, expected ' + str(r['nlocals'])}) "
          f"strtab={r['strtab_bytes']:>6}B types={r['types']}")
