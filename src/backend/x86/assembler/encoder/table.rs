//! Instructions encoded from a table.
//!
//! The encoders grew one `match` arm at a time, and an instruction no arm
//! names is "unhandled" however plain its encoding. The rows here are the
//! plain ones: a fixed opcode, at most one ModR/M pair, immediates, and an
//! operand size that follows the usual rules. A row is the whole
//! description of a form, so a missing instruction or a missing spelling
//! (`verrw`, `ud1 %ecx, %eax`, `in $5`) is a line, not an encoder.
//!
//! [`plan`] matches a statement against the rows and says what to emit; the
//! 64-bit and the 32-bit encoder each turn that into bytes with their own
//! prefix and ModR/M helpers (`InstructionEncoder::emit_plan`).
//!
//! Every row was written from the Intel and AMD manuals and then checked
//! against GNU as 2.47 statement by statement, in 64-bit and in 32-bit mode
//! (`scripts/gas_table_sweep.py`; the unit tests below pin a sample).

use crate::backend::x86::assembler::parser::{ImmediateValue, MemoryOperand, Operand, Register};

/// One operand a row takes, in AT&T order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Arg {
    /// ModR/M `reg`: a general register of the operand size.
    Reg,
    /// ModR/M `reg`: a general register that names an address (`movdir64b
    /// (%rdx), %rcx`): as wide as addresses are, and no word on the operand
    /// size.
    RegAddr,
    /// ModR/M `r/m`: a general register of the operand size, or memory.
    Rm,
    /// ModR/M `r/m`: a general register of the operand size.
    RmReg,
    /// ModR/M `r/m`: a 32-bit general register, whatever the operand size.
    RmReg32,
    /// ModR/M `r/m`: a 64-bit general register, whatever the operand size.
    RmReg64,
    /// ModR/M `r/m`: a general register that names an address (`umonitor
    /// %rcx`); one narrower than addresses are takes the address-size
    /// prefix.
    RmAddr,
    /// ModR/M `r/m`: memory.
    Mem,
    /// The accumulator of the operand size, implied by the opcode.
    Acc,
    /// The accumulator as an address (`vmrun %rax`), implied by the opcode;
    /// narrower than addresses are, it takes the address-size prefix.
    AccAddr,
    /// A register the opcode implies, by name.
    Implied(&'static str),
    /// The port register: `%dx` or `(%dx)`.
    Port,
    /// A memory operand of which only the segment and the address size are
    /// encoded (`xlat %es:(%rbx)`): one that names a register.
    Segment,
    /// An 8-bit immediate, signed or unsigned.
    Imm8,
    /// An unsigned 8-bit immediate.
    Imm8u,
    /// An immediate of which the low 8 bits are taken (GNU as: a port
    /// number).
    Imm8t,
    /// A 16-bit immediate, signed or unsigned.
    Imm16,
    /// A branch target: a 32-bit displacement from the end of the
    /// instruction.
    Rel32,
    /// A register of the x87 stack: its number is added to the opcode.
    St,
}

/// What an operand size costs in a 32- or 64-bit code segment.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Width {
    /// Nothing: the default size, or a size the instruction ignores.
    Plain,
    /// The operand-size prefix `66`.
    Data16,
    /// `REX.W`.
    RexW,
}

/// Where a row is valid.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Mode {
    Any,
    Long,
    Legacy,
}

/// One form of one instruction.
#[derive(Debug)]
pub(crate) struct Row {
    name: &'static str,
    args: &'static [Arg],
    /// The mandatory prefix in front of the opcode (`66`, `F2`, `F3`), or 0.
    prefix: u8,
    opcode: &'static [u8],
    /// The ModR/M `reg` field, when it is part of the opcode.
    digit: Option<u8>,
    /// The operand sizes the form takes, by suffix letter, and what each
    /// costs. `' '` is the form without a suffix and without a register
    /// that implies one.
    sizes: &'static [(u8, Width)],
    /// The low bit of the opcode selects the operand size: clear for `b`.
    wide_bit: bool,
    /// A stack operation whose unsuffixed form `.code16gcc` makes 32 bits
    /// wide (GNU as: `leave` there is `66 c9`).
    stack: bool,
    mode: Mode,
}

const P: Width = Width::Plain;
const D: Width = Width::Data16;
const W: Width = Width::RexW;

/// No operand size at all.
const FIXED: &[(u8, Width)] = &[(b' ', P)];
/// 16, 32 or 64 bits, from a register or a suffix.
const WLQ: &[(u8, Width)] = &[(b'w', D), (b'l', P), (b'q', W)];
/// The same for a form that may be all memory: 32 bits unless told.
const WLQ_MEM: &[(u8, Width)] = &[(b' ', P), (b'w', D), (b'l', P), (b'q', W)];
/// 16 or 32 bits.
const WL: &[(u8, Width)] = &[(b'w', D), (b'l', P)];
/// 32 or 64 bits; all memory is 32.
const LQ_MEM: &[(u8, Width)] = &[(b' ', P), (b'l', P), (b'q', W)];
/// 16, 32 or 64 bits, the last without `REX.W` (the instruction stores a
/// 16-bit value and zero-extends it).
const WLQ_NOREX: &[(u8, Width)] = &[(b'w', D), (b'l', P), (b'q', P)];
/// A register of any width: the instruction reads its low 16 bits.
const ANY: &[(u8, Width)] = &[(b'w', P), (b'l', P), (b'q', P)];
/// A 16-bit memory operand.
const WORD_MEM: &[(u8, Width)] = &[(b' ', P), (b'w', P)];
/// A memory operand that `q` widens.
const Q_MEM: &[(u8, Width)] = &[(b' ', P), (b'q', W)];
/// A memory operand `q` says nothing new about.
const Q_SAME: &[(u8, Width)] = &[(b' ', P), (b'q', P)];
/// Port input and output: a byte, a word or a doubleword.
const BWL: &[(u8, Width)] = &[(b'b', P), (b'w', D), (b'l', P)];
const BWL_OR_NONE: &[(u8, Width)] = &[(b' ', P), (b'b', P), (b'w', D), (b'l', P)];
/// A descriptor-table operand: six bytes in 32-bit code, ten in 64-bit
/// code, and a 16-bit limit with a 24-bit base under `w`.
const TABLE32: &[(u8, Width)] = &[(b' ', P), (b'w', D), (b'l', P)];
/// The x87 environment: 14 or 94 bytes under `s`, 28 or 108 under `l`.
const ENVIRONMENT: &[(u8, Width)] = &[(b's', D), (b'l', P)];
/// The stack operations of 64-bit mode: 64 bits unless `w`.
const STACK64: &[(u8, Width)] = &[(b' ', P), (b'w', D), (b'q', P)];
/// The same in 32-bit mode.
const STACK32: &[(u8, Width)] = &[(b' ', P), (b'w', D), (b'l', P)];
/// Far returns and `iret`: 32 bits unless told.
const FAR: &[(u8, Width)] = &[(b' ', P), (b'w', D), (b'l', P), (b'q', W)];

const fn row(
    name: &'static str,
    args: &'static [Arg],
    prefix: u8,
    opcode: &'static [u8],
    digit: Option<u8>,
    sizes: &'static [(u8, Width)],
) -> Row {
    Row {
        name,
        args,
        prefix,
        opcode,
        digit,
        sizes,
        wide_bit: false,
        stack: false,
        mode: Mode::Any,
    }
}

/// A stack operation (see `Row::stack`).
const fn stack(mut r: Row) -> Row {
    r.stack = true;
    r
}

/// A form of 64-bit mode only.
const fn long(mut r: Row) -> Row {
    r.mode = Mode::Long;
    r
}

/// A form of 16- and 32-bit mode only.
const fn legacy(mut r: Row) -> Row {
    r.mode = Mode::Legacy;
    r
}

/// The opcode's low bit follows the operand size.
const fn wide(mut r: Row) -> Row {
    r.wide_bit = true;
    r
}

/// An instruction that is its opcode and nothing else.
const fn bytes(name: &'static str, prefix: u8, opcode: &'static [u8]) -> Row {
    row(name, &[], prefix, opcode, None, FIXED)
}

use Arg::*;

const EAX: Arg = Implied("eax");
const ECX: Arg = Implied("ecx");
const EDX: Arg = Implied("edx");
const RAX: Arg = Implied("rax");
const RCX: Arg = Implied("rcx");
const RDX: Arg = Implied("rdx");
const ST0: Arg = Implied("st");
const CL: Arg = Implied("cl");

/// The rows, by family. Lookup is a scan: the table is consulted for an
/// instruction no `match` arm of the encoders took, which is rare.
static ROWS: &[Row] = &[
    // --- Segment and system-table words (0F 00, 0F 01, 0F 02, 0F 03) ---
    // The stores write 16 bits and take an operand size only for the
    // register form; the loads read 16 bits whatever the register is called.
    row("sldt", &[RmReg], 0, &[0x0F, 0x00], Some(0), WLQ_NOREX),
    row("sldt", &[Mem], 0, &[0x0F, 0x00], Some(0), WORD_MEM),
    row("str", &[RmReg], 0, &[0x0F, 0x00], Some(1), WLQ_NOREX),
    row("str", &[Mem], 0, &[0x0F, 0x00], Some(1), WORD_MEM),
    row("lldt", &[RmReg], 0, &[0x0F, 0x00], Some(2), ANY),
    row("lldt", &[Mem], 0, &[0x0F, 0x00], Some(2), WORD_MEM),
    row("ltr", &[RmReg], 0, &[0x0F, 0x00], Some(3), ANY),
    row("ltr", &[Mem], 0, &[0x0F, 0x00], Some(3), WORD_MEM),
    row("verr", &[RmReg], 0, &[0x0F, 0x00], Some(4), ANY),
    row("verr", &[Mem], 0, &[0x0F, 0x00], Some(4), WORD_MEM),
    row("verw", &[RmReg], 0, &[0x0F, 0x00], Some(5), ANY),
    row("verw", &[Mem], 0, &[0x0F, 0x00], Some(5), WORD_MEM),
    row("smsw", &[RmReg], 0, &[0x0F, 0x01], Some(4), WLQ),
    row("smsw", &[Mem], 0, &[0x0F, 0x01], Some(4), WORD_MEM),
    row("lmsw", &[RmReg], 0, &[0x0F, 0x01], Some(6), &[(b'w', P)]),
    row("lmsw", &[Mem], 0, &[0x0F, 0x01], Some(6), WORD_MEM),
    // `lar`/`lsl` read a selector (16 bits of a register of the operand
    // size, or of a 16-bit one, or memory) into a register of any size.
    row("lar", &[Rm, Reg], 0, &[0x0F, 0x02], None, WLQ_NOREX),
    row("lsl", &[Rm, Reg], 0, &[0x0F, 0x03], None, WLQ_NOREX),
    long(row("lkgs", &[RmReg], 0xF2, &[0x0F, 0x00], Some(6), ANY)),
    long(row("lkgs", &[Mem], 0xF2, &[0x0F, 0x00], Some(6), WORD_MEM)),
    legacy(row(
        "arpl",
        &[Reg, Rm],
        0,
        &[0x63],
        None,
        &[(b'w', P), (b'l', P)],
    )),
    legacy(row("bound", &[Reg, Mem], 0, &[0x62], None, WL)),
    legacy(row("sgdt", &[Mem], 0, &[0x0F, 0x01], Some(0), TABLE32)),
    legacy(row("sidt", &[Mem], 0, &[0x0F, 0x01], Some(1), TABLE32)),
    legacy(row("lgdt", &[Mem], 0, &[0x0F, 0x01], Some(2), TABLE32)),
    legacy(row("lidt", &[Mem], 0, &[0x0F, 0x01], Some(3), TABLE32)),
    bytes("clts", 0, &[0x0F, 0x06]),
    // --- Double-precision shifts: count, source, destination; the count
    // is `%cl` when it is not written ---
    row("shld", &[Imm8, Reg, Rm], 0, &[0x0F, 0xA4], None, WLQ),
    row("shld", &[CL, Reg, Rm], 0, &[0x0F, 0xA5], None, WLQ),
    row("shld", &[Reg, Rm], 0, &[0x0F, 0xA5], None, WLQ),
    row("shrd", &[Imm8, Reg, Rm], 0, &[0x0F, 0xAC], None, WLQ),
    row("shrd", &[CL, Reg, Rm], 0, &[0x0F, 0xAD], None, WLQ),
    row("shrd", &[Reg, Rm], 0, &[0x0F, 0xAD], None, WLQ),
    // --- Far pointers ---
    row("lss", &[Mem, Reg], 0, &[0x0F, 0xB2], None, WL),
    row("lfs", &[Mem, Reg], 0, &[0x0F, 0xB4], None, WL),
    row("lgs", &[Mem, Reg], 0, &[0x0F, 0xB5], None, WL),
    legacy(row("lds", &[Mem, Reg], 0, &[0xC5], None, WL)),
    legacy(row("les", &[Mem, Reg], 0, &[0xC4], None, WL)),
    // --- Port input and output ---
    wide(row("in", &[Imm8t, Acc], 0, &[0xE4], None, BWL)),
    wide(row("in", &[Port, Acc], 0, &[0xEC], None, BWL)),
    wide(row("in", &[Imm8t], 0, &[0xE4], None, BWL_OR_NONE)),
    wide(row("in", &[Port], 0, &[0xEC], None, BWL_OR_NONE)),
    wide(row("out", &[Acc, Imm8t], 0, &[0xE6], None, BWL)),
    wide(row("out", &[Acc, Port], 0, &[0xEE], None, BWL)),
    wide(row("out", &[Imm8t], 0, &[0xE6], None, BWL_OR_NONE)),
    wide(row("out", &[Port], 0, &[0xEE], None, BWL_OR_NONE)),
    // --- Stack frames, flags, returns ---
    long(row("enter", &[Imm16, Imm8], 0, &[0xC8], None, STACK64)),
    legacy(stack(row(
        "enter",
        &[Imm16, Imm8],
        0,
        &[0xC8],
        None,
        STACK32,
    ))),
    long(row("leave", &[], 0, &[0xC9], None, STACK64)),
    legacy(stack(row("leave", &[], 0, &[0xC9], None, STACK32))),
    long(row("pushf", &[], 0, &[0x9C], None, STACK64)),
    legacy(stack(row("pushf", &[], 0, &[0x9C], None, STACK32))),
    long(row("popf", &[], 0, &[0x9D], None, STACK64)),
    legacy(stack(row("popf", &[], 0, &[0x9D], None, STACK32))),
    long(row("ret", &[], 0, &[0xC3], None, STACK64)),
    long(row("ret", &[Imm16], 0, &[0xC2], None, STACK64)),
    legacy(stack(row("ret", &[], 0, &[0xC3], None, STACK32))),
    legacy(stack(row("ret", &[Imm16], 0, &[0xC2], None, STACK32))),
    stack(row("lret", &[], 0, &[0xCB], None, FAR)),
    stack(row("lret", &[Imm16], 0, &[0xCA], None, FAR)),
    stack(row("retf", &[], 0, &[0xCB], None, FAR)),
    stack(row("retf", &[Imm16], 0, &[0xCA], None, FAR)),
    row("iret", &[], 0, &[0xCF], None, FAR),
    // --- Translation ---
    row("xlat", &[], 0, &[0xD7], None, &[(b' ', P), (b'b', P)]),
    row(
        "xlat",
        &[Segment],
        0,
        &[0xD7],
        None,
        &[(b' ', P), (b'b', P)],
    ),
    // --- Decimal adjustment ---
    legacy(bytes("aaa", 0, &[0x37])),
    legacy(bytes("aas", 0, &[0x3F])),
    legacy(bytes("daa", 0, &[0x27])),
    legacy(bytes("das", 0, &[0x2F])),
    legacy(bytes("aam", 0, &[0xD4, 0x0A])),
    legacy(row("aam", &[Imm8], 0, &[0xD4], None, FIXED)),
    legacy(bytes("aad", 0, &[0xD5, 0x0A])),
    legacy(row("aad", &[Imm8], 0, &[0xD5], None, FIXED)),
    // --- Traps ---
    bytes("int1", 0, &[0xF1]),
    legacy(bytes("into", 0, &[0xCE])),
    bytes("ud2a", 0, &[0x0F, 0x0B]),
    bytes("ud0", 0, &[0x0F, 0xFF]),
    row("ud0", &[Rm, Reg], 0, &[0x0F, 0xFF], None, WLQ),
    bytes("ud1", 0, &[0x0F, 0xB9]),
    row("ud1", &[Rm, Reg], 0, &[0x0F, 0xB9], None, WLQ),
    bytes("ud2b", 0, &[0x0F, 0xB9]),
    row("ud2b", &[Rm, Reg], 0, &[0x0F, 0xB9], None, WLQ),
    long(bytes("udb", 0, &[0xD6])),
    // --- Waiting, tracing, cache lines, direct stores ---
    row("tpause", &[RmReg32], 0x66, &[0x0F, 0xAE], Some(6), FIXED),
    row(
        "tpause",
        &[RmReg32, EDX, EAX],
        0x66,
        &[0x0F, 0xAE],
        Some(6),
        FIXED,
    ),
    row("umwait", &[RmReg32], 0xF2, &[0x0F, 0xAE], Some(6), FIXED),
    row(
        "umwait",
        &[RmReg32, EDX, EAX],
        0xF2,
        &[0x0F, 0xAE],
        Some(6),
        FIXED,
    ),
    row("umonitor", &[RmAddr], 0xF3, &[0x0F, 0xAE], Some(6), FIXED),
    row("ptwrite", &[Rm], 0xF3, &[0x0F, 0xAE], Some(4), LQ_MEM),
    row("cldemote", &[Mem], 0, &[0x0F, 0x1C], Some(0), FIXED),
    row(
        "movdiri",
        &[Reg, Mem],
        0,
        &[0x0F, 0x38, 0xF9],
        None,
        &[(b'l', P), (b'q', W)],
    ),
    row(
        "movdir64b",
        &[Mem, RegAddr],
        0x66,
        &[0x0F, 0x38, 0xF8],
        None,
        FIXED,
    ),
    row(
        "enqcmd",
        &[Mem, RegAddr],
        0xF2,
        &[0x0F, 0x38, 0xF8],
        None,
        FIXED,
    ),
    row(
        "enqcmds",
        &[Mem, RegAddr],
        0xF3,
        &[0x0F, 0x38, 0xF8],
        None,
        FIXED,
    ),
    // --- Transactions ---
    row("xbegin", &[Rel32], 0, &[0xC7, 0xF8], None, FIXED),
    row("xabort", &[Imm8u], 0, &[0xC6, 0xF8], None, FIXED),
    bytes("xend", 0, &[0x0F, 0x01, 0xD5]),
    bytes("xtest", 0, &[0x0F, 0x01, 0xD6]),
    bytes("xsusldtrk", 0xF2, &[0x0F, 0x01, 0xE8]),
    bytes("xresldtrk", 0xF2, &[0x0F, 0x01, 0xE9]),
    // --- Processor state: the `q` spellings of the 64-bit forms ---
    row("fxsave", &[Mem], 0, &[0x0F, 0xAE], Some(0), Q_MEM),
    row("fxrstor", &[Mem], 0, &[0x0F, 0xAE], Some(1), Q_MEM),
    row("xsave", &[Mem], 0, &[0x0F, 0xAE], Some(4), Q_MEM),
    row("xrstor", &[Mem], 0, &[0x0F, 0xAE], Some(5), Q_MEM),
    row("xsaveopt", &[Mem], 0, &[0x0F, 0xAE], Some(6), Q_MEM),
    row("cmpxchg8b", &[Mem], 0, &[0x0F, 0xC7], Some(1), Q_SAME),
    // --- Virtualisation (AMD SVM, SEV-SNP) ---
    bytes("clgi", 0, &[0x0F, 0x01, 0xDD]),
    bytes("stgi", 0, &[0x0F, 0x01, 0xDC]),
    bytes("vmrun", 0, &[0x0F, 0x01, 0xD8]),
    row("vmrun", &[AccAddr], 0, &[0x0F, 0x01, 0xD8], None, FIXED),
    bytes("vmload", 0, &[0x0F, 0x01, 0xDA]),
    row("vmload", &[AccAddr], 0, &[0x0F, 0x01, 0xDA], None, FIXED),
    bytes("vmsave", 0, &[0x0F, 0x01, 0xDB]),
    row("vmsave", &[AccAddr], 0, &[0x0F, 0x01, 0xDB], None, FIXED),
    bytes("skinit", 0, &[0x0F, 0x01, 0xDE]),
    row("skinit", &[EAX], 0, &[0x0F, 0x01, 0xDE], None, FIXED),
    bytes("invlpga", 0, &[0x0F, 0x01, 0xDF]),
    row(
        "invlpga",
        &[AccAddr, ECX],
        0,
        &[0x0F, 0x01, 0xDF],
        None,
        FIXED,
    ),
    bytes("vmgexit", 0xF3, &[0x0F, 0x01, 0xD9]),
    bytes("clzero", 0, &[0x0F, 0x01, 0xFC]),
    row("clzero", &[AccAddr], 0, &[0x0F, 0x01, 0xFC], None, FIXED),
    bytes("invlpgb", 0, &[0x0F, 0x01, 0xFE]),
    row(
        "invlpgb",
        &[AccAddr, ECX, EDX],
        0,
        &[0x0F, 0x01, 0xFE],
        None,
        FIXED,
    ),
    bytes("tlbsync", 0, &[0x0F, 0x01, 0xFF]),
    bytes("rdpru", 0, &[0x0F, 0x01, 0xFD]),
    bytes("mcommit", 0xF3, &[0x0F, 0x01, 0xFA]),
    long(bytes("psmash", 0xF3, &[0x0F, 0x01, 0xFF])),
    long(row(
        "psmash",
        &[AccAddr],
        0xF3,
        &[0x0F, 0x01, 0xFF],
        None,
        FIXED,
    )),
    bytes("pvalidate", 0xF2, &[0x0F, 0x01, 0xFF]),
    row(
        "pvalidate",
        &[AccAddr],
        0xF2,
        &[0x0F, 0x01, 0xFF],
        None,
        FIXED,
    ),
    row(
        "pvalidate",
        &[AccAddr, ECX, EDX],
        0xF2,
        &[0x0F, 0x01, 0xFF],
        None,
        FIXED,
    ),
    long(row(
        "rmpread",
        &[RAX, Segment],
        0xF2,
        &[0x0F, 0x01, 0xFD],
        None,
        FIXED,
    )),
    long(row(
        "rmpupdate",
        &[Segment, RAX],
        0xF2,
        &[0x0F, 0x01, 0xFE],
        None,
        FIXED,
    )),
    long(bytes("rmpupdate", 0xF2, &[0x0F, 0x01, 0xFE])),
    long(row(
        "rmpupdate",
        &[AccAddr],
        0xF2,
        &[0x0F, 0x01, 0xFE],
        None,
        FIXED,
    )),
    long(row(
        "rmpupdate",
        &[AccAddr, RCX],
        0xF2,
        &[0x0F, 0x01, 0xFE],
        None,
        FIXED,
    )),
    long(bytes("rmpadjust", 0xF3, &[0x0F, 0x01, 0xFE])),
    long(row(
        "rmpadjust",
        &[AccAddr],
        0xF3,
        &[0x0F, 0x01, 0xFE],
        None,
        FIXED,
    )),
    long(row(
        "rmpadjust",
        &[AccAddr, RCX, RDX],
        0xF3,
        &[0x0F, 0x01, 0xFE],
        None,
        FIXED,
    )),
    long(bytes("rmpquery", 0xF3, &[0x0F, 0x01, 0xFD])),
    long(row(
        "rmpquery",
        &[AccAddr, RCX, RDX],
        0xF3,
        &[0x0F, 0x01, 0xFD],
        None,
        FIXED,
    )),
    // --- Trust domains, enclaves, key management ---
    long(bytes("seamcall", 0x66, &[0x0F, 0x01, 0xCF])),
    long(bytes("seamops", 0x66, &[0x0F, 0x01, 0xCE])),
    long(bytes("seamret", 0x66, &[0x0F, 0x01, 0xCD])),
    bytes("tdcall", 0x66, &[0x0F, 0x01, 0xCC]),
    bytes("pconfig", 0, &[0x0F, 0x01, 0xC5]),
    long(bytes("pbndkb", 0, &[0x0F, 0x01, 0xC7])),
    bytes("getsec", 0, &[0x0F, 0x37]),
    bytes("rsm", 0, &[0x0F, 0xAA]),
    // --- Interrupts and returns of newer privilege models ---
    long(row(
        "senduipi",
        &[RmReg64],
        0xF3,
        &[0x0F, 0xC7],
        Some(6),
        FIXED,
    )),
    long(bytes("testui", 0xF3, &[0x0F, 0x01, 0xED])),
    long(bytes("uiret", 0xF3, &[0x0F, 0x01, 0xEC])),
    long(bytes("erets", 0xF2, &[0x0F, 0x01, 0xCA])),
    long(bytes("eretu", 0xF3, &[0x0F, 0x01, 0xCA])),
    row(
        "hreset",
        &[Imm8],
        0xF3,
        &[0x0F, 0x3A, 0xF0, 0xC0],
        None,
        FIXED,
    ),
    // --- Model-specific registers ---
    long(bytes("rdmsrlist", 0xF2, &[0x0F, 0x01, 0xC6])),
    long(bytes("wrmsrlist", 0xF3, &[0x0F, 0x01, 0xC6])),
    bytes("wrmsrns", 0, &[0x0F, 0x01, 0xC6]),
    // --- 3DNow! state, VIA PadLock and Zhaoxin ---
    bytes("femms", 0, &[0x0F, 0x0E]),
    bytes("xstore", 0, &[0x0F, 0xA7, 0xC0]),
    bytes("xstorerng", 0, &[0x0F, 0xA7, 0xC0]),
    bytes("xcryptecb", 0xF3, &[0x0F, 0xA7, 0xC8]),
    bytes("xcryptcbc", 0xF3, &[0x0F, 0xA7, 0xD0]),
    bytes("xcryptctr", 0xF3, &[0x0F, 0xA7, 0xD8]),
    bytes("xcryptcfb", 0xF3, &[0x0F, 0xA7, 0xE0]),
    bytes("xcryptofb", 0xF3, &[0x0F, 0xA7, 0xE8]),
    bytes("montmul", 0xF3, &[0x0F, 0xA6, 0xC0]),
    bytes("xsha1", 0xF3, &[0x0F, 0xA6, 0xC8]),
    bytes("xsha256", 0xF3, &[0x0F, 0xA6, 0xD0]),
    bytes("xsha384", 0xF3, &[0x0F, 0xA6, 0xD8]),
    bytes("xsha512", 0xF3, &[0x0F, 0xA6, 0xE0]),
    bytes("sm3", 0xF3, &[0x0F, 0xA6, 0xE8]),
    bytes("montmul2", 0xF3, &[0x0F, 0xA6, 0xF0]),
    bytes("xmodexp", 0xF3, &[0x0F, 0xA6, 0xF8]),
    bytes("sm4", 0xF3, &[0x0F, 0xA7, 0xF0]),
    bytes("xrng2", 0xF3, &[0x0F, 0xA7, 0xF8]),
    bytes("sm2", 0xF2, &[0x0F, 0xA6, 0xC0]),
    // --- x87: a memory operand without a suffix is a single-precision
    // number or a 16-bit integer (what GNU as assumes, with a warning) ---
    row("fadd", &[Mem], 0, &[0xD8], Some(0), FIXED),
    row("fmul", &[Mem], 0, &[0xD8], Some(1), FIXED),
    row("fcom", &[Mem], 0, &[0xD8], Some(2), FIXED),
    row("fcomp", &[Mem], 0, &[0xD8], Some(3), FIXED),
    row("fsub", &[Mem], 0, &[0xD8], Some(4), FIXED),
    row("fsubr", &[Mem], 0, &[0xD8], Some(5), FIXED),
    row("fdiv", &[Mem], 0, &[0xD8], Some(6), FIXED),
    row("fdivr", &[Mem], 0, &[0xD8], Some(7), FIXED),
    row("fld", &[Mem], 0, &[0xD9], Some(0), FIXED),
    row("fst", &[Mem], 0, &[0xD9], Some(2), FIXED),
    row("fstp", &[Mem], 0, &[0xD9], Some(3), FIXED),
    row("fiadd", &[Mem], 0, &[0xDE], Some(0), FIXED),
    row("fimul", &[Mem], 0, &[0xDE], Some(1), FIXED),
    row("ficom", &[Mem], 0, &[0xDE], Some(2), FIXED),
    row("ficomp", &[Mem], 0, &[0xDE], Some(3), FIXED),
    row("fisub", &[Mem], 0, &[0xDE], Some(4), FIXED),
    row("fisubr", &[Mem], 0, &[0xDE], Some(5), FIXED),
    row("fidiv", &[Mem], 0, &[0xDE], Some(6), FIXED),
    row("fidivr", &[Mem], 0, &[0xDE], Some(7), FIXED),
    row("fild", &[Mem], 0, &[0xDF], Some(0), FIXED),
    row("fisttp", &[Mem], 0, &[0xDF], Some(1), FIXED),
    row("fist", &[Mem], 0, &[0xDF], Some(2), FIXED),
    row("fistp", &[Mem], 0, &[0xDF], Some(3), FIXED),
    // ... and the spelled sizes: `s` and `l` of the real forms,
    row("fadds", &[Mem], 0, &[0xD8], Some(0), FIXED),
    row("fmuls", &[Mem], 0, &[0xD8], Some(1), FIXED),
    row("fcoms", &[Mem], 0, &[0xD8], Some(2), FIXED),
    row("fcomps", &[Mem], 0, &[0xD8], Some(3), FIXED),
    row("fsubs", &[Mem], 0, &[0xD8], Some(4), FIXED),
    row("fsubrs", &[Mem], 0, &[0xD8], Some(5), FIXED),
    row("fdivs", &[Mem], 0, &[0xD8], Some(6), FIXED),
    row("fdivrs", &[Mem], 0, &[0xD8], Some(7), FIXED),
    row("faddl", &[Mem], 0, &[0xDC], Some(0), FIXED),
    row("fmull", &[Mem], 0, &[0xDC], Some(1), FIXED),
    row("fcoml", &[Mem], 0, &[0xDC], Some(2), FIXED),
    row("fcompl", &[Mem], 0, &[0xDC], Some(3), FIXED),
    row("fsubl", &[Mem], 0, &[0xDC], Some(4), FIXED),
    row("fsubrl", &[Mem], 0, &[0xDC], Some(5), FIXED),
    row("fdivl", &[Mem], 0, &[0xDC], Some(6), FIXED),
    row("fdivrl", &[Mem], 0, &[0xDC], Some(7), FIXED),
    row("flds", &[Mem], 0, &[0xD9], Some(0), FIXED),
    row("fsts", &[Mem], 0, &[0xD9], Some(2), FIXED),
    row("fstps", &[Mem], 0, &[0xD9], Some(3), FIXED),
    row("fldl", &[Mem], 0, &[0xDD], Some(0), FIXED),
    row("fstl", &[Mem], 0, &[0xDD], Some(2), FIXED),
    row("fstpl", &[Mem], 0, &[0xDD], Some(3), FIXED),
    row("fldt", &[Mem], 0, &[0xDB], Some(5), FIXED),
    row("fstpt", &[Mem], 0, &[0xDB], Some(7), FIXED),
    // `s`, `l` and `ll` (or `q`) of the integer forms,
    row("fiadds", &[Mem], 0, &[0xDE], Some(0), FIXED),
    row("fimuls", &[Mem], 0, &[0xDE], Some(1), FIXED),
    row("ficoms", &[Mem], 0, &[0xDE], Some(2), FIXED),
    row("ficomps", &[Mem], 0, &[0xDE], Some(3), FIXED),
    row("fisubs", &[Mem], 0, &[0xDE], Some(4), FIXED),
    row("fisubrs", &[Mem], 0, &[0xDE], Some(5), FIXED),
    row("fidivs", &[Mem], 0, &[0xDE], Some(6), FIXED),
    row("fidivrs", &[Mem], 0, &[0xDE], Some(7), FIXED),
    row("fiaddl", &[Mem], 0, &[0xDA], Some(0), FIXED),
    row("fimull", &[Mem], 0, &[0xDA], Some(1), FIXED),
    row("ficoml", &[Mem], 0, &[0xDA], Some(2), FIXED),
    row("ficompl", &[Mem], 0, &[0xDA], Some(3), FIXED),
    row("fisubl", &[Mem], 0, &[0xDA], Some(4), FIXED),
    row("fisubrl", &[Mem], 0, &[0xDA], Some(5), FIXED),
    row("fidivl", &[Mem], 0, &[0xDA], Some(6), FIXED),
    row("fidivrl", &[Mem], 0, &[0xDA], Some(7), FIXED),
    row("filds", &[Mem], 0, &[0xDF], Some(0), FIXED),
    row("fisttps", &[Mem], 0, &[0xDF], Some(1), FIXED),
    row("fists", &[Mem], 0, &[0xDF], Some(2), FIXED),
    row("fistps", &[Mem], 0, &[0xDF], Some(3), FIXED),
    row("fildl", &[Mem], 0, &[0xDB], Some(0), FIXED),
    row("fisttpl", &[Mem], 0, &[0xDB], Some(1), FIXED),
    row("fistl", &[Mem], 0, &[0xDB], Some(2), FIXED),
    row("fistpl", &[Mem], 0, &[0xDB], Some(3), FIXED),
    row("fildll", &[Mem], 0, &[0xDF], Some(5), FIXED),
    row("fildq", &[Mem], 0, &[0xDF], Some(5), FIXED),
    row("fistpll", &[Mem], 0, &[0xDF], Some(7), FIXED),
    row("fistpq", &[Mem], 0, &[0xDF], Some(7), FIXED),
    row("fisttpll", &[Mem], 0, &[0xDD], Some(1), FIXED),
    row("fisttpq", &[Mem], 0, &[0xDD], Some(1), FIXED),
    // packed decimal, and the environment in its 16- and 32-bit layout.
    row("fbld", &[Mem], 0, &[0xDF], Some(4), FIXED),
    row("fbstp", &[Mem], 0, &[0xDF], Some(6), FIXED),
    row("fldenv", &[Mem], 0, &[0xD9], Some(4), ENVIRONMENT),
    row("fnstenv", &[Mem], 0, &[0xD9], Some(6), ENVIRONMENT),
    row("frstor", &[Mem], 0, &[0xDD], Some(4), ENVIRONMENT),
    row("fnsave", &[Mem], 0, &[0xDD], Some(6), ENVIRONMENT),
    // --- x87: register forms and old names ---
    row("fst", &[St], 0, &[0xDD, 0xD0], None, FIXED),
    bytes("fcompi", 0, &[0xDF, 0xF1]),
    row("fcompi", &[St], 0, &[0xDF, 0xF0], None, FIXED),
    row("fcompi", &[St, ST0], 0, &[0xDF, 0xF0], None, FIXED),
    bytes("fucompi", 0, &[0xDF, 0xE9]),
    row("fucompi", &[St], 0, &[0xDF, 0xE8], None, FIXED),
    row("fucompi", &[St, ST0], 0, &[0xDF, 0xE8], None, FIXED),
    bytes("fneni", 0, &[0xDB, 0xE0]),
    bytes("fndisi", 0, &[0xDB, 0xE1]),
    bytes("fnsetpm", 0, &[0xDB, 0xE4]),
    bytes("frstpm", 0, &[0xDB, 0xE5]),
    bytes("feni", 0, &[0x9B, 0xDB, 0xE0]),
    bytes("fdisi", 0, &[0x9B, 0xDB, 0xE1]),
    bytes("fsetpm", 0, &[0x9B, 0xDB, 0xE4]),
];

/// The `reg` field of the ModR/M byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RegField<'a> {
    /// No ModR/M byte, or one whose `reg` is 0.
    None,
    Digit(u8),
    Register(&'a str),
}

/// The `r/m` operand of the ModR/M byte.
#[derive(Clone, Copy, Debug)]
pub(crate) enum RmField<'a> {
    Register(&'a str),
    Memory(&'a MemoryOperand),
}

/// What to emit for a statement a row matched, front to back.
#[derive(Debug)]
pub(crate) struct Plan<'a> {
    /// The address-size prefix `67`, asked for by a register that names an
    /// address (a memory operand asks for its own).
    pub addr_prefix: bool,
    /// The operand-size prefix `66`.
    pub data16: bool,
    /// The mandatory prefix, or 0.
    pub prefix: u8,
    pub rex_w: bool,
    pub opcode: &'static [u8],
    /// Or-ed into the last opcode byte.
    pub opcode_low: u8,
    pub reg: RegField<'a>,
    pub rm: Option<RmField<'a>>,
    /// Immediates in encoding order: byte count and value.
    pub imm: [Option<(u8, i64)>; 2],
    /// A branch target.
    pub target: Option<&'a str>,
    /// An operand width was spelled or implied (16-bit code inverts the
    /// meaning of `66` for exactly these).
    pub sized: bool,
    /// A register names an address, which 16-bit code cannot say this way.
    pub names_address: bool,
    /// A stack operation without a size: 32 bits wide under `.code16gcc`.
    pub stack_default: bool,
}

/// The size letter a general register implies.
fn register_letter(name: &str) -> Option<u8> {
    super::register_size_suffix(name).map(|c| c as u8)
}

fn register<'a>(op: &'a Operand) -> Option<&'a str> {
    match op {
        Operand::Register(reg) if reg.mask.is_none() && !reg.zeroing => Some(reg.name.as_str()),
        _ => None,
    }
}

fn memory(op: &Operand) -> Option<&MemoryOperand> {
    match op {
        Operand::Memory(mem) => Some(mem),
        _ => None,
    }
}

fn integer(op: &Operand) -> Option<i64> {
    match op {
        Operand::Immediate(ImmediateValue::Integer(value)) => Some(*value),
        _ => None,
    }
}

/// `%dx`, or exactly `(%dx)`: the port is implied by the opcode, so no other
/// address may pass for it.
fn is_port(op: &Operand) -> bool {
    use crate::backend::x86::assembler::parser::Displacement;
    match op {
        Operand::Register(reg) => reg.name == "dx",
        Operand::Memory(mem) => {
            mem.base.as_ref().is_some_and(|base| base.name == "dx")
                && mem.index.is_none()
                && mem.scale.is_none()
                && mem.segment.is_none()
                && matches!(
                    mem.displacement,
                    Displacement::None | Displacement::Integer(0)
                )
        }
        _ => false,
    }
}

/// Whether the table has a row called `name` (with or without a size
/// suffix): such a name is not a suffixed spelling of something else.
pub(crate) fn names(name: &str) -> bool {
    rows_named(name).next().is_some()
}

/// The rows `mnemonic` can mean, each with the size suffix it was spelled
/// with (`0` for none).
fn rows_named(mnemonic: &str) -> impl Iterator<Item = (&'static Row, u8)> + '_ {
    let (stem, suffix) = match mnemonic.as_bytes().split_last() {
        Some((&last, _)) if matches!(last, b'b' | b'w' | b'l' | b'q' | b's') => {
            (&mnemonic[..mnemonic.len() - 1], last)
        }
        _ => ("", 0),
    };
    ROWS.iter().filter_map(move |row| {
        if row.name == mnemonic {
            Some((row, 0))
        } else if suffix != 0
            && row.name == stem
            && row.sizes.iter().any(|&(letter, _)| letter == suffix)
        {
            Some((row, suffix))
        } else {
            None
        }
    })
}

/// Match `mnemonic ops` against the table.
///
/// `None` when the table has no row of that name; `Some(Err(..))`, with
/// GNU as's wording, when it has rows and none takes the operands.
/// `long` says the code is 64-bit.
pub(crate) fn plan<'a>(
    mnemonic: &str,
    ops: &'a [Operand],
    long: bool,
) -> Option<Result<Plan<'a>, String>> {
    let mut named: Option<&'static str> = None;
    for (row, suffix) in rows_named(mnemonic) {
        named = Some(row.name);
        match row.mode {
            Mode::Long if !long => continue,
            Mode::Legacy if long => continue,
            _ => {}
        }
        if let Some(plan) = row.matches(ops, suffix, long) {
            return Some(Ok(plan));
        }
    }
    named.map(|name| Err(format!("operand type mismatch for `{name}'")))
}

impl Row {
    fn matches<'a>(&self, ops: &'a [Operand], suffix: u8, long: bool) -> Option<Plan<'a>> {
        if ops.len() != self.args.len() {
            return None;
        }
        // The address size a register that names an address has to have,
        // and the one that takes the prefix.
        let (native, narrow) = if long { (b'q', b'l') } else { (b'l', b'w') };
        let mut plan = Plan {
            addr_prefix: false,
            data16: false,
            prefix: self.prefix,
            rex_w: false,
            opcode: self.opcode,
            opcode_low: 0,
            reg: match self.digit {
                Some(digit) => RegField::Digit(digit),
                None => RegField::None,
            },
            rm: None,
            imm: [None, None],
            target: None,
            sized: false,
            names_address: false,
            stack_default: false,
        };
        // The size letter the registers of the operand size imply.
        let mut implied: Option<u8> = None;
        // `lar`/`lsl`: a 16-bit selector register beside a wider
        // destination says nothing about the operand size.
        let selector = matches!(self.name, "lar" | "lsl");
        let mut sized = |letter: u8, source: bool| -> bool {
            if selector && source && letter == b'w' {
                return true;
            }
            match implied {
                Some(other) if other != letter => false,
                _ => {
                    implied = Some(letter);
                    true
                }
            }
        };
        let address = |plan: &mut Plan<'a>, letter: u8| -> bool {
            plan.names_address = true;
            plan.addr_prefix |= letter == narrow;
            letter == native || letter == narrow
        };
        let mut imm = 0usize;
        for (i, (arg, op)) in self.args.iter().zip(ops).enumerate() {
            let source = i + 1 < ops.len();
            match *arg {
                Reg => {
                    let name = register(op)?;
                    if !sized(register_letter(name)?, source) {
                        return None;
                    }
                    plan.reg = RegField::Register(name);
                }
                RegAddr => {
                    let name = register(op)?;
                    let letter = register_letter(name)?;
                    if !address(&mut plan, letter) {
                        return None;
                    }
                    // The registers of the memory operand say how wide an
                    // address is too, and have to say the same (`movdir64b
                    // (%rdx), %ecx` was `movdir64b (%rdx), %rcx`).
                    let another_width = |part: &Register| match part.name.as_str() {
                        "rip" => letter != b'q',
                        "eip" => letter != b'l',
                        part => register_letter(part).is_some_and(|wide| wide != letter),
                    };
                    if ops.iter().filter_map(memory).any(|mem| {
                        [&mem.base, &mem.index]
                            .into_iter()
                            .flatten()
                            .any(another_width)
                    }) {
                        return None;
                    }
                    // The prefix comes with the memory operand, whose
                    // registers have to be as wide: this one only repeats
                    // it. An address without registers leaves the word to
                    // this one; in 64-bit code that is the prefix and the
                    // same bytes, in 32-bit code a 16-bit address form the
                    // table does not write.
                    let absolute = ops.iter().any(|other| {
                        memory(other).is_some_and(|mem| mem.base.is_none() && mem.index.is_none())
                    });
                    if plan.addr_prefix && absolute && !long {
                        return None;
                    }
                    plan.addr_prefix &= absolute;
                    plan.reg = RegField::Register(name);
                }
                Rm => match op {
                    Operand::Memory(mem) => plan.rm = Some(RmField::Memory(mem)),
                    _ => {
                        let name = register(op)?;
                        if !sized(register_letter(name)?, source) {
                            return None;
                        }
                        plan.rm = Some(RmField::Register(name));
                    }
                },
                RmReg => {
                    let name = register(op)?;
                    if !sized(register_letter(name)?, source) {
                        return None;
                    }
                    plan.rm = Some(RmField::Register(name));
                }
                RmReg32 | RmReg64 => {
                    let name = register(op)?;
                    let want = if *arg == RmReg32 { b'l' } else { b'q' };
                    if register_letter(name)? != want {
                        return None;
                    }
                    plan.rm = Some(RmField::Register(name));
                }
                RmAddr => {
                    let name = register(op)?;
                    if !address(&mut plan, register_letter(name)?) {
                        return None;
                    }
                    plan.rm = Some(RmField::Register(name));
                }
                Mem => plan.rm = Some(RmField::Memory(memory(op)?)),
                Acc => {
                    let name = register(op)?;
                    if !matches!(name, "al" | "ax" | "eax" | "rax")
                        || !sized(register_letter(name)?, source)
                    {
                        return None;
                    }
                }
                AccAddr => {
                    let name = register(op)?;
                    if !matches!(name, "ax" | "eax" | "rax")
                        || !address(&mut plan, register_letter(name)?)
                    {
                        return None;
                    }
                }
                Implied(implied_name) => {
                    if register(op)? != implied_name {
                        return None;
                    }
                }
                Port => {
                    if !is_port(op) {
                        return None;
                    }
                }
                Segment => {
                    super::implied_address(self.name, memory(op)?).ok()?;
                }
                Imm8 | Imm8u | Imm8t | Imm16 => {
                    let value = integer(op)?;
                    let (bytes, fits) = match *arg {
                        Imm8 => (1, (-128..=255).contains(&value)),
                        Imm8u => (1, (0..=255).contains(&value)),
                        Imm8t => (1, true),
                        _ => (2, (-32768..=65535).contains(&value)),
                    };
                    if !fits {
                        return None;
                    }
                    *plan.imm.get_mut(imm)? = Some((bytes, value));
                    imm += 1;
                }
                Rel32 => match op {
                    Operand::Label(name) => plan.target = Some(name),
                    _ => return None,
                },
                St => {
                    let number = match register(op)? {
                        "st" => 0,
                        name => name
                            .strip_prefix("st(")?
                            .strip_suffix(')')?
                            .parse::<u8>()
                            .ok()
                            .filter(|&n| n < 8)?,
                    };
                    plan.opcode_low |= number;
                }
            }
        }
        // A suffix and the registers have to agree; one of them, or
        // neither, selects the size.
        let letter = match (suffix, implied) {
            (0, None) => b' ',
            (0, Some(letter)) => letter,
            (suffix, None) => suffix,
            (suffix, Some(letter)) if suffix == letter => letter,
            _ => return None,
        };
        if letter == b'q' && !long {
            return None;
        }
        let &(_, width) = self.sizes.iter().find(|&&(l, _)| l == letter)?;
        plan.data16 = width == Width::Data16;
        plan.rex_w = width == Width::RexW;
        plan.stack_default = self.stack && letter == b' ';
        // (A byte operation has no operand-size prefix in any mode.)
        plan.sized =
            !matches!(letter, b' ' | b'b') && self.sizes.iter().any(|&(_, w)| w == Width::Data16);
        if self.wide_bit && letter != b'b' {
            plan.opcode_low = 1;
        }
        Some(plan)
    }
}

impl super::InstructionEncoder {
    /// Emit what a row of the table matched.
    pub(crate) fn emit_plan(&mut self, plan: &Plan) -> Result<(), String> {
        let reg = match plan.reg {
            RegField::Register(name) => name,
            _ => "",
        };
        // (A register of APX makes the prefix below a REX2, which `encode`
        // folds the `0F` escape into, or refuses for a longer escape.)
        if plan.addr_prefix {
            self.bytes.push(0x67);
            // Under it an address is 32 bits wide and has no sign: a
            // symbol in one takes `R_X86_64_32` (`movdir64b sym, %ecx`).
            self.disp32_unsigned = true;
        }
        if plan.data16 {
            self.bytes.push(0x66);
        }
        if plan.prefix != 0 {
            self.bytes.push(plan.prefix);
        }
        let size = if plan.rex_w { 8 } else { 4 };
        match plan.rm {
            Some(RmField::Memory(mem)) => self.emit_rex_rm(size, reg, mem),
            Some(RmField::Register(rm)) if reg.is_empty() => self.emit_rex_unary(size, rm),
            Some(RmField::Register(rm)) => self.emit_rex_rr(size, reg, rm),
            None if plan.rex_w => self.bytes.push(0x48),
            None => {}
        }
        let (last, head) = plan
            .opcode
            .split_last()
            .ok_or("internal: a table row without an opcode")?;
        self.bytes.extend_from_slice(head);
        self.bytes.push(last | plan.opcode_low);
        let number =
            |name: &str| super::reg_num(name).ok_or_else(|| format!("bad register name `%{name}'"));
        let field = match plan.reg {
            RegField::None => 0,
            RegField::Digit(digit) => digit,
            RegField::Register(name) => number(name)?,
        };
        match plan.rm {
            Some(RmField::Register(rm)) => {
                let rm = number(rm)?;
                self.bytes.push(self.modrm(3, field, rm));
            }
            Some(RmField::Memory(mem)) => self.encode_modrm_mem(field, mem)?,
            None => {}
        }
        for &(bytes, value) in plan.imm.iter().flatten() {
            self.bytes
                .extend_from_slice(&value.to_le_bytes()[..usize::from(bytes)]);
        }
        if let Some(target) = plan.target {
            self.add_relocation(target, super::R_X86_64_PLT32, -4);
            self.bytes.extend_from_slice(&[0; 4]);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The table's scan takes the first row that matches, so two rows of
    /// one name must not take the same operands in different ways -- and a
    /// row must say something a reader can check: an opcode, sizes.
    #[test]
    fn rows_are_well_formed() {
        for row in ROWS {
            assert!(!row.opcode.is_empty(), "{}: no opcode", row.name);
            assert!(!row.sizes.is_empty(), "{}: no sizes", row.name);
            assert!(
                matches!(row.prefix, 0 | 0x66 | 0xF2 | 0xF3),
                "{}: prefix {:#x}",
                row.name,
                row.prefix
            );
            let modrm = row
                .args
                .iter()
                .filter(|a| matches!(a, Rm | RmReg | RmReg32 | RmReg64 | RmAddr | Mem));
            assert!(modrm.count() <= 1, "{}: two r/m operands", row.name);
            let reg = row
                .args
                .iter()
                .filter(|a| matches!(a, Reg | RegAddr))
                .count();
            assert!(
                reg <= 1 && !(reg == 1 && row.digit.is_some()),
                "{}: the reg field is taken twice",
                row.name
            );
            let letters: Vec<u8> = row.sizes.iter().map(|&(l, _)| l).collect();
            let mut unique = letters.clone();
            unique.sort_unstable();
            unique.dedup();
            assert_eq!(unique.len(), letters.len(), "{}: a size twice", row.name);
            // A suffixed spelling must not be another row's name.
            for &(letter, _) in row.sizes {
                if letter != b' ' {
                    let spelled = format!("{}{}", row.name, letter as char);
                    assert!(
                        !ROWS.iter().any(|other| other.name == spelled),
                        "`{spelled}` is both a row and a suffixed `{}`",
                        row.name
                    );
                }
            }
        }
    }

    #[test]
    fn a_name_the_table_lacks_is_not_its_business() {
        assert!(plan("mov", &[], true).is_none());
        assert!(plan("verrz", &[], true).is_none());
        assert!(names("verr") && names("verrw") && !names("verrb"));
    }
}
