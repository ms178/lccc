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
    # Extensions whose ICLASS names the encoder emits but which XED keeps in
    # their own files.  Each one was added to close a NO-CLASS finding rather
    # than to widen the gate for its own sake: VNNI dot products (the `vpdp*`
    # family), the AVX2 variable shifts of `hsw-vshift-isa`, SM4, the extended
    # XSAVE forms, CLDEMOTE, MOVDIR, the VAES *IMC* helper, and XOP/VPERMIL2/
    # FMA4 which `encoder/xop.rs` encodes.
    ("datafiles/vnni/vnni-isa.xed.txt", "VNNI dot products (EVEX)"),
    ("datafiles/avx-vnni/avx-vnni-isa.xed.txt", "VNNI dot products (VEX)"),
    ("datafiles/avx-vnni-int8/avx-vnni-int8-isa.xed.txt", "VNNI int8 mixed-sign (VEX)"),
    ("datafiles/avx-vnni-int16/avx-vnni-int16-isa.xed.txt", "VNNI int16 mixed-sign (VEX)"),
    ("datafiles/hswavx/hsw-vshift-isa.txt", "AVX2 variable shifts (VPSLLV*)"),
    ("datafiles/sm4/sm4-isa.xed.txt", "SM4 key/rnds (VEX)"),
    ("datafiles/sm4-evex/sm4-evex-isa.xed.txt", "SM4 (EVEX)"),
    ("datafiles/xsaveopt/xsaveopt-isa.txt", "XSAVEOPT"),
    ("datafiles/xsavec/xsavec-isa.txt", "XSAVEC"),
    ("datafiles/xsaves/xsaves-isa.txt", "XSAVES/XRSTORS"),
    ("datafiles/cldemote/cldemote-isa.xed.txt", "CLDEMOTE"),
    ("datafiles/movdir/movdir-isa.xed.txt", "MOVDIRI/MOVDIR64B"),
    ("datafiles/avx/avx-aes-isa.txt", "VAESIMC"),
    ("datafiles/amd/amdxop/amd-xop-isa.txt", "XOP"),
    ("datafiles/amd/amdxop/amd-vpermil2-isa.txt", "VPERMIL2"),
    ("datafiles/amd/amdxop/amd-fma4-isa.txt", "FMA4"),
    ("datafiles/amd/xed-amd-3dnow.txt", "3DNow!/MMX extensions"),
    # EVEX.  The encoder's EVEX rows are AVX-512 forms of instructions that also
    # have VEX rows (`vpxord`, `vprold`, `vmovdqa32`), so without the AVX-512
    # description the catalogue would have no encoding with `family: evex` at
    # all and those rows could only be reported as unmodelled.
    ("datafiles/avx512f/avx512-foundation-isa.xed.txt", "AVX-512 F (+ EVEX encodings)"),
    # Exhaustiveness pass: the classes the encoder emits that the files above do
    # not carry at all.  Each one is here because a modelled row named it and the
    # catalogue had no class to compare against -- gather (VEX), LZCNT/TZCNT,
    # ADCX/ADOX, the SHSTK/WRSS family, RDRAND/RDSEED and APX (PUSH2/POP2/JMPABS).
    ("datafiles/hswavx/gather-isa.txt", "VGATHER* (VEX/VSIB)"),
    ("datafiles/hswavx/movnt-load-isa.txt", "VMOVNTDQA"),
    ("datafiles/hsw/lzcnt-isa.xed.txt", "LZCNT"),
    ("datafiles/hswbmi/tzcnt-isa.xed.txt", "TZCNT"),
    ("datafiles/bdw/adox-adcx-isa.xed.txt", "ADCX/ADOX"),
    ("datafiles/cet/cet-isa.xed.txt", "SHSTK/CET: WRSS/WRUSS/RSTORSSP/CLRSSBSY"),
    ("datafiles/rdrand/rdrand-isa.xed.txt", "RDRAND/RDSEED"),
    ("datafiles/apx-f/apx-f-isa.xed.txt", "APX: PUSH2/POP2/JMPABS/CCMP + NF/ND forms"),
    ("datafiles/rdseed/rdseed-isa.xed.txt", "RDSEED"),
    ("datafiles/hsw/invpcid-isa.xed.txt", "INVPCID (legacy 66 0F 38 82)"),
    ("datafiles/ivbavx/fp16-isa.txt", "F16C: VCVTPH2PS / VCVTPS2PH (VEX)"),
    ("datafiles/wrmsrns/wrmsrns-isa.xed.txt", "WRMSRNS"),
    ("datafiles/user-msr/user-msr-isa.xed.txt", "URDMSR/UWRMSR (VEX map 7)"),
    ("datafiles/avx-ifma/avx-ifma-isa.xed.txt", "VPMADD52* (IFMA, VEX 0F38 W1)"),
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

# The 64-bit and 32-bit encoders route most of their remaining mnemonics through
# small helpers that fix the map and the mandatory prefix and take the opcode (or
# the ModR/M digit) as an argument.  Each entry below was read out of the helper
# body: `map` is the `V0F`/`0F 38`/`0F 3A` the helper emits, `pp` is the
# mandatory prefix it pushes (or the one its argument selects), `reg` is the
# ModR/M digit for the `/digit` forms, and `opcode` names the argument carrying
# the opcode byte.  Entries that can emit more than one encoding list one spec
# per encoding, because `verify` requires *all* of them to be legal.
LEGACY_SPECS = {
    # ---- x87 ------------------------------------------------------------
    "encode_fcom_mem": {"family": "legacy", "map": ("const", "none"),
                        "pp": ("const", "none"), "opcode": ("arg", 1),
                        "reg": ("arg", 2)},
    "encode_fld_st": {"family": "legacy", "map": ("const", "none"),
                      "pp": ("const", "none"), "opcode": 0xD9,
                      "reg": ("const", 0)},
    "encode_fstp_st": {"family": "legacy", "map": ("const", "none"),
                       "pp": ("const", "none"), "opcode": 0xDD,
                       "reg": ("const", 3)},
    "encode_fxch": {"family": "legacy", "map": ("const", "none"),
                    "pp": ("const", "none"), "opcode": 0xD9,
                    "reg": ("const", 1)},
    "encode_fcom": {"family": "legacy", "map": ("const", "none"),
                    "pp": ("const", "none"), "opcode": 0xD8,
                    "reg": ("const", 2)},
    "encode_fcomp": {"family": "legacy", "map": ("const", "none"),
                     "pp": ("const", "none"), "opcode": 0xD8,
                     "reg": ("const", 3)},
    # `fcomi`/`fucomi` are DB /6 and DB /5: the digit comes from the base byte.
    "encode_fcomi": {"family": "legacy", "map": ("const", "none"),
                     "pp": ("const", "none"), "opcode": 0xDB,
                     "reg": ("x87_base", 1)},
    "encode_fcomip": {"family": "legacy", "map": ("const", "none"),
                      "pp": ("const", "none"), "opcode": 0xDF,
                      "reg": ("const", 6)},
    "encode_fucomip": {"family": "legacy", "map": ("const", "none"),
                       "pp": ("const", "none"), "opcode": 0xDF,
                       "reg": ("const", 5)},
    # `fnstsw` is DF E0 (`%ax`) or DD /7 (memory); `fstsw` is the same with the
    # WAIT byte in front, which does not change the instruction class.
    "encode_fnstsw": [{"family": "legacy", "map": ("const", "none"),
                       "pp": ("const", "none"), "opcode": 0xDF,
                       "reg": ("const", 4)},
                      {"family": "legacy", "map": ("const", "none"),
                       "pp": ("const", "none"), "opcode": 0xDD,
                       "reg": ("const", 7)}],
    "encode_fstsw": [{"family": "legacy", "map": ("const", "none"),
                      "pp": ("const", "none"), "opcode": 0xDF,
                      "reg": ("const", 4)},
                     {"family": "legacy", "map": ("const", "none"),
                      "pp": ("const", "none"), "opcode": 0xDD,
                      "reg": ("const", 7)}],
    # ---- 0F / 0F38 / 0F3A system, cache and state ------------------------
    "encode_clflush": {"family": "legacy", "map": ("const", "0f"),
                       "pp": ("const", "none"), "opcode": 0xAE,
                       "reg": ("const", 7)},
    "encode_invlpg": {"family": "legacy", "map": ("const", "0f"),
                      "pp": ("const", "none"), "opcode": 0x01,
                      "reg": ("const", 7)},
    "encode_verw": {"family": "legacy", "map": ("const", "0f"),
                    "pp": ("const", "none"), "opcode": 0x00,
                    "reg": ("const", 5)},
    "encode_lmsw": {"family": "legacy", "map": ("const", "0f"),
                    "pp": ("const", "none"), "opcode": 0x01,
                    "reg": ("const", 6)},
    "encode_smsw": {"family": "legacy", "map": ("const", "0f"),
                    "pp": ("const", "none"), "opcode": 0x01,
                    "reg": ("const", 4)},
    "encode_sldt": {"family": "legacy", "map": ("const", "0f"),
                    "pp": ("const", "none"), "opcode": 0x00,
                    "reg": ("const", 0)},
    "encode_lsl": {"family": "legacy", "map": ("const", "0f"),
                   "pp": ("const", "none"), "opcode": 0x03},
    "encode_lar": {"family": "legacy", "map": ("const", "0f"),
                   "pp": ("const", "none"), "opcode": 0x02},
    "encode_system_reg16": {"family": "legacy", "map": ("const", "0f"),
                            "pp": ("const", "none"), "opcode": 0x00,
                            "reg": ("arg", 1)},
    "encode_cmpxchg8b": {"family": "legacy", "map": ("const", "0f"),
                         "pp": ("const", "none"), "opcode": 0xC7,
                         "reg": ("const", 1)},
    "encode_fxsaveq": {"family": "legacy", "map": ("const", "0f"),
                       "pp": ("const", "none"), "opcode": 0xAE,
                       "reg": ("const", 0)},
    "encode_fxrstorq": {"family": "legacy", "map": ("const", "0f"),
                        "pp": ("const", "none"), "opcode": 0xAE,
                        "reg": ("const", 1)},
    "encode_movnti": {"family": "legacy", "map": ("const", "0f"),
                      "pp": ("const", "none"), "opcode": 0xC3},
    "encode_movnti_q": {"family": "legacy", "map": ("const", "0f"),
                        "pp": ("const", "none"), "opcode": 0xC3},
    "encode_prefetch": {"family": "legacy", "map": ("const", "0f"),
                        "pp": ("const", "none"), "opcode": 0x18,
                        "reg": ("arg", 1)},
    "encode_prefetch_0f0d": {"family": "legacy", "map": ("const", "0f"),
                             "pp": ("const", "none"), "opcode": 0x0D,
                             "reg": ("arg", 1)},
    "encode_sse_mem_only": {"family": "legacy", "map": ("const", "0f"),
                            "pp": ("const", "none"), "opcode": 0xAE,
                            "reg": ("arg", 1)},
    "encode_rdrand_rdseed": {"family": "legacy", "map": ("const", "0f"),
                             "pp": ("const", "none"), "opcode": 0xC7,
                             "reg": ("arg", 1)},
    "encode_rdrand": {"family": "legacy", "map": ("const", "0f"),
                      "pp": ("const", "none"), "opcode": ("arg", 2),
                      "reg": ("arg", 3)},
    "encode_bit_scan": {"family": "legacy", "map": ("const", "0f"),
                        "pp": ("const", "none"), "opcode": ("arg", 2)},
    "encode_invpcid": {"family": "legacy", "map": ("const", "0f38"),
                       "pp": ("const", "66"), "opcode": 0x82},
    # SHSTK: `rstorssp` is F3-prefixed 0F 01 /5, `clrssbsy` F3-prefixed 0F AE /6.
    "encode_rstorssp": {"family": "legacy", "map": ("const", "0f"),
                        "pp": ("const", "f3"), "opcode": 0x01,
                        "reg": ("const", 5)},
    "encode_clrssbsy": {"family": "legacy", "map": ("const", "0f"),
                        "pp": ("const", "f3"), "opcode": 0xAE,
                        "reg": ("const", 6)},
    # ---- MMX/SSE spellings that emit one of two opcode sets --------------
    "encode_movd": [{"family": "legacy", "map": ("const", "0f"),
                     "pp": ("const", "none"), "opcode": 0x6E},
                    {"family": "legacy", "map": ("const", "0f"),
                     "pp": ("const", "none"), "opcode": 0x7E},
                    {"family": "legacy", "map": ("const", "0f"),
                     "pp": ("const", "66"), "opcode": 0x6E},
                    {"family": "legacy", "map": ("const", "0f"),
                     "pp": ("const", "66"), "opcode": 0x7E}],
    "encode_movq": [{"family": "legacy", "map": ("const", "0f"),
                     "pp": ("const", "66"), "opcode": 0xD6},
                    {"family": "legacy", "map": ("const", "0f"),
                     "pp": ("const", "f3"), "opcode": 0x7E},
                    {"family": "legacy", "map": ("const", "0f"),
                     "pp": ("const", "none"), "opcode": 0x6F},
                    {"family": "legacy", "map": ("const", "0f"),
                     "pp": ("const", "none"), "opcode": 0x7F}],
    "encode_pextrw": [{"family": "legacy", "map": ("const", "0f"),
                       "pp": ("const", "none"), "opcode": 0xC5},
                      {"family": "legacy", "map": ("const", "0f3a"),
                       "pp": ("const", "66"), "opcode": 0x15}],
    "encode_pinsrw": [{"family": "legacy", "map": ("const", "0f"),
                       "pp": ("const", "none"), "opcode": 0xC4},
                      {"family": "legacy", "map": ("const", "0f"),
                       "pp": ("const", "66"), "opcode": 0xC4}],
    "encode_extractps": {"family": "legacy", "map": ("const", "0f3a"),
                         "pp": ("const", "66"), "opcode": 0x17},
    "encode_sse_byte_shift": {"family": "legacy", "map": ("const", "0f"),
                              "pp": ("const", "66"), "opcode": 0x73,
                              "reg": ("arg", 1)},
    "encode_unary_rm_66": {"family": "legacy", "map": ("const", "none"),
                           "pp": ("const", "none"),
                           "opcode": ("size_map", {1: 0xF6, 2: 0xF7, 4: 0xF7,
                                                   8: 0xF7}, 2),
                           "reg": ("arg", 1)},
    # ---- dual-encoded legacy pairs ---------------------------------------
    "encode_movbe": [{"family": "legacy", "map": ("const", "0f38"),
                      "pp": ("const", "none"), "opcode": 0xF0},
                     {"family": "legacy", "map": ("const", "0f38"),
                      "pp": ("const", "none"), "opcode": 0xF1}],
    "encode_crc32": [{"family": "legacy", "map": ("const", "0f38"),
                      "pp": ("const", "f2"), "opcode": 0xF0},
                     {"family": "legacy", "map": ("const", "0f38"),
                      "pp": ("const", "f2"), "opcode": 0xF1}],
    "encode_wrss": {"family": "legacy", "map": ("const", "0f38"),
                    "pp": ("const", "none"), "opcode": 0xF6},
    "encode_adx": {"family": "legacy", "map": ("const", "0f38"),
                   "pp": ("argppbyte", 1), "opcode": 0xF6},
    "encode_xchg": [{"family": "legacy", "map": ("const", "none"),
                     "pp": ("const", "none"), "opcode": 0x86},
                    {"family": "legacy", "map": ("const", "none"),
                     "pp": ("const", "none"), "opcode": 0x87},
                    {"family": "legacy", "map": ("const", "none"),
                     "pp": ("const", "none"), "opcode": 0x90,
                     "opreg": True}],
    # ---- relative and indirect control transfer -------------------------
    "encode_call": [{"family": "legacy", "map": ("const", "none"),
                     "pp": ("const", "none"), "opcode": 0xE8},
                    {"family": "legacy", "map": ("const", "none"),
                     "pp": ("const", "none"), "opcode": 0xFF,
                     "reg": ("const", 2)}],
    "encode_jmp": [{"family": "legacy", "map": ("const", "none"),
                    "pp": ("const", "none"), "opcode": 0xE9},
                   {"family": "legacy", "map": ("const", "none"),
                    "pp": ("const", "none"), "opcode": 0xFF,
                    "reg": ("const", 4)}],
    # `push`/`pop` short forms are `50+rd`/`58+rd`: one opcode, the register in
    # the low three bits.  The immediate and r/m forms are their own opcodes.
    "encode_push": [{"family": "legacy", "map": ("const", "none"),
                     "pp": ("const", "none"), "opcode": 0x50},
                    {"family": "legacy", "map": ("const", "none"),
                     "pp": ("const", "none"), "opcode": 0x6A},
                    {"family": "legacy", "map": ("const", "none"),
                     "pp": ("const", "none"), "opcode": 0x68},
                    {"family": "legacy", "map": ("const", "none"),
                     "pp": ("const", "none"), "opcode": 0xFF,
                     "reg": ("const", 6)},
                    # `push %fs` / `push %gs` are the only segment pushes long
                    # mode keeps (`0F A0` / `0F A8`).
                    {"family": "legacy", "map": ("const", "0f"),
                     "pp": ("const", "none"), "opcode": 0xA0},
                    {"family": "legacy", "map": ("const", "0f"),
                     "pp": ("const", "none"), "opcode": 0xA8}],
    "encode_pop": [{"family": "legacy", "map": ("const", "none"),
                    "pp": ("const", "none"), "opcode": 0x58},
                   {"family": "legacy", "map": ("const", "none"),
                    "pp": ("const", "none"), "opcode": 0x8F,
                    "reg": ("const", 0)},
                   {"family": "legacy", "map": ("const", "0f"),
                    "pp": ("const", "none"), "opcode": 0xA1},
                   {"family": "legacy", "map": ("const", "0f"),
                    "pp": ("const", "none"), "opcode": 0xA9}],
    "encode_push16": [{"family": "legacy", "map": ("const", "none"),
                       "pp": ("const", "none"), "opcode": 0x50},
                      {"family": "legacy", "map": ("const", "none"),
                       "pp": ("const", "none"), "opcode": 0x68},
                      {"family": "legacy", "map": ("const", "none"),
                       "pp": ("const", "none"), "opcode": 0xFF,
                       "reg": ("const", 6)}],
    "encode_pop16": [{"family": "legacy", "map": ("const", "none"),
                      "pp": ("const", "none"), "opcode": 0x58},
                     {"family": "legacy", "map": ("const", "none"),
                      "pp": ("const", "none"), "opcode": 0x8F,
                      "reg": ("const", 0)}],
    # ---- misc ------------------------------------------------------------
    "encode_lea": {"family": "legacy", "map": ("const", "none"),
                   "pp": ("const", "none"), "opcode": 0x8D},
    "encode_nop_rm": {"family": "legacy", "map": ("const", "0f"),
                      "pp": ("const", "none"), "opcode": 0x1F,
                      "reg": ("const", 0)},
    "encode_bswap": {"family": "legacy", "map": ("const", "0f"),
                     "pp": ("const", "none"), "opcode": 0xC8},
    # Far call/jump: `FF /3` and `FF /5` plus the immediate far pointers 9A/EA.
    "encode_lcall": [{"family": "legacy", "map": ("const", "none"),
                      "pp": ("const", "none"), "opcode": 0xFF,
                      "reg": ("const", 3)},
                     {"family": "legacy", "map": ("const", "none"),
                      "pp": ("const", "none"), "opcode": 0x9A}],
    "encode_ljmp": [{"family": "legacy", "map": ("const", "none"),
                     "pp": ("const", "none"), "opcode": 0xFF,
                     "reg": ("const", 5)},
                    {"family": "legacy", "map": ("const", "none"),
                     "pp": ("const", "none"), "opcode": 0xEA}],
    # movabs: REX.W + B8+rd with a 64-bit immediate, i.e. the MOV class.
    "encode_movabs": {"family": "legacy", "map": ("const", "none"),
                      "pp": ("const", "none"), "opcode": 0xB8,
                      "opreg": True},
    "encode_int": {"family": "legacy", "map": ("const", "none"),
                   "pp": ("const", "none"), "opcode": 0xCD},
    "encode_cvtsi2x": {"family": "legacy", "map": ("const", "0f"),
                       "pp": ("argpp", 2), "opcode": ("arg", 1)},
}

# The VEX helpers take the opcode map and the mandatory prefix as arguments in
# the 64-bit encoder (`mm` 1..3, `pp` 0..3) and select the map in the body in the
# 32-bit one.  Both are modelled with the same rules: whichever of the two is an
# argument carries a rule, the other one is a constant from the body.
VEX_SPECS = {
    "encode_avx_cvt_to_gp": {"family": "vex", "map": ("const", "0f"),
                             "pp": ("argpp", 2), "w": ("const", 0),
                             "opcode": ("arg", 1)},
    "encode_avx_cvt_from_gp": {"family": "vex", "map": ("const", "0f"),
                               "pp": ("argpp", 2), "w": ("argw", 3),
                               "opcode": ("arg", 1)},
    "encode_avx_gather": {"family": "vex", "map": ("const", "0f38"),
                          "pp": ("const", "66"), "w": ("argw", 2),
                          "opcode": ("arg", 1)},
    "encode_avx_store": {"family": "vex", "map": ("const", "0f"),
                         "pp": ("boolpp", 2), "w": ("const", 0),
                         "opcode": ("arg", 1)},
    "encode_avx_extract_gp": {"family": "vex", "map": ("const", "0f"),
                              "pp": ("boolpp", 2), "w": ("const", 0),
                              "opcode": ("arg", 1)},
    "encode_avx_cmp_scalar": {"family": "vex", "map": ("const", "0f"),
                              "pp": ("argpp", 2), "w": ("const", 0),
                              "opcode": ("arg", 1)},
    "encode_avx_2op_3a_pp_imm8": {"family": "vex", "map": ("const", "0f3a"),
                                  "pp": ("argpp", 2), "w": ("const", 0),
                                  "opcode": ("arg", 1)},
    "encode_avx_2op_0f_imm8": {"family": "vex", "map": ("const", "0f"),
                               "pp": ("argpp", 2), "w": ("const", 0),
                               "opcode": ("arg", 1)},
    "encode_avx_shuffle": {"family": "vex", "map": ("const", "0f"),
                           "pp": ("argpp", 2), "w": ("const", 0),
                           "opcode": ("arg", 1)},
    "encode_avx_shuffle_3a_w1": {"family": "vex", "map": ("const", "0f3a"),
                                 "pp": ("boolpp", 2), "w": ("const", 1),
                                 "opcode": ("arg", 1)},
    "encode_avx_extract_imm8": {"family": "vex", "map": ("const", "0f3a"),
                                "pp": ("boolpp", 2), "w": ("const", 0),
                                "opcode": ("arg", 1)},
    "encode_avx_extract_gpr_imm8": {"family": "vex", "map": ("const", "0f3a"),
                                    "pp": ("boolpp", 2), "w": ("const", 0),
                                    "opcode": ("arg", 1)},
    "encode_avx_extract_byte": {"family": "vex", "map": ("const", "0f3a"),
                                "pp": ("boolpp", 2), "w": ("const", 0),
                                "opcode": ("arg", 1)},
    "encode_avx_insert_gp": {"family": "vex", "map": ("const", "0f3a"),
                             "pp": ("boolpp", 2), "w": ("const", 0),
                             "opcode": ("arg", 1)},
    "encode_avx_insert_gp_0f": {"family": "vex", "map": ("const", "0f"),
                                "pp": ("boolpp", 2), "w": ("const", 0),
                                "opcode": ("arg", 1)},
    "encode_avx_2op_38_pp": {"family": "vex", "map": ("const", "0f38"),
                             "pp": ("argpp", 2), "w": ("const", 0),
                             "opcode": ("arg", 1)},
    "encode_avx_maskmov": [{"family": "vex", "map": ("const", "0f38"),
                            "pp": ("const", "66"), "w": ("argw", 3),
                            "opcode": ("arg", 1)},
                           {"family": "vex", "map": ("const", "0f38"),
                            "pp": ("const", "66"), "w": ("argw", 3),
                            "opcode": ("arg", 2)}],
    "encode_avx_scalar_mov": [{"family": "vex", "map": ("const", "0f"),
                               "pp": ("argpp", 3), "w": ("const", 0),
                               "opcode": ("arg", 1)},
                              {"family": "vex", "map": ("const", "0f"),
                               "pp": ("argpp", 3), "w": ("const", 0),
                               "opcode": ("arg", 2)}],
    # `pp = if is_66 { 1 } else { 2 }`: vmovdqa is 66, vmovdqu is F3.
    "encode_avx_mov": [{"family": "vex", "map": ("const", "0f"),
                        "pp": ("boolpp", 3, "f3"), "w": ("const", 0),
                        "opcode": ("arg", 1)},
                       {"family": "vex", "map": ("const", "0f"),
                        "pp": ("boolpp", 3, "f3"), "w": ("const", 0),
                        "opcode": ("arg", 2)}],
    "encode_bmi_andn": {"family": "vex", "map": ("const", "0f38"),
                        "pp": ("const", "none"), "w": ("argw", 1),
                        "opcode": 0xF2},
    "encode_bmi2_rorx": {"family": "vex", "map": ("const", "0f3a"),
                         "pp": ("const", "f2"), "w": ("argw", 1),
                         "opcode": 0xF0},
    "encode_bmi_blsx": {"family": "vex", "map": ("const", "0f38"),
                        "pp": ("const", "none"), "w": ("argw", 2),
                        "opcode": 0xF3, "reg": ("arg", 1)},
    "encode_vex_comis": {"family": "vex", "map": ("const", "0f"),
                         "pp": ("argpp", 2), "w": ("const", 0),
                         "opcode": ("arg", 1)},
    "encode_avx_3op_0f_imm8_shift": {"family": "vex", "map": ("const", "0f"),
                                     "pp": ("const", "66"), "w": ("const", 0),
                                     "opcode": ("arg", 1),
                                     "reg": ("arg", 2)},
    "encode_avx_vcvtps2ph": {"family": "vex", "map": ("const", "0f3a"),
                             "pp": ("const", "66"), "w": ("const", 0),
                             "opcode": 0x1D},
    "encode_vnni_int16_vex": {"family": "vex", "map": ("const", "0f38"),
                              "pp": ("const", "f3"), "w": ("const", 0),
                              "opcode": ("arg", 2)},
    "encode_vpermil2": {"family": "vex", "map": ("const", "0f3a"),
                        "pp": ("const", "66"), "w": ("const", 0),
                        "opcode": ("arg", 1)},
    "encode_avx_extract_w": [{"family": "vex", "map": ("const", "0f"),
                              "pp": ("const", "66"), "w": ("const", 0),
                              "opcode": 0xC5},
                             {"family": "vex", "map": ("const", "0f3a"),
                              "pp": ("const", "66"), "w": ("const", 0),
                              "opcode": 0x15}],
    "encode_avx_movd": [{"family": "vex", "map": ("const", "0f"),
                         "pp": ("const", "66"), "w": ("const", 0),
                         "opcode": 0x6E},
                        {"family": "vex", "map": ("const", "0f"),
                         "pp": ("const", "66"), "w": ("const", 0),
                         "opcode": 0x7E}],
    "encode_avx_movq": [{"family": "vex", "map": ("const", "0f"),
                         "pp": ("const", "66"), "w": ("const", 1),
                         "opcode": 0x6E},
                        {"family": "vex", "map": ("const", "0f"),
                         "pp": ("const", "66"), "w": ("const", 1),
                         "opcode": 0x7E},
                        {"family": "vex", "map": ("const", "0f"),
                         "pp": ("const", "66"), "w": ("const", 1),
                         "opcode": 0xD6},
                        {"family": "vex", "map": ("const", "0f"),
                         "pp": ("const", "f3"), "w": ("const", 0),
                         "opcode": 0x7E}],
    # EVEX: `vpxord`/`vpxorq`/`vpandd` are EVEX.66.0F with W from the row, and
    # `vprol*`/`vpror*` put the rotation in ModR/M.reg of EVEX.66.0F 72.
    "encode_evex_3op": {"family": "evex", "map": ("const", "0f"),
                        "pp": ("argpp", 2), "w": ("argw", 3),
                        "opcode": ("arg", 1)},
    "encode_evex_rotate_imm": {"family": "evex", "map": ("const", "0f"),
                               "pp": ("const", "66"), "w": ("argw", 3),
                               "opcode": ("arg", 1), "reg": ("arg", 2)},
    # APX: `push2`/`pop2` are EVEX map 4 with ND=1 and W0/W1 selecting
    # PUSH2/POP2 vs PUSH2P/POP2P; `jmpabs` is the REX2-prefixed one-byte 0xA1.
    "encode_push2": {"family": "evex", "map": ("const", "map4"),
                     "pp": ("const", "none"), "w": ("boolw", 1),
                     "opcode": 0xFF, "reg": ("const", 6)},
    "encode_pop2": {"family": "evex", "map": ("const", "map4"),
                    "pp": ("const", "none"), "w": ("boolw", 1),
                    "opcode": 0x8F, "reg": ("const", 0)},
    "encode_jmpabs": {"family": "legacy", "map": ("const", "none"),
                      "pp": ("const", "none"), "opcode": 0xA1},
    "encode_evex_vmovdqa": [{"family": "evex", "map": ("const", "0f"),
                             "pp": ("const", "66"),
                             "w": ("boolw", 1),
                             "opcode": 0x6F},
                            {"family": "evex", "map": ("const", "0f"),
                             "pp": ("const", "66"),
                             "w": ("boolw", 1),
                             "opcode": 0x7F}],
}

def _literal_opcodes(specs: dict) -> dict:
    """Turn a bare `"opcode": 0x8D` into an explicit constant rule.

    `HELPER_SPEC` reads a bare integer opcode as an *argument index* (the EVEX
    helpers take the opcode as their fifth parameter), so the tables above spell
    their constants as `("const", ..)`; this lets them stay readable.
    """
    out = {}
    for name, spec in specs.items():
        many = isinstance(spec, list)
        fixed = []
        for one in (spec if many else [spec]):
            one = dict(one)
            if isinstance(one.get("opcode"), int):
                one["opcode"] = ("const", one["opcode"])
            fixed.append(one)
        out[name] = fixed if many else fixed[0]
    return out


HELPER_SPEC.update(_literal_opcodes(LEGACY_SPECS))
HELPER_SPEC.update(_literal_opcodes(VEX_SPECS))

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
    # XED writes the same three mandatory-prefix markers two ways: the SSE-era
    # files use `refining_f3`, the newer ones `f3_refining_prefix`, and
    # `f2_refining_prefix`/`refining_f2` likewise.  Missing both spellings made
    # TZCNT look like a bare `0F BC`, i.e. like BSF's encoding.
    "refining_f2": "f2",
    "refining_f3": "f3",
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
    # AT&T swaps the two non-commutative pop forms relative to the Intel
    # mnemonics: GAS `fsubp` is SDM FSUBRP (DE /4) and GAS `fdivp` is SDM
    # FDIVRP (DE /6).  Verified against as 2.47: `fsubp` = de e1, `fsubrp` =
    # de e9, `fdivp` = de f1, `fdivrp` = de f9.
    "fsubp": "FSUBRP", "fsubrp": "FSUBP",
    "fdivp": "FDIVRP", "fdivrp": "FDIVP",
    # MOVSX/MOVZX spell the destination width in the AT&T suffix, which is not
    # part of the class name; the 32->64 form is its own class (MOVSXD).
    # APX `pushp`/`popp` are the REX2-prefixed short push/pop forms, i.e. the
    # PUSH/POP classes rather than classes of their own.
    "pushp": "PUSH", "popp": "POP",
    "movsbl": "MOVSX", "movsbw": "MOVSX", "movsbq": "MOVSX",
    "movswl": "MOVSX", "movswq": "MOVSX", "movslq": "MOVSXD",
    "movzbl": "MOVZX", "movzbw": "MOVZX", "movzbq": "MOVZX",
    "movzwl": "MOVZX", "movzwq": "MOVZX", "movzlq": "MOVZX",
    "movsbl": "MOVSX", "movswl": "MOVSX", "movsbw": "MOVSX", "movzb": "MOVZX",
    "movzbl": "MOVZX", "movzwl": "MOVZX", "movzbw": "MOVZX",
    # The suffix-less spellings infer the width from the operands.
    "movzb": "MOVZX", "movzw": "MOVZX",
    "cltd": "CDQ", "cwtl": "CWDE", "cbtw": "CBW", "cwtd": "CWD",
    "popf": "POPF", "pushf": "PUSHF", "popa": "POPA", "pusha": "PUSHA",
    # XED's classes carry the near/far distinction in the *name*: `CALL_NEAR`,
    # `CALL_FAR`, `JMP_FAR`, `RET_FAR`.
    "lcall": "CALL_FAR", "ljmp": "JMP_FAR",
    "movabs": "MOV", "movabsq": "MOV", "movabsl": "MOV",
    # The 32-bit/64-bit string forms are their own classes in XED, and AT&T
    # spells them with an `l` suffix that the class name does not have.
    "movsl": "MOVSD", "cmpsl": "CMPSD", "stosl": "STOSD",
    "lodsl": "LODSD", "scasl": "SCASD", "insl": "INSD", "outsl": "OUTSD",
    "call": "CALL_NEAR", "callq": "CALL_NEAR", "calll": "CALL_NEAR",
    "callw": "CALL_NEAR", "ret": "RET_NEAR", "retq": "RET_NEAR",
    "retl": "RET_NEAR", "retw": "RET_NEAR",
    "lret": "RET_FAR", "lretq": "RET_FAR", "lretw": "RET_FAR",
    "ljmpl": "JMP_FAR", "ljmpw": "JMP_FAR", "ljmpq": "JMP_FAR",
    "lcalll": "CALL_FAR", "lcallw": "CALL_FAR", "lcallq": "CALL_FAR",
    # XED splits the SSE4.1 `pextrw $imm8` form from the SSE2 register form.
    "pextrw": ["PEXTRW", "PEXTRW_SSE4"],
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


# Rows whose left-hand side is not a mnemonic.  Two shapes produce them: a
# segment-register arm of a `push`/`pop` helper (`"fs" => push %fs`) and the
# suffix arm of the `vcmpXX{ps,pd,ss,sd}` pseudo-op rewriter (`"ps" => ...`).
# They encode real instructions, but they name an operand or a suffix, so the
# SDM's instruction classes cannot classify them; `fs`/`gs` are checked by the
# as-2.47 gate, which does carry them as mnemonics.
NOT_A_MNEMONIC = frozenset({
    "fs", "gs", "cs", "ds", "es", "ss",
    "ps", "pd", "sd", "b", "w", "l", "q",
    # Relocation kinds the 64-bit encoder dispatches on (`self.gotpcrel_x_type()`
    # and friends): they are suffixes of a memory operand, not instructions.
    "gotpcrel", "gottpoff", "tlsdesc", "gotpcrelx",
})


# Instruction classes the Intel SDM documents but XED's datafiles do not model.
# Each entry needs a provenance line and a byte-level assertion elsewhere in the
# test suite (the as-2.47 differential for `prefetchw` is in
# `tests/asm-diff/misc.casefile`), so this table cannot be used to paper over a
# wrong opcode: it only supplies the *reference*, the bytes are still checked.
SDM_ONLY_CLASSES = {
    # SDM Vol. 2B "PREFETCHW - Prefetch Data into Caches in Anticipation of a
    # Write": 0F 0D /1, CPUID.(EAX=80000001H:ECX).PREFETCHW.  AMD's ABM
    # extension, adopted by Intel; XED ships no PREFETCHW ICLASS at all.
    "PREFETCHW": [{"family": "legacy", "map": "0f", "pp": "none", "w": None,
                   "opcode": 0x0D, "modrm": True, "reg": 1}],
}


def with_sdm_only_classes(catalogue: dict) -> dict:
    """Merge `SDM_ONLY_CLASSES` into a loaded catalogue.

    Done at load time rather than in `--build-catalogue`, so the generated file
    keeps saying what it is: a transcription of XED.
    """
    for name, encodings in SDM_ONLY_CLASSES.items():
        known = catalogue.setdefault(name, [])
        for enc in encodings:
            if enc not in known:
                known.append(enc)
    return catalogue


def _upper_class(mnem: str) -> list[str]:
    """Candidate SDM instruction classes for an AT&T mnemonic."""
    m = mnem.lower()
    cands = []
    if m == "setcc":
        # `setcc` is the parameterised row of the whole SETcc family: the
        # condition is an argument, so every condition class is a candidate and
        # the emitted opcode has to be *some* legal `0F 90+cc`.
        return sorted("SET" + c.upper() for c in (
            "o", "no", "b", "ae", "e", "ne", "be", "a",
            "s", "ns", "p", "np", "l", "ge", "le", "g"))
    if m in ALIASES:
        alias = ALIASES[m]
        cands.extend(alias if isinstance(alias, list) else [alias])
    # The literal spelling comes before the suffix-stripped one, because a
    # mnemonic can end in a size letter and still name its own class:
    # `pslldq`/`psrldq` are PSLLDQ/PSRLDQ, and stripping the `q` would compare
    # them against the PSLLD shift class instead.
    if m.upper() not in cands:
        cands.append(m.upper())
    for pat in (r"[bwlq]$", r"[bwlqt]$"):
        stripped = re.sub(pat, "", m)
        if stripped.upper() not in cands:
            cands.append(stripped.upper())
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
            if not ic:
                continue
            # One ICLASS block carries one PATTERN *per operand form*: MOVUPS
            # lists 0x0F 0x10 and 0x0F 0x11 in the same block, and a parser that
            # reads only the first one records the load direction and reports
            # every store row in the encoder as an invented encoding.  XED
            # writes the register and memory forms as separate patterns too, so
            # this is the difference between modelling half the instruction and
            # half of its patterns.
            for pat in re.finditer(r"^PATTERN\s*:\s*(.+)$", block, re.M):
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
    # XED marks EVEX-encoded (AVX-512/APX) patterns with `EVV`/`EVX`/`EVEX*`
    # tokens and writes the EVEX W bit as a bare `W0`/`W1`; the map and the
    # mandatory prefix use the same `V0F*`/`V66`-style tokens as VEX.
    if any(t.startswith("EVEX") or t in ("EVV", "EVX") for t in toks):
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
    for pos, tok in enumerate(toks):
        # XED spells "the opcode carries the register (or the condition code) in
        # its low three bits" as `0xB8+rd` / `0x90+cc` / `0x50+ro`.  The base
        # byte is the opcode; without stripping the suffix the catalogue entry
        # ends up with `opcode: null` and every BSWAP/SETcc/PUSH row that uses
        # the short form has nothing to match against.
        base_tok = re.sub(r"\+[a-z]{2}$", "", tok)
        if re.fullmatch(r"0b[01_]+", tok):
            # `0b1011_0 SRM[rrr]` is how XED writes an opcode whose low bits are
            # the register number: the literal holds the high bits and the
            # `SRM[rrr]` that follows supplies three zero bits (`mov r8, imm8`
            # is `0b1011_0` + `000` = 0xB0).
            bits = tok[2:].replace("_", "")
            if pos + 1 < len(toks) and re.match(r"SR[MB]\[", toks[pos + 1]):
                opcodes.append(int(bits, 2) << 3)
            else:
                opcodes.append(int(bits, 2))
        elif re.fullmatch(r"0x[0-9A-Fa-f]{2}", base_tok):
            opcodes.append(int(base_tok, 16))
        elif tok in ("V0F38", "V0F3A", "V0F"):
            map_ = tok[1:].lower()
        elif tok in ("MAP4", "VMAP4", "MAP5", "MAP6", "MAP7"):
            # APX/AVX-512/APX-FP use EVEX/VEX maps 4..7; their opcodes are not in
            # any of the 0F maps, and collapsing them into `0f` would make, say,
            # `urdmsr` (VEX map 7) look like an `0F` instruction.
            map_ = tok.lower().replace("v", "")
        elif tok in ("VW0", "V0", "W0"):
            w = 0
        elif tok in ("VW1", "V1", "W1"):
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


def _balanced(text: str) -> bool:
    depth = 0
    for ch in text:
        if ch in "([{":
            depth += 1
        elif ch in ")]}":
            depth -= 1
    return depth <= 0


def extract_rows(path: Path) -> list[dict]:
    """Every declarative mnemonic row in one encoder source file.

    An arm whose body is longer than one line (`"vroundss" => Self::check(..)
    .and_then(|()| self.encode_..)`) is joined until its brackets balance: the
    encoding call is what the row means, and a body truncated at the first line
    would model the row by its precondition instead of its encoding.
    """
    lines = path.read_text(encoding="utf-8").splitlines()
    rows = []
    for index, line in enumerate(lines):
        lineno = index + 1
        if line.strip().startswith("//"):
            continue
        m = ROW_RE.match(line)
        if not m:
            continue
        names = re.findall(r'"([a-z0-9._]+)"', m.group(1))
        body = m.group(2)
        extra = index
        while extra + 1 < len(lines):
            nxt = lines[extra + 1].strip()
            # A continuation either leaves a bracket open or starts with a
            # method-call/operator that can only belong to the previous
            # expression (`\.and_then(|()| ...)`, `"? +"`).
            if _balanced(body) and not nxt.startswith((".", "?", "|", "&", "+", ",")):
                break
            extra += 1
            body += " " + nxt
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

    The argument positions come from the helper bodies in the encoders, so a
    helper whose parameters are reordered fails here as UNMODELLED instead of
    being modelled with the old layout.

    A spec is a dict of rules, or a list of them when the helper can emit more
    than one legal encoding for the same call (`vmovdqa` loads and stores,
    `movq` has an MMX and an SSE spelling, ...).  Every rule is one of
    `("const", value)`, `("arg", idx)`, `("argpp", idx)` (a mandatory-prefix
    index 0..3), `("boolpp", idx)` (`is_66: bool`), `("argmap", idx)` (a map
    index 1..3), `("argw", idx)`, `("size_map", {size: value}, idx)`,
    `("argbytes_map"|"argbytes_last", idx)` (a byte-slice argument),
    `("kopmask", idx, 0|1)` (AVX-512 mask-register opcodes) or
    `("x87_base", idx)` (the `(base & 0x38) >> 3` digit of an x87 ModR/M byte).
    """
    specs = HELPER_SPEC[name]
    if isinstance(specs, dict):
        specs = [specs]
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
        # Anything that is not a rule tuple is a literal constant (`"w": 1`).
        if not isinstance(rule, tuple):
            return rule
        kind = rule[0]
        if kind == "const":
            return rule[1]
        if kind == "size_map":
            idx = rule[2]
            if idx >= len(vals):
                raise ValueError("missing argument")
            return rule[1].get(int(vals[idx], 0))
        idx = rule[1]
        if idx >= len(vals):
            raise ValueError("missing argument")
        raw = vals[idx]
        if kind == "argmap":
            return METHOD_MAP.get(int(raw, 0))
        if kind == "argpp":
            return PP_BY_INDEX.get(int(raw, 0))
        if kind == "boolpp":
            # Some helpers spell the flag `has_66`/`is_66` but mean "66 or F3"
            # (`encode_avx_mov` passes pp=1 for vmovdqa and pp=2 for vmovdqu),
            # so the false branch names its prefix as a third rule element.
            false_pp = rule[2] if len(rule) > 2 else "none"
            return {"true": "66", "false": false_pp}[raw]
        if kind == "argw":
            return int(raw, 0)
        if kind == "boolw":
            return {"true": 1, "false": 0}[raw]
        if kind == "argppbyte":
            # A helper that pushes the mandatory prefix byte itself
            # (`encode_adx(ops, 0x66, 4)` for ADCX, `0xF3` for ADOX).
            return {0x66: "66", 0xF3: "f3", 0xF2: "f2"}[int(raw, 0)]
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

    encs = []
    for spec in specs:
        # A spec may spell the opcode as a rule (`("arg", 1)`, `("const", "dd")`)
        # or as a bare literal; both mean "this is the opcode byte".
        opcode_rule = spec["opcode"]
        if not isinstance(opcode_rule, tuple):
            opcode_rule = (("arg", opcode_rule) if isinstance(opcode_rule, int)
                           else ("const", opcode_rule))
        try:
            enc = {
                "family": spec["family"],
                "map": take(spec.get("map")),
                "pp": take(spec.get("pp")),
                "w": take(spec.get("w")),
                "opcode": take(opcode_rule),
                "modrm": True,
                "reg": take(spec.get("reg")),
            }
        except (ValueError, KeyError, TypeError):
            return None
        if enc["map"] is None or enc["pp"] is None or enc["opcode"] is None:
            return None
        encs.append(enc)
    return encs or None


def row_is_64bit(row: dict) -> bool:
    """Whether a row comes from the 64-bit encoder.

    The two backends share the SDM-format model but differ in one respect that
    changes the meaning of the bytes: `40..4F` is a REX prefix in 64-bit mode
    and an `inc`/`dec` opcode in 32-bit mode.  Deciding per row from its path
    keeps `--backend all` correct instead of applying one mode to both.
    """
    return "/i686/" not in row["file"].replace("\\", "/")


def model_any(row: dict, name: str | None = None) -> list[dict] | None:
    """Model a row with the mode implied by the backend that declares it."""
    return model_row(row, row_is_64bit(row), name)


def row_helper(body: str) -> str | None:
    """The encoder helper a row ends up calling.

    A row body can be an expression: `Self::check_avx_scalar_xmm(..).and_then(
    |()| self.encode_avx_3op_3a_imm8(..))` validates its operands and *then*
    encodes.  The last `encode_` call is the one that reaches the wire, so that
    is the one modelled; the precondition does not change the bytes.
    """
    calls = re.findall(r"self\.(encode_[a-z0-9_]+)\(", body)
    return calls[-1] if calls else None


def model_row(row: dict, b64: bool = False, name: str | None = None) -> list[dict] | None:
    """Interpret one encoder row as SDM-comparable encodings.

    `name` is the mnemonic being checked.  A row binds several spellings to one
    helper call (`addq,addl,addw,addb` -> `encode_alu`), and the AT&T suffix is
    what selects the operand size, so the encoding is a property of the (row,
    name) pair, not of the row.

    The order matters.  A row can call a modelled helper (nothing else to say),
    name its opcode bytes instead (`self.encode_rm_reg(ops, &[0x88], 0)`), or --
    for x87 -- pass the opcode bytes *and* a ModR/M digit, which the byte array
    alone cannot express.  The digit case is therefore resolved before the byte
    array, and the byte array is the last resort rather than a branch that only
    one helper shape is allowed to reach.
    """
    body = row["body"]
    if name is None:
        # A row is modelled *for a mnemonic*: several helpers take the opcode
        # (or the whole operation) from the spelling.  Defaulting to the row's
        # own first name keeps `model_row(row)` a well-defined call instead of
        # silently reporting every mnemonic-driven row as unmodelled.
        name = row["names"][0] if row["names"] else None
    helper = row_helper(body)
    if helper is not None:
        args = call_args(body, helper)
        if args is not None:
            model = HELPER_MODELS.get(helper)
            if model is not None:
                encs = model(args, name, b64)
                if encs is not None:
                    return encs
            elif helper in HELPER_SPEC:
                encs = _model_spec_helper(helper, body, b64)
                if encs is not None:
                    return encs
            elif helper in HELPERS:
                encs = _model_legacy_args(helper, args)
                if encs is not None:
                    return encs
    # x87 memory forms carry the operation in ModRM.reg, passed as the helper's
    # trailing argument: `self.encode_x87_mem(ops, &[0xDB], 7)`.  The literal
    # byte array names only the opcode, so the extension has to be read from the
    # call before the generic byte-array path claims the row.
    m = re.match(r"self\.encode_(?:x87_mem|x87_wait_mem)\(", body)
    if m and row["arrays"]:
        args = call_args(body, m.group(0)[5:-1].split("(")[0])
        ext = arg_int(args, len(args) - 1) if args else None
        enc = legacy_encoding(row["arrays"][0], b64)
        if enc is not None and ext is not None:
            enc["reg"] = ext
            return [enc]
    # x87 register forms: `DE <modrm>` with the digit fixed by the helper's
    # constant (`self.encode_x87_pop_reg(ops, "faddp", 0xC0)`).
    m = re.match(r'self\.encode_x87_pop_reg\(ops,\s*"[a-z0-9]+",\s*(0x[0-9A-Fa-f]+)', body)
    if m:
        return [form(0xDE, reg=(int(m.group(1), 16) >> 3) & 7)]
    # A row can push a single literal byte with no helper at all -- the segment
    # arm of the 32-bit `push` helper is written that way (`"es" =>
    # self.bytes.push(0x06)`), and the byte *is* the whole encoding.
    m = re.fullmatch(r"self\.bytes\.push\((0x[0-9A-Fa-f]+)\)", body.strip())
    if m:
        return [form(int(m.group(1), 16))]
    # Rows that only name opcode bytes.
    if row["arrays"]:
        encs = []
        for arr in row["arrays"]:
            enc = legacy_encoding(arr, b64)
            if enc is None:
                return None
            encs.append(enc)
        return encs
    return None


def _model_legacy_args(helper: str, args: list[str]) -> list[dict] | None:
    """The `HELPERS` table: a VEX helper whose opcode is its first integer arg."""
    map_, fixed_pp, pp_idx, w = HELPERS[helper]
    opcode = next((arg_int(args, i) for i in range(1, len(args))
                   if arg_int(args, i) is not None), None)
    if opcode is None:
        return None
    pp = fixed_pp
    if pp is None:
        if pp_idx is None or pp_idx >= len(args):
            return None
        raw = args[pp_idx].strip()
        if re.fullmatch(r"\d+", raw):
            pp = PP_BY_INDEX.get(int(raw))
        elif raw in ("true", "false"):
            pp = "66" if raw == "true" else "none"
        else:
            return None
    if pp is None:
        return None
    return [form(opcode, map_=map_, pp=pp, w=w, family="vex")]


# ---------------------------------------------------------------------------
# Per-helper models
# ---------------------------------------------------------------------------
# A declarative spec can only express "this argument is the opcode, that one is
# pp".  About a third of the encoder's rows call a helper whose opcode byte is a
# *function* of its arguments (the operand size, the group digit, the direction
# of the store), and modelling those needs code.  Each model below returns every
# encoding the helper can emit for one call site, because `verify` requires all
# of them to be legal: a wrong group digit, a wrong size byte or a wrong base
# opcode then shows up as a finding instead of passing on one lucky form.
#
# The rules are read out of the helper bodies in `src/backend/*/assembler/
# encoder/`; `tests/corpus/test_gas_encoding_crossref.py` pins the interesting
# cases (a wrong digit, a 16-bit operand size) so a model cannot silently drift.

ARG_CALL = re.compile(r"(?:self\.|Self::)?([a-z_][a-z0-9_]*)\(")
SIZE_SUFFIX_AT = re.compile(r"[bwlq]$")
SIZE_OF_SUFFIX = {"b": 1, "w": 2, "l": 4, "q": 8}


def call_args(body: str, name: str | None = None) -> list[str] | None:
    """Split one call's arguments, balancing brackets and nested calls."""
    start = -1
    if name:
        m = re.search(r"\b" + re.escape(name) + r"\(", body)
        if m:
            start = m.end() - 1
    if start < 0:
        start = body.find("(")
        if start < 0:
            return None
    depth, end = 0, -1
    for i in range(start, len(body)):
        if body[i] in "([":
            depth += 1
        elif body[i] in ")]":
            depth -= 1
            if depth == 0:
                end = i
                break
    if end < 0:
        return None
    args, depth, cur = [], 0, ""
    for ch in body[start + 1 : end]:
        if ch in "([":
            depth += 1
        elif ch in ")]":
            depth -= 1
        if ch == "," and depth == 0:
            args.append(cur.strip())
            cur = ""
        else:
            cur += ch
    args.append(cur.strip())
    return args


RUST_INT = re.compile(r"^(0[xX][0-9A-Fa-f_]+|[0-9][0-9_]*)(?:_?u(?:8|16|32|64|size))?$")


def arg_int(args: list[str], idx: int) -> int | None:
    """`args[idx]` as an integer literal, with Rust's `u8`/`_u8` suffixes."""
    if idx >= len(args):
        return None
    m = RUST_INT.match(args[idx].strip())
    if not m:
        return None
    return int(m.group(1).replace("_", ""), 0)


def arg_bool(args: list[str], idx: int) -> bool | None:
    if idx >= len(args):
        return None
    return {"true": True, "false": False}.get(args[idx].strip())


def name_size(name: str | None) -> int | None:
    """The operand size an AT&T mnemonic suffix spells, or None."""
    if not name or not SIZE_SUFFIX_AT.search(name):
        return None
    return SIZE_OF_SUFFIX[name[-1]]


def form(opcode: int, *, map_: str = "none", pp: str | None = "none",
         w: int | None = None, reg: int | None = None,
         family: str = "legacy") -> dict:
    return {"family": family, "map": map_, "pp": pp, "w": w,
            "opcode": opcode, "modrm": True, "reg": reg}


def _sized(opcodes: dict[int, int], size: int, reg: int | None,
           **kw) -> list[dict]:
    """One encoding for one operand size.

    The `66` in front of a 16-bit legacy form is the operand-size prefix, and
    neither reference records it: GNU as spells the size in the mnemonic and
    leaves the prefix to the encoder, and XED's `osz_refining_prefix` is an
    operand attribute rather than a pattern prefix, so the catalogue carries no
    `pp` for these rows either.  The model therefore reports the opcode-level
    form and leaves `pp`/`w` unset; the operand size it *does* know is checked
    against the mnemonic suffix directly (`SIZE_ARG`), which is the check that
    would catch `negw` wired to a 32-bit size.
    """
    if size not in opcodes:
        return []
    return [form(opcodes[size], reg=reg, **kw)]


def _sizes(b64: bool) -> list[int]:
    return [1, 2, 4, 8] if b64 else [1, 2, 4]


def model_sizes(name: str | None, b64: bool) -> list[int]:
    """Every operand size a row for `name` can encode."""
    size = name_size(name)
    if size is None:
        return _sizes(b64)
    return [size] if size in _sizes(b64) else []


def _model_unary_rm(args, name, b64) -> list[dict] | None:
    """`encode_unary_rm(ops, digit, size)` - Group 3 / Group 5 with a /digit.

    `inc`/`dec` (digit 0/1) are Group 5 (`FE`/`FF`); `not`, `neg`, `mul`,
    `div`, `idiv` are Group 3 (`F6`/`F7`).  The helper picks the table from the
    digit itself, so a row wired to the wrong group is a finding.
    """
    digit, size = arg_int(args, 1), arg_int(args, 2)
    if digit is None or size is None:
        return None
    if digit <= 1:
        opcodes = {1: 0xFE, 2: 0xFF, 4: 0xFF, 8: 0xFF}
    else:
        opcodes = {1: 0xF6, 2: 0xF7, 4: 0xF7, 8: 0xF7}
    return _sized(opcodes, size, digit)


def _model_inc_dec(args, name, b64) -> list[dict] | None:
    """`encode_inc_dec(ops, digit, size)` - Group 5; XED's canonical form.

    The 32-bit `40+rd`/`48+rd` register range is expanded by the GAS snapshot
    gap-report layer; it is not a single opcode in XED's canonical model.
    """
    digit, size = arg_int(args, 1), arg_int(args, 2)
    if digit is None or size is None:
        return None
    return _sized({1: 0xFE, 2: 0xFF, 4: 0xFF, 8: 0xFF}, size, digit)


def _model_div_idiv(args, name, b64) -> list[dict] | None:
    """`encode_div_idiv(ops, digit, size, label)` - Group 3 `div`/`idiv`."""
    digit, size = arg_int(args, 1), arg_int(args, 2)
    if digit is None or size is None:
        return None
    return _sized({1: 0xF6, 2: 0xF7, 4: 0xF7, 8: 0xF7}, size, digit)


def _model_shift(args, name, b64) -> list[dict] | None:
    """`encode_shift(ops, mnemonic, digit)` - Group 2 in all four count forms.

    The opcode selects the count source (`D0/D1` for 1, `D2/D3` for CL,
    `C0/C1` for imm8) and the *digit* is the operation, which occupies the
    ModR/M reg field only in the imm8 forms - the others encode the operand
    there, so their reg field is not comparable.
    """
    digit = arg_int(args, 2)
    if digit is None:
        return None
    forms = []
    for size in model_sizes(name, b64):
        for op_one, op_cl, op_imm in ((0xD0, 0xD2, 0xC0), (0xD1, 0xD3, 0xC1)):
            if (op_one == 0xD0) != (size == 1):
                continue
            for opcode, reg in ((op_one, None), (op_cl, None), (op_imm, digit)):
                forms.extend(_sized({size: opcode}, size, reg))
    return forms or None


def _model_double_shift(args, name, b64) -> list[dict] | None:
    """`encode_double_shift(ops, opcode, size)` - `0F A4/A5` and `0F AC/AD`.

    `shld` passes `A4` and `shrd` `AC`; the CL form is the next opcode, and the
    source register drives ModR/M.reg (so it is not a /digit).
    """
    opcode, size = arg_int(args, 1), arg_int(args, 2)
    if opcode is None or size is None:
        return None
    forms = []
    for op in (opcode, opcode + 1):
        forms.extend(_sized({size: op}, size, None, map_="0f"))
    return forms


MOVSX_OPCODE = {1: 0xBE, 2: 0xBF, 4: 0x63}


def _model_movsx(args, name, b64) -> list[dict] | None:
    """`encode_movsx(ops, from_size, to_size)` - `0F BE`/`0F BF`/`0F 63`."""
    frm, to = arg_int(args, 1), arg_int(args, 2)
    if frm is None or to is None or frm not in MOVSX_OPCODE:
        return None
    # `0F BE`/`0F BF` live in the 0F map; MOVSXD is the one-byte `63` and must
    # not be given the escape, which would make it a different instruction.
    map_ = "none" if frm == 4 else "0f"
    # The destination width picks the REX.W bit; the source width picks the
    # opcode byte.  The 66 prefix in a `movsbl` row would be wrong, so it is
    # fixed at "none" and the `to == 2` case is the only 16-bit one.
    return [form(MOVSX_OPCODE[frm], map_=map_, pp="none", reg=None)]


def _model_alu(args, name, b64) -> list[dict] | None:
    """`encode_alu(ops, mnemonic, digit)` - Group 1 plus the accumulator forms.

    Four operand shapes reach the wire: r/m,reg (`00/01`), reg,r/m (`02/03`),
    the AL/eAX immediate (`04/05`) and the group-1 immediate with the digit in
    ModR/M.reg (`80/81/83`).  All of them belong to the same instruction class,
    so a row must produce a legal encoding for each.
    """
    digit = arg_int(args, 2)
    if digit is None:
        return None
    forms = []
    for size in model_sizes(name, b64):
        # `W`: the byte form is the listed opcode and the wider ones are `| 1`
        # (`00/01`, `02/03`, `04/05`).  Without it the model claimed the byte
        # opcode for every size -- legal under the same mnemonic, so `--verify`
        # could not see it, and `--missing` listed the wide forms as gaps.
        wide = 0 if size == 1 else 1
        forms.extend(_sized({size: 0x00 + digit * 8 + wide}, size, None))
        forms.extend(_sized({size: 0x02 + digit * 8 + wide}, size, None))
        forms.extend(_sized({size: 0x04 + digit * 8 + wide}, size, None))
        if size == 1:
            forms.extend(_sized({size: 0x80}, size, digit))
        else:
            forms.extend(_sized({size: 0x81}, size, digit))
            forms.extend(_sized({size: 0x83}, size, digit))
    return forms or None


def _model_test(args, name, b64) -> list[dict] | None:
    """`encode_test(ops, mnemonic)` - `84/85`, `A8/A9` and `F6/F7 /0`."""
    forms = []
    for size in model_sizes(name, b64):
        # `W`: the byte form is the listed opcode, the wider ones are `| 1`
        # (`84`/`85` and `A8`/`A9`).  Spelling `{size: 0x84}` looked right and
        # claimed the byte opcode for every size, which the snapshot accepted
        # because `test 84` exists under the same name -- the gate could not
        # tell, and `--missing` then reported `85`/`A9`/`F7 /0` as gaps.
        wide = 0 if size == 1 else 1
        forms.extend(_sized({size: 0x84 + wide}, size, None))
        forms.extend(_sized({size: 0xA8 + wide}, size, None))
        forms.extend(_sized({size: 0xF6 if size == 1 else 0xF7}, size, 0))
    return forms or None


def _model_sse_op_38(args, name, b64) -> list[dict] | None:
    """`encode_sse_op_38(ops, opcode)` - `66 0F 38 <op>`."""
    opcode = arg_int(args, 1)
    if opcode is None:
        return None
    return [form(opcode, map_="0f38", pp="66")]


def _model_avx_shift(args, name, b64) -> list[dict] | None:
    """`encode_avx_shift(ops, reg_op, imm_ext, imm_op, has_66)`.

    Two opcode shapes: the VEX.NDS register/memory form (`reg_op`, with the
    destination in ModR/M.reg) and the immediate form (`imm_op`, with the
    operation in ModR/M.reg and the source in r/m).
    """
    reg_op, imm_ext, imm_op = arg_int(args, 1), arg_int(args, 2), arg_int(args, 3)
    has_66 = arg_bool(args, 4)
    if None in (reg_op, imm_ext, imm_op) or has_66 is None:
        return None
    pp = "66" if has_66 else "none"
    return [form(reg_op, map_="0f", pp=pp, family="vex"),
            form(imm_op, map_="0f", pp=pp, reg=imm_ext, family="vex")]


def _model_avx_mov_np(args, name, b64) -> list[dict] | None:
    """`encode_avx_mov_np(ops, load_op, store_op, is_66)`."""
    load, store, is_66 = arg_int(args, 1), arg_int(args, 2), arg_bool(args, 3)
    if None in (load, store) or is_66 is None:
        return None
    pp = "66" if is_66 else "none"
    return [form(load, map_="0f", pp=pp, family="vex"),
            form(store, map_="0f", pp=pp, family="vex")]


def arg_bytes(args: list[str], idx: int) -> list[int] | None:
    """`args[idx]` as the byte slice literal the helper pushes verbatim."""
    if idx >= len(args):
        return None
    m = re.search(r"\[([^\]]*)\]", args[idx])
    if not m:
        return None
    out = []
    for part in m.group(1).split(","):
        part = part.strip()
        if not part:
            continue
        v = RUST_INT.match(part)
        if not v:
            return None
        out.append(int(v.group(1).replace("_", ""), 0))
    return out or None


def _model_avx_broadcast(args, name, b64) -> list[dict] | None:
    """`encode_avx_broadcast(ops, &[0x18])` - VEX.128/256.66.0F38 <op>.

    The helper is `emit_vex(r, x, b, 2, 0, 0, l, 1)`: map 0F38, W=0, and
    mandatory prefix 66 (pp=1), with the opcode byte taken from the array.
    Reading the array as a legacy opcode instead would claim these are
    `0x18`-map-less one-byte instructions, which is what a stray `LCCC-EXTRA`
    on `vbroadcastss` used to mean.
    """
    op = arg_bytes(args, 1)
    if not op:
        return None
    return [form(op[-1], map_="0f38", pp="66", w=0, family="vex")]


def _model_vex_mem_only(args, name, b64) -> list[dict] | None:
    """`encode_vex_mem_only(ops, &[0x0F, 0xAE], ext)` - VEX.128.0F /ext.

    `emit_vex(false, x, b, 1, 0, 0, 0, 0)` puts the instruction in map 0F with no
    mandatory prefix and W=0; only the final byte of the array is emitted, and
    the operation lives in ModR/M.reg (`vldmxcsr` /2, `vstmxcsr` /3).
    """
    op, ext = arg_bytes(args, 1), arg_int(args, 2)
    if not op or ext is None:
        return None
    return [form(op[-1], map_="0f", pp="none", w=0, reg=ext, family="vex")]


MOV_RD_OPCODE = {1: 0xB0, 2: 0xB8, 4: 0xB8, 8: 0xB8}
MOV_RM_OPCODE = {1: 0x88, 2: 0x89, 4: 0x89, 8: 0x89}
MOV_MR_OPCODE = {1: 0x8A, 2: 0x8B, 4: 0x8B, 8: 0x8B}
MOV_IMM_OPCODE = {1: 0xC6, 2: 0xC7, 4: 0xC7, 8: 0xC7}
MOVZX_OPCODE = {1: 0xB6, 2: 0xB7}
MOVSX_OPCODE_EXT = {1: 0xBE, 2: 0xBF, 4: 0x63}


def _model_x87_arith_reg(args, name, b64) -> list[dict] | None:
    """`encode_x87_arith_reg(ops, op_st0, op_sti, base_modrm)`.

    The helper emits one of three forms and all three carry the same operation:
    the no-operand spelling pops (`DE /d`), `%st(i)` alone is the `D8 /d` form,
    and `%st, %st(i)` is `DC /d`.  The digit is in the base byte, so `fsub`
    (base `E0`) must come out as `/4` and never as `fsubr`'s `/5`.
    """
    op_st0, op_sti, base = arg_int(args, 1), arg_int(args, 2), arg_int(args, 3)
    if None in (op_st0, op_sti, base):
        return None
    # The no-operand spelling pops, i.e. it encodes `faddp` (DE C1), which XED
    # catalogues as FADDP and not as FADD.  It is therefore *not* part of this
    # row's class; the `faddp` rows cover it, including the AT&T swap that maps
    # `fsubp` onto FSUBRP.
    reg = (base >> 3) & 7
    return [form(op_st0, reg=reg), form(op_sti, reg=reg)]


def _model_mov(args, name, b64) -> list[dict] | None:
    """`encode_mov(ops, size)` and its suffix-less wrappers.

    Five operand shapes reach the wire: `B0+rb`/`B8+rd` (immediate to register),
    `88/89` (register to r/m), `8A/8B` (r/m to register), and `C6/C7 /0`
    (immediate to r/m, which also covers the sign-extended 64-bit immediate).
    """
    size = arg_int(args, 1)
    sizes = [size] if size in MOV_RD_OPCODE else model_sizes(name, b64)
    encs = []
    for s in sizes:
        encs.append(form(MOV_RD_OPCODE[s]))
        encs.append(form(MOV_RM_OPCODE[s]))
        encs.append(form(MOV_MR_OPCODE[s]))
        encs.append(form(MOV_IMM_OPCODE[s], reg=0))
    return encs or None


def _model_movzx(args, name, b64) -> list[dict] | None:
    """`encode_movzx(ops, src_size, dst_size)` and `movzx` with inferred widths.

    The opcode carries the *source* width only; the destination width is a
    prefix (or REX.W) and does not change the instruction class.
    """
    src = arg_int(args, 1)
    srcs = [src] if src in MOVZX_OPCODE else [1, 2]
    return [form(MOVZX_OPCODE[s], map_="0f") for s in srcs]


def _model_movsx_infer_both(args, name, b64) -> list[dict] | None:
    """Suffix-less `movsx`/`movzx`: every source width the mnemonic can take."""
    # The 4->8 form is `movslq`, whose class is MOVSXD rather than MOVSX, and it
    # has its own row; including 0x63 here would compare it with MOVSX and
    # report a legal `movslq` as an invented encoding.
    return ([form(MOVSX_OPCODE_EXT[s], map_="0f") for s in (1, 2)] if b64
            else [form(MOVSX_OPCODE_EXT[s], map_="0f") for s in (1, 2)])


def _model_movzx_infer_both(args, name, b64) -> list[dict] | None:
    """Suffix-less `movzx`: both source widths, 0F B6 / 0F B7."""
    return [form(MOVZX_OPCODE[s], map_="0f") for s in (1, 2)]


def _model_imul(args, name, b64) -> list[dict] | None:
    """`encode_imul(ops, size)`: one-, two- and three-operand forms.

    `F6/F7 /5` is the one-operand form; two/three operands are `0F AF` plus the
    two immediate forms (`6B /r ib` for a sign-extended byte, `69 /r iz`).
    """
    size = arg_int(args, 1)
    if size is None:
        return None
    wide = 0xF6 if size == 1 else 0xF7
    return [form(wide, reg=5), form(0xAF, map_="0f"), form(0x6B), form(0x69)]


def _model_bt(args, name, b64) -> list[dict] | None:
    """`encode_bt(ops, mnemonic)`: `0F A3/AB/B3/BB` and `0F BA /4../7`."""
    stem = (name or "bt").rstrip("bwlq") or "bt"
    table = {"bt": (0xA3, 4), "bts": (0xAB, 5), "btr": (0xB3, 6), "btc": (0xBB, 7)}
    if stem not in table:
        return None
    opcode, ext = table[stem]
    return [form(opcode, map_="0f"), form(0xBA, map_="0f", reg=ext)]


def _model_sized_pair(rules: dict[str, tuple[str, int]]) -> object:
    """Build a model that picks an opcode from the mnemonic's size suffix."""
    def model(args, name, b64):
        key = (name or "")[-1:]
        hit = rules.get(key) or rules.get("d")
        if hit is None:
            return None
        map_, opcode = hit
        return [form(opcode, map_=map_)]
    return model


_model_cmpxchg = _model_sized_pair({"b": ("0f", 0xB0), "d": ("0f", 0xB1)})
_model_xadd = _model_sized_pair({"b": ("0f", 0xC0), "d": ("0f", 0xC1)})


def _model_out(args, name, b64) -> list[dict] | None:
    """`out`: `EE/E6` for a byte port operand, `EF/E7` for a word or dword.

    The operand size picks the same `| 1` the ALU uses: `out %al,%dx` is `EE`
    and `out %eax,%dx` is `EF`.  Claiming only the byte pair left the wide forms
    in the gap list even though the encoder emits them.
    """
    return [form(0xEE), form(0xE6), form(0xEF), form(0xE7)]


def _model_in(args, name, b64) -> list[dict] | None:
    """`in`: `EC/E4` byte, `ED/E5` word and dword (see `_model_out`)."""
    return [form(0xEC), form(0xE4), form(0xED), form(0xE5)]


def _model_bsr_bsf(args, name, b64) -> list[dict] | None:
    """`bsf`/`bsr` in every spelling: `0F BC` and `0F BD`."""
    stem = (name or "").rstrip("-")
    opcode = {True: 0xBC, False: 0xBD}.get(stem.startswith("bsf"))
    if opcode is None:
        return None
    return [form(opcode, map_="0f")]


def _model_bit_count(args, name, b64) -> list[dict] | None:
    """`lzcnt`/`tzcnt`/`popcnt`: F3-prefixed `0F BD` / `0F BC` / `0F B8`."""
    stem = (name or "").rstrip("bwlq")
    opcode = {"lzcnt": 0xBD, "tzcnt": 0xBC, "popcnt": 0xB8}.get(stem)
    if opcode is None:
        return None
    return [form(opcode, map_="0f", pp="f3")]


SETCC_OPCODE = {
    "o": 0x90, "no": 0x91, "b": 0x92, "c": 0x92, "nae": 0x92,
    "ae": 0x93, "nb": 0x93, "nc": 0x93, "e": 0x94, "z": 0x94,
    "ne": 0x95, "nz": 0x95, "be": 0x96, "na": 0x96, "a": 0x97,
    "nbe": 0x97, "s": 0x98, "ns": 0x99, "p": 0x9A, "pe": 0x9A,
    "np": 0x9B, "po": 0x9B, "l": 0x9C, "nge": 0x9C, "ge": 0x9D,
    "nl": 0x9D, "le": 0x9E, "ng": 0x9E, "g": 0x9F, "nle": 0x9F,
}


def _model_setcc(args, name, b64) -> list[dict] | None:
    """`setcc`: `0F 90+cc`; the row name carries the condition."""
    spelled = args[1].strip().strip('"') if len(args) > 1 and args[1].strip().startswith('"') else None
    cond = (spelled or name or "")[3:]
    if cond in SETCC_OPCODE:
        return [form(SETCC_OPCODE[cond], map_="0f", reg=0)]
    return None


def _model_string_op(args, name, b64) -> list[dict] | None:
    """`encode_string_op(ops, opcode, size, ..)`: the byte form is `opcode`,
    the wider forms are the next opcode."""
    opcode, size = arg_int(args, 1), arg_int(args, 2)
    if opcode is None:
        return None
    if size is None:
        # `string_op_size(mnemonic)?` is resolved from the mnemonic at run time;
        # the AT&T suffix is the same information and XED splits the two widths
        # into separate classes (`MOVSB` is 0xA4, `MOVSW`/`MOVSQ` are 0xA5).
        size = name_size(name)
        if size is None:
            return [form(opcode), form(opcode + 1)]
    return [form(opcode)] if size == 1 else [form(opcode + 1)]


def _model_wrss(args, name, b64) -> list[dict] | None:
    """`encode_wrss(ops, size, is_user)`.

    WRSS is `0F 38 F6`; WRUSS is `66 0F 38 F5`.  The 66 here is a mandatory
    prefix (it distinguishes the user-mode form), not the operand-size prefix,
    so it belongs in `pp`.
    """
    is_user = arg_bool(args, 2)
    if is_user is None:
        return None
    return [form(0xF5, map_="0f38", pp="66") if is_user
            else form(0xF6, map_="0f38", pp="none")]


VCVT_PARAMS_RE = re.compile(
    r'"(?P<name>[a-z0-9]+)"\s*=>\s*Some\(p\(\s*'
    r"(?:(?P<vex>Some\(\((?P<vex_op>0x[0-9A-Fa-f]+),\s*(?P<vex_pp>\d+)\)\))|None)"
    r"\s*,\s*(?P<map>\d+),\s*(?P<pp>\d+),\s*(?P<w>\d+),\s*"
    r"(?P<op>0x[0-9A-Fa-f]+),\s*VcvtKind::(?P<kind>\w+)")

EVEX_MAP = {1: "0f", 2: "0f38", 3: "0f3a", 5: "map5", 6: "map6"}


def _vcvt_table() -> dict[str, dict]:
    """`{mnemonic: params}` from `vcvt_base_params` in the 64-bit encoder.

    The table *is* the encoder's data: copying it into the checker would let the
    two drift, which is exactly the failure mode this gate exists to catch.  It
    is read from the source on first use and cached.
    """
    path = REPO / "src/backend/x86/assembler/encoder/avx.rs"
    text = path.read_text(encoding="utf-8", errors="replace")
    start = text.find("fn vcvt_base_params")
    end = text.find("\n}", start)
    if start < 0 or end < 0:
        return {}
    table = {}
    for m in VCVT_PARAMS_RE.finditer(text[start:end]):
        table[m.group("name")] = {
            "vex": (int(m.group("vex_op"), 16), int(m.group("vex_pp")))
            if m.group("vex") else None,
            "map": int(m.group("map")),
            "pp": int(m.group("pp")),
            "w": int(m.group("w")),
            "opcode": int(m.group("op"), 16),
            "kind": m.group("kind"),
        }
    return table


_VCVT: dict[str, dict] | None = None


def _model_vcvt(args, name, b64) -> list[dict] | None:
    """`encode_vcvt(ops, mnemonic)`: the VEX form, plus the EVEX one when
    the row can reach `encode_evex_vcvt` (EVEX-only mnemonics and the
    narrowing memory form)."""
    global _VCVT
    if _VCVT is None:
        _VCVT = _vcvt_table()
    spelled = args[1].strip().strip('"') if len(args) > 1 else (name or "")
    spelled = spelled.lower()
    params = _VCVT.get(spelled)
    if params is None and spelled.endswith(("x", "y", "z")):
        params = _VCVT.get(spelled[:-1])
    if params is None:
        return None
    encs = []
    if params["vex"] is not None:
        op, pp = params["vex"]
        encs.append(form(op, map_="0f", pp=PP_BY_INDEX[pp], w=0, family="vex"))
        if params["kind"] != "Narrow":
            return encs
    map_ = EVEX_MAP.get(params["map"])
    if map_ is None:
        return encs or None
    encs.append(form(params["opcode"], map_=map_, pp=PP_BY_INDEX[params["pp"]],
                     w=params["w"], family="evex"))
    return encs


def _model_user_msr(args, name, b64) -> list[dict] | None:
    """`encode_user_msr(ops, mnemonic)`.

    `wrmsrns` is the plain `0F 01 /0`; `urdmsr`/`uwrmsr` are VEX map 7 with the
    F2/F3 mandatory prefix.  The class differs, so the prefix has to follow the
    mnemonic rather than being reported as "either".
    """
    spelled = args[1].strip().strip('"') if len(args) > 1 else (name or "")
    spelled = spelled.lower()
    if spelled == "wrmsrns":
        return [form(0x01, map_="0f", pp="none", reg=0)]
    if spelled == "urdmsr":
        return [form(0xF8, map_="map7", pp="f2", w=0, family="vex")]
    if spelled == "uwrmsr":
        return [form(0xF8, map_="map7", pp="f3", w=0, family="vex")]
    # The row passes its `mnemonic` binding, so the *name* being verified is the
    # only place the concrete spelling exists.
    if spelled == "mnemonic" and name in ("urdmsr", "uwrmsr", "wrmsrns"):
        return _model_user_msr(["ops", f'"{name}"'], name, b64)
    return None


def _model_pushp(args, name, b64) -> list[dict] | None:
    """`pushp`/`popp`: REX2-prefixed `50+rd` / `58+rd`."""
    pop = arg_bool(args, 1)
    if pop is None:
        return None
    return [form(0x58 if pop else 0x50)]


def _model_suffixless_alu(args, name, b64) -> list[dict] | None:
    """Suffix-less `add`/`sub`/...: `encode_alu` with the size taken from the
    registers, so every size the mnemonic can be assembled in is legal."""
    digit = arg_int(args, 1)
    return None if digit is None else _model_alu(["ops", "?", str(digit)], None, b64)


def _model_suffixless_shift(args, name, b64) -> list[dict] | None:
    """Suffix-less `shl`/`shr`/`sar`: `encode_shift` with the inferred size."""
    digit = arg_int(args, 1)
    return None if digit is None else _model_shift(["ops", "?", str(digit)], None, b64)


def _model_suffixless_test(args, name, b64) -> list[dict] | None:
    return _model_test(["ops"], None, b64)


def _model_suffixless_unary(args, name, b64) -> list[dict] | None:
    """Suffix-less `inc`/`dec`/`neg`/...: group 3 or group 5, all sizes."""
    ext = arg_int(args, 1)
    if ext is None:
        return None
    encs = []
    for size in model_sizes(None, b64):
        encs.extend(_sized({size: 0xFE if ext <= 1 else (0xF6 if size == 1 else 0xF7)},
                           size, ext))
    return encs or None


def _model_suffixless_mov(args, name, b64) -> list[dict] | None:
    return _model_mov(["ops"], None, b64)


HELPER_MODELS = {
    "encode_unary_rm": _model_unary_rm,
    "encode_inc_dec": _model_inc_dec,
    "encode_div_idiv": _model_div_idiv,
    "encode_shift": _model_shift,
    "encode_double_shift": _model_double_shift,
    "encode_movsx": _model_movsx,
    "encode_alu": _model_alu,
    "encode_test": _model_test,
    "encode_sse_op_38": _model_sse_op_38,
    "encode_avx_shift": _model_avx_shift,
    "encode_avx_mov_np": _model_avx_mov_np,
    "encode_avx_broadcast": _model_avx_broadcast,
    "encode_vex_mem_only": _model_vex_mem_only,
    "encode_x87_arith_reg": _model_x87_arith_reg,
    "encode_mov": _model_mov,
    "encode_mov_infer_size": _model_mov,
    "encode_suffixless_mov": _model_suffixless_mov,
    "encode_movzx": _model_movzx,
    "encode_movzx_infer_dst": _model_movzx,
    "encode_movzx_infer_both": _model_movzx_infer_both,
    "encode_movsx_infer_both": _model_movsx_infer_both,
    "encode_imul": _model_imul,
    "encode_bt": _model_bt,
    "encode_cmpxchg": _model_cmpxchg,
    "encode_xadd": _model_xadd,
    "encode_out": _model_out,
    "encode_in": _model_in,
    "encode_bsr_bsf": _model_bsr_bsf,
    "encode_bsf_bsr": _model_bsr_bsf,
    "encode_bsr_bsf_16": _model_bsr_bsf,
    "encode_bit_count": _model_bit_count,
    "encode_setcc": _model_setcc,
    "encode_string_op": _model_string_op,
    "encode_pushp": _model_pushp,
    "encode_wrss": _model_wrss,
    "encode_vcvt": _model_vcvt,
    "encode_user_msr": _model_user_msr,
    "encode_suffixless_alu": _model_suffixless_alu,
    "encode_suffixless_shift": _model_suffixless_shift,
    "encode_suffixless_test": _model_suffixless_test,
    "encode_suffixless_unary": _model_suffixless_unary,
}


# Helpers that take the operand size as an argument, and the argument index.
# The AT&T suffix is part of the mnemonic, so a row that binds `negw` to a
# 32-bit size emits a legal-looking instruction of the wrong width; that is a
# defect the reference tables cannot see (they are size-agnostic here) but the
# name can, so it is checked directly.
SIZE_ARG = {
    "encode_unary_rm": 2,
    "encode_inc_dec": 2,
    "encode_div_idiv": 2,
    "encode_double_shift": 2,
    "encode_movsx": 2,
}


def size_mismatch(row: dict, name: str) -> bool:
    """True when the mnemonic's suffix contradicts the helper's size argument."""
    helper = re.match(r"self\.(encode_[a-z0-9_]+)\(", row["body"])
    if not helper or helper.group(1) not in SIZE_ARG:
        return False
    spelled = name_size(name)
    if spelled is None:
        return False
    args = call_args(row["body"], helper.group(1))
    if args is None:
        return False
    return arg_int(args, SIZE_ARG[helper.group(1)]) != spelled


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
            # The mode is a property of the row, not of the run: `--backend all`
            # covers both encoders, and `40..4F` is REX in one and `inc`/`dec` in
            # the other.  Modelling every row with the 64-bit flag would check
            # half the table against the wrong instruction set.
            wide = row_is_64bit(row) if b64 else False
            # A row whose left-hand side names a relocation kind (`gotpcrel`,
            # `gottpoff`, `tlsdesc`) or an operand (`es`, `fs`) is not a
            # mnemonic row; `NOT_A_MNEMONIC` lists those and they cannot be
            # modelled as instructions, so they are skipped rather than being
            # reported as unmodelled.
            if all(n in NOT_A_MNEMONIC for n in row["names"]):
                continue
            if model_row(row, wide, row["names"][0]) is None:
                unmodeLLed.append(row)
                continue
            for name in row["names"]:
                if name in NOT_A_MNEMONIC:
                    continue
                encs = model_row(row, wide, name)
                if encs is None:
                    findings.append((row, name, "UNMODELLED", name))
                    continue
                if size_mismatch(row, name):
                    findings.append((row, name, "SIZE-MISMATCH", name))
                    continue
                classes = _upper_class(name)
                present = [c for c in classes if c in catalogue]
                if not present:
                    findings.append((row, name, "NO-CLASS", classes[0]))
                    continue
                # A mnemonic can span two SDM classes: `pextrw` is both the SSE2
                # register form (PEXTRW) and the SSE4.1 `$imm8` form
                # (PEXTRW_SSE4), and the AT&T spelling names both.  The encoding
                # has to belong to *some* class the mnemonic can name, so the
                # candidates are unioned instead of picking one arbitrarily.
                for enc in encs:
                    if any(_matches(enc, form)
                           for cls in present for form in catalogue[cls]):
                        verified += 1
                    else:
                        findings.append((row, name, "LCCC-EXTRA", present[0]))
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
    catalogue = with_sdm_only_classes(json.loads(cat_path.read_text()))

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
