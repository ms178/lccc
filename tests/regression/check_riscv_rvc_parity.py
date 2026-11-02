#!/usr/bin/env python3
"""Hold LCCC's RISC-V assembler to GNU as on RVC compression, byte for byte.

GNU as compresses every relocation-free instruction that has a 2-byte form,
the words of pseudo-instruction expansions included, and relaxes symbolic
`beqz`/`bnez`/`j` (`jal ra` on RV32) to `c.beqz`/`c.bnez`/`c.j`/`c.jal` while
the target is in the same section and in reach.  The corpus covers every
compressible form at its immediate boundaries, with near-misses on both
sides, plus a seeded random mix, for RV64 and RV32, and the branch reach
boundaries.  `.text` must be identical except for displacement fields under
a branch relocation (GNU as fills them for local targets, LCCC leaves them
to the relocation; RELA linkers read only the addend), and the relocation
lists must be identical.  Without the cross binutils the script prints a
SKIP line.
"""

from __future__ import annotations

import argparse
import random
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
TRIPLE = "riscv64-linux-gnu"
XLENS = {
    64: ("rv64imafdc_zicsr_zifencei", "lp64d"),
    32: ("rv32imafdc_zicsr_zifencei", "ilp32d"),
}
# Displacement bits under each branch relocation (R_RISCV_*).
DISPLACEMENT_MASKS = {16: (4, 0xFE000F80), 17: (4, 0xFFFFF000), 44: (2, 0x1C7C), 45: (2, 0x1FFC)}

X = [f"x{i}" for i in range(32)]
C = [f"x{i}" for i in range(8, 16)]
F = [f"f{i}" for i in range(32)]
FC = [f"f{i}" for i in range(8, 16)]


class GateFailure(RuntimeError):
    pass


def run(argv: list[str], cwd: Path) -> str:
    done = subprocess.run(argv, cwd=cwd, text=True, capture_output=True, check=False, timeout=120)
    if done.returncode:
        raise GateFailure(f"{' '.join(argv)} failed ({done.returncode}):\n{done.stderr}")
    return done.stdout


def boundary_corpus(xlen: int) -> list[str]:
    lines: list[str] = []
    for rd in ("x1", "x8", "x15", "x31"):
        for imm in (-33, -32, -1, 0, 1, 31, 32):
            lines += [f"addi {rd}, {rd}, {imm}", f"addi {rd}, x0, {imm}", f"andi {rd}, {rd}, {imm}"]
            if xlen == 64:
                lines.append(f"addiw {rd}, {rd}, {imm}")
        for imm in (0, 1, 31, 32, 0xFFFE0, 0xFFFFF):
            lines.append(f"lui {rd}, {imm}")
        for rs in ("x0", "x1", "x9", "x31"):
            lines += [f"add {rd}, {rd}, {rs}", f"add {rd}, x0, {rs}", f"addi {rd}, {rs}, 0"]
            lines += [f"add {rd}, {rs}, {rd}", f"sub {rd}, {rd}, {rs}"]
            for op in ("xor", "or", "and") + (("addw", "subw") if xlen == 64 else ()):
                lines += [f"{op} {rd}, {rd}, {rs}", f"{op} {rd}, {rs}, {rd}"]
        for shamt in (0, 1, 31) + ((32, 63) if xlen == 64 else ()):
            lines += [f"slli {rd}, {rd}, {shamt}", f"srli {rd}, {rd}, {shamt}", f"srai {rd}, {rd}, {shamt}"]
    for rd in ("x0", "x2", "x5"):
        lines += [f"lui {rd}, 1", f"addi {rd}, {rd}, 5"]
    lines += ["addi x0, x0, 0", "addi x0, x0, 1", "nop", "ebreak", "ret", "jr x5", "jalr x5", "jr x0"]
    for imm in (-528, -512, -16, 0, 8, 16, 496, 512):
        lines.append(f"addi sp, sp, {imm}")
    for imm in (0, 2, 4, 8, 1020, 1024):
        lines += [f"addi x8, sp, {imm}", f"addi x15, sp, {imm}", f"addi x5, sp, {imm}"]
    word_ops = [("lw", "sw", X, 4)]
    if xlen == 64:
        word_ops.append(("ld", "sd", X, 8))
    else:
        word_ops.append(("flw", "fsw", F, 4))
    word_ops.append(("fld", "fsd", F, 8))
    for load, store, regs, size in word_ops:
        for reg in (regs[0], regs[1], regs[8], regs[15], regs[31]):
            for off in (-size, 0, size, 2 * size + 1, 31 * size, 32 * size, 63 * size, 64 * size):
                lines += [f"{load} {reg}, {off}(sp)", f"{store} {reg}, {off}(sp)"]
                lines += [f"{load} {reg}, {off}(x9)", f"{store} {reg}, {off}(x9)"]
                lines += [f"{load} {reg}, {off}(x5)", f"{store} {reg}, {off}(x5)"]
    # GNU as compresses by spelling (the opcode table's RVC aliases), not by
    # encoding: `mv x5, x0` and `jalr x1, 0(x5)` stay 4 bytes.
    lines += ["mv x5, x0", "mv x5, x6", "move x5, x6", "add x5, x0, x6", "addi x5, x5, 0"]
    lines += ["jalr x1, 0(x5)", "jalr x0, 0(x5)", "jalr x1, 4(x5)", "unimp", "c.unimp"]
    lines += ["li x0, 5", "li x2, 4096", "li x5, 4096"] + (["sext.w x8, x8"] if xlen == 64 else [])
    # `li`: GNU as's `load_const` (RV32: `normalize_constant_expr` first).
    values = [0, 1, -1, 31, 32, -32, -33, 2047, 2048, -2048, -2049, 4096, 0x1F000, 0x20000]
    values += [0x7FFFF800, 0x7FFFFFFF, -0x80000000, 0x12345678, 0x80000000, 0xFFFFFFFF]
    values += [0xFFFFFFE0, 0xFFFFF000, 0xFFFE0000, 0x80000001, 0xFFFFF801]
    if xlen == 64:
        values += [0x100000000, 0x123456789, 1 << 40, (1 << 40) + 1, -0x123456789ABCDEF]
        values += [0x7FFFFFFFFFFFFFFF, -(1 << 63), 0xFFFFFFFF00000000 - (1 << 64), 0xFFFFF800]
    for value in values:
        lines += [f"li x10, {value}", f"li x8, {value}", f"li x2, {value}"]
    return lines


def random_corpus(xlen: int, rng: random.Random, count: int) -> list[str]:
    def imm(lo: int, hi: int, step: int = 1) -> int:
        return rng.randrange(lo, hi + 1, step) if step == 1 else rng.randrange(lo // step, hi // step + 1) * step

    forms = [
        lambda: f"addi {rng.choice(X)}, {rng.choice(X)}, {imm(-40, 40)}",
        lambda: f"li {rng.choice(X)}, {rng.choice([imm(-40, 40), imm(-5000, 5000), imm(-(1 << 31), (1 << 31) - 1)])}",
        lambda: f"mv {rng.choice(X)}, {rng.choice(X)}",
        lambda: f"{rng.choice(['add', 'sub', 'and', 'or', 'xor'])} {rng.choice(X)}, {rng.choice(X)}, {rng.choice(X)}",
        lambda: f"andi {rng.choice(X)}, {rng.choice(X)}, {imm(-40, 40)}",
        lambda: f"{rng.choice(['slli', 'srli', 'srai'])} {rng.choice(X)}, {rng.choice(X)}, {imm(0, xlen - 1)}",
        lambda: f"addi sp, sp, {imm(-600, 600, 8)}",
        lambda: f"addi {rng.choice(X)}, sp, {imm(-8, 1100, 4)}",
        lambda: f"lw {rng.choice(X)}, {imm(-8, 300, 4)}({rng.choice(X)})",
        lambda: f"sw {rng.choice(X)}, {imm(-8, 300, 4)}({rng.choice(X)})",
        lambda: f"fld {rng.choice(F)}, {imm(-8, 600, 8)}({rng.choice(X)})",
        lambda: f"fsd {rng.choice(F)}, {imm(-8, 600, 8)}({rng.choice(X)})",
        lambda: f"jalr {rng.choice(X)}, {rng.choice([0, 0, 4])}({rng.choice(X)})",
        lambda: f"mul {rng.choice(X)}, {rng.choice(X)}, {rng.choice(X)}",
    ]
    if xlen == 64:
        forms += [
            lambda: f"addiw {rng.choice(X)}, {rng.choice(X)}, {imm(-40, 40)}",
            lambda: f"{rng.choice(['addw', 'subw'])} {rng.choice(X)}, {rng.choice(X)}, {rng.choice(X)}",
            lambda: f"ld {rng.choice(X)}, {imm(-8, 600, 8)}({rng.choice(X)})",
            lambda: f"sd {rng.choice(X)}, {imm(-8, 600, 8)}({rng.choice(X)})",
            lambda: f"li {rng.choice(X)}, {imm(-(1 << 62), (1 << 62))}",
        ]
    else:
        forms += [
            lambda: f"flw {rng.choice(F)}, {imm(-8, 300, 4)}({rng.choice(X)})",
            lambda: f"fsw {rng.choice(F)}, {imm(-8, 300, 4)}({rng.choice(X)})",
        ]
    return [rng.choice(forms)() for _ in range(count)]


def branch_corpus(xlen: int) -> list[str]:
    """Branches at and just past the c.beqz (256) and c.j (2048) reach."""
    lines = ["branch_entry:"]
    for pads in (126, 127, 128):  # 2-byte c.nop pads: the target sits 2 * pads + 2 away
        lines += [f"beqz x8, .Lfwd{pads}", f"bnez x15, .Lfwd{pads}"]
        lines += ["nop"] * pads + [f".Lfwd{pads}:"]
    for pads in (126, 127, 128):
        lines += [f".Lback{pads}:"] + ["nop"] * pads + [f"beqz x9, .Lback{pads}"]
    for pads in (1021, 1022, 1023, 1024):
        lines += [f"j .Ljf{pads}"] + ["nop"] * pads + [f".Ljf{pads}:"]
    lines += [f".Ljb:"] + ["nop"] * 1023 + ["j .Ljb", "j branch_entry"]
    lines += ["beqz x5, .Lnear", "beq x8, x9, .Lnear", "bnez x8, undefined_target", "j undefined_target"]
    lines += ["beqz x8, .Lelsewhere", "j .Lelsewhere", ".Lnear:"]
    # Only these spellings have RVC aliases: `jal target` (RV32 `c.jal`), not
    # `jal ra, target`; `beq rs, x0`, not `beq x0, rs`.
    lines += ["jal branch_entry", "jal ra, branch_entry", "jal ra, undefined_target", "jal x0, branch_entry"]
    lines += ["bne x8, x0, branch_entry", "beq x0, x8, branch_entry"]
    # A weak target may be preempted: always the long form.
    lines += [".weak wsym", "beqz x8, wsym", "beq x8, x9, wsym", "j wsym", "wsym:"]
    # Conditional branches out of +-4 KiB: the inverted branch over a `jal`
    # (6 bytes from an RVC spelling, 8 otherwise); jumps stay 4.
    lines += [".Lfarback:", "beq x8, x9, .Lfar", "beqz x8, .Lfar", "blt x5, x6, .Lfar", "j .Lfar"]
    lines += ["nop"] * 2100 + [".Lfar:", "beqz x8, .Lfarback", "bne x8, x9, .Lfarback", "bgeu x5, x6, .Lfarback"]
    for pads in (2045, 2046, 2047):  # the target sits 2 * pads + 4 away
        lines += [f"beq x8, x9, .Lb4k{pads}", f"bnez x9, .Lb4k{pads}"] + ["nop"] * pads + [f".Lb4k{pads}:"]
    lines += [".section .text.other,\"ax\",@progbits", ".Lelsewhere:", "ret"]
    return lines


def text_and_relocs(objcopy: str, readelf: str, obj: Path, work: Path) -> tuple[bytes, list[tuple[int, int, str]]]:
    raw = work / f"{obj.stem}.text.bin"
    run([objcopy, "-O", "binary", "-j", ".text", str(obj), str(raw)], work)
    data = bytearray(raw.read_bytes())
    relocs: list[tuple[int, int, str]] = []
    section = None
    for line in run([readelf, "-rW", str(obj)], work).splitlines():
        if line.startswith("Relocation section"):
            section = line.split("'")[1]
            continue
        fields = line.split()
        if section != ".rela.text" or len(fields) < 3 or not fields[0][:1].isdigit():
            continue
        try:
            offset, info = int(fields[0], 16), int(fields[1], 16)
        except ValueError:
            continue
        kind = info & 0xFFFFFFFF if len(fields[1]) > 8 else info & 0xFF
        relocs.append((offset, kind, " ".join(fields[2:])))
        if kind in DISPLACEMENT_MASKS:
            size, mask = DISPLACEMENT_MASKS[kind]
            word = int.from_bytes(data[offset : offset + size], "little") & ~mask
            data[offset : offset + size] = word.to_bytes(size, "little")
    return bytes(data), relocs


def first_difference(ours: bytes, theirs: bytes) -> str:
    index = next((i for i, (a, b) in enumerate(zip(ours, theirs)) if a != b), min(len(ours), len(theirs)))
    return f"first difference at .text+{index:#x} (lccc {len(ours)} bytes, GNU as {len(theirs)})"


def check_xlen(xlen: int, lccc: str, tools: dict[str, str], work: Path) -> int:
    march, mabi = XLENS[xlen]
    rng = random.Random(0x52564320 + xlen)
    cases = {
        "boundaries": boundary_corpus(xlen),
        "random": random_corpus(xlen, rng, 3000),
        "branches": branch_corpus(xlen),
    }
    total = 0
    for name, body in cases.items():
        source = work / f"rv{xlen}-{name}.s"
        source.write_text(".text\n" + "\n".join(f"  {line}" if not line.endswith(":") else line for line in body) + "\n")
        ours, theirs = work / f"rv{xlen}-{name}.lccc.o", work / f"rv{xlen}-{name}.gas.o"
        run([lccc, f"-march={march}", f"-mabi={mabi}", "-c", str(source), "-o", str(ours)], work)
        run([tools["as"], f"-march={march}", f"-mabi={mabi}", str(source), "-o", str(theirs)], work)
        lt, lr = text_and_relocs(tools["objcopy"], tools["readelf"], ours, work)
        gt, gr = text_and_relocs(tools["objcopy"], tools["readelf"], theirs, work)
        if lr != gr:
            only_ours = sorted(set(lr) - set(gr))[:6]
            only_theirs = sorted(set(gr) - set(lr))[:6]
            raise GateFailure(f"rv{xlen} {name}: relocations differ\n lccc only: {only_ours}\n GNU as only: {only_theirs}")
        if lt != gt:
            raise GateFailure(f"rv{xlen} {name}: .text differs from GNU as: {first_difference(lt, gt)}")
        total += len(body)
    return total


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lccc", default="target/fastbuild/lccc-riscv")
    args = parser.parse_args()
    lccc = str((ROOT / args.lccc).resolve()) if "/" in args.lccc else shutil.which(args.lccc)
    tools = {tool: shutil.which(f"{TRIPLE}-{tool}") for tool in ("as", "objcopy", "readelf")}
    missing = [f"{TRIPLE}-{tool}" for tool, path in tools.items() if path is None]
    if missing:
        print(f"SKIP: RVC parity with GNU as ({', '.join(missing)} not installed)")
        return 0
    try:
        if not lccc or not Path(lccc).exists():
            raise GateFailure(f"lccc not found: {args.lccc}")
        with tempfile.TemporaryDirectory(prefix="lccc-rvc-") as tmp:
            counts = {xlen: check_xlen(xlen, lccc, tools, Path(tmp)) for xlen in (64, 32)}
        print(
            "PASS: RVC compression and branch relaxation identical to GNU as "
            f"(rv64: {counts[64]} lines, rv32: {counts[32]} lines; boundaries, seeded random mix, branch reach)"
        )
        return 0
    except (GateFailure, OSError, subprocess.SubprocessError) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
