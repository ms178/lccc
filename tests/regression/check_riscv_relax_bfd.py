#!/usr/bin/env python3
"""Hold lccc-ld's RISC-V linker relaxation to GNU ld 2.47, byte for byte.

Every case is one object (generated assembly or the C fixture
riscv_relax/bfd_mix.c) or the three strmerge objects, linked freestanding (`-static -e _start`) by lccc-ld
and by the pinned bfd ld (scripts/ensure_riscv_ld_247.sh).  bfd is given
`--eh-frame-hdr` (lccc-ld always builds `.eh_frame_hdr`; GCC's driver omits
the flag for `-static`, so without it bfd's image would lack the section and
every later read-only section would shift) and
`-Ttext=<lccc's .text address>`: lccc writes more program headers, and with
`.text` at the same address both images must be identical where relaxation
reaches:

  * every allocated section: same address and size;
  * every allocated PROGBITS section: same bytes (`.text` above all);
  * every symbol both symbol tables define: same value;
  * every `.debug_*` section (the `-g` cases): same bytes.

The generated cases sweep the decisions bfd takes with margins -- call and
tail reach around +-1 MiB and +-2 KiB, the gp window edges for lui/auipc
pairs and every load/store form, c.lui, near-zero absolute and undefined weak
symbols, TLS local-exec, R_RISCV_ALIGN after deletions -- each with RVC on
and off, relaxed and with --no-relax.  The strmerge cases
(riscv_relax/strmerge_{a,b}.c, strmerge_c.s) hold the merging of allocated
SHF_MERGE inputs to bfd/merge.c: pooled and tail-merged strings, merged
.rodata.cst16 / .srodata.cst8 constants, symbols inside merged inputs and the
section flags of a .sdata filled only by .srodata.

The hosted cases (riscv_relax/hosted_unwind.c, GCC-compiled) are linked the
way users link: crt objects, glibc and libgcc, `-static` and dynamic, by the
lccc driver and by the cross GCC driving the pinned ld (`-B<dir>/`, checked
with `-Wl,-v`: `-fuse-ld=bfd` would make collect2 look for `ld.bfd` and
silently fall back to the distro linker).  Those images place PLT/IPLT stubs
differently, so the comparison is decision-exact but layout-free: function
sizes in address order, `.text` and `.eh_frame` sizes, the decoded
`.eh_frame` with addresses and pc-relative pointers masked, and -- with qemu
-- the same output and exit status from both programs, which unwind their
own stacks.  This is where ifunc calls (static memcpy through the IPLT, never
relaxed by bfd) and hundreds of archive members' CIEs meet relaxation.
riscv_relax/nested_trampoline.c joins them: its stack trampoline only runs
when the executable `.note.GNU-stack` marker yields PT_GNU_STACK RWX (under
qemu, which enforces page protection, a RW stack faults), and lccc-ld must
print GNU ld's executable-stack warning unless `--no-warn-execstack`.
The `.plt`/`.rela.plt` sizes must match too (one IPLT slot per *referenced*
IFUNC), imported functions must be exported with a PLT `st_value` exactly
when an input takes their address (GNU ld --emit-relocs is the oracle; GNU
ld itself exports every PLT import, which breaks unwinding), and riscv_relax/pointer_equality.c checks the property
that serves (a function pointer taken in position-dependent code equals the
one libc sees).  riscv_relax/shared_import.c runs a shared object whose
import calls relaxation turned into `jal`s to their PLT entries.
Without the cross toolchain or the pinned ld the script prints a SKIP line
(`--require`, used by CI, turns every SKIP into a failure).
"""

from __future__ import annotations

import argparse
import re
import shutil
import struct
import subprocess
import sys
import tempfile
from dataclasses import dataclass, field
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
TRIPLE = "riscv64-linux-gnu"
DEFAULT_LD = Path.home() / ".cache/bfd-ld-2.47-riscv64-linux-gnu/bin/ld"
MARCH = {"rvc": "rv64gc", "norvc": "rv64g"}
SHT_PROGBITS, SHT_SYMTAB, SHT_NOTE, SHT_NOBITS, SHT_DYNSYM = 1, 2, 7, 8, 11
PT_LOAD, PT_NOTE, PT_PHDR, PT_RISCV_ATTRIBUTES = 1, 4, 6, 0x70000003
PF_X, PF_W, PF_R = 1, 2, 4
SHF_ALLOC = 0x2


class GateFailure(RuntimeError):
    pass


def run(argv: list[str], cwd: Path) -> str:
    done = subprocess.run(argv, cwd=cwd, text=True, capture_output=True, check=False, timeout=300)
    if done.returncode:
        raise GateFailure(f"{' '.join(map(str, argv))} failed ({done.returncode}):\n{done.stderr}")
    return done.stdout


@dataclass
class Elf:
    sections: dict[str, tuple[int, int, int, int, bytes]] = field(default_factory=dict)
    symbols: dict[str, set[int]] = field(default_factory=dict)
    # Defined STT_FUNC symbols as (value, size, name).
    funcs: list[tuple[int, int, str]] = field(default_factory=list)
    # Program headers as (type, flags, filesz, memsz, align).
    phdrs: list[tuple[int, int, int, int, int]] = field(default_factory=list)
    # Dynamic symbols as name -> (st_info, st_shndx).
    dynsyms: dict[str, tuple[int, int]] = field(default_factory=dict)
    # Section header (type, flags, entsize) by name.
    headers: dict[str, tuple[int, int, int]] = field(default_factory=dict)


def read_elf(path: Path) -> Elf:
    """Sections (type, flags, addr, size, bytes) and symbol values by name."""
    data = path.read_bytes()
    if data[:4] != b"\x7fELF" or data[4] != 2 or data[5] != 1:
        raise GateFailure(f"{path}: not an ELF64 little-endian file")
    shoff, = struct.unpack_from("<Q", data, 0x28)
    shentsize, shnum, shstrndx = struct.unpack_from("<HHH", data, 0x3A)
    raw = [struct.unpack_from("<IIQQQQIIQQ", data, shoff + i * shentsize) for i in range(shnum)]
    names = raw[shstrndx]

    def cstr(table_off: int, off: int) -> str:
        end = data.index(b"\0", table_off + off)
        return data[table_off + off:end].decode()

    elf = Elf()
    phoff, = struct.unpack_from("<Q", data, 0x20)
    phentsize, phnum = struct.unpack_from("<HH", data, 0x36)
    for i in range(phnum):
        p_type, p_flags, _, _, _, filesz, memsz, align = struct.unpack_from("<IIQQQQQQ", data, phoff + i * phentsize)
        elf.phdrs.append((p_type, p_flags, filesz, memsz, align))
    for sh in raw:
        name = cstr(names[4], sh[0])
        sh_type, flags, addr, off, size = sh[1], sh[2], sh[3], sh[4], sh[5]
        body = b"" if sh_type == SHT_NOBITS else data[off:off + size]
        elf.sections[name] = (sh_type, flags, addr, size, body)
        elf.headers[name] = (sh_type, flags, sh[9])
        if sh_type == SHT_DYNSYM:
            strtab = raw[sh[6]]
            for i in range(1, size // 24):
                st_name, st_info, _, st_shndx = struct.unpack_from("<IBBH", data, off + i * 24)
                elf.dynsyms[cstr(strtab[4], st_name)] = (st_info, st_shndx)
        if sh_type == SHT_SYMTAB:
            strtab = raw[sh[6]]
            for i in range(1, size // 24):
                st_name, st_info, _, st_shndx, st_value, _ = struct.unpack_from("<IBBHQQ", data, off + i * 24)
                if st_info & 0xF in (3, 4) or not st_name:  # STT_SECTION, STT_FILE
                    continue
                elf.symbols.setdefault(cstr(strtab[4], st_name), set()).add(st_value)
                if st_info & 0xF == 2 and st_shndx:  # defined STT_FUNC
                    st_size, = struct.unpack_from("<Q", data, off + i * 24 + 16)
                    elf.funcs.append((st_value, st_size, cstr(strtab[4], st_name)))
    return elf


def show_phdrs(phdrs: list[tuple[int, int]]) -> str:
    return " ".join(f"{p_type:#x}/{p_flags}" for p_type, p_flags in phdrs)


def separate_code_loads(phdrs: list[tuple[int, int]]) -> str | None:
    """lccc-ld lays `-shared` images out separate-code (bfd's x86-64 default,
    `-z separate-code`): headers and read-only data are mapped R, only code
    R+X. Why the LOADs are not compared with RISC-V bfd's `noseparate-code`
    pair: the policy is no W+X LOAD, exactly one R+X and one R+W, and every
    other LOAD read-only."""
    loads = [flags for kind, flags in phdrs if kind == PT_LOAD]
    if any(f & PF_W and f & PF_X for f in loads):
        return "a LOAD is writable and executable"
    if loads.count(PF_R | PF_X) != 1 or loads.count(PF_R | PF_W) != 1:
        return "not exactly one R+X and one R+W LOAD"
    if any(f not in (PF_R, PF_R | PF_X, PF_R | PF_W) for f in loads):
        return "a LOAD has flags outside R, R+X, R+W"
    return None


def compare_headers(lccc: Elf, bfd: Elf, separate_code: bool = False) -> list[str]:
    """Every non-empty section bfd emits has lccc's type, flags and entsize,
    and the program headers are bfd's, in bfd's order, with bfd's flags.
    With `separate_code` (lccc's `-shared` layout) the LOADs follow
    `separate_code_loads` instead, PT_PHDR is not compared, and every other
    header must still be bfd's, in bfd's order."""
    problems = []
    mine_ph = [(p[0], p[1]) for p in lccc.phdrs]
    theirs_ph = [(p[0], p[1]) for p in bfd.phdrs]
    if separate_code:
        if (why := separate_code_loads(mine_ph)) is not None:
            problems.append(f"separate-code LOADs {show_phdrs(mine_ph)}: {why}")
        mine_ph = [p for p in mine_ph if p[0] not in (PT_LOAD, PT_PHDR)]
        theirs_ph = [p for p in theirs_ph if p[0] not in (PT_LOAD, PT_PHDR)]
    if mine_ph != theirs_ph:
        problems.append(f"program headers (type/flags) {show_phdrs(mine_ph)}, bfd {show_phdrs(theirs_ph)}")
    for name, header in bfd.headers.items():
        mine = lccc.headers.get(name)
        if name and bfd.sections[name][3] and mine is not None and mine != header:
            problems.append(f"section {name}: (type, flags, entsize) {mine}, bfd {header}")
    return problems


def compare(lccc: Elf, bfd: Elf) -> list[str]:
    problems: list[str] = compare_headers(lccc, bfd)
    alloc = {n: s for n, s in bfd.sections.items() if s[1] & SHF_ALLOC and s[3]}
    for name, (sh_type, _, addr, size, body) in alloc.items():
        mine = lccc.sections.get(name)
        if mine is None:
            problems.append(f"section {name} missing")
            continue
        if (mine[2], mine[3]) != (addr, size):
            problems.append(f"section {name}: {mine[2]:#x}+{mine[3]:#x}, bfd {addr:#x}+{size:#x}")
        elif sh_type == SHT_PROGBITS and mine[4] != body:
            at = next(i for i, (a, b) in enumerate(zip(mine[4], body)) if a != b)
            problems.append(f"section {name}: bytes differ first at {addr + at:#x}")
    for name, (_, _, _, size, body) in bfd.sections.items():
        if (name.startswith(".debug_") or name in (".riscv.attributes", ".comment")) and size:
            mine = lccc.sections.get(name)
            if mine is None or mine[4] != body:
                problems.append(f"section {name}: bytes differ")
    for name, values in sorted(bfd.symbols.items()):
        mine = lccc.symbols.get(name)
        if mine is not None and mine != values:
            problems.append(f"symbol {name}: {sorted(map(hex, mine))}, bfd {sorted(map(hex, values))}")
    return problems


# ── Generated assembly cases ────────────────────────────────────────────────

HEAD = "\t.text\n\t.globl _start\n\t.type _start, @function\n"


def call_cases() -> dict[str, str]:
    """call/tail to targets around jal (+-1 MiB) and c.j (+-2 KiB) reach,
    forward and backward, so the margin bfd reserves decides."""
    out: dict[str, str] = {}
    for pad in (0x7F0, 0x7F6, 0x7F8, 0x7FA, 0x7FC, 0x800, 0x804, 0x810):
        out[f"tail-fwd-{pad:#x}"] = HEAD + f"_start:\n\ttail far\n\t.skip {pad}\nfar:\n\tret\n"
        out[f"tail-back-{pad:#x}"] = HEAD + f"far:\n\tret\n\t.skip {pad}\n_start:\n\ttail far\n"
    for pad in (0xFFFF0, 0xFFFF8, 0xFFFFA, 0xFFFFC, 0xFFFFE, 0x100000, 0x100008):
        out[f"call-fwd-{pad:#x}"] = (
            HEAD + f"_start:\n\tcall far\n\tcall near\n\tj _start\nnear:\n\tret\n\t.skip {pad}\nfar:\n\tret\n"
        )
        out[f"call-back-{pad:#x}"] = HEAD + f"far:\n\tret\n\t.skip {pad}\n_start:\n\tcall far\n\tj _start\n"
    # Calls whose reach depends on the deletions of other calls between.
    body = "".join(f"\tcall f{i}\n" for i in range(64))
    funcs = "".join(f"f{i}:\n\tret\n\t.skip {0x3FF8 + 2 * i}\n" for i in range(64))
    out["call-chain"] = HEAD + f"_start:\n{body}\tj _start\n{funcs}"
    # Undefined weak callee: bfd rewrites to a jump to address 0's neighbour.
    out["call-undef-weak"] = HEAD + "\t.weak nothere\n_start:\n\tcall nothere\n\ttail nothere\n"
    return out


def data_cases() -> dict[str, str]:
    """lui/auipc pairs against data across the gp window and c.lui range,
    with every low-part form (addi, load, store)."""
    out: dict[str, str] = {}
    forms = (
        ("lui", "\tlui a0, %hi({s})\n\taddi a0, a0, %lo({s})\n"
                "\tlui a1, %hi({s})\n\tlw a2, %lo({s})(a1)\n"
                "\tlui a3, %hi({s})\n\tsw a2, %lo({s})(a3)\n"),
        ("pcrel", "1:\tauipc a0, %pcrel_hi({s})\n\taddi a0, a0, %pcrel_lo(1b)\n"
                  "2:\tauipc a1, %pcrel_hi({s})\n\tld a2, %pcrel_lo(2b)(a1)\n"
                  "3:\tauipc a3, %pcrel_hi({s})\n\tsd a2, %pcrel_lo(3b)(a3)\n"),
    )
    # Symbols at offsets from __SDATA_BEGIN__ straddling gp-0x800..gp+0x800
    # (gp = __SDATA_BEGIN__ + 0x800 here), with alignment-driven reserve.
    offs = (0, 0x7F8, 0xFF0, 0xFF8, 0xFFC, 0x1000, 0x1004, 0x1800, 0x10000)
    for kind, form in forms:
        for align in (3, 6):
            refs = "".join(form.format(s=f"d{i}") for i in range(len(offs)))
            syms = "".join(
                f"\t.skip {off - (offs[i - 1] if i else 0) - (8 if i else 0)}\n\t.globl d{i}\nd{i}:\n\t.8byte 0\n"
                for i, off in enumerate(offs)
            )
            out[f"{kind}-gp-a{align}"] = (
                HEAD + f"_start:\n{refs}\tj _start\n\t.section .sdata,\"aw\"\n\t.p2align {align}\n{syms}"
            )
        # Below the small data: .data symbols reached only from below gp.
        refs = "".join(form.format(s=f"b{i}") for i in range(4))
        syms = "".join(f"\t.globl b{i}\nb{i}:\n\t.skip {0x7F8 + 8 * i}\n" for i in range(4))
        out[f"{kind}-data"] = (
            HEAD + f"_start:\n{refs}\tj _start\n\t.data\n{syms}\t.section .sdata,\"aw\"\n\t.8byte 1\n"
        )
    # c.lui: targets out of gp reach whose %hi fits 6 bits, any rd but x0/x2.
    for rd in ("a0", "s1", "t6", "sp", "zero"):
        out[f"clui-{rd}"] = (
            HEAD + f"_start:\n\tlui {rd}, %hi(far)\n\taddi {rd}, {rd}, %lo(far)\n\tlui {rd}, %hi(edge)\n"
            f"\tlw {rd}, %lo(edge)({rd})\n\tj _start\n\t.bss\n\t.skip 0x6000\nfar:\n\t.skip 0x8000\nedge:\n"
            "\t.skip 8\n\t.section .sdata,\"aw\"\n\t.8byte 1\n"
        )
    # Absolute symbols near zero (x0 base) and undefined weak data.
    out["abs-near-zero"] = HEAD + (
        "\t.globl lo, hi, mid\n\t.set lo, 0x10\n\t.set mid, 0x7ff\n\t.set hi, 0x800\n\t.weak wd\n_start:\n"
        + "".join(f"\tlui a0, %hi({s})\n\tlw a1, %lo({s})(a0)\n" for s in ("lo", "mid", "hi", "wd"))
        + "".join(f"1:\tauipc a0, %pcrel_hi({s})\n\tlw a1, %pcrel_lo(1b)(a0)\n" for s in ("wd",))
        + "\tj _start\n"
    )
    # TLS local-exec: tprel within and beyond the 12-bit window.
    out["tls-le"] = HEAD + "_start:\n" + "".join(
        f"\tlui a0, %tprel_hi({s})\n\tadd a0, a0, tp, %tprel_add({s})\n\tlw a1, %tprel_lo({s})(a0)\n"
        f"\tlui a2, %tprel_hi({s})\n\tadd a2, a2, tp, %tprel_add({s})\n\tsw a1, %tprel_lo({s})(a2)\n"
        for s in ("t0v", "t1v", "t2v", "t3v")
    ) + (
        "\tj _start\n\t.section .tbss,\"awT\",@nobits\nt0v:\n\t.skip 0x7f8\nt1v:\n\t.skip 8\nt2v:\n"
        "\t.skip 0x800\nt3v:\n\t.skip 8\n"
    )
    return out


def align_cases() -> dict[str, str]:
    """R_RISCV_ALIGN after deletions, and section-relative references that
    must follow the bytes (`.Lx-1`, the DWARF location-list idiom)."""
    out: dict[str, str] = {}
    for p2 in (2, 3, 4, 5, 6):
        out[f"align-{p2}"] = HEAD + (
            "_start:\n\tcall g\n\tlui a0, %hi(v)\n\tlw a0, %lo(v)(a0)\n"
            f"\t.p2align {p2}\n.Lx:\n\tcall g\n\t.p2align {p2}\ng:\n\tret\n"
            "\t.data\n\t.8byte .Lx-1\n\t.8byte .Lx\n\t.8byte g-1\n"
            "\t.section .sdata,\"aw\"\nv:\n\t.4byte 1\n"
        )
    return out


def generated_cases() -> dict[str, str]:
    return {**call_cases(), **data_cases(), **align_cases()}


# ── Driver ─────────────────────────────────────────────────────────────────


def link_pair(lccc: Path, ld: Path, objs: list[Path], tmp: Path, tag: str, relax: bool,
              options: tuple[str, ...] = ()) -> list[str]:
    """Link `objs` with both linkers, passing each the linker `options`."""
    mine, theirs = tmp / f"{tag}.lccc", tmp / f"{tag}.bfd"
    extra = ([] if relax else ["-Wl,--no-relax"]) + [f"-Wl,{o}" for o in options]
    run([str(lccc), "-nostdlib", "-static", "-Wl,-e,_start", *extra, *map(str, objs), "-o", str(mine)], tmp)
    mine_elf = read_elf(mine)
    text = mine_elf.sections.get(".text")
    if text is None:
        return [".text missing from lccc output"]
    run(
        [str(ld), "-static", "-e", "_start", "--eh-frame-hdr", f"-Ttext={text[2]:#x}", *([] if relax else ["--no-relax"]),
         *options, *map(str, objs), "-o", str(theirs)],
        tmp,
    )
    return compare(mine_elf, read_elf(theirs))


# ── `.riscv.attributes` merging ─────────────────────────────────────────────
#
# Each case links several objects whose attributes GAS writes from
# `.attribute` directives; the merged section must equal bfd's byte for byte.
# The sets exercise every rule of `riscv_merge_attributes`: the arch union in
# canonical order at the newest versions, the priv-spec upgrade (including the
# abandoned 1.9.1), the unaligned-access OR, stack alignment, and unknown tags
# surviving only where every input agrees.

ATTR_OBJECTS = {
    "c": ('rv64i2p1_c2p0', "unaligned_access, 1\npriv_spec, 1\npriv_spec_minor, 11\n20, 7"),
    "d": ('rv64i2p1_f2p2_d2p2_zicsr2p0', "stack_align, 16\npriv_spec, 1\npriv_spec_minor, 12\n20, 7\n22, 3"),
    "z": ('rv64i2p0_m2p0_xfoo1p0_zba1p0_sstc1p0_zbb1p0',
          "priv_spec, 1\npriv_spec_minor, 9\npriv_spec_revision, 1"),
    "a": ("rv64i2p1_a2p1", "21, \"vendor\""),
    "v": ("rv64i2p1_v1p0", "21, \"vendor\""),
    "m3": ("rv64i2p1_m3p0_zcb1p0", "stack_align, 16"),
    "s8": ("rv64i2p1", "stack_align, 8"),
}
# Non-canonical inputs GAS never writes (it adds implied subsets such as
# zicsr for f), encoded byte for byte: the arch union must still match bfd.
RAW_ARCH_OBJECTS = {"rf": "rv64i2p1_f2p2", "rc": "rv64i2p1_c2p0", "rzc": "rv64i2p1_zca1p0_zcb1p0",
                    "rm": "rv64i2p1_m2p0_zmmul1p0", "re": "rv64e2p0", "rv32": "rv32i2p1"}
ATTR_CASES = {"cd": "cd", "dcz": "dcz", "zc": "zc", "zav": ["z", "a", "v"], "m3dc": ["m3", "d", "c"], "va": "va",
              "raw-fc": ["rf", "rc"], "raw-zcm": ["rzc", "rm"], "raw-fcd": ["rf", "rc", "d"]}
# bfd refuses these ("failed to merge target specific data"); so must lccc.
ATTR_CONFLICTS = {"stack": ["d", "s8"], "xlen": ["c", "rv32"], "base": ["c", "re"]}


def uleb128(value: int) -> bytes:
    out = bytearray()
    while True:
        byte, value = value & 0x7F, value >> 7
        out.append(byte | (0x80 if value else 0))
        if not value:
            return bytes(out)


def raw_attributes(arch: str) -> bytes:
    """A `.riscv.attributes` section holding only `Tag_RISCV_arch`."""
    attrs = uleb128(5) + arch.encode() + b"\0"
    sub = b"\x01" + struct.pack("<I", len(attrs) + 5) + attrs
    vendor = b"riscv\0" + sub
    return b"A" + struct.pack("<I", len(vendor) + 4) + vendor


def attribute_objects(cc: str, tmp: Path) -> dict[str, Path]:
    objs = {}
    for i, (key, (arch, extra)) in enumerate(ATTR_OBJECTS.items()):
        body = "".join(f".attribute {line}\n" for line in extra.split("\n") if line)
        start = ".globl _start\n_start:\n" if i == 0 else ""
        src = tmp / f"attr-{key}.s"
        src.write_text(f'.attribute arch, "{arch}"\n{body}.text\n{start}attr_{key}:\n  nop\n')
        objs[key] = src.with_suffix(".o")
        run([cc, "-mabi=lp64", "-march=rv64gc", "-c", str(src), "-o", str(objs[key])], tmp)
    for key, arch in RAW_ARCH_OBJECTS.items():
        src = tmp / f"attr-{key}.s"
        data = ",".join(str(b) for b in raw_attributes(arch))
        src.write_text(f'.section .riscv.attributes,"",%0x70000003\n.byte {data}\n.text\nattr_{key}:\n  nop\n')
        objs[key] = src.with_suffix(".o")
        run([cc, "-mabi=lp64", "-march=rv64i", "-Wa,-mno-arch-attr", "-c", str(src), "-o", str(objs[key])], tmp)
    return objs


def attribute_cases(lccc: Path, ld: Path, objs: dict[str, Path], tmp: Path) -> list[tuple[str, list[str]]]:
    # `_start` lives in the first object of ATTR_OBJECTS; put it in every link.
    first = next(iter(ATTR_OBJECTS))
    results = []
    for name, keys in ATTR_CASES.items():
        inputs = [objs[k] for k in keys]
        if objs[first] not in inputs:
            inputs.append(objs[first])
        results.append((f"attrs-{name}", link_pair(lccc, ld, inputs, tmp, f"attrs-{name}", True)))
    for name, keys in ATTR_CONFLICTS.items():
        inputs = [str(objs[k]) for k in keys]
        if str(objs[first]) not in inputs:
            inputs.append(str(objs[first]))
        codes = []
        for argv in ([str(lccc), "-nostdlib", "-static", "-Wl,-e,_start", *inputs, "-o", str(tmp / "conflict.lccc")],
                     [str(ld), "-static", "-e", "_start", *inputs, "-o", str(tmp / "conflict.bfd")]):
            codes.append(subprocess.run(argv, cwd=tmp, capture_output=True, check=False, timeout=120).returncode)
        problems = [] if codes[0] and codes[1] else [f"exit codes lccc {codes[0]}, bfd {codes[1]}: both must refuse"]
        results.append((f"attrs-conflict-{name}", problems))
    return results


# ── Build ids ───────────────────────────────────────────────────────────────
#
# GNU ld discards every input `.note.gnu.build-id`: the output carries its own
# note (`--build-id`) or none. The stale input here is one bfd stamped itself.

def build_id_cases(args: argparse.Namespace, objs: dict[str, Path], tmp: Path) -> list[tuple[str, list[str]]]:
    stamped = tmp / "build-id-stamped.o"
    run([str(args.ld), "-r", "--build-id", str(objs["d"]), "-o", str(stamped)], tmp)
    first = objs[next(iter(ATTR_OBJECTS))]
    results = []
    for tag, inputs, options in (("build-id-stale", [first, stamped], ()),
                                 ("build-id-restamp", [first, stamped], ("--build-id",)),
                                 ("build-id-stale-first", [stamped, first], ("--build-id=md5",)),
                                 ("build-id-uuid", [first], ("--build-id=0x0123456789abcdef",))):
        results.append((tag, link_pair(args.lccc, args.ld, inputs, tmp, tag, True, options)))
    return results


# ── PT_GNU_STACK ────────────────────────────────────────────────────────────
#
# GNU ld emits PT_GNU_STACK only when an input carries a `.note.GNU-stack`
# marker or the command line decides (-z [no]execstack); a `"x"` marker (a
# nested-function trampoline) makes the stack executable. The program-header
# comparison in `compare_headers` checks presence, order and flags.

def stack_cases(args: argparse.Namespace, objs: dict[str, Path], tmp: Path) -> list[tuple[str, list[str]]]:
    markers = {}
    for key, flags in (("exec", "x"), ("noexec", "")):
        src = tmp / f"stack-{key}.s"
        src.write_text(f'.text\nstack_{key}:\n  nop\n.section .note.GNU-stack,"{flags}",@progbits\n')
        markers[key] = src.with_suffix(".o")
        run([args.cc, "-mabi=lp64", "-march=rv64gc", "-c", str(src), "-o", str(markers[key])], tmp)
    first = objs[next(iter(ATTR_OBJECTS))]
    results = []
    for tag, inputs, options in (("stack-unmarked", [first], ()),
                                 ("stack-marked", [first, markers["noexec"]], ()),
                                 ("stack-trampoline", [first, markers["noexec"], markers["exec"]], ()),
                                 ("stack-z-execstack", [first], ("-z", "execstack")),
                                 ("stack-z-noexecstack", [first, markers["exec"]], ("-z", "noexecstack"))):
        results.append((tag, link_pair(args.lccc, args.ld, inputs, tmp, tag, True, options)))
    return results


# ── Hosted links ────────────────────────────────────────────────────────────


def text_functions(elf: Elf) -> list[tuple[str, int]]:
    """(name, size) of every function in `.text`, in address order, one per
    address (aliases share it; the alphabetically first name stands)."""
    text = elf.sections.get(".text")
    if text is None:
        return []
    lo, hi = text[2], text[2] + text[3]
    by_addr: dict[int, tuple[str, int]] = {}
    for value, size, name in elf.funcs:
        if lo <= value < hi and (value not in by_addr or name < by_addr[value][0]):
            by_addr[value] = (name, size)
    return [by_addr[a] for a in sorted(by_addr)]


def compare_hosted(mine: Elf, theirs: Elf, separate_code: bool = False) -> list[str]:
    """Image layouts legitimately differ (lccc-ld places PLT/IPLT stubs and
    program headers its own way), so absolute addresses and pc-relative
    immediates are not comparable.  Every relaxation decision changes a
    function's size, though, and the input order is the same, so: function
    sizes in order, `.text` size and `.eh_frame` size must all match -- and
    `.rodata`'s, which hundreds of archive members' merged string and
    constant pools (`bfd/merge.c`) decide independently of the layout."""
    problems: list[str] = compare_headers(mine, theirs, separate_code)
    for name in (".text", ".eh_frame", ".rodata"):
        a, b = mine.sections.get(name), theirs.sections.get(name)
        if a is None or b is None:
            problems.append(f"section {name} missing ({'lccc' if a is None else 'bfd'})")
        elif a[3] != b[3]:
            problems.append(f"section {name}: size {a[3]:#x}, bfd {b[3]:#x}")
    # One PLT/IPLT stub and one jump-slot or IRELATIVE relocation per symbol
    # that needs one: a static glibc link must not give an unreferenced IFUNC
    # alias (`__libc_memcpy` beside `memcpy`) a slot of its own.
    for name in (".plt", ".rela.plt"):
        a, b = mine.sections.get(name), theirs.sections.get(name)
        if (a is None) != (b is None) or (a and b and a[3] != b[3]):
            problems.append(f"section {name}: size {a and hex(a[3])}, bfd {b and hex(b[3])}")
    fa, fb = text_functions(mine), text_functions(theirs)
    if [n for n, _ in fa] != [n for n, _ in fb]:
        problems.append(f"function order differs ({len(fa)} vs bfd {len(fb)} functions)")
    else:
        problems.extend(
            f"function {n}: size {sa:#x}, bfd {sb:#x}" for (n, sa), (_, sb) in zip(fa, fb) if sa != sb
        )
    if not fa:
        problems.append(".text has no function symbols")
    problems.extend(compare_image_metadata(mine, theirs))
    return problems


def compare_image_metadata(mine: Elf, theirs: Elf) -> list[str]:
    """What does not depend on layout must match bfd exactly: the merged
    `.riscv.attributes`, every note copied from the inputs, the build-id
    note's size (its contents hash the image), the PT_NOTE and
    PT_RISCV_ATTRIBUTES program headers, and the gp symbol bfd exports from
    a dynamic executable for ld.so (`allocate_dynrelocs`)."""
    problems: list[str] = []
    for name, (sh_type, _, _, size, body) in theirs.sections.items():
        if name != ".riscv.attributes" and sh_type != SHT_NOTE:
            continue
        sec = mine.sections.get(name)
        if sec is None:
            problems.append(f"section {name} missing")
        elif name == ".note.gnu.build-id":
            if sec[3] != size:
                problems.append(f"section {name}: size {sec[3]:#x}, bfd {size:#x}")
        elif sec[4] != body:
            problems.append(f"section {name}: contents differ from bfd's")
    for p_type, what in ((PT_NOTE, "PT_NOTE"), (PT_RISCV_ATTRIBUTES, "PT_RISCV_ATTRIBUTES")):
        a = [p for p in mine.phdrs if p[0] == p_type]
        b = [p for p in theirs.phdrs if p[0] == p_type]
        if a != b:
            problems.append(f"{what} (type, flags, filesz, memsz, align): {a}, bfd {b}")
    gp = "__global_pointer$"
    if gp in theirs.dynsyms and mine.dynsyms.get(gp) != theirs.dynsyms[gp]:
        problems.append(f"dynamic symbol {gp}: {mine.dynsyms.get(gp)}, bfd {theirs.dynsyms[gp]}")
    return problems


def frame_table(exe: Path, tmp: Path) -> list[str]:
    """`.eh_frame` as decoded by the cross readelf, minus what depends on
    where things landed: record offsets, pc values, and the pc-relative
    pointers in augmentation data (only their length is kept)."""
    dump = run([f"{TRIPLE}-readelf", "--debug-dump=frames", str(exe)], tmp)
    start = dump.find("Contents of the .eh_frame section")
    if start < 0:
        return []
    rows = []
    for line in dump[start:].splitlines()[1:]:
        head = re.match(r"^[0-9a-f]+ ([0-9a-f]+ [0-9a-f]+ (?:CIE|FDE cie=[0-9a-f]+))(?: pc=([0-9a-f]+)\.\.([0-9a-f]+))?",
                        line)
        if head:
            span = f" len={int(head[3], 16) - int(head[2], 16)}" if head[2] else ""
            rows.append(head[1] + span)
        elif "Augmentation data:" in line:
            rows.append(f"aug-data bytes={len(line.split(':', 1)[1].split())}")
        else:
            rows.append(re.sub(r" to [0-9a-f]+$", "", line))
    return rows


def driver_ld(args: argparse.Namespace) -> list[str]:
    """Driver flags that make `cc` link with the pinned ld.  collect2 looks
    for `ld` in the -B directory; `-fuse-ld=bfd` would make it look for
    `ld.bfd` instead and silently fall back to the system linker."""
    return [f"-B{args.ld.parent}/"] if args.ld.name == "ld" else [f"-fuse-ld={args.ld}"]


def hosted_pair(args: argparse.Namespace, obj: Path, tmp: Path, tag: str, static: bool) -> list[str]:
    mine, theirs = tmp / f"{tag}.lccc", tmp / f"{tag}.bfd"
    mode = ["-static"] if static else []
    # Debian's gcc passes --build-id to ld on every link; lccc's driver
    # does not, so ask for it explicitly to compare like with like.
    run([str(args.lccc), *mode, "-Wl,--build-id", str(obj), "-o", str(mine)], tmp)
    # GCC defaults to PIE (Scrt1.o); lccc's driver links position-dependent.
    run([args.cc, *mode, "-no-pie", *driver_ld(args), "-Wl,--eh-frame-hdr", str(obj), "-o", str(theirs)], tmp)
    problems = compare_hosted(read_elf(mine), read_elf(theirs)) + compare_frames(mine, theirs, tmp)
    if not static:
        problems += import_value_problems(args, obj, mine, theirs, tmp, tag)
    qemu = shutil.which(f"qemu-{TRIPLE.split('-')[0]}")
    sysroot = Path(f"/usr/{TRIPLE}")
    if qemu and sysroot.is_dir():
        outs = []
        for exe in (mine, theirs):
            done = subprocess.run([qemu, "-L", str(sysroot), str(exe)], cwd=tmp, text=True, capture_output=True,
                                  check=False, timeout=120)
            outs.append((done.returncode, done.stdout))
        if outs[0] != outs[1] or outs[0][0] != 0:
            problems.append(f"run: lccc {outs[0]!r}, bfd {outs[1]!r}")
    return problems


def import_values(exe: Path, tmp: Path) -> dict[str, bool]:
    """Undefined `.dynsym` functions -> whether st_value is non-zero."""
    out: dict[str, bool] = {}
    for line in run([f"{TRIPLE}-readelf", "--dyn-syms", "-W", str(exe)], tmp).splitlines():
        f = line.split()
        if len(f) >= 8 and f[0].endswith(":") and f[3] == "FUNC" and f[6] == "UND":
            out[f[7].split("@")[0]] = int(f[1], 16) != 0
    return out


# Relocation types through which a reference never observes an imported
# function's address in the executable: calls go through the PLT entry and
# the GOT form is filled by the dynamic linker with the canonical address.
# The `*_LO12_*` halves name their `PCREL_HI20` label, not the symbol, and
# RELAX/ALIGN/NONE carry no reference at all.
CALL_OR_GOT_RELOCS = frozenset({"R_RISCV_CALL", "R_RISCV_CALL_PLT", "R_RISCV_JAL", "R_RISCV_RVC_JUMP",
                                "R_RISCV_GOT_HI20"})
NO_REFERENCE_RELOCS = frozenset({"R_RISCV_NONE", "R_RISCV_RELAX", "R_RISCV_ALIGN", "R_RISCV_PCREL_LO12_I",
                                 "R_RISCV_PCREL_LO12_S"})


def address_taken_imports(args: argparse.Namespace, obj: Path, tmp: Path, tag: str) -> set[str]:
    """Names some input (crt files and libc_nonshared included) references
    other than by a call or through the GOT, from allocated sections: GNU ld
    re-links the program with --emit-relocs and every surviving relocation of
    an input section is classified.  The oracle is ld's own view of the
    inputs, independent of lccc-ld."""
    exe = tmp / f"{tag}.bfd-q"
    run([args.cc, "-no-pie", *driver_ld(args), "-Wl,-q", str(obj), "-o", str(exe)], tmp)
    taken: set[str] = set()
    section = ""
    for line in run([f"{TRIPLE}-readelf", "-rW", str(exe)], tmp).splitlines():
        if line.startswith("Relocation section '"):
            section = line.split("'")[1]
            continue
        f = line.split()
        if (len(f) < 5 or not f[2].startswith("R_RISCV_") or section in (".rela.dyn", ".rela.plt")
                or section.startswith(".rela.debug") or f[2] in CALL_OR_GOT_RELOCS
                or f[2] in NO_REFERENCE_RELOCS):
            continue
        taken.add(f[4].split("@")[0])
    return taken


def import_value_problems(args: argparse.Namespace, obj: Path, mine: Path, theirs: Path, tmp: Path,
                          tag: str) -> list[str]:
    """An imported function's PLT entry is exported as its st_value exactly
    when position-dependent code takes the function's address: the dynamic
    linker then makes the PLT entry canonical, so a pointer taken here equals
    the one every shared object sees (riscv_relax/pointer_equality.c runs it).

    GNU ld 2.47 exports it for EVERY import with a PLT entry.  For a
    call-only import that is not merely redundant but harmful: libgcc's
    unwinder and dladdr then resolve the import to the executable's PLT stub
    (tests/regression/check_cfi_unwind.sh fails with GNU ld itself), so
    lccc-ld follows x86-64 GNU ld and LLD, which export it only when pointer
    equality needs it.  The contract is therefore checked against the inputs'
    relocations, and GNU ld must agree on every address-taken import."""
    a, b = import_values(mine, tmp), import_values(theirs, tmp)
    taken = address_taken_imports(args, obj, tmp, tag)
    problems = [f"dynsym {n}: imported by {'lccc' if n in a else 'bfd'} only" for n in sorted(a.keys() ^ b.keys())]
    for n in sorted(a.keys() & b.keys()):
        want = n in taken
        if a[n] != want:
            problems.append(f"dynsym {n}: st_value {'set' if a[n] else 'zero'}, want "
                            f"{'set (address taken)' if want else 'zero (call/GOT references only)'}")
        if want and not b[n]:
            problems.append(f"dynsym {n}: bfd leaves an address-taken import's st_value zero")
    if tag.startswith("ptreq") and not (a.get("strlen") and "strlen" in taken):
        problems.append("ptreq: strlen must be an address-taken import with a PLT st_value (oracle vacuous?)")
    return problems


EXECSTACK_WARNING = "requires executable stack (because the .note.GNU-stack section is executable)"


def execstack_warning_problems(args: argparse.Namespace, obj: Path, tmp: Path, static: bool) -> list[str]:
    """lccc-ld reports an executable-marked input like GNU ld 2.47 does, and
    `--no-warn-execstack` silences it."""
    mode = ["-static"] if static else []
    problems = []
    for extra, want in (([], True), (["-Wl,--no-warn-execstack"], False)):
        done = subprocess.run([str(args.lccc), *mode, *extra, str(obj), "-o", str(tmp / "warn.lccc")], cwd=tmp,
                              text=True, capture_output=True, check=False, timeout=120)
        line = f"lccc-ld: warning: {obj}: {EXECSTACK_WARNING}"
        if done.returncode != 0 or (line in done.stderr.splitlines()) != want:
            problems.append(f"execstack warning {' '.join(extra) or '(default)'}: rc {done.returncode}, "
                            f"stderr {done.stderr.strip()!r}, want {'it' if want else 'none'}")
    return problems


def compare_frames(mine: Path, theirs: Path, tmp: Path) -> list[str]:
    fa, fb = frame_table(mine, tmp), frame_table(theirs, tmp)
    if not fb:
        return ["bfd output has no .eh_frame to compare"]
    if fa != fb:
        at = next((i for i, (a, b) in enumerate(zip(fa, fb)) if a != b), min(len(fa), len(fb)))
        return [f".eh_frame decodes differently at row {at}: "
                f"{fa[at] if at < len(fa) else '<end>'!r}, bfd {fb[at] if at < len(fb) else '<end>'!r}"]
    return []


def shared_pair(args: argparse.Namespace, obj: Path, tmp: Path, tag: str) -> list[str]:
    """A shared object from the hosted fixture compiled -fPIC: GNU ld relaxes
    those too, minus the rewrites a position-independent image cannot use."""
    mine, theirs = tmp / f"{tag}.lccc.so", tmp / f"{tag}.bfd.so"
    run([str(args.lccc), "-shared", "-Wl,--build-id", str(obj), "-o", str(mine)], tmp)
    run([args.cc, "-shared", *driver_ld(args), "-Wl,--eh-frame-hdr", str(obj), "-o", str(theirs)], tmp)
    return compare_hosted(read_elf(mine), read_elf(theirs), separate_code=True) + compare_frames(
        mine, theirs, tmp)


def shared_import_problems(args: argparse.Namespace, tmp: Path) -> list[str]:
    """The shared object must actually work: each linker's `libgreet.so` is
    loaded by the same bfd-linked program under qemu and must print what
    GNU ld's prints. Comparing sizes alone missed a PLT that vanished after
    relaxation (every import call became `jal 0`) and calls to the
    library's own preemptible `hook` that bypassed interposition. By
    default the program's `hook` (2) wins; `-Bsymbolic` and
    `-Bsymbolic-functions` bind the library's (1)."""
    qemu = shutil.which(f"qemu-{TRIPLE.split('-')[0]}")
    sysroot = Path(f"/usr/{TRIPLE}")
    if not qemu or not sysroot.is_dir():
        return ["qemu or the target sysroot is missing"] if args.require else []
    src = ROOT / "tests/regression/riscv_relax/shared_import.c"
    lib, main = tmp / "shared-import.o", tmp / "shared-import-main.o"
    run([args.cc, "-O2", "-fPIC", "-c", str(src), "-o", str(lib)], tmp)
    run([args.cc, "-O2", "-DSHARED_IMPORT_MAIN", "-c", str(src), "-o", str(main)], tmp)
    problems = []
    for mode, hook in (("", 2), ("-Bsymbolic", 1), ("-Bsymbolic-functions", 1)):
        outs, flags = [], []
        for who in ("lccc", "bfd"):
            d = tmp / f"shared-import{mode}-{who}"
            d.mkdir(exist_ok=True)
            so = d / "libgreet.so"
            # New dtags explicitly: lccc-ld defaults to them (as distro
            # linkers do), the pinned GNU ld's configure default does not,
            # and they decide whether -Bsymbolic also writes DT_FLAGS.
            extra = ["-Wl,--enable-new-dtags", *([f"-Wl,{mode}"] if mode else [])]
            if who == "lccc":
                run([str(args.lccc), "-shared", *extra, str(lib), "-o", str(so)], tmp)
            else:
                run([args.cc, "-shared", *driver_ld(args), *extra, str(lib), "-o", str(so)], tmp)
            dyn = run([f"{TRIPLE}-readelf", "-dW", str(so)], tmp)
            flags.append(sorted(t for t in ("(SYMBOLIC)", "(FLAGS)") if t in dyn))
            exe = d / "main"
            run([args.cc, *driver_ld(args), str(main), f"-L{d}", "-lgreet", "-o", str(exe)], tmp)
            done = subprocess.run([qemu, "-L", str(sysroot), "-E", f"LD_LIBRARY_PATH={d}", str(exe)], cwd=tmp,
                                  text=True, capture_output=True, check=False, timeout=120)
            outs.append((done.returncode, done.stdout))
        label = mode or "default"
        if outs[0] != outs[1] or outs[0] != (0, f"hello world 5 {hook}\n"):
            problems.append(f"{label} run: lccc {outs[0]!r}, bfd {outs[1]!r}")
        if flags[0] != flags[1]:
            problems.append(f"{label} dynamic tags: lccc {flags[0]}, bfd {flags[1]}")
    return problems


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--lccc", type=Path, default=ROOT / "target/fastbuild/lccc-riscv")
    ap.add_argument("--ld", type=Path, default=DEFAULT_LD)
    ap.add_argument("--cc", default=f"{TRIPLE}-gcc")
    ap.add_argument("--only", help="run only cases whose name contains this")
    ap.add_argument("--keep", type=Path, help="keep the work directory here")
    ap.add_argument("--require", action="store_true",
                    help="a missing tool (pinned ld, cross gcc, static libc, qemu) fails instead of skipping (CI)")
    args = ap.parse_args()
    # Every tool runs with its cwd in the work directory, so a relative tool
    # path (ci_local.sh passes `target/fastbuild/lccc-riscv`) is anchored to
    # the caller's directory here; a bare name stays a PATH lookup.
    for name in ("lccc", "ld"):
        tool = getattr(args, name)
        if not tool.is_absolute() and len(tool.parts) > 1:
            setattr(args, name, tool.resolve())

    def skip(why: str) -> int:
        print(f"{'FAIL' if args.require else 'SKIP'}: {why}")
        return 1 if args.require else 0

    for tool, what in ((args.lccc, "lccc-riscv"), (args.ld, "bfd ld 2.47")):
        if not Path(tool).is_file():
            return skip(f"{what} not found at {tool}")
    if shutil.which(args.cc) is None:
        return skip(f"{args.cc} not available")
    version = run([str(args.ld), "--version"], ROOT).splitlines()[0]
    if " 2.47" not in version:
        print(f"FAIL: {args.ld} is not GNU ld 2.47: {version}")
        return 1

    tmp = Path(tempfile.mkdtemp(prefix="riscv-relax-bfd-"))
    if args.keep:
        args.keep.mkdir(parents=True, exist_ok=True)
        tmp = args.keep
    jobs: list[tuple[str, list[Path]]] = []
    for name, asm in generated_cases().items():
        for rvc, march in MARCH.items():
            src = tmp / f"{name}-{rvc}.s"
            src.write_text(asm)
            obj = src.with_suffix(".o")
            run([args.cc, f"-march={march}", "-mabi=lp64d", "-c", str(src), "-o", str(obj)], tmp)
            jobs.append((f"{name}-{rvc}", [obj]))
    fixture = ROOT / "tests/regression/riscv_relax/bfd_mix.c"
    for opt in ("-O0", "-O2", "-Os"):
        for rvc, march in MARCH.items():
            for dbg in ("", "-g"):
                tag = f"bfd_mix{opt}{dbg}-{rvc}"
                obj = tmp / f"{tag}.o"
                run(
                    [args.cc, opt, *([dbg] if dbg else []), f"-march={march}", "-mabi=lp64d",
                     "-ffreestanding", "-fno-pic", "-fno-reorder-functions", "-fno-asynchronous-unwind-tables",
                     "-c", str(fixture), "-o", str(obj)],
                    tmp,
                )
                jobs.append((tag, [obj]))
    # SHF_MERGE merging of allocated inputs across objects: strings (with tail
    # merging and an 8-aligned group), .rodata.cst16 / .srodata.cst8
    # constants, named symbols inside merged inputs, a past-the-end
    # reference, and a string input with relocations (left unmerged).
    for rvc, march in MARCH.items():
        for dbg in ("", "-g"):
            tag = f"strmerge-O2{dbg}-{rvc}"
            objs = []
            for name in ("strmerge_a.c", "strmerge_b.c", "strmerge_c.s"):
                obj = tmp / f"{tag}-{Path(name).stem}.o"
                run([args.cc, "-O2", *([dbg] if dbg else []), f"-march={march}", "-mabi=lp64d", "-ffreestanding",
                     "-fno-pic", "-fno-asynchronous-unwind-tables",
                     "-c", str(ROOT / "tests/regression/riscv_relax" / name), "-o", str(obj)], tmp)
                objs.append(obj)
            jobs.append((tag, objs))
    hosted: list[tuple[str, Path]] = []
    shared: list[tuple[str, Path]] = []
    hosted_src = ROOT / "tests/regression/riscv_relax/hosted_unwind.c"
    if shutil.which(f"{TRIPLE}-readelf") is None:
        print(f"FAIL: {TRIPLE}-readelf (cross binutils) not found next to {args.cc}")
        return 1
    if run([args.cc, "-print-file-name=libc.a"], tmp).strip() == "libc.a":
        if args.require:
            print(f"FAIL: {args.cc} has no static libc for the hosted cases")
            return 1
        print(f"SKIP hosted: {args.cc} has no static libc")
    elif args.require and shutil.which(f"qemu-{TRIPLE.split('-')[0]}") is None:
        print(f"FAIL: qemu-{TRIPLE.split('-')[0]} not found; the hosted cases must run their programs")
        return 1
    elif " 2.47" not in (used := run([args.cc, *driver_ld(args), "-nostdlib", "-Wl,-v", "-o", "/dev/null"], tmp)):
        print(f"FAIL: {args.cc} {' '.join(driver_ld(args))} does not link with GNU ld 2.47: {used.strip()}")
        return 1
    else:
        for opt in ("-O2", "-Os"):
            for dbg in ("", "-g"):
                tag = f"hosted{opt}{dbg}"
                obj = tmp / f"{tag}.o"
                run([args.cc, opt, *([dbg] if dbg else []), "-c", str(hosted_src), "-o", str(obj)], tmp)
                hosted.append((tag, obj))
                pic = tmp / f"{tag}-pic.o"
                run([args.cc, opt, *([dbg] if dbg else []), "-fPIC", "-c", str(hosted_src), "-o", str(pic)], tmp)
                shared.append((f"{tag}-shared", pic))
        # A stack trampoline: the program only runs if the linker honours the
        # executable `.note.GNU-stack` marker (PT_GNU_STACK RWX).
        tramp = tmp / "trampoline-O2.o"
        run([args.cc, "-O2", "-c", str(ROOT / "tests/regression/riscv_relax/nested_trampoline.c"), "-o", str(tramp)],
            tmp)
        hosted.append(("trampoline-O2", tramp))
        # Function-pointer equality across objects: a dynamic-only property.
        ptreq = tmp / "ptreq-O2.o"
        run([args.cc, "-O2", "-fno-pic", "-c", str(ROOT / "tests/regression/riscv_relax/pointer_equality.c"), "-o",
             str(ptreq)], tmp)
        hosted.append(("ptreq-O2", ptreq))

    failed = passed = 0
    for tag, objs in jobs:
        if args.only and args.only not in tag:
            continue
        for relax in (True, False):
            label = f"{tag} {'relax' if relax else 'no-relax'}"
            try:
                problems = link_pair(args.lccc, args.ld, objs, tmp, f"{tag}-{int(relax)}", relax)
            except GateFailure as exc:
                problems = [str(exc).strip()]
            if problems:
                failed += 1
                print(f"FAIL {label}:")
                for p in problems[:8]:
                    print(f"    {p}")
            else:
                passed += 1
    if not args.only or any(k in args.only for k in ("attrs", "build-id", "stack")):
        try:
            attr_objs = attribute_objects(args.cc, tmp)
            attr_results = [*attribute_cases(args.lccc, args.ld, attr_objs, tmp),
                            *build_id_cases(args, attr_objs, tmp),
                            *stack_cases(args, attr_objs, tmp)]
        except GateFailure as exc:
            attr_results = [("attrs", [str(exc).strip()])]
        for label, problems in attr_results:
            if problems:
                failed += 1
                print(f"FAIL {label}:")
                for p in problems[:8]:
                    print(f"    {p}")
            else:
                passed += 1
    for tag, obj in hosted:
        if args.only and args.only not in tag:
            continue
        for static in (True, False):
            if static and tag.startswith("ptreq"):
                continue
            label = f"{tag} {'static' if static else 'dynamic'}"
            try:
                problems = hosted_pair(args, obj, tmp, f"{tag}-{'s' if static else 'd'}", static)
                if tag.startswith("trampoline"):
                    problems += execstack_warning_problems(args, obj, tmp, static)
            except GateFailure as exc:
                problems = [str(exc).strip()]
            if problems:
                failed += 1
                print(f"FAIL {label}:")
                for p in problems[:8]:
                    print(f"    {p}")
            else:
                passed += 1
    for tag, obj in shared:
        if args.only and args.only not in tag:
            continue
        try:
            problems = shared_pair(args, obj, tmp, tag)
        except GateFailure as exc:
            problems = [str(exc).strip()]
        if problems:
            failed += 1
            print(f"FAIL {tag}:")
            for p in problems[:8]:
                print(f"    {p}")
        else:
            passed += 1
    if shared and (not args.only or args.only in "shared-import"):
        try:
            problems = shared_import_problems(args, tmp)
        except GateFailure as exc:
            problems = [str(exc).strip()]
        if problems:
            failed += 1
            print("FAIL shared-import:")
            for p in problems[:8]:
                print(f"    {p}")
        else:
            passed += 1
    print(f"riscv relax vs GNU ld 2.47: {passed} passed, {failed} failed")
    if not args.keep:
        shutil.rmtree(tmp, ignore_errors=True)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
