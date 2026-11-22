//! The registers a statement names, held against the registers its
//! encoding can address.
//!
//! The encoders take a register's number and ask no more, and `%xmm3`,
//! `%mm3`, `%st(3)`, `%k3`, `%bl`, `%ebx`, `%ds` and `%cr3` are all number
//! three. So `rdrand %xmm0` was `rdrand %eax`, `adc %cl, %mm3` was `adc
//! %cl, %bl`, `addps %ecx, %xmm1` was `addps %xmm1, %xmm1` and `seta %fs`
//! was `seta %ah`: of the statements that put a register of a kind the
//! instruction does not take where it takes another, GNU as 2.47 refuses
//! all and these encoders assembled one in five into an instruction on
//! some other register.
//!
//! Which registers an instruction takes is written in its encoding. An
//! opcode of the one-byte map addresses general registers, `D8`..`DF` the
//! x87 stack, the packed-integer opcodes of the `0F` map `%mm` registers
//! without a mandatory prefix and `%xmm` registers with one, a VEX opcode
//! vector registers and -- a few of them -- a mask register or a 32- or
//! 64-bit general register. [`check`] reads the opcode the encoder wrote
//! and refuses a statement that
//!
//! * names a kind of register the opcode has no field for;
//! * names general registers of two widths, where the opcode is none of
//!   the few that take two (`movzx`, a shift by `%cl`, `in`, `crc32`);
//! * moves between a general register and a register of another kind
//!   (`movd`, `cvtsi2sd`, `pextrw`, `kmov`) and does not name one of each,
//!   or a memory operand in the place of one;
//! * compares into a mask register and does not name exactly one;
//! * has a register where the opcode exists with memory only (`movntdqa`);
//! * names vector registers of widths the opcode does not take together
//!   (`vpshufb %xmm1, %ymm2, %ymm3`, `vpermd %xmm1, %xmm1, %xmm1`): the
//!   combinations each VEX, EVEX and XOP opcode takes are a table, written
//!   from GNU as's own answers by `scripts/x86_vector_widths.py`;
//! * names its destination among its sources where the instruction faults
//!   on that (`vfmaddcph %xmm1, %xmm2, %xmm1`).
//!
//! It needs no list of mnemonics, and an instruction added to an encoder
//! is covered when it is added.
//!
//! What it cannot see is a general register of the wrong width where the
//! encoding takes the width from a mnemonic (`incsspd %rcx`), or a
//! register in the wrong place among others of its kind: that is each
//! encoder's to refuse. `scripts/gas_table_sweep.py` puts a register of
//! every kind in every position of every mnemonic of GNU as's table, and
//! vector registers of every width in every vector position, and records
//! the statements GNU as refuses and an encoder here assembles.

use super::vector_widths::{OPCODES, SHAPES};
use crate::backend::x86::assembler::parser::{Instruction, Operand};

const G8: u32 = 1 << 0;
const G16: u32 = 1 << 1;
const G32: u32 = 1 << 2;
const G64: u32 = 1 << 3;
const SEGMENT: u32 = 1 << 4;
const CONTROL: u32 = 1 << 5;
const DEBUG: u32 = 1 << 6;
const TEST: u32 = 1 << 7;
const X87: u32 = 1 << 8;
const MMX: u32 = 1 << 9;
const XMM: u32 = 1 << 10;
const YMM: u32 = 1 << 11;
const ZMM: u32 = 1 << 12;
const MASK: u32 = 1 << 13;
const BOUND: u32 = 1 << 14;
const TILE: u32 = 1 << 15;

/// Any general register.
const GENERAL: u32 = G8 | G16 | G32 | G64;
/// A general register a vector instruction can take: 32 or 64 bits wide.
const WIDE: u32 = G32 | G64;
/// What a VEX, EVEX or XOP vector instruction addresses.
const VECTOR: u32 = XMM | YMM | ZMM | TILE;
/// The registers that are neither general nor a tile: the other side of
/// a move or a conversion from or to a general register.
const FILES: u32 = XMM | YMM | ZMM | MMX | MASK;

/// The kind of the register `name`, or 0 for a name that is none here
/// (`%rip`, `%eiz`, a misspelling: not this check's to judge).
fn class_of(name: &str) -> u32 {
    let raw = name.as_bytes();
    let mut lower = [0u8; 8];
    let Some(buffer) = lower.get_mut(..raw.len()) else {
        return 0;
    };
    for (to, from) in buffer.iter_mut().zip(raw) {
        *to = from.to_ascii_lowercase();
    }
    let name: &[u8] = buffer;
    match name {
        b"al" | b"cl" | b"dl" | b"bl" | b"ah" | b"ch" | b"dh" | b"bh" | b"spl" | b"bpl"
        | b"sil" | b"dil" => G8,
        b"ax" | b"cx" | b"dx" | b"bx" | b"sp" | b"bp" | b"si" | b"di" => G16,
        b"eax" | b"ecx" | b"edx" | b"ebx" | b"esp" | b"ebp" | b"esi" | b"edi" => G32,
        b"rax" | b"rcx" | b"rdx" | b"rbx" | b"rsp" | b"rbp" | b"rsi" | b"rdi" => G64,
        b"es" | b"cs" | b"ss" | b"ds" | b"fs" | b"gs" => SEGMENT,
        b"st" => X87,
        [b's', b't', b'(', b'0'..=b'7', b')'] => X87,
        _ => numbered(name),
    }
}

/// The kind of a register that is a stem and a number: `%xmm17`, `%r9d`.
fn numbered(name: &[u8]) -> u32 {
    let digits = name
        .iter()
        .position(u8::is_ascii_digit)
        .unwrap_or(name.len());
    let (stem, rest) = name.split_at(digits);
    let end = rest
        .iter()
        .position(|b| !b.is_ascii_digit())
        .unwrap_or(rest.len());
    let (number, suffix) = rest.split_at(end);
    let Some(number) = std::str::from_utf8(number)
        .ok()
        .and_then(|n| n.parse::<u8>().ok())
    else {
        return 0;
    };
    match (stem, suffix) {
        (b"r", b"b") if (8..32).contains(&number) => G8,
        (b"r", b"w") if (8..32).contains(&number) => G16,
        (b"r", b"d") if (8..32).contains(&number) => G32,
        (b"r", b"") if (8..32).contains(&number) => G64,
        (b"xmm", b"") if number < 32 => XMM,
        (b"ymm", b"") if number < 32 => YMM,
        (b"zmm", b"") if number < 32 => ZMM,
        (b"mm", b"") if number < 8 => MMX,
        (b"k", b"") if number < 8 => MASK,
        (b"cr", b"") if number < 16 => CONTROL,
        (b"db" | b"dr", b"") if number < 16 => DEBUG,
        (b"tr", b"") if number < 8 => TEST,
        (b"bnd", b"") if number < 4 => BOUND,
        (b"tmm", b"") if number < 8 => TILE,
        _ => 0,
    }
}

/// What a statement names as operands. A register in an address is the
/// address's and is not counted.
#[derive(Default, Debug, PartialEq, Eq)]
struct Named {
    /// The kinds of its register operands.
    kinds: u32,
    /// How many of them are general registers.
    general: u32,
    /// How many are mask registers.
    masks: u32,
    /// How many are of a kind in [`FILES`].
    files: u32,
    /// How many of its operands are in memory.
    memory: u32,
    /// The widths of its vector register operands: a base-4 digit each
    /// (1 `%xmm`, 2 `%ymm`, 3 `%zmm`), the first operand's first.
    vectors: u32,
    /// One of its vector registers is numbered 16 or more: a register only
    /// an EVEX prefix has the bits for.
    high: bool,
}

/// The operands of a statement: its register operands, the register an
/// indirect branch goes through, and its memory operands.
fn named(operands: &[Operand]) -> Named {
    let mut found = Named::default();
    for operand in operands {
        let operand = match operand {
            Operand::Indirect(target) => target.as_ref(),
            other => other,
        };
        match operand {
            Operand::Register(register) => {
                let kind = class_of(&register.name);
                found.kinds |= kind;
                found.general += u32::from(kind & GENERAL != 0);
                found.masks += u32::from(kind == MASK);
                found.files += u32::from(kind & FILES != 0);
                if kind & (XMM | YMM | ZMM) != 0 {
                    let width = 1 + u32::from(kind != XMM) + u32::from(kind == ZMM);
                    found.vectors = found.vectors.saturating_mul(4).saturating_add(width);
                    found.high |= register
                        .name
                        .get(3..)
                        .and_then(|number| number.parse::<u8>().ok())
                        .is_some_and(|number| number >= 16);
                }
            }
            // A symbol alone is an address too.
            Operand::Memory(_) | Operand::Label(_) => found.memory += 1,
            _ => {}
        }
    }
    found
}

/// Whether a register operand is a `%mm`, vector, mask or bound register:
/// what GNU as calls a SIMD instruction's operands, in front of which it
/// takes no operand-size prefix word.
pub(crate) fn names_simd_register(operands: &[Operand]) -> bool {
    named(operands).kinds & (MMX | XMM | YMM | ZMM | MASK | BOUND) != 0
}

/// The code a statement is assembled for: how its bytes are read.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Code {
    /// `.code16` and `.code16gcc`: an operand is 16 bits wide, and 32
    /// under `66`.
    Bits16,
    /// 32-bit code: an operand is 32 bits wide, and 16 under `66`.
    Bits32,
    /// 64-bit code: as 32-bit code, and `40`..`4F` are REX prefixes; `C4`,
    /// `C5`, `62` and `D5` are always prefixes too.
    Bits64,
}

/// Where an opcode stands.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Space {
    /// The one-byte map.
    One,
    /// `0F`.
    Two,
    /// `0F 38`.
    Three38,
    /// `0F 3A`.
    Three3A,
    /// `0F 0F`: 3DNow!, the opcode behind the operands.
    Now,
    /// VEX or EVEX, and the map.
    Vex(u8),
    /// XOP, and the map.
    Xop(u8),
}

/// No `66`, `F3` or `F2` in front of an opcode.
const NO_PREFIX: u8 = 0;
const PREFIX_66: u8 = 1;
const PREFIX_F3: u8 = 2;
const PREFIX_F2: u8 = 3;

/// The opcode of one encoded instruction.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Encoded {
    /// Its map.
    space: Space,
    /// The opcode byte.
    opcode: u8,
    /// The `66`, `F3` or `F2` in front, as a VEX prefix numbers them (on a
    /// vector opcode, the mandatory prefix). Of several, `F3` and `F2` go
    /// before `66`.
    prefix: u8,
    /// EVEX-encoded.
    evex: bool,
    /// `REX.W`, or the `W` of a VEX, EVEX, XOP or REX2 prefix.
    wide: bool,
    /// A REX or a REX2 prefix stands in front of the opcode.
    extended: bool,
    /// An operand-size prefix `66` is among the prefixes, whatever else is
    /// (`popcnt %cx, %dx` is `66 F3 0F B8`); of an APX instruction, the
    /// `66` its EVEX prefix stands for.
    data16: bool,
    /// The byte behind the opcode: the ModR/M byte of an opcode that has
    /// one.
    modrm: u8,
}

impl Encoded {
    /// The `reg` field of the ModR/M byte: the opcode extension of a group.
    fn extension(&self) -> u8 {
        (self.modrm >> 3) & 7
    }
}

/// The opcode in `bytes`. `None` for bytes that stop short of one (a
/// prefix written as a statement).
fn opcode_of(bytes: &[u8], long: bool) -> Option<Encoded> {
    let mut at = 0;
    let mut prefix = NO_PREFIX;
    let mut data16 = false;
    loop {
        match *bytes.get(at)? {
            0x66 => {
                data16 = true;
                if prefix == NO_PREFIX {
                    prefix = PREFIX_66;
                }
            }
            0xF3 => prefix = PREFIX_F3,
            0xF2 => prefix = PREFIX_F2,
            0x26 | 0x2E | 0x36 | 0x3E | 0x64 | 0x65 | 0x67 | 0xF0 => {}
            // `fstsw` is `fwait` and `fnstsw`.
            0x9B if matches!(bytes.get(at + 1), Some(0xD8..=0xDF)) => {}
            _ => break,
        }
        at += 1;
    }
    let mut first = *bytes.get(at)?;
    let mut wide = false;
    let mut extended = false;
    if long && (0x40..=0x4F).contains(&first) {
        wide = first & 0x08 != 0;
        extended = true;
        at += 1;
        first = *bytes.get(at)?;
    }
    let second = bytes.get(at + 1).copied();
    // `W` is the top bit of the byte behind the map of a three-byte VEX, an
    // EVEX or an XOP prefix, and bit 3 of the payload of a REX2 one.
    let third_w = bytes.get(at + 2).is_some_and(|byte| byte & 0x80 != 0);
    // In 32-bit code `C4`, `C5` and `62` with a memory operand are `les`,
    // `lds` and `bound`; a VEX or EVEX prefix reads as a register form.
    let vex = long || second.is_some_and(|byte| byte >= 0xC0);
    // The map, where the opcode byte stands, and what the prefix says of
    // `66`, `F3` and `F2`.
    let (space, opcode_at, prefix, evex) = match first {
        // REX2: one payload byte, whose top bit selects the `0F` map.
        0xD5 if long => {
            let space = if second? & 0x80 != 0 {
                Space::Two
            } else {
                Space::One
            };
            wide = second? & 0x08 != 0;
            extended = true;
            (space, at + 2, prefix, false)
        }
        0x62 if vex => {
            wide = third_w;
            (
                Space::Vex(second? & 0x07),
                at + 4,
                *bytes.get(at + 2)? & 3,
                true,
            )
        }
        0xC4 if vex => {
            wide = third_w;
            (
                Space::Vex(second? & 0x1F),
                at + 3,
                *bytes.get(at + 2)? & 3,
                false,
            )
        }
        0xC5 if vex => (Space::Vex(1), at + 2, second? & 3, false),
        // `8F /0` is `pop`; XOP has a map of 8 or more where `pop` has
        // its ModR/M byte.
        0x8F if second.is_some_and(|byte| byte & 0x1F >= 8) => {
            wide = third_w;
            (
                Space::Xop(second? & 0x1F),
                at + 3,
                *bytes.get(at + 2)? & 3,
                false,
            )
        }
        0x0F => match second? {
            0x38 => (Space::Three38, at + 2, prefix, false),
            0x3A => (Space::Three3A, at + 2, prefix, false),
            0x0F => {
                return Some(Encoded {
                    space: Space::Now,
                    opcode: 0,
                    prefix,
                    evex: false,
                    wide,
                    extended,
                    data16,
                    modrm: 0,
                });
            }
            _ => (Space::Two, at + 1, prefix, false),
        },
        _ => (Space::One, at, prefix, false),
    };
    Some(Encoded {
        space,
        opcode: *bytes.get(opcode_at)?,
        prefix,
        evex,
        wide,
        extended,
        // A VEX, EVEX or XOP prefix has the `66` in its `pp` field.
        data16: match space {
            Space::Vex(_) | Space::Xop(_) => prefix == PREFIX_66,
            _ => data16,
        },
        modrm: bytes.get(opcode_at + 1).copied().unwrap_or(0),
    })
}

/// How an encoding stands to EVEX.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Evex {
    /// No EVEX prefix.
    None,
    /// An EVEX prefix of AVX-512 or AVX10, which every code has.
    Vector,
    /// An EVEX prefix APX alone has, and 64-bit code alone: its map 4, and
    /// the instructions on general and mask registers that AVX-512 left
    /// to VEX (`andn`, `cmpccxadd`, `kmovw`).
    Apx,
}

/// How the instruction in `bytes` stands to EVEX.
pub(crate) fn evex_of(bytes: &[u8], long: bool) -> Evex {
    match opcode_of(bytes, long) {
        Some(encoded) if encoded.evex => match (encoded.space, encoded.opcode) {
            (Space::Vex(4), _)
            | (Space::Vex(1), 0x90..=0x93)
            | (Space::Vex(2), 0xF2 | 0xF3 | 0xF5 | 0xF6 | 0xF7 | 0xE0..=0xEF)
            | (Space::Vex(3), 0xF0) => Evex::Apx,
            _ => Evex::Vector,
        },
        _ => Evex::None,
    }
}

/// The width of the general registers an opcode takes, where every one of
/// them is as wide as the operation: the integer instructions, `cmovcc`
/// and `cfcmovcc`, the scalar conversions from and to a general register,
/// `movd` and `movq`, BMI and `cmpccxadd`. The encoders of these take a
/// register by its number and the width from a suffix or from another
/// operand: `cmovaq (%rsp), %r9w` was `cmovaq (%rsp), %r9`, `cvttss2siq
/// %xmm1, %ecx` converted into `%rcx`, `rorxq $3, %ecx, %ecx` rotated
/// `%rcx`, and in 32-bit code `addw $-3, %ecx` added to `%cx` and `movb
/// $0x90, %cx` wrote `%cl`.
///
/// A shift's count is `%cl` whatever is shifted ([`shifts_by_count`]), and
/// the width a prefix written as a word gives ([`sized_by_word`]) is not
/// the registers'.
fn general_width(encoded: &Encoded, code: Code) -> Option<u32> {
    let Encoded {
        space,
        opcode,
        prefix,
        wide,
        data16,
        ..
    } = *encoded;
    // 32 bits, or 64 under `W`.
    let by_w = if wide { G64 } else { G32 };
    // The operand size: 64 bits under `W`; else `66` selects the one of 16
    // and 32 bits that is not the code's own.
    let by_size = if wide {
        G64
    } else if data16 != (code == Code::Bits16) {
        G16
    } else {
        G32
    };
    // What is pushed, popped and branched through is as wide as the stack
    // in 64-bit code, whatever `W` says.
    let by_stack = match code {
        Code::Bits64 if !data16 => G64,
        _ => by_size,
    };
    // The low bit of an opcode of the first instructions selects the
    // operand size: clear for bytes.
    let by_low_bit = if opcode & 1 == 0 { G8 } else { by_size };
    let scalar = prefix >= PREFIX_F3;
    let extension = encoded.extension();
    Some(match (space, opcode) {
        // `add`, `or`, `adc`, `sbb`, `and`, `sub`, `xor`, `cmp`: from and
        // to a register, and a number to the accumulator.
        (Space::One, 0x00..=0x3F) if opcode & 7 < 6 => by_low_bit,
        // The same with a number, `test`, `xchg`, `mov`; `mov` of a
        // number and of the accumulator from and to an address; `test` of
        // a number; the shifts; `not`, `neg`, `mul`, `div` and their kin.
        (
            Space::One,
            0x80..=0x8B
            | 0xA0..=0xA3
            | 0xA8
            | 0xA9
            | 0xC0
            | 0xC1
            | 0xC6
            | 0xC7
            | 0xD0..=0xD3
            | 0xF6
            | 0xF7
            | 0xFE,
        ) => by_low_bit,
        // `inc` and `dec`; `call`, `jmp` and `push` through a register.
        (Space::One, 0xFF) => match extension {
            0 | 1 => by_size,
            2 | 4 | 6 => by_stack,
            _ => return None,
        },
        // `inc` and `dec` of a register, where these are no REX prefixes.
        (Space::One, 0x40..=0x4F) => by_size,
        (Space::One, 0x50..=0x5F) => by_stack,
        (Space::One, 0x8F) if extension == 0 => by_stack,
        // The accumulator exchanged with itself is `nop`, and `xchg %rax,
        // %rax` is written as that: `90` and nothing in front. (`41 90` is
        // `xchg %r8d, %eax`.)
        (Space::One, 0x90) if code == Code::Bits64 && !wide && !data16 && !encoded.extended => {
            G32 | G64
        }
        // `imul` by a number, `lea`, `xchg` with the accumulator.
        (Space::One, 0x69 | 0x6B | 0x8D | 0x90..=0x97) => by_size,
        (Space::One, 0xB0..=0xB7) => G8,
        // `mov` of a number into a register. Without `W` it clears the
        // upper half of the 64-bit one, which is how the encoder writes
        // `movq $0xffffffff, %rax`.
        (Space::One, 0xB8..=0xBF) if code == Code::Bits64 && !wide && !data16 => G32 | G64,
        (Space::One, 0xB8..=0xBF) => by_size,
        (Space::Two, 0x40..=0x4F) => by_size,
        // The bit tests, the double shifts, `imul`, `popcnt`, the bit
        // scans and the counts in their place under `F3`.
        (
            Space::Two,
            0xA3 | 0xA4 | 0xA5 | 0xAB | 0xAC | 0xAD | 0xAF | 0xB3 | 0xB8 | 0xBA..=0xBD,
        ) => by_size,
        // `cmpxchg` and `xadd`.
        (Space::Two, 0xB0 | 0xB1 | 0xC0 | 0xC1) => by_low_bit,
        // `movbe`; under `F2` the opcodes are `crc32`, which takes a
        // register of each of two widths.
        (Space::Three38, 0xF0 | 0xF1) if prefix != PREFIX_F2 => by_size,
        // APX: under `F2` these are `setcc`, which has its own registers.
        (Space::Vex(4), 0x40..=0x4F) if prefix != PREFIX_F2 => by_size,
        // The rest of APX's map: the first instructions keep their
        // opcodes behind the EVEX prefix, and their sizes with them.
        // (`{evex} cmpb %ebp, (%rdx)` and `{evex} testw $5, %al` were
        // assembled.)
        (Space::Vex(4), 0x00..=0x3F) if opcode & 7 < 4 => by_low_bit,
        (
            Space::Vex(4),
            0x80..=0x85 | 0x8A | 0x8B | 0xC0 | 0xC1 | 0xD0..=0xD3 | 0xF6 | 0xF7 | 0xFE,
        ) => by_low_bit,
        (Space::Vex(4), 0xFF) if extension < 2 => by_size,
        // The double shifts, `imul`, `movbe`, `popcnt`, `tzcnt`, `lzcnt`.
        (
            Space::Vex(4),
            0x24 | 0x2C | 0xA5 | 0xAD | 0x69 | 0x6B | 0xAF | 0x60 | 0x61 | 0x88 | 0xF4 | 0xF5,
        ) => by_size,
        (Space::Two, 0x2A | 0x2C | 0x2D) if scalar => by_w,
        (Space::Vex(1 | 5), 0x2A | 0x2C | 0x2D | 0x78 | 0x79 | 0x7B) if scalar => by_w,
        (Space::Two | Space::Vex(1), 0x6E | 0x7E) if prefix != PREFIX_F3 => by_w,
        // `pextrd` and `pinsrd`, and `pextrq` and `pinsrq` under `W`.
        (Space::Three3A | Space::Vex(3), 0x16 | 0x22) => by_w,
        // APX's `invept`, `invvpid` and `invpcid`; `wrss` and `wruss`.
        (Space::Vex(4), 0xF0..=0xF2) if prefix == PREFIX_F3 => G64,
        (Space::Vex(4), 0x66) if prefix == NO_PREFIX => by_w,
        (Space::Vex(4), 0x65) if prefix == PREFIX_66 => by_w,
        (Space::Vex(2), 0xF2 | 0xF3 | 0xF5 | 0xF6 | 0xF7 | 0xE0..=0xEF) | (Space::Vex(3), 0xF0) => {
            by_w
        }
        _ => return None,
    })
}

/// The kinds of register a memory form of the opcode names, where the
/// opcode moves between a general register and a vector register and one
/// of the two can be memory: the other kind alone.
///
/// The encoders take a register by its number. `vcvtsi2sd (%rbx), %ecx,
/// %xmm2` named a general register where the first source stands and was
/// assembled as `vcvtsi2sdl (%rbx), %xmm1, %xmm2`; `vcvtsd2sil (%rbx),
/// %xmm1` converted into `%ecx`; in 32-bit code `pextrw $5, %ecx, (%ebx)`
/// extracted from `%xmm1`.
fn beside_memory(encoded: &Encoded) -> Option<u32> {
    let Encoded {
        space,
        opcode,
        prefix,
        ..
    } = *encoded;
    let scalar = prefix >= PREFIX_F3;
    Some(match (space, opcode) {
        // A conversion from a general register or memory.
        (Space::Two | Space::Vex(1 | 5), 0x2A) | (Space::Vex(1 | 5), 0x7B) if scalar => XMM,
        // A conversion into a general register.
        (Space::Two | Space::Vex(1 | 5), 0x2C | 0x2D) | (Space::Vex(1 | 5), 0x78 | 0x79)
            if scalar =>
        {
            WIDE
        }
        // `movd` and `movq` from and to a general register or memory.
        (Space::Two | Space::Vex(1), 0x6E | 0x7E) if prefix != PREFIX_F3 => MMX | XMM,
        // `pinsrw`; `pextrb`, `pextrw`, `pextrd` and `extractps` to
        // memory; `pinsrb` and `pinsrd`.
        (Space::Two | Space::Vex(1), 0xC4) => MMX | XMM,
        (Space::Three3A | Space::Vex(3), 0x14..=0x17 | 0x20 | 0x22) => XMM,
        _ => return None,
    })
}

/// The first of `%ah`, `%ch`, `%dh` and `%bh` among the operands.
fn high_byte(operands: &[Operand]) -> Option<&str> {
    operands.iter().find_map(|operand| match operand {
        Operand::Register(register)
            if matches!(register.name.as_str(), "ah" | "ch" | "dh" | "bh") =>
        {
            Some(register.name.as_str())
        }
        _ => None,
    })
}

/// Whether the opcode shifts by a count in `%cl`: the shifts and the
/// double shifts. The count is the statement's first operand, and is not
/// as wide as what is shifted.
fn shifts_by_count(encoded: &Encoded) -> bool {
    matches!(
        (encoded.space, encoded.opcode),
        (Space::One, 0xD2 | 0xD3)
            | (Space::Two, 0xA5 | 0xAD)
            | (Space::Vex(4), 0xD2 | 0xD3 | 0xA5 | 0xAD)
    )
}

/// The operands of a shift without its count.
fn without_count(operands: &[Operand]) -> &[Operand] {
    match operands {
        [Operand::Register(count), shifted @ ..] if !shifted.is_empty() && count.name == "cl" => {
            shifted
        }
        _ => operands,
    }
}

/// Whether a prefix written as a word in front of the statement sets its
/// operand size: `data16 addl %eax, %ebx` is `66 01 C3`, and `rex64 cmova
/// %eax, %ebx` is `48 0F 47 D8`. The registers of such a statement are as
/// wide as the statement without the word.
fn sized_by_word(instr: &Instruction) -> bool {
    instr
        .prefixes
        .iter()
        .any(|word| matches!(word.as_str(), "data16" | "data32" | "rex64"))
}

/// The kinds of register the opcode can address, in 64-bit code (`long`)
/// or 16- and 32-bit code.
fn addressable(encoded: &Encoded, long: bool) -> u32 {
    let Encoded {
        space,
        opcode,
        prefix,
        evex,
        ..
    } = *encoded;
    // A packed-integer opcode: `%mm` registers, or `%xmm` registers under
    // a mandatory prefix.
    let packed = if prefix == NO_PREFIX { MMX } else { XMM };
    // A register as wide as an address.
    let native = if long { G64 } else { G32 };
    // A scalar conversion from or to a general register is under `F3` or
    // `F2`; the packed one in its place is not.
    let scalar = prefix >= PREFIX_F3;
    match space {
        Space::One => match opcode {
            0xD8..=0xDE => X87,
            // `fnstsw %ax`.
            0xDF if encoded.modrm == 0xE0 => G16,
            0xDF => X87,
            // `mov` to and from a segment register; `push` and `pop` of
            // one.
            0x8C | 0x8E | 0x06 | 0x07 | 0x0E | 0x16 | 0x17 | 0x1E | 0x1F => GENERAL | SEGMENT,
            // `push` and `pop` of a general register.
            0x50..=0x5F => G16 | native,
            // `lea` has no byte form.
            0x8D => G16 | WIDE,
            _ => GENERAL,
        },
        Space::Two => match opcode {
            // Packed and scalar floating point.
            0x10..=0x17 | 0x28 | 0x29 | 0x2B | 0x2E | 0x2F | 0x51..=0x5F | 0xC2 | 0xC6 => XMM,
            // `movmskps`.
            0x50 => XMM | WIDE,
            // The conversions between `%xmm` and a general register, and
            // between `%xmm` and `%mm`.
            0x2A | 0x2C | 0x2D if scalar => XMM | WIDE,
            0x2A | 0x2C | 0x2D => XMM | MMX,
            // `movq2dq`, `movdq2q`.
            0xD6 => XMM | MMX,
            // `movq` between `%xmm` registers.
            0x7E if prefix == PREFIX_F3 => XMM,
            // `movd`, `pinsrw`, `pextrw`, `pmovmskb`.
            0x6E | 0x7E | 0xC4 | 0xC5 | 0xD7 => packed | WIDE,
            // `extrq` and `insertq` under a prefix, `vmread` and `vmwrite`
            // without.
            0x78 | 0x79 if prefix != NO_PREFIX => XMM,
            0x78 | 0x79 => native,
            0x60..=0x7F | 0xD0..=0xFE => packed,
            // `lar`, `lsl` and the long `nop` have no byte form.
            0x02 | 0x03 | 0x1F => G16 | WIDE,
            // `setcc` has no other.
            0x90..=0x9F => G8,
            // `movnti`, `bswap`.
            0xC3 | 0xC8..=0xCF => WIDE,
            0x1A | 0x1B => GENERAL | BOUND,
            0x20 | 0x22 => GENERAL | CONTROL,
            0x21 | 0x23 => GENERAL | DEBUG,
            0x24 | 0x26 => GENERAL | TEST,
            // `push` and `pop` of `%fs` and `%gs`.
            0xA0 | 0xA1 | 0xA8 | 0xA9 => GENERAL | SEGMENT,
            // `rdsspd`, `rdsspq`.
            0x1E if prefix == PREFIX_F3 => WIDE,
            // `umonitor` takes an address in a register of any size; the
            // others under a prefix -- `rdfsbase` and its kin, `ptwrite`,
            // `incssp`, `tpause`, `umwait` -- 32 or 64 bits.
            0xAE if prefix == PREFIX_F3 && encoded.extension() == 6 => G16 | WIDE,
            0xAE if prefix != NO_PREFIX => WIDE,
            // `rdpid` and `senduipi`; `rdrand` and `rdseed`.
            0xC7 if prefix == PREFIX_F3 => native,
            0xC7 => G16 | WIDE,
            _ => GENERAL,
        },
        Space::Three38 => match opcode {
            // `invept`, `invvpid`, `invpcid`.
            0x80..=0x82 => native,
            // `movrs`; `movbe`, `crc32`, `adcx`, `wrss`, `movdiri`, the
            // Key Locker handles, `aadd`.
            0x8A | 0x8B | 0xF0..=0xFF => GENERAL,
            0x00..=0x0B | 0x1C..=0x1E => packed,
            _ => XMM,
        },
        Space::Three3A => match opcode {
            0x0F => packed,
            // `pextrb`, `pextrw`, `pextrd`, `extractps`; `pinsrb`,
            // `pinsrd`.
            0x14..=0x17 | 0x20 | 0x22 => XMM | WIDE,
            _ => XMM,
        },
        Space::Now => MMX,
        Space::Vex(map) => match (map, opcode) {
            // `kmov` between a mask and a general register.
            (1, 0x92 | 0x93) => MASK | WIDE,
            // The mask instructions.
            (1, 0x41 | 0x42 | 0x44..=0x47 | 0x4A | 0x4B | 0x90 | 0x91 | 0x98 | 0x99) => MASK,
            // The compares: into a mask where EVEX-encoded.
            (1, 0xC2 | 0x64..=0x66 | 0x74..=0x76) | (2, 0x29 | 0x37) if evex => VECTOR | MASK,
            // `vmovq`, `vmovd` and `vmovw` between `%xmm` registers.
            (1 | 5, 0x6E | 0x7E) if prefix == PREFIX_F3 => XMM,
            // `vmovd`, `vmovw`; `vpinsrw`, `vpextrw`.
            (1 | 5, 0x6E | 0x7E) | (1, 0xC4 | 0xC5) => XMM | WIDE,
            // `vmovmskps`, `vpmovmskb`.
            (1, 0x50 | 0xD7) => XMM | YMM | WIDE,
            // The scalar conversions from and to a general register.
            (1 | 5, 0x2A | 0x2C | 0x2D | 0x78 | 0x79 | 0x7B) | (5, 0x6C | 0x6D) if scalar => {
                XMM | WIDE
            }
            // BMI; `cmpccxadd`.
            (2, 0xF2 | 0xF3 | 0xF5 | 0xF6 | 0xF7 | 0xE0..=0xEF) => WIDE,
            // `vpbroadcast` from a general register.
            (2, 0x7A..=0x7C) => VECTOR | WIDE,
            // `vptestm`, `vptestnm`, `vpshufbitqmb`.
            (2, 0x26 | 0x27 | 0x8F) => VECTOR | MASK,
            // The moves between a mask and a vector.
            (2, 0x28..=0x2A | 0x38..=0x3A) if prefix == PREFIX_F3 => VECTOR | MASK,
            // `vp2intersect`.
            (2, 0x68) if prefix == PREFIX_F2 => VECTOR | MASK,
            // `vpextr`, `vextractps`, `vpinsr`.
            (3, 0x14..=0x17 | 0x20 | 0x22) => XMM | WIDE,
            // The mask shifts.
            (3, 0x30..=0x33) => MASK,
            // `vpcmp`, `vfpclass`, `vcmpph`.
            (3, 0x1E | 0x1F | 0x3E | 0x3F | 0x66 | 0x67 | 0xC2) => VECTOR | MASK,
            // `rorx`.
            (3, 0xF0) => WIDE,
            // APX. SHA and Key Locker, whose handle instructions take
            // general registers.
            (4, 0xD4 | 0xD8..=0xDF) => XMM | GENERAL,
            // `setcc` under `F2`: a byte register, or -- zeroing the rest
            // of it -- a 32- or 64-bit one. `cfcmovcc` and `cmovcc`.
            (4, 0x40..=0x4F) if prefix == PREFIX_F2 => G8 | WIDE,
            (4, 0x40..=0x4F) => G16 | WIDE,
            // The promoted legacy instructions.
            (4, _) => GENERAL,
            // `urdmsr`, `uwrmsr`, `rdmsr` and `wrmsrns` with an immediate.
            (7, _) => G64,
            _ => VECTOR,
        },
        Space::Xop(_) => match opcode {
            // TBM and LWP.
            0x01 | 0x02 | 0x10 | 0x12 => WIDE,
            _ => XMM | YMM,
        },
    }
}

/// Whether the opcode takes general registers of two widths.
fn mixes_widths(encoded: &Encoded) -> bool {
    match encoded.space {
        // `movsxd`; a shift by `%cl`; `in` and `out` through `%dx`.
        Space::One => matches!(
            encoded.opcode,
            0x63 | 0xD2 | 0xD3 | 0xE4..=0xE7 | 0xEC..=0xEF
        ),
        // The system instructions that name their implied registers
        // (`invlpga %rax, %ecx`); `lar`, `lsl`; `shld` and `shrd` by
        // `%cl`; `movzx`, `movsx`.
        Space::Two => matches!(
            encoded.opcode,
            0x01 | 0x02 | 0x03 | 0xA5 | 0xAD | 0xB6 | 0xB7 | 0xBE | 0xBF
        ),
        // `crc32`.
        Space::Three38 => matches!(encoded.opcode, 0xF0 | 0xF1),
        // APX: the shifts and `crc32` again.
        Space::Vex(4) => matches!(encoded.opcode, 0xD2 | 0xD3 | 0xA5 | 0xAD | 0xF0 | 0xF1),
        // `lwpins`, `lwpval`: a 32-bit value, a register of either width.
        Space::Xop(_) => encoded.opcode == 0x12,
        _ => false,
    }
}

/// Whether the opcode exists with a memory operand only: the
/// non-temporal moves, whose helpers are those of the moves that take a
/// register too (`vmovntdqa %xmm1, %xmm1` was assembled).
fn memory_only(encoded: &Encoded) -> bool {
    let Encoded { opcode, prefix, .. } = *encoded;
    match encoded.space {
        Space::Two | Space::Vex(1) => match opcode {
            // `movntps` and its kin, `movnti`, `movntq` and `movntdq`; the
            // stores of `movlps` and `movhps`.
            0x13 | 0x17 | 0x2B | 0xC3 | 0xE7 => true,
            // `movlpd` and `movhpd`: between registers these opcodes are
            // `movhlps` and `movlhps`, and under `F3` and `F2` `movsldup`,
            // `movshdup` and `movddup`.
            0x12 | 0x16 => prefix == PREFIX_66,
            // `lddqu`.
            0xF0 => prefix == PREFIX_F2,
            _ => false,
        },
        // `movntdqa`; under `F3` the opcode is `vpbroadcastmb2q`.
        Space::Three38 | Space::Vex(2) => opcode == 0x2A && prefix == PREFIX_66,
        _ => false,
    }
}

/// Whether the statement names as many registers of each side as an
/// opcode that addresses the kinds `takes` has fields for.
fn counts_fit(found: &Named, takes: u32) -> bool {
    // A compare into a mask, a move between a mask and a vector.
    if takes & MASK != 0 && takes & VECTOR != 0 && found.masks != 1 {
        return false;
    }
    // A move or a conversion between a general register and a register
    // of another kind: one of each, or memory in the place of one.
    if takes & GENERAL == WIDE && takes & FILES != 0 {
        return found.general <= 1
            && found.general + found.memory >= 1
            && found.files + found.memory >= 1;
    }
    true
}

/// Whether the opcode takes vector registers of the widths `vectors`
/// (as [`Named::vectors`] has them) together. A statement is held against
/// the statements of its opcode that name as many vector registers: an
/// opcode the table has none of is not judged.
fn widths_fit(encoded: &Encoded, vectors: u32) -> bool {
    let (space, map) = match encoded.space {
        Space::Vex(map) => (0, map),
        Space::Xop(map) => (1, map),
        // Outside VEX every vector register is an `%xmm`.
        _ => return true,
    };
    let key = space << 24
        | u32::from(map) << 16
        | u32::from(encoded.opcode) << 8
        | u32::from(encoded.prefix);
    let Ok(at) = OPCODES.binary_search_by_key(&key, |&(opcode, _)| opcode) else {
        return true;
    };
    let Some(shape) = OPCODES
        .get(at)
        .and_then(|&(_, shape)| SHAPES.get(usize::from(shape)))
    else {
        return true;
    };
    // A base-4 digit is two bits, and the first is not 0.
    let registers = |code: u32| (u32::BITS - code.leading_zeros()).div_ceil(2);
    u8::try_from(vectors).is_ok_and(|code| shape.contains(&code))
        || shape
            .iter()
            .all(|&known| registers(u32::from(known)) != registers(vectors))
}

/// Whether the opcode's destination has to be a register none of its
/// sources is: the complex multiplications of AVX512-FP16 (`vfmaddcph`,
/// `vfcmulcsh` and their kin), which fault otherwise.
fn destination_apart(encoded: &Encoded) -> bool {
    encoded.space == Space::Vex(6)
        && matches!(encoded.opcode, 0x56 | 0x57 | 0xD6 | 0xD7)
        && encoded.prefix >= PREFIX_F3
}

/// Whether the last register operand -- the destination -- is also one of
/// the others.
fn destination_among_sources(operands: &[Operand]) -> bool {
    let mut registers = operands.iter().rev().filter_map(|operand| match operand {
        Operand::Register(register) => Some(register.name.as_str()),
        _ => None,
    });
    registers
        .next()
        .is_some_and(|destination| registers.any(|source| source == destination))
}

/// Refuse a statement whose registers its encoding `bytes` cannot
/// address, or addresses at another width than they have.
pub(crate) fn check(instr: &Instruction, bytes: &[u8], code: Code) -> Result<(), String> {
    let found = named(&instr.operands);
    if found.kinds == 0 {
        return Ok(());
    }
    let long = code == Code::Bits64;
    let Some(encoded) = opcode_of(bytes, long) else {
        return Ok(());
    };
    let takes = addressable(&encoded, long);
    // The AMX instructions take tiles, and some a row number or a vector
    // beside them.
    let fits = if found.kinds & TILE != 0 {
        found.kinds & !(takes | WIDE) == 0
    } else {
        found.kinds & !takes == 0
            && counts_fit(&found, takes)
            && !(memory_only(&encoded) && encoded.modrm >= 0xC0)
    };
    // `%xmm16` and up are EVEX's: a VEX or XOP opcode has no bit for them
    // (`vfrczpd %xmm17, %xmm17` was `vfrczpd %xmm1, %xmm1`).
    let unreachable = found.high && !encoded.evex;
    if !fits || unreachable {
        return Err(format!(
            "operand type mismatch for `{}'",
            instr.mnemonic.to_ascii_lowercase()
        ));
    }
    // Behind a REX or a REX2 prefix `%ah` to `%bh` are `%spl` to `%dil`:
    // `movsbq %ah, %rdx` was assembled as `movsbq %spl, %rdx`, and `movzbl
    // %ch, %r17d` as `movzbl %bpl, %r17d`.
    if encoded.extended {
        if let Some(name) = high_byte(&instr.operands) {
            return Err(format!(
                "can't encode register '%{name}' in an instruction requiring REX/REX2 prefix"
            ));
        }
    }
    if found.memory != 0 && encoded.modrm < 0xC0 {
        if let Some(kinds) = beside_memory(&encoded) {
            if found.kinds & !kinds != 0 {
                return Err(format!(
                    "operand type mismatch for `{}'",
                    instr.mnemonic.to_ascii_lowercase()
                ));
            }
        }
    }
    if found.kinds & GENERAL != 0 && !sized_by_word(instr) {
        if let Some(width) = general_width(&encoded, code) {
            let sized = if shifts_by_count(&encoded) {
                named(without_count(&instr.operands)).kinds
            } else {
                found.kinds
            };
            if sized & GENERAL & !width != 0 {
                return Err(format!(
                    "operand size mismatch for `{}'",
                    instr.mnemonic.to_ascii_lowercase()
                ));
            }
        }
    }
    if ((found.kinds & GENERAL).count_ones() > 1 && !mixes_widths(&encoded))
        || (found.vectors != 0 && !widths_fit(&encoded, found.vectors))
    {
        return Err(format!(
            "operand size mismatch for `{}'",
            instr.mnemonic.to_ascii_lowercase()
        ));
    }
    if destination_apart(&encoded) && destination_among_sources(&instr.operands) {
        return Err(format!(
            "destination and source registers must be distinct for `{}'",
            instr.mnemonic.to_ascii_lowercase()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::x86::assembler::parser::Register;

    #[test]
    fn every_register_name_has_its_kind() {
        for (name, kind) in [
            ("al", G8),
            ("dh", G8),
            ("sil", G8),
            ("r9b", G8),
            ("r31b", G8),
            ("cx", G16),
            ("r15w", G16),
            ("ebp", G32),
            ("r8d", G32),
            ("rsp", G64),
            ("r8", G64),
            ("r31", G64),
            ("fs", SEGMENT),
            ("cr8", CONTROL),
            ("db7", DEBUG),
            ("dr3", DEBUG),
            ("tr3", TEST),
            ("st", X87),
            ("st(7)", X87),
            ("mm7", MMX),
            ("xmm31", XMM),
            ("ymm0", YMM),
            ("zmm19", ZMM),
            ("k7", MASK),
            ("K1", MASK),
            ("XMM3", XMM),
            ("bnd3", BOUND),
            ("tmm7", TILE),
        ] {
            assert_eq!(class_of(name), kind, "{name}");
        }
        // Names that are no operand register: an address's, or none.
        for name in [
            "rip", "eiz", "riz", "r7", "r32", "mm8", "xmm32", "k8", "st(8)", "foo", "",
        ] {
            assert_eq!(class_of(name), 0, "{name}");
        }
    }

    /// The opcode of `bytes` as (map, opcode byte, prefix, EVEX).
    fn found(bytes: &[u8], long: bool) -> Option<(Space, u8, u8, bool)> {
        opcode_of(bytes, long).map(|e| (e.space, e.opcode, e.prefix, e.evex))
    }

    #[test]
    fn the_opcode_is_found_behind_every_prefix() {
        let cases: [(&[u8], bool, (Space, u8, u8, bool)); 21] = [
            (&[0x01, 0xC8], true, (Space::One, 0x01, NO_PREFIX, false)),
            (
                &[0x66, 0x01, 0xC8],
                true,
                (Space::One, 0x01, PREFIX_66, false),
            ),
            (
                &[0x48, 0x01, 0xC8],
                true,
                (Space::One, 0x01, NO_PREFIX, false),
            ),
            // 32-bit code: `inc %eax`, not a REX prefix.
            (&[0x40], false, (Space::One, 0x40, NO_PREFIX, false)),
            (
                &[0x65, 0x67, 0x66, 0xF0, 0x01, 0x03],
                false,
                (Space::One, 0x01, PREFIX_66, false),
            ),
            (
                &[0x9B, 0xDF, 0xE0],
                true,
                (Space::One, 0xDF, NO_PREFIX, false),
            ),
            (&[0x9B], true, (Space::One, 0x9B, NO_PREFIX, false)),
            (
                &[0x66, 0x48, 0x0F, 0x6E, 0xC0],
                true,
                (Space::Two, 0x6E, PREFIX_66, false),
            ),
            // `F3` and `F2` go before `66`, wherever they stand.
            (
                &[0x66, 0xF2, 0x0F, 0x38, 0xF1, 0xC1],
                true,
                (Space::Three38, 0xF1, PREFIX_F2, false),
            ),
            (
                &[0xF3, 0x66, 0x0F, 0xB8, 0xC1],
                true,
                (Space::Two, 0xB8, PREFIX_F3, false),
            ),
            (
                &[0x66, 0x0F, 0x38, 0xF6, 0xC1],
                true,
                (Space::Three38, 0xF6, PREFIX_66, false),
            ),
            (
                &[0x0F, 0x0F, 0xC1, 0x9E],
                true,
                (Space::Now, 0, NO_PREFIX, false),
            ),
            // VEX and EVEX: the prefix is a field.
            (
                &[0xC5, 0xF9, 0x58, 0xC1],
                true,
                (Space::Vex(1), 0x58, PREFIX_66, false),
            ),
            (
                &[0xC5, 0xFA, 0x7E, 0xC1],
                true,
                (Space::Vex(1), 0x7E, PREFIX_F3, false),
            ),
            (
                &[0xC4, 0xE2, 0x70, 0xF2, 0xC2],
                true,
                (Space::Vex(2), 0xF2, NO_PREFIX, false),
            ),
            (
                &[0xC4, 0xE2, 0x73, 0xF6, 0xC9],
                true,
                (Space::Vex(2), 0xF6, PREFIX_F2, false),
            ),
            (
                &[0x62, 0xF4, 0x74, 0x18, 0x10, 0xC9],
                true,
                (Space::Vex(4), 0x10, NO_PREFIX, true),
            ),
            (
                &[0x62, 0xF4, 0x7F, 0x18, 0x47, 0xC1],
                true,
                (Space::Vex(4), 0x47, PREFIX_F2, true),
            ),
            // 32-bit code: `les (%eax), %ecx` and `pop (%eax)`.
            (&[0xC4, 0x08], false, (Space::One, 0xC4, NO_PREFIX, false)),
            (&[0x8F, 0x00], false, (Space::One, 0x8F, NO_PREFIX, false)),
            (
                &[0x8F, 0xE9, 0x78, 0x01, 0xC8],
                true,
                (Space::Xop(9), 0x01, NO_PREFIX, false),
            ),
        ];
        for (bytes, long, expected) in cases {
            assert_eq!(found(bytes, long), Some(expected), "{bytes:02x?}");
        }
        assert_eq!(
            found(&[0xD5, 0x80, 0xAF, 0xC1], true),
            Some((Space::Two, 0xAF, NO_PREFIX, false))
        );
        assert_eq!(
            found(&[0xD5, 0x10, 0x01, 0xC1], true),
            Some((Space::One, 0x01, NO_PREFIX, false))
        );
        // The byte behind the opcode, and the opcode extension in it.
        let group = opcode_of(&[0xF3, 0x0F, 0xAE, 0xE9], true);
        assert_eq!(group.map(|e| (e.modrm, e.extension())), Some((0xE9, 5)));
        assert_eq!(opcode_of(&[0x90], true).map(|e| e.modrm), Some(0));
        // A prefix alone is no instruction.
        assert_eq!(opcode_of(&[0xF0], true), None);
        assert_eq!(opcode_of(&[], true), None);
    }

    /// What the opcode of `bytes` addresses in 64-bit code.
    fn takes(bytes: &[u8]) -> u32 {
        opcode_of(bytes, true).map_or(0, |encoded| addressable(&encoded, true))
    }

    #[test]
    fn an_opcode_addresses_the_registers_of_its_map() {
        // `add %ecx, %eax`; `fadd %st(1)`; `fnstsw %ax`.
        assert_eq!(takes(&[0x01, 0xC8]), GENERAL);
        assert_eq!(takes(&[0xD8, 0xC1]), X87);
        assert_eq!(takes(&[0xDF, 0xE0]), G16);
        assert_eq!(takes(&[0xDF, 0xF2]), X87);
        // `mov %eax, %ds`; `push %rcx`, and `push %cx` in 32-bit code.
        assert_eq!(takes(&[0x8E, 0xD8]), GENERAL | SEGMENT);
        assert_eq!(takes(&[0x51]), G16 | G64);
        let push = opcode_of(&[0x51], false).map(|e| addressable(&e, false));
        assert_eq!(push, Some(G16 | G32));
        // `paddb` on `%mm` registers, and on `%xmm` registers under `66`.
        assert_eq!(takes(&[0x0F, 0xFC, 0xC1]), MMX);
        assert_eq!(takes(&[0x66, 0x0F, 0xFC, 0xC1]), XMM);
        // `addps`; `cvtsi2sd` and `cvtpi2ps`; `movd` and `movq`.
        assert_eq!(takes(&[0x0F, 0x58, 0xC1]), XMM);
        assert_eq!(takes(&[0xF2, 0x0F, 0x2A, 0xC1]), XMM | WIDE);
        assert_eq!(takes(&[0x0F, 0x2A, 0xC1]), XMM | MMX);
        assert_eq!(takes(&[0x66, 0x0F, 0x6E, 0xC1]), XMM | WIDE);
        assert_eq!(takes(&[0xF3, 0x0F, 0x7E, 0xC1]), XMM);
        // `rdrand`; `rdfsbase`, `umonitor`; `vmread`; `invept`.
        assert_eq!(takes(&[0x0F, 0xC7, 0xF1]), G16 | WIDE);
        assert_eq!(takes(&[0xF3, 0x0F, 0xAE, 0xC1]), WIDE);
        assert_eq!(takes(&[0xF3, 0x0F, 0xAE, 0xF1]), G16 | WIDE);
        assert_eq!(takes(&[0x0F, 0x78, 0xC8]), G64);
        assert_eq!(takes(&[0x66, 0x0F, 0x38, 0x80, 0x0A]), G64);
        // VEX: vectors; a mask or a general register where one is taken.
        assert_eq!(takes(&[0xC5, 0xF0, 0x58, 0xC1]), VECTOR);
        assert_eq!(takes(&[0xC5, 0xF4, 0x41, 0xC1]), MASK);
        assert_eq!(takes(&[0xC5, 0xF8, 0x93, 0xC1]), MASK | WIDE);
        assert_eq!(takes(&[0xC5, 0xF9, 0x6E, 0xC1]), XMM | WIDE);
        assert_eq!(takes(&[0xC5, 0xFA, 0x7E, 0xC1]), XMM);
        assert_eq!(takes(&[0xC5, 0xF3, 0x2A, 0xC9]), XMM | WIDE);
        // `vpcmpeqd` into a vector, and EVEX-encoded into a mask.
        assert_eq!(takes(&[0xC5, 0xF1, 0x76, 0xC1]), VECTOR);
        assert_eq!(takes(&[0x62, 0xF1, 0x75, 0x08, 0x76, 0xC9]), VECTOR | MASK);
        // `vpminsd` and `vpmovm2d` share an opcode byte.
        assert_eq!(takes(&[0x62, 0xF2, 0x75, 0x08, 0x39, 0xC9]), VECTOR);
        assert_eq!(takes(&[0x62, 0xF2, 0x7E, 0x08, 0x38, 0xC9]), VECTOR | MASK);
        // What has no byte form: `lea`, `lar`, the long `nop`, `movnti`,
        // `bswap`. `seta` has no other; `rdpid` is as wide as an address.
        assert_eq!(takes(&[0x8D, 0x08]), G16 | WIDE);
        assert_eq!(takes(&[0x0F, 0x02, 0xC1]), G16 | WIDE);
        assert_eq!(takes(&[0x0F, 0x1F, 0xC1]), G16 | WIDE);
        assert_eq!(takes(&[0x0F, 0xC3, 0x0A]), WIDE);
        assert_eq!(takes(&[0x0F, 0xC9]), WIDE);
        assert_eq!(takes(&[0x0F, 0x97, 0xC1]), G8);
        assert_eq!(takes(&[0xF3, 0x0F, 0xC7, 0xF9]), G64);
        // `andn`; `setzua`, `cfcmova`; `lwpins`.
        assert_eq!(takes(&[0xC4, 0xE2, 0x70, 0xF2, 0xC9]), WIDE);
        assert_eq!(takes(&[0x62, 0xF4, 0x7F, 0x18, 0x47, 0xC1]), G8 | WIDE);
        assert_eq!(takes(&[0x62, 0xF4, 0x7C, 0x08, 0x47, 0xC1]), G16 | WIDE);
        assert_eq!(takes(&[0x8F, 0xEA, 0x70, 0x12, 0xC1]), WIDE);
    }

    #[test]
    fn two_widths_are_for_the_opcodes_that_take_two() {
        let mixes = |bytes: &[u8]| opcode_of(bytes, true).is_some_and(|e| mixes_widths(&e));
        // `movzbl`, `movslq`, `shl %cl`, `shld %cl`, `in %dx`, `crc32b`.
        assert!(mixes(&[0x0F, 0xB6, 0xC1]));
        assert!(mixes(&[0x48, 0x63, 0xC1]));
        assert!(mixes(&[0xD3, 0xE0]));
        assert!(mixes(&[0x0F, 0xA5, 0xC8]));
        assert!(mixes(&[0xEC]));
        assert!(mixes(&[0xF2, 0x0F, 0x38, 0xF0, 0xC1]));
        // `add`, `shld $5`, `andn`, `vmread`.
        assert!(!mixes(&[0x01, 0xC8]));
        assert!(!mixes(&[0x0F, 0xA4, 0xC8, 0x05]));
        assert!(!mixes(&[0xC4, 0xE2, 0x70, 0xF2, 0xC9]));
        assert!(!mixes(&[0x0F, 0x78, 0xC8]));
    }

    #[test]
    fn a_non_temporal_move_is_to_or_from_memory() {
        let only = |bytes: &[u8]| opcode_of(bytes, true).is_some_and(|e| memory_only(&e));
        // `movntdq`, `movnti`, `vmovntps`, `vmovntdqa`.
        assert!(only(&[0x66, 0x0F, 0xE7, 0x1A]));
        assert!(only(&[0x0F, 0xC3, 0x08]));
        assert!(only(&[0xC5, 0xF8, 0x2B, 0x08]));
        assert!(only(&[0xC4, 0xE2, 0x79, 0x2A, 0x08]));
        // `vpbroadcastmb2q %k1, %xmm1`, `movaps`.
        assert!(!only(&[0x62, 0xF2, 0xFE, 0x08, 0x2A, 0xC9]));
        assert!(!only(&[0x0F, 0x28, 0xC1]));
        // The half moves: `movlpd` and `movhpd`, and the stores of `movlps`
        // and `movhps`. Between registers `0F 12` is `movhlps`, and under
        // `F2` `movddup`.
        assert!(only(&[0x66, 0x0F, 0x12, 0x08]));
        assert!(only(&[0x66, 0x0F, 0x16, 0x08]));
        assert!(only(&[0x0F, 0x13, 0x08]));
        assert!(only(&[0xC5, 0xF8, 0x17, 0x08]));
        assert!(!only(&[0x0F, 0x12, 0xC1]));
        assert!(!only(&[0xF2, 0x0F, 0x12, 0xC1]));
        // `lddqu`, and `psubb` on the same opcode byte.
        assert!(only(&[0xF2, 0x0F, 0xF0, 0x08]));
        assert!(only(&[0xC5, 0xFB, 0xF0, 0x08]));
        assert!(!only(&[0x66, 0x0F, 0xF8, 0xC1]));
    }

    /// Whether the opcode of `bytes` takes vector registers of `widths`
    /// together: `x`, `y` and `z` in the statement's order.
    fn fits(bytes: &[u8], widths: &str) -> bool {
        let vectors = widths
            .bytes()
            .fold(0, |code, width| code * 4 + u32::from(width - b'x') + 1);
        opcode_of(bytes, true).is_some_and(|encoded| widths_fit(&encoded, vectors))
    }

    #[test]
    fn vector_registers_are_of_widths_the_opcode_takes_together() {
        // `vaddps`: one width, any of the three (the third EVEX-encoded).
        let vaddps = [0xC5, 0xF0, 0x58, 0xC1];
        for widths in ["xxx", "yyy", "zzz", "xx", "zz"] {
            assert!(fits(&vaddps, widths), "{widths}");
        }
        for widths in ["xyy", "yxx", "yyx", "xxz", "xy", "zy"] {
            assert!(!fits(&vaddps, widths), "{widths}");
        }
        // `vaddsd`, `vdppd`: `%xmm` only. `vpermd`: never `%xmm`.
        assert!(fits(&[0xC5, 0xF3, 0x58, 0xC1], "xxx"));
        assert!(!fits(&[0xC5, 0xF3, 0x58, 0xC1], "yyy"));
        assert!(fits(&[0xC4, 0xE3, 0x71, 0x41, 0xC1, 0x05], "xxx"));
        assert!(!fits(&[0xC4, 0xE3, 0x75, 0x41, 0xC1, 0x05], "yyy"));
        assert!(fits(&[0xC4, 0xE2, 0x75, 0x36, 0xC1], "yyy"));
        assert!(!fits(&[0xC4, 0xE2, 0x71, 0x36, 0xC1], "xxx"));
        // `vpsllw`: the count is an `%xmm` whatever is shifted.
        let vpsllw = [0xC5, 0xF5, 0xF1, 0xC1];
        assert!(fits(&vpsllw, "xyy") && fits(&vpsllw, "xzz") && fits(&vpsllw, "xxx"));
        assert!(!fits(&vpsllw, "yyy") && !fits(&vpsllw, "xyz"));
        // `vpmovsxbw` widens and `vpmovwb` narrows, each by its own steps.
        let vpmovsxbw = [0xC4, 0xE2, 0x7D, 0x20, 0xC1];
        assert!(fits(&vpmovsxbw, "xx") && fits(&vpmovsxbw, "xy") && fits(&vpmovsxbw, "yz"));
        assert!(!fits(&vpmovsxbw, "yy") && !fits(&vpmovsxbw, "yx") && !fits(&vpmovsxbw, "xz"));
        let vpmovwb = [0x62, 0xF2, 0x7E, 0x28, 0x30, 0xC1];
        assert!(fits(&vpmovwb, "yx") && fits(&vpmovwb, "zy"));
        assert!(!fits(&vpmovwb, "xy") && !fits(&vpmovwb, "zx"));
        // `vinsertf128`, `vextractf128`; `vcvtps2ph`.
        assert!(fits(&[0xC4, 0xE3, 0x75, 0x18, 0xC1, 0x01], "xyy"));
        assert!(!fits(&[0xC4, 0xE3, 0x75, 0x18, 0xC1, 0x01], "yyy"));
        assert!(fits(&[0xC4, 0xE3, 0x7D, 0x19, 0xC1, 0x01], "yx"));
        assert!(fits(&[0xC4, 0xE3, 0x79, 0x1D, 0xC1, 0x05], "yx"));
        assert!(!fits(&[0xC4, 0xE3, 0x79, 0x1D, 0xC1, 0x05], "xz"));
        // XOP: `vpcmov` on `%xmm` and `%ymm`, never both.
        let vpcmov = [0x8F, 0xE8, 0x70, 0xA2, 0xC1, 0x20];
        assert!(fits(&vpcmov, "xxxx") && fits(&vpcmov, "yyyy"));
        assert!(!fits(&vpcmov, "xxyy") && !fits(&vpcmov, "zzzz"));
        // `vmovaps`: a store names one register, a move between registers
        // two.
        let store = [0xC5, 0xF8, 0x29, 0x08];
        assert!(fits(&store, "x") && fits(&store, "y") && fits(&store, "yy"));
        assert!(!fits(&store, "xy"));
        // Not judged: an opcode outside VEX, one the table has nothing on,
        // and a statement of more or fewer vector registers than any the
        // table has of its opcode.
        assert!(fits(&[0x66, 0x0F, 0x58, 0xC1], "xy"));
        assert!(fits(&[0xC4, 0xE2, 0x70, 0xF2, 0xC1], "xy"));
        assert!(fits(&vaddps, "x") && fits(&vaddps, "xxxx") && fits(&vaddps, "xxxxx"));
    }

    #[test]
    fn a_complex_multiplication_keeps_its_destination_apart() {
        let apart = |bytes: &[u8]| opcode_of(bytes, true).is_some_and(|e| destination_apart(&e));
        // `vfmaddcph`, `vfcmaddcph`, `vfmulcsh`, `vfcmulcph`.
        assert!(apart(&[0x62, 0xF6, 0x6E, 0x08, 0x56, 0xD9]));
        assert!(apart(&[0x62, 0xF6, 0x6F, 0x08, 0x56, 0xD9]));
        assert!(apart(&[0x62, 0xF6, 0x6E, 0x08, 0xD7, 0xD9]));
        assert!(apart(&[0x62, 0xF6, 0x6F, 0x48, 0xD6, 0xD9]));
        // `vfmadd132ph` (map 6, `66`), `vaddps`.
        assert!(!apart(&[0x62, 0xF6, 0x6D, 0x08, 0x98, 0xD9]));
        assert!(!apart(&[0xC5, 0xF0, 0x58, 0xC1]));
        let register = |name: &str| Operand::Register(Register::new(name));
        let memory = || Operand::Label("sym".to_string());
        let among = |operands: &[Operand]| destination_among_sources(operands);
        assert!(!among(&[
            register("xmm1"),
            register("xmm2"),
            register("xmm3")
        ]));
        assert!(among(&[
            register("xmm1"),
            register("xmm2"),
            register("xmm1")
        ]));
        assert!(among(&[
            register("xmm1"),
            register("xmm2"),
            register("xmm2")
        ]));
        assert!(among(&[memory(), register("zmm3"), register("zmm3")]));
        assert!(!among(&[memory(), register("zmm2"), register("zmm3")]));
        assert!(!among(&[register("xmm1")]) && !among(&[]));
    }

    #[test]
    fn the_table_of_widths_is_in_order() {
        // `widths_fit` searches it.
        assert!(OPCODES.windows(2).all(|pair| pair[0].0 < pair[1].0));
        assert!(
            OPCODES
                .iter()
                .all(|&(_, shape)| usize::from(shape) < SHAPES.len())
        );
        assert!(SHAPES.iter().all(|shape| !shape.is_empty()
            && shape.windows(2).all(|pair| pair[0] < pair[1])
            && shape.iter().all(|&code| {
                // Base-4 digits 1 to 3: no register is of no width.
                let mut code = code;
                while code != 0 && code % 4 != 0 {
                    code /= 4;
                }
                code == 0
            })));
    }

    /// `W` is found in every prefix that has one.
    #[test]
    fn the_w_bit_is_read_from_every_prefix() {
        let wide = |bytes: &[u8]| opcode_of(bytes, true).map(|encoded| encoded.wide);
        for (bytes, expected) in [
            // `cmova %ecx, %edx`; with REX.W; with REX.R only.
            (&[0x0F, 0x47, 0xD1][..], false),
            (&[0x48, 0x0F, 0x47, 0xD1], true),
            (&[0x44, 0x0F, 0x47, 0xD1], false),
            // `66` and REX.W together: W wins the size, and is still W.
            (&[0x66, 0x48, 0x0F, 0x47, 0xD1], true),
            // Three-byte VEX: `rorx $3, %ecx, %ecx` and its 64-bit form.
            (&[0xC4, 0xE3, 0x7B, 0xF0, 0xC9, 0x03], false),
            (&[0xC4, 0xE3, 0xFB, 0xF0, 0xC9, 0x03], true),
            // Two-byte VEX has no W.
            (&[0xC5, 0xFA, 0x2C, 0xC9], false),
            // EVEX: `vmovd %xmm17, %ecx`, `vmovq %xmm17, %rcx`.
            (&[0x62, 0xE1, 0x7D, 0x08, 0x7E, 0xC9], false),
            (&[0x62, 0xE1, 0xFD, 0x08, 0x7E, 0xC9], true),
            // XOP: `vpcmov` with W.
            (&[0x8F, 0xE8, 0xF0, 0xA2, 0xC1, 0x20], true),
            (&[0x8F, 0xE8, 0x70, 0xA2, 0xC1, 0x20], false),
            // REX2: `imulq %r16, %rax` (`d5 98 af c0`), `addl %r16d, %eax`.
            (&[0xD5, 0x98, 0xAF, 0xC0], true),
            (&[0xD5, 0x40, 0x01, 0xC0], false),
        ] {
            assert_eq!(wide(bytes), Some(expected), "{bytes:02x?}");
        }
        // In 32-bit code `48` is `dec %eax`, an opcode of its own.
        assert_eq!(
            opcode_of(&[0x48], false).map(|encoded| (encoded.opcode, encoded.wide)),
            Some((0x48, false))
        );
    }

    /// An operand-size prefix is noted beside a mandatory `F3` or `F2`,
    /// and read out of the `pp` field of a VEX, EVEX or XOP prefix.
    #[test]
    fn the_operand_size_prefix_is_found_among_the_others() {
        let data16 = |bytes: &[u8]| opcode_of(bytes, true).map(|encoded| encoded.data16);
        for (bytes, expected) in [
            (&[0x01, 0xC8][..], false),
            (&[0x66, 0x01, 0xC8], true),
            // In either order with `F3`, and behind a segment prefix.
            (&[0x66, 0xF3, 0x0F, 0xB8, 0xD1], true),
            (&[0xF3, 0x66, 0x0F, 0xB8, 0xD1], true),
            (&[0xF3, 0x0F, 0xB8, 0xD1], false),
            (&[0x64, 0x66, 0x01, 0x08], true),
            // REX2 behind `66`.
            (&[0x66, 0xD5, 0x40, 0x01, 0xC0], true),
            // `vmovd` has `66` in its `pp` field, `vcvtsi2ss` has `F3`.
            (&[0xC5, 0xF9, 0x6E, 0xC1], true),
            (&[0xC5, 0x72, 0x2A, 0xC9], false),
            (&[0x62, 0xF4, 0x7D, 0x08, 0x43, 0xD1], true),
            (&[0x62, 0xF4, 0x7C, 0x08, 0x43, 0xD1], false),
        ] {
            assert_eq!(data16(bytes), Some(expected), "{bytes:02x?}");
        }
    }

    /// The opcodes whose general registers are all as wide as the
    /// operation, and the width each encoding takes.
    #[test]
    fn the_width_an_opcode_takes_its_general_registers_in() {
        let width =
            |bytes: &[u8]| opcode_of(bytes, true).and_then(|e| general_width(&e, Code::Bits64));
        for (bytes, expected) in [
            // `cmovcc`: 16 bits under `66`, 64 under REX.W, else 32.
            (&[0x0F, 0x47, 0xD1][..], Some(G32)),
            (&[0x66, 0x0F, 0x47, 0xD1], Some(G16)),
            (&[0x48, 0x0F, 0x47, 0xD1], Some(G64)),
            // APX `cfcmovcc` / three-operand `cmovcc`; `setcc` under `F2`
            // has registers of its own.
            (&[0x62, 0xF4, 0x7C, 0x08, 0x43, 0xD1], Some(G32)),
            (&[0x62, 0xF4, 0x7D, 0x08, 0x43, 0xD1], Some(G16)),
            (&[0x62, 0xF4, 0xFC, 0x08, 0x43, 0xD1], Some(G64)),
            (&[0x62, 0xF4, 0x7F, 0x08, 0x44, 0xC0], None),
            // `cvttss2si` into 32 and into 64 bits; the packed
            // `cvttps2pi` is another instruction.
            (&[0xF3, 0x0F, 0x2C, 0xC9], Some(G32)),
            (&[0xF3, 0x48, 0x0F, 0x2C, 0xC9], Some(G64)),
            (&[0x0F, 0x2C, 0xC9], None),
            // `vcvtsi2ss`; `movd` and `movq`; `movq` between `%xmm`s.
            (&[0xC5, 0x72, 0x2A, 0xC9], Some(G32)),
            (&[0x66, 0x0F, 0x6E, 0xC0], Some(G32)),
            (&[0x66, 0x48, 0x0F, 0x6E, 0xC0], Some(G64)),
            (&[0xF3, 0x0F, 0x7E, 0xC1], None),
            // BMI: `andn`, `rorx`.
            (&[0xC4, 0xE2, 0x70, 0xF2, 0xC2], Some(G32)),
            (&[0xC4, 0xE3, 0xFB, 0xF0, 0xC9, 0x03], Some(G64)),
            // The integer instructions: the low bit of the opcode is clear
            // for bytes.
            (&[0x01, 0xC8], Some(G32)),
            (&[0x66, 0x01, 0xC8], Some(G16)),
            (&[0x48, 0x01, 0xC8], Some(G64)),
            (&[0x00, 0xC8], Some(G8)),
            (&[0x04, 0x05], Some(G8)),
            (&[0x05, 0x34, 0x12, 0x00, 0x00], Some(G32)),
            (&[0x80, 0xC1, 0x05], Some(G8)),
            (&[0x66, 0x83, 0xC1, 0xFD], Some(G16)),
            (&[0x85, 0xC9], Some(G32)),
            (&[0x86, 0xC8], Some(G8)),
            (&[0x49, 0x91], Some(G64)),
            (&[0x66, 0x95], Some(G16)),
            // `xchg %rax, %rax` is `nop`, written without a `W`; `xchg
            // %r8, %rax` has one.
            (&[0x90], Some(G32 | G64)),
            (&[0x49, 0x90], Some(G64)),
            (&[0x41, 0x90], Some(G32)),
            (&[0x66, 0x90], Some(G16)),
            (&[0xB1, 0x90], Some(G8)),
            (&[0x48, 0xB8, 1, 2, 3, 4, 5, 6, 7, 8], Some(G64)),
            // `movq $0xffffffff, %rax` is written as `movl` is.
            (&[0xB8, 0xFF, 0xFF, 0xFF, 0xFF], Some(G32 | G64)),
            (&[0x66, 0xB8, 0x34, 0x12], Some(G16)),
            (&[0xA2, 1, 2, 3, 4, 5, 6, 7, 8], Some(G8)),
            (&[0x66, 0xA3, 1, 2, 3, 4, 5, 6, 7, 8], Some(G16)),
            (&[0xD2, 0xEE], Some(G8)),
            (&[0x48, 0xD3, 0xE2], Some(G64)),
            (&[0xF6, 0xE1], Some(G8)),
            (&[0xF7, 0xF1], Some(G32)),
            (&[0xFE, 0xC1], Some(G8)),
            (&[0x0F, 0xA5, 0xC3], Some(G32)),
            (&[0x66, 0x0F, 0xBC, 0xC1], Some(G16)),
            (&[0x0F, 0xC0, 0x0A], Some(G8)),
            (&[0x48, 0x0F, 0xB1, 0x0A], Some(G64)),
            (&[0x0F, 0x38, 0xF0, 0x08], Some(G32)),
            // `popcnt %cx, %dx`: the `66` beside the mandatory `F3`.
            (&[0x66, 0xF3, 0x0F, 0xB8, 0xD1], Some(G16)),
            (&[0xF3, 0x48, 0x0F, 0xB8, 0xD1], Some(G64)),
            // What is pushed, popped and branched through is 64 bits
            // wide without a `W`; `inc` beside them in the group is not.
            (&[0x51], Some(G64)),
            (&[0x66, 0x51], Some(G16)),
            (&[0x41, 0x58], Some(G64)),
            (&[0x8F, 0xC1], Some(G64)),
            (&[0xFF, 0xD0], Some(G64)),
            (&[0xFF, 0xE0], Some(G64)),
            (&[0x66, 0xFF, 0xD0], Some(G16)),
            (&[0xFF, 0xC0], Some(G32)),
            (&[0x48, 0xFF, 0xC8], Some(G64)),
            // What mixes widths, or has registers of another kind beside
            // the general one: `movzbl`, `movslq`, `crc32`, `mov` from a
            // segment register, `push %fs`, `lar`, `in`, `stos`.
            (&[0x0F, 0xB6, 0xC1], None),
            (&[0x48, 0x63, 0xC8], None),
            (&[0xF2, 0x0F, 0x38, 0xF1, 0xC9], None),
            (&[0x8C, 0xD8], None),
            (&[0x0F, 0xA0], None),
            (&[0x0F, 0x02, 0xCD], None),
            (&[0xEC], None),
            (&[0xAA], None),
            // A far call through memory has no register.
            (&[0xFF, 0x18], None),
        ] {
            assert_eq!(width(bytes), expected, "{bytes:02x?}");
        }
    }

    /// `66` selects the operand size that is not the code's own, and in
    /// 16- and 32-bit code the stack is as wide as an operand.
    #[test]
    fn the_operand_size_prefix_is_read_for_the_code() {
        let width = |bytes: &[u8], code| {
            opcode_of(bytes, false).and_then(|encoded| general_width(&encoded, code))
        };
        for (bytes, in_32, in_16) in [
            (&[0x01, 0xC8][..], G32, G16),
            (&[0x66, 0x01, 0xC8], G16, G32),
            (&[0x00, 0xC8], G8, G8),
            (&[0x0F, 0x47, 0xD1], G32, G16),
            (&[0x66, 0x0F, 0x47, 0xD1], G16, G32),
            // `inc %ebp` and `dec %bp`: no REX prefixes here.
            (&[0x45], G32, G16),
            (&[0x66, 0x4D], G16, G32),
            (&[0x51], G32, G16),
            (&[0x66, 0x51], G16, G32),
            (&[0xB9, 0x78, 0x56, 0x34, 0x12], G32, G16),
            (&[0x90], G32, G16),
            (&[0xFF, 0xD1], G32, G16),
            (&[0x66, 0xFF, 0xE1], G16, G32),
            (&[0x67, 0x8D, 0x0A], G32, G16),
            // `popcnt %cx, %dx` in 32-bit code, `popcnt %ecx, %edx` in
            // 16-bit code.
            (&[0x66, 0xF3, 0x0F, 0xB8, 0xD1], G16, G32),
        ] {
            assert_eq!(width(bytes, Code::Bits32), Some(in_32), "{bytes:02x?}");
            assert_eq!(width(bytes, Code::Bits16), Some(in_16), "{bytes:02x?}");
        }
        // The conversions and BMI go by `W`, which there is none of: 32
        // bits in every code.
        for bytes in [
            &[0xF3, 0x0F, 0x2C, 0xC9][..],
            &[0x66, 0x0F, 0x6E, 0xC9],
            &[0xC4, 0xE2, 0x68, 0xF2, 0xD9],
        ] {
            assert_eq!(width(bytes, Code::Bits16), Some(G32), "{bytes:02x?}");
            assert_eq!(width(bytes, Code::Bits32), Some(G32), "{bytes:02x?}");
        }
    }

    /// `%cl` in front of what a shift shifts is its count.
    #[test]
    fn a_shift_s_count_is_not_what_it_shifts() {
        let by_count = |bytes: &[u8]| opcode_of(bytes, true).is_some_and(|e| shifts_by_count(&e));
        for bytes in [
            &[0xD3, 0xE2][..],
            &[0xD2, 0xEE],
            &[0x0F, 0xA5, 0xC3],
            &[0x48, 0x0F, 0xAD, 0xC3],
        ] {
            assert!(by_count(bytes), "{bytes:02x?}");
        }
        // By one, by a number; `bt` and `imul` beside the double shifts.
        for bytes in [
            &[0xD1, 0xE2][..],
            &[0xC1, 0xE2, 0x05],
            &[0x0F, 0xA4, 0xC3, 0x05],
            &[0x0F, 0xA3, 0xC3],
            &[0x0F, 0xAF, 0xC3],
        ] {
            assert!(!by_count(bytes), "{bytes:02x?}");
        }
        let register = |name: &str| Operand::Register(Register::new(name));
        let kinds = |names: &[&str]| {
            let operands: Vec<Operand> = names.iter().map(|name| register(name)).collect();
            named(without_count(&operands)).kinds
        };
        assert_eq!(kinds(&["cl", "edx"]), G32);
        assert_eq!(kinds(&["cl", "ax", "bx"]), G16);
        // `shl %cl` shifts `%cl` by one, and `shl %cl, %cl` by itself.
        assert_eq!(kinds(&["cl"]), G8);
        assert_eq!(kinds(&["cl", "cl"]), G8);
        // Another register in front is no count.
        assert_eq!(kinds(&["dl", "edx"]), G8 | G32);
    }

    #[test]
    fn a_register_only_evex_reaches_is_noted() {
        let register = |name: &str| Operand::Register(Register::new(name));
        for (names, high) in [
            (&["xmm15", "xmm0"][..], false),
            (&["xmm16", "xmm0"], true),
            (&["ymm7", "ymm31"], true),
            (&["zmm9", "zmm10", "zmm11"], false),
            (&["rax", "r31"], false),
            (&["k7", "xmm3"], false),
        ] {
            let operands: Vec<Operand> = names.iter().map(|name| register(name)).collect();
            assert_eq!(named(&operands).high, high, "{names:?}");
        }
    }

    #[test]
    fn a_move_between_files_names_one_register_of_each() {
        let named = |general, files, memory, masks| Named {
            kinds: 0,
            general,
            masks,
            files,
            memory,
            vectors: 0,
            high: false,
        };
        let transfer = XMM | WIDE;
        // `movd %eax, %xmm0`, `movd (%rax), %xmm0`, `cvtsd2si (%rax), %eax`.
        assert!(counts_fit(&named(1, 1, 0, 0), transfer));
        assert!(counts_fit(&named(0, 1, 1, 0), transfer));
        assert!(counts_fit(&named(1, 0, 1, 0), transfer));
        // `movd %xmm0, %xmm1`, `movd %eax, %ecx`, `vcvtsi2sd %eax, %ecx, %xmm0`.
        assert!(!counts_fit(&named(0, 2, 0, 0), transfer));
        assert!(!counts_fit(&named(2, 0, 0, 0), transfer));
        assert!(!counts_fit(&named(2, 1, 0, 0), transfer));
        // `kmovd %eax, %k1` and `kmovd %k1, %k2` (which has its own opcode).
        assert!(counts_fit(&named(1, 1, 0, 1), MASK | WIDE));
        assert!(counts_fit(&named(0, 2, 0, 2), MASK));
        // `vpcmpeqd %xmm0, %xmm1, %k1`; without a mask, and with two.
        assert!(counts_fit(&named(0, 3, 0, 1), VECTOR | MASK));
        assert!(!counts_fit(&named(0, 3, 0, 0), VECTOR | MASK));
        assert!(!counts_fit(&named(0, 3, 0, 2), VECTOR | MASK));
        // Nothing is counted where one kind of register is all there is.
        assert!(counts_fit(&named(3, 0, 0, 0), GENERAL));
        assert!(counts_fit(&named(0, 3, 0, 0), VECTOR));
    }
}
