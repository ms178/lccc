#!/usr/bin/env python3
"""Cross-reference EVERY instruction the i686 encoder declares with the SDM.

`gen_encoding_sweep.py` finds *quality* gaps (one instruction, several legal
encodings).  This tool answers a different and much larger question: for every
mnemonic the integrated i686 assembler claims to implement, is the encoding it
emits a legal form of that instruction according to the Intel SDM?

Why it matters
--------------
The encoder is a table of a thousand-odd rows of the shape

    "addsd" => self.encode_sse_op(ops, &[0xF2, 0x0F, 0x58]),

A wrong row is not a style problem: it assembles to a *different instruction*
and the failure surfaces as wrong code at run time, in whichever program
happened to use that mnemonic.  A per-instruction differential catches it only
if a corpus happens to contain the instruction; a table cross-reference catches
it for every row the moment the row is written.

How it works
------------
1. **The reference.**  A compact catalogue is derived from Intel's own ISA
   description — the `datafiles/*.txt` files in intelxed/xed, which XED
   generates from and validates against the Intel SDM — and vendored under
   `scripts/isa/` with a manifest recording every source, its digest and its
   licence.  Each catalogue entry is `ICLASS -> set of encodings`, where an
   encoding is `(map, mandatory prefix, W, opcode bytes, ModR/M?)`.  Vendoring
   is what lets CI check all of it offline; a gate that needs the network is a
   gate that gets skipped.
2. **The encoder model.**  The same row shape is parsed out of the Rust source.
   Rows that spell their prefixes as literal bytes are self-describing; rows
   that call a helper (VEX/EVEX) are interpreted through `HELPERS`, the table
   that records what each helper does to the map, the mandatory prefix and W.
   A row the model cannot interpret is reported as UNMODELLED — never skipped
   silently, because a skipped row is exactly the row that is wrong.
3. **The verdict.**  LCCC-LEGAL: the SDM has that encoding for that mnemonic.
   LCCC-EXTRA: LCCC encodes something the SDM does not list.  That is either a
   wrong opcode or a wrong mnemonic -> SDM, and both are defects.

`--coverage` answers the converse question: which instruction classes in the
catalogue has the i686 encoder not implemented at all.  That is the worklist
for ISA implementation, ranked by whether the encoder's own code generator can
even reach them.

Usage
-----
    scripts/sdm_isa_crossref.py                    # verify every row
    scripts/sdm_isa_crossref.py --strict           # exit 1 on any finding
    scripts/sdm_isa_crossref.py --coverage         # + unimplemented classes
    scripts/sdm_isa_crossref.py --list-rows FILE   # one file only
    scripts/sdm_isa_crossref.py --build-catalogue scripts/isa/catalogue.json
    scripts/sdm_isa_crossref.py --families         # legacy choice-family check

The `--build-catalogue` mode needs the reference text files on disk (fetch them
with `--fetch-reference`, which needs network); every other mode is hermetic.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
ISA_DIR = REPO / "scripts" / "isa"
CATALOGUE = ISA_DIR / "catalogue.json"
MANIFEST = ISA_DIR / "MANIFEST.json"
# Both assemblers are in scope: the 32-bit encoder is not the only one that
# emits these bytes, and a table cross-reference that only covers half the
# backends hides defects in the other half.
BACKENDS = {
    "i686": [
        "src/backend/i686/assembler/encoder/mod.rs",
        "src/backend/i686/assembler/encoder/gp_integer.rs",
        "src/backend/i686/assembler/encoder/sse.rs",
        "src/backend/i686/assembler/encoder/vex.rs",
        "src/backend/i686/assembler/encoder/x87.rs",
        "src/backend/i686/assembler/encoder/system.rs",
    ],
    "x86": [
        "src/backend/x86/assembler/encoder/mod.rs",
        "src/backend/x86/assembler/encoder/gp_integer.rs",
        "src/backend/x86/assembler/encoder/sse.rs",
        "src/backend/x86/assembler/encoder/avx.rs",
        "src/backend/x86/assembler/encoder/x87_misc.rs",
        "src/backend/x86/assembler/encoder/core.rs",
        "src/backend/x86/assembler/encoder/system.rs",
    ],
}
ENCODER_FILES = BACKENDS["i686"]


def encoder_files(backend: str = "i686") -> list[str]:
    """Encoder sources of `backend` (`i686`, `x86`, or `all`)."""
    if backend == "all":
        return [f for files in BACKENDS.values() for f in files]
    return list(BACKENDS[backend])

# Reference files, as published by intelxed/xed.  XED's ISA descriptions are
# generated from and validated against the Intel Software Developer's Manual;
# they are split by extension, so the union of these covers exactly the ISA the
# i686 encoder can emit.  `digest` pins the revision a catalogue was built from.
SOURCES = [
    ("datafiles/xed-isa.txt", "legacy x87/MMX/SSE..SSE4, AES, PCLMULQDQ, system"),
    ("datafiles/avx/avx-isa.txt", "VEX AVX (128/256)"),
    ("datafiles/hswavx/hsw-int256-isa.txt", "AVX2 256-bit integer"),
    ("datafiles/hswavx/avx-fma-isa.xed.txt", "FMA (VEX 0F38)"),
    ("datafiles/sha/sha-isa.xed.txt", "SHA1RNDS4/SHA256RNDS2 (SSE, 0F38/0F3A)"),
    ("datafiles/gfni-vaes-vpcl/gfni-sse-isa.xed.txt", "GF2P8* SSE forms"),
    ("datafiles/gfni-vaes-vpcl/gfni-vex-isa.xed.txt", "GF2P8* VEX forms"),
    ("datafiles/gfni-vaes-vpcl/vaes-vex-isa.xed.txt", "VAES/VPCLMULQDQ (VEX)"),
    ("datafiles/hswbmi/hsw-bmi-vex-isa.xed.txt", "BMI VEX forms"),
]
XED_REPO = "intelxed/xed"
XED_LICENSE = "Apache-2.0"

# What each VEX/EVEX helper does to the encoding.  Read from the helper
# definitions in encoder/vex.rs, not guessed: `map` is the VEX map, `pp` the
# mandatory prefix, and a row passes its own opcode byte.  `args` names the
# helper parameters the row supplies, so a signature change shows up as an
# UNMODELLED row rather than as a silently wrong verdict.
# Argument positions count from 0 at `&self`, so 0 = ops, 1 = the opcode byte,
# 2 = the mandatory-prefix argument (a bool `has_66` or an explicit pp index).
# Getting this wrong once made every VEX row UNMODELLED, which is why an
# unmodelled row is reported rather than skipped: a silent skip looks exactly
# like a pass.
HELPERS = {
    "encode_avx_3op_np": ("0f", "none", None, None),
    "encode_avx_3op": ("0f", None, 2, None),
    "encode_avx_3op_pp": ("0f", None, 2, None),
    "encode_avx_3op_commutative": ("0f", None, 2, None),
    "encode_avx_3op_38": ("0f38", None, 2, None),
    "encode_avx_3op_38_pp": ("0f38", None, 2, None),
    "encode_avx_3op_38_w1": ("0f38", "66", None, 1),
    "encode_avx_3op_3a_imm8": ("0f3a", "66", None, None),
    "encode_avx_3op_3a_pp_imm8": ("0f3a", None, 2, None),
    "encode_avx_4op_3a": ("0f3a", "66", None, None),
    "encode_avx_2op_0f": ("0f", None, 2, None),
    "encode_avx_2op_2op_38": ("0f38", "66", None, None),
    "encode_avx_2op_38": ("0f38", "66", None, None),
    "encode_avx_3op_0f_imm8": ("0f", None, 2, None),
    "encode_avx_broadcast": ("0f38", "66", None, None),
}

# The 64-bit encoder spells the same information as *arguments*: an opcode-map
# number (1 = 0F, 2 = 0F38, 3 = 0F3A), a mandatory-prefix number (0 = none,
# 1 = 66, 2 = F3, 3 = F2), a W bit and the opcode byte.  Reading the argument
# positions out of the source (not out of a naming convention) is what makes
# this exact: the models below were taken from the helper bodies, and each one
# says which argument carries what.
METHOD_MAP = {1: "0f", 2: "0f38", 3: "0f3a"}
KOPMASK_SIZE = {8: ("none", 1), 4: ("66", 1), 2: ("none", 0), 1: ("66", 0)}


def _evex_five(map_idx: int = 1, pp_idx: int = 2, w_idx: int = 3, opcode_idx: int = 4) -> dict:
    return {"family": "evex", "map": ("argmap", map_idx), "pp": ("argpp", pp_idx),
            "w": ("arg", w_idx), "opcode": opcode_idx}


HELPER_SPEC = {
    # EVEX: (ops, map, pp, w, opcode), with extra trailing parameters on some.
    "encode_evex_binary": _evex_five(),
    "encode_evex_unary": _evex_five(),
    "encode_evex_binary_scalar": _evex_five(),
    "encode_evex_binary_scalar_elem": _evex_five(),
    "encode_evex_3src_imm": _evex_five(),
    "encode_evex_3src_imm_sae": _evex_five(),
    # (ops, mnemonic, map, pp, w, opcode, ...)
    "encode_evex_cmp_mask": _evex_five(2, 3, 4, 5),
    # (ops, opcode, pp)
    # (ops, opcode, pp): the body emits `emit_vex(.., 2, 0, ..)`, so W is
    # fixed at 0 even though the mnemonic family varies its mandatory prefix.
    "encode_avx_3op_38_pp": {"family": "vex", "map": ("const", "0f38"),
                             "pp": ("argpp", 2), "w": ("const", 0), "opcode": 1},
    # (ops, opcode, pp, mnemonic) -> VEX.128.0F: `emit_vex(.., mm = 1, ..)`.
    "encode_avx_scalar_3op": {"family": "vex", "map": ("const", "0f"),
                              "pp": ("argpp", 2), "w": ("const", 0), "opcode": 1},
    "encode_avx_3op_38_pp_w1": {"family": "vex", "map": ("const", "0f38"),
                                "pp": ("argpp", 2), "w": 1, "opcode": 1},
    # (ops, opcode, base, default_reg, name): `fcmov*` is DA/DB with the
    # condition in the ModR/M reg field, i.e. reg = (base & 0x38) >> 3.
    "encode_x87_st0_sti": {"family": "legacy", "map": ("const", "none"),
                           "pp": ("const", "none"), "w": None,
                           "opcode": ("arg", 1), "reg": ("x87_base", 2)},
    # (ops) with the byte pair fixed in the helper: fucom DD /4, fucomp DD /5.
    "encode_fucom": {"family": "legacy", "map": ("const", "none"),
                     "pp": ("const", "none"), "w": None,
                     "opcode": ("const", 0xDD), "reg": ("const", 4)},
    "encode_fucomp": {"family": "legacy", "map": ("const", "none"),
                      "pp": ("const", "none"), "w": None,
                      "opcode": ("const", 0xDD), "reg": ("const", 5)},
    # (ops, &[map, opcode], /digit): `vldmxcsr`/`vstmxcsr`, VEX.0F AE /2 /3.
    "encode_vex_mem_only": {"family": "vex", "map": ("argbytes_map", 1),
                            "pp": ("const", "none"), "w": 0,
                            "opcode": ("argbytes_last", 1), "reg": ("arg", 2)},
    # (ops, opcode, pp, w): BMI2 is VEX.0F38 with the W bit selecting 32/64.
    "encode_bmi2_shift": {"family": "vex", "map": ("const", "0f38"),
                          "pp": ("argpp", 2), "w": ("arg", 3), "opcode": 1},
    # (ops, opcode, size): opmask arithmetic, VEX.L0.66.0F, w/pp from the size.
    "encode_kopmask_3op": {"family": "vex", "map": ("const", "0f"),
                           "pp": ("kopmask", 2, 0), "w": ("kopmask", 2, 1), "opcode": 1},
    "encode_kopmask_2op": {"family": "vex", "map": ("const", "0f"),
                           "pp": ("kopmask", 2, 0), "w": ("kopmask", 2, 1), "opcode": 1},
}

PP_BY_INDEX = {0: "none", 1: "66", 2: "f3", 3: "f2"}
PP_BY_BYTE = {0x66: "66", 0xF3: "f3", 0xF2: "f2"}
PP_BYTE_PREFIXES = (0x66, 0xF2, 0xF3)

# Mandatory-prefix markers as XED spells them in PATTERN.
PP_MARKERS = {
    "no_refining_prefix": "none",
    "osz_refining_prefix": "66",
    "REFINING66()": "66",
    "f2_refining_prefix": "f2",
    "f3_refining_prefix": "f3",
    "V66": "66",
    "VF2": "f2",
    "VF3": "f3",
    "VNP": "none",
}

# Mnemonic aliases: the encoder spells some instructions the way AT&T does, and
# XED names the same instruction by its canonical (Intel manual) name.  An
# alias is only trusted when the canonical class exists.
ALIASES = {
    "sete": "SETZ", "setne": "SETNZ", "setl": "SETL", "setle": "SETLE",
    "setg": "SETG", "setge": "SETGE", "setb": "SETB", "setbe": "SETBE",
    "seta": "SETA", "setae": "SETAE", "sets": "SETS", "setns": "SETNS",
    "seto": "SETO", "setno": "SETNO", "setp": "SETP", "setnp": "SETNP",
    "cmove": "CMOVZ", "cmovne": "CMOVNZ", "cmovb": "CMOVB", "cmovae": "CMOVNB",
    "cmovbe": "CMOVBE", "cmova": "CMOVNBE", "cmovs": "CMOVS", "cmovns": "CMOVNS",
    "cmovl": "CMOVL", "cmovle": "CMOVLE", "cmovg": "CMOVNLE", "cmovge": "CMOVNL",
    "je": "JZ", "jne": "JNZ", "jz": "JZ", "jnz": "JNZ",
    "jb": "JB", "jae": "JNB", "jbe": "JBE", "ja": "JNBE",
    "jl": "JL", "jge": "JNL", "jle": "JLE", "jg": "JNLE",
    "js": "JS", "jns": "JNS", "jo": "JO", "jno": "JNO", "jp": "JP", "jnp": "JNP",
    "shl": "SHL", "sal": "SHL", "shldl": "SHLD", "shrdl": "SHRD",
    # FCMOVcc is documented under both spellings: `a`/`ae`/`na`/`nae` name the
    # same arithmetic condition the integer CMOVcc uses, i.e. not-below-or-equal
    # and friends, which XED catalogues as the NBE/NB/BE/B forms (SDM Vol. 2B).
    "fcmova": "FCMOVNBE", "fcmovae": "FCMOVNB",
    "fcmovna": "FCMOVBE", "fcmovnae": "FCMOVB",
    "movsbl": "MOVSX", "movswl": "MOVSX", "movsbw": "MOVSX", "movzb": "MOVZX",
    "movzbl": "MOVZX", "movzwl": "MOVZX", "movzbw": "MOVZX",
    "cltd": "CDQ", "cwtl": "CWDE", "cbtw": "CBW", "cwtd": "CWD",
    "popf": "POPF", "pushf": "PUSHF", "popa": "POPA", "pusha": "PUSHA",
    "lret": "RETFAR", "lretq": "RETFAR", "lretw": "RETFAR",
    "ljmpl": "JMPFAR", "lcalll": "CALLFAR", "lcallw": "CALLFAR",
    "nop": "NOP", "int3": "INT3", "ud2": "UD2",
    "movs": "MOVS", "lods": "LODS", "cmps": "CMPS", "stos": "STOS",
    "scas": "SCAS", "ins": "INS", "outs": "OUTS",
    # XED suffixes the SSE form when the bare mnemonic is the string op.
    "movsd": "MOVSD_XMM", "cmpsd": "CMPSD_XMM", "movss": "MOVSS",
    # x87: AT&T spells the memory width, the SDM names one instruction.
    "flds": "FLD", "fldl": "FLD", "fldt": "FLD",
    "fsts": "FST", "fstps": "FSTP", "fstpl": "FSTP", "fstpt": "FSTP",
    "filds": "FILD", "fildl": "FILD", "fildq": "FILD", "fildll": "FILD",
    "fistps": "FISTP", "fistpl": "FISTP", "fistpq": "FISTP", "fistpll": "FISTP",
    "fistl": "FIST", "fist": "FIST", "fists": "FIST",
    "fadds": "FADD", "faddl": "FADD", "fsubs": "FSUB", "fsubl": "FSUB",
    "fmuls": "FMUL", "fmull": "FMUL", "fdivs": "FDIV", "fdivl": "FDIV",
    "fsubrs": "FSUBR", "fsubrl": "FSUBR",
    "fdivrs": "FDIVR", "fdivrl": "FDIVR",
    "fcoms": "FCOM", "fcoml": "FCOM", "fcomps": "FCOMP", "fcompl": "FCOMP",
    "fstcw": "FNSTCW", "fstsw": "FNSTSW", "fclex": "FNCLEX", "finit": "FNINIT",
    "fstenv": "FNSTENV", "fsave": "FNSAVE",
    "fisttps": "FISTTP", "fisttpl": "FISTTP", "fisttpll": "FISTTP",
    "fucomip": "FUCOMIP", "fcomip": "FCOMIP", "fcomi": "FCOMI", "ffreep": "FFREEP",
    "fiadd": "FIADD", "fiadds": "FIADD", "fiaddl": "FIADD",
    "fimul": "FIMUL", "fimuls": "FIMUL", "fimull": "FIMUL",
    "ficom": "FICOM", "ficoms": "FICOM", "ficoml": "FICOM",
    "ficomp": "FICOMP", "ficomps": "FICOMP", "ficompl": "FICOMP",
    "fisub": "FISUB", "fisubs": "FISUB", "fisubl": "FISUB",
    "fisubr": "FISUBR", "fisubrs": "FISUBR", "fisubrl": "FISUBR",
    "fidiv": "FIDIV", "fidivs": "FIDIV", "fidivl": "FIDIV",
    "fidivr": "FIDIVR", "fidivrs": "FIDIVR", "fidivrl": "FIDIVR",
    "fcmovb": "FCMOVB", "fcmove": "FCMOVE", "fcmovbe": "FCMOVBE",
    "fcmovu": "FCMOVU", "fcmovnb": "FCMOVNB", "fcmovne": "FCMOVNE",
    "fcmovnbe": "FCMOVNBE", "fcmovnu": "FCMOVNU",
}

ROW_RE = re.compile(r'^\s*((?:"[a-z0-9._]+"\s*(?:\|\s*)?)+)\s*=>\s*(\S.*?),?\s*$')
BYTES_RE = re.compile(r"&\[([0-9A-Fa-fx,\s]+)\]")


def _upper_class(mnem: str) -> list[str]:
    """Candidate SDM instruction classes for an AT&T mnemonic."""
    m = mnem.lower()
    cands = []
    if m in ALIASES:
        cands.append(ALIASES[m])
    for pat in (r"[bwlq]$", r"[bwlqt]$"):
        stripped = re.sub(pat, "", m)
        if stripped.upper() not in cands:
            cands.append(stripped.upper())
    if m.upper() not in cands:
        cands.append(m.upper())
    # VEX/EVEX rows are named after the legacy instruction in XED everywhere,
    # but keep the literal spelling as a candidate in case it is not.
    return cands


def parse_reference(paths: list[Path]) -> dict[str, list[dict]]:
    """`{ICLASS: [encoding, ...]}` from XED-format ISA descriptions."""
    cat: dict[str, list[dict]] = {}
    for path in paths:
        text = path.read_text(encoding="utf-8", errors="replace")
        text = re.sub(r"#BEGIN_LEGAL.*?#END_LEGAL", "", text, flags=re.S)
        for block in text.split("\n{\n")[1:]:
            ic = re.search(r"ICLASS\s*:\s*(\S+)", block)
            pat = re.search(r"PATTERN\s*:\s*(.+)", block)
            if not ic or not pat:
                continue
            enc = parse_pattern(pat.group(1))
            if enc is None:
                continue
            cat.setdefault(ic.group(1), [])
            if enc not in cat[ic.group(1)]:
                cat[ic.group(1)].append(enc)
    return cat


def parse_pattern(pattern: str) -> dict | None:
    """One XED PATTERN -> (family, map, pp, w, opcodes, modrm) or None."""
    toks = pattern.split()
    family = "legacy"
    if any(t.startswith("EVEX") for t in toks):
        family = "evex"
    elif any(t.startswith("VEX") or t in ("VV1", "VV2") or t.startswith("V0F")
             for t in toks):
        family = "vex"
    elif any(t.startswith("XOP") for t in toks):
        family = "xop"

    map_ = "none"
    pp = "none"
    w = None
    opcodes: list[int] = []
    modrm = "MODRM()" in pattern
    for tok in toks:
        if re.fullmatch(r"0x[0-9A-Fa-f]{2}", tok):
            opcodes.append(int(tok, 16))
        elif tok in ("V0F38", "V0F3A", "V0F"):
            map_ = tok[1:].lower()
        elif tok in ("VW0", "V0"):
            w = 0
        elif tok in ("VW1", "V1"):
            w = 1
        elif tok in PP_MARKERS:
            pp = PP_MARKERS[tok]
    if not opcodes:
        return None
    # Legacy map: the opcode list itself carries the 0F escape, which is what
    # an encoder row spells as &[0x0F, 0x38, 0xNN].
    if family == "legacy":
        if opcodes[:1] == [0x0F]:
            if opcodes[1:2] in ([0x38], [0x3A]):
                map_ = "0f" + hex(opcodes[1])[2:]
                opcodes = opcodes[2:]
            else:
                map_ = "0f"
                opcodes = opcodes[1:]
            opcodes = opcodes[:1]
    return {
        "family": family,
        "map": map_,
        "pp": pp,
        "w": w,
        "opcode": opcodes[-1] if opcodes else None,
        "modrm": modrm,
        "reg": (lambda m: int(m.group(1), 2) if m else None)(
            re.search(r"REG\[0b([01]+)\]", pattern)),
    }


def extract_rows(path: Path) -> list[dict]:
    """Every declarative mnemonic row in one encoder source file."""
    rows = []
    for lineno, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if line.strip().startswith("//"):
            continue
        m = ROW_RE.match(line)
        if not m:
            continue
        names = re.findall(r'"([a-z0-9._]+)"', m.group(1))
        body = m.group(2)
        # Only rows that dispatch to an encoder are instruction rows.  Tuple
        # tables and prefix maps share the `"name" => ...` shape but carry data,
        # and counting them as rows only inflates the UNMODELLED list.
        if not body.startswith(("self.", "Self::", "encode_")):
            continue
        byte_arrays = []
        for arr in BYTES_RE.findall(body):
            vals = [int(v.strip(), 16) for v in arr.split(",") if v.strip()]
            if vals:
                byte_arrays.append(vals)
        rows.append({
            "file": str(path.relative_to(REPO)),
            "line": lineno,
            "names": names,
            "body": body,
            "arrays": byte_arrays,
        })
    return rows


def _model_spec_helper(name: str, body: str, b64: bool = False) -> list[dict] | None:
    """Read a helper call through `HELPER_SPEC`.

    The argument positions come from the helper bodies in the 64-bit encoder, so
    a helper whose parameters are reordered fails here as UNMODELLED instead of
    being modelled with the old layout.
    """
    spec = HELPER_SPEC[name]
    # A helper argument can itself carry parentheses (`encode_x87_st0_sti(ops,
    # 0xDF, 0xF0, Some(1), "fcomip")`), so the argument list is delimited by
    # balancing brackets from the call's own `(` and not by the first `)`.
    start = body.find("(", body.find(name) + len(name))
    if start < 0:
        return None
    depth_args, end = 0, -1
    for i in range(start, len(body)):
        if body[i] == "(":
            depth_args += 1
        elif body[i] == ")":
            depth_args -= 1
            if depth_args == 0:
                end = i
                break
    if end < 0:
        return None
    vals, depth, cur = [], 0, ""
    for ch in body[start + 1 : end]:
        if ch == "[":
            depth += 1
        elif ch == "]":
            depth -= 1
        if ch == "," and depth == 0:
            vals.append(cur.strip())
            cur = ""
        else:
            cur += ch
    vals.append(cur.strip())

    def byte_array(idx: int) -> list[int]:
        raw = vals[idx]
        inner = raw[raw.index("[") + 1 : raw.rindex("]")]
        return [int(v.strip(), 16) for v in inner.split(",") if v.strip()]

    def take(rule):
        if rule is None:
            return None
        kind = rule[0]
        if kind == "const":
            return rule[1]
        idx = rule[1]
        if idx >= len(vals):
            raise ValueError("missing argument")
        raw = vals[idx]
        if kind == "argmap":
            return METHOD_MAP.get(int(raw, 0))
        if kind == "argpp":
            return PP_BY_INDEX.get(int(raw, 0))
        if kind == "argbytes_map":
            arr = byte_array(idx)
            stripped = legacy_encoding(arr, b64)
            return None if stripped is None else stripped["map"]
        if kind == "argbytes_last":
            return byte_array(idx)[-1]
        if kind == "kopmask":
            pair = KOPMASK_SIZE.get(int(vals[idx], 0))
            return None if pair is None else pair[rule[2]]
        if kind == "x87_base":
            # DA C0 -> /0, DA D0 -> /2, DB D8 -> /3.
            return (int(raw, 0) & 0x38) >> 3
        if kind == "arg":
            return int(raw, 0)
        raise ValueError(f"bad spec {rule!r}")

    # A spec may spell the opcode as a rule (`("arg", 1)`, `("const", "dd")`) or
    # as a bare literal; both mean "this is where the opcode byte comes from".
    opcode_rule = spec["opcode"]
    if not isinstance(opcode_rule, tuple):
        opcode_rule = ("arg", opcode_rule) if isinstance(opcode_rule, int) else ("const", opcode_rule)

    try:
        enc = {
            "family": spec["family"],
            "map": take(spec["map"]),
            "pp": take(spec["pp"]),
            "w": take(spec["w"]),
            "opcode": take(opcode_rule),
            "modrm": True,
            "reg": take(spec.get("reg")),
        }
    except (ValueError, TypeError):
        return None
    if enc["map"] is None or enc["pp"] is None:
        return None
    return [enc]


def row_is_64bit(row: dict) -> bool:
    """Whether a row comes from the 64-bit encoder.

    The two backends share the SDM-format model but differ in one respect that
    changes the meaning of the bytes: `40..4F` is a REX prefix in 64-bit mode
    and an `inc`/`dec` opcode in 32-bit mode.  Deciding per row from its path
    keeps `--backend all` correct instead of applying one mode to both.
    """
    return "/i686/" not in row["file"].replace("\\", "/")


def model_any(row: dict) -> list[dict] | None:
    """Model a row with the mode implied by the backend that declares it."""
    return model_row(row, row_is_64bit(row))


def model_row(row: dict, b64: bool = False) -> list[dict] | None:
    """Interpret one encoder row as SDM-comparable encodings."""
    body = row["body"]
    encs: list[dict] = []
    helper = re.match(r"self\.(encode_[a-z0-9_]+)\(", body)
    if helper and helper.group(1) in HELPER_SPEC:
        return _model_spec_helper(helper.group(1), body, b64)
    if helper and helper.group(1) in HELPERS:
        name = helper.group(1)
        map_, fixed_pp, pp_idx, w = HELPERS[name]
        args = re.search(r"\(([^)]*)\)", body)
        if args is None:
            return None
        vals = [a.strip() for a in args.group(1).split(",")]
        try:
            opcode = next(int(a, 0) for a in vals[1:]
                          if re.fullmatch(r"0x[0-9A-Fa-f]+|\d+", a))
        except StopIteration:
            return None
        pp = fixed_pp
        if pp is None and pp_idx is not None and pp_idx < len(vals):
            raw = vals[pp_idx]
            if re.fullmatch(r"\d+", raw):
                pp = PP_BY_INDEX.get(int(raw))
            elif raw in ("true", "false"):
                pp = "66" if raw == "true" else "none"
            else:
                return None
        if pp is None:
            return None
        return [{"family": "vex", "map": map_, "pp": pp, "w": w,
                 "opcode": opcode, "modrm": True}]
    # x87 memory forms carry the operation in ModRM.reg, passed as the helper's
    # trailing argument: `self.encode_x87_mem(ops, &[0xDB], 7)`.  The literal byte
    # array names only the opcode, so the extension has to be read from the call
    # *before* the generic byte-array path claims the row.
    m = re.match(r"self\.(encode_x87_mem|encode_x87_wait_mem|encode_x87_st_i)\(", body)
    if m and row["arrays"]:
        vals = [a.strip() for a in re.search(r"\(([^)]*)\)", body).group(1).split(",")]
        try:
            ext = int(vals[-1], 0)
        except ValueError:
            return None
        enc = legacy_encoding(row["arrays"][0], b64)
        if enc is None:
            return None
        enc["reg"] = ext
        return [enc]
    if row["arrays"]:
        for arr in row["arrays"]:
            enc = legacy_encoding(arr, b64)
            if enc is None:
                return None
            encs.append(enc)
        return encs
    # x87 memory forms: `<opcode>, <reg-extension>` where reg is ModRM.reg.
    m = re.match(r"self\.(encode_x87_mem|encode_x87_wait_mem)\(([^)]*)", body)
    if m and row["arrays"]:
        vals = [a.strip() for a in m.group(2).split(",")]
        try:
            ext = int(vals[-1], 0)
        except ValueError:
            return None
        enc = legacy_encoding(row["arrays"][0])
        if enc is None:
            return None
        enc["reg"] = ext
        return [enc]
    # x87 register forms: `DE <modrm>` with reg fixed by the helper's constant.
    m = re.match(r'self\.encode_x87_pop_reg\(ops,\s*"[a-z0-9]+",\s*(0x[0-9A-Fa-f]+)',
                 body)
    if m:
        modrm = int(m.group(1), 16)
        return [{"family": "legacy", "map": "none", "pp": "none", "w": None,
                 "opcode": 0xDE, "modrm": True, "reg": (modrm >> 3) & 7}]
    m = re.match(r"self\.(encode_avx_[a-z0-9_]+)\(([^)]*)", body)
    if m and m.group(1) in HELPERS:
        name, args = m.group(1), m.group(2)
        map_, fixed_pp, pp_idx, w = HELPERS[name]
        vals = [a.strip() for a in args.split(",")]
        # args[0] is `ops`; the opcode is the first integer literal
        try:
            opcode = next(int(a, 0) for a in vals[1:] if re.fullmatch(r"0x[0-9A-Fa-f]+|\d+", a))
        except StopIteration:
            return None
        pp = fixed_pp
        if pp is None and pp_idx is not None and pp_idx < len(vals):
            raw = vals[pp_idx]
            if re.fullmatch(r"\d+", raw):
                pp = PP_BY_INDEX.get(int(raw))
            elif raw in ("true", "false"):
                pp = "66" if raw == "true" else "none"
            else:
                return None
        if pp is None:
            return None
        encs.append({"family": "vex", "map": map_, "pp": pp, "w": w,
                     "opcode": opcode, "modrm": True})
        # A `_commutative` row is the same encoding; nothing else to add.
        return encs
    return None


def legacy_encoding(arr: list[int], b64: bool = False) -> dict | None:
    """`&[0x66, 0x0F, 0x58]` -> (map, pp, opcode).

    `0x66` and `0xF2`/`0xF3` are not alternatives: the SSE mandatory prefix is
    the F2/F3, and 66 in front of it is the operand-size override. CRC32's
    16-bit form is exactly `66 F2 0F 38 F1`, so a model that treats 66 as a
    mandatory prefix overwrites the F2 and reports a legal row as wrong.
    """
    pp = "none"
    seen_66 = False
    if b64:
        # In 64-bit mode `40..4F` is a REX prefix, not an inc/dec opcode: it
        # carries W/R/X/B and no encoding information of its own here.  Only
        # strip it for the 64-bit backend — in the 32-bit encoder `&[0x40]` is
        # `inc %eax`.
        while arr and 0x40 <= arr[0] <= 0x4F:
            arr = arr[1:]
    while arr and arr[0] in PP_BYTE_PREFIXES:
        if arr[0] == 0x66:
            seen_66 = True
        else:
            pp = PP_BY_BYTE[arr[0]]
        arr = arr[1:]
    if pp == "none" and seen_66:
        pp = "66"
    map_ = "none"
    if arr and arr[0] == 0x0F:
        arr = arr[1:]
        map_ = "0f"
        if arr and arr[0] in (0x38, 0x3A):
            map_ = "0f" + hex(arr[0])[2:]
            arr = arr[1:]
    if len(arr) != 1:
        return None
    return {"family": "legacy", "map": map_, "pp": pp, "w": None,
            "opcode": arr[0], "modrm": True, "reg": None}


def _matches(want: dict, have: dict) -> bool:
    if want["family"] != have["family"] or want["map"] != have["map"]:
        return False
    if want["pp"] != have["pp"] or want["opcode"] != have["opcode"]:
        return False
    if want["w"] is not None and have["w"] is not None and want["w"] != have["w"]:
        return False
    # x87 puts the operation in ModRM.reg; when either side names it, both must.
    if want.get("reg") is not None and have.get("reg") is not None:
        if want["reg"] != have["reg"]:
            return False
    return True


def verify(catalogue: dict, files: list[Path], verbose: bool,
           b64: bool = False) -> tuple[list, list]:
    findings, unmodeLLed = [], []
    verified = 0
    for path in files:
        for row in extract_rows(path):
            encs = model_row(row, b64)
            if encs is None:
                unmodeLLed.append(row)
                continue
            for name in row["names"]:
                classes = _upper_class(name)
                hit = next((c for c in classes if c in catalogue), None)
                if hit is None:
                    findings.append((row, name, "NO-CLASS", classes[0]))
                    continue
                for enc in encs:
                    if any(_matches(enc, form) for form in catalogue[hit]):
                        verified += 1
                    else:
                        findings.append((row, name, "LCCC-EXTRA", hit))
    if verbose:
        print(f"rows verified: {verified}")
    return findings, unmodeLLed


def coverage(catalogue: dict, implemented: set[str]) -> list[str]:
    return sorted(c for c in catalogue if c not in implemented)


def build_catalogue(out: Path, manifest_out: Path, ref_dir: Path) -> int:
    paths = []
    sources = []
    for rel, role in SOURCES:
        p = ref_dir / Path(rel).name
        if not p.is_file():
            print(f"missing reference: {p}", file=sys.stderr)
            return 2
        paths.append(p)
        sources.append({
            "repo": XED_REPO, "path": rel, "role": role,
            "sha256": hashlib.sha256(p.read_bytes()).hexdigest(),
            "bytes": p.stat().st_size, "license": XED_LICENSE,
        })
    cat = parse_reference(paths)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(cat, indent=0, sort_keys=True) + "\n")
    manifest_out.write_text(json.dumps({
        "note": ("Derived catalogue of Intel-SDM instruction encodings, generated "
                 "by scripts/sdm_isa_crossref.py --build-catalogue from XED's ISA "
                 "descriptions (which XED generates from and validates against the "
                 "SDM). Regenerate, do not hand-edit."),
        "generator": "scripts/sdm_isa_crossref.py",
        "sources": sources,
        "classes": len(cat),
        "encodings": sum(len(v) for v in cat.values()),
    }, indent=2) + "\n")
    print(f"catalogue: {len(cat)} classes, "
          f"{sum(len(v) for v in cat.values())} encodings -> {out}")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--catalogue", default=str(CATALOGUE))
    ap.add_argument("--backend", default="all", choices=["all", *BACKENDS],
                    help="encoder backend(s) to check (default: all)")
    ap.add_argument("--list-rows", nargs="*", default=None,
                    help="verify only these encoder files (repo-relative)")
    ap.add_argument("--strict", action="store_true",
                    help="exit 1 when any row is SDM-extra or unmodeLLed")
    ap.add_argument("--coverage", action="store_true",
                    help="also report instruction classes with no implementation")
    ap.add_argument("--limit", type=int, default=40,
                    help="findings to print (default 40)")
    ap.add_argument("--build-catalogue", metavar="OUT")
    ap.add_argument("--reference-dir", metavar="DIR",
                    help="directory holding the XED reference files")
    ap.add_argument("--manifest", default=str(MANIFEST))
    # `--list-rows` is a variable-length option; parsing it with the plain
    # parser lets the option starve on CPython 3.12 when another optional
    # follows it, so parse intermixed as the CLI contract requires.
    args = ap.parse_intermixed_args()

    if args.build_catalogue:
        ref = Path(args.reference_dir or ".")
        return build_catalogue(Path(args.build_catalogue), Path(args.manifest), ref)

    cat_path = Path(args.catalogue)
    if not cat_path.is_file():
        print(f"missing catalogue {cat_path}; run --build-catalogue", file=sys.stderr)
        return 2
    catalogue = json.loads(cat_path.read_text())

    files = ([REPO / f for f in args.list_rows] if args.list_rows
             else [REPO / f for f in encoder_files(args.backend) if (REPO / f).is_file()])
    b64 = args.backend != "i686"
    findings, unmodeLLed = verify(catalogue, files, verbose=True, b64=b64)
    implemented = {c for f in files for r in extract_rows(f)
                   for n in r["names"] for c in _upper_class(n)}

    print()
    print(f"catalogue: {len(catalogue)} instruction classes, "
          f"{sum(len(v) for v in catalogue.values())} encodings")
    print(f"encoder:   {len(implemented)} classes referenced by declared rows")
    if findings:
        print(f"\n{len(findings)} finding(s):")
        for row, name, kind, cls in findings[: args.limit]:
            print(f"  {kind:12s} {name:14s} ({cls}) at {row['file']}:{row['line']}")
            print(f"               {row['body'][:96]}")
    if unmodeLLed:
        print(f"\n{len(unmodeLLed)} unmodeLLed row(s) (not verified, not skipped):")
        for row in unmodeLLed[: args.limit]:
            print(f"  {row['file']}:{row['line']} {row['names'][0]}: "
                  f"{row['body'][:80]}")
    if args.coverage:
        missing = coverage(catalogue, implemented)
        print(f"\n{len(missing)} catalogue class(es) with no encoder row:")
        print("  " + " ".join(missing[:400]))

    bad = bool(findings) or (args.strict and bool(unmodeLLed))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
