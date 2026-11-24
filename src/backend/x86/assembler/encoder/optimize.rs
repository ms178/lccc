//! A cheaper instruction for the same effect.
//!
//! `movq $1, %rax` and `movl $1, %eax` leave the same value in `%rax`, in
//! seven bytes and in five. `leaq (%rsp), %rsi` and `movq %rsp, %rsi`,
//! `movslq %eax, %rax` and `cltq`, `movzwq %ax, %rax` and `movzwl %ax,
//! %eax`, `testl $1, %eax` and `testb $1, %al`, `movsd %xmm0, (%rdi)` and
//! `movlps %xmm0, (%rdi)`: [`cheaper`] is the second of each pair, for the
//! statements its caller may replace -- what the compiler wrote
//! (`assembler::generated`), and what a person wrote if `-Wa,-O` asks.
//!
//! # What a replacement has to be
//!
//! 1. **The same effect.** The same registers, memory and flags after it,
//!    and the same faults, on every processor that runs the statement. A
//!    flag the written instruction leaves undefined may come out defined,
//!    never the other way round (which is why `cmp $0, %eax` is not `test
//!    %eax, %eax`: `cmp` defines AF and `test` does not).
//! 2. **No new requirement.** It needs no instruction-set extension the
//!    written one does not need: `vpxord %zmm17, %zmm17, %zmm17` stays,
//!    because its 128-bit form needs AVX512VL, and `psllq $1, %mm0` stays,
//!    because `paddq` on `%mm` registers came with SSE2. (What needs
//!    AVX-512 has AVX and AVX2: no processor has the one without the
//!    others.)
//! 3. **Never longer**, and at the same length only where it is the faster
//!    one on some processor and the slower one on none ([`Gain`]).
//! 4. **Never slower**, unless size was asked for ([`Optimize::Smaller`],
//!    `-Wa,-Os`): `movzbw %cl, %cx` as `movb $0, %ch` is two bytes shorter
//!    and makes the next read of `%cx` wait for a merge.
//!
//! # Why each one is the same
//!
//! * A 32-bit operation zero-extends its result into the 64-bit register
//!   (SDM Vol. 1, 3.4.1.1), so a 64-bit operation whose result has no bit
//!   above 31 set, and whose flags do not look above bit 31, is the 32-bit
//!   one: `mov` of a 32-bit number, a zero extension, `xor` and `sub` of a
//!   register with itself, `and` and `test` with a mask that fits 31 bits,
//!   `bt` of a bit below 32.
//! * `test` with a mask that fits 7 bits looks at the low byte only, and
//!   its sign flag is clear at any width.
//! * `and` and `or` of a register with itself write back what was there and
//!   set the flags `test` sets. `test` does not write, and it fuses with
//!   the conditional branch behind it where `or` does not.
//! * `shl $1` is `add` of the register to itself: the same result, carry
//!   and overflow, and the one flag `shl` leaves undefined (AF) defined;
//!   `add` runs on more ports. `xadd` of a register with itself is that
//!   `add` too.
//! * `lea` computes and reads nothing; with one register and no
//!   displacement it copies, and a segment or an address-size prefix on it
//!   changes nothing it writes. A register copy is free on the processors
//!   that rename it away, which `lea` is not.
//! * An address is a sum: `(,%rax,2)` is `(%rax,%rax)` without the four
//!   displacement bytes an index without a base costs, and `(%r13,%rax)`
//!   is `(%rax,%r13)` without the displacement byte `%r13` costs as a
//!   base. (Not with `%rbp` for a base on either side: an address based on
//!   it is the stack segment's, and faults as such.)
//! * A store has no domain: `movdqu`, `movupd` and `movups` of a register
//!   to memory store the same sixteen bytes, `movq`, `movsd` and `movlps`
//!   the same eight, and the packed-single one has no prefix byte.
//!   Likewise the packed-double moves and bit operations between
//!   registers (`movapd`, `xorpd`, ...): no processor keeps single and
//!   double precision in separate domains. The integer ones (`movdqa`,
//!   `pxor`) are a domain of their own on some processors, and become the
//!   packed-single ones between registers only where size was asked for.
//! * A VEX- or EVEX-encoded instruction on `%xmm` registers zeroes the
//!   rest of the vector register (SDM Vol. 1, 14.1.2 and 15.5), so what
//!   clears a `%ymm` or `%zmm` register is its `%xmm` form, and the
//!   EVEX-only spelling of an instruction that has a VEX encoding is that
//!   instruction. What clears a register does not depend on which register
//!   it reads, so it is the exclusive-or -- the one every processor clears
//!   by without running anything -- of a register that needs no prefix
//!   byte.
//! * APX: a three-operand form whose destination is one of its sources is
//!   the two-operand instruction, where both are 32 or 64 bits wide (a
//!   narrower three-operand form clears the rest of the destination).
//!
//! Most of these are the replacements GNU as makes under `-O2` and `-Os`,
//! and `scripts/asm_optimize_sweep.py` holds this module to the shorter of
//! those two for every statement both assemblers take, and runs every
//! replacement against the instruction it replaces on the host's
//! processor. Where the two assemblers differ this module is the stricter
//! in what it requires (rule 2, and a relocation never changes its type)
//! and goes further in what it finds; `tests/asm-optimize/README.md` has
//! the list.
//!
//! The caller encodes the statement as written first, and keeps that
//! encoding unless the replacement encodes too and its [`Gain`] holds:
//! nothing here can refuse a statement or lengthen one.

use super::*;

/// What a replacement is for, which decides how its encoding is compared
/// with the written one's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Gain {
    /// Fewer bytes where it has fewer: kept only if its encoding is the
    /// shorter one.
    Shorter,
    /// Less time on some processor and more on none: kept unless its
    /// encoding is the longer one.
    Faster,
}

/// An encoder whose output can be taken back: what [`encode`] needs of the
/// 64-bit encoder and of the 32-bit one.
pub(crate) trait Atomic {
    /// How much has been written: bytes, relocations, location counter.
    fn mark(&self) -> (usize, usize, u64);
    /// Take back everything written since `mark`.
    fn rewind(&mut self, mark: (usize, usize, u64));
    /// Encode `instr` as written, leaving nothing behind if it is refused.
    fn written(&mut self, instr: &Instruction) -> Result<(), String>;
}

/// Encode `instr`: as written, or as the cheaper instruction that has the
/// same effect if the statement may be replaced (`Instruction::optimize`).
/// Returns the instruction that replaced it, if one did.
///
/// The statement is encoded as written first, so an invalid one is refused
/// for what it is. Of the replacements ([`cheaper`]) the shortest encoding
/// is kept, and of two as short the later one if it is the faster.
pub(crate) fn encode<E: Atomic>(
    encoder: &mut E,
    instr: &Instruction,
    long: bool,
) -> Result<Option<Instruction>, String> {
    let start = encoder.mark();
    encoder.written(instr)?;
    if instr.optimize == Optimize::Never {
        return Ok(None);
    }
    let replacements = cheaper(instr, long);
    if replacements.is_empty() {
        return Ok(None);
    }
    let mut shortest = encoder.mark().0 - start.0;
    let mut kept = None;
    // Whether a replacement since the kept one was the faster kind.
    let mut faster = false;
    for (replacement, gain) in replacements {
        encoder.rewind(start);
        if encoder.written(&replacement).is_err() {
            break;
        }
        let length = encoder.mark().0 - start.0;
        faster |= gain == Gain::Faster;
        if length < shortest || (length == shortest && faster) {
            shortest = length;
            kept = Some(replacement);
            faster = false;
        }
    }
    encoder.rewind(start);
    encoder.written(kept.as_ref().unwrap_or(instr))?;
    Ok(kept)
}

/// The general registers by number, in each width: byte, word, doubleword,
/// quadword. (`%ah`..`%bh` have no number of their own and are not here.)
const GENERAL: [[&str; 4]; 32] = [
    ["al", "ax", "eax", "rax"],
    ["cl", "cx", "ecx", "rcx"],
    ["dl", "dx", "edx", "rdx"],
    ["bl", "bx", "ebx", "rbx"],
    ["spl", "sp", "esp", "rsp"],
    ["bpl", "bp", "ebp", "rbp"],
    ["sil", "si", "esi", "rsi"],
    ["dil", "di", "edi", "rdi"],
    ["r8b", "r8w", "r8d", "r8"],
    ["r9b", "r9w", "r9d", "r9"],
    ["r10b", "r10w", "r10d", "r10"],
    ["r11b", "r11w", "r11d", "r11"],
    ["r12b", "r12w", "r12d", "r12"],
    ["r13b", "r13w", "r13d", "r13"],
    ["r14b", "r14w", "r14d", "r14"],
    ["r15b", "r15w", "r15d", "r15"],
    ["r16b", "r16w", "r16d", "r16"],
    ["r17b", "r17w", "r17d", "r17"],
    ["r18b", "r18w", "r18d", "r18"],
    ["r19b", "r19w", "r19d", "r19"],
    ["r20b", "r20w", "r20d", "r20"],
    ["r21b", "r21w", "r21d", "r21"],
    ["r22b", "r22w", "r22d", "r22"],
    ["r23b", "r23w", "r23d", "r23"],
    ["r24b", "r24w", "r24d", "r24"],
    ["r25b", "r25w", "r25d", "r25"],
    ["r26b", "r26w", "r26d", "r26"],
    ["r27b", "r27w", "r27d", "r27"],
    ["r28b", "r28w", "r28d", "r28"],
    ["r29b", "r29w", "r29d", "r29"],
    ["r30b", "r30w", "r30d", "r30"],
    ["r31b", "r31w", "r31d", "r31"],
];

/// Number and width in bytes of a general register that has a number of
/// its own: not `%ah`..`%bh`, and not `%rip` or `%riz`.
fn general(name: &str) -> Option<(u8, u8)> {
    if matches!(name, "ah" | "ch" | "dh" | "bh") {
        return None;
    }
    match reg_class(name) {
        RegClass::Gp(width) => Some((gp_id(name)?, width)),
        _ => None,
    }
}

/// The general register `number` in the width `bytes`.
fn named(number: u8, bytes: u8) -> Option<Operand> {
    let column = match bytes {
        1 => 0,
        2 => 1,
        4 => 2,
        8 => 3,
        _ => return None,
    };
    let name = GENERAL.get(usize::from(number))?.get(column)?;
    Some(Operand::Register(Register::new(name)))
}

/// Width (1 `%xmm`, 2 `%ymm`, 3 `%zmm`) and number of a vector register.
fn vector(name: &str) -> Option<(u8, u8)> {
    let width = match name.as_bytes().first()? {
        b'x' => 1,
        b'y' => 2,
        b'z' => 3,
        _ => return None,
    };
    Some((width, vec_reg_id(name)?))
}

/// The vector register `number` in `width`, undecorated.
fn vector_named(width: u8, number: u8) -> Operand {
    let letter = match width {
        1 => 'x',
        2 => 'y',
        _ => 'z',
    };
    Operand::Register(Register::new(&format!("{letter}mm{number}")))
}

/// `register` in another width, its mask and zeroing kept.
fn resized(register: &Register, width: u8) -> Option<Operand> {
    let Operand::Register(plain) = vector_named(width, vec_reg_id(&register.name)?) else {
        return None;
    };
    Some(Operand::Register(Register {
        name: plain.name,
        ..register.clone()
    }))
}

/// A mnemonic as a stem of `stems` and the operand width its suffix says,
/// if it has one: `andq` is (`and`, 8).
fn suffixed<'a>(mnemonic: &str, stems: &[&'a str]) -> Option<(&'a str, Option<u8>)> {
    stems.iter().find_map(|&stem| {
        let width = match mnemonic.strip_prefix(stem)? {
            "" => None,
            "b" => Some(1),
            "w" => Some(2),
            "l" => Some(4),
            "q" => Some(8),
            _ => return None,
        };
        Some((stem, width))
    })
}

/// `stem` with the suffix of `bytes`.
fn spelled(stem: &str, bytes: u8) -> String {
    let suffix = match bytes {
        1 => "b",
        2 => "w",
        4 => "l",
        _ => "q",
    };
    format!("{stem}{suffix}")
}

/// `instr` as another mnemonic on other operands, without its prefixes.
fn rewritten(instr: &Instruction, mnemonic: &str, operands: Vec<Operand>) -> Instruction {
    Instruction {
        prefixes: Vec::new(),
        mnemonic: mnemonic.to_string(),
        operands,
        ..instr.clone()
    }
}

fn number(value: i64) -> Operand {
    Operand::Immediate(ImmediateValue::Integer(value))
}

fn register(register: &Register) -> Operand {
    Operand::Register(register.clone())
}

fn shorter(instr: Instruction) -> Option<(Instruction, Gain)> {
    Some((instr, Gain::Shorter))
}

fn faster(instr: Instruction) -> Option<(Instruction, Gain)> {
    Some((instr, Gain::Faster))
}

/// Whether `operand` is the register `other`, undecorated.
fn is_register(operand: &Operand, other: &Register) -> bool {
    matches!(operand, Operand::Register(register)
        if register.name == other.name && register.mask.is_none())
}

/// The cheaper instructions for `instr` this module knows, each made from
/// the one before and with what it gains over that one: `vextracti32x4
/// $0` is `vmovdqa32`, which is `vmovdqa`. `long` is 64-bit code.
///
/// None for a statement that asks for an encoding (`{evex}`, `{vex3}`,
/// `{rex2}`) or carries a prefix: what it asked for is then part of what
/// it is. (But `lock` on an `xchg` with memory, which locks without it.)
/// A statement that writes no flags (`{nf}`) has replacements of its own
/// ([`flagless`]).
pub(crate) fn cheaper(instr: &Instruction, long: bool) -> Vec<(Instruction, Gain)> {
    let mut chain = Vec::new();
    if instr.optimize == Optimize::Never
        || instr.force_evex
        || instr.force_rex2
        || instr.vex_hint.is_some()
    {
        return chain;
    }
    // A bare symbol is an address (`movsd %xmm0, counter`), which the
    // parser cannot tell from a branch target and leaves a label; the
    // rules below know memory operands.
    let addressed;
    let instr = if !mnemonic_takes_label(&instr.mnemonic)
        && instr
            .operands
            .iter()
            .any(|operand| matches!(operand, Operand::Label(name) if !name.starts_with('{')))
    {
        let mut copy = instr.clone();
        copy.operands.iter_mut().for_each(normalize_bare_label);
        addressed = copy;
        &addressed
    } else {
        instr
    };
    if !instr.prefixes.is_empty() {
        let locked_exchange = matches!(instr.prefixes.as_slice(), [prefix] if prefix == "lock")
            && suffixed(&instr.mnemonic, &["xchg"]).is_some()
            && instr
                .operands
                .iter()
                .any(|o| matches!(o, Operand::Memory(_)));
        if locked_exchange {
            chain.push((
                rewritten(instr, &instr.mnemonic, instr.operands.clone()),
                Gain::Shorter,
            ));
        }
        return chain;
    }
    let step = |instr: &Instruction| {
        if instr.nf {
            flagless(instr, long)
        } else {
            step(instr, long)
        }
    };
    let mut next = step(instr);
    while let Some(link) = next {
        next = (chain.len() < 3).then(|| step(&link.0)).flatten();
        chain.push(link);
    }
    chain
}

/// The mnemonics that have a three-operand APX form, without their
/// suffixes. `shld` and `shrd` before `shl` and `shr`, `adcx` before
/// `adc`: [`suffixed`] takes the first that fits.
const TWO_OPERAND: &[&str] = &[
    "add", "or", "adcx", "adox", "adc", "sbb", "and", "sub", "xor", "imul", "inc", "dec", "not",
    "neg", "rol", "ror", "rcl", "rcr", "shld", "shrd", "shl", "sal", "shr", "sar",
];

/// Whether a statement of this mnemonic and these operands, as the code
/// generator writes them (`movq $1, %rax`: one space behind the mnemonic,
/// operands apart by a comma and a space, nothing inside one), can have a
/// replacement: what `assembler::generated` asks of every line before it
/// parses one. It may say yes to a statement that has none; it does not
/// say no to one that has (`candidate_admits_every_rewrite`).
pub(crate) fn candidate(mnemonic: &str, operands: &str, long: bool, optimize: Optimize) -> bool {
    let smaller = optimize == Optimize::Smaller;
    let text = operands.as_bytes();
    // `$5, ...`: a number written out, and one that is not negative.
    let numeric = text.first() == Some(&b'$')
        && text
            .get(1)
            .is_some_and(|c| c.is_ascii_digit() || *c == b'-');
    let natural = numeric && text.get(1) != Some(&b'-');
    // `..., %rax`: the last operand is a register.
    let to_register = || {
        operands.rsplit_once(',').is_some_and(|(_, last)| {
            let last = last.trim_start();
            last.starts_with('%') && !last.contains(':') && !last.ends_with(')')
        })
    };
    // `%xmm0, 8(%rsp)`: a vector register to memory.
    let store = || text.starts_with(b"%xmm") && !to_register();
    // `%eax, %eax`: one register twice, and nothing else.
    let twice = || {
        operands
            .split_once(',')
            .is_some_and(|(first, second)| first.starts_with('%') && first.trim() == second.trim())
    };
    // Three operands or more: commas outside parentheses.
    let several = || {
        let mut depth = 0_i32;
        let mut commas = 0;
        for byte in text {
            match byte {
                b'(' => depth += 1,
                b')' => depth -= 1,
                b',' if depth == 0 => commas += 1,
                _ => {}
            }
        }
        commas >= 2
    };
    let Some(&first) = mnemonic.as_bytes().first() else {
        return false;
    };
    let by_name = match first {
        b'a' => {
            matches!(mnemonic, "andpd" | "andnpd")
                || (mnemonic.starts_with("and") && (natural && to_register() || twice()))
        }
        b'b' => mnemonic.starts_with("bt") && natural,
        b'c' => long && mnemonic.starts_with("ctest") && natural && to_register(),
        b'e' => mnemonic == "extractps",
        b'i' => mnemonic == "insertps",
        b'k' => mnemonic.starts_with("kxor") || mnemonic.starts_with("kandn"),
        b'l' => {
            (mnemonic.starts_with("lea") && mnemonic != "leave" && lea_candidate(operands, long))
                || (mnemonic == "lock" && operands.starts_with("xchg"))
        }
        b'm' => match mnemonic {
            "mov" => natural && to_register(),
            "movq" => natural && to_register() || store(),
            "movabs" | "movabsq" => numeric,
            "movslq" | "movsxd" | "movsx" | "movswl" | "movsbw" => {
                text.starts_with(b"%eax,") || text.starts_with(b"%ax,") || text.starts_with(b"%al,")
            }
            "movzbq" | "movzwq" => long,
            "movzb" | "movzw" | "movzx" => long || smaller,
            "movzbw" => smaller,
            "movapd" | "movupd" | "movlpd" | "movhpd" => true,
            "movdqa" | "movdqu" => smaller || store(),
            "movsd" => store(),
            "movddup" => twice(),
            "movbe" | "movbel" | "movbeq" => long && twice(),
            _ => false,
        },
        b'o' => mnemonic == "orpd" || (mnemonic.starts_with("or") && twice()),
        b'p' => match mnemonic {
            "pxor" | "pand" | "por" => smaller,
            "pandn" => smaller || twice(),
            _ => {
                (mnemonic.starts_with("psll") && text.starts_with(b"$1,"))
                    || (mnemonic.starts_with("pcmp") && twice())
                    || (mnemonic.starts_with("pextr")
                        && (text.starts_with(b"$0,") || text.starts_with(b"$1,")))
            }
        },
        b's' => {
            (mnemonic.starts_with("sub") && twice())
                || ((mnemonic.starts_with("shl") || mnemonic.starts_with("sal"))
                    && (text.starts_with(b"$1,")
                        || !text.contains(&b',')
                        || (long && text.starts_with(b"%") && !text.starts_with(b"%cl,"))))
        }
        b't' => mnemonic.starts_with("test") && mnemonic != "testb" && natural && to_register(),
        b'u' => mnemonic == "unpcklpd",
        b'v' => true,
        b'x' => {
            mnemonic == "xorpd"
                || ((mnemonic.starts_with("xor") || mnemonic.starts_with("xadd")) && twice())
        }
        _ => false,
    };
    by_name
        || address_candidate(operands)
        || (long
            && (suffixed(mnemonic, TWO_OPERAND).is_some()
                || mnemonic.starts_with("cmov")
                || mnemonic.starts_with("cfcmov"))
            && (several() || twice()))
}

/// [`candidate`] for a statement behind `{nf}`.
pub(crate) fn flagless_candidate(mnemonic: &str) -> bool {
    suffixed(mnemonic, FLAGLESS).is_some()
}

/// Whether an address in `operands` can be written shorter: twice an
/// index without a base, or a base that costs a displacement byte.
fn address_candidate(operands: &str) -> bool {
    operands.find('(').is_some_and(|open| {
        let inner = &operands[open + 1..];
        (inner.starts_with(',') && inner.contains(",2)"))
            || inner.starts_with("%r13,")
            || inner.starts_with("%r21,")
            || inner.starts_with("%r29,")
    })
}

/// Whether the operands of a `lea` can be those of one that has a
/// replacement: a segment, no register at all, one register and nothing
/// else, an index without a base, a base that costs a displacement byte,
/// or (in 64-bit code) a register narrower than the address.
fn lea_candidate(operands: &str, long: bool) -> bool {
    let Some(open) = operands.find('(') else {
        return true;
    };
    let (displacement, rest) = operands.split_at(open);
    let Some(close) = rest.find(')') else {
        return true;
    };
    let inner = &rest[1..close];
    if displacement.contains(':') || inner.starts_with(',') {
        return true;
    }
    // `%eax`, `%r8d`.
    let narrow = |part: &str| {
        let part = part.trim();
        long && (part.starts_with("%e") || part.ends_with('d'))
    };
    match inner.split_once(',') {
        None => matches!(displacement.trim(), "" | "0") || narrow(inner),
        Some((base, index)) => {
            matches!(base, "%rbp" | "%ebp" | "%r13" | "%r21" | "%r29")
                || narrow(base)
                || narrow(index.split(',').next().unwrap_or(""))
        }
    }
}

fn step(instr: &Instruction, long: bool) -> Option<(Instruction, Gain)> {
    let mnemonic = instr.mnemonic.as_str();
    let operands = instr.operands.as_slice();
    let by_name = match *mnemonic.as_bytes().first()? {
        b'a' | b'o' => {
            logic(instr, mnemonic, operands, long).or_else(|| packed(instr, mnemonic, operands))
        }
        b'b' => bit_test(instr, mnemonic, operands, long),
        b'c' => test(instr, mnemonic, operands, long),
        b'e' | b'i' | b'p' | b'u' => packed(instr, mnemonic, operands),
        b'k' => mask(instr, mnemonic, operands),
        b'l' => lea(instr, mnemonic, operands, long),
        b'm' => {
            moves(instr, mnemonic, operands, long).or_else(|| packed(instr, mnemonic, operands))
        }
        b's' => arithmetic(instr, mnemonic, operands, long),
        b'x' => arithmetic(instr, mnemonic, operands, long)
            .or_else(|| packed(instr, mnemonic, operands)),
        b't' => test(instr, mnemonic, operands, long),
        b'v' => vex(instr, mnemonic, operands),
        _ => None,
    };
    by_name
        .or_else(|| {
            long.then(|| two_operand(instr, mnemonic, operands))
                .flatten()
        })
        .or_else(|| readdress(instr, mnemonic, operands))
}

/// `mov` of a number that fits 32 bits into a 64-bit register; the sign
/// extensions of the accumulator that have a name of their own; the zero
/// extensions into a 64-bit register, and (for size) of a byte register
/// into its own word.
fn moves(
    instr: &Instruction,
    mnemonic: &str,
    operands: &[Operand],
    long: bool,
) -> Option<(Instruction, Gain)> {
    match (mnemonic, operands) {
        (
            "mov" | "movq" | "movabs" | "movabsq",
            [
                Operand::Immediate(ImmediateValue::Integer(value)),
                Operand::Register(to),
            ],
        ) if long => {
            let (id, 8) = general(&to.name)? else {
                return None;
            };
            if (0..=0xFFFF_FFFF_i64).contains(value) {
                return shorter(rewritten(
                    instr,
                    "movl",
                    vec![number(*value), named(id, 4)?],
                ));
            }
            // Ten bytes for a number the sign-extending form has in seven.
            (mnemonic.starts_with("movabs") && i32::try_from(*value).is_ok())
                .then(|| rewritten(instr, "movq", operands.to_vec()))
                .and_then(shorter)
        }
        ("movzbq" | "movzwq" | "movzb" | "movzw" | "movzx", [from, Operand::Register(to)])
            if long && general(&to.name).is_some_and(|(_, width)| width == 8) =>
        {
            // The width of what is read: said by the mnemonic, or by the
            // register (`movzx` and its kin read a register only).
            let read = match (mnemonic, from) {
                ("movzbq", Operand::Register(_) | Operand::Memory(_)) => 1,
                ("movzwq", Operand::Register(_) | Operand::Memory(_)) => 2,
                (_, Operand::Register(from)) => match (mnemonic, general_or_high(&from.name)?) {
                    ("movzb" | "movzx", 1) => 1,
                    ("movzw" | "movzx", 2) => 2,
                    _ => return None,
                },
                _ => return None,
            };
            let (id, _) = general(&to.name)?;
            let name = if read == 1 { "movzbl" } else { "movzwl" };
            shorter(rewritten(instr, name, vec![from.clone(), named(id, 4)?]))
        }
        // APX's `movbe` between registers, of a register onto itself: the
        // swap in place, which every processor since the 486 has.
        ("movbe" | "movbel" | "movbeq", [Operand::Register(from), Operand::Register(to)])
            if long && from.name == to.name =>
        {
            let (_, width @ (4 | 8)) = general(&to.name)? else {
                return None;
            };
            shorter(rewritten(
                instr,
                &spelled("bswap", width),
                vec![register(to)],
            ))
        }
        (_, [Operand::Register(from), Operand::Register(to)]) => {
            let (from, to) = (from.name.as_str(), to.name.as_str());
            let name = match (mnemonic, from, to) {
                ("movslq" | "movsxd" | "movsx", "eax", "rax") if long => "cltq",
                ("movswl" | "movsx", "ax", "eax") => "cwtl",
                ("movsbw" | "movsx", "al", "ax") => "cbtw",
                // Two bytes for four, and a write to a high-byte register
                // that the next read of the whole one has to merge.
                (
                    "movzbw" | "movzb" | "movzx",
                    "al" | "cl" | "dl" | "bl",
                    "ax" | "cx" | "dx" | "bx",
                ) if instr.optimize == Optimize::Smaller && gp_id(from) == gp_id(to) => {
                    let high = ["ah", "ch", "dh", "bh"].get(usize::from(gp_id(from)?))?;
                    return shorter(rewritten(
                        instr,
                        "movb",
                        vec![number(0), Operand::Register(Register::new(high))],
                    ));
                }
                _ => return None,
            };
            shorter(rewritten(instr, name, Vec::new()))
        }
        _ => None,
    }
}

/// Width in bytes of a general register, `%ah`..`%bh` included.
fn general_or_high(name: &str) -> Option<u8> {
    match reg_class(name) {
        RegClass::Gp(width) => Some(width),
        _ => None,
    }
}

/// `xor` and `sub` of a 64-bit register with itself, `xadd` of a register
/// with itself, `shl $1`.
fn arithmetic(
    instr: &Instruction,
    mnemonic: &str,
    operands: &[Operand],
    long: bool,
) -> Option<(Instruction, Gain)> {
    let (stem, suffix) = suffixed(mnemonic, &["xor", "sub", "xadd", "shl", "sal"])?;
    let operand = match (stem, operands) {
        // A shift by one, its count written or not.
        (
            "shl" | "sal",
            [
                Operand::Immediate(ImmediateValue::Integer(1)),
                Operand::Register(operand),
            ]
            | [Operand::Register(operand)],
        ) => operand,
        ("xor" | "sub" | "xadd", [Operand::Register(first), Operand::Register(second)])
            if first.name == second.name =>
        {
            first
        }
        // APX: twice another register's value.
        (
            "shl" | "sal",
            [
                Operand::Immediate(ImmediateValue::Integer(1)),
                Operand::Register(from),
                Operand::Register(to),
            ]
            | [Operand::Register(from), Operand::Register(to)],
        ) if long && from.name != "cl" => {
            let ((_, width), (_, to_width)) = (general(&from.name)?, general(&to.name)?);
            if width != to_width || suffix.is_some_and(|said| said != width) {
                return None;
            }
            return faster(rewritten(
                instr,
                &spelled("add", width),
                vec![register(from), register(from), register(to)],
            ));
        }
        // APX: nothing, into a third register, all of which a
        // three-operand form writes at any width.
        (
            "xor" | "sub",
            [
                Operand::Register(first),
                Operand::Register(second),
                Operand::Register(to),
            ],
        ) if long && first.name == second.name => {
            let ((_, width), (id, to_width)) = (general(&first.name)?, general(&to.name)?);
            if width != to_width || suffix.is_some_and(|said| said != width) {
                return None;
            }
            let to = named(id, 4)?;
            return faster(rewritten(instr, &spelled(stem, 4), vec![to.clone(), to]));
        }
        _ => return None,
    };
    let RegClass::Gp(width) = reg_class(&operand.name) else {
        return None;
    };
    if suffix.is_some_and(|said| said != width) {
        return None;
    }
    let twice = |operand: Operand| vec![operand.clone(), operand];
    match stem {
        "xor" | "sub" => {
            let (id, 8) = general(&operand.name)? else {
                return None;
            };
            if !long {
                return None;
            }
            // In `%r8`..`%r15` no byte is saved, and the 32-bit one is
            // still the one every processor knows to depend on nothing.
            faster(rewritten(instr, &spelled(stem, 4), twice(named(id, 4)?)))
        }
        _ => faster(rewritten(
            instr,
            &spelled("add", width),
            twice(register(operand)),
        )),
    }
}

/// `and` of a 64-bit register with a mask that fits 31 bits; `and` and
/// `or` of a register with itself.
fn logic(
    instr: &Instruction,
    mnemonic: &str,
    operands: &[Operand],
    long: bool,
) -> Option<(Instruction, Gain)> {
    let (stem, suffix) = suffixed(mnemonic, &["and", "or"])?;
    match operands {
        [
            Operand::Immediate(ImmediateValue::Integer(mask)),
            Operand::Register(to),
        ] if long && stem == "and" && suffix.is_none_or(|said| said == 8) => {
            let (id, 8) = general(&to.name)? else {
                return None;
            };
            if !(0..=0x7FFF_FFFF_i64).contains(mask) {
                return None;
            }
            shorter(rewritten(instr, "andl", vec![number(*mask), named(id, 4)?]))
        }
        [Operand::Register(first), Operand::Register(second)] if first.name == second.name => {
            let RegClass::Gp(width) = reg_class(&first.name) else {
                return None;
            };
            // In 64-bit code the 32-bit `and` clears the upper half of
            // the register, which `test` does not.
            if suffix.is_some_and(|said| said != width) || (long && width == 4) {
                return None;
            }
            faster(rewritten(instr, &spelled("test", width), operands.to_vec()))
        }
        _ => None,
    }
}

/// `test` of a register with a mask that fits 7 bits, or 31.
fn test(
    instr: &Instruction,
    mnemonic: &str,
    operands: &[Operand],
    long: bool,
) -> Option<(Instruction, Gain)> {
    // `test`, or APX's `ctest` on a condition.
    let (name, suffix) = match suffixed(mnemonic, &["test"]) {
        Some((name, suffix)) => (name, suffix),
        None => conditional_test(mnemonic)?,
    };
    let [
        Operand::Immediate(ImmediateValue::Integer(mask)),
        Operand::Register(operand),
    ] = operands
    else {
        return None;
    };
    let (id, width) = general(&operand.name)?;
    if width == 1 || suffix.is_some_and(|said| said != width) {
        return None;
    }
    // The low byte of every register has a name in 64-bit code; outside
    // it only the first four do.
    if (0..=0x7F).contains(mask) && (long || id < 4) {
        return shorter(rewritten(
            instr,
            &format!("{name}b"),
            vec![number(*mask), named(id, 1)?],
        ));
    }
    if name == "test" && width == 8 && (0..=0x7FFF_FFFF_i64).contains(mask) {
        return shorter(rewritten(
            instr,
            "testl",
            vec![number(*mask), named(id, 4)?],
        ));
    }
    None
}

/// `ctest` on a condition, as the mnemonic without its suffix and the
/// width the suffix says: `ctestbel` is (`ctestbe`, 4), `ctestb` the
/// condition `b`.
fn conditional_test(mnemonic: &str) -> Option<(&str, Option<u8>)> {
    let rest = mnemonic.strip_prefix("ctest")?;
    let known = |condition: &str| {
        matches!(
            condition,
            "o" | "no"
                | "b"
                | "c"
                | "nae"
                | "ae"
                | "nb"
                | "nc"
                | "e"
                | "z"
                | "ne"
                | "nz"
                | "be"
                | "na"
                | "a"
                | "nbe"
                | "s"
                | "ns"
                | "t"
                | "f"
                | "l"
                | "nge"
                | "ge"
                | "nl"
                | "le"
                | "ng"
                | "g"
                | "nle"
        )
    };
    if known(rest) {
        return Some((mnemonic, None));
    }
    let (condition, suffix) = rest.split_at(rest.len().checked_sub(1)?);
    let width = match suffix {
        "b" => 1,
        "w" => 2,
        "l" => 4,
        "q" => 8,
        _ => return None,
    };
    known(condition).then(|| (mnemonic.split_at(mnemonic.len() - 1).0, Some(width)))
}

/// `bt` of a bit below 32 in a 64-bit register; `bt` of a bit of a 16-bit
/// register, and outside 64-bit code `bts`, `btr` and `btc` of one: the
/// 32-bit instruction changes the same bit, and has no prefix.
fn bit_test(
    instr: &Instruction,
    mnemonic: &str,
    operands: &[Operand],
    long: bool,
) -> Option<(Instruction, Gain)> {
    let (stem, suffix) = suffixed(mnemonic, &["bts", "btr", "btc", "bt"])?;
    let [
        Operand::Immediate(ImmediateValue::Integer(bit)),
        Operand::Register(operand),
    ] = operands
    else {
        return None;
    };
    let (id, width) = general(&operand.name)?;
    if suffix.is_some_and(|said| said != width) {
        return None;
    }
    // What writes the register writes all 64 bits of it in 64-bit code.
    let reads = stem == "bt";
    let narrower = match width {
        8 => reads && long && (0..32).contains(bit),
        2 => (reads || !long) && (0..16).contains(bit),
        _ => false,
    };
    if !narrower {
        return None;
    }
    shorter(rewritten(
        instr,
        &spelled(stem, 4),
        vec![number(*bit), named(id, 4)?],
    ))
}

/// `lea` that copies a register or a number, or carries a prefix or a
/// displacement it has no use for.
fn lea(
    instr: &Instruction,
    mnemonic: &str,
    operands: &[Operand],
    long: bool,
) -> Option<(Instruction, Gain)> {
    let (_, suffix) = suffixed(mnemonic, &["lea"])?;
    let [Operand::Memory(address), Operand::Register(to)] = operands else {
        return None;
    };
    let (to_id, to_width) = general(&to.name)?;
    if to_width == 1
        || suffix.is_some_and(|said| said != to_width)
        || address.mask.is_some()
        || address.broadcast.is_some()
    {
        return None;
    }
    let readdressed = |address: MemoryOperand| {
        shorter(rewritten(
            instr,
            mnemonic,
            vec![Operand::Memory(address), register(to)],
        ))
    };
    // A segment: `lea` reads no memory.
    let unsegmented = || {
        address.segment.as_ref().and_then(|_| {
            readdressed(MemoryOperand {
                segment: None,
                ..address.clone()
            })
        })
    };
    // The registers of the address, each a general one. With `%rip` or
    // `%riz` in it, only a segment can go.
    let part = |part: &Option<Register>| match part {
        Some(part) => general(&part.name).map(Some),
        None => Some(None),
    };
    let (Some(base), Some(index)) = (part(&address.base), part(&address.index)) else {
        return unsegmented();
    };
    let scale = address.scale.unwrap_or(1);
    let plain = matches!(
        address.displacement,
        Displacement::None | Displacement::Integer(0)
    );
    // One register and nothing else: a copy, as wide as the narrower of
    // the address and the destination, zero-extended.
    let lone = match (base, index) {
        (Some(base), None) => Some(base),
        (None, Some(index)) if scale == 1 => Some(index),
        _ => None,
    };
    if let (Some((from_id, address_width)), true) = (lone, plain) {
        if address_width == 2 {
            // A 16-bit address, which is the slow kind of `lea`.
            let name = match to_width {
                2 => "movw",
                4 => "movzwl",
                _ => return None,
            };
            return faster(rewritten(
                instr,
                name,
                vec![named(from_id, 2)?, register(to)],
            ));
        }
        // A 32-bit copy clears the upper half of a 64-bit destination, as
        // a 32-bit address does.
        let width = address_width.min(to_width);
        return faster(rewritten(
            instr,
            &spelled("mov", width),
            vec![named(from_id, width)?, named(to_id, width)?],
        ));
    }
    if let (None, None) = (base, index) {
        return absolute(instr, &address.displacement, to, to_id, to_width, long);
    }
    // 32-bit registers in a 64-bit program's address: the address-size
    // prefix goes if the result is 32 bits wide either way.
    let narrow = |part: Option<(u8, u8)>| part.is_none_or(|(_, width)| width == 4);
    if long && narrow(base) && narrow(index) {
        let displacement = match &address.displacement {
            Displacement::None => Displacement::None,
            // The same number modulo 2^32, as 64-bit addressing has it.
            Displacement::Integer(value) if (-0x8000_0000..=0xFFFF_FFFF_i64).contains(value) => {
                Displacement::Integer(i64::from(*value as u32 as i32))
            }
            // A symbol's relocation is the 32-bit one without a sign
            // either way: for the prefix as written, for the narrow
            // destination without it.
            Displacement::Symbol(_)
            | Displacement::SymbolPlusOffset(..)
            | Displacement::SymbolAddend(..) => address.displacement.clone(),
            _ => return None,
        };
        let widened = |part: &Option<Register>| -> Option<Option<Register>> {
            match part {
                Some(part) => {
                    let (id, _) = general(&part.name)?;
                    let Operand::Register(wide) = named(id, 8)? else {
                        return None;
                    };
                    Some(Some(wide))
                }
                None => Some(None),
            }
        };
        let address = MemoryOperand {
            segment: None,
            displacement,
            base: widened(&address.base)?,
            index: widened(&address.index)?,
            ..address.clone()
        };
        let (name, to) = if to_width == 8 {
            ("leal", named(to_id, 4)?)
        } else {
            (mnemonic, register(to))
        };
        return shorter(rewritten(instr, name, vec![Operand::Memory(address), to]));
    }
    // The same sum the shorter way. Twice a register plus a displacement
    // is three parts for two, which is the slower `lea` on some
    // processors: for size only.
    if let Some(better) = resummed(address, true) {
        let third = better.base.is_some()
            && !matches!(better.displacement, Displacement::None)
            && address.base.is_none();
        if !third || instr.optimize == Optimize::Smaller {
            return readdressed(MemoryOperand {
                segment: None,
                ..better
            });
        }
    }
    unsegmented()
}

/// `lea` of an address without a register: a number, or a symbol.
fn absolute(
    instr: &Instruction,
    displacement: &Displacement,
    to: &Register,
    to_id: u8,
    to_width: u8,
    long: bool,
) -> Option<(Instruction, Gain)> {
    let immediate = match displacement {
        Displacement::Integer(value) => {
            let value = *value;
            // In 64-bit code the address is the displacement sign-extended;
            // outside it, the 32 bits of it.
            let fits =
                i32::try_from(value).is_ok() || (!long && (0..=0xFFFF_FFFF_i64).contains(&value));
            fits.then_some(())?;
            let (name, value, to) = match to_width {
                8 if value >= 0 => ("movl", value, named(to_id, 4)?),
                8 => ("movq", value, register(to)),
                4 => ("movl", value, register(to)),
                2 => ("movw", i64::from(value as u16 as i16), register(to)),
                _ => return None,
            };
            return shorter(rewritten(instr, name, vec![number(value), to]));
        }
        Displacement::Symbol(name) => ImmediateValue::Symbol(name.clone()),
        Displacement::SymbolPlusOffset(name, offset) | Displacement::SymbolAddend(name, offset) => {
            ImmediateValue::SymbolPlusOffset(name.clone(), *offset)
        }
        _ => return None,
    };
    // A symbol keeps the relocation it has: the sign-extended 32-bit one
    // into a 64-bit register, the one without a sign into a 32-bit one.
    // (Into a 16-bit register it would become a 16-bit one.)
    matches!((long, to_width), (true, 8 | 4) | (false, 4)).then_some(())?;
    shorter(rewritten(
        instr,
        &spelled("mov", to_width),
        vec![Operand::Immediate(immediate), register(to)],
    ))
}

/// The same address as a shorter sum: twice a register as the register
/// plus itself, and a base that costs a displacement byte (`%r13`, and for
/// an address that is only computed `%rbp`) as the index of the other
/// register. `computed` says the address is not accessed (`lea`).
///
/// `%rbp` does not become the base of an address that is accessed, nor
/// stop being it: such an address is the stack segment's, which outside
/// 64-bit code is another segment and inside it another fault.
fn resummed(address: &MemoryOperand, computed: bool) -> Option<MemoryOperand> {
    let index = address.index.as_ref()?;
    let (index_id, index_width) = general(&index.name)?;
    let zero = matches!(
        address.displacement,
        Displacement::None | Displacement::Integer(0)
    );
    let costly = |id: u8| id & 7 == 5;
    match (&address.base, address.scale.unwrap_or(1)) {
        (None, 2) => {
            // A symbol's four bytes are there either way.
            let numeric = zero || matches!(address.displacement, Displacement::Integer(_));
            (numeric && (computed || index_id != 5)).then(|| MemoryOperand {
                base: Some(index.clone()),
                scale: None,
                displacement: if zero {
                    Displacement::None
                } else {
                    address.displacement.clone()
                },
                ..address.clone()
            })
        }
        (Some(base), 1) => {
            let (base_id, base_width) = general(&base.name)?;
            (zero
                && base_width == index_width
                && base_width >= 4
                && costly(base_id)
                && !costly(index_id)
                && (computed || base_id != 5))
                .then(|| MemoryOperand {
                    base: Some(index.clone()),
                    index: Some(base.clone()),
                    scale: None,
                    displacement: Displacement::None,
                    ..address.clone()
                })
        }
        _ => None,
    }
}

/// The address of any instruction but `lea` as a shorter sum
/// ([`resummed`]). Not of the instructions that take their base and their
/// index apart: the bound instructions and the tile ones.
fn readdress(
    instr: &Instruction,
    mnemonic: &str,
    operands: &[Operand],
) -> Option<(Instruction, Gain)> {
    let apart = |operand: &Operand| {
        matches!(operand, Operand::Register(register)
            if register.name.starts_with("tmm") || register.name.starts_with("bnd"))
    };
    if mnemonic.starts_with("lea")
        || mnemonic.starts_with("bnd")
        || mnemonic.starts_with("tile")
        || operands.iter().any(apart)
    {
        return None;
    }
    let (at, better) = operands
        .iter()
        .enumerate()
        .find_map(|(at, operand)| match operand {
            Operand::Memory(address) => Some((at, resummed(address, false)?)),
            _ => None,
        })?;
    let mut operands = operands.to_vec();
    *operands.get_mut(at)? = Operand::Memory(better);
    shorter(rewritten(instr, mnemonic, operands))
}

/// A condition, and the one that holds when it does not.
const CONDITIONS: &[(&str, &str)] = &[
    ("o", "no"),
    ("no", "o"),
    ("b", "ae"),
    ("c", "nc"),
    ("nae", "ae"),
    ("ae", "b"),
    ("nb", "b"),
    ("nc", "c"),
    ("e", "ne"),
    ("z", "nz"),
    ("ne", "e"),
    ("nz", "z"),
    ("be", "a"),
    ("na", "a"),
    ("a", "be"),
    ("nbe", "be"),
    ("s", "ns"),
    ("ns", "s"),
    ("p", "np"),
    ("pe", "po"),
    ("np", "p"),
    ("po", "pe"),
    ("l", "ge"),
    ("nge", "ge"),
    ("ge", "l"),
    ("nl", "l"),
    ("le", "g"),
    ("ng", "g"),
    ("g", "le"),
    ("nle", "le"),
];

/// A `cmov` mnemonic as its condition's opposite and its suffix.
fn opposite(mnemonic: &str) -> Option<String> {
    let rest = mnemonic.strip_prefix("cmov")?;
    let inverse = |condition: &str| {
        CONDITIONS
            .iter()
            .find_map(|(name, inverse)| (*name == condition).then_some(*inverse))
    };
    if let Some(inverse) = inverse(rest) {
        return Some(format!("cmov{inverse}"));
    }
    let (condition, suffix) = rest.split_at(rest.len().checked_sub(1)?);
    matches!(suffix, "w" | "l" | "q").then_some(())?;
    Some(format!("cmov{}{suffix}", inverse(condition)?))
}

/// APX: a three-operand form whose destination is one of its sources, as
/// the two-operand instruction. For 32- and 64-bit operands only: a
/// narrower three-operand form clears the rest of the destination
/// register, which the two-operand one keeps.
fn two_operand(
    instr: &Instruction,
    mnemonic: &str,
    operands: &[Operand],
) -> Option<(Instruction, Gain)> {
    let (Operand::Register(to), sources) = operands.split_last()? else {
        return None;
    };
    if to.mask.is_some() || general(&to.name).is_none_or(|(_, width)| width < 4) {
        return None;
    }
    let kept = |operands: Vec<Operand>| shorter(rewritten(instr, mnemonic, operands));
    // `cfcmov` between registers is `cmov`: there is no fault for it to
    // leave out. (From memory it leaves one out where its condition does
    // not hold, which `cmov` does not.)
    let conditional = match mnemonic.strip_prefix("cf") {
        Some(name) if name.starts_with("cmov") => Some((name, true)),
        _ => mnemonic.starts_with("cmov").then_some((mnemonic, false)),
    };
    if let Some((name, faultless)) = conditional {
        let [from, other] = sources else {
            return None;
        };
        if faultless && !matches!(from, Operand::Register(_)) {
            return None;
        }
        if is_register(other, to) {
            return shorter(rewritten(instr, name, vec![from.clone(), register(to)]));
        }
        // The destination is what is moved if the condition holds: the
        // other one is, if it does not.
        if is_register(from, to) && matches!(other, Operand::Register(_)) {
            return shorter(rewritten(
                instr,
                &opposite(name)?,
                vec![other.clone(), register(to)],
            ));
        }
        return None;
    }
    let (stem, _) = suffixed(mnemonic, TWO_OPERAND)?;
    match (stem, sources) {
        ("inc" | "dec" | "not" | "neg", [from]) if is_register(from, to) => {
            kept(vec![register(to)])
        }
        // A shift by one, of another register's value into this one.
        ("rol" | "ror" | "rcl" | "rcr" | "shl" | "sal" | "shr" | "sar", [from])
            if is_register(from, to) =>
        {
            kept(vec![register(to)])
        }
        ("rol" | "ror" | "rcl" | "rcr" | "shl" | "sal" | "shr" | "sar", [count, from])
            if is_register(from, to) =>
        {
            kept(vec![count.clone(), register(to)])
        }
        ("shld" | "shrd", [count, from, other]) if is_register(other, to) => {
            kept(vec![count.clone(), from.clone(), register(to)])
        }
        (
            "add" | "or" | "adc" | "sbb" | "and" | "sub" | "xor" | "imul" | "adcx" | "adox",
            [from, other],
        ) => {
            // `imul` of a number is the three-operand form it always had.
            if stem == "imul" && matches!(from, Operand::Immediate(_)) {
                return None;
            }
            if is_register(other, to) {
                return kept(vec![from.clone(), register(to)]);
            }
            // The sum the other way round.
            let commutes = !matches!(stem, "sbb" | "sub");
            (commutes
                && is_register(from, to)
                && matches!(other, Operand::Register(_) | Operand::Memory(_)))
            .then(|| vec![other.clone(), register(to)])
            .and_then(kept)
        }
        _ => None,
    }
}

/// The mnemonics [`flagless`] knows, without their suffixes.
const FLAGLESS: &[&str] = &[
    "add", "sub", "xor", "rol", "ror", "imulzu", "imul", "shl", "sal",
];

/// `value` as a number of `width` bytes, if it is one: `0xffff` is -1 at
/// two bytes.
fn of_width(value: i64, width: u8) -> Option<i64> {
    match width {
        1 => (-0x80..=0xFF)
            .contains(&value)
            .then_some(i64::from(value as i8)),
        2 => (-0x8000..=0xFFFF)
            .contains(&value)
            .then_some(i64::from(value as i16)),
        4 => (-0x8000_0000..=0xFFFF_FFFF_i64)
            .contains(&value)
            .then_some(i64::from(value as i32)),
        _ => Some(value),
    }
}

/// APX statements that write no flags (`{nf}`): with nothing but the
/// result to keep, adding a number is subtracting its opposite, adding
/// one is `inc`, a rotation by all bits but one is the rotation by one the
/// other way, a multiplication by a power of two is a shift, and a
/// register less itself is a zero to load. Sums of
/// registers and numbers are `lea`, which writes no flags and needs no
/// APX: a multiplication by 3, 5 or 9 always (`lea` is the faster), an
/// addition for size (it runs on fewer ports than `add`).
fn flagless(instr: &Instruction, long: bool) -> Option<(Instruction, Gain)> {
    if !long {
        return None;
    }
    let mnemonic = instr.mnemonic.as_str();
    let operands = instr.operands.as_slice();
    let (stem, suffix) = suffixed(mnemonic, FLAGLESS)?;
    // What is operated on, and where the result goes, if elsewhere.
    let (first, rest) = operands.split_first()?;
    let width = match (suffix, operands.last()?) {
        (Some(width), _) => width,
        (None, Operand::Register(to)) => general_or_high(&to.name)?,
        // A number into memory, and no suffix: 32 bits.
        (None, Operand::Memory(_)) if matches!(first, Operand::Immediate(_)) => 4,
        _ => return None,
    };
    let smaller = instr.optimize == Optimize::Smaller;
    // The 64-bit register an address is made of, and the register of
    // `width` the result goes to: for `lea`.
    let wide = |operand: &Operand| -> Option<Register> {
        let Operand::Register(operand) = operand else {
            return None;
        };
        let (id, said) = general(&operand.name)?;
        match (said == width, named(id, 8)?) {
            (true, Operand::Register(wide)) => Some(wide),
            _ => None,
        }
    };
    let lea = |address: MemoryOperand, to: &Operand| {
        let instr = Instruction {
            nf: false,
            ..rewritten(
                instr,
                &spelled("lea", width),
                vec![Operand::Memory(address), to.clone()],
            )
        };
        Some((instr, Gain::Shorter))
    };
    let sum = |base: Register, index: Option<(Register, u8)>, displacement: i64| MemoryOperand {
        segment: None,
        displacement: if displacement == 0 {
            Displacement::None
        } else {
            Displacement::Integer(displacement)
        },
        base: Some(base),
        scale: index
            .as_ref()
            .map(|(_, scale)| *scale)
            .filter(|scale| *scale != 1),
        index: index.map(|(index, _)| index),
        mask: None,
        zeroing: false,
        broadcast: None,
    };
    // A 16-bit three-operand form clears the rest of its destination,
    // which a 16-bit `lea` keeps; a byte has no `lea`.
    let leas = |rest: &[Operand]| width >= 4 || (width == 2 && rest.len() == 1);
    match (stem, first) {
        ("add" | "sub", Operand::Immediate(ImmediateValue::Integer(value))) => {
            let value = of_width(*value, width)?;
            let adds = stem == "add";
            if value == 1 || value == -1 {
                let name = if adds == (value == 1) { "inc" } else { "dec" };
                return shorter(rewritten(instr, &spelled(name, width), rest.to_vec()));
            }
            if value == 0x80 && width > 1 {
                let name = if adds { "sub" } else { "add" };
                let mut operands = vec![number(-0x80)];
                operands.extend_from_slice(rest);
                return shorter(rewritten(instr, &spelled(name, width), operands));
            }
            if smaller && leas(rest) {
                let displacement = if adds { value } else { value.checked_neg()? };
                // Of a 32-bit sum only the low 32 bits are kept: less
                // -2^31 is plus -2^31 there. (Not of a 64-bit one.)
                let displacement = if width == 4 {
                    i64::from(displacement as i32)
                } else {
                    displacement
                };
                i32::try_from(displacement).ok()?;
                return lea(sum(wide(rest.first()?)?, None, displacement), rest.last()?);
            }
            None
        }
        ("sub" | "xor", Operand::Register(from)) => {
            // A register less itself, or exclusive-or itself, is zero: a
            // number to load, which writes no flags either and waits for
            // no register. (Without `{nf}` this is the instruction every
            // processor clears a register by; that one writes the flags.)
            let (to, whole) = match rest {
                [Operand::Register(to)] if to.name == from.name => (to, width >= 4),
                // Three operands: the rest of the destination is cleared
                // at any width.
                [second, Operand::Register(to)] if is_register(second, from) => (to, true),
                _ => return None,
            };
            let (id, _) = general(&to.name)?;
            let size = if whole { 4 } else { width };
            let instr = Instruction {
                nf: false,
                ..rewritten(
                    instr,
                    &spelled("mov", size),
                    vec![number(0), named(id, size)?],
                )
            };
            Some((instr, Gain::Shorter))
        }
        ("add", Operand::Register(_)) if smaller && leas(rest) => {
            // `%rsp` can be the base of an address only, and `%rbp` and
            // `%r13` cost a displacement byte as one.
            let (from, other) = (wide(first)?, wide(rest.first()?)?);
            let costly = |register: &Register| gp_id(&register.name).is_some_and(|id| id & 7 == 5);
            let (base, index) = if from.name == "rsp" || (costly(&other) && !costly(&from)) {
                (from, other)
            } else {
                (other, from)
            };
            (index.name != "rsp")
                .then(|| sum(base, Some((index, 1)), 0))
                .and_then(|address| lea(address, rest.last()?))
        }
        ("shl" | "sal", Operand::Immediate(ImmediateValue::Integer(1)))
            if smaller && leas(rest) =>
        {
            let twice = wide(rest.first()?)?;
            (twice.name != "rsp")
                .then(|| sum(twice.clone(), Some((twice, 1)), 0))
                .and_then(|address| lea(address, rest.last()?))
        }
        ("rol" | "ror", Operand::Immediate(ImmediateValue::Integer(count)))
            if *count == i64::from(width) * 8 - 1 =>
        {
            let name = if stem == "rol" { "ror" } else { "rol" };
            let mut operands = vec![number(1)];
            operands.extend_from_slice(rest);
            shorter(rewritten(instr, &spelled(name, width), operands))
        }
        ("imul" | "imulzu", Operand::Immediate(ImmediateValue::Integer(factor))) if width > 1 => {
            // The one-source form multiplies its register.
            let (from, to) = match rest {
                [to] => (to, to),
                [from, to] => (from, to),
                _ => return None,
            };
            let Operand::Register(to_register) = to else {
                return None;
            };
            let in_place = is_register(from, to_register);
            let zeroes = stem == "imulzu";
            if !zeroes && matches!(factor, 3 | 5 | 9) {
                let times = wide(from)?;
                return (times.name != "rsp")
                    .then(|| sum(times.clone(), Some((times, (*factor - 1) as u8)), 0))
                    .and_then(|address| lea(address, to))
                    .map(|(instr, _)| (instr, Gain::Faster));
            }
            // A power of two, the top bit of the width included.
            let factor =
                of_width(*factor, width)? as u64 & (u64::MAX >> (64 - u32::from(width) * 8));
            if !factor.is_power_of_two() || factor == 1 {
                return None;
            }
            let count = number(i64::from(factor.trailing_zeros()));
            let name = spelled("shl", width);
            // A 16-bit result that is not to clear the rest of its
            // register has to be shifted in place; one that is, cannot be.
            let operands = match (width, zeroes, in_place) {
                (2, false, false) => return None,
                (2, true, _) | (_, _, false) => vec![count, from.clone(), to.clone()],
                (_, _, true) => vec![count, to.clone()],
            };
            faster(rewritten(instr, &name, operands))
        }
        _ => None,
    }
}

/// The packed-single twin of a packed-double move or bit operation, and
/// (for size) of a packed-integer one.
fn single(mnemonic: &str, smaller: bool) -> Option<&'static str> {
    Some(match mnemonic {
        "movapd" => "movaps",
        "movupd" => "movups",
        "movlpd" => "movlps",
        "movhpd" => "movhps",
        "xorpd" => "xorps",
        "andpd" => "andps",
        "andnpd" => "andnps",
        "orpd" => "orps",
        "movdqa" if smaller => "movaps",
        "movdqu" if smaller => "movups",
        "pxor" if smaller => "xorps",
        "pand" if smaller => "andps",
        "pandn" if smaller => "andnps",
        "por" if smaller => "orps",
        _ => return None,
    })
}

/// The packed-single instruction that stores what `mnemonic` stores.
fn stored(mnemonic: &str) -> Option<&'static str> {
    Some(match mnemonic {
        "movdqa" => "movaps",
        "movdqu" => "movups",
        "movq" | "movsd" => "movlps",
        _ => return None,
    })
}

/// Without VEX: the packed-double family and the stores, the packed
/// shifts by one, the operations of a register with itself, and the
/// extractions and insertions that are moves.
fn packed(
    instr: &Instruction,
    mnemonic: &str,
    operands: &[Operand],
) -> Option<(Instruction, Gain)> {
    let xmm = |operand: &Operand| match operand {
        Operand::Register(operand) => is_xmm(&operand.name),
        Operand::Memory(_) => true,
        _ => false,
    };
    let same = |first: &Register, second: &Register| first.name == second.name;
    if let ([Operand::Register(from), Operand::Memory(_)], Some(name)) =
        (operands, stored(mnemonic))
    {
        if is_xmm(&from.name) {
            return shorter(rewritten(instr, name, operands.to_vec()));
        }
    }
    if let (Some(name), [from, to]) = (
        single(mnemonic, instr.optimize == Optimize::Smaller),
        operands,
    ) {
        // `%xmm` registers and memory: the integer ones have `%mm` forms.
        let registers = [from, to]
            .iter()
            .any(|operand| matches!(operand, Operand::Register(_)));
        return (registers && xmm(from) && xmm(to))
            .then(|| rewritten(instr, name, operands.to_vec()))
            .and_then(shorter);
    }
    match (mnemonic, operands) {
        ("unpcklpd", [Operand::Register(from), Operand::Register(to)])
            if is_xmm(&from.name) && is_xmm(&to.name) =>
        {
            shorter(rewritten(instr, "movlhps", operands.to_vec()))
        }
        // Its own low half twice.
        ("movddup", [Operand::Register(from), Operand::Register(to)])
            if same(from, to) && is_xmm(&from.name) =>
        {
            shorter(rewritten(instr, "movlhps", operands.to_vec()))
        }
        (
            "psllw" | "pslld" | "psllq",
            [
                Operand::Immediate(ImmediateValue::Integer(1)),
                Operand::Register(operand),
            ],
        ) if is_xmm(&operand.name)
            // `paddq` on `%mm` registers came with SSE2.
            || (is_mmx(&operand.name) && mnemonic != "psllq") =>
        {
            let name = format!("padd{}", &mnemonic[4..]);
            shorter(rewritten(
                instr,
                &name,
                vec![register(operand), register(operand)],
            ))
        }
        // Greater than itself, or what it is not and is: nothing, and the
        // exclusive-or is what every processor clears a register by
        // without running anything.
        (
            "pcmpgtb" | "pcmpgtw" | "pcmpgtd" | "pcmpgtq" | "pandn",
            [Operand::Register(first), Operand::Register(second)],
        ) if same(first, second) && (is_xmm(&first.name) || is_mmx(&first.name)) => {
            faster(rewritten(instr, "pxor", operands.to_vec()))
        }
        ("andnps" | "andnpd", [Operand::Register(first), Operand::Register(second)])
            if same(first, second) && is_xmm(&first.name) =>
        {
            faster(rewritten(instr, "xorps", operands.to_vec()))
        }
        ("pcmpeqq", [Operand::Register(first), Operand::Register(second)])
            if same(first, second) && is_xmm(&first.name) =>
        {
            // Equal to itself: everything, which the doubleword compare
            // says in one opcode byte less.
            shorter(rewritten(instr, "pcmpeqd", operands.to_vec()))
        }
        (
            "pextrd" | "pextrq" | "extractps",
            [
                Operand::Immediate(ImmediateValue::Integer(0)),
                Operand::Register(from),
                to,
            ],
        ) if is_xmm(&from.name) => {
            let (name, to) = element_zero(mnemonic, to)?;
            shorter(rewritten(instr, name, vec![register(from), to]))
        }
        // The high half, stored.
        (
            "pextrq",
            [
                Operand::Immediate(ImmediateValue::Integer(1)),
                Operand::Register(from),
                to @ Operand::Memory(_),
            ],
        ) if is_xmm(&from.name) => {
            shorter(rewritten(instr, "movhps", vec![register(from), to.clone()]))
        }
        (
            "insertps",
            [
                Operand::Immediate(ImmediateValue::Integer(select)),
                from,
                Operand::Register(to),
            ],
        ) if is_xmm(&to.name) => {
            match from {
                // Every element zeroed, whatever is inserted. (From memory it
                // would still be read.)
                Operand::Register(from) if is_xmm(&from.name) && select & 0xF == 0xF => {
                    faster(rewritten(instr, "xorps", vec![register(to), register(to)]))
                }
                Operand::Register(from) if is_xmm(&from.name) && *select == 0 => shorter(
                    rewritten(instr, "movss", vec![register(from), register(to)]),
                ),
                // Element 0 from memory and the other three zeroed: the load.
                Operand::Memory(_) if select & 0x3F == 0x0E => {
                    shorter(rewritten(instr, "movss", vec![from.clone(), register(to)]))
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// The move that extracting element 0 is, and its destination: a
/// `pextrd`, `pextrq` or `extractps` (`v` or not) into `to`.
fn element_zero(mnemonic: &str, to: &Operand) -> Option<(&'static str, Operand)> {
    let vex = mnemonic.starts_with('v');
    let stem = mnemonic.strip_prefix('v').unwrap_or(mnemonic);
    let (name, to) = match (stem, to) {
        (_, Operand::Register(to)) => {
            let (id, width) = general(&to.name)?;
            match (stem, width) {
                // The upper half of a 64-bit destination is cleared
                // either way.
                ("pextrd", 4) | ("extractps", 4 | 8) => ("movd", named(id, 4)?),
                ("pextrq", 8) => ("movq", named(id, 8)?),
                _ => return None,
            }
        }
        ("pextrd", Operand::Memory(_)) => ("movd", to.clone()),
        ("pextrq", Operand::Memory(_)) => ("movq", to.clone()),
        ("extractps", Operand::Memory(_)) => ("movss", to.clone()),
        _ => return None,
    };
    let name = match (vex, name) {
        (false, name) => name,
        (true, "movd") => "vmovd",
        (true, "movq") => "vmovq",
        (true, _) => "vmovss",
    };
    Some((name, to))
}

/// `kxor` and `kandn` of a mask register with itself: nothing, at any
/// width, and the 16-bit exclusive-or has the shorter prefix and is the
/// one a processor clears a mask register by.
fn mask(instr: &Instruction, mnemonic: &str, operands: &[Operand]) -> Option<(Instruction, Gain)> {
    let gain = match mnemonic {
        "kxord" | "kxorq" | "kandnd" | "kandnq" => Gain::Shorter,
        "kandnw" | "kandnb" => Gain::Faster,
        _ => return None,
    };
    match operands {
        [
            Operand::Register(first),
            Operand::Register(second),
            Operand::Register(to),
        ] if first.name == second.name && is_kreg(&first.name) && is_kreg(&to.name) => {
            Some((rewritten(instr, "kxorw", operands.to_vec()), gain))
        }
        _ => None,
    }
}

/// Whether every operand can be VEX-encoded: `%xmm` and `%ymm` registers
/// below 16, no mask, no broadcast, no register of an address above 15;
/// a number is an immediate either encoding has.
fn vex_encodable(operands: &[Operand]) -> bool {
    operands.iter().all(|operand| match operand {
        Operand::Register(operand) => {
            operand.mask.is_none()
                && !operand.zeroing
                && operand.broadcast.is_none()
                && vector(&operand.name).is_some_and(|(width, number)| width < 3 && number < 16)
        }
        Operand::Memory(address) => {
            address.mask.is_none()
                && !address.zeroing
                && address.broadcast.is_none()
                && [&address.base, &address.index]
                    .into_iter()
                    .flatten()
                    .all(|part| gp_id(&part.name).is_none_or(|id| id < 16))
        }
        Operand::Immediate(ImmediateValue::Integer(_)) => true,
        _ => false,
    })
}

/// The VEX-encoded instruction an EVEX-only spelling is, where its
/// operands allow (`vex_encodable`): the same operation without a mask to
/// tell its elements apart.
fn vex_spelling(mnemonic: &str) -> Option<&'static str> {
    Some(match mnemonic {
        "vmovdqa32" | "vmovdqa64" => "vmovdqa",
        "vmovdqu8" | "vmovdqu16" | "vmovdqu32" | "vmovdqu64" => "vmovdqu",
        "vpandd" | "vpandq" => "vpand",
        "vpandnd" | "vpandnq" => "vpandn",
        "vpord" | "vporq" => "vpor",
        "vpxord" | "vpxorq" => "vpxor",
        "vextractf32x4" | "vextractf64x2" => "vextractf128",
        "vextracti32x4" | "vextracti64x2" => "vextracti128",
        "vinsertf32x4" | "vinsertf64x2" => "vinsertf128",
        "vinserti32x4" | "vinserti64x2" => "vinserti128",
        "vbroadcastf32x4" | "vbroadcastf64x2" => "vbroadcastf128",
        "vbroadcasti32x4" | "vbroadcasti64x2" => "vbroadcasti128",
        "vbroadcastf32x2" => "vbroadcastsd",
        "vbroadcasti32x2" => "vpbroadcastq",
        _ => return None,
    })
}

/// The moves `vextract... $0` is: aligned between registers, unaligned to
/// memory (the extraction never faults on alignment), each with the
/// extraction's element size, which a mask goes by; and the width
/// extracted (1 `%xmm`, 2 `%ymm`).
fn extraction(mnemonic: &str) -> Option<(&'static str, &'static str, u8)> {
    Some(match mnemonic {
        "vextractf128" | "vextractf32x4" => ("vmovaps", "vmovups", 1),
        "vextractf64x2" => ("vmovapd", "vmovupd", 1),
        "vextracti128" => ("vmovdqa", "vmovdqu", 1),
        "vextracti32x4" => ("vmovdqa32", "vmovdqu32", 1),
        "vextracti64x2" => ("vmovdqa64", "vmovdqu64", 1),
        "vextractf32x8" => ("vmovaps", "vmovups", 2),
        "vextractf64x4" => ("vmovapd", "vmovupd", 2),
        "vextracti32x8" => ("vmovdqa32", "vmovdqu32", 2),
        "vextracti64x4" => ("vmovdqa64", "vmovdqu64", 2),
        _ => return None,
    })
}

/// What clears the vector register `to`: the exclusive-or `name` of a
/// register with itself, in the 128-bit form that clears the whole
/// register, reading the register that costs no prefix byte. `None` if
/// `to` cannot be VEX-encoded.
fn cleared(instr: &Instruction, name: &str, read: u8, to: u8) -> Option<Instruction> {
    (to < 16).then_some(())?;
    // A register above 7 in the r/m field takes the three-byte VEX prefix.
    let read = match (read < 8, to < 8) {
        (true, _) => read,
        (false, true) => to,
        (false, false) => 0,
    };
    Some(rewritten(
        instr,
        name,
        vec![
            vector_named(1, read),
            vector_named(1, read),
            vector_named(1, to),
        ],
    ))
}

/// The exclusive-or that gives what `mnemonic` gives of a register with
/// itself: nothing. In the same domain, so that nothing waits for what
/// reads it.
fn clearing(mnemonic: &str) -> Option<&'static str> {
    Some(match mnemonic {
        "vpxor" | "vpxord" | "vpxorq" | "vpandn" | "vpandnd" | "vpandnq" | "vpsubb" | "vpsubw"
        | "vpsubd" | "vpsubq" | "vpcmpgtb" | "vpcmpgtw" | "vpcmpgtd" | "vpcmpgtq" => "vpxor",
        "vxorps" | "vandnps" => "vxorps",
        "vxorpd" | "vandnpd" => "vxorpd",
        _ => return None,
    })
}

/// The VEX and EVEX instructions.
fn vex(instr: &Instruction, mnemonic: &str, operands: &[Operand]) -> Option<(Instruction, Gain)> {
    // The instruction by its VEX name, if it has another.
    if let (Some(name), true) = (vex_spelling(mnemonic), vex_encodable(operands)) {
        return shorter(rewritten(instr, name, operands.to_vec()));
    }
    let unmasked = |operand: &Register| operand.mask.is_none() && !operand.zeroing;
    match operands {
        // What clears a register: an operation of a register with
        // itself whose result does not depend on it.
        [
            Operand::Register(first),
            Operand::Register(second),
            Operand::Register(to),
        ] if first.name == second.name && unmasked(to) => {
            let ((width, read), (to_width, to_number)) = (vector(&first.name)?, vector(&to.name)?);
            if width != to_width {
                return None;
            }
            // Equal to itself: everything, in one opcode byte less.
            if mnemonic == "vpcmpeqq" && width < 3 {
                return shorter(rewritten(instr, "vpcmpeqd", operands.to_vec()));
            }
            let name = clearing(mnemonic)?;
            if let Some(clear) = cleared(instr, name, read, to_number) {
                // VEX-encoded on `%xmm`, which every processor that runs
                // the statement as written can run. Nothing to do for one
                // that is that already.
                let same = width == 1 && name == mnemonic && read < 8;
                return (!same).then_some(clear).and_then(faster);
            }
            // EVEX either way. From 256 bits the 128-bit form needs the
            // same extensions; from 512 it would need another.
            (width == 2 && !mnemonic.starts_with("vpcmp"))
                .then(|| {
                    Some(rewritten(
                        instr,
                        mnemonic,
                        vec![resized(first, 1)?, resized(second, 1)?, resized(to, 1)?],
                    ))
                })
                .flatten()
                .and_then(faster)
        }
        [
            Operand::Immediate(ImmediateValue::Integer(0)),
            Operand::Register(from),
            to,
        ] => {
            if let Some((aligned, unaligned, width)) = extraction(mnemonic) {
                let (from_width, _) = vector(&from.name)?;
                let low = resized(from, width)?;
                let (name, operands) = match to {
                    Operand::Register(_) => (aligned, vec![low, to.clone()]),
                    Operand::Memory(_) => (unaligned, vec![low, to.clone()]),
                    _ => return None,
                };
                // Out of a `%zmm` register the statement needs AVX-512
                // without its vector-length extension, which the move
                // needs unless it is VEX-encoded.
                let name = match (vex_encodable(&operands), from_width) {
                    (true, _) => vex_spelling(name).unwrap_or(name),
                    (false, 3) => return None,
                    (false, _) => name,
                };
                return shorter(rewritten(instr, name, operands));
            }
            if !is_xmm(&from.name) {
                return None;
            }
            let (name, to) = element_zero(mnemonic, to)?;
            shorter(rewritten(instr, name, vec![register(from), to]))
        }
        // The high half, stored.
        [
            Operand::Immediate(ImmediateValue::Integer(1)),
            Operand::Register(from),
            to @ Operand::Memory(_),
        ] if mnemonic == "vpextrq" && is_xmm(&from.name) => shorter(rewritten(
            instr,
            "vmovhps",
            vec![register(from), to.clone()],
        )),
        [
            Operand::Immediate(ImmediateValue::Integer(1)),
            Operand::Register(from),
            Operand::Register(to),
        ] if matches!(mnemonic, "vpsllw" | "vpslld" | "vpsllq")
            && from.mask.is_none()
            && vector(&from.name).is_some() =>
        {
            let name = format!("vpadd{}", &mnemonic[5..]);
            shorter(rewritten(
                instr,
                &name,
                vec![register(from), register(from), register(to)],
            ))
        }
        [
            Operand::Immediate(ImmediateValue::Integer(select)),
            from,
            Operand::Register(second),
            Operand::Register(to),
        ] if mnemonic == "vinsertps" && unmasked(to) => {
            let (1, to_number) = vector(&to.name)? else {
                return None;
            };
            match from {
                Operand::Register(from) if select & 0xF == 0xF && is_xmm(&from.name) => {
                    cleared(instr, "vxorps", to_number, to_number).and_then(faster)
                }
                Operand::Register(from) if *select == 0 && is_xmm(&from.name) => {
                    shorter(rewritten(
                        instr,
                        "vmovss",
                        vec![register(from), register(second), register(to)],
                    ))
                }
                // Element 0 from memory and the other three zeroed: the
                // load, whatever the second source is.
                Operand::Memory(_) if select & 0x3F == 0x0E => {
                    shorter(rewritten(instr, "vmovss", vec![from.clone(), register(to)]))
                }
                _ => None,
            }
        }
        [Operand::Register(from), Operand::Register(to)]
            if mnemonic == "vpbroadcastq"
                && is_xmm(&from.name)
                && is_xmm(&to.name)
                && unmasked(to) =>
        {
            shorter(rewritten(
                instr,
                "vpunpcklqdq",
                vec![register(from), register(from), register(to)],
            ))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::i686::assembler::encoder::InstructionEncoder as Encoder32;
    use crate::backend::x86::assembler::parser::parse_statement;

    /// The bytes of `statement` in 64-bit code, replaced as far as
    /// `optimize` allows.
    fn bytes(statement: &str, optimize: Optimize) -> Vec<u8> {
        let instr = parse_statement(statement, optimize)
            .unwrap_or_else(|| panic!("`{statement}` is not a statement"));
        let mut encoder = InstructionEncoder::new();
        encoder
            .encode(&instr)
            .unwrap_or_else(|error| panic!("`{statement}`: {error}"));
        encoder.bytes
    }

    /// The same in 32-bit code.
    fn bytes32(statement: &str, optimize: Optimize) -> Vec<u8> {
        let instr = parse_statement(statement, optimize)
            .unwrap_or_else(|| panic!("`{statement}` is not a statement"));
        let mut encoder = Encoder32::new();
        encoder
            .encode(&instr)
            .unwrap_or_else(|error| panic!("`{statement}`: {error}"));
        encoder.bytes
    }

    /// A statement, and the instruction it is assembled as wherever a
    /// replacement is allowed.
    const REPLACED: &[(&str, &str)] = &[
        // A 64-bit operation whose result has no bit above 31.
        ("movq $1, %rax", "movl $1, %eax"),
        ("movabsq $0x12345678, %rcx", "movl $0x12345678, %ecx"),
        ("movabsq $-1, %rcx", "movq $-1, %rcx"),
        ("andq $0x7f, %rax", "andl $0x7f, %eax"),
        ("testq $0x7fffffff, %rcx", "testl $0x7fffffff, %ecx"),
        ("btq $5, %rcx", "btl $5, %ecx"),
        ("xorq %rax, %rax", "xorl %eax, %eax"),
        ("subq %r9, %r9", "subl %r9d, %r9d"),
        ("movzbq %cl, %rax", "movzbl %cl, %eax"),
        ("movzwq %ax, %rax", "movzwl %ax, %eax"),
        // The extensions of the accumulator.
        ("movslq %eax, %rax", "cltq"),
        ("movswl %ax, %eax", "cwtl"),
        ("movsbw %al, %ax", "cbtw"),
        // `test` of the low byte; `test` for a register with itself.
        ("testl $1, %eax", "testb $1, %al"),
        ("testq $0x40, %rdi", "testb $0x40, %dil"),
        ("andq %rcx, %rcx", "testq %rcx, %rcx"),
        ("orw %cx, %cx", "testw %cx, %cx"),
        ("orb %dl, %dl", "testb %dl, %dl"),
        // A register added to itself.
        ("shll $1, %edx", "addl %edx, %edx"),
        ("shlq $1, %r9", "addq %r9, %r9"),
        ("xaddl %ecx, %ecx", "addl %ecx, %ecx"),
        // `lea` that copies, and what an address costs.
        ("leaq (%rsp), %rsi", "movq %rsp, %rsi"),
        ("leal (%rsi), %eax", "movl %esi, %eax"),
        ("leaq 16, %rax", "movl $16, %eax"),
        ("leaq %fs:(%rax,%rcx), %rdx", "leaq (%rax,%rcx), %rdx"),
        ("leal (%eax,%ecx), %edx", "leal (%rax,%rcx), %edx"),
        ("leaq (,%rax,2), %rcx", "leaq (%rax,%rax), %rcx"),
        ("movl (,%rax,2), %ecx", "movl (%rax,%rax), %ecx"),
        ("movl (%r13,%rax), %ecx", "movl (%rax,%r13), %ecx"),
        ("lock xchgl %eax, (%rdx)", "xchgl %eax, (%rdx)"),
        // APX: the two-operand instruction, and what `{nf}` allows.
        ("addq %r8, %r9, %r9", "addq %r8, %r9"),
        ("addq %r9, %r8, %r9", "addq %r8, %r9"),
        ("subq %r8, %r9, %r9", "subq %r8, %r9"),
        ("cfcmovael %ecx, %edx, %edx", "cmovael %ecx, %edx"),
        ("{nf} addl $1, %ecx", "{nf} incl %ecx"),
        ("{nf} subq $1, %rcx", "{nf} decq %rcx"),
        ("{nf} addl $0x80, %ecx", "{nf} subl $-0x80, %ecx"),
        ("{nf} roll $31, %ecx", "{nf} rorl $1, %ecx"),
        ("{nf} imull $5, %ecx, %edx", "leal (%rcx,%rcx,4), %edx"),
        ("{nf} imull $8, %ecx, %edx", "{nf} shll $3, %ecx, %edx"),
        (
            "{nf} imulq $0x80, (%rbp), %rcx",
            "{nf} shlq $7, (%rbp), %rcx",
        ),
        (
            "{nf} imulzuw $0x80, (%rsp), %r9w",
            "{nf} shlw $7, (%rsp), %r9w",
        ),
        ("{nf} sub $1, (%rbp)", "{nf} decl (%rbp)"),
        ("{nf} addq $-1, 8(%rsp)", "{nf} decq 8(%rsp)"),
        ("{nf} rol $31, (%rdx)", "{nf} rorl $1, (%rdx)"),
        ("{nf} subl %ecx, %ecx, %ecx", "movl $0, %ecx"),
        ("{nf} xorw %cx, %cx, %dx", "movl $0, %edx"),
        ("{nf} xorq %rcx, %rcx", "movl $0, %ecx"),
        ("{nf} subw %cx, %cx", "movw $0, %cx"),
        ("{nf} xorb %cl, %cl", "movb $0, %cl"),
        ("movbe %ecx, %ecx", "bswapl %ecx"),
        ("movbeq %r9, %r9", "bswapq %r9"),
        // No prefix byte: packed single for packed double, and for a store.
        ("movapd %xmm1, %xmm2", "movaps %xmm1, %xmm2"),
        ("movupd (%rax), %xmm2", "movups (%rax), %xmm2"),
        ("movsd %xmm0, (%rdi)", "movlps %xmm0, (%rdi)"),
        ("movq %xmm0, (%rdi)", "movlps %xmm0, (%rdi)"),
        ("movdqa %xmm1, (%rdx)", "movaps %xmm1, (%rdx)"),
        ("movdqu %xmm1, (%rdx)", "movups %xmm1, (%rdx)"),
        ("xorpd %xmm1, %xmm1", "xorps %xmm1, %xmm1"),
        ("xorpd %xmm1, %xmm2", "xorps %xmm1, %xmm2"),
        // What clears a register.
        ("andnps %xmm1, %xmm1", "xorps %xmm1, %xmm1"),
        ("vpxor %ymm9, %ymm9, %ymm2", "vpxor %xmm2, %xmm2, %xmm2"),
        ("vpxord %zmm1, %zmm1, %zmm2", "vpxor %xmm1, %xmm1, %xmm2"),
        ("vxorps %ymm3, %ymm3, %ymm3", "vxorps %xmm3, %xmm3, %xmm3"),
        ("kxorq %k1, %k1, %k2", "kxorw %k1, %k1, %k2"),
        ("kandnw %k1, %k1, %k2", "kxorw %k1, %k1, %k2"),
        // The VEX encoding of an EVEX-only spelling.
        ("vmovdqa32 %xmm1, %xmm2", "vmovdqa %xmm1, %xmm2"),
        ("vmovdqu64 (%rax), %ymm2", "vmovdqu (%rax), %ymm2"),
        ("vpxord %xmm1, %xmm2, %xmm3", "vpxor %xmm1, %xmm2, %xmm3"),
        ("vpandq %ymm1, %ymm2, %ymm3", "vpand %ymm1, %ymm2, %ymm3"),
        // Element 0, and a shift by one.
        ("vextractf128 $0, %ymm1, %xmm2", "vmovaps %xmm1, %xmm2"),
        ("vextracti32x4 $0, %zmm1, %xmm2", "vmovdqa %xmm1, %xmm2"),
        ("pextrd $0, %xmm1, %eax", "movd %xmm1, %eax"),
        ("pextrq $0, %xmm1, %rax", "movq %xmm1, %rax"),
        ("extractps $0, %xmm1, %eax", "movd %xmm1, %eax"),
        ("psllq $1, %xmm1", "paddq %xmm1, %xmm1"),
        ("pslld $1, %xmm3", "paddd %xmm3, %xmm3"),
        ("leaw (%rax), %cx", "movw %ax, %cx"),
        // A bare symbol is an address like another.
        ("movsd %xmm0, counter", "movlps %xmm0, counter"),
        ("vmovdqa64 counter+8, %ymm2", "vmovdqa counter+8, %ymm2"),
        ("leaq counter, %rax", "movq $counter, %rax"),
        ("leal counter, %eax", "movl $counter, %eax"),
        ("lock xchgl %eax, counter", "xchgl %eax, counter"),
    ];

    /// The replacements that cost time, made only where size was asked
    /// for.
    const SMALLER: &[(&str, &str)] = &[
        ("movzbw %cl, %cx", "movb $0, %ch"),
        ("movdqa %xmm1, %xmm2", "movaps %xmm1, %xmm2"),
        ("movdqa (%rdx), %xmm1", "movaps (%rdx), %xmm1"),
        ("pxor %xmm9, %xmm9", "xorps %xmm9, %xmm9"),
        ("{nf} addl %ecx, %edx, %eax", "leal (%rdx,%rcx), %eax"),
        (
            "{nf} subl $-0x80000000, %ecx",
            "leal -0x80000000(%rcx), %ecx",
        ),
    ];

    /// Statements that look like one of the above and are not.
    const KEPT: &[&str] = &[
        // Already the shortest.
        "movq $-1, %rax",
        "andq $0x7fffffff, %r9",
        // The result has bits above 31, or the flags look there.
        "andq $-1, %rax",
        "btq $32, %rcx",
        // The sign flag is bit 7 of the mask.
        "testl $0x80, %eax",
        // A 32-bit `and` clears the upper half of the register; `test`
        // writes nothing.
        "andl %ecx, %ecx",
        // `%rbp` as a base makes the address the stack segment's.
        "movl (%rbp,%rax), %ecx",
        "movl (%rax,%rbp), %ecx",
        // Subtraction does not commute.
        "subq %r9, %r8, %r9",
        // A narrow three-operand form clears the rest of its destination.
        "addw %cx, %dx, %dx",
        // The two-operand `cfcmov` leaves its destination alone, and
        // `cmov` writes it either way.
        "cfcmovael %ecx, %edx",
        // The 128-bit form of an instruction on `%zmm17` needs AVX512VL.
        "vpxord %zmm17, %zmm17, %zmm17",
        "vextractf64x4 $0, %zmm20, (%rbp)",
        // `paddq` on `%mm` registers came with SSE2.
        "psllq $1, %mm0",
        // `cmp` defines AF, `test` leaves it undefined.
        "cmpl $0, %eax",
        // The rest of the destination comes from the first source.
        "vcmpeqss %xmm9, %xmm1, %xmm1",
        // Of two NaNs the first source's is the result.
        "vaddps %xmm9, %xmm1, %xmm1",
        // Asked for as written.
        "{nooptimize} movq $1, %rax",
        // A relocation keeps its type.
        "movq $sym, %rax",
        "leaw sym, %ax",
        "leaq sym(%rip), %rax",
        // A branch target is no address.
        "jmp counter",
        // No shorter: `%r9d` costs the load a prefix byte too.
        "{nf} xorl %r9d, %r9d, %r9d",
        // The two registers differ; a word swapped in place is undefined.
        "{nf} subl %ecx, %edx, %edx",
        "movbe %ecx, %edx",
        "movbe %cx, %cx",
        // Less -2^31 is plus 2^31, which no 64-bit address can add.
        "{nf} subq $-0x80000000, %rcx",
        // The count is `%cl`, and what is shifted has no size said.
        "{nf} rol %cl, (%rdx)",
        // Nothing to gain.
        "movl $1, %eax",
        "addq %r8, %r9",
        "ret",
    ];

    #[test]
    fn a_replacement_is_the_instruction_it_names() {
        for (written, replacement) in REPLACED {
            let plain = bytes(written, Optimize::Never);
            let wanted = bytes(replacement, Optimize::Never);
            assert_ne!(plain, wanted, "`{written}` is `{replacement}` as written");
            for level in [Optimize::Faster, Optimize::Smaller] {
                let found = bytes(written, level);
                // Where size was asked for, a still shorter one may win.
                if level == Optimize::Faster || found.len() == wanted.len() {
                    assert_eq!(
                        found, wanted,
                        "`{written}` should be `{replacement}` at {level:?}"
                    );
                }
                assert!(found.len() <= plain.len(), "`{written}` grew at {level:?}");
            }
            // What replaces a statement is itself as short as it gets.
            assert_eq!(
                bytes(replacement, Optimize::Faster).len(),
                wanted.len(),
                "`{replacement}` has a shorter form still"
            );
        }
    }

    #[test]
    fn what_costs_time_is_for_size_only() {
        for (written, replacement) in SMALLER {
            let wanted = bytes(replacement, Optimize::Never);
            assert_eq!(
                bytes(written, Optimize::Smaller),
                wanted,
                "`{written}` should be `{replacement}` for size"
            );
            assert_ne!(
                bytes(written, Optimize::Faster),
                wanted,
                "`{written}` should not be `{replacement}` for speed"
            );
            assert!(wanted.len() < bytes(written, Optimize::Never).len());
        }
    }

    #[test]
    fn what_is_not_the_same_stays() {
        for written in KEPT {
            let plain = bytes(written, Optimize::Never);
            for level in [Optimize::Faster, Optimize::Smaller] {
                assert_eq!(bytes(written, level), plain, "`{written}` at {level:?}");
            }
        }
    }

    #[test]
    fn nothing_is_replaced_unless_asked() {
        for (written, _) in REPLACED.iter().chain(SMALLER) {
            let instr = parse_statement(written, Optimize::Never).expect(written);
            let mut encoder = InstructionEncoder::new();
            let replaced = encoder.encode_choosing(&instr).expect(written);
            assert!(replaced.is_none(), "`{written}` was replaced as written");
        }
    }

    /// The text rewriter asks [`candidate`] before it parses a statement:
    /// it may say yes to a statement that has no replacement, never no to
    /// one that has.
    #[test]
    fn candidate_admits_every_rewrite() {
        for (written, _) in REPLACED.iter().chain(SMALLER) {
            let (flagless, statement) = match written.strip_prefix("{nf} ") {
                Some(statement) => (true, statement),
                None => (false, *written),
            };
            // The first word and the rest, as the text rewriter splits a
            // line (`lock` is the first word of a locked statement).
            let (mnemonic, operands) = statement.split_once(' ').unwrap_or((statement, ""));
            let admitted = if flagless {
                flagless_candidate(mnemonic)
            } else {
                candidate(mnemonic, operands, true, Optimize::Smaller)
            };
            assert!(admitted, "`{written}` is not a candidate");
        }
    }

    #[test]
    fn thirty_two_bit_code_has_its_own_registers() {
        let replaced = [
            ("testl $1, %eax", "testb $1, %al"),
            ("testl $0x40, %ebx", "testb $0x40, %bl"),
            // No upper half to clear here.
            ("andl %ecx, %ecx", "testl %ecx, %ecx"),
            ("orl %esi, %esi", "testl %esi, %esi"),
            ("shll $1, %edx", "addl %edx, %edx"),
            ("leal (%esp), %esi", "movl %esp, %esi"),
            ("leal (,%eax,2), %ecx", "leal (%eax,%eax), %ecx"),
            ("leal 16, %eax", "movl $16, %eax"),
            ("leal %fs:(%eax,%ecx), %edx", "leal (%eax,%ecx), %edx"),
            ("leaw (%bx), %ax", "movw %bx, %ax"),
            ("movl (,%eax,2), %ecx", "movl (%eax,%eax), %ecx"),
            ("movswl %ax, %eax", "cwtl"),
            ("movapd %xmm1, %xmm2", "movaps %xmm1, %xmm2"),
            ("movsd %xmm0, (%edi)", "movlps %xmm0, (%edi)"),
            ("movdqa %xmm1, (%edx)", "movaps %xmm1, (%edx)"),
            ("vpxor %ymm1, %ymm1, %ymm2", "vpxor %xmm1, %xmm1, %xmm2"),
            ("vmovdqa32 %xmm1, %xmm2", "vmovdqa %xmm1, %xmm2"),
            ("pextrd $0, %xmm1, %eax", "movd %xmm1, %eax"),
            ("lock xchgl %eax, (%edx)", "xchgl %eax, (%edx)"),
            ("movsd %xmm0, counter", "movlps %xmm0, counter"),
            ("leal counter, %ecx", "movl $counter, %ecx"),
            ("vmovdqu8 %xmm6, counter", "vmovdqu %xmm6, counter"),
        ];
        for (written, replacement) in replaced {
            let wanted = bytes32(replacement, Optimize::Never);
            assert_ne!(bytes32(written, Optimize::Never), wanted, "{written}");
            assert_eq!(bytes32(written, Optimize::Faster), wanted, "{written}");
        }
        // `%sil` and `%dil` are 64-bit code's: these have no byte form.
        for written in [
            "testl $1, %esi",
            "testw $1, %si",
            "testl $1, %edi",
            "movl (%ebp,%eax), %ecx",
            "cmpl $0, %eax",
        ] {
            let plain = bytes32(written, Optimize::Never);
            assert_eq!(bytes32(written, Optimize::Faster), plain, "{written}");
            assert_eq!(bytes32(written, Optimize::Smaller), plain, "{written}");
        }
    }

    /// The assembler's own rule for a person's text: `-Wa,-O` and no
    /// `{nooptimize}`, and never behind data or a lone prefix.
    #[test]
    fn a_text_is_replaced_statement_by_statement() {
        use crate::backend::x86::assembler::parser::{AsmItem, parse_asm_with};
        let text = "\tmovq $1, %rax\n\
                    \t{nooptimize} movq $1, %rcx\n\
                    \t.byte 0x66\n\
                    \tmovq $1, %rdx\n\
                    \tmovq $1, %rbx\n\
                    \tlock\n\
                    \txchgl %eax, (%rdx)\n";
        let levels = |optimize| -> Vec<Optimize> {
            parse_asm_with(text, optimize)
                .expect("parses")
                .iter()
                .filter_map(|item| match item {
                    AsmItem::Instruction(instr) if instr.mnemonic != "lock" => Some(instr.optimize),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(levels(Optimize::Never), [Optimize::Never; 5]);
        assert_eq!(
            levels(Optimize::Faster),
            [
                Optimize::Faster,
                Optimize::Never,
                Optimize::Never,
                Optimize::Faster,
                Optimize::Never
            ]
        );
    }
}
