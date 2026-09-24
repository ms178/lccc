#!/usr/bin/env python3
"""Vector-encoding optimization corpus: shape-deduped VEX/EVEX test corpus.

`encdiff.py` judges LCCC against every reachable oracle (local GAS 2.47 plus
Clang 23.1 / GCC 16.2 / ICX / ICC over Compiler Explorer) and reports the
SHORTEST legal encoding, not just GAS agreement. Feeding it the raw GAS
testsuite would burn thousands of redundant Godbolt calls on instructions
that differ only in register numbers with identical encoding length.

This tool extracts the VEX/EVEX instruction lines (v*, k*, gf2p8*) from the
binutils testsuite, normalizes each operand to its encoding-length shape
(register class low/mid/high, memory addressing mode, displacement width,
decorators), keeps one concrete representative per (mnemonic, shape) key,
filters to GAS-64-accepted lines, and writes the corpus for encdiff.

Usage:
  scripts/vec_opt.py [--jobs 2] [--out /tmp/vec.corpus]
  scripts/encdiff.py --file /tmp/vec.corpus --lccc ./target/fastbuild/lccc

Operand-shape classes (anything that can change the emitted byte LENGTH):
  xmmL/xmmM/xmmH   xmm0-7 / xmm8-15 / xmm16-31 (VEX2-vs-VEX3, EVEX ext bits)
  ymm/zmm likewise; r32L/r32M, r64L/r64M for GPRs; kR for k0-7
  mem shapes: base class + rsp/rbp/rip specials + index*scale + disp width
  decorators: +k (mask), +z (zeroing), +bcst ({1toN}), +sae (sae/er)
"""
import argparse
import concurrent.futures as cf
import os
import re
import subprocess
import sys
import tempfile
from collections import defaultdict
from pathlib import Path

GAS = os.environ.get("LCCC_GAS", "as")
TS = Path(os.environ.get(
    "LCCC_TESTSUITE",
    "/home/user/tools/testsuite/binutils-2.47/gas/testsuite/gas/i386"))

MNEM = re.compile(r"^\s*([a-z][a-z0-9]*)\b", re.I)
LABEL = re.compile(r"^\s*[.\w$]+:\s*(.*)$")
DIRECTIVE = re.compile(r"^\s*\.")
MACRO_START = re.compile(r"^\s*\.macro\b", re.I)
MACRO_END = re.compile(r"^\s*\.endm\b", re.I)
INTEL = re.compile(r"^\s*\.intel_syntax\b", re.I)

GP32 = {"eax": 0, "ecx": 1, "edx": 2, "ebx": 3, "esp": 4, "ebp": 5,
        "esi": 6, "edi": 7, "r8d": 8, "r9d": 9, "r10d": 10, "r11d": 11,
        "r12d": 12, "r13d": 13, "r14d": 14, "r15d": 15}
GP64 = {"rax": 0, "rcx": 1, "rdx": 2, "rbx": 3, "rsp": 4, "rbp": 5,
        "rsi": 6, "rdi": 7, "r8": 8, "r9": 9, "r10": 10, "r11": 11,
        "r12": 12, "r13": 13, "r14": 14, "r15": 15}


def vec_mnemonic(mn):
    """True for VEX/EVEX-encoded (or XOP/coprocessor-adjacent) mnemonics."""
    if mn in ("verr", "verw"):
        return False
    if mn.startswith("v"):
        return True
    if re.match(r"^k(add|and|mov|not|or|shift|test|unpck|xnor|xor)", mn):
        return True
    return mn.startswith("gf2p8")


def extract_lines(path):
    # Same candidate rules as the ISA gap sweep: AT&T only (stop at
    # .intel_syntax), macro bodies skipped, insn must carry a register
    # sigil / immediate / indirect star.
    PSEUDO = {"byte", "word", "long", "quad", "ascii", "asciz", "string",
              "space", "skip", "align", "p2align", "set", "equ", "org",
              "section", "text", "data", "bss", "globl", "global",
              "extern", "type", "size", "comm", "lcomm", "macro"}
    out = []
    in_macro = False
    try:
        text = path.read_text(errors="replace")
    except OSError:
        return out
    for i, raw in enumerate(text.splitlines(), 1):
        if MACRO_START.match(raw):
            in_macro = True
            continue
        if MACRO_END.match(raw):
            in_macro = False
            continue
        if in_macro:
            continue
        if INTEL.match(raw):
            break  # rest of file is Intel syntax
        line = raw.split("#", 1)[0].rstrip()
        if not line.strip():
            continue
        if DIRECTIVE.match(line):
            continue
        m = LABEL.match(line)
        if m:
            line = m.group(1)
            if not line.strip():
                continue
        m = MNEM.match(line)
        if not m:
            continue
        mn = m.group(1).lower()
        if mn in PSEUDO:
            continue
        if "%" not in line and "$" not in line and "*" not in line:
            continue
        if "\\" in line:
            continue
        if not vec_mnemonic(mn):
            continue
        out.append(line.strip())
    return out


def split_operands(s):
    """Split on top-level commas (brace/paren aware)."""
    parts, depth, cur = [], 0, []
    for ch in s:
        if ch in "({":
            depth += 1
        elif ch in ")}":
            depth = max(0, depth - 1)
        if ch == "," and depth == 0:
            parts.append("".join(cur).strip())
            cur = []
        else:
            cur.append(ch)
    if "".join(cur).strip():
        parts.append("".join(cur).strip())
    return parts


def reg_class(name):
    n = name.lower()
    m = re.match(r"^(xmm|ymm|zmm)(\d+)$", n)
    if m:
        num = int(m.group(2))
        cls = "L" if num < 8 else ("M" if num < 16 else "H")
        return m.group(1) + cls
    if n in GP32:
        return "r32L" if GP32[n] < 8 else "r32M"
    if n in GP64:
        return "r64L" if GP64[n] < 8 else "r64M"
    if re.match(r"^r(8|9|1[0-5])[bw]$", n):
        return "r816M"
    if n in ("al", "cl", "dl", "bl", "spl", "bpl", "sil", "dil"):
        return "r8L"
    if n in ("ah", "ch", "dh", "bh"):
        return "r8h"
    if n in ("ax", "cx", "dx", "bx", "sp", "bp", "si", "di"):
        return "r16L"
    if re.match(r"^r(8|9|1[0-5])w$", n):
        return "r16M"
    if re.match(r"^k[0-7]$", n):
        return "kR"
    if n in ("mm0", "mm1", "mm2", "mm3", "mm4", "mm5", "mm6", "mm7"):
        return "mmR"
    return "other"


def mem_shape(op):
    """Shape key for a memory operand (addressing mode + disp width)."""
    low = op.lower()
    # decorators
    deco = ""
    if "{z}" in low:
        deco += "+z"
    if re.search(r"\{k[0-7]\}", low):
        deco += "+k"
    if re.search(r"\{1to\d+\}", low):
        deco += "+bcst"
    if "sae" in low:
        deco += "+sae"
    # strip decorators and segment
    core = re.sub(r"\{[^}]*\}", "", op)
    seg = ""
    m = re.match(r"\s*%([a-z0-9]+):(.*)$", core)
    if m:
        seg = "seg+"
        core = m.group(2)
    m = re.match(r"\s*(.*?)\(([^()]*)\)\s*$", core)
    if not m:
        return "mem?" + deco
    disp = m.group(1).strip()
    inside = [x.strip() for x in m.group(2).split(",")]
    while len(inside) < 3:
        inside.append("")
    base, index, scale = inside[0], inside[1], inside[2] or "1"
    if disp in ("", "0", "0x0"):
        d = "d0"
    elif re.match(r"^[+-]?\d+$", disp):
        d = "d8" if -128 <= int(disp) <= 127 else "d32"
    elif re.match(r"^0x[0-9a-f]+$", disp):
        d = "d8" if int(disp, 16) <= 127 else "d32"
    else:
        d = "dsym"
    specials = {"%esp": "SP", "%rsp": "SP", "%ebp": "BP", "%rbp": "BP",
                "%r13": "BP", "%r13d": "BP", "%rip": "RIP", "%eip": "RIP"}
    b = specials.get(base, None)
    if b is None:
        b = reg_class(base[1:]) if base.startswith("%") else ("n" if base == "" else "?")
    if index.startswith("%"):
        x = reg_class(index[1:]) + ("*%s" % scale if scale != "1" else "")
    else:
        x = "n"
    if b == "BP" and d == "d0":
        d = "d8"  # forced disp8
    return f"{seg}mem[{b}+{x}+{d}]{deco}"


def op_shape(op):
    low = op.strip().lower()
    if low.startswith("$") or re.match(r"^[+-]?(0x[0-9a-f]+|\d+)$", low):
        return "IMM"
    if "(" in low and ")" in low:
        return mem_shape(op)
    m = re.match(r"^%([a-z0-9]+)(.*)$", low)
    if m:
        deco = ""
        rest = m.group(2)
        if "{z}" in rest:
            deco += "+z"
        if re.search(r"\{k[0-7]\}", rest):
            deco += "+k"
        if "sae" in rest:
            deco += "+sae"
        return reg_class(m.group(1)) + deco
    if re.match(r"^[a-z_.$][\w$.]*$", low):
        return "SYM"
    return "?"


def line_key(line):
    m = MNEM.match(line)
    mn = m.group(1).lower()
    rest = line[m.end():].strip()
    shapes = tuple(op_shape(o) for o in split_operands(rest)) if rest else ()
    return (mn, shapes)


def asm_ok(lines):
    """True if GAS-64 accepts all lines in one file."""
    with tempfile.NamedTemporaryFile("w", suffix=".s", delete=False) as f:
        f.write(".text\n" + "\n".join(lines) + "\n")
        name = f.name
    try:
        r = subprocess.run([GAS, "--64", "-o", os.devnull, name],
                           capture_output=True, text=True, timeout=60)
        return r.returncode == 0
    except (OSError, subprocess.TimeoutExpired):
        return False
    finally:
        os.unlink(name)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--jobs", type=int, default=2)
    ap.add_argument("--out", default="/tmp/vec.corpus")
    ap.add_argument("--no-filter", action="store_true",
                    help="skip GAS-accept filtering (raw dedup only)")
    args = ap.parse_args()

    files = sorted(TS.glob("*.s"))
    by_mn = defaultdict(list)
    total = 0
    for path in files:
        for line in extract_lines(path):
            total += 1
            by_mn[MNEM.match(line).group(1).lower()].append(line)
    print(f"files={len(files)} vec_lines={total} vec_mnemonics={len(by_mn)}",
          flush=True)

    # Dedup to one concrete representative per (mnemonic, shape) key.
    rep = {}
    for mn, lines in by_mn.items():
        for line in lines:
            try:
                key = line_key(line)
            except Exception:
                continue
            rep.setdefault(key, line)
    print(f"shape_keys={len(rep)}", flush=True)

    cands = sorted(rep.values())
    if args.no_filter:
        Path(args.out).write_text("\n".join(cands) + "\n")
        print(f"wrote {len(cands)} (unfiltered) -> {args.out}", flush=True)
        return
       # GAS-accept filter: batch per mnemonic, then singles for failures.
    by_mn_rep = {}
    for line in cands:
        by_mn_rep.setdefault(MNEM.match(line).group(1).lower(), []).append(line)
    mnems = sorted(by_mn_rep)

    def check_batch(mn):
        return mn, asm_ok(by_mn_rep[mn][:400])

    gas_ok, need = {}, []
    with cf.ThreadPoolExecutor(max_workers=args.jobs) as ex:
        for mn, ok in ex.map(check_batch, mnems):
            if ok:
                gas_ok[mn] = by_mn_rep[mn]
            else:
                need.append(mn)
    print(f"batch_ok={len(gas_ok)} batch_fail={len(need)}", flush=True)

    singles = [(mn, ln) for mn in need for ln in by_mn_rep[mn]]

    def check_single(item):
        mn, ln = item
        return mn, ln, asm_ok([ln])

    extra = defaultdict(list)
    with cf.ThreadPoolExecutor(max_workers=args.jobs) as ex:
        for mn, ln, ok in ex.map(check_single, singles):
            if ok:
                extra[mn].append(ln)
    for mn, lines in extra.items():
        gas_ok[mn] = lines
    out = sorted(ln for lines in gas_ok.values() for ln in lines)
    Path(args.out).write_text("\n".join(out) + "\n")
    print(f"wrote {len(out)} GAS-accepted representatives -> {args.out}",
          flush=True)


if __name__ == "__main__":
    main()
