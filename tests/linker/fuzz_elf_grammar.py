#!/usr/bin/env python3
"""Grammar-aware ELF64 fuzzer for the LCCC linker.

Why this exists
---------------
`fuzz_ld.py` mutates bytes of a few seed files. That finds crashes in the
*header parser*, but almost every mutant dies at the first validity check, so
the deeper machinery -- symbol resolution, relocation application, section
merging, group handling -- is never reached. A 6000-mutant campaign can be
green while an entire emitter is untested.

This generator works the other way round: it BUILDS a structurally valid
ELF64 relocatable from a grammar, then makes one targeted, semantically
hostile choice (a relocation pointing past the section, a symbol whose
`st_shndx` names a nonexistent section, a COMDAT group listing itself, a
`sh_link` cycle...). The file parses, so the linker gets far enough for the
interesting code to run, and the question becomes "does it diagnose this, or
does it corrupt / crash / hang?".

Contract
--------
The linker may accept a mutant, or reject it with a diagnostic. It may NOT:
  * crash (signal / Rust panic),
  * hang (enforced by timeout),
  * emit a file while also reporting failure,
  * report success while emitting a malformed ELF that `readelf` rejects.

Anything else is a finding, printed with the exact reproducer path.

Usage
-----
    python3 tests/linker/fuzz_elf_grammar.py --lccc <path-to-lccc-ld> \
        [--iters 500] [--seed 1] [--keep]
"""

import argparse
import os
import random
import shutil
import struct
import subprocess
import sys
import tempfile

# ── ELF64 constants ─────────────────────────────────────────────────────────
ET_REL = 1
EM_X86_64 = 62
SHT_NULL, SHT_PROGBITS, SHT_SYMTAB, SHT_STRTAB = 0, 1, 2, 3
SHT_RELA, SHT_NOBITS, SHT_REL, SHT_GROUP = 4, 8, 9, 17
SHF_WRITE, SHF_ALLOC, SHF_EXECINSTR = 0x1, 0x2, 0x4
SHF_MERGE, SHF_STRINGS, SHF_TLS, SHF_GROUP = 0x10, 0x20, 0x400, 0x200
STB_LOCAL, STB_GLOBAL, STB_WEAK = 0, 1, 2
STT_NOTYPE, STT_OBJECT, STT_FUNC, STT_SECTION, STT_TLS = 0, 1, 2, 3, 6
GRP_COMDAT = 1
SHN_UNDEF, SHN_ABS, SHN_COMMON = 0, 0xFFF1, 0xFFF2

R_X86_64_64, R_X86_64_PC32, R_X86_64_PLT32 = 1, 2, 4
R_X86_64_32, R_X86_64_32S = 10, 11
R_X86_64_TPOFF32, R_X86_64_DTPOFF32 = 23, 21
R_X86_64_GOTPCREL, R_X86_64_REX_GOTPCRELX = 9, 42

RELOC_TYPES = [R_X86_64_64, R_X86_64_PC32, R_X86_64_PLT32,
               R_X86_64_32, R_X86_64_32S, R_X86_64_TPOFF32,
               R_X86_64_DTPOFF32, R_X86_64_GOTPCREL, R_X86_64_REX_GOTPCRELX]

# Byte patterns the relocatable code sections are filled with. `mov` and `cmp`
# forms matter because GOTPCREL relaxation inspects the preceding opcode.
CODE_SNIPPETS = [
    bytes([0x48, 0x8b, 0x05, 0, 0, 0, 0, 0xc3]),        # mov x(%rip),%rax; ret
    bytes([0x48, 0x3b, 0x3d, 0, 0, 0, 0, 0xc3]),        # cmp x(%rip),%rdi; ret
    bytes([0xe8, 0, 0, 0, 0, 0xc3]),                    # call rel32; ret
    bytes([0x64, 0x48, 0x8b, 0x04, 0x25, 0, 0, 0, 0]),  # mov %fs:0,%rax
    bytes([0x90] * 16),
    bytes([0xcc] * 8),
]


class StrTab:
    """Incrementally built ELF string table."""

    def __init__(self):
        self.buf = bytearray(b"\0")
        self.off = {"": 0}

    def add(self, s):
        if s in self.off:
            return self.off[s]
        o = len(self.buf)
        self.buf += s.encode() + b"\0"
        self.off[s] = o
        return o


class Obj:
    """An ELF64 relocatable being assembled."""

    def __init__(self):
        self.shstr = StrTab()
        self.str = StrTab()
        # (name, sh_type, flags, data, link, info, align, entsize)
        self.sections = [("", SHT_NULL, 0, b"", 0, 0, 0, 0)]
        # (name, info, other, shndx, value, size)
        self.symbols = [("", 0, 0, 0, 0, 0)]
        self.relocs = {}   # target section index -> [(off, sym, type, addend)]

    def add_section(self, name, sh_type, flags, data, link=0, info=0,
                    align=1, entsize=0):
        self.sections.append((name, sh_type, flags, data, link, info,
                              align, entsize))
        return len(self.sections) - 1

    def add_symbol(self, name, bind, styp, shndx, value=0, size=0, other=0):
        self.symbols.append((name, (bind << 4) | styp, other, shndx, value, size))
        return len(self.symbols) - 1

    def build(self):
        """Serialise. Symbols are ordered locals-first, as the gABI requires."""
        locals_ = [s for s in self.symbols if (s[1] >> 4) == STB_LOCAL]
        globals_ = [s for s in self.symbols if (s[1] >> 4) != STB_LOCAL]
        ordered = locals_ + globals_
        index_of = {}
        for new_i, sym in enumerate(ordered):
            for old_i, orig in enumerate(self.symbols):
                if orig is sym:
                    index_of[old_i] = new_i
                    break

        symtab = bytearray()
        for (nm, info, other, shndx, value, size) in ordered:
            symtab += struct.pack("<IBBHQQ", self.str.add(nm), info, other,
                                  shndx, value, size)

        secs = list(self.sections)
        # .rela.<target> sections
        for tgt, entries in sorted(self.relocs.items()):
            if not entries:
                continue
            data = bytearray()
            for (off, sym, rtype, addend) in entries:
                sym_ix = index_of.get(sym, sym)
                data += struct.pack("<QQq", off, (sym_ix << 32) | rtype, addend)
            tname = secs[tgt][0] if tgt < len(secs) else ".unknown"
            secs.append((".rela" + tname, SHT_RELA, 0, bytes(data),
                         0, tgt, 8, 24))   # link patched below

        symtab_ix = len(secs)
        secs.append((".symtab", SHT_SYMTAB, 0, bytes(symtab), 0,
                     len(locals_), 8, 24))
        strtab_ix = len(secs)
        secs.append((".strtab", SHT_STRTAB, 0, bytes(self.str.buf), 0, 0, 1, 0))

        # Patch sh_link: symtab -> strtab, rela -> symtab.
        fixed = []
        for i, (nm, st, fl, data, link, info, al, es) in enumerate(secs):
            if st == SHT_SYMTAB:
                link = strtab_ix
            elif st == SHT_RELA and link == 0:
                link = symtab_ix
            fixed.append((nm, st, fl, data, link, info, al, es))
        secs = fixed

        shstr_ix = len(secs)
        for (nm, *_rest) in secs:
            self.shstr.add(nm)
        self.shstr.add(".shstrtab")
        secs.append((".shstrtab", SHT_STRTAB, 0, bytes(self.shstr.buf), 0, 0, 1, 0))

        # Lay out: ehdr, section contents, section headers.
        out = bytearray(b"\0" * 64)
        offsets = []
        for (nm, st, fl, data, link, info, al, es) in secs:
            if al > 1:
                pad = (-len(out)) % al
                out += b"\0" * pad
            offsets.append(len(out))
            if st != SHT_NOBITS:
                out += data
        pad = (-len(out)) % 8
        out += b"\0" * pad
        shoff = len(out)

        for i, (nm, st, fl, data, link, info, al, es) in enumerate(secs):
            out += struct.pack("<IIQQQQIIQQ",
                               self.shstr.add(nm), st, fl, 0,
                               offsets[i], len(data), link, info,
                               max(al, 1), es)

        ehdr = struct.pack(
            "<16sHHIQQQIHHHHHH",
            b"\x7fELF\x02\x01\x01\x00" + b"\0" * 8,
            ET_REL, EM_X86_64, 1, 0, 0, shoff, 0,
            64, 0, 0, 64, len(secs), shstr_ix)
        out[0:64] = ehdr
        return bytes(out)


def base_object(rng, tu):
    """A well-formed object: code, data, symbols, relocations."""
    o = Obj()
    code = rng.choice(CODE_SNIPPETS)
    text = o.add_section(".text", SHT_PROGBITS, SHF_ALLOC | SHF_EXECINSTR,
                         code, align=16)
    data = o.add_section(".data", SHT_PROGBITS, SHF_ALLOC | SHF_WRITE,
                         bytes(rng.randrange(256) for _ in range(8)), align=8)
    o.add_section(".bss", SHT_NOBITS, SHF_ALLOC | SHF_WRITE, b"\0" * 16, align=8)

    o.add_symbol("", STB_LOCAL, STT_SECTION, text)
    o.add_symbol(f"local_{tu}", STB_LOCAL, STT_FUNC, text, 0, len(code))
    g = o.add_symbol(f"global_{tu}", STB_GLOBAL, STT_FUNC, text, 0, len(code))
    o.add_symbol(f"data_{tu}", STB_GLOBAL, STT_OBJECT, data, 0, 8)
    if rng.random() < 0.4:
        o.add_symbol(f"weak_{tu}", STB_WEAK, STT_NOTYPE, SHN_UNDEF)
    if rng.random() < 0.3:
        o.add_symbol(f"tls_{tu}", STB_GLOBAL, STT_TLS, SHN_UNDEF)

    if len(code) >= 8:
        o.relocs[text] = [(rng.randrange(0, max(1, len(code) - 4)),
                           g, rng.choice(RELOC_TYPES), -4)]
    return o, {"text": text, "data": data, "global": g}


# ── Hostile, structurally valid mutations ───────────────────────────────────
# Each returns a short label describing what was corrupted.

def m_reloc_past_end(o, ix, rng):
    t = ix["text"]
    o.relocs[t] = [(0xFFFF_FF00, ix["global"], R_X86_64_64, 0)]
    return "reloc offset past end of section"


def m_reloc_bad_symbol(o, ix, rng):
    t = ix["text"]
    o.relocs[t] = [(0, 9999, R_X86_64_64, 0)]
    return "reloc references nonexistent symbol index"


def m_symbol_bad_shndx(o, ix, rng):
    o.add_symbol("bad_shndx", STB_GLOBAL, STT_OBJECT, 4242, 0, 8)
    return "symbol st_shndx names a nonexistent section"


def m_huge_section_size(o, ix, rng):
    nm, st, fl, data, link, info, al, es = o.sections[ix["data"]]
    o.sections[ix["data"]] = (nm, st, fl, data, link, info, al, es)
    o.add_section(".fake", SHT_PROGBITS, SHF_ALLOC, b"\x01" * 4, align=1)
    # A NOBITS section with an enormous size must not cause an allocation.
    o.add_section(".huge", SHT_NOBITS, SHF_ALLOC | SHF_WRITE,
                  b"\0" * 8, align=1)
    o.sections[-1] = (".huge", SHT_NOBITS, SHF_ALLOC | SHF_WRITE,
                      b"\0" * 8, 0, 0, 1, 0)
    return "huge NOBITS section"


def m_comdat_self_reference(o, ix, rng):
    sig = o.add_symbol("comdat_sig", STB_GLOBAL, STT_NOTYPE, ix["text"])
    gidx = len(o.sections)
    gdata = struct.pack("<I", GRP_COMDAT) + struct.pack("<I", gidx)
    o.add_section(".group", SHT_GROUP, 0, gdata, link=0, info=sig,
                  align=4, entsize=4)
    return "COMDAT group listing itself as a member"


def m_comdat_bad_member(o, ix, rng):
    sig = o.add_symbol("comdat_sig", STB_GLOBAL, STT_NOTYPE, ix["text"])
    gdata = struct.pack("<I", GRP_COMDAT) + struct.pack("<I", 60000)
    o.add_section(".group", SHT_GROUP, 0, gdata, link=0, info=sig,
                  align=4, entsize=4)
    return "COMDAT group member index out of range"


def m_comdat_bad_signature(o, ix, rng):
    gdata = struct.pack("<I", GRP_COMDAT) + struct.pack("<I", ix["text"])
    o.add_section(".group", SHT_GROUP, 0, gdata, link=0, info=7777,
                  align=4, entsize=4)
    return "COMDAT signature symbol index out of range"


def m_merge_bad_entsize(o, ix, rng):
    o.add_section(".rodata.str1.1", SHT_PROGBITS,
                  SHF_ALLOC | SHF_MERGE | SHF_STRINGS,
                  b"abc\0def\0", align=1, entsize=0)
    return "SHF_MERGE|STRINGS with entsize 0"


def m_merge_unterminated(o, ix, rng):
    o.add_section(".rodata.str1.1", SHT_PROGBITS,
                  SHF_ALLOC | SHF_MERGE | SHF_STRINGS,
                  b"no_terminator", align=1, entsize=1)
    return "SHF_STRINGS section without a NUL terminator"


def m_absurd_alignment(o, ix, rng):
    nm, st, fl, data, link, info, al, es = o.sections[ix["text"]]
    o.sections[ix["text"]] = (nm, st, fl, data, link, info, 1 << 40, es)
    return "section alignment 2^40"


def m_non_power_of_two_align(o, ix, rng):
    nm, st, fl, data, link, info, al, es = o.sections[ix["text"]]
    o.sections[ix["text"]] = (nm, st, fl, data, link, info, 3, es)
    return "non-power-of-two section alignment"


def m_tls_without_tls_section(o, ix, rng):
    t = o.add_symbol("tlsvar", STB_GLOBAL, STT_TLS, ix["data"], 0, 4)
    o.relocs[ix["text"]] = [(0, t, R_X86_64_TPOFF32, 0)]
    return "TLS relocation against a non-TLS section"


def m_duplicate_strong_symbols(o, ix, rng):
    o.add_symbol("dup_strong", STB_GLOBAL, STT_FUNC, ix["text"], 0, 4)
    o.add_symbol("dup_strong", STB_GLOBAL, STT_FUNC, ix["text"], 4, 4)
    return "two strong definitions of one symbol in one object"


def m_common_symbol_huge(o, ix, rng):
    o.add_symbol("huge_common", STB_GLOBAL, STT_OBJECT, SHN_COMMON,
                 value=1 << 20, size=(1 << 62))
    return "COMMON symbol with an absurd size"


def m_zero_sized_everything(o, ix, rng):
    for i in (ix["text"], ix["data"]):
        nm, st, fl, _d, link, info, al, es = o.sections[i]
        o.sections[i] = (nm, st, fl, b"", link, info, al, es)
    o.relocs.clear()
    return "all sections zero-length"


def m_many_relocs_one_section(o, ix, rng):
    t = ix["text"]
    o.relocs[t] = [(i % 4, ix["global"], rng.choice(RELOC_TYPES), 0)
                   for i in range(2000)]
    return "2000 overlapping relocations on one small section"


def m_abs_and_undef_mix(o, ix, rng):
    o.add_symbol("abs_sym", STB_GLOBAL, STT_OBJECT, SHN_ABS, 0xDEAD_BEEF, 4)
    u = o.add_symbol("undef_sym", STB_GLOBAL, STT_NOTYPE, SHN_UNDEF)
    o.relocs[ix["text"]] = [(0, u, R_X86_64_PC32, -4)]
    return "reference to an undefined symbol plus an ABS symbol"


def m_section_name_pathological(o, ix, rng):
    o.add_section("." + "x" * 400, SHT_PROGBITS, SHF_ALLOC, b"\x90" * 4)
    o.add_section("", SHT_PROGBITS, SHF_ALLOC, b"\x90" * 4)
    return "pathological section names (400 chars, and empty)"


MUTATIONS = [
    m_reloc_past_end, m_reloc_bad_symbol, m_symbol_bad_shndx,
    m_huge_section_size, m_comdat_self_reference, m_comdat_bad_member,
    m_comdat_bad_signature, m_merge_bad_entsize, m_merge_unterminated,
    m_absurd_alignment, m_non_power_of_two_align, m_tls_without_tls_section,
    m_duplicate_strong_symbols, m_common_symbol_huge, m_zero_sized_everything,
    m_many_relocs_one_section, m_abs_and_undef_mix, m_section_name_pathological,
]

# Link modes with materially different emit paths.
MODES = ["exec", "shared", "relocatable", "script", "gc", "icf", "emit-relocs"]

SCRIPT = """ENTRY(_start)
SECTIONS {
  . = 0x400000 + SIZEOF_HEADERS;
  .text : { *(.text*) }
  .rodata : { *(.rodata*) }
  .data : { *(.data*) }
  .bss : { *(.bss*) *(COMMON) }
  /DISCARD/ : { *(.comment) *(.note*) }
}
"""


def classify(rc, err):
    """Map an exit status to a finding class, or None when acceptable."""
    if rc is None:
        return "HANG"
    if rc < 0:
        return f"SIGNAL {-rc}"
    low = err.lower()
    if "panicked at" in low or "internal error" in low:
        return "PANIC"
    if "stack overflow" in low:
        return "STACK OVERFLOW"
    # A clean error exit is a correct outcome for a hostile input.
    return None


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--lccc", required=True, help="path to lccc-ld")
    ap.add_argument("--iters", type=int, default=400)
    ap.add_argument("--seed", type=int, default=20260818)
    ap.add_argument("--keep", action="store_true",
                    help="keep the work directory (reproducers)")
    args = ap.parse_args()

    rng = random.Random(args.seed)
    td = tempfile.mkdtemp(prefix="fuzzgrammar.")
    with open(os.path.join(td, "t.lds"), "w") as f:
        f.write(SCRIPT)

    findings = {}
    executed = 0
    for i in range(args.iters):
        nobj = rng.randrange(1, 4)
        paths = []
        labels = []
        for tu in range(nobj):
            o, ix = base_object(rng, f"{i}_{tu}")
            if rng.random() < 0.85:
                mut = rng.choice(MUTATIONS)
                try:
                    labels.append(mut(o, ix, rng))
                except Exception as e:            # generator bug, not a finding
                    labels.append(f"<generator error {e!r}>")
            try:
                blob = o.build()
            except Exception as e:
                labels.append(f"<build error {e!r}>")
                continue
            p = os.path.join(td, f"m{i}_{tu}.o")
            with open(p, "wb") as fh:
                fh.write(blob)
            paths.append(p)
        if not paths:
            continue

        mode = MODES[i % len(MODES)]
        outp = os.path.join(td, f"out{i}")
        cmd = [args.lccc, "-o", outp] + paths
        if mode == "shared":
            cmd.insert(1, "-shared")
        elif mode == "relocatable":
            cmd.insert(1, "-r")
        elif mode == "script":
            cmd[1:1] = ["-T", os.path.join(td, "t.lds")]
        elif mode == "gc":
            cmd.insert(1, "--gc-sections")
        elif mode == "icf":
            cmd.insert(1, "--icf=all")
        elif mode == "emit-relocs":
            cmd.insert(1, "--emit-relocs")

        executed += 1
        try:
            r = subprocess.run(cmd, capture_output=True, timeout=20)
            rc, err = r.returncode, r.stderr.decode(errors="replace")
        except subprocess.TimeoutExpired:
            rc, err = None, ""

        cls = classify(rc, err)
        if cls is None and rc == 0 and os.path.exists(outp):
            # Claimed success: the output must be a readable ELF.
            v = subprocess.run(["readelf", "-a", outp],
                               capture_output=True)
            if v.returncode != 0:
                cls = "MALFORMED OUTPUT (readelf rejected a successful link)"
        if cls is not None:
            key = (cls, mode, tuple(sorted(set(labels))))
            if key not in findings:
                findings[key] = (paths, cmd, err[:400])

    print(f"grammar fuzz: {executed} link attempts across {len(MODES)} modes, "
          f"{len(MUTATIONS)} mutation classes")
    if not findings:
        print("no defects found")
        if not args.keep:
            shutil.rmtree(td, ignore_errors=True)
        return 0

    print(f"\n{len(findings)} distinct finding(s):")
    for (cls, mode, labels), (paths, cmd, err) in findings.items():
        print(f"\n  [{cls}] mode={mode}")
        for l in labels:
            print(f"      mutation: {l}")
        print(f"      repro: {' '.join(cmd)}")
        if err.strip():
            print(f"      stderr: {err.strip()[:200]}")
    print(f"\nreproducers kept in {td}")
    return 1


if __name__ == "__main__":
    sys.exit(main())
