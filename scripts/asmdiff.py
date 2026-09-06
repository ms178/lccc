#!/usr/bin/env python3
"""Whole-object differential between LCCC's integrated assembler and GNU as.

Where `insndiff.py` compares ONE instruction at a time, this compares whole
translation units: every allocated section's bytes, the full relocation table,
and the symbol table. That matters because the three most damaging assembler
bug classes are invisible in a single-instruction view:

  * relocation errors     — link-time corruption, never seen in .text bytes
  * symbol table errors   — wrong binding/type/size, breaks linking or ABI
  * layout errors         — relaxation and alignment interacting across a
                            whole section, which is exactly where jump
                            relaxation and PGO block alignment collide

GNU as is the oracle. LCCC must either agree exactly, or reject exactly what
GNU as rejects: silently accepting a malformed instruction means emitting
bytes for something the programmer did not write.

Corpus format (`*.casefile`): a sequence of

    ;;; case-name [flags]
    <assembly>

Flags:
    reject   both assemblers must REFUSE this input
    nosym    compare bytes and relocations but not the symbol table

Usage:
    scripts/asmdiff.py                       # all corpora under tests/asm-diff
    scripts/asmdiff.py path/to/x.casefile -v
"""
from __future__ import annotations

import argparse
import concurrent.futures
import os
import re
import shutil
import subprocess
import sys
import tempfile
from dataclasses import dataclass, field
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_CORPUS = REPO_ROOT / "tests" / "asm-diff"


# ─── Minimal ELF64 reader (no third-party dependency) ─────────────────────

def _u(b: bytes, off: int, n: int) -> int:
    return int.from_bytes(b[off:off + n], "little")


@dataclass
class ElfImage:
    content: dict[str, bytes] = field(default_factory=dict)
    relocs: dict[str, list[tuple]] = field(default_factory=dict)
    symbols: list[tuple] = field(default_factory=list)
    defined: dict[str, tuple[str, int]] = field(default_factory=dict)


def read_elf(path: Path) -> ElfImage:
    b = path.read_bytes()
    if b[:4] != b"\x7fELF" or b[4] != 2:
        raise ValueError(f"{path}: not an ELF64 object")
    e_shoff, e_shentsize = _u(b, 0x28, 8), _u(b, 0x3A, 2)
    e_shnum, e_shstrndx = _u(b, 0x3C, 2), _u(b, 0x3E, 2)

    sh = []
    for i in range(e_shnum):
        o = e_shoff + i * e_shentsize
        sh.append(dict(
            name=_u(b, o, 4), type=_u(b, o + 4, 4), flags=_u(b, o + 8, 8),
            off=_u(b, o + 0x18, 8), size=_u(b, o + 0x20, 8),
            link=_u(b, o + 0x28, 4), info=_u(b, o + 0x2C, 4)))

    def raw(i):
        s = sh[i]
        return b"" if s["type"] == 8 else b[s["off"]: s["off"] + s["size"]]

    shstr = raw(e_shstrndx)

    def nm(tab: bytes, off: int) -> str:
        end = tab.find(b"\0", off)
        return tab[off:end].decode("utf-8", "replace")

    names = [nm(shstr, s["name"]) for s in sh]
    img = ElfImage()

    SHT_SYMTAB, SHT_STRTAB, SHT_RELA, SHT_REL = 2, 3, 4, 9
    syms: list[tuple] = []
    for i, s in enumerate(sh):
        if s["type"] != SHT_SYMTAB:
            continue
        strt, d = raw(s["link"]), raw(i)
        for k in range(len(d) // 24):
            o = k * 24
            syms.append((nm(strt, _u(d, o, 4)), d[o + 4] >> 4, d[o + 4] & 0xF,
                         _u(d, o + 6, 2), _u(d, o + 8, 8), _u(d, o + 16, 8)))
        break

    def sname(ix: int) -> str:
        return {0: "UNDEF", 0xFFF1: "ABS", 0xFFF2: "COMMON"}.get(
            ix, names[ix] if ix < len(names) else f"SHN{ix}")

    img.symbols = [(n, bi, ty, sname(sx), v, sz)
                   for (n, bi, ty, sx, v, sz) in syms]
    for (n, _bi, ty, sx, v, _sz) in syms:
        if n and ty not in (3, 4) and sx not in (0, 0xFFF1, 0xFFF2):
            img.defined.setdefault(n, (sname(sx), v))

    for i, s in enumerate(sh):
        if s["type"] in (SHT_SYMTAB, SHT_STRTAB, SHT_RELA, SHT_REL):
            continue
        if names[i] in ("", ".shstrtab", ".strtab", ".symtab"):
            continue
        img.content[names[i]] = raw(i)

    for i, s in enumerate(sh):
        if s["type"] not in (SHT_RELA, SHT_REL):
            continue
        tgt = names[s["info"]] if s["info"] < len(names) else f"SEC{s['info']}"
        step, d = (24 if s["type"] == SHT_RELA else 16), raw(i)
        out = []
        for k in range(len(d) // step):
            o = k * step
            info = _u(d, o + 8, 8)
            add = 0
            if s["type"] == SHT_RELA:
                add = _u(d, o + 16, 8)
                if add >= 1 << 63:
                    add -= 1 << 64
            si = info >> 32
            sn = syms[si][0] if si < len(syms) else f"SYM{si}"
            if not sn and si < len(syms):
                sn = "@" + sname(syms[si][3])
            out.append((_u(d, o, 8), info & 0xFFFFFFFF, sn, add))
        img.relocs.setdefault(tgt, []).extend(sorted(out))
    return img


# ─── Comparison ───────────────────────────────────────────────────────────

def _interesting(name: str) -> bool:
    if name.startswith((".debug", ".note")):
        return False
    return name not in (".comment", ".eh_frame", ".eh_frame_hdr", ".group",
                        ".llvm_addrsig")


def normalize_relocs(img: ElfImage) -> dict[str, list[tuple]]:
    """Reduce each relocation to (offset, type, section-or-extern, addend).

    A reference to a DEFINED LOCAL symbol may legitimately be encoded either
    against that symbol with addend A, or against its section symbol with
    addend (sym.value + A). GAS picks the latter, LCCC the former; both link
    identically. Canonicalizing removes that cosmetic difference while still
    catching a genuinely wrong target or addend.
    """
    out: dict[str, list[tuple]] = {}
    for sec, rels in img.relocs.items():
        norm = []
        for (off, typ, sym, add) in rels:
            if sym.startswith("@"):
                norm.append((off, typ, sym[1:], add))
            elif sym in img.defined:
                s, v = img.defined[sym]
                norm.append((off, typ, s, v + add))
            else:
                norm.append((off, typ, sym, add))
        out[sec] = sorted(norm)
    return out


def hexdiff(a: bytes, b: bytes, limit: int = 8) -> str:
    out, shown = [], 0
    for i in range(max(len(a), len(b))):
        x = a[i] if i < len(a) else None
        y = b[i] if i < len(b) else None
        if x != y:
            out.append(f"    @{i:#06x}: lccc={'--' if x is None else f'{x:02x}'} "
                       f"gas={'--' if y is None else f'{y:02x}'}")
            shown += 1
            if shown >= limit:
                out.append(f"    ... (len lccc={len(a)} gas={len(b)})")
                break
    return "\n".join(out)


_SCALE1 = re.compile(r"\(,%([a-z0-9]+),1\)")
_ZERODISP = re.compile(r"(?<![0-9a-fx])0x0\(")


# VEX 3-operand instructions whose two SOURCE operands may be exchanged with
# no change to the result bits. Integer and bitwise only: FP add/mul propagate
# SRC1's NaN payload, so exchanging them is observable and is NOT done.
_COMMUTATIVE_VEX = {
    "vpand", "vpor", "vpxor", "vpaddb", "vpaddw", "vpaddd", "vpaddq",
    "vpmullw", "vpaddsb", "vpaddsw", "vpaddusb", "vpaddusw",
    "vpminub", "vpmaxub", "vpminsw", "vpmaxsw", "vpavgb", "vpavgw",
    "vpmulhw", "vpmulhuw", "vpcmpeqb", "vpcmpeqw", "vpcmpeqd",
    "vandps", "vandpd", "vorps", "vorpd", "vxorps", "vxorpd",
}
_VEX3 = re.compile(r"^(v\S+)\s+(%\S+),(%\S+),(%\S+)$")


# A 32-bit register write zero-extends into its 64-bit parent, so
# `mov $0xffffffff,%eax` and `movabs $0xffffffff,%rax` leave the same value in
# %rax -- the first in 5 bytes, the second in 10. Canonicalise the pair so the
# semantic comparison sees them as equal.
_GP32_TO_64 = {
    "eax": "rax", "ebx": "rbx", "ecx": "rcx", "edx": "rdx",
    "esi": "rsi", "edi": "rdi", "ebp": "rbp", "esp": "rsp",
    **{f"r{n}d": f"r{n}" for n in range(8, 16)},
}
_MOV_IMM = re.compile(r"^(movabs|mov)\s+\$(0x[0-9a-f]+|\d+),%(\w+)$")


def _canon_mov_imm(insn: str) -> str:
    """Normalise a zero-extending 32-bit immediate load to its 64-bit meaning."""
    m = _MOV_IMM.match(insn.strip())
    if not m:
        return insn
    val = int(m.group(2), 0)
    reg = m.group(3)
    if reg in _GP32_TO_64 and 0 <= val <= 0xFFFFFFFF:
        return f"mov ${val:#x},%{_GP32_TO_64[reg]}"
    return f"mov ${val:#x},%{reg}"


def _canon_commutative(insn: str) -> str:
    """Put the two sources of a commutative VEX op in a canonical order.

    An assembler may exchange them to reach the shorter 2-byte VEX prefix, so
    the disassembly differs textually while denoting the same operation.
    """
    m = _VEX3.match(insn.strip())
    if not m or m.group(1) not in _COMMUTATIVE_VEX:
        return insn
    a, b = sorted((m.group(2), m.group(3)))
    return f"{m.group(1)} {a},{b},{m.group(4)}"


def _norm_disasm(text: str) -> str:
    """Normalise a disassembly listing for semantic comparison.

    Two encodings are equivalent when they decode to the same instruction with
    the same effective address. The two spellings that differ purely by
    encoding choice are a redundant scale-1 index (`-0x1(,%rdi,1)` is the same
    address as `-0x1(%rdi)`) and an explicit zero displacement.
    """
    out = []
    for line in text.splitlines():
        m = re.match(r"^\s+[0-9a-f]+:\s+((?:[0-9a-f]{2} )+)\s*\t(.*)$", line)
        if not m:
            continue
        insn = m.group(2).strip()
        insn = _SCALE1.sub(r"(%\1)", insn)
        insn = _ZERODISP.sub("(", insn)
        insn = re.sub(r"\s+", " ", insn)
        insn = _canon_commutative(insn)
        insn = _canon_mov_imm(insn)
        # Drop the trailing branch-target comment objdump appends.
        insn = insn.split("#")[0].strip()
        out.append(insn)
    return "\n".join(out)


def semantically_equal(a_obj: Path, b_obj: Path, objdump: str) -> bool:
    """True when two objects disassemble to the same instruction sequence."""
    try:
        ta = subprocess.run([objdump, "-d", str(a_obj)],
                            capture_output=True, text=True, timeout=120).stdout
        tb = subprocess.run([objdump, "-d", str(b_obj)],
                            capture_output=True, text=True, timeout=120).stdout
    except (OSError, subprocess.SubprocessError):
        return False
    return _norm_disasm(ta) == _norm_disasm(tb)


def compare(lo: ElfImage, go: ElfImage, *, check_symbols: bool) -> list[str]:
    errs: list[str] = []
    names = sorted({n for n in lo.content if _interesting(n)} |
                   {n for n in go.content if _interesting(n)})
    for n in names:
        a, b = lo.content.get(n), go.content.get(n)
        # A zero-length section carries no code or data; whether an assembler
        # materializes its header is cosmetic.
        if a is None:
            if b:
                errs.append(f"section {n}: missing in LCCC ({len(b)} bytes in gas)")
            continue
        if b is None:
            if a:
                errs.append(f"section {n}: extra in LCCC ({len(a)} bytes)")
            continue
        if a != b:
            errs.append(f"section {n}: {len(a)} vs {len(b)} bytes differ\n"
                        + hexdiff(a, b))

    nla, nlb = normalize_relocs(lo), normalize_relocs(go)
    for n in sorted(set(nla) | set(nlb)):
        if not _interesting(n):
            continue
        ra, rb = nla.get(n, []), nlb.get(n, [])
        if ra != rb:
            errs.append(f"relocs {n}:\n  only-lccc={[x for x in ra if x not in rb]!r}"
                        f"\n  only-gas ={[x for x in rb if x not in ra]!r}")

    if check_symbols:
        sa = sorted(s for s in lo.symbols if s[2] != 4 and s[0])
        sb = sorted(s for s in go.symbols if s[2] != 4 and s[0])
        if sa != sb:
            errs.append(f"symbols:\n  only-lccc={[x for x in sa if x not in sb]!r}"
                        f"\n  only-gas ={[x for x in sb if x not in sa]!r}")
    return errs


# ─── Cases ────────────────────────────────────────────────────────────────

@dataclass
class Case:
    name: str
    text: str
    mode: str = "accept"
    check_symbols: bool = True
    # When set, a section may differ from GNU as provided LCCC's encoding is
    # strictly shorter AND decodes to the same instruction sequence. This is
    # how a deliberate encoding win is asserted without weakening the check
    # into "any difference is fine".
    allow_better: bool = False


HEADER = re.compile(r"^;;;\s*(\S+)\s*(.*)$")


def load_cases(path: Path) -> list[Case]:
    cases: list[Case] = []
    cur: Case | None = None
    buf: list[str] = []
    for line in path.read_text().splitlines():
        m = HEADER.match(line)
        if m:
            if cur:
                cur.text = "\n".join(buf) + "\n"
                cases.append(cur)
            flags = m.group(2).split()
            cur = Case(f"{path.stem}/{m.group(1)}", "",
                       "reject" if "reject" in flags else "accept",
                       "nosym" not in flags,
                       "betterok" in flags)
            buf = []
        elif cur is not None:
            buf.append(line)
    if cur:
        cur.text = "\n".join(buf) + "\n"
        cases.append(cur)
    return cases


def run_case(c: Case, lccc: str, gas: str, wd: str, verbose: bool):
    tag = re.sub(r"[^A-Za-z0-9_.-]", "_", c.name)
    src = Path(wd) / f"{tag}.s"
    src.write_text(c.text)
    lo, go = Path(wd) / f"{tag}.l.o", Path(wd) / f"{tag}.g.o"

    r1 = subprocess.run([lccc, "-c", str(src), "-o", str(lo)],
                        capture_output=True, text=True, timeout=180)
    r2 = subprocess.run([gas, "--64", "-o", str(go), str(src)],
                        capture_output=True, text=True, timeout=180)

    if c.mode == "reject":
        if r2.returncode == 0:
            return (c.name, False, "ORACLE BROKEN: GNU as accepted a 'reject' case")
        if r1.returncode == 0:
            return (c.name, False, "FALSE-ACCEPT: LCCC accepted invalid input")
        return (c.name, True, "")

    if r2.returncode != 0:
        return (c.name, False, "ORACLE BROKEN: GNU as rejected an 'accept' case:\n"
                + r2.stderr.strip()[:600])
    if r1.returncode != 0:
        return (c.name, False, "REJECTS-VALID: LCCC failed to assemble:\n"
                + (r1.stderr or r1.stdout).strip()[:900])
    try:
        errs = compare(read_elf(lo), read_elf(go), check_symbols=c.check_symbols)
    except Exception as exc:  # noqa: BLE001
        return (c.name, False, f"ELF parse error: {exc}")

    if errs and c.allow_better:
        # Accept the difference only if every byte-level complaint is LCCC
        # being smaller, and the two objects still decode identically.
        objdump = os.environ.get("LCCC_OBJDUMP") or (
            shutil.which("objdump") or "objdump")
        size_l = len(read_elf(lo).content.get(".text", b""))
        size_g = len(read_elf(go).content.get(".text", b""))
        only_size = all("bytes differ" in e for e in errs)
        if only_size and size_l < size_g and semantically_equal(lo, go, objdump):
            return (c.name, True, f"BETTER: {size_l}B vs gas {size_g}B, "
                                  f"same disassembly")
    if errs:
        body = "\n".join(errs)
        return (c.name, False, body if verbose else body[:1800])
    return (c.name, True, "")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("cases", nargs="*", type=Path)
    ap.add_argument("--lccc", default=str(REPO_ROOT / "target" / "release" / "lccc"))
    ap.add_argument("--as", dest="gas", default=os.environ.get("LCCC_GAS", "as"))
    ap.add_argument("--jobs", type=int, default=os.cpu_count() or 2)
    ap.add_argument("-v", "--verbose", action="store_true")
    ap.add_argument("--max-report", type=int, default=30)
    args = ap.parse_args()

    if not (shutil.which(args.gas) or Path(args.gas).exists()):
        print(f"error: assembler oracle {args.gas!r} not found", file=sys.stderr)
        return 2
    if not Path(args.lccc).exists():
        print(f"error: lccc not built at {args.lccc!r}", file=sys.stderr)
        return 2

    files = args.cases or sorted(DEFAULT_CORPUS.glob("*.casefile"))
    cases = [c for f in files for c in load_cases(f)]
    if not cases:
        print("error: no cases found", file=sys.stderr)
        return 2

    passed, failures = 0, []
    with tempfile.TemporaryDirectory(prefix="asmdiff-") as wd:
        with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as ex:
            for fut in concurrent.futures.as_completed(
                    [ex.submit(run_case, c, args.lccc, args.gas, wd, args.verbose)
                     for c in cases]):
                name, ok, msg = fut.result()
                if ok:
                    passed += 1
                else:
                    failures.append((name, msg))

    for name, msg in sorted(failures)[: args.max_report]:
        print(f"FAIL {name}\n{msg}\n")
    if len(failures) > args.max_report:
        print(f"... and {len(failures) - args.max_report} more")

    print(f"=== asm-diff: {passed} passed, {len(failures)} failed "
          f"({len(cases)} cases, oracle={args.gas}) ===")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
