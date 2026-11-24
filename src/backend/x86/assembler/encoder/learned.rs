//! Instructions encoded from rows learned from GNU as.
//!
//! An instruction is encoded by the arm that was written for it, and one
//! nobody wrote an arm for was "unhandled" however regular its encoding.
//! GNU as assembles some five hundred mnemonics the arms do not know -- the
//! AMD extensions (3DNow!, FMA4, XOP, TBM), Key Locker, AMX, AVX10.2, SM3,
//! SHA512, the MMX conversions of SSE -- and forms of known ones that no arm
//! takes. They differ in an opcode and in where the operands go, which is a
//! row of a table and not an encoder.
//!
//! `scripts/x86_learned_rows.py` writes that table (`learned_rows.rs`) by
//! asking GNU as 2.47: from the bytes of two statements that differ in
//! their registers it works out the prefix, the map, the opcode and the
//! field of each operand, and further statements say what else the form
//! takes -- a mask, a broadcast, a rounding mode, a register of APX, the
//! reach of a one-byte displacement, the range of a number, which operands
//! must be different registers. This file matches a statement against the
//! rows and writes the bytes. The script's `--verify` holds what comes out
//! to GNU as over the statements the solving held still (every register of
//! a kind, the addresses, the decorations, the numbers, the encoding
//! hints); `gas_table_sweep.py` compares again, from the opcode table.
//!
//! The rows are tried only for a statement the arms and the hand-written
//! table (`table`) refuse, so nothing an arm encodes is encoded differently
//! because of them, and a statement the compiler writes never gets here.
//!
//! One thing GNU as was asked is held against every encoder: the range of
//! a number that is one byte of the instruction (`NUMBERS`, `check_number`).
//! An arm wrote the low byte of whatever the statement said.

use super::InstructionEncoder;
use super::learned_rows::{NAMES, NUMBER_NAMES, NUMBER_STARTS, NUMBERS, ROWS, STARTS};
use super::operand_class::Code;
use crate::backend::x86::assembler::parser::{
    ImmediateValue, MemoryOperand, Operand, rex_prefix_word,
};

/// One form of one mnemonic: the kind of each operand, and its encoding.
#[derive(Clone, Copy, Debug)]
pub(super) struct Row {
    /// What else the form is and takes: the `W` bit, the modes, the
    /// decorations (the constants below).
    pub flags: u32,
    /// Pairs of operands that must be different registers: bit `i * 5 + j`
    /// for the operands `i < j`.
    pub distinct: u32,
    /// The kind of each operand in AT&T order (`R8`..`XMM0`); 0 behind the
    /// last one.
    pub kinds: [u8; 5],
    /// The field each operand is encoded in (`REG`..`LOW4`).
    pub slots: [u8; 5],
    /// `LEGACY`, `VEX`, `EVEX` or `XOP`.
    pub space: u8,
    /// The opcode map: of a legacy opcode 0, 1 (`0F`), 2 (`0F 38`), 3
    /// (`0F 3A`) or 4 (`0F 0F`, with the opcode behind the operands); else
    /// the map field of the prefix.
    pub map: u8,
    /// The mandatory prefix as a VEX prefix numbers it: none, `66`, `F3`,
    /// `F2`.
    pub prefix: u8,
    /// The vector length field.
    pub l: u8,
    pub opcode: u8,
    /// `ModRM.reg` where no operand has it, or -1.
    pub digit: i8,
    /// `ModRM.rm` of a register form where no operand has it, or -1.
    pub fixed_rm: i8,
    /// A byte behind the operands that is part of the opcode, or -1.
    pub tail: i16,
    /// The bytes behind the opcode of a form without a ModRM byte.
    pub literal: [u8; 3],
    pub literal_len: u8,
    /// A one-byte displacement counts units of two to this power.
    pub disp8: u8,
    /// The `N` of the `{1toN}` the form takes, or 0.
    pub broadcast: u8,
    /// `disp8` of the broadcast form.
    pub broadcast_disp8: u8,
    /// The operand a rounding word stands in front of.
    pub rounding_at: u8,
    /// The vector length field under `{sae}`.
    pub sae_l: u8,
    /// The range of the number (`IMM_*`), or 0.
    pub immediate: u8,
    /// The operands that cannot be the stack pointer, a bit each (`push2`
    /// and `pop2`).
    pub no_sp: u8,
}

/// A row, from its fields in the order `x86_learned_rows.py` writes them:
/// the last seven are `disp8`, `broadcast`, `broadcast_disp8`,
/// `rounding_at`, `sae_l`, `immediate` and `no_sp`.
pub(super) const fn row(
    flags: u32,
    distinct: u32,
    kinds: [u8; 5],
    slots: [u8; 5],
    space: u8,
    map: u8,
    prefix: u8,
    l: u8,
    opcode: u8,
    digit: i8,
    fixed_rm: i8,
    tail: i16,
    literal: [u8; 3],
    literal_len: u8,
    rest: [u8; 7],
) -> Row {
    Row {
        flags,
        distinct,
        kinds,
        slots,
        space,
        map,
        prefix,
        l,
        opcode,
        digit,
        fixed_rm,
        tail,
        literal,
        literal_len,
        disp8: rest[0],
        broadcast: rest[1],
        broadcast_disp8: rest[2],
        rounding_at: rest[3],
        sae_l: rest[4],
        immediate: rest[5],
        no_sp: rest[6],
    }
}

pub(super) const W: u32 = 1 << 0;
/// A ModRM byte follows the opcode.
pub(super) const MODRM: u32 = 1 << 1;
/// An operand size was chosen: 16-bit code inverts the operand-size prefix.
pub(super) const SIZED: u32 = 1 << 2;
/// The operand-size prefix `66`, in front of the mandatory one.
pub(super) const DATA16: u32 = 1 << 3;
/// The address has a SIB byte whatever it is.
pub(super) const SIB: u32 = 1 << 4;
pub(super) const MASK: u32 = 1 << 5;
pub(super) const ZERO: u32 = 1 << 6;
/// Takes `%r16` to `%r31`, as operands and in the address.
pub(super) const EGPR: u32 = 1 << 7;
pub(super) const M64: u32 = 1 << 8;
pub(super) const M32: u32 = 1 << 9;
pub(super) const M16: u32 = 1 << 10;
/// Takes `{sae}`.
pub(super) const SAE: u32 = 1 << 11;
/// Takes the four rounding modes.
pub(super) const ROUND: u32 = 1 << 12;
/// The EVEX encoding of a form that has another: chosen where a register
/// needs it, or where `{evex}` asks.
pub(super) const WHEN_EVEX: u32 = 1 << 13;
/// The VEX encoding of a form GNU as writes with EVEX: chosen under `{vex}`.
pub(super) const WHEN_VEX: u32 = 1 << 14;
/// The `ND` bit of an APX instruction.
pub(super) const ND: u32 = 1 << 15;
/// The general register is an address itself (`movdir64b`, `enqcmd`): the
/// registers of the memory operand are as wide as it is.
pub(super) const ADDRESSED: u32 = 1 << 16;

pub(super) const LEGACY: u8 = 0;
pub(super) const VEX: u8 = 1;
pub(super) const EVEX: u8 = 2;
pub(super) const XOP: u8 = 3;

// The kinds, as `x86_learned_rows.py` numbers them (`KINDS`).
const R8: u8 = 1;
const R16: u8 = 2;
const R32: u8 = 3;
const R64: u8 = 4;
const MM: u8 = 5;
const XMM: u8 = 6;
const YMM: u8 = 7;
const ZMM: u8 = 8;
const MASK_REGISTER: u8 = 9;
const TMM: u8 = 10;
const MEMORY: u8 = 11;
const IMM8: u8 = 12;
/// From here on a kind is one register: `FIXED[kind - FIRST_FIXED]`.
const FIRST_FIXED: u8 = 13;
const FIXED: [&str; 14] = [
    "al", "ax", "eax", "rax", "cl", "cx", "ecx", "rcx", "dx", "edx", "rdx", "ebx", "rbx", "xmm0",
];

// The fields (`SLOTS`).
const REG: u8 = 1;
const RM: u8 = 2;
const VVVV: u8 = 3;
const IS4: u8 = 4;
const IMM: u8 = 5;
const LOW4: u8 = 6;

// The ranges of a number (`refine` in the script).
const IMM_BYTE: u8 = 1; // -128..=255
const IMM_UNSIGNED: u8 = 2; // 0..=255
const IMM_SIGNED: u8 = 3; // -128..=127
const IMM_NIBBLE: u8 = 4; // 0..=15

/// The range of the one-byte number of a mnemonic's forms.
#[derive(Clone, Copy, Debug)]
pub(super) struct Number {
    /// The modes the range is of (`M64`, `M32`, `M16`).
    pub modes: u32,
    /// The kinds of the operands of the form, as a row has them; no kinds
    /// for every form of the mnemonic.
    pub kinds: [u8; 5],
    /// The range (`IMM_*`).
    pub range: u8,
}

/// A range, from its fields in the order `x86_learned_rows.py` writes them.
pub(super) const fn number(modes: u32, kinds: [u8; 5], range: u8) -> Number {
    Number {
        modes,
        kinds,
        range,
    }
}

/// Whether `value` is in the range `range` names.
fn in_range(range: u8, value: i64) -> bool {
    match range {
        IMM_BYTE => (-128..=255).contains(&value),
        IMM_UNSIGNED => (0..=255).contains(&value),
        IMM_SIGNED => (-128..=127).contains(&value),
        IMM_NIBBLE => (0..=15).contains(&value),
        _ => false,
    }
}

/// Whether the operands `ops` are of the kinds `kinds`, in order.
fn of_kinds(kinds: &[u8; 5], ops: &[Operand]) -> bool {
    let arity = kinds.iter().take_while(|&&kind| kind != 0).count();
    ops.len() == arity
        && ops.iter().zip(kinds).all(|(op, &kind)| match op {
            Operand::Immediate(_) => kind == IMM8,
            Operand::Memory(_) | Operand::Label(_) => kind == MEMORY,
            Operand::Register(reg) => match kind.checked_sub(FIRST_FIXED) {
                Some(fixed) => FIXED.get(usize::from(fixed)) == Some(&reg.name.as_str()),
                None => classify(&reg.name).is_some_and(|(is, ..)| is == kind),
            },
            _ => false,
        })
}

/// Hold the number of `name ops` to the range GNU as takes it in, where it
/// is one byte of the instruction.
///
/// GNU as refuses a number outside it; an arm that wrote `value as u8`
/// assembled `bt $256, %ecx` as a test of bit 0 and `int $256` as `int
/// $0`. The ranges are of GNU as's forms, whatever encodes them: 0 to 255
/// for most, -128 to 255 for those it takes a negative spelling of
/// (`pshufd $-1`), and none for a form whose number another row of its
/// table takes wider (`add`) or that it cuts with a warning (`rolb $256,
/// %cl`).
///
/// Asked of every statement, so it costs one with no number a look at its
/// operands and one with a number a search among the names -- no
/// allocation: `mnemonic` is compared as it is written, in either case.
pub(crate) fn check_number(mnemonic: &str, ops: &[Operand], code: Code) -> Result<(), String> {
    let mut numbers = ops.iter().filter_map(|op| match op {
        Operand::Immediate(immediate) => Some(immediate),
        _ => None,
    });
    let (Some(ImmediateValue::Integer(value)), None) = (numbers.next(), numbers.next()) else {
        return Ok(());
    };
    let lowercased = |name: &&str| {
        name.bytes()
            .cmp(mnemonic.bytes().map(|byte| byte.to_ascii_lowercase()))
    };
    let Ok(at) = NUMBER_NAMES.binary_search_by(lowercased) else {
        return Ok(());
    };
    let entries = NUMBER_STARTS
        .get(at)
        .zip(NUMBER_STARTS.get(at + 1))
        .and_then(|(&from, &to)| NUMBERS.get(usize::from(from)..usize::from(to)))
        .unwrap_or(&[]);
    let mode = match code {
        Code::Bits64 => M64,
        Code::Bits32 => M32,
        Code::Bits16 => M16,
    };
    let entry = entries.iter().find(|entry| {
        entry.modes & mode != 0 && (entry.kinds[0] == 0 || of_kinds(&entry.kinds, ops))
    });
    match entry {
        Some(entry) if !in_range(entry.range, *value) => Err(format!(
            "operand type mismatch for `{}'",
            mnemonic.to_ascii_lowercase()
        )),
        _ => Ok(()),
    }
}

/// The kind and the number of a register, and whether it is one of the
/// byte registers no prefix but the plain one can name.
fn classify(name: &str) -> Option<(u8, u8, bool)> {
    let numbered = |digits: &str, limit: u8| {
        // `%xmm01` is no register.
        if digits.len() > 1 && digits.starts_with('0') {
            return None;
        }
        digits.parse::<u8>().ok().filter(|&number| number < limit)
    };
    let (kind, number) = match name.as_bytes() {
        [b'x', b'm', b'm', ..] => (XMM, numbered(&name[3..], 32)?),
        [b'y', b'm', b'm', ..] => (YMM, numbered(&name[3..], 32)?),
        [b'z', b'm', b'm', ..] => (ZMM, numbered(&name[3..], 32)?),
        [b't', b'm', b'm', ..] => (TMM, numbered(&name[3..], 8)?),
        [b'm', b'm', ..] => (MM, numbered(&name[2..], 8)?),
        [b'k', ..] => (MASK_REGISTER, numbered(&name[1..], 8)?),
        _ => {
            let number = super::gp_id(name)?;
            let kind = match super::register_size_suffix(name)? {
                'b' => R8,
                'w' => R16,
                'l' => R32,
                _ => R64,
            };
            let high = matches!(name, "ah" | "ch" | "dh" | "bh");
            return Some((kind, number, high));
        }
    };
    Some((kind, number, false))
}

/// The rows of `name`, or `None` when the table does not know it.
pub(super) fn rows_named(name: &str) -> Option<&'static [Row]> {
    let at = NAMES.binary_search(&name).ok()?;
    let from = usize::from(*STARTS.get(at)?);
    let to = usize::from(*STARTS.get(at + 1)?);
    ROWS.get(from..to)
}

/// Whether rows are called `name`.
pub(crate) fn knows(name: &str) -> bool {
    rows_named(name).is_some()
}

/// What a row says to write for a statement it takes.
#[derive(Debug)]
pub(crate) struct Learned<'a> {
    row: &'static Row,
    reg: u8,
    rm: u8,
    memory: Option<&'a MemoryOperand>,
    vvvv: u8,
    /// The fourth register, in the upper half of the byte behind the
    /// operands.
    is4: u8,
    /// The number: a byte of its own, or the lower half of that byte.
    number: u8,
    mask: u8,
    zero: bool,
    broadcast: bool,
    /// `EVEX.b` and the vector length field that goes with it.
    rounding: Option<u8>,
    /// The register in `ModRM.rm` is a general one: its fifth bit is `B4`,
    /// where a vector register's is `X`.
    rm_general: bool,
    /// `%spl`, `%bpl`, `%sil` or `%dil` is an operand: only behind a REX
    /// prefix are they told from `%ah` to `%bh`.
    low_byte: bool,
}

impl Learned<'_> {
    /// The operand size was chosen by the statement: 16-bit code inverts
    /// the operand-size prefix of such an instruction.
    pub(crate) fn sized(&self) -> bool {
        self.row.flags & SIZED != 0
    }
}

/// What the statement asks of the encoding beside its operands.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Asked {
    pub code: Code,
    /// `{evex}`.
    pub evex: bool,
    /// `{vex}`, `{vex2}` or `{vex3}`.
    pub vex: bool,
    /// `{rex2}`.
    pub rex2: bool,
}

/// Whether a learned row may stand behind the prefix words `prefixes`.
///
/// The encoders hold a repeat prefix, `lock` and the HLE hints against the
/// mnemonic before any row is looked at (`repeat_prefix`,
/// `check_lock_prefixes`), and write the bytes of those, of a segment word
/// and of the size words around whatever encoded the instruction, and
/// the bits of a REX word into it or in front of it (`rex.w pfadd %mm1,
/// %mm2` is `48 0f 0f d1 9e`). `notrack` and `bnd` are for the arms.
pub(crate) fn stands_behind(prefixes: &[String]) -> bool {
    prefixes.iter().all(|word| {
        matches!(
            word.as_str(),
            "rep"
                | "repe"
                | "repz"
                | "repne"
                | "repnz"
                | "lock"
                | "xacquire"
                | "xrelease"
                | "data16"
                | "data32"
                | "addr32"
                | "addr16"
                | "cs"
                | "ds"
                | "es"
                | "fs"
                | "gs"
                | "ss"
        ) || rex_prefix_word(word).is_some()
    })
}

/// Match `name ops` against the learned rows.
///
/// `None` when no row is called `name`; `Some(Err(..))`, in GNU as's words,
/// when rows are and none takes the operands.
pub(crate) fn plan<'a>(
    name: &str,
    ops: &'a [Operand],
    asked: Asked,
) -> Option<Result<Learned<'a>, String>> {
    let rows = rows_named(name)?;
    // A rounding word is an operand of its own; the rows count the others.
    let mut rounding: Option<(usize, &str)> = None;
    let mut operands: [Option<&Operand>; 5] = [None; 5];
    let mut count = 0;
    for op in ops {
        match op {
            Operand::Label(word) if super::registers::is_evex_sae_token(word) => {
                if rounding.is_some() {
                    return Some(Err(mismatch(name)));
                }
                rounding = Some((count, word.as_str()));
            }
            _ => {
                let Some(slot) = operands.get_mut(count) else {
                    return Some(Err(mismatch(name)));
                };
                *slot = Some(op);
                count += 1;
            }
        }
    }
    let mode = match asked.code {
        Code::Bits64 => M64,
        Code::Bits32 => M32,
        Code::Bits16 => M16,
    };
    let operands = &operands[..count];
    let row_for = |asked: Asked| {
        rows.iter()
            .filter(|row| row.flags & mode != 0)
            .find_map(|row| row.take(operands, rounding, asked))
    };
    if let Some(learned) = row_for(asked) {
        return Some(Ok(learned));
    }
    // No row. Where one takes the operands in another encoding than the
    // statement asks for, that is what is wrong with it, and GNU as's word.
    if asked.evex || asked.vex {
        let any = Asked {
            evex: false,
            vex: false,
            ..asked
        };
        if row_for(any).is_some() {
            let encoding = if asked.evex { "EVEX" } else { "VEX/XOP" };
            return Some(Err(format!("no {encoding} encoding for `{name}'")));
        }
    }
    Some(Err(mismatch(name)))
}

fn mismatch(name: &str) -> String {
    format!("operand type mismatch for `{name}'")
}

impl Row {
    fn arity(&self) -> usize {
        self.kinds
            .iter()
            .position(|&kind| kind == 0)
            .unwrap_or(self.kinds.len())
    }

    /// The fields of the statement's operands, if this is its row.
    fn take<'a>(
        &'static self,
        ops: &[Option<&'a Operand>],
        rounding: Option<(usize, &str)>,
        asked: Asked,
    ) -> Option<Learned<'a>> {
        if ops.len() != self.arity() {
            return None;
        }
        // The encoding the statement asks for.
        let evex = self.space == EVEX;
        if (asked.evex && !evex) || (asked.vex && !matches!(self.space, VEX | XOP)) {
            return None;
        }
        if self.flags & WHEN_VEX != 0 && !asked.vex {
            return None;
        }
        // REX2 stands in front of the one-byte opcodes and of the `0F`
        // map.
        if asked.rex2 && !(self.space == LEGACY && matches!(self.map, 0 | 1 | 4)) {
            return None;
        }
        let mut learned = Learned {
            row: self,
            reg: 0,
            rm: 0,
            memory: None,
            vvvv: 0,
            is4: 0,
            number: 0,
            mask: 0,
            zero: false,
            broadcast: false,
            rounding: None,
            rm_general: false,
            low_byte: false,
        };
        // What only EVEX says: a register beyond the sixteenth, a mask, a
        // broadcast, a rounding word.
        let mut decorated = rounding.is_some();
        let mut upper_vector = false;
        let mut egpr = false;
        let mut extended = false;
        let mut high_byte = false;
        // The widths of the registers of the address, and of the general
        // registers among the operands: a bit for each kind.
        let (mut address_kinds, mut general_kinds) = (0u32, 0u32);
        let mut numbers = [0u8; 5];
        let last = ops.len().wrapping_sub(1);
        for (at, (op, (&kind, &slot))) in ops
            .iter()
            .zip(self.kinds.iter().zip(&self.slots))
            .enumerate()
        {
            let op = (*op)?;
            // A mask stands on the last operand; a broadcast on the
            // address.
            let (mask, zeroing, broadcast) = match op {
                Operand::Register(reg) => (reg.mask.as_deref(), reg.zeroing, reg.broadcast),
                Operand::Memory(mem) => (mem.mask.as_deref(), mem.zeroing, mem.broadcast),
                _ => (None, false, None),
            };
            if mask.is_some() || zeroing {
                if at != last || self.flags & MASK == 0 {
                    return None;
                }
                // `%k0` is "no mask", and zeroing needs one.
                let &[b'k' | b'K', digit @ b'1'..=b'7'] = mask?.as_bytes() else {
                    return None;
                };
                let number = digit - b'0';
                if zeroing && self.flags & ZERO == 0 {
                    return None;
                }
                learned.mask = number;
                learned.zero = zeroing;
                decorated = true;
            }
            let number = match (kind, op) {
                (MEMORY, Operand::Memory(mem)) => {
                    match broadcast {
                        Some(count) if count == self.broadcast && count != 0 => {
                            learned.broadcast = true;
                            decorated = true;
                        }
                        Some(_) => return None,
                        None => {}
                    }
                    for part in [&mem.base, &mem.index].into_iter().flatten() {
                        match super::gp_id(&part.name) {
                            Some(number) => {
                                egpr |= number >= 16;
                                extended |= number >= 8;
                                if let Some((is, ..)) = classify(&part.name) {
                                    address_kinds |= 1 << is;
                                }
                            }
                            // `%rip`, or a vector index, which no row takes.
                            None if matches!(part.name.as_str(), "rip" | "eip") => {
                                address_kinds |= 1 << if part.name == "rip" { R64 } else { R32 };
                                // An address with a SIB byte has no place
                                // for the instruction pointer.
                                if self.flags & SIB != 0 {
                                    return None;
                                }
                            }
                            None => return None,
                        }
                    }
                    learned.memory = Some(mem);
                    0
                }
                (IMM8, Operand::Immediate(ImmediateValue::Integer(value))) => {
                    if !in_range(self.immediate, *value) || broadcast.is_some() {
                        return None;
                    }
                    learned.number = *value as u8;
                    0
                }
                (_, Operand::Register(reg)) if broadcast.is_none() => {
                    let (is, number, high) = classify(&reg.name)?;
                    if kind >= FIRST_FIXED {
                        if FIXED.get(usize::from(kind - FIRST_FIXED)) != Some(&reg.name.as_str()) {
                            return None;
                        }
                    } else if is != kind {
                        return None;
                    }
                    if number == 4 && !high && self.no_sp & (1 << at) != 0 {
                        return None;
                    }
                    if matches!(is, R16 | R32 | R64) {
                        general_kinds |= 1 << is;
                    }
                    if kind < FIRST_FIXED && slot != 0 {
                        match is {
                            XMM | YMM | ZMM => upper_vector |= number >= 16,
                            R8 | R16 | R32 | R64 => egpr |= number >= 16,
                            _ => {}
                        }
                        extended |= number >= 8;
                        learned.low_byte |= is == R8 && (4..8).contains(&number) && !high;
                        high_byte |= high;
                        if slot == RM {
                            learned.rm_general = matches!(is, R8 | R16 | R32 | R64);
                        }
                    }
                    number
                }
                _ => return None,
            };
            numbers[at] = number;
            match slot {
                REG => learned.reg = number,
                RM if kind != MEMORY => learned.rm = number,
                VVVV => learned.vvvv = number,
                IS4 => learned.is4 = number,
                _ => {}
            }
        }
        // An address in a register, and the address beside it: of one
        // width (`movdir64b (%eax), %rcx` is no statement).
        if self.flags & ADDRESSED != 0 && address_kinds != 0 && address_kinds != general_kinds {
            return None;
        }
        // Operands that must be different registers.
        if self.distinct != 0 {
            for first in 0..ops.len() {
                for second in first + 1..ops.len() {
                    if self.distinct & (1 << (first * 5 + second)) != 0
                        && numbers[first] == numbers[second]
                    {
                        return None;
                    }
                }
            }
        }
        // What the encoding can name.
        match self.space {
            LEGACY => {
                if upper_vector || (egpr && self.flags & EGPR == 0) {
                    return None;
                }
                // `%ah` to `%bh` are `%spl` to `%dil` behind a REX prefix.
                if high_byte && (extended || learned.low_byte || self.flags & W != 0) {
                    return None;
                }
            }
            VEX | XOP => {
                if upper_vector || egpr || high_byte {
                    return None;
                }
            }
            _ => {
                if (egpr && self.flags & EGPR == 0) || high_byte {
                    return None;
                }
            }
        }
        // The EVEX encoding of a form that has a shorter one is for the
        // statements that need it.
        if self.flags & WHEN_EVEX != 0 && !(asked.evex || upper_vector || egpr || decorated) {
            return None;
        }
        if asked.code != Code::Bits64 && (extended || upper_vector) {
            return None;
        }
        if self.digit >= 0 {
            learned.reg = self.digit as u8;
        }
        if self.fixed_rm >= 0 {
            learned.rm = self.fixed_rm as u8;
        }
        match rounding {
            None => {}
            Some((at, word)) => {
                if at != usize::from(self.rounding_at) || learned.memory.is_some() {
                    return None;
                }
                learned.rounding = Some(match super::registers::evex_sae_rounding(word)? {
                    None if self.flags & SAE != 0 => self.sae_l,
                    Some(mode) if self.flags & ROUND != 0 => mode,
                    _ => return None,
                });
            }
        }
        Some(learned)
    }
}

impl InstructionEncoder {
    /// Write the instruction `learned` describes.
    pub(crate) fn emit_learned(&mut self, learned: &Learned) -> Result<(), String> {
        let row = learned.row;
        let wide = row.flags & W != 0;
        let (reg, rm, vvvv) = (learned.reg, learned.rm, learned.vvvv);
        // The extension bits of the address: bit 3 and bit 4 of the index
        // and of the base.
        let (x3, b3, b4, x4) = match learned.memory {
            Some(mem) => Self::evex_addr_bits(mem),
            None => (false, rm & 8 != 0, false, false),
        };
        // The fifth bit of a register in `ModRM.rm`: `X` for a vector
        // register, `B4` for a general one.
        let rm_general = learned.rm_general;
        let rm_high = learned.memory.is_none() && rm & 16 != 0;
        match row.space {
            LEGACY => {
                if row.flags & DATA16 != 0 {
                    self.bytes.push(0x66);
                }
                match row.prefix {
                    1 => self.bytes.push(0x66),
                    2 => self.bytes.push(0xF3),
                    3 => self.bytes.push(0xF2),
                    _ => {}
                }
                self.emit_rex_or_rex2(
                    wide,
                    reg & 8 != 0,
                    x3,
                    b3,
                    reg & 16 != 0,
                    x4,
                    b4 || (rm_high && rm_general),
                    learned.low_byte,
                );
                match row.map {
                    0 => {}
                    1 => self.bytes.push(0x0F),
                    2 => self.bytes.extend_from_slice(&[0x0F, 0x38]),
                    3 => self.bytes.extend_from_slice(&[0x0F, 0x3A]),
                    _ => self.bytes.extend_from_slice(&[0x0F, 0x0F]),
                }
                if row.map != 4 {
                    self.bytes.push(row.opcode);
                }
            }
            VEX => {
                self.emit_vex(
                    reg & 8 != 0,
                    x3,
                    b3,
                    row.map,
                    u8::from(wide),
                    vvvv & 15,
                    row.l,
                    row.prefix,
                );
                self.bytes.push(row.opcode);
            }
            XOP => {
                let inverted = |bit: bool| u8::from(!bit);
                self.bytes.extend_from_slice(&[
                    0x8F,
                    inverted(reg & 8 != 0) << 7 | inverted(x3) << 6 | inverted(b3) << 5 | row.map,
                    u8::from(wide) << 7 | (!vvvv & 15) << 3 | row.l << 2 | row.prefix,
                    row.opcode,
                ]);
            }
            _ => {
                let (length, flagged) = match learned.rounding {
                    Some(length) => (length, true),
                    None => (row.l, learned.broadcast || row.flags & ND != 0),
                };
                self.emit_evex(
                    reg & 8 != 0,
                    x3 || (rm_high && !rm_general),
                    b3,
                    reg & 16 != 0,
                    row.map,
                    u8::from(wide),
                    vvvv & 15,
                    vvvv & 16 != 0,
                    row.prefix,
                    length,
                    learned.zero,
                    learned.mask,
                    flagged,
                    b4 || (rm_high && rm_general),
                    x4,
                );
                self.bytes.push(row.opcode);
            }
        }
        if row.flags & MODRM == 0 {
            self.bytes
                .extend_from_slice(&row.literal[..usize::from(row.literal_len.min(3))]);
            return Ok(());
        }
        match learned.memory {
            Some(mem) => {
                let at = self.bytes.len();
                let relocations = self.relocations.len();
                if row.space == EVEX {
                    let shift = if learned.broadcast {
                        row.broadcast_disp8
                    } else {
                        row.disp8
                    };
                    self.encode_evex_mem(reg & 7, mem, 1 << shift.min(6))?;
                } else {
                    self.encode_modrm_mem(reg & 7, mem)?;
                }
                if row.flags & SIB != 0 {
                    self.force_sib(at, relocations)?;
                }
            }
            None => self.bytes.push(0xC0 | (reg & 7) << 3 | (rm & 7)),
        }
        if let Ok(tail) = u8::try_from(row.tail) {
            self.bytes.push(tail);
        }
        let has = |wanted: u8| row.slots.contains(&wanted);
        if has(IS4) {
            let low = if has(LOW4) { learned.number & 15 } else { 0 };
            self.bytes.push(learned.is4 << 4 | low);
        } else if has(IMM) {
            self.bytes.push(learned.number);
        }
        if row.space == LEGACY && row.map == 4 {
            self.bytes.push(row.opcode);
        }
        Ok(())
    }

    /// Give the address whose ModRM byte stands at `at` a SIB byte if it
    /// has none (`tileloadd (%rbx), %tmm1` is `.. 0c 23`).
    fn force_sib(&mut self, at: usize, relocations: usize) -> Result<(), String> {
        let Some(&modrm) = self.bytes.get(at) else {
            return Err("internal: an address without a ModRM byte".to_string());
        };
        let base = modrm & 7;
        if base == 4 {
            return Ok(());
        }
        if modrm >> 6 == 0 && base == 5 {
            return Err(
                "an address relative to the instruction pointer cannot be used here".to_string(),
            );
        }
        self.bytes[at] = (modrm & !7) | 4;
        self.bytes.insert(at + 1, 0x20 | base);
        let moved = at as u64;
        for relocation in self.relocations.iter_mut().skip(relocations) {
            if relocation.offset > moved {
                relocation.offset += 1;
            }
        }
        if let Some(emission) = &mut self.memory_emission {
            if emission.start == at {
                emission.end += 1;
            }
        }
        Ok(())
    }

    /// Encode `name ops` from the learned rows. `None` when no row is
    /// called `name`; else whether the instruction chose its operand size
    /// (16-bit code inverts the operand-size prefix of one that did), or
    /// what the rows have against the operands.
    pub(crate) fn encode_learned(
        &mut self,
        name: &str,
        ops: &[Operand],
        asked: Asked,
    ) -> Option<Result<bool, String>> {
        let learned = match plan(name, ops, asked)? {
            Ok(learned) => learned,
            Err(refusal) => return Some(Err(refusal)),
        };
        // `{vex3}` is honoured by `emit_vex`, and `{vex2}` falls back to
        // three bytes where two cannot say the instruction.
        Some(self.emit_learned(&learned).map(|()| learned.sized()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_names_are_sorted_and_every_one_has_rows() {
        assert!(NAMES.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(STARTS.len(), NAMES.len() + 1);
        assert!(STARTS.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(usize::from(*STARTS.last().unwrap()), ROWS.len());
        for name in NAMES {
            assert!(!rows_named(name).unwrap().is_empty(), "{name}");
        }
        assert!(rows_named("mov").is_none());
    }

    #[test]
    fn every_row_is_one_the_emitter_knows() {
        for (at, row) in ROWS.iter().enumerate() {
            let arity = row.arity();
            assert!(
                row.kinds[..arity]
                    .iter()
                    .all(|&kind| { (R8..FIRST_FIXED + FIXED.len() as u8).contains(&kind) })
            );
            assert!(row.kinds[arity..].iter().all(|&kind| kind == 0), "row {at}");
            assert!(row.slots.iter().all(|&slot| slot <= LOW4), "row {at}");
            assert!(
                row.space <= XOP && row.prefix <= 3 && row.l <= 3,
                "row {at}"
            );
            assert!(row.flags & (M64 | M32 | M16) != 0, "row {at}");
            assert!(
                usize::from(row.literal_len) <= row.literal.len(),
                "row {at}"
            );
            assert!(row.disp8 <= 6 && row.broadcast_disp8 <= 6, "row {at}");
            // One operand at most in each field, and an address in `rm`.
            for field in [REG, RM, VVVV, IS4, IMM, LOW4] {
                assert!(
                    row.slots.iter().filter(|&&slot| slot == field).count() <= 1,
                    "row {at}"
                );
            }
            for (&kind, &slot) in row.kinds.iter().zip(&row.slots) {
                assert_eq!(
                    kind == MEMORY,
                    kind != 0 && slot == RM && kind > TMM,
                    "row {at}"
                );
                assert_eq!(kind == IMM8, matches!(slot, IMM | LOW4), "row {at}");
            }
            // The decorations are EVEX's.
            if row.space != EVEX {
                assert_eq!(row.flags & (MASK | ZERO | SAE | ROUND | ND), 0, "row {at}");
                assert_eq!(row.broadcast, 0, "row {at}");
            }
            assert!(row.flags & ZERO == 0 || row.flags & MASK != 0, "row {at}");
            assert!(row.flags & SAE == 0 || row.flags & ROUND == 0, "row {at}");
        }
    }

    #[test]
    fn a_register_is_classified_by_its_name() {
        assert_eq!(classify("xmm31"), Some((XMM, 31, false)));
        assert_eq!(classify("ymm7"), Some((YMM, 7, false)));
        assert_eq!(classify("zmm16"), Some((ZMM, 16, false)));
        assert_eq!(classify("tmm7"), Some((TMM, 7, false)));
        assert_eq!(classify("mm3"), Some((MM, 3, false)));
        assert_eq!(classify("k5"), Some((MASK_REGISTER, 5, false)));
        assert_eq!(classify("ah"), Some((R8, 4, true)));
        assert_eq!(classify("spl"), Some((R8, 4, false)));
        assert_eq!(classify("r17d"), Some((R32, 17, false)));
        assert_eq!(classify("r9w"), Some((R16, 9, false)));
        assert_eq!(classify("rbx"), Some((R64, 3, false)));
        for name in [
            "xmm32", "xmm01", "tmm8", "mm8", "k8", "mmx", "st", "cr0", "fs", "rip", "",
        ] {
            assert_eq!(classify(name), None, "{name}");
        }
    }
}
