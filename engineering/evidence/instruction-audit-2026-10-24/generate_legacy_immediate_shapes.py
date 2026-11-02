#!/usr/bin/env python3
"""Generate SDM-derived legacy SIMD shape cases; asmdiff checks GNU too.

Use the latest tag-cache as/objdump when running these casefiles. There is
no old-toolchain compatibility lane or oracle-dependent trimming here.
"""
from pathlib import Path
import itertools

ROOT = Path(__file__).resolve().parents[3]
MNEMONICS = Path(__file__).with_name("legacy-immediate-mnemonics.txt").read_text().splitlines()
REGISTERS = ("mm1", "xmm1", "ymm1", "zmm1", "eax", "ax", "al", "k1", "st(0)", "xmm16")
IMMEDIATES = (-129, -128, -1, 0, 1, 127, 128, 255, 256, 9223372036854775807, -9223372036854775808)


def bank(register, bits32):
    if register.startswith("xmm") and int(register[3:]) < (8 if bits32 else 16):
        return "xmm"
    if register.startswith("mm") and int(register[2:]) < 8:
        return "mm"
    return None


def allowed(mnemonic, register, bits32):
    kind = bank(register, bits32)
    return kind == "mm" if mnemonic == "pshufw" else kind in ("mm", "xmm") if mnemonic == "palignr" else kind == "xmm"


def cases(bits32):
    for mnemonic in MNEMONICS:
        for src, dst in itertools.product(REGISTERS, repeat=2):
            valid = allowed(mnemonic, dst, bits32) and bank(src, bits32) == bank(dst, bits32)
            yield f"{mnemonic} $3, %{src}, %{dst}", valid
        for dst in REGISTERS:
            yield f"{mnemonic} $3, (%eax), %{dst}", allowed(mnemonic, dst, bits32)
        reg = "mm1" if mnemonic == "pshufw" else "xmm1"
        unsigned = mnemonic in ("palignr", "aeskeygenassist", "roundsd", "roundss", "roundpd", "roundps", "insertps", "gf2p8affineqb", "gf2p8affineinvqb", "cmpsd", "cmpss", "cmppd", "cmpps")
        for immediate in IMMEDIATES:
            for src in (f"%{reg}", "(%eax)"):
                yield f"{mnemonic} ${immediate}, {src}, %{reg}", (0 if unsigned else -128) <= immediate <= 255
        for src, dst in itertools.product(("xmm7", "xmm8", "xmm15", "mm7", "mm8", "mm9"), repeat=2):
            valid = allowed(mnemonic, dst, bits32) and bank(src, bits32) == bank(dst, bits32)
            yield f"{mnemonic} $3, %{src}, %{dst}", valid
        for dst in ("mm1", "xmm1"):
            yield f"{mnemonic} $3, anchor+17, %{dst}", allowed(mnemonic, dst, bits32)


def main():
    assert len(MNEMONICS) == len(set(MNEMONICS)) == 27
    for bits32 in (False, True):
        output = ROOT / "tests/asm-diff"
        if bits32:
            output /= "i686"
        rows = []
        accepted = 0
        for index, (line, valid) in enumerate(cases(bits32)):
            rows.extend((f";;; legacy-simd-shape-{index}" + ("" if valid else " reject"), ".text", line, ""))
            accepted += valid
        path = output / "legacy-immediate-shapes.casefile"
        path.write_text("\n".join(rows))
        print(f"{path.relative_to(ROOT)}: {index + 1} cases, {accepted} accepted")


if __name__ == "__main__":
    main()
