#!/usr/bin/env python3
"""The scalar and system intrinsics of include/x86gprintrin.h against GCC 16.2.

Each of these intrinsics is one instruction: `_mm_clwb` is CLWB, `_wrpkru`
is WRPKRU with ECX and EDX cleared, `_xsaveopt` is XSAVEOPT with the mask in
EDX:EAX. Most cannot run in a test (they are privileged, or the machine
lacks them), so this gate asks the two questions that can be answered
without executing one:

1. **Which instruction, in which form?** The fixture's callers
   (`fixtures/x86_gpr_intrinsics/callers.c`) are compiled by lccc and by
   GCC, and for each caller the instructions that are not register
   shuffling are compared in order, operands reduced to kind and width:
   `wrfsbase %eax` is not `wrfsbase %rax`, `setc` is not `setz`.

2. **With which operands?** Both programs are run under GDB with the same
   arguments and stopped *in front of* the instruction, which is then
   stepped over without being executed. At that point every explicit
   operand (a register's value, a memory operand's address relative to the
   argument buffer) and every register the instruction reads implicitly
   (`IMPLICIT`: RAX for CLZERO, ECX for RDPKRU, EDX:EAX for the XSAVE
   family, RBX/RCX/RDX for the enclave leaves, ...) must hold the value it
   holds in GCC's program.

Both hold for x86-64 and for i686. Without GDB or GCC 16.2 the gate fails,
unless `--allow-missing-tools` is given, in which case it says what it did
not check.
"""

from __future__ import annotations

import argparse
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
CALLERS = HERE / "fixtures" / "x86_gpr_intrinsics" / "callers.c"
CONTROL = HERE / "fixtures" / "x86_gpr_intrinsics" / "control.c"
# What the negative controls have to be reported as.
CONTROL_FINDINGS = [
    # (objdump's names for `setz` and `setc`)
    "t_control_form: lccc `tpause reg32; sete reg8`, GCC `tpause reg32; setb reg8`",
    "t_control_implicit: at `wrpkru` lccc has ['ax=0x1001', 'cx=0x1', 'dx=0x0'], "
    "GCC has ['ax=0x1001', 'cx=0x0', 'dx=0x0']",
    "t_control_operand: at `clwb` lccc has ['buf+0x80'], GCC has ['buf+0x40']",
]

# The options that enable the fixture's intrinsics under GCC; the last
# two do not exist for 32-bit code.
OPTIONS = ("-mclflushopt -mclwb -mcldemote -mclzero -mwbnoinvd -mmovdiri -mmovdir64b "
           "-menqcmd -mrdpid -mptwrite -mpku -mserialize -mhreset -mwaitpkg -mmwaitx -mrtm "
           "-mtsxldtrk -mxsaveopt -mxsavec -mxsaves -mfxsr -mshstk -msgx -mpconfig -msse4.2").split()
OPTIONS_64 = ["-mfsgsbase", "-muintr"]

# Instructions that move values into place and say nothing about the
# intrinsic. `set*` is not here: which flag an intrinsic returns is part of
# what it is.
GLUE = re.compile(
    r"^(mov[a-z]*|lea[lq]?|push[lq]?|pop[lq]?|ret[lq]?|xor[lq]?|and[lq]?|or[lq]?|add[lq]?|sub[lq]?|"
    r"sh[lr][lq]?|sa[lr][lq]?|nop[wl]?|endbr64|endbr32|call[lq]?|cltq|cdqe|cwtl|data16|xchg|"
    r"test[blq]?|cmp[blq]?|j[a-z]+|not[lq]?|neg[lq]?|cs|ds)$")

# Registers an instruction reads without naming them.
A, B, C, D = "ax", "bx", "cx", "dx"
IMPLICIT = {
    "clzero": [A], "rdpkru": [C], "wrpkru": [A, C, D], "hreset": [A],
    "umwait": [A, D], "tpause": [A, D], "monitorx": [A, C, D], "mwaitx": [C, A, B],
    "xsaveopt": [A, D], "xsavec": [A, D], "xsaves": [A, D], "xrstors": [A, D],
    "xsaveopt64": [A, D], "xsavec64": [A, D], "xsaves64": [A, D], "xrstors64": [A, D],
    "encls": [A, B, C, D], "enclu": [A, B, C, D], "enclv": [A, B, C, D],
    "pconfig": [A, B, C, D], "rdpmc": [C], "xbegin": [A],
}
# A constant leaf loads only the registers the leaf defines (a caller of a
# two-operand leaf passes two): GCC leaves the others alone, and so they
# hold whatever the caller left there in both programs.
IMPLICIT_BY_CALLER = {"t_encls_ecreate": [A, B, C], "t_encls_eremove": [A, C]}
# RDSSP reads its register operand as well (it is left alone where shadow
# stacks are off): it has to be zero.
ZERO_OPERAND = {"rdsspq", "rdsspd"}
# The register operand is only written: what it held before is nobody's.
WRITE_ONLY = {"rdpid", "rdfsbase", "rdgsbase"}
# These take their source from a register or from memory, and which one is
# the compiler's choice (on i686 GCC reads the argument where the caller
# left it): the form is the operand's width, which a memory operand states
# in the mnemonic's suffix.
REGISTER_OR_MEMORY = {"crc32": {"b": 8, "w": 16, "l": 32, "q": 64}, "ptwrite": {"l": 32, "q": 64}}
# These go through the stack; stepping over one half would leave it unbalanced.
NOT_STEPPED = {"t_readeflags", "t_writeeflags"}

REG64 = "rax rbx rcx rdx rsi rdi rbp rsp r8 r9 r10 r11 r12 r13 r14 r15".split()
REG32 = "eax ebx ecx edx esi edi ebp esp".split()


def register_class(name: str) -> tuple[str, int]:
    """('ax', 64) for rax, ('ax', 32) for eax, ('r8', 32) for r8d, ..."""
    m = re.fullmatch(r"r(\d+)([dwb]?)", name)
    if m:
        return f"r{m.group(1)}", {"": 64, "d": 32, "w": 16, "b": 8}[m.group(2)]
    if name.startswith("r"):
        return name[1:], 64
    if name.startswith("e"):
        return name[1:], 32
    if name.endswith("l") and len(name) <= 3:
        return {"al": "ax", "bl": "bx", "cl": "cx", "dl": "dx", "sil": "si", "dil": "di",
                "bpl": "bp", "spl": "sp"}[name], 8
    if name.endswith("h"):
        return name[0] + "x", 8
    return name, 16


def split_operands(text: str) -> list[str]:
    parts, depth, current = [], 0, ""
    for ch in text:
        if ch == "(":
            depth += 1
        elif ch == ")":
            depth -= 1
        if ch == "," and depth == 0:
            parts.append(current)
            current = ""
        else:
            current += ch
    if current.strip():
        parts.append(current)
    return [p.strip() for p in parts]


def operand_kind(op: str) -> str:
    if op.startswith("$"):
        return f"imm({op[1:]})"
    if op.startswith("%"):
        return f"reg{register_class(op[1:])[1]}"
    if "(%rip)" in op or "(%eip)" in op:
        return "mem(pc)"
    if op.startswith("*"):
        return "indirect"
    return "mem" if "(" in op or re.match(r"^(0x)?[0-9a-f]+$", op) else f"sym({op})"


class Insn:
    def __init__(self, addr: int, mnemonic: str, operands: list[str], comment: str):
        self.addr, self.mnemonic, self.operands, self.comment = addr, mnemonic, operands, comment
        self.next = 0

    def stem(self) -> str:
        """The mnemonic without the operand-size suffix objdump adds to a
        `REGISTER_OR_MEMORY` instruction whose source is in memory."""
        for stem in REGISTER_OR_MEMORY:
            if self.mnemonic.startswith(stem) and self.mnemonic[len(stem):] in ("", "b", "w", "l", "q"):
                return stem
        return self.mnemonic

    def shape(self, wide: bool) -> str:
        stem = self.stem()
        kinds = [operand_kind(op) for op in self.operands]
        if stem in REGISTER_OR_MEMORY and kinds and kinds[0] == "mem":
            suffix = self.mnemonic[len(stem):]
            width = REGISTER_OR_MEMORY[stem].get(suffix, 64 if wide and stem == "ptwrite" and suffix == "q" else 32)
            kinds[0] = f"reg{width}"
        return " ".join([stem] + kinds)


def disassemble(objdump: str, binary: Path) -> dict[str, list[Insn]]:
    text = subprocess.run([objdump, "-d", "--no-show-raw-insn", str(binary)],
                          capture_output=True, text=True, check=True).stdout
    functions: dict[str, list[Insn]] = {}
    current = None
    for line in text.splitlines():
        head = re.match(r"^[0-9a-f]+ <(\w+)>:$", line)
        if head:
            current = functions.setdefault(head.group(1), []) if head.group(1).startswith("t_") else None
            continue
        m = re.match(r"^\s*([0-9a-f]+):\s+(.*)$", line)
        if not m or current is None:
            continue
        body, _, comment = m.group(2).partition("#")
        fields = body.split(None, 1)
        if not fields:
            continue
        # `xbegin 401234 <t_xbegin+0xb>`: the target is not an operand to compare.
        operands = split_operands(re.sub(r"<[^>]*>", "", fields[1])) if len(fields) > 1 else []
        if fields[0] == "xbegin":
            operands = []
        if current:
            current[-1].next = int(m.group(1), 16)
        current.append(Insn(int(m.group(1), 16), fields[0], operands, comment.strip()))
    return functions


def contract(insns: list[Insn]) -> list[Insn]:
    return [i for i in insns if not GLUE.match(i.mnemonic)]


def driver_source(source: Path, wide: bool) -> str:
    """A `main` that calls every caller with arguments each compiler's code
    can be recognised by: pointers into one buffer, distinct integers."""
    text = source.read_text()
    if not wide:
        text = re.sub(r"#ifdef __x86_64__\n.*?#else\n(.*?)#endif", r"\1", text, flags=re.S)
    else:
        text = re.sub(r"#else\n.*?#endif", "#endif", text, flags=re.S)
    calls = []
    for m in re.finditer(r"^[\w ]+?\**\s*(t_\w+)\(([^)]*)\)\s*\{", text, re.M):
        name, params = m.group(1), m.group(2).strip()
        args = []
        for index, param in enumerate([] if params == "void" else params.split(",")):
            param = param.strip()
            if "*" in param:
                args.append(f"(void *)(g_buf + {64 * (index + 1)})")
            elif "long long" in param:
                args.append(f"0x11223344556677{88 + index}ull")
            else:
                args.append(f"0x{0x1001 + 0x110 * index:x}")
        calls.append(f"    {name}({', '.join(args)});")
    protos = "\n".join(
        m.group(0).rstrip("{").strip() + ";"
        for m in re.finditer(r"^[\w ]+?\**\s*t_\w+\([^)]*\)\s*\{", text, re.M))
    return ("#include <stddef.h>\n" + protos +
            "\nchar g_buf[4096] __attribute__((aligned(64)));\nint main(void)\n{\n" +
            "\n".join(calls) + "\n    return 0;\n}\n")


def register_dump(gdb: str, binary: Path, stops: list[tuple[str, int, int]], wide: bool):
    """{caller: {register: value}} at each stop; the stop's instruction is
    stepped over, never executed."""
    regs = REG64 if wide else REG32
    script = ["set pagination off", "set confirm off", "set width 0"]
    for name, addr, after in stops:
        script += [f"break *{addr:#x}", "commands", "silent", f'printf "STOP {name}\\n"']
        script += [f'printf "REG {reg} %lx\\n", (unsigned long)${reg}' for reg in regs]
        script += ['printf "BUF %lx\\n", (unsigned long)&g_buf', f"set $pc = {after:#x}", "continue", "end"]
    script += ["run", "quit"]
    path = binary.with_suffix(".gdb")
    path.write_text("\n".join(script) + "\n")
    out = subprocess.run([gdb, "-q", "-nx", "-batch", "-x", str(path), str(binary)],
                         capture_output=True, text=True, timeout=300, check=False).stdout
    dumps: dict[str, dict[str, int]] = {}
    current = None
    for line in out.splitlines():
        fields = line.split()
        if len(fields) == 2 and fields[0] == "STOP":
            current = dumps.setdefault(fields[1], {})
        elif current is not None and len(fields) == 3 and fields[0] == "REG":
            current[fields[1]] = int(fields[2], 16)
        elif current is not None and len(fields) == 2 and fields[0] == "BUF":
            current["buf"] = int(fields[1], 16)
    return dumps, out


def read_register(dump: dict[str, int], name: str, wide: bool) -> int:
    family, width = register_class(name)
    full = (family if family.startswith("r") else "r" + family) if wide else "e" + family
    value = dump[full]
    return value & ((1 << width) - 1)


def operand_value(op: str, dump: dict[str, int], wide: bool) -> str:
    """What the operand denotes when the instruction is reached."""
    if op.startswith("$"):
        return op
    if op.startswith("%"):
        value = read_register(dump, op[1:], wide)
        if dump["buf"] <= value < dump["buf"] + 4096:
            return f"buf{value - dump['buf']:+#x}"
        return f"{value:#x}"
    m = re.fullmatch(r"(-?(?:0x)?[0-9a-f]*)\((%\w+)?(?:,(%\w+)(?:,(\d))?)?\)", op)
    if not m or (m.group(2) or "").lstrip("%") in ("rip", "eip"):
        return op if not m else "pc-relative"
    address = int(m.group(1) or "0", 0)
    if m.group(2):
        address += read_register(dump, m.group(2)[1:], wide)
    if m.group(3):
        address += read_register(dump, m.group(3)[1:], wide) * int(m.group(4) or "1")
    address &= (1 << (64 if wide else 32)) - 1
    return f"buf{address - dump['buf']:+#x}"


def check_mode(args, tools, wide: bool, tmp: Path, source: Path = CALLERS,
               lccc_flags: tuple[str, ...] = ()) -> tuple[list[str], int, int]:
    mode = "-m64" if wide else "-m32"
    flags = ["-O2", mode, "-no-pie", "-fno-pie", *OPTIONS, *(OPTIONS_64 if wide else [])]
    driver = tmp / f"driver-{source.stem}{mode}.c"
    driver.write_text(driver_source(source, wide))
    binaries = {}
    for label, compiler in (("lccc", args.lccc), ("gcc", tools["gcc"])):
        if compiler is None:
            continue
        binary = tmp / f"{label}-{source.stem}{mode}"
        extra = lccc_flags if label == "lccc" else ()
        built = subprocess.run([compiler, *flags, *extra, str(source), str(driver), "-o", str(binary)],
                               capture_output=True, text=True, check=False)
        if built.returncode != 0:
            return [f"{mode}: {label} does not build the fixture: {built.stderr.strip()[:400]}"], 0, 0
        binaries[label] = binary
    ours = disassemble(tools["objdump"], binaries["lccc"])
    problems = []
    if not ours:
        return [f"{mode}: no caller found in lccc's program"], 0, 0
    if "gcc" not in binaries:
        return [], len(ours), 0
    theirs = disassemble(tools["objdump"], binaries["gcc"])
    stops = {"lccc": [], "gcc": []}
    for name in sorted(theirs):
        if name not in ours:
            problems.append(f"{mode}: {name}: missing from lccc's program")
            continue
        mine, want = contract(ours[name]), contract(theirs[name])
        if [i.shape(wide) for i in mine] != [i.shape(wide) for i in want]:
            problems.append(f"{mode}: {name}: lccc `{'; '.join(i.shape(wide) for i in mine)}`, "
                            f"GCC `{'; '.join(i.shape(wide) for i in want)}`")
            continue
        if name in NOT_STEPPED or not want:
            continue
        stops["lccc"].append((name, mine[0].addr, mine[0].next))
        stops["gcc"].append((name, want[0].addr, want[0].next))
    compared = 0
    if tools["gdb"]:
        dumps = {}
        for label in ("lccc", "gcc"):
            dumps[label], raw = register_dump(tools["gdb"], binaries[label], stops[label], wide)
            if len(dumps[label]) != len(stops[label]):
                problems.append(f"{mode}: GDB stopped in {len(dumps[label])} of {len(stops[label])} "
                                f"callers of {label}'s program: {raw.strip()[-300:]}")
        for (name, _, _) in stops["gcc"]:
            if name not in dumps["lccc"] or name not in dumps["gcc"]:
                continue
            mine, want = contract(ours[name])[0], contract(theirs[name])[0]
            seen = {}
            for label, insn in (("lccc", mine), ("gcc", want)):
                dump = dumps[label][name]
                values = [operand_value(op, dump, wide) for op in insn.operands]
                stem = insn.stem()
                if stem in WRITE_ONLY:
                    values = []
                if stem in REGISTER_OR_MEMORY:
                    # A source in memory is the argument where the caller put
                    # it: the same value by construction, not a register to
                    # read. CRC32 reads the low half of its accumulator.
                    if operand_kind(mine.operands[0]) != operand_kind(want.operands[0]):
                        values[0] = "the source argument"
                    if stem == "crc32":
                        values[-1] = f"{int(values[-1], 16) & 0xffffffff:#x}"
                families = IMPLICIT_BY_CALLER.get(name, IMPLICIT.get(insn.mnemonic, []))
                for family in families:
                    width = 64 if wide and insn.mnemonic in ("clzero", "monitorx") and family == A else 32
                    if insn.mnemonic in ("encls", "enclu", "enclv", "pconfig") and family != A:
                        width = 64 if wide else 32
                    value = dump[("r" if wide else "e") + family] & ((1 << width) - 1)
                    if dump["buf"] <= value < dump["buf"] + 4096:
                        values.append(f"{family}=buf{value - dump['buf']:+#x}")
                    else:
                        values.append(f"{family}={value:#x}")
                if insn.mnemonic in ZERO_OPERAND and values[0] != "0x0":
                    problems.append(f"{mode}: {name}: {label} runs {insn.mnemonic} on a register "
                                    f"holding {values[0]}, not zero")
                seen[label] = values
            compared += 1
            if seen["lccc"] != seen["gcc"]:
                problems.append(f"{mode}: {name}: at `{want.mnemonic}` lccc has {seen['lccc']}, "
                                f"GCC has {seen['gcc']}")
    return problems, len(theirs), compared


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--lccc", default=str(ROOT / "target/fastbuild/lccc"))
    parser.add_argument("--gcc", default=None)
    parser.add_argument("--gdb", default=None)
    parser.add_argument("--objdump", default=shutil.which("objdump") or "objdump")
    parser.add_argument("--allow-missing-tools", action="store_true",
                        help="pass without GCC 16.2 or GDB, saying what was not checked")
    args = parser.parse_args()

    gcc = None
    for candidate in (args.gcc, "gcc-16", "gcc"):
        path = candidate and shutil.which(candidate)
        if path and "16.2" in subprocess.run([path, "-dumpfullversion"], capture_output=True,
                                             text=True, check=False).stdout:
            gcc = path
            break
    gdb = shutil.which(args.gdb or "gdb")
    tools = {"gcc": gcc, "gdb": gdb, "objdump": args.objdump}
    missing = [name for name in ("gcc", "gdb") if tools[name] is None]
    if missing and not args.allow_missing_tools:
        print(f"FAIL: not found: {', '.join(missing)} (GCC must be 16.2; or --allow-missing-tools)")
        return 1

    problems, summary = [], []
    with tempfile.TemporaryDirectory(prefix="lccc-gpr-intrinsics-") as tmp:
        # The gate itself first: three planted differences, each reported.
        if not missing:
            found, _, _ = check_mode(args, tools, True, Path(tmp), CONTROL, ("-DCONTROL_BROKEN",))
            if sorted(found) != sorted(f"-m64: {finding}" for finding in CONTROL_FINDINGS):
                problems.append(f"negative controls: expected {CONTROL_FINDINGS}, got {found}")
        for wide in (True, False):
            found, callers, compared = check_mode(args, tools, wide, Path(tmp))
            problems += found
            summary.append(f"{'x86-64' if wide else 'i686'}: {callers} callers, "
                           f"{compared} operand sets compared under GDB")
    for problem in problems:
        print(f"FAIL: {problem}")
    unchecked = (f"; NOT CHECKED (missing {', '.join(missing)})" if missing
                 else "; 3 negative controls reported")
    print(f"x86-gpr-intrinsics: {'; '.join(summary)}{unchecked}, {'FAIL' if problems else 'PASS'}")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
