//! AArch64 instruction encoder.
//!
//! Encodes AArch64 instructions into 32-bit machine code words.
//! This covers the subset of instructions emitted by our codegen.
//!
//! AArch64 instructions are always 4 bytes (32 bits), little-endian.
//! The encoding format varies by instruction class.

// Encoding helpers for all AArch64 instruction formats; not all formats used yet.
#![allow(dead_code)]

use super::parser::Operand;

mod bitfield;
mod compare_branch;
mod data_processing;
mod fp_scalar;
mod load_store;
mod neon;
mod sysreg_table;
mod system;

pub(crate) use bitfield::*;
pub(crate) use compare_branch::*;
pub(crate) use data_processing::*;
pub(crate) use fp_scalar::*;
pub(crate) use load_store::*;
pub(crate) use neon::*;
pub(crate) use system::*;

/// Result of encoding an instruction.
#[derive(Debug, Clone)]
pub enum EncodeResult {
    /// Successfully encoded as a 4-byte instruction word
    Word(u32),
    /// Instruction needs a relocation to be applied later
    WordWithReloc { word: u32, reloc: Relocation },
    /// Multiple encoded words (e.g., movz+movk sequence)
    Words(Vec<u32>),
    /// Skip this instruction (e.g., pseudo-instruction handled elsewhere)
    Skip,
}

/// Relocation types for AArch64 ELF
#[derive(Debug, Clone)]
pub enum RelocType {
    /// R_AARCH64_CALL26 - for BL instruction (26-bit PC-relative)
    Call26,
    /// R_AARCH64_JUMP26 - for B instruction (26-bit PC-relative)
    Jump26,
    /// R_AARCH64_ADR_PREL_PG_HI21 - for ADRP (page-relative, bits [32:12])
    AdrpPage21,
    /// R_AARCH64_ADD_ABS_LO12_NC - for ADD :lo12: (low 12 bits)
    AddAbsLo12,
    /// R_AARCH64_LDST8_ABS_LO12_NC
    Ldst8AbsLo12,
    /// R_AARCH64_LDST16_ABS_LO12_NC
    Ldst16AbsLo12,
    /// R_AARCH64_LDST32_ABS_LO12_NC
    Ldst32AbsLo12,
    /// R_AARCH64_LDST64_ABS_LO12_NC
    Ldst64AbsLo12,
    /// R_AARCH64_LDST128_ABS_LO12_NC
    Ldst128AbsLo12,
    /// R_AARCH64_ADR_GOT_PAGE21 - GOT-relative ADRP
    AdrGotPage21,
    /// R_AARCH64_LD64_GOT_LO12_NC - GOT entry LDR
    Ld64GotLo12,
    /// R_AARCH64_TLSLE_ADD_TPREL_HI12
    TlsLeAddTprelHi12,
    /// R_AARCH64_TLSLE_ADD_TPREL_LO12_NC
    TlsLeAddTprelLo12,
    /// R_AARCH64_CONDBR19 - conditional branch, 19-bit offset
    CondBr19,
    /// R_AARCH64_TSTBR14 - test-and-branch, 14-bit offset
    TstBr14,
    /// R_AARCH64_ADR_PREL_LO21 - for ADR (21-bit PC-relative)
    AdrPrelLo21,
    /// R_AARCH64_ABS64 - 64-bit absolute
    Abs64,
    /// R_AARCH64_ABS32 - 32-bit absolute
    Abs32,
    /// R_AARCH64_PREL32 - 32-bit PC-relative
    Prel32,
    /// R_AARCH64_PREL64 - 64-bit PC-relative
    Prel64,
    /// R_AARCH64_LD_PREL_LO19 - LDR literal, 19-bit PC-relative
    Ldr19,
    /// R_AARCH64_MOVW_UABS_G0/G1/G2/G3 - movz/movk absolute halfword
    /// (build-time overflow check: the full value must fit)
    MovwUabsG0,
    MovwUabsG1,
    MovwUabsG2,
    MovwUabsG3,
    /// R_AARCH64_MOVW_UABS_G0/G1/G2 _NC - no-check variants (value is
    /// truncated; used for later movk chunks). The ABI defines no G3_NC.
    MovwUabsG0Nc,
    MovwUabsG1Nc,
    MovwUabsG2Nc,
    /// R_AARCH64_MOVW_SABS_G0/G1/G2 - signed variants. The ABI defines
    /// no SABS_G3 (the top halfword of a signed value uses UABS_G3).
    MovwSabsG0,
    MovwSabsG1,
    MovwSabsG2,
}

impl RelocType {
    /// Get the ELF relocation type number
    pub fn elf_type(&self) -> u32 {
        match self {
            RelocType::Abs64 => 257,             // R_AARCH64_ABS64
            RelocType::Abs32 => 258,             // R_AARCH64_ABS32
            RelocType::Prel32 => 261,            // R_AARCH64_PREL32
            RelocType::Prel64 => 260,            // R_AARCH64_PREL64
            RelocType::Call26 => 283,            // R_AARCH64_CALL26
            RelocType::Jump26 => 282,            // R_AARCH64_JUMP26
            RelocType::AdrPrelLo21 => 274,       // R_AARCH64_ADR_PREL_LO21
            RelocType::AdrpPage21 => 275,        // R_AARCH64_ADR_PREL_PG_HI21
            RelocType::AddAbsLo12 => 277,        // R_AARCH64_ADD_ABS_LO12_NC
            RelocType::Ldst8AbsLo12 => 278,      // R_AARCH64_LDST8_ABS_LO12_NC
            RelocType::Ldst16AbsLo12 => 284,     // R_AARCH64_LDST16_ABS_LO12_NC
            RelocType::Ldst32AbsLo12 => 285,     // R_AARCH64_LDST32_ABS_LO12_NC
            RelocType::Ldst64AbsLo12 => 286,     // R_AARCH64_LDST64_ABS_LO12_NC
            RelocType::Ldst128AbsLo12 => 299,    // R_AARCH64_LDST128_ABS_LO12_NC
            RelocType::AdrGotPage21 => 311,      // R_AARCH64_ADR_GOT_PAGE21
            RelocType::Ld64GotLo12 => 312,       // R_AARCH64_LD64_GOT_LO12_NC
            RelocType::TlsLeAddTprelHi12 => 549, // R_AARCH64_TLSLE_ADD_TPREL_HI12
            RelocType::TlsLeAddTprelLo12 => 551, // R_AARCH64_TLSLE_ADD_TPREL_LO12_NC
            RelocType::CondBr19 => 280,          // R_AARCH64_CONDBR19
            RelocType::TstBr14 => 279,           // R_AARCH64_TSTBR14
            RelocType::Ldr19 => 273,             // R_AARCH64_LD_PREL_LO19
            // MOVW halfword relocations (IHI0056B / LLVM AArch64.def):
            // G0=0x107=263 .. G3=0x10d=269, SABS_G0=0x10e=270 .. SABS_G2=272
            RelocType::MovwUabsG0 => 263,
            RelocType::MovwUabsG0Nc => 264,
            RelocType::MovwUabsG1 => 265,
            RelocType::MovwUabsG1Nc => 266,
            RelocType::MovwUabsG2 => 267,
            RelocType::MovwUabsG2Nc => 268,
            RelocType::MovwUabsG3 => 269,
            RelocType::MovwSabsG0 => 270,
            RelocType::MovwSabsG1 => 271,
            RelocType::MovwSabsG2 => 272,
        }
    }
}

/// A relocation to be applied.
#[derive(Debug, Clone)]
pub struct Relocation {
    pub reloc_type: RelocType,
    pub symbol: String,
    pub addend: i64,
}

/// Parse a register name to its 5-bit encoding number (0-30, 31 for sp/zr).
///
/// Allocation-free: the previous `to_lowercase()` copy showed up as ~2% of
/// assembler instruction-retirement on the Callgrind A/B (see
/// `docs/REVIEW_756_ADJUDICATION.md`). Identity aliases are matched
/// case-insensitively; numbered forms accept only canonical decimal digits.
pub fn parse_reg_num(name: &str) -> Option<u32> {
    if name.eq_ignore_ascii_case("sp")
        || name.eq_ignore_ascii_case("wsp")
        || name.eq_ignore_ascii_case("xzr")
        || name.eq_ignore_ascii_case("wzr")
    {
        return Some(31);
    }
    if name.eq_ignore_ascii_case("lr") {
        return Some(30);
    }
    let b = name.as_bytes();
    let prefix = *b.first()?;
    match prefix.to_ascii_lowercase() {
        b'x' | b'w' | b'd' | b's' | b'q' | b'v' | b'h' | b'b' => {
            // `str::parse::<u32>` is far more permissive than the
            // AArch64 register grammar: it accepts a leading `+` and
            // arbitrary leading zeros, so "x+5", "x007" and "w+31"
            // all used to resolve to a real register instead of being
            // rejected. Accept only bare decimal digits, and only in
            // canonical (no leading zero) form -- matching GAS, which
            // rejects "x007" as an unknown symbol.
            // (Upstream fork issues #118 and #207.)
            let digits = &b[1..];
            if digits.is_empty()
                || !digits.iter().all(|d| d.is_ascii_digit())
                || (digits.len() > 1 && digits[0] == b'0')
            {
                return None;
            }
            let mut num = 0u32;
            for &d in digits {
                num = num.checked_mul(10)?.checked_add(u32::from(d - b'0'))?;
                if num > 31 {
                    return None;
                }
            }
            Some(num)
        }
        _ => None,
    }
}

/// Identity of a general-purpose register operand.
///
/// A register *number* is not a register identity. AArch64 gives encoding 31
/// two different meanings, and which one applies depends on the instruction
/// form *and* on the operand slot: in `add x0, x1, x2` slot 2 reads 31 as XZR,
/// in `add x0, sp, #8` slot 1 reads 31 as SP, and `clz x0, sp` is not an
/// instruction at all, because CLZ's source slot accepts only `Xn` or `XZR`.
///
/// Collapsing a spelling to a number before the form has been checked is how
/// an invalid operand turns into a *different valid instruction*. Two real
/// cases, both rejected by GNU as and both silently assembled here:
/// `fmov h0, wsp` encoded as `FMOV H0, WZR` (0x1ee703e0), reading the zero
/// register where the stack pointer was written, and `mov d0, #1` encoded as
/// `movz w0, #1`. `GpReg` keeps the three identities -- numbered, SP, ZR --
/// apart until each slot's role has been checked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct GpReg {
    /// 0..=30 for `x0`-`x30`/`w0`-`w30`/`lr`; 31 for the `sp`/`zr` aliases.
    pub num: u32,
    /// `x`, `sp`, `xzr` and `lr` spell 64-bit registers; `w`, `wsp` and `wzr`
    /// spell 32-bit ones.
    pub is_64: bool,
    /// `sp`/`wsp`. The stack pointer is never the zero register.
    pub is_sp: bool,
    /// `xzr`/`wzr`. The zero register is never the stack pointer.
    pub is_zr: bool,
}

/// What one operand slot of a GP-only instruction may hold.
///
/// These are the architecture's own operand classes, spelled per slot. A class
/// check ("is this a GP register?") cannot answer the question an encoder has
/// to answer ("may *this slot* of *this form* hold the stack pointer?"), which
/// is why `sp` passed a class check and then encoded as the zero register.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GpRole {
    /// `Rd`/`Rn`/`Rm` where 31 means the zero register: `Xn|XZR`, `Wn|WZR`.
    /// The stack pointer is not encodable here.
    RegOrZr,
    /// Slots where 31 means the stack pointer: `Xn|SP`, `Wn|WSP`. The zero
    /// register is not encodable here.
    RegOrSp,
    /// Slots that accept either meaning of 31, such as the destination of the
    /// non-flags-setting logical immediates and the first operand of the
    /// `cmp`/`cmn`/`mov`-to-SP aliases.
    RegSpOrZr,
    /// A numbered register only; neither 31-encoding is legal in this slot.
    Reg,
}

impl GpReg {
    /// Resolve a register spelling to a general-purpose identity.
    ///
    /// Accepts exactly the spellings GNU as accepts for a GP register:
    /// `x0`-`x30`, `w0`-`w30`, `lr`, `sp`/`wsp`, `xzr`/`wzr`. It refuses
    /// FP/SIMD spellings (`d0`, `v3`), out-of-range numbers (`x32`),
    /// non-canonical decimals (`x007`), and `x31`/`w31`: register 31 is
    /// spelled by the role the form gives it, and accepting a role-free
    /// spelling is exactly what lets a caller mistake SP for ZR.
    ///
    /// Allocation-free and case-insensitive on purpose -- this is on the
    /// assembler's hot path, where an earlier `to_lowercase()` cost 2.17% of
    /// total instruction count (Callgrind; see
    /// `docs/REVIEW_756_ADJUDICATION.md`).
    pub(crate) fn by_name(name: &str) -> Option<GpReg> {
        if name.eq_ignore_ascii_case("sp") {
            return Some(GpReg {
                num: 31,
                is_64: true,
                is_sp: true,
                is_zr: false,
            });
        }
        if name.eq_ignore_ascii_case("wsp") {
            return Some(GpReg {
                num: 31,
                is_64: false,
                is_sp: true,
                is_zr: false,
            });
        }
        if name.eq_ignore_ascii_case("xzr") {
            return Some(GpReg {
                num: 31,
                is_64: true,
                is_sp: false,
                is_zr: true,
            });
        }
        if name.eq_ignore_ascii_case("wzr") {
            return Some(GpReg {
                num: 31,
                is_64: false,
                is_sp: false,
                is_zr: true,
            });
        }
        if name.eq_ignore_ascii_case("lr") {
            return Some(GpReg {
                num: 30,
                is_64: true,
                is_sp: false,
                is_zr: false,
            });
        }
        let b = name.as_bytes();
        let (is_64, digits) = match b.first()? {
            b'x' | b'X' => (true, &b[1..]),
            b'w' | b'W' => (false, &b[1..]),
            _ => return None,
        };
        if digits.is_empty() || digits.len() > 2 {
            return None;
        }
        // Canonical decimal only: `x007` is not a register spelling, and
        // accepting it would give one register two spellings that a caller
        // could then disagree about.
        if digits.len() == 2 && digits[0] == b'0' {
            return None;
        }
        let mut num = 0u32;
        for &d in digits {
            if !d.is_ascii_digit() {
                return None;
            }
            num = num * 10 + u32::from(d - b'0');
        }
        // 31 is spelled `xzr`/`wzr`/`sp`/`wsp`, never `x31`.
        if num > 30 {
            return None;
        }
        Some(GpReg {
            num,
            is_64,
            is_sp: false,
            is_zr: false,
        })
    }

    /// Whether two operands have the same register width.
    pub(crate) fn same_width_as(self, other: GpReg) -> bool {
        self.is_64 == other.is_64
    }
}

/// The spelling of operand `idx`, for diagnostics.
pub(crate) fn operand_spelling(operands: &[Operand], idx: usize) -> String {
    match operands.get(idx) {
        Some(Operand::Reg(n)) => n.clone(),
        Some(_) | None => String::from("?"),
    }
}

/// Whether `name` spells a floating-point/SIMD register.
///
/// The two register grammars are disjoint by construction -- this is the
/// floating-point half of the pair whose general-purpose half is
/// [`GpReg::by_name`] -- so a reader can always say which grammar the operand
/// it refused belongs to.
pub(crate) fn is_fp_spelling_pub(name: &str) -> bool {
    let Some((&first, rest)) = name.as_bytes().split_first() else {
        return false;
    };
    if !matches!(first | 0x20, b'b' | b'h' | b's' | b'd' | b'q' | b'v') {
        return false;
    }
    !rest.is_empty()
        && rest.len() <= 2
        && rest.iter().all(u8::is_ascii_digit)
        && !(rest.len() == 2 && rest[0] == b'0')
}

/// A transferred register of a load/store: one of the two register files.
///
/// `LDR`/`STR` and their unscaled, signed, pair and by-element relatives are
/// the instructions whose data register may come from *either* file -- `ldr
/// d0,[x0]` and `ldr x0,[x0]` are the same mnemonic, and the width letter is
/// what selects the access size.  The reader therefore answers both questions
/// a caller has: which file (`is_fp`), and how wide (`letter` for the
/// floating-point file, `is_64` for the general-purpose one).
///
/// `v<n>` without an arrangement is refused here rather than at each call
/// site: the scalar forms are spelled `b`/`h`/`s`/`d`/`q`, and accepting the
/// vector spelling would assemble an instruction the programmer did not write.
pub(crate) struct LdstRt {
    pub num: u32,
    pub is_fp: bool,
    /// The width letter of a floating-point register (`b`/`h`/`s`/`d`/`q`),
    /// or 0 for a general-purpose one.
    pub letter: u8,
    /// General-purpose only: 64-bit spelling (`x`, `lr`, `sp`, `xzr`).
    pub is_64: bool,
    /// The spelling, for diagnostics.
    pub name: String,
}

/// Read a load/store data register: see [`LdstRt`].
pub(crate) fn ldst_rt(operands: &[Operand], idx: usize, mn: &str) -> Result<LdstRt, String> {
    let name = match operands.get(idx) {
        Some(Operand::Reg(n)) => n.as_str(),
        Some(_) => {
            return Err(format!(
                "{mn}: operand {idx} must be a register, not an immediate or memory operand"
            ));
        }
        None => return Err(format!("{mn}: missing operand {idx} (expected a register)")),
    };
    if let Some(gp) = GpReg::by_name(name) {
        return Ok(LdstRt {
            num: gp.num,
            is_fp: false,
            letter: 0,
            is_64: gp.is_64,
            name: name.to_string(),
        });
    }
    if let Ok((num, letter, n)) = fp_reg(operands, idx, mn) {
        if letter == b'v' {
            return Err(format!(
                "{mn}: `{n}` is a vector register; this instruction takes the 128-bit \
                 scalar spelling `q{}`",
                &n[1..]
            ));
        }
        return Ok(LdstRt {
            num,
            is_fp: true,
            letter,
            is_64: true,
            name: n,
        });
    }
    Err(format!(
        "{mn}: operand {idx} `{name}` is not a load/store data register; this slot \
         takes a general-purpose register (x0-x30, w0-w30, lr, sp, wsp, xzr, wzr) \
         or a floating-point one (b0-b31, h0-h31, s0-s31, d0-d31, q0-q31)"
    ))
}

/// Read one register operand and check it against the role its slot has.
///
/// Every GP-only encoder routes its operands through here, so a register-class
/// error, an illegal use of encoding 31, and a width mismatch are all reported
/// at the operand that caused them -- and, unlike the assembler being
/// compared against, with the fix named:
///
/// ```text
/// clz: operand 1 `sp` is the stack pointer, but this slot reads encoding 31
/// as the zero register; write `xzr` if the zero register is what you meant
/// ```
pub(crate) fn reg_operand(
    operands: &[Operand],
    idx: usize,
    role: GpRole,
    mn: &str,
) -> Result<GpReg, String> {
    // Borrowed, not cloned: this runs once per register operand of every
    // GP instruction, and the whole point of `GpReg` is that validation costs
    // no allocation.
    let name = match operands.get(idx) {
        Some(Operand::Reg(n)) => n.as_str(),
        Some(_) => {
            return Err(format!(
                "{mn}: operand {idx} must be a register, not an immediate or memory operand"
            ));
        }
        None => return Err(format!("{mn}: missing operand {idx} (expected a register)")),
    };
    let reg = GpReg::by_name(name).ok_or_else(|| {
        format!(
            "{mn}: operand {idx} `{name}` is not a general-purpose register \
             (expected x0-x30, w0-w30, lr, sp, wzr or xzr)"
        )
    })?;
    match role {
        GpRole::RegOrZr if reg.is_sp => Err(format!(
            "{mn}: operand {idx} `{name}` is the stack pointer, but this slot reads \
             encoding 31 as the zero register; write `{}` if the zero register is \
             what you meant",
            if reg.is_64 { "xzr" } else { "wzr" }
        )),
        GpRole::RegOrSp if reg.is_zr => Err(format!(
            "{mn}: operand {idx} `{name}` is the zero register, but this slot reads \
             encoding 31 as the stack pointer; write `{}` if the stack pointer is \
             what you meant",
            if reg.is_64 { "sp" } else { "wsp" }
        )),
        GpRole::Reg if reg.is_sp || reg.is_zr => Err(format!(
            "{mn}: operand {idx} `{name}` is not allowed here; this slot takes a \
             numbered register (x0-x30/w0-w30) only"
        )),
        _ => Ok(reg),
    }
}

/// Read the two operands of a "data-processing (1 source)" instruction
/// (`clz`, `cls`, `rbit`, `rev`, `rev16`).
///
/// Both slots are `Rd|XZR`, so neither may be the stack pointer, and both must
/// be the same width: `clz x0, w1` has no encoding.
pub(crate) fn dp1src_reg_pair(operands: &[Operand], mn: &str) -> Result<(GpReg, GpReg), String> {
    let rd = reg_operand(operands, 0, GpRole::RegOrZr, mn)?;
    let rn = reg_operand(operands, 1, GpRole::RegOrZr, mn)?;
    if !rd.same_width_as(rn) {
        return Err(format!(
            "{mn}: `{}` and `{}` are different widths; both operands of {mn} must \
             be the same size",
            operand_spelling(operands, 0),
            operand_spelling(operands, 1)
        ));
    }
    Ok((rd, rn))
}

/// Read the three register operands of a shifted-register logical
/// (`and`, `orr`, `eor`, `orn`, `eon`, `bic`, `bics`).
///
/// All three slots are `Rd|Rn|Rm` with 31 meaning the zero register, so the
/// stack pointer is not encodable in any of them, and all three must share one
/// width.
pub(crate) fn logical_reg3(
    operands: &[Operand],
    mn: &str,
) -> Result<(GpReg, GpReg, GpReg), String> {
    let rd = reg_operand(operands, 0, GpRole::RegOrZr, mn)?;
    let rn = reg_operand(operands, 1, GpRole::RegOrZr, mn)?;
    let rm = reg_operand(operands, 2, GpRole::RegOrZr, mn)?;
    if !rd.same_width_as(rn) || !rd.same_width_as(rm) {
        return Err(format!(
            "{mn}: `{}`, `{}` and `{}` must all be the same width",
            operand_spelling(operands, 0),
            operand_spelling(operands, 1),
            operand_spelling(operands, 2)
        ));
    }
    Ok((rd, rn, rm))
}

/// Whether the two register operands of a conversion are both floating-point
/// registers, i.e. whether the *scalar SIMD&FP* form of it was written.
///
/// The eleven float-conversion mnemonics each have two forms whose operands are
/// spelled differently: the general-purpose-destination one (`fcvtms w0,s1`,
/// `scvtf s0,w1`) and the scalar register-file one (`fcvtms s0,s1`,
/// `scvtf s0,s1`).  Neither reader can decide for the other — `reg_operand`
/// refuses an FP spelling and `fp_reg` refuses a GP one — so the dispatcher
/// asks this question first and each form's own reader then validates what it
/// is given.
pub(crate) fn scalar_fp_pair(operands: &[Operand], mn: &str) -> bool {
    fp_reg(operands, 0, mn).is_ok() && fp_reg(operands, 1, mn).is_ok()
}

/// Read `n` general-purpose operands that must all share one width, with
/// encoding 31 read as the zero register.
///
/// This is the shape of the multiply/divide, conditional-select, bitfield and
/// extract families: every slot is `Xd|XZR` (never SP), and one instruction
/// covers both widths through `sf`, so a mixed list has no encoding.  Both
/// halves are checked here because both were fail-open: `mul sp, x1, x2`
/// (SP in a zero-register slot) and `madd wzr, x1, x2, x3` (a 32-bit
/// destination with 64-bit sources) assembled happily, the first as the XZR
/// form and the second by ignoring the widths entirely.
///
/// Returns the register numbers and the shared width.
pub(crate) fn gp_same_width(
    operands: &[Operand],
    n: usize,
    mn: &str,
) -> Result<(Vec<u32>, bool), String> {
    let mut nums = Vec::with_capacity(n);
    let mut is_64 = false;
    let mut first_name = String::new();
    for i in 0..n {
        let r = reg_operand(operands, i, GpRole::RegOrZr, mn)?;
        if i == 0 {
            is_64 = r.is_64;
            first_name = operand_spelling(operands, i);
        } else if r.is_64 != is_64 {
            return Err(format!(
                "{mn}: `{first_name}` and `{}` are different widths; this instruction \
                 has one sf field, so every register operand of {mn} must be the same \
                 size",
                operand_spelling(operands, i)
            ));
        }
        nums.push(r.num);
    }
    Ok((nums, is_64))
}

/// Read one general-purpose operand and require a specific width.
///
/// The `*MADDL`/`*MULL` long forms take a 64-bit destination and 32-bit
/// sources, so their slots cannot all be the same width and each one has to
/// say which it is.
pub(crate) fn gp_widened(
    operands: &[Operand],
    idx: usize,
    is_64: bool,
    mn: &str,
) -> Result<u32, String> {
    let r = reg_operand(operands, idx, GpRole::RegOrZr, mn)?;
    if r.is_64 != is_64 {
        return Err(format!(
            "{mn}: operand {idx} `{}` must be the {}-bit {} register of this form",
            operand_spelling(operands, idx),
            if is_64 { 64 } else { 32 },
            if is_64 { "destination" } else { "source" },
        ));
    }
    Ok(r.num)
}

/// Encode the `option` field of an extended-register operand.
///
/// The old code fell through to `_ => 0b011`, so a misspelt extend silently
/// became UXTX and encoded a different instruction.
pub(crate) fn extend_option(kind: &str, mn: &str) -> Result<u32, String> {
    let option = match kind.to_ascii_lowercase().as_str() {
        "uxtb" => 0b000,
        "uxth" => 0b001,
        "uxtw" => 0b010,
        "uxtx" => 0b011,
        "sxtb" => 0b100,
        "sxth" => 0b101,
        "sxtw" => 0b110,
        "sxtx" => 0b111,
        other => {
            return Err(format!(
                "{mn}: unknown extend `{other}` (expected uxtb, uxth, uxtw, uxtx, \
                 sxtb, sxth, sxtw or sxtx)"
            ));
        }
    };
    Ok(option)
}

/// Encode the `shift`+`imm6` fields of a shifted-register operand.
///
/// The old code fell through to `_ => 0b00` for an unknown shift kind and
/// masked the amount with `& 0x3F`, so `add x0, x1, x2, lsl #64` silently
/// became `add x0, x1, x2` and `orr w0, w1, w2, ror #31` became an LSL. Both
/// are wrong-code bugs, not strictness bugs: an out-of-range amount now
/// reports the legal range, and `ror` is refused where the architecture has no
/// rotate (arithmetic forms), instead of being reinterpreted.
pub(crate) fn shift_field(
    kind: &str,
    amount: u32,
    is_64: bool,
    allow_ror: bool,
    mn: &str,
) -> Result<(u32, u32), String> {
    let max = if is_64 { 63 } else { 31 };
    let shift = match kind.to_ascii_lowercase().as_str() {
        "lsl" => 0b00,
        "lsr" => 0b01,
        "asr" => 0b10,
        "ror" if allow_ror => 0b11,
        "ror" => {
            return Err(format!(
                "{mn}: `ror` is not available in this form (only the logical \
                 instructions and `mvn` have a rotate); use lsl, lsr or asr"
            ));
        }
        other => {
            return Err(format!(
                "{mn}: unknown shift `{other}` (expected lsl, lsr, asr{})",
                if allow_ror { " or ror" } else { "" }
            ));
        }
    };
    if amount > max {
        return Err(format!(
            "{mn}: shift amount #{amount} is out of range for a {}-bit operand \
             (0-{max})",
            if is_64 { 64 } else { 32 }
        ));
    }
    Ok((shift, amount))
}

/// Check if a register name is a 64-bit (X) register or SP.
fn is_64bit_reg(name: &str) -> bool {
    name.eq_ignore_ascii_case("sp")
        || name.eq_ignore_ascii_case("xzr")
        || name.eq_ignore_ascii_case("lr")
        || name
            .as_bytes()
            .first()
            .is_some_and(|&c| c == b'x' || c == b'X')
}

/// Check if a register name is a 32-bit (W) register.
fn is_32bit_reg(name: &str) -> bool {
    name.eq_ignore_ascii_case("wsp")
        || name.eq_ignore_ascii_case("wzr")
        || name
            .as_bytes()
            .first()
            .is_some_and(|&c| c == b'w' || c == b'W')
}

/// Check if a register is a floating-point/SIMD register.
/// Is `name` a SIMD/FP register (`b<0-31>`, `h`, `s`, `d`, `q`, `v<0-31>`)?
///
/// A first-letter test is not enough: `sp` starts with `s` and `b`/`h`/`d`/`q`
/// are also the width letters of the GPR loads (`ldrb`, `ldrh`, ...).  The
/// half-word form `sp` (*stack pointer*) therefore tested as an S register,
/// which made `stur sp,[x0,#8]` encode as `stur s31,[x0,#8]` -- a store of the
/// wrong register file -- and let `ldur sp,[x0,#8]` slip past the transferred-
/// register gate as a scalar FP access.  A name is an FP register only when the
/// width letter is followed by a register number.
fn is_fp_reg(name: &str) -> bool {
    let bytes = name.as_bytes();
    if bytes.is_empty() {
        return false;
    }
    let width = bytes[0].to_ascii_lowercase();
    if !matches!(width, b'd' | b's' | b'q' | b'v' | b'h' | b'b') {
        return false;
    }
    let digits = &name[1..];
    !digits.is_empty() && digits.bytes().all(|c| c.is_ascii_digit())
}

/// Encode a condition code string to 4-bit encoding.
fn encode_cond(cond: &str) -> Option<u32> {
    // Allocation-free: condition codes are four-character tokens on the
    // branch hot path. `eq_ignore_ascii_case` matches GAS's case folding
    // without a temporary String.
    if cond.eq_ignore_ascii_case("eq") {
        Some(0)
    } else if cond.eq_ignore_ascii_case("ne") {
        Some(1)
    } else if cond.eq_ignore_ascii_case("cs") || cond.eq_ignore_ascii_case("hs") {
        Some(2)
    } else if cond.eq_ignore_ascii_case("cc") || cond.eq_ignore_ascii_case("lo") {
        Some(3)
    } else if cond.eq_ignore_ascii_case("mi") {
        Some(4)
    } else if cond.eq_ignore_ascii_case("pl") {
        Some(5)
    } else if cond.eq_ignore_ascii_case("vs") {
        Some(6)
    } else if cond.eq_ignore_ascii_case("vc") {
        Some(7)
    } else if cond.eq_ignore_ascii_case("hi") {
        Some(8)
    } else if cond.eq_ignore_ascii_case("ls") {
        Some(9)
    } else if cond.eq_ignore_ascii_case("ge") {
        Some(10)
    } else if cond.eq_ignore_ascii_case("lt") {
        Some(11)
    } else if cond.eq_ignore_ascii_case("gt") {
        Some(12)
    } else if cond.eq_ignore_ascii_case("le") {
        Some(13)
    } else if cond.eq_ignore_ascii_case("al") {
        Some(14)
    } else if cond.eq_ignore_ascii_case("nv") {
        Some(15)
    } else {
        None
    }
}

/// 64-bit address register: `Xn` or `SP`. Encoding 31 is the stack pointer.
/// A W-register or the zero register is not a valid memory base (GAS rejects
/// `str x0, [w0]` and `str x0, [xzr]`; both previously encoded as `[x0]`).
pub(crate) fn mem_xn_or_sp(name: &str, mn: &str) -> Result<u32, String> {
    let r = GpReg::by_name(name).ok_or_else(|| {
        format!("{mn}: `{name}` is not a 64-bit address register (expected x0-x30, lr or sp)")
    })?;
    if !r.is_64 {
        return Err(format!(
            "{mn}: `{name}` is 32-bit; the address register must be Xn or SP"
        ));
    }
    if r.is_zr {
        return Err(format!(
            "{mn}: the zero register cannot be a memory base; use SP or Xn"
        ));
    }
    Ok(r.num)
}

/// 64-bit GP data register: `Xt|XZR`. Encoding 31 is the zero register.
/// Used by MRS/MSR/SYS/DC/IC/AT/TLBI, which have no SP encoding in Rt.
pub(crate) fn gp_xt(name: &str, mn: &str) -> Result<u32, String> {
    let r = GpReg::by_name(name).ok_or_else(|| {
        format!(
            "{mn}: `{name}` is not a 64-bit general-purpose register (expected x0-x30, lr or xzr)"
        )
    })?;
    if !r.is_64 {
        return Err(format!(
            "{mn}: `{name}` is 32-bit; this instruction takes Xt (x0-x30, lr, xzr)"
        ));
    }
    if r.is_sp {
        return Err(format!(
            "{mn}: the stack pointer is not valid here; write `xzr` if the zero register is what you meant"
        ));
    }
    Ok(r.num)
}

/// Index register of a register-offset load/store: `Xm|XZR` or `Wm|WZR`.
/// Encoding 31 is the zero register; SP is not encodable as Rm
/// (`str x0, [x1, sp]` previously stored using XZR as the index).
pub(crate) fn mem_rm(name: &str, mn: &str) -> Result<GpReg, String> {
    let r = GpReg::by_name(name).ok_or_else(|| {
        format!("{mn}: `{name}` is not a general-purpose index register (expected x/w0-30, lr, xzr or wzr)")
    })?;
    if r.is_sp {
        return Err(format!(
            "{mn}: the stack pointer cannot be an index register; write `{}` if the zero register is what you meant",
            if r.is_64 { "xzr" } else { "wzr" }
        ));
    }
    Ok(r)
}

/// Shared arity guard. Duplicated `if operands.len() < n` sites drift in
/// both the count and the diagnostic; one helper keeps them honest.
pub(crate) fn expect_operands(operands: &[Operand], n: usize, mn: &str) -> Result<(), String> {
    if operands.len() < n {
        return Err(format!("{mn} requires {n} operands"));
    }
    Ok(())
}

/// Reject an immediate that does not fit in `[min, max]` instead of masking
/// it into a different legal encoding (the silent-wrap class).
pub(crate) fn imm_in_range(
    imm: i64,
    min: i64,
    max: i64,
    mn: &str,
    what: &str,
) -> Result<u32, String> {
    if imm < min || imm > max {
        return Err(format!(
            "{mn}: {what} #{imm} is out of range ({min} to {max})"
        ));
    }
    Ok(imm as u32)
}

/// Encode an AArch64 instruction from its mnemonic and parsed operands.
pub fn encode_instruction(
    mnemonic: &str,
    operands: &[Operand],
    raw_operands: &str,
) -> Result<EncodeResult, String> {
    let mn = mnemonic.to_lowercase();

    // Handle condition-code suffixed branches: b.eq, b.ne, b.lt, etc.
    if let Some(cond) = mn.strip_prefix("b.") {
        return encode_cond_branch(cond, operands);
    }

    // Handle condition-code branches without the dot: beq, bne, bge, blt, etc.
    // These are common aliases used in GNU assembler syntax.
    {
        let cond_aliases: &[(&str, &str)] = &[
            ("beq", "eq"),
            ("bne", "ne"),
            ("bcs", "cs"),
            ("bhs", "hs"),
            ("bcc", "cc"),
            ("blo", "lo"),
            ("bmi", "mi"),
            ("bpl", "pl"),
            ("bvs", "vs"),
            ("bvc", "vc"),
            ("bhi", "hi"),
            ("bls", "ls"),
            ("bge", "ge"),
            ("blt", "lt"),
            ("bgt", "gt"),
            ("ble", "le"),
            ("bal", "al"),
        ];
        for &(alias, cond) in cond_aliases {
            if mn == alias {
                return encode_cond_branch(cond, operands);
            }
        }
    }

    match mn.as_str() {
        // Data processing - register
        "mov" => encode_mov(operands),
        "movz" => encode_movz(operands),
        "movk" => encode_movk(operands),
        "movn" => encode_movn(operands),
        "add" => {
            if is_neon_scalar_d_reg_op(operands) {
                encode_neon_scalar_three_same(operands, 0, 0b10000, 0b11)
            } else {
                encode_add_sub(operands, false, false)
            }
        }
        "adds" => encode_add_sub(operands, false, true),
        "sub" => {
            if is_neon_scalar_d_reg_op(operands) {
                encode_neon_scalar_three_same(operands, 1, 0b10000, 0b11)
            } else {
                encode_add_sub(operands, true, false)
            }
        }
        "subs" => encode_add_sub(operands, true, true),
        "and" => encode_logical(operands, 0b00, "and"),
        "orr" => encode_logical(operands, 0b01, "orr"),
        "eor" => encode_logical(operands, 0b10, "eor"),
        "ands" => encode_logical(operands, 0b11, "ands"),
        "orn" => encode_orn(operands),
        "eon" => encode_eon(operands),
        "bics" => encode_bics(operands),
        "mul" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
                    encode_neon_elem(operands, 0, 0b1000)
                } else {
                    encode_neon_three_same(operands, 0, 0b10011)
                }
            } else {
                encode_mul(operands)
            }
        }
        "madd" => encode_madd(operands),
        "msub" => encode_msub(operands),
        "smull" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
                    encode_neon_elem_long(operands, 0, 0b1010, false) // SMULL (by element)
                } else {
                    encode_neon_three_diff(operands, 0, 0b1100, false) // SMULL (vector)
                }
            } else {
                encode_smull(operands) // SMULL (scalar)
            }
        }
        "umull" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
                    encode_neon_elem_long(operands, 1, 0b1010, false) // UMULL (by element)
                } else {
                    encode_neon_three_diff(operands, 1, 0b1100, false) // UMULL (vector)
                }
            } else {
                encode_umull(operands) // UMULL (scalar)
            }
        }
        "smaddl" => encode_smaddl(operands),
        "umaddl" => encode_umaddl(operands),
        "mneg" => encode_mneg(operands),
        "udiv" => encode_div(operands, true),
        "sdiv" => encode_div(operands, false),
        "umulh" => encode_umulh(operands),
        "smulh" => encode_smulh(operands),
        "neg" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_two_misc(operands, 1, 0b01011)
            } else {
                encode_neg(operands)
            }
        }
        "negs" => encode_negs(operands),
        "mvn" => encode_mvn(operands),
        "adc" => encode_adc(operands, false),
        "adcs" => encode_adc(operands, true),
        "sbc" => encode_sbc(operands, false),
        "sbcs" => encode_sbc(operands, true),

        // Shifts
        "lsl" => encode_shift(operands, 0b00),
        "lsr" => encode_shift(operands, 0b01),
        "asr" => encode_shift(operands, 0b10),
        "ror" => encode_shift(operands, 0b11),

        // Extensions
        "sxtw" => encode_sxtw(operands),
        "sxth" => encode_sxth(operands),
        "sxtb" => encode_sxtb(operands),
        "uxtw" => encode_uxtw(operands),
        "uxth" => encode_uxth(operands),
        "uxtb" => encode_uxtb(operands),

        // Compare
        "cmp" => encode_cmp(operands),
        "cmn" => encode_cmn(operands),
        "tst" => encode_tst(operands),
        "ccmp" => encode_ccmp_ccmn(operands, true),
        "ccmn" => encode_ccmp_ccmn(operands, false),

        // Conditional select
        "csel" => encode_csel(operands),
        "csinc" => encode_csinc(operands),
        "csinv" => encode_csinv(operands),
        "csneg" => encode_csneg(operands),
        "cset" => encode_cset(operands),
        "csetm" => encode_csetm(operands),

        // Branches
        "b" => encode_branch(operands),
        "bl" => encode_bl(operands),
        "br" => encode_br(operands),
        "blr" => encode_blr(operands),
        "ret" => encode_ret(operands),
        "cbz" => encode_cbz(operands, false),
        "cbnz" => encode_cbz(operands, true),
        "tbz" => encode_tbz(operands, false),
        "tbnz" => encode_tbz(operands, true),

        // Loads/stores - size determined from register width
        "ldr" => encode_ldr_str_auto(operands, true),
        "str" => encode_ldr_str_auto(operands, false),
        // Mnemonic-keyed destination classes: the `b`/`h` spellings are
        // 32-bit-register accesses (LdstDest::Gpr32), and `ldrsb/ldrsh` take
        // w or x.  An `ldrb x9,[x10]` is a different instruction than the one
        // written and GNU as rejects it; see LdstDest in load_store.rs.
        "ldrb" => encode_ldr_str_checked(operands, true, 0b00, false, false, LdstDest::Gpr32),
        "strb" => encode_ldr_str_checked(operands, false, 0b00, false, false, LdstDest::Gpr32),
        "ldrh" => encode_ldr_str_checked(operands, true, 0b01, false, false, LdstDest::Gpr32),
        "strh" => encode_ldr_str_checked(operands, false, 0b01, false, false, LdstDest::Gpr32),
        "ldrw" | "ldrsw" => encode_ldrsw(operands),
        "ldrsb" => encode_ldrs_checked(operands, 0b00, LdstDest::Gpr32Or64, "ldrsb"),
        "ldrsh" => encode_ldrs_checked(operands, 0b01, LdstDest::Gpr32Or64, "ldrsh"),
        "ldur" => encode_ldur_stur_checked(operands, true, 0b00, LdstDest::Any, None, false, "ldur"),
        "stur" => encode_ldur_stur_checked(operands, false, 0b00, LdstDest::Any, None, false, "stur"),
        // Unscaled spelled widths (measured words: ldurb 38408000, sturh 78008000).
        "ldurb" => encode_ldur_stur_checked(operands, true, 0b00, LdstDest::Gpr32, Some(0b00), false, "ldurb"),
        "sturb" => encode_ldur_stur_checked(operands, false, 0b00, LdstDest::Gpr32, Some(0b00), false, "sturb"),
        "ldurh" => encode_ldur_stur_checked(operands, true, 0b00, LdstDest::Gpr32, Some(0b01), false, "ldurh"),
        "sturh" => encode_ldur_stur_checked(operands, false, 0b00, LdstDest::Gpr32, Some(0b01), false, "sturh"),
        "ldursb" => encode_ldur_stur_checked(operands, true, 0b00, LdstDest::Gpr32Or64, Some(0b00), true, "ldursb"),
        "ldursh" => encode_ldur_stur_checked(operands, true, 0b00, LdstDest::Gpr32Or64, Some(0b01), true, "ldursh"),
        "ldursw" => encode_ldur_stur_checked(operands, true, 0b00, LdstDest::Gpr64, Some(0b10), true, "ldursw"),
        // Unprivileged base: no FP/SIMD form exists, so the bare spellings are
        // GPR-only as well.
        "ldtr" => encode_ldur_stur_checked(operands, true, 0b10, LdstDest::Gpr32Or64, None, false, "ldtr"),
        "sttr" => encode_ldur_stur_checked(operands, false, 0b10, LdstDest::Gpr32Or64, None, false, "sttr"),
        "ldtrb" => encode_ldur_stur_checked(operands, true, 0b10, LdstDest::Gpr32, Some(0b00), false, "ldtrb"),
        "sttrb" => encode_ldur_stur_checked(operands, false, 0b10, LdstDest::Gpr32, Some(0b00), false, "sttrb"),
        "ldtrh" => encode_ldur_stur_checked(operands, true, 0b10, LdstDest::Gpr32, Some(0b01), false, "ldtrh"),
        "sttrh" => encode_ldur_stur_checked(operands, false, 0b10, LdstDest::Gpr32, Some(0b01), false, "sttrh"),
        "ldtrsb" => encode_ldur_stur_checked(operands, true, 0b10, LdstDest::Gpr32Or64, Some(0b00), true, "ldtrsb"),
        "ldtrsh" => encode_ldur_stur_checked(operands, true, 0b10, LdstDest::Gpr32Or64, Some(0b01), true, "ldtrsh"),
        "ldtrsw" => encode_ldur_stur_checked(operands, true, 0b10, LdstDest::Gpr64, Some(0b10), true, "ldtrsw"),
        "ldp" => encode_ldp_stp(operands, true),
        "stp" => encode_ldp_stp(operands, false),
        "ldnp" => encode_ldnp_stnp(operands, true),
        "stnp" => encode_ldnp_stnp(operands, false),
        "ldxr" => encode_ldxr_stxr(operands, "ldxr", true, None),
        "stxr" => encode_ldxr_stxr(operands, "stxr", false, None),
        "ldxrb" => encode_ldxr_stxr(operands, "ldxrb", true, Some(0b00)),
        "stxrb" => encode_ldxr_stxr(operands, "stxrb", false, Some(0b00)),
        "ldxrh" => encode_ldxr_stxr(operands, "ldxrh", true, Some(0b01)),
        "stxrh" => encode_ldxr_stxr(operands, "stxrh", false, Some(0b01)),
        "ldaxr" => encode_ldaxr_stlxr(operands, "ldaxr", true, None),
        "stlxr" => encode_ldaxr_stlxr(operands, "stlxr", false, None),
        "ldaxrb" => encode_ldaxr_stlxr(operands, "ldaxrb", true, Some(0b00)),
        "stlxrb" => encode_ldaxr_stlxr(operands, "stlxrb", false, Some(0b00)),
        "ldaxrh" => encode_ldaxr_stlxr(operands, "ldaxrh", true, Some(0b01)),
        "stlxrh" => encode_ldaxr_stlxr(operands, "stlxrh", false, Some(0b01)),
        "ldar" => encode_ldar_stlr(operands, "ldar", true, None),
        "stlr" => encode_ldar_stlr(operands, "stlr", false, None),
        "ldarb" => encode_ldar_stlr(operands, "ldarb", true, Some(0b00)),
        "stlrb" => encode_ldar_stlr(operands, "stlrb", false, Some(0b00)),
        "ldarh" => encode_ldar_stlr(operands, "ldarh", true, Some(0b01)),
        "stlrh" => encode_ldar_stlr(operands, "stlrh", false, Some(0b01)),
        "ldxp" => encode_ldxp_stxp(operands, "ldxp", true, false),
        "ldaxp" => encode_ldxp_stxp(operands, "ldaxp", true, true),
        "stxp" => encode_ldxp_stxp(operands, "stxp", false, false),
        "stlxp" => encode_ldxp_stxp(operands, "stlxp", false, true),

        // Address computation
        "adrp" => encode_adrp(operands),
        "adr" => encode_adr(operands),

        // Floating point (scalar or vector based on operand type)
        "fmov" => encode_fmov(operands),
        "fadd" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_float_three_same(operands, 0, 0, 0b11010)
            } else {
                encode_fp_arith(operands, 0b0010)
            }
        }
        "fsub" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_float_three_same(operands, 0, 1, 0b11010)
            } else {
                encode_fp_arith(operands, 0b0011)
            }
        }
        "fmul" => {
            // The element operand is what makes this a by-element instruction:
            // the two registers may be spelled either as scalars (`s0,s1`,
            // which is how a compiler emits it) or as 64-bit vectors, so
            // dispatching on the *first* operand's spelling sent every
            // scalar-spelled `fmul s0,s1,v2.s[0]` to the scalar two-operand
            // encoder, which then rejected the lane operand.
            if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
                encode_neon_float_elem(operands, 0b1001, 0, "fmul")
            } else if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_float_three_same(operands, 1, 0, 0b11011)
            } else {
                encode_fp_arith(operands, 0b0000)
            }
        }
        "fmulx" => {
            if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
                encode_neon_float_elem(operands, 0b1001, 1, "fmulx")
            } else {
                encode_neon_float_three_same(operands, 1, 0, 0b11011)
            }
        }
        "fdiv" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_float_three_same(operands, 1, 0, 0b11111)
            } else {
                encode_fp_arith(operands, 0b0001)
            }
        }
        "fmax" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_float_three_same(operands, 0, 0, 0b11110)
            } else {
                encode_fp_arith(operands, 0b0100)
            }
        }
        "fmin" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_float_three_same(operands, 0, 1, 0b11110)
            } else {
                encode_fp_arith(operands, 0b0101)
            }
        }
        "fmaxnm" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_float_three_same(operands, 0, 0, 0b11000)
            } else {
                encode_fp_arith(operands, 0b0110)
            }
        }
        "fminnm" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_float_three_same(operands, 0, 1, 0b11000)
            } else {
                encode_fp_arith(operands, 0b0111)
            }
        }
        "fneg" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_float_two_misc(operands, 1, 0, 0b01111)
            } else {
                encode_fneg(operands)
            }
        }
        "fabs" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_float_two_misc(operands, 0, 1, 0b01111)
            } else {
                encode_fabs(operands)
            }
        }
        "fsqrt" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_float_two_misc(operands, 1, 1, 0b11111)
            } else {
                encode_fsqrt(operands)
            }
        }
        "frintn" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_float_two_misc(operands, 0, 0, 0b11000)
            } else {
                encode_fp_1src(operands, 0b001000)
            }
        }
        "frintp" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_float_two_misc(operands, 0, 1, 0b11000)
            } else {
                encode_fp_1src(operands, 0b001001)
            }
        }
        "frintm" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_float_two_misc(operands, 0, 0, 0b11001)
            } else {
                encode_fp_1src(operands, 0b001010)
            }
        }
        "frintz" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_float_two_misc(operands, 0, 1, 0b11001)
            } else {
                encode_fp_1src(operands, 0b001011)
            }
        }
        "frinta" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_float_two_misc(operands, 1, 0, 0b11000)
            } else {
                encode_fp_1src(operands, 0b001100)
            }
        }
        "frintx" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_float_two_misc(operands, 1, 0, 0b11001)
            } else {
                encode_fp_1src(operands, 0b001110)
            }
        }
        "frinti" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_float_two_misc(operands, 1, 1, 0b11001)
            } else {
                encode_fp_1src(operands, 0b001111)
            }
        }
        "fmadd" => encode_fmadd_fmsub(operands, false),
        "fmsub" => encode_fmadd_fmsub(operands, true),
        "fnmadd" => encode_fnmadd_fnmsub(operands, false),
        "fnmsub" => encode_fnmadd_fnmsub(operands, true),
        "fcmp" => encode_fcmp(operands, raw_operands, false),
        "fcmpe" => encode_fcmp(operands, raw_operands, true),
        "fcvtzs" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                if operands.get(2).is_some() {
                    encode_neon_float_two_misc_fixed(operands, 0, true)
                } else {
                    encode_neon_float_two_misc(operands, 0, 1, 0b11011)
                }
            } else if scalar_fp_pair(operands, "fcvtzs") {
                encode_fp_convert_scalar("fcvtzs", operands)
            } else {
                encode_fcvt_rounding(operands, 0b11, 0b000)
            }
        }
        "fcvtzu" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                if operands.get(2).is_some() {
                    encode_neon_float_two_misc_fixed(operands, 1, true)
                } else {
                    encode_neon_float_two_misc(operands, 1, 1, 0b11011)
                }
            } else if scalar_fp_pair(operands, "fcvtzu") {
                encode_fp_convert_scalar("fcvtzu", operands)
            } else {
                encode_fcvt_rounding(operands, 0b11, 0b001)
            }
        }
        // Vector forms of the eight rounding conversions (`fcvtns v0.4s,
        // v1.4s` = 0x4e21a820, `fcvtps v0.4h, v1.4h` = 0x0ef9a820, all
        // 5 arrangements x 8 ops measured under the matrix law).  Without
        // this branch the vector spellings fell through to
        // `encode_fcvt_rounding`, which demands a register-file pair and
        // rejected them all.
        "fcvtas" | "fcvtau" | "fcvtns" | "fcvtnu" | "fcvtms" | "fcvtmu" | "fcvtps"
        | "fcvtpu"
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) =>
        {
            let (u, size_hi, op) = match mn.as_str() {
                "fcvtas" => (0, 0, 0b11100),
                "fcvtau" => (1, 0, 0b11100),
                "fcvtns" => (0, 0, 0b11010),
                "fcvtnu" => (1, 0, 0b11010),
                "fcvtms" => (0, 0, 0b11011),
                "fcvtmu" => (1, 0, 0b11011),
                "fcvtps" => (0, 1, 0b11010),
                _ => (1, 1, 0b11010), // fcvtpu
            };
            encode_neon_float_two_misc(operands, u, size_hi, op)
        }
        // The remaining eight rounding conversions have no fixed-point form and
        // no general-purpose-destination *and* register-file ambiguity to
        // resolve: when both operands are floating-point registers the form is
        // the scalar SIMD&FP one (`fcvtas s0,s1`), otherwise the result goes to
        // a general-purpose register (`fcvtas w0,s1`).
        "fcvtas" | "fcvtau" | "fcvtns" | "fcvtnu" | "fcvtms" | "fcvtmu" | "fcvtps"
        | "fcvtpu"
            if scalar_fp_pair(operands, &mn) =>
        {
            encode_fp_convert_scalar(&mn, operands)
        }
        "fcvtas" => encode_fcvt_rounding(operands, 0b00, 0b100),
        "fcvtau" => encode_fcvt_rounding(operands, 0b00, 0b101),
        "fcvtns" => encode_fcvt_rounding(operands, 0b00, 0b000),
        "fcvtnu" => encode_fcvt_rounding(operands, 0b00, 0b001),
        "fcvtms" => encode_fcvt_rounding(operands, 0b10, 0b000),
        "fcvtmu" => encode_fcvt_rounding(operands, 0b10, 0b001),
        "fcvtps" => encode_fcvt_rounding(operands, 0b01, 0b000),
        "fcvtpu" => encode_fcvt_rounding(operands, 0b01, 0b001),
        "ucvtf" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                if operands.get(2).is_some() {
                    encode_neon_float_two_misc_fixed(operands, 1, false)
                } else {
                    encode_neon_float_two_misc(operands, 1, 0, 0b11101)
                }
            } else if scalar_fp_pair(operands, "ucvtf") {
                encode_fp_convert_scalar("ucvtf", operands)
            } else {
                encode_ucvtf(operands)
            }
        }
        "scvtf" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                if operands.get(2).is_some() {
                    encode_neon_float_two_misc_fixed(operands, 0, false)
                } else {
                    encode_neon_float_two_misc(operands, 0, 0, 0b11101)
                }
            } else if scalar_fp_pair(operands, "scvtf") {
                encode_fp_convert_scalar("scvtf", operands)
            } else {
                encode_scvtf(operands)
            }
        }
        "fcvt" => encode_fcvt_precision(operands),
        "fcvtl" => encode_neon_fcvtl(operands, false),
        "fcvtl2" => encode_neon_fcvtl(operands, true),
        "fcvtn" => encode_neon_fcvtn(operands, false),
        "fcvtn2" => encode_neon_fcvtn(operands, true),
        // NEON float three-same instructions (vector-only)
        "fmla" => {
            if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
                encode_neon_float_elem(operands, 0b0001, 0, "fmla")
            } else {
                encode_neon_float_three_same(operands, 0, 0, 0b11001)
            }
        }
        "fmls" => {
            if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
                encode_neon_float_elem(operands, 0b0101, 0, "fmls")
            } else {
                encode_neon_float_three_same(operands, 0, 1, 0b11001)
            }
        }
        "frecps" => encode_neon_float_three_same(operands, 0, 0, 0b11111),
        "frsqrts" => encode_neon_float_three_same(operands, 0, 1, 0b11111),
        "fcmeq" => {
            if matches!(operands.get(2), Some(Operand::Imm(0))) {
                encode_neon_float_cmp_zero(operands, raw_operands, 0, 1, 0b01101)
            } else {
                encode_neon_float_three_same(operands, 0, 0, 0b11100)
            }
        }
        "fcmge" => {
            if matches!(operands.get(2), Some(Operand::Imm(0))) {
                encode_neon_float_cmp_zero(operands, raw_operands, 1, 1, 0b01100)
            } else {
                encode_neon_float_three_same(operands, 1, 0, 0b11100)
            }
        }
        "fcmgt" => {
            if matches!(operands.get(2), Some(Operand::Imm(0))) {
                encode_neon_float_cmp_zero(operands, raw_operands, 0, 1, 0b01100)
            } else {
                encode_neon_float_three_same(operands, 1, 1, 0b11100)
            }
        }
        "fcmle" => encode_neon_float_cmp_zero(operands, raw_operands, 1, 1, 0b01101),
        "fcmlt" => encode_neon_float_cmp_zero(operands, raw_operands, 0, 1, 0b01110),
        "facge" => encode_neon_float_three_same(operands, 1, 0, 0b11101),
        "facgt" => encode_neon_float_three_same(operands, 1, 1, 0b11101),

        // NEON/SIMD
        "cnt" => encode_cnt(operands),
        "cmeq" => {
            // CMEQ has two forms:
            // - CMEQ Vd, Vn, Vm (three-same, U=1): compare registers
            // - CMEQ Vd, Vn, #0 (two-reg misc, U=0): compare to zero
            if matches!(operands.get(2), Some(Operand::Imm(0))) {
                encode_neon_cmp_zero(operands, 0, 0b01001)
            } else {
                encode_neon_three_same(operands, 1, 0b10001)
            }
        }
        "cmhi" => encode_neon_three_same(operands, 1, 0b00110),
        "cmhs" => encode_neon_three_same(operands, 1, 0b00111),
        "cmge" => {
            if matches!(operands.get(2), Some(Operand::Imm(0))) {
                encode_neon_cmp_zero(operands, 1, 0b01000) // CMGE #0
            } else {
                encode_neon_three_same(operands, 0, 0b00111)
            }
        }
        "cmgt" => {
            if matches!(operands.get(2), Some(Operand::Imm(0))) {
                encode_neon_cmp_zero(operands, 0, 0b01000) // CMGT #0
            } else {
                encode_neon_three_same(operands, 0, 0b00110)
            }
        }
        "cmtst" => encode_neon_three_same(operands, 0, 0b10001),
        "uqsub" => encode_neon_three_same(operands, 1, 0b00101),
        "sqsub" => encode_neon_three_same(operands, 0, 0b00101),
        "uhadd" => encode_neon_three_same(operands, 1, 0b00000),
        "shadd" => encode_neon_three_same(operands, 0, 0b00000),
        "urhadd" => encode_neon_three_same(operands, 1, 0b00010),
        "srhadd" => encode_neon_three_same(operands, 0, 0b00010),
        "uhsub" => encode_neon_three_same(operands, 1, 0b00100),
        "shsub" => encode_neon_three_same(operands, 0, 0b00100),
        "umax" => encode_neon_three_same(operands, 1, 0b01100),
        "smax" => encode_neon_three_same(operands, 0, 0b01100),
        "umin" => encode_neon_three_same(operands, 1, 0b01101),
        "smin" => encode_neon_three_same(operands, 0, 0b01101),
        "uabd" => encode_neon_three_same(operands, 1, 0b01110),
        "sabd" => encode_neon_three_same(operands, 0, 0b01110),
        "uaba" => encode_neon_three_same(operands, 1, 0b01111),
        "saba" => encode_neon_three_same(operands, 0, 0b01111),
        "uqadd" => encode_neon_three_same(operands, 1, 0b00001),
        "sqadd" => encode_neon_three_same(operands, 0, 0b00001),
        "sshl" => encode_neon_three_same(operands, 0, 0b01000),
        "ushl" => encode_neon_three_same(operands, 1, 0b01000),
        "sqshl" => {
            if matches!(operands.get(2), Some(Operand::Imm(_))) {
                encode_neon_shift_left_imm(operands, 0, 0b01110)
            } else {
                encode_neon_three_same(operands, 0, 0b01001)
            }
        }
        "uqshl" => {
            if matches!(operands.get(2), Some(Operand::Imm(_))) {
                encode_neon_shift_left_imm(operands, 1, 0b01110)
            } else {
                encode_neon_three_same(operands, 1, 0b01001)
            }
        }
        "srshl" => encode_neon_three_same(operands, 0, 0b01010),
        "urshl" => encode_neon_three_same(operands, 1, 0b01010),
        "sqrshl" => encode_neon_three_same(operands, 0, 0b01011),
        "uqrshl" => encode_neon_three_same(operands, 1, 0b01011),
        "addp" => {
            if operands.len() == 2
                && matches!(operands.first(), Some(Operand::Reg(r)) if r.starts_with('d') || r.starts_with('D'))
            {
                // Scalar ADDP: addp Dd, Vn.2d
                encode_neon_scalar_addp(operands)
            } else {
                encode_neon_three_same(operands, 0, 0b10111)
            }
        }
        "uminp" => encode_neon_three_same(operands, 1, 0b10101),
        "umaxp" => encode_neon_three_same(operands, 1, 0b10100),
        "sminp" => encode_neon_three_same(operands, 0, 0b10101),
        "smaxp" => encode_neon_three_same(operands, 0, 0b10100),
        // NEON two-reg misc (integer)
        "abs" => encode_neon_two_misc(operands, 0, 0b01011),
        // neg dispatch moved to early scalar section
        "cls" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_two_misc(operands, 0, 0b00100)
            } else {
                encode_cls(operands)
            }
        }
        "clz" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_two_misc(operands, 1, 0b00100)
            } else {
                encode_clz(operands)
            }
        }
        "rev16" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_two_misc(operands, 0, 0b00001)
            } else {
                encode_rev16(operands)
            }
        }
        "rev32" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_two_misc(operands, 1, 0b00000)
            } else {
                encode_rev32(operands)
            }
        }
        "saddlp" => encode_neon_pairwise_long(operands, 0, 0b00010),
        "uaddlp" => encode_neon_pairwise_long(operands, 1, 0b00010),
        "sadalp" => encode_neon_pairwise_long(operands, 0, 0b00110),
        "uadalp" => encode_neon_pairwise_long(operands, 1, 0b00110),
        "sqxtun" => encode_neon_two_misc_narrow(operands, 1, 0b10010, false),
        "sqxtun2" => encode_neon_two_misc_narrow(operands, 1, 0b10010, true),
        "sqabs" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_two_misc(operands, 0, 0b00111)
            } else {
                encode_neon_scalar_two_misc(operands, 0, 0b00111)
            }
        }
        // SQNEG is SQABS with U=1 (same two-reg-misc opcode field): GAS
        // emits `sqneg b16, b2` = 0x7e207850 / `sqneg v16.8b, v2.8b` =
        // 0x2e207850.  Passing U=0 and opcode 0b01000 produced 0x5e208850
        // (the unallocated 011110-class word).
        "sqneg" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_two_misc(operands, 1, 0b00111)
            } else {
                encode_neon_scalar_two_misc(operands, 1, 0b00111)
            }
        }
        // Compare to zero forms
        "cmlt" => encode_neon_cmp_zero(operands, 0, 0b01010), // CMLT #0
        "cmle" => encode_neon_cmp_zero(operands, 1, 0b01001), // CMLE #0
        // NEON shift right narrow
        "shrn" => encode_neon_shrn(operands, 0b100001, false),
        "shrn2" => encode_neon_shrn(operands, 0b100001, true),
        "rshrn" => encode_neon_shrn(operands, 0b100011, false),
        "rshrn2" => encode_neon_shrn(operands, 0b100011, true),
        // NEON shift right accumulate and rounding shift right
        "srshr" => encode_neon_shift_right(operands, 0, 0b001001),
        "urshr" => encode_neon_shift_right(operands, 1, 0b001001),
        "ssra" => encode_neon_shift_right(operands, 0, 0b000101),
        "usra" => encode_neon_shift_right(operands, 1, 0b000101),
        "srsra" => encode_neon_shift_right(operands, 0, 0b001101),
        "ursra" => encode_neon_shift_right(operands, 1, 0b001101),
        // NEON shift left long
        "ushll" => encode_neon_shll(operands, 1, false),
        "ushll2" => encode_neon_shll(operands, 1, true),
        "sshll" => encode_neon_shll(operands, 0, false),
        "sshll2" => encode_neon_shll(operands, 0, true),
        // NEON three-different extras
        "uabal" => encode_neon_three_diff(operands, 1, 0b0101, false),
        "uabal2" => encode_neon_three_diff(operands, 1, 0b0101, true),
        "sabal" => encode_neon_three_diff(operands, 0, 0b0101, false),
        "sabal2" => encode_neon_three_diff(operands, 0, 0b0101, true),
        "uabdl" => encode_neon_three_diff(operands, 1, 0b0111, false),
        "uabdl2" => encode_neon_three_diff(operands, 1, 0b0111, true),
        "sabdl" => encode_neon_three_diff(operands, 0, 0b0111, false),
        "sabdl2" => encode_neon_three_diff(operands, 0, 0b0111, true),
        // ADDHN/RADDHN/SUBHN/RSUBHN (narrowing high)
        "addhn" => encode_neon_three_diff_narrow(operands, 0, 0b0100, false),
        "addhn2" => encode_neon_three_diff_narrow(operands, 0, 0b0100, true),
        "raddhn" => encode_neon_three_diff_narrow(operands, 1, 0b0100, false),
        "raddhn2" => encode_neon_three_diff_narrow(operands, 1, 0b0100, true),
        "subhn" => encode_neon_three_diff_narrow(operands, 0, 0b0110, false),
        "subhn2" => encode_neon_three_diff_narrow(operands, 0, 0b0110, true),
        "rsubhn" => encode_neon_three_diff_narrow(operands, 1, 0b0110, false),
        "rsubhn2" => encode_neon_three_diff_narrow(operands, 1, 0b0110, true),
        // NEON sat shift right narrow
        "sqshrn" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_qshrn(operands, 0, false, false)
            } else {
                encode_neon_scalar_qshrn(operands, 0, 0b100101)
            }
        }
        "sqshrn2" => encode_neon_qshrn(operands, 0, false, true),
        // Scalar forms (FP destination, no arrangement) share the 0x5f
        // class with SQSHRN; sending them to the vector encoder rejected
        // every scalar row.
        "uqshrn" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_qshrn(operands, 1, false, false)
            } else {
                encode_neon_scalar_qshrn(operands, 1, 0b100101)
            }
        }
        "uqshrn2" => encode_neon_qshrn(operands, 1, false, true),
        "sqrshrn" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_qshrn(operands, 0, true, false)
            } else {
                encode_neon_scalar_qshrn(operands, 0, 0b100111)
            }
        }
        "sqrshrn2" => encode_neon_qshrn(operands, 0, true, true),
        "uqrshrn" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_qshrn(operands, 1, true, false)
            } else {
                encode_neon_scalar_qshrn(operands, 1, 0b100111)
            }
        }
        "uqrshrn2" => encode_neon_qshrn(operands, 1, true, true),
        "sqrshrun" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_sqshrun(operands, true, false)
            } else {
                // Scalar SQRSHRUN: 0x7f class, opcode 100011 (GAS
                // `sqrshrun h7, s3, #9` = 0x7f178c67).
                encode_neon_scalar_qshrn(operands, 1, 0b100011)
            }
        }
        "sqrshrun2" => encode_neon_sqshrun(operands, true, true),
        // NEON permute: TRN1/TRN2
        "trn1" => encode_neon_zip_uzp(operands, 0b010, false),
        "trn2" => encode_neon_zip_uzp(operands, 0b110, false),
        // NEON replicate loads
        "ld2r" => encode_neon_ldnr(operands, 2),
        "ld3r" => encode_neon_ldnr(operands, 3),
        "ld4r" => encode_neon_ldnr(operands, 4),
        "shll" => encode_neon_shll_alias(operands, false),
        "shll2" => encode_neon_shll_alias(operands, true),
        "ushr" => encode_neon_ushr(operands),
        "sshr" => encode_neon_sshr(operands),
        "shl" => encode_neon_shl(operands),
        "sli" => encode_neon_sli(operands),
        "sri" => encode_neon_sri(operands),
        "ext" => encode_neon_ext(operands),
        "addv" => encode_neon_addv(operands),
        "umaxv" => encode_neon_across(operands, 1, 0b01010),
        "uminv" => encode_neon_across(operands, 1, 0b11010),
        "fmaxv" => encode_neon_fp_across(operands, "fmaxv", 0b00, 0b01111),
        "fminv" => encode_neon_fp_across(operands, "fminv", 0b10, 0b01111),
        "fmaxnmv" => encode_neon_fp_across(operands, "fmaxnmv", 0b00, 0b01100),
        "fminnmv" => encode_neon_fp_across(operands, "fminnmv", 0b10, 0b01100),
        "smov" => encode_neon_umov_smov(operands, "smov", true),
        "smaxv" => encode_neon_across(operands, 0, 0b01010),
        "sminv" => encode_neon_across(operands, 0, 0b11010),
        "umov" => encode_neon_umov_smov(operands, "umov", false),
        "dup" => encode_neon_dup(operands),
        "ins" => encode_neon_ins(operands),
        "not" => encode_neon_not(operands),
        "movi" => encode_neon_movi(operands),
        "bic" => encode_bic(operands),
        "bsl" => encode_neon_bsl(operands),
        "bit" => encode_neon_bitwise_insert(operands, 0b10),
        "bif" => encode_neon_bitwise_insert(operands, 0b11),
        "faddp" => encode_neon_faddp(operands),
        "saddlv" => encode_neon_across_long(operands, 0, 0b00011),
        "uaddlv" => encode_neon_across_long(operands, 1, 0b00011),
        "sqdmlal" => {
            if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
                encode_neon_elem_long(operands, 0, 0b0011, false)
            } else {
                encode_neon_three_diff(operands, 0, 0b1001, false)
            }
        }
        "sqdmlal2" => {
            if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
                encode_neon_elem_long(operands, 0, 0b0011, true)
            } else {
                encode_neon_three_diff(operands, 0, 0b1001, true)
            }
        }
        "sqdmlsl" => {
            if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
                encode_neon_elem_long(operands, 0, 0b0111, false)
            } else {
                encode_neon_three_diff(operands, 0, 0b1011, false)
            }
        }
        "sqdmlsl2" => {
            if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
                encode_neon_elem_long(operands, 0, 0b0111, true)
            } else {
                encode_neon_three_diff(operands, 0, 0b1011, true)
            }
        }
        "sqdmull" => {
            if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
                encode_neon_elem_long(operands, 0, 0b1011, false)
            } else {
                encode_neon_three_diff(operands, 0, 0b1101, false)
            }
        }
        "sqdmull2" => {
            if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
                encode_neon_elem_long(operands, 0, 0b1011, true)
            } else {
                encode_neon_three_diff(operands, 0, 0b1101, true)
            }
        }
        "sqdmulh" => {
            if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
                encode_neon_elem(operands, 0, 0b1100)
            } else {
                encode_neon_three_same(operands, 0, 0b10110)
            }
        }
        "sqrdmulh" => {
            if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
                encode_neon_elem(operands, 0, 0b1101)
            } else {
                encode_neon_three_same(operands, 1, 0b10110)
            }
        }
        "pmul" => encode_neon_pmul(operands),
        "mla" => {
            if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
                encode_neon_elem(operands, 1, 0b0000)
            } else {
                encode_neon_mla(operands)
            }
        }
        "mls" => {
            if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
                encode_neon_elem(operands, 1, 0b0100)
            } else {
                encode_neon_mls(operands)
            }
        }
        "rev64" => encode_neon_rev64(operands),
        "tbl" => encode_neon_tbl(operands),
        "tbx" => encode_neon_tbx(operands),
        "ld1" => encode_neon_ld_st_dispatch(operands, true, 1),
        "ld1r" => encode_neon_ld1r(operands),
        "ld2" => encode_neon_ld_st_dispatch(operands, true, 2),
        "ld3" => encode_neon_ld_st_dispatch(operands, true, 3),
        "ld4" => encode_neon_ld_st_dispatch(operands, true, 4),
        "st1" => encode_neon_ld_st_dispatch(operands, false, 1),
        "st2" => encode_neon_ld_st_dispatch(operands, false, 2),
        "st3" => encode_neon_ld_st_dispatch(operands, false, 3),
        "st4" => encode_neon_ld_st_dispatch(operands, false, 4),
        "uzp1" => encode_neon_zip_uzp(operands, 0b001, false),
        "uzp2" => encode_neon_zip_uzp(operands, 0b101, false),
        "zip1" => encode_neon_zip_uzp(operands, 0b011, false),
        "zip2" => encode_neon_zip_uzp(operands, 0b111, false),
        "eor3" => encode_neon_eor3(operands),
        "pmull" => encode_neon_pmull(operands, false),
        "pmull2" => encode_neon_pmull(operands, true),
        "aese" => encode_neon_aes(operands, 0b00100),
        "aesd" => encode_neon_aes(operands, 0b00101),
        "aesmc" => encode_neon_aes(operands, 0b00110),
        "aesimc" => encode_neon_aes(operands, 0b00111),

        // NEON three-different (widening/narrowing)
        "usubl" => encode_neon_three_diff(operands, 1, 0b0010, false),
        "usubl2" => encode_neon_three_diff(operands, 1, 0b0010, true),
        "ssubl" => encode_neon_three_diff(operands, 0, 0b0010, false),
        "ssubl2" => encode_neon_three_diff(operands, 0, 0b0010, true),
        "usubw" => encode_neon_three_diff(operands, 1, 0b0011, false),
        "usubw2" => encode_neon_three_diff(operands, 1, 0b0011, true),
        "ssubw" => encode_neon_three_diff(operands, 0, 0b0011, false),
        "ssubw2" => encode_neon_three_diff(operands, 0, 0b0011, true),
        "uaddl" => encode_neon_three_diff(operands, 1, 0b0000, false),
        "uaddl2" => encode_neon_three_diff(operands, 1, 0b0000, true),
        "saddl" => encode_neon_three_diff(operands, 0, 0b0000, false),
        "saddl2" => encode_neon_three_diff(operands, 0, 0b0000, true),
        "uaddw" => encode_neon_three_diff(operands, 1, 0b0001, false),
        "uaddw2" => encode_neon_three_diff(operands, 1, 0b0001, true),
        "saddw" => encode_neon_three_diff(operands, 0, 0b0001, false),
        "saddw2" => encode_neon_three_diff(operands, 0, 0b0001, true),
        "umlal" => {
            if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
                encode_neon_elem_long(operands, 1, 0b0010, false)
            } else {
                encode_neon_three_diff(operands, 1, 0b1000, false)
            }
        }
        "umlal2" => {
            if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
                encode_neon_elem_long(operands, 1, 0b0010, true)
            } else {
                encode_neon_three_diff(operands, 1, 0b1000, true)
            }
        }
        "smlal" => {
            if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
                encode_neon_elem_long(operands, 0, 0b0010, false)
            } else {
                encode_neon_three_diff(operands, 0, 0b1000, false)
            }
        }
        "smlal2" => {
            if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
                encode_neon_elem_long(operands, 0, 0b0010, true)
            } else {
                encode_neon_three_diff(operands, 0, 0b1000, true)
            }
        }
        "umlsl" => {
            if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
                encode_neon_elem_long(operands, 1, 0b0110, false)
            } else {
                encode_neon_three_diff(operands, 1, 0b1010, false)
            }
        }
        "umlsl2" => {
            if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
                encode_neon_elem_long(operands, 1, 0b0110, true)
            } else {
                encode_neon_three_diff(operands, 1, 0b1010, true)
            }
        }
        "smlsl" => {
            if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
                encode_neon_elem_long(operands, 0, 0b0110, false)
            } else {
                encode_neon_three_diff(operands, 0, 0b1010, false)
            }
        }
        "smlsl2" => {
            if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
                encode_neon_elem_long(operands, 0, 0b0110, true)
            } else {
                encode_neon_three_diff(operands, 0, 0b1010, true)
            }
        }
        "umull2" => {
            if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
                encode_neon_elem_long(operands, 1, 0b1010, true)
            } else {
                encode_neon_three_diff(operands, 1, 0b1100, true)
            }
        }
        "smull2" => {
            if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
                encode_neon_elem_long(operands, 0, 0b1010, true)
            } else {
                encode_neon_three_diff(operands, 0, 0b1100, true)
            }
        }

        // NEON saturating shift right narrow
        "sqshrun" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_sqshrun(operands, false, false)
            } else {
                // Scalar SQSHRUN: 0x7f class, opcode 100001 (GAS
                // `sqshrun b31, h23, #2` = 0x7f0e86ff).
                encode_neon_scalar_qshrn(operands, 1, 0b100001)
            }
        }
        "sqshrun2" => encode_neon_sqshrun(operands, false, true),

        // NEON extend long (aliases for USHLL/SSHLL #0)
        "uxtl" => encode_neon_xtl(operands, 1, false),
        "uxtl2" => encode_neon_xtl(operands, 1, true),
        "sxtl" => encode_neon_xtl(operands, 0, false),
        "sxtl2" => encode_neon_xtl(operands, 0, true),

        // NEON two-register narrowing
        "uqxtn" => encode_neon_two_misc_narrow(operands, 1, 0b10100, false),
        "uqxtn2" => encode_neon_two_misc_narrow(operands, 1, 0b10100, true),
        "sqxtn" => encode_neon_two_misc_narrow(operands, 0, 0b10100, false),
        "sqxtn2" => encode_neon_two_misc_narrow(operands, 0, 0b10100, true),
        "xtn" => encode_neon_two_misc_narrow(operands, 0, 0b10010, false),
        "xtn2" => encode_neon_two_misc_narrow(operands, 0, 0b10010, true),

        // System
        "hint" => encode_hint(operands),
        "bti" => encode_bti(raw_operands),
        "nop" => Ok(EncodeResult::Word(0xd503201f)),
        "yield" => Ok(EncodeResult::Word(0xd503203f)),
        "wfe" => Ok(EncodeResult::Word(0xd503205f)),
        "wfi" => Ok(EncodeResult::Word(0xd503207f)),
        "sev" => Ok(EncodeResult::Word(0xd503209f)),
        "sevl" => Ok(EncodeResult::Word(0xd50320bf)),
        "eret" => Ok(EncodeResult::Word(0xd69f03e0)),
        // CLREX (CRm=0b1111 = SY): GAS 2.47 encodes `clrex` as 0xd5033f5f
        // (bytes 5f3f03d5); the old 0xd503305f kept CRm=0 (a different
        // synchronization domain) and mismatched every oracle row.
        "clrex" => Ok(EncodeResult::Word(0xd5033f5f)),
        "dc" => encode_dc(operands, raw_operands),
        "tlbi" => encode_tlbi(operands, raw_operands),
        "ic" => encode_ic(raw_operands),
        "dmb" => encode_dmb(operands),
        "dsb" => encode_dsb(operands),
        "isb" => Ok(EncodeResult::Word(0xd5033fdf)),
        "mrs" => encode_mrs(operands),
        "msr" => encode_msr(operands),
        "hlt" => encode_hlt(operands),
        "svc" => encode_svc(operands),
        "hvc" => encode_hvc(operands),
        "smc" => encode_smc(operands),
        "at" => encode_at(operands, raw_operands),
        "sys" => encode_sys(raw_operands),
        // SYSL: the load-form of SYS (SYSL Xt, #op1, cCRn, cCRm, #op2).
        "sysl" => encode_sysl(raw_operands),
        "brk" => encode_brk(operands),

        // Bitfield extract/insert
        "ubfx" => encode_ubfx(operands),
        "sbfx" => encode_sbfx(operands),
        "ubfm" => encode_ubfm(operands),
        "sbfm" => encode_sbfm(operands),
        "ubfiz" => encode_ubfiz(operands),
        "sbfiz" => encode_sbfiz(operands),
        "bfm" => encode_bfm(operands),
        "bfi" => encode_bfi(operands),
        "bfxil" => encode_bfxil(operands),
        "extr" => encode_extr(operands),

        // Additional conditional operations
        "cneg" => encode_cneg(operands),
        "cinc" => encode_cinc(operands),
        "cinv" => encode_cinv(operands),

        // Bit manipulation
        "rbit" => {
            // RBIT has both scalar and NEON forms
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_rbit(operands)
            } else {
                encode_rbit(operands)
            }
        }
        "rev" => encode_rev(operands),

        // CRC32
        "crc32b" | "crc32h" | "crc32w" | "crc32x" | "crc32cb" | "crc32ch" | "crc32cw"
        | "crc32cx" => encode_crc32(mnemonic, operands),

        // Prefetch
        "prfm" => encode_prfm(operands, false),
        // PRFUM: the unscaled (imm9) form of PRFM, `prfum pldl1keep,[x0,#-8]`
        // (0xf89f8000).  The signed offset is the whole point of the form: the
        // unsigned form cannot express a negative or unaligned offset at all.
        "prfum" => encode_prfm(operands, true),

        // LSE atomics
        "cas" | "casa" | "casal" | "casl" | "casb" | "casab" | "casalb" | "caslb" | "cash"
        | "casah" | "casalh" | "caslh" => encode_cas(mnemonic, operands),
        // LSE compare-and-swap pair (kernel mm/slub.c __cmpxchg_double & co.)
        "casp" | "caspa" | "caspal" | "caspl" => encode_casp(mnemonic, operands),
        "swp" | "swpa" | "swpal" | "swpl" | "swpb" | "swpab" | "swpalb" | "swplb" | "swph"
        | "swpah" | "swpalh" | "swplh" => encode_swp(mnemonic, operands),
        "ldadd" | "ldadda" | "ldaddal" | "ldaddl" | "ldaddb" | "ldaddab" | "ldaddalb"
        | "ldaddlb" | "ldaddh" | "ldaddah" | "ldaddalh" | "ldaddlh" | "ldclr" | "ldclra"
        | "ldclral" | "ldclrl" | "ldclrb" | "ldclrab" | "ldclralb" | "ldclrlb" | "ldclrh"
        | "ldclrah" | "ldclralh" | "ldclrlh" | "ldeor" | "ldeora" | "ldeoral" | "ldeorl"
        | "ldeorb" | "ldeorab" | "ldeoralb" | "ldeorlb" | "ldeorh" | "ldeorah" | "ldeoralh"
        | "ldeorlh" | "ldset" | "ldseta" | "ldsetal" | "ldsetl" | "ldsetb" | "ldsetab"
        | "ldsetalb" | "ldsetlb" | "ldseth" | "ldsetah" | "ldsetalh" | "ldsetlh"
        // FEAT_LSE min/max: opc 100..111.  `ldsmax x0,x1,[x2]` (0xf8204021) and
        // `ldumin x0,x1,[x2]` were rejected outright although the instruction
        // exists; `ldsmaxb/ldsmaxh/...` are the same family one size down.
        | "ldsmax" | "ldsmaxa" | "ldsmaxal" | "ldsmaxl" | "ldsmaxb" | "ldsmaxab"
        | "ldsmaxalb" | "ldsmaxlb" | "ldsmaxh" | "ldsmaxah" | "ldsmaxalh" | "ldsmaxlh"
        | "ldsmin" | "ldsmina" | "ldsminal" | "ldsminl" | "ldsminb" | "ldsminab"
        | "ldsminalb" | "ldsminlb" | "ldsminh" | "ldsminah" | "ldsminalh" | "ldsminlh"
        | "ldumax" | "ldumaxa" | "ldumaxal" | "ldumaxl" | "ldumaxb" | "ldumaxab"
        | "ldumaxalb" | "ldumaxlb" | "ldumaxh" | "ldumaxah" | "ldumaxalh" | "ldumaxlh"
        | "ldumin" | "ldumina" | "lduminal" | "lduminl" | "lduminb" | "lduminab"
        | "lduminalb" | "lduminlb" | "lduminh" | "lduminah" | "lduminalh" | "lduminlh" => {
            encode_ldop(mnemonic, operands)
        }
        // LSE atomic store aliases (Rt=XZR, discard result)
        "stadd" | "staddl" | "staddb" | "staddlb" | "staddh" | "staddlh" | "stclr" | "stclrl"
        | "stclrb" | "stclrlb" | "stclrh" | "stclrlh" | "steor" | "steorl" | "steorb"
        | "steorlb" | "steorh" | "steorlh" | "stset" | "stsetl" | "stsetb" | "stsetlb"
        | "stseth" | "stsetlh" => encode_stop(mnemonic, operands),

        // NEON move-not-immediate
        "mvni" => encode_neon_mvni(operands),

        _ => {
            // TODO: handle remaining instructions
            Err(format!(
                "unsupported instruction: {} {}",
                mnemonic, raw_operands
            ))
        }
    }
}

// ── Encoding helpers ──────────────────────────────────────────────────────

/// Read a register operand as a *general-purpose* register: its number and
/// whether it is 64-bit.
///
/// This used to be `parse_reg_num` + `is_64bit_reg`, which between them accept
/// every register spelling the parser can produce -- `parse_reg_num` collapses
/// `d0`, `s0`, `h0`, `b0`, `q0` and `v0` onto the same number as `x0`. A
/// caller therefore had no way to say "GP slot" and no way to be told it had
/// been handed a floating-point register: `mul d0, x1, x2` parsed `d0` as
/// register 0 within the *GP* multiply and emitted `mul x0, x1, x2`. The
/// 1,624-row `sweep register class` group in the operand-legality matrix
/// exists to pin every such slot, and it found 401 of them.
///
/// The GP grammar is now exactly [`GpReg::by_name`]'s -- `x0`-`x30`,
/// `w0`-`w30`, `lr`, `sp`/`wsp`, `xzr`/`wzr` -- so a floating-point or vector
/// spelling is refused here, at the operand that has it, and the message names
/// the two grammars instead of the number the parser collapsed it onto.
/// Callers that own a *floating-point* slot want [`fp_reg`]; the ones that own
/// a slot which genuinely accepts both register files (the `LDR`/`STR` data
/// register, `FMOV`'s mixed forms, `MRS`/`MSR`'s numerics) ask for what they
/// mean explicitly.
pub(crate) fn get_reg(operands: &[Operand], idx: usize) -> Result<(u32, bool), String> {
    match operands.get(idx) {
        Some(Operand::Reg(name)) => match GpReg::by_name(name) {
            Some(r) => Ok((r.num, r.is_64)),
            None => Err(format!(
                "operand {idx} `{name}` is not a general-purpose register; this slot \
                 takes x0-x30, w0-w30, lr, sp, wsp, xzr or wzr{}",
                if is_fp_spelling_pub(name) {
                    format!(
                        " (`{name}` is a floating-point/SIMD register, which is a \
                         different register file -- check the instruction's form)"
                    )
                } else {
                    String::new()
                }
            )),
        },
        other => Err(format!(
            "expected register at operand {}, got {:?}",
            idx, other
        )),
    }
}

/// Read a register operand as a floating-point/SIMD register: its number and
/// its width letter, lower-cased.
///
/// Returns the letter (`b`, `h`, `s`, `d`, `q` or `v`) rather than a width
/// enum because the families disagree about which of them are *spellable*:
/// scalar arithmetic takes `h`/`s`/`d` only, rounding-precision conversions
/// add `h`, the load/store data register takes all six, and `v<n>` without an
/// arrangement is only legal where the form does not need one.  Deciding that
/// at the call site keeps the reader honest about the class and leaves each
/// family's own grammar visible where it is enforced.
pub(crate) fn fp_reg(
    operands: &[Operand],
    idx: usize,
    mn: &str,
) -> Result<(u32, u8, String), String> {
    let name = match operands.get(idx) {
        Some(Operand::Reg(n)) => n.as_str(),
        Some(_) => {
            return Err(format!(
                "{mn}: operand {idx} must be a register, not an immediate or memory operand"
            ));
        }
        None => return Err(format!("{mn}: missing operand {idx} (expected a register)")),
    };
    let lower = name.to_ascii_lowercase();
    let Some((&first, rest)) = lower.as_bytes().split_first() else {
        return Err(format!("{mn}: operand {idx} is an empty register name"));
    };
    if !matches!(first, b'b' | b'h' | b's' | b'd' | b'q' | b'v') || rest.is_empty() {
        return Err(format!(
            "{mn}: operand {idx} `{name}` is not a floating-point register; this slot \
             takes b0-b31, h0-h31, s0-s31, d0-d31, q0-q31 or v0-v31, and `{name}` is a \
             general-purpose register if it is a register at all"
        ));
    }
    if rest.len() > 2
        || !rest.iter().all(u8::is_ascii_digit)
        || (rest.len() == 2 && rest[0] == b'0')
    {
        return Err(format!(
            "{mn}: operand {idx} `{name}` is not a canonical register number"
        ));
    }
    let mut num = 0u32;
    for &d in rest {
        num = num * 10 + u32::from(d - b'0');
    }
    if num > 31 {
        return Err(format!(
            "{mn}: operand {idx} `{name}` is out of range; the register file has 32 \
             registers (0-31)"
        ));
    }
    Ok((num, first, name.to_string()))
}

/// The scalar-FP `type` field for a width letter.
///
/// ```text
/// S (single, 32-bit) -> 0b00
/// D (double, 64-bit) -> 0b01
/// H (half, 16-bit)  -> 0b11
/// ```
pub(crate) fn fp_ftype_letter(letter: u8, mn: &str) -> Result<u32, String> {
    match letter {
        b's' => Ok(0b00),
        b'd' => Ok(0b01),
        b'h' => Ok(0b11),
        _ => Err(format!(
            "{mn}: this instruction has no `{}`-register form; it takes h, s or d",
            char::from(letter)
        )),
    }
}

fn get_imm(operands: &[Operand], idx: usize) -> Result<i64, String> {
    match operands.get(idx) {
        Some(Operand::Imm(v)) => Ok(*v),
        other => Err(format!(
            "expected immediate at operand {}, got {:?}",
            idx, other
        )),
    }
}

fn get_symbol(operands: &[Operand], idx: usize) -> Result<(String, i64), String> {
    match operands.get(idx) {
        Some(Operand::Symbol(s)) => Ok((s.clone(), 0)),
        Some(Operand::Label(s)) => Ok((s.clone(), 0)),
        Some(Operand::SymbolOffset(s, off)) => Ok((s.clone(), *off)),
        Some(Operand::Modifier { symbol, .. }) => Ok((symbol.clone(), 0)),
        Some(Operand::ModifierOffset { symbol, offset, .. }) => Ok((symbol.clone(), *offset)),
        // The parser misclassifies symbol names that collide with register names,
        // condition codes, or barrier names. These are valid symbols in context.
        Some(Operand::Reg(name)) => Ok((name.clone(), 0)),
        Some(Operand::Cond(name)) => Ok((name.clone(), 0)),
        Some(Operand::Barrier(name)) => Ok((name.clone(), 0)),
        other => Err(format!(
            "expected symbol at operand {}, got {:?}",
            idx, other
        )),
    }
}

fn sf_bit(is_64: bool) -> u32 {
    if is_64 { 1 } else { 0 }
}

// =============================================================================
// Register-name parsing (upstream fork issues #118 and #207)
// =============================================================================
#[cfg(test)]
mod parse_reg_num_tests {
    use super::*;

    #[test]
    fn accepts_canonical_aarch64_register_spellings() {
        for (name, want) in [
            ("x0", 0),
            ("x30", 30),
            ("w0", 0),
            ("w30", 30),
            ("d31", 31),
            ("s31", 31),
            ("v15", 15),
            ("h7", 7),
            ("b3", 3),
            ("q12", 12),
            ("sp", 31),
            ("wsp", 31),
            ("xzr", 31),
            ("wzr", 31),
            ("lr", 30),
            ("X9", 9), // case-insensitive
            ("W9", 9),
        ] {
            assert_eq!(
                parse_reg_num(name),
                Some(want),
                "parse_reg_num({name:?}) should be Some({want})"
            );
        }
    }

    /// `str::parse::<u32>` accepts a leading `+` and leading zeros; the
    /// AArch64 register grammar does not. These used to resolve to a real
    /// register instead of being rejected.
    #[test]
    fn rejects_malformed_register_spellings() {
        for name in [
            "x+5", "x+0", "w+31", "x007", "w007", "x00", "x05", "d007", "x", "w", "x32", "w32",
            "x99", "x-1", "y5", "5", "", "x 5", "x5x", "x1_0",
        ] {
            assert_eq!(
                parse_reg_num(name),
                None,
                "parse_reg_num({name:?}) should be None"
            );
        }
    }

    /// Leading zeros are the subtle one: "x007" looks harmless but GAS treats
    /// it as an unknown symbol, and silently accepting it here would let a
    /// typo assemble into a branch to the wrong register.
    #[test]
    fn rejects_leading_zero_register_numbers() {
        // Single "0" is canonical.
        assert_eq!(parse_reg_num("x0"), Some(0));
        // Anything with a leading zero and more digits is not.
        assert_eq!(parse_reg_num("x00"), None);
        assert_eq!(parse_reg_num("x01"), None);
        assert_eq!(parse_reg_num("x007"), None);
        assert_eq!(parse_reg_num("x030"), None);
    }
}

#[cfg(test)]
mod register_class_tests {
    use super::*;

    fn reg(r: &str) -> Operand {
        Operand::Reg(r.to_string())
    }
    fn cond(c: &str) -> Operand {
        Operand::Cond(c.to_string())
    }
    fn enc(mn: &str, ops: &[Operand]) -> u32 {
        match encode_instruction(mn, ops, "") {
            Ok(EncodeResult::Word(w)) => w,
            Ok(other) => panic!("{mn}: expected a single word, got {other:?}"),
            Err(e) => panic!("{mn}: expected an encoding, got error: {e}"),
        }
    }

    /// ADC/SBC have one `sf` for all three slots, and every slot reads
    /// encoding 31 as the zero register.  All four of those facts used to be
    /// fail-open: SP in the destination and in the third operand assembled as
    /// the zero register, and a 32-bit operand next to 64-bit ones was ignored.
    #[test]
    fn add_subtract_with_carry_is_one_width_and_refuses_the_stack_pointer() {
        assert_eq!(enc("adc", &[reg("xzr"), reg("x1"), reg("x2")]), 0x9a02003f);
        assert_eq!(enc("adc", &[reg("x0"), reg("xzr"), reg("x2")]), 0x9a0203e0);
        assert_eq!(enc("adc", &[reg("x0"), reg("x1"), reg("xzr")]), 0x9a1f0020);
        assert_eq!(enc("adcs", &[reg("wzr"), reg("w1"), reg("w2")]), 0x3a02003f);
        assert_eq!(enc("sbc", &[reg("x0"), reg("x1"), reg("xzr")]), 0xda1f0020);
        assert_eq!(enc("sbc", &[reg("wzr"), reg("w1"), reg("w2")]), 0x5a02003f);
        assert_eq!(enc("sbcs", &[reg("wzr"), reg("w1"), reg("w2")]), 0x7a02003f);
        for (mn, ops) in [
            ("adc", vec![reg("sp"), reg("x1"), reg("x2")]),
            ("adc", vec![reg("wsp"), reg("w1"), reg("w2")]),
            ("adc", vec![reg("x0"), reg("x1"), reg("sp")]),
            ("adc", vec![reg("x0"), reg("x1"), reg("w0")]),
            ("adc", vec![reg("w0"), reg("x1"), reg("x2")]),
            ("adc", vec![reg("xzr"), reg("x1"), reg("w2")]),
            ("sbc", vec![reg("x0"), reg("x1"), reg("wzr")]),
        ] {
            assert!(
                encode_instruction(mn, &ops, "").is_err(),
                "{mn} {ops:?} has no encoding"
            );
        }
    }

    /// The conditional-select aliases: the destination is an `Rd|XZR` slot and
    /// the two register operands share one width, so `cset sp, eq` (which used
    /// to assemble as `cset xzr, eq`) and the mixed-width forms are refused.
    #[test]
    fn conditional_selects_refuse_the_stack_pointer() {
        assert_eq!(enc("cset", &[reg("xzr"), cond("eq")]), 0x9a9f17ff);
        assert_eq!(enc("csetm", &[reg("wzr"), cond("ne")]), 0x5a9f03ff);
        assert_eq!(
            enc("cinc", &[reg("xzr"), reg("x1"), cond("eq")]),
            0x9a81143f
        );
        assert_eq!(
            enc("cinv", &[reg("x0"), reg("xzr"), cond("eq")]),
            0xda9f13e0
        );
        assert_eq!(
            enc("cneg", &[reg("xzr"), reg("xzr"), cond("eq")]),
            0xda9f17ff
        );
        for (mn, ops) in [
            ("cset", vec![reg("sp"), cond("eq")]),
            ("csetm", vec![reg("wsp"), cond("ne")]),
            ("cinc", vec![reg("sp"), reg("x1"), cond("eq")]),
            ("cinc", vec![reg("w0"), reg("x1"), cond("eq")]),
            ("cinv", vec![reg("xzr"), reg("w1"), cond("eq")]),
            ("cneg", vec![reg("w0"), reg("x1"), cond("eq")]),
        ] {
            assert!(
                encode_instruction(mn, &ops, "").is_err(),
                "{mn} {ops:?} has no encoding"
            );
        }
    }
}

// ── GAS 2.47 golden-word ratchet for encoder defect fixes ────────────────
//
// Every word below was produced by GAS 2.47 (`~/.cache/gas-2.47-…/bin/as`,
// aarch64) from the exact instruction text; the failure modes were found by
// `scripts/gen_aarch64_dispatcher_goldens.py` (differential over the aarch64
// dispatcher arms).  Keep these rows literal: they are the regression locks
// for silent-wrong-word bugs, not documentation.
#[cfg(test)]
mod gas247_golden_word_tests {
    use super::super::parser::{AsmStatement, parse_asm};
    use super::*;

    fn encode1(text: &str) -> u32 {
        encode1_impl(text, false)
    }

    /// Like `encode1`, but accepts a `WordWithReloc` and returns its base
    /// word -- that is what the .o's `.text` bytes (and therefore the
    /// dispatcher-goldens TSV) record for relocation-carrying forms such
    /// as `adrp x0, foo`.  All other callers keep the strict
    /// "no relocs in golden rows" assertion above.
    fn encode1_base(text: &str) -> u32 {
        encode1_impl(text, true)
    }

    fn encode1_impl(text: &str, allow_reloc: bool) -> u32 {
        let stmts = parse_asm(text).unwrap_or_else(|e| panic!("parse {text:?}: {e}"));
        let mut words = Vec::new();
        for st in &stmts {
            if let AsmStatement::Instruction {
                mnemonic,
                operands,
                raw_operands,
            } = st
            {
                match encode_instruction(mnemonic, operands, raw_operands) {
                    Ok(EncodeResult::Word(w)) => words.push(w),
                    Ok(EncodeResult::WordWithReloc { word, .. }) if allow_reloc => words.push(word),
                    Ok(_) => panic!("unexpected reloc result for {text:?}"),
                    Err(e) => panic!("encode {text:?}: {e}"),
                }
            }
        }
        assert_eq!(
            words.len(),
            1,
            "expected exactly one instruction in {text:?}"
        );
        words[0]
    }

    fn encode1_err(text: &str) -> String {
        let stmts = parse_asm(text).unwrap_or_else(|e| panic!("parse {text:?}: {e}"));
        for st in &stmts {
            if let AsmStatement::Instruction {
                mnemonic,
                operands,
                raw_operands,
            } = st
            {
                if let Err(e) = encode_instruction(mnemonic, operands, raw_operands) {
                    return e;
                }
            }
        }
        panic!("expected an error for {text:?}, but it encoded")
    }

    fn le(bytes: [u8; 4]) -> u32 {
        u32::from_le_bytes(bytes)
    }

    #[test]
    fn widening_three_diff_takes_q_from_high_and_size_from_vm() {
        // saddw/saddw2/ssubw/usubw/uaddw2 families: Q = "2"-suffix only,
        // size = size(Vm).  Deriving both from Vn emitted swapped Q/size.
        for (text, want) in [
            ("saddw v8.8h, v16.8h, v26.8b", 0x0e3a1208u32),
            ("saddw2 v20.4s, v13.4s, v2.8h", le([0xb4, 0x11, 0x62, 0x4e])),
            ("ssubw v31.8h, v16.8h, v25.8b", le([0x1f, 0x32, 0x39, 0x0e])),
            ("ssubw v27.2d, v9.2d, v6.2s", le([0x3b, 0x31, 0xa6, 0x0e])),
            ("ssubw2 v2.8h, v2.8h, v18.16b", le([0x42, 0x30, 0x32, 0x4e])),
            ("usubw v31.2d, v19.2d, v13.2s", le([0x7f, 0x32, 0xad, 0x2e])),
            (
                "uaddw2 v18.8h, v31.8h, v21.16b",
                le([0xf2, 0x13, 0x35, 0x6e]),
            ),
            ("uaddw v11.4s, v9.4s, v7.4h", le([0x2b, 0x11, 0x67, 0x2e])),
        ] {
            assert_eq!(encode1(text), want, "{text}");
        }
    }

    #[test]
    fn pmull_size_follows_destination_arrangement() {
        for (text, want) in [
            ("pmull v16.8h, v15.8b, v10.8b", le([0xf0, 0xe1, 0x2a, 0x0e])),
            ("pmull v17.8h, v27.8b, v18.8b", le([0x71, 0xe3, 0x32, 0x0e])),
            (
                "pmull2 v26.8h, v0.16b, v2.16b",
                le([0x1a, 0xe0, 0x22, 0x4e]),
            ),
            ("pmull2 v7.8h, v4.16b, v1.16b", le([0x87, 0xe0, 0x21, 0x4e])),
        ] {
            assert_eq!(encode1(text), want, "{text}");
        }
    }

    #[test]
    fn clrex_uses_cr15_sy() {
        assert_eq!(encode1("clrex"), 0xd5033f5f);
    }

    #[test]
    fn ld1r_ld2r_ld3r_ld4r_register_post_index() {
        for (text, want) in [
            ("ld1r {v0.8b}, [x29], x12", 0x0dccc3a0u32),
            ("ld1r {v11.2d}, [x13], x8", le([0xab, 0xcd, 0xc8, 0x4d])),
            ("ld1r {v12.8b}, [x23], x24", le([0xec, 0xc2, 0xd8, 0x0d])),
            (
                "ld2r {v14.8b-v15.8b}, [x24], x14",
                le([0x0e, 0xc3, 0xee, 0x0d]),
            ),
            (
                "ld3r {v24.8b-v26.8b}, [sp], x7",
                le([0xf8, 0xe3, 0xc7, 0x0d]),
            ),
            (
                "ld3r {v3.2s-v5.2s}, [x6], x25",
                le([0xc3, 0xe8, 0xd9, 0x0d]),
            ),
            (
                "ld4r {v19.8b-v22.8b}, [x5], x12",
                le([0xb3, 0xe0, 0xec, 0x0d]),
            ),
            (
                "ld4r {v29.8b-v0.8b}, [x3], x23",
                le([0x7d, 0xe0, 0xf7, 0x0d]),
            ),
            // No-writeback still matches GAS:
            ("ld1r {v0.8b}, [x29]", 0x0d40c3a0),
        ] {
            assert_eq!(encode1(text), want, "{text}");
        }
    }

    #[test]
    fn ld_replicate_rejects_forms_gas_rejects() {
        // GAS refuses an immediate post-index, SP/XZR Rm and extra operands
        // for LD*R; dropping them silently would change the address mode.
        for text in [
            "ld1r {v0.8b}, [x29], #8",
            "ld1r {v0.8b}, [x29], sp",
            "ld1r {v0.8b}, [x29], xzr",
            "ld2r {v0.8b-v1.8b}, [x29], #8",
            "ld3r {v0.8b-v2.8b}, [x29], w12",
        ] {
            encode1_err(text);
        }
    }

    #[test]
    fn fcvt_scalar_fp_destination_class() {
        for (text, want) in [
            // fcvtzs/fcvtzu no-immediate (0x5e class, code 1011):
            ("fcvtzs d8, d4", 0x5ee1b888u32),
            ("fcvtzs s8, s4", 0x5ea1b888),
            ("fcvtzu d7, d9", le([0x27, 0xb9, 0xe1, 0x7e])),
            // fcvtzs/fcvtzu fixed-point (0x5f class, opcode 111111):
            ("fcvtzs d18, d5, #11", le([0xb2, 0xfc, 0x75, 0x5f])),
            ("fcvtzs s11, s8, #15", le([0x0b, 0xfd, 0x31, 0x5f])),
            ("fcvtzs s19, s19, #15", le([0x73, 0xfe, 0x31, 0x5f])),
            ("fcvtzu s9, s20, #1", le([0x89, 0xfe, 0x3f, 0x7f])),
            ("fcvtzs s8, s4, #32", 0x5f20fc88),
            // The eight rounding conversions (code table A=0100 N=0010
            // P=1010 M=0011):
            ("fcvtau s0, s1", le([0x20, 0xc8, 0x21, 0x7e])),
            ("fcvtmu s10, s5", le([0xaa, 0xb8, 0x21, 0x7e])),
            ("fcvtau d8, d4", 0x7e61c888),
            ("fcvtns d8, d4", 0x5e61a888),
            ("fcvtnu d8, d4", 0x7e61a888),
            ("fcvtps d8, d4", 0x5ee1a888),
            ("fcvtpu d8, d4", 0x7ee1a888),
            ("fcvtms s8, s4", 0x5e21b888),
            // SCVTF/UCVTF scalar (code 0101; fixed opcode 111001):
            ("scvtf d8, d4", 0x5e61d888),
            ("scvtf s8, s4", 0x5e21d888),
            ("ucvtf d11, d3, #4", le([0x6b, 0xe4, 0x7c, 0x7f])),
            ("ucvtf d8, d4, #5", 0x7f7be488),
            ("scvtf d8, d4, #64", 0x5f40e488),
        ] {
            assert_eq!(encode1(text), want, "{text}");
        }
    }

    #[test]
    fn fcvt_scalar_fp_destination_rejects_what_gas_rejects() {
        for text in [
            // Cross-precision forms do not exist in this class.  (The H
            // form `fcvtzs h8, h4` used to sit here, but bare GAS default
            // arch (armv8-a) rejecting it is an arch artifact: on the
            // pinned matrix law (`.arch armv9.4-a+sme`) the scalar FP16
            // conversion exists -- see the batch-4 accept rows below.)
            "fcvtzs d8, s4",
            "fcvtzs s8, d4",
            // fbits range: 1..=32 for S, 1..=64 for D.
            "fcvtzs s8, s4, #33",
            "fcvtzs d8, d4, #65",
            "fcvtzs d8, d4, #0",
            // Rounding-mode conversions have no fixed-point form.
            "fcvtau s8, s4, #3",
        ] {
            encode1_err(text);
        }
    }

    #[test]
    fn sqneg_scalar_and_vector_set_u_bit() {
        for (text, want) in [
            ("sqneg b16, b2", le([0x50, 0x78, 0x20, 0x7e])),
            ("sqneg d3, d9", 0x7ee07923),
            ("sqneg h3, h9", 0x7e607923),
            ("sqneg s3, s9", 0x7ea07923),
            ("sqneg v16.8b, v2.8b", le([0x50, 0x78, 0x20, 0x2e])),
            // SQABS keeps U=0 (guard against over-broad fix):
            ("sqabs b16, b2", le([0x50, 0x78, 0x20, 0x5e])),
            ("sqabs v16.8b, v2.8b", le([0x50, 0x78, 0x20, 0x0e])),
        ] {
            assert_eq!(encode1(text), want, "{text}");
        }
    }

    #[test]
    fn scalar_shift_right_narrow_uses_5f_class() {
        for (text, want) in [
            ("sqshrn b31, h23, #2", le([0xff, 0x96, 0x0e, 0x5f])),
            ("sqshrn s8, d5, #10", le([0xa8, 0x94, 0x36, 0x5f])),
            ("sqshrn h23, s9, #13", 0x5f139537),
            ("sqshrn b31, h23, #8", 0x5f0896ff),
            ("sqshrn s9, d10, #32", 0x5f209549),
            ("uqshrn b1, h2, #3", 0x7f0d9441),
        ] {
            assert_eq!(encode1(text), want, "{text}");
        }
    }

    #[test]
    fn scalar_shift_right_narrow_rejects_out_of_range() {
        for text in [
            "sqshrn b31, h23, #16",
            "sqshrn b31, h23, #0",
            "sqshrn b31, h23, #9",
        ] {
            encode1_err(text);
        }
    }

    #[test]
    fn mov_element_routes_by_destination_register_file() {
        for (text, want) in [
            // SIMD&FP destination -> scalar element move (INS alias):
            ("mov s15, v5.s[3]", le([0xaf, 0x04, 0x1c, 0x5e])),
            ("mov d15, v5.d[0]", 0x5e0804af),
            // General-purpose destination -> UMOV:
            ("mov w15, v5.s[3]", le([0xaf, 0x3c, 0x1c, 0x0e])),
            // Vector destination -> INS element/element:
            ("mov v15.s[3], v5.s[3]", 0x6e1c64af),
            ("mov v15.d[0], v5.d[0]", 0x6e0804af),
            // GP-source INS unchanged:
            ("ins v15.s[3], w7", 0x4e1c1cef),
        ] {
            assert_eq!(encode1(text), want, "{text}");
        }
    }

    // ── Batch 4: dispatcher-golden campaign (post-rebase).
    //    Every word below is a verbatim GAS 2.47 measurement under the
    //    pinned matrix law (`.arch armv9.4-a+sme`) except where noted;
    //    rejects are GAS-rejected spellings. ──────────────────────────

    #[test]
    fn batch4_scalar_fp16_conversions_follow_the_matrix_arch() {
        for (text, want) in [
            // GAS default arch (armv8-a) rejects the H form; the matrix
            // pins armv9.4-a+sme where it exists.  Bare-default verdicts
            // are arch artifacts (s/d words are identical either way).
            ("fcvtzs h8, h4", 0x5ef9b888u32),
            ("ucvtf h0, h0", 0x7e79d800u32),
            ("fcvtzs h0, h0, #16", 0x5f10fc00u32),
            // imm-form doubles (arch-independent, regressed from the
            // pre-rebase goldens):
            ("fcvtzs d18, d5, #11", 0x5f75fcb2u32),
        ] {
            assert_eq!(encode1(text), want, "{text}");
        }
    }

    #[test]
    fn batch4_ld1r_post_index_imm_matches_the_measured_law() {
        // #imm iff imm == num_structs * (1 << size) -- the post-index
        // immediate is the TOTAL structure size in bytes (measured set;
        // `ld1r {v0.2d}, [x0], #16` stays rejected: total is 8, not 16).
        for (text, want) in [
            ("ld1r {v0.8b}, [x0], #1", 0x0ddfc000u32),
            ("ld1r {v0.16b}, [x0], #1", 0x4ddfc000u32),
            ("ld1r {v0.8h}, [x0], #2", 0x4ddfc400u32),
            ("ld1r {v0.4s}, [x0], #4", 0x4ddfc800u32),
            ("ld1r {v0.2d}, [x0], #8", 0x4ddfcc00u32),
            ("ld3r {v0.8b, v1.8b, v2.8b}, [x0], #3", 0x0ddfe000u32),
            ("ld4r {v0.8b, v1.8b, v2.8b, v3.8b}, [x0], #4", 0x0dffe000u32),
        ] {
            assert_eq!(encode1(text), want, "{text}");
        }
        for text in ["ld1r {v0.2d}, [x0], #16", "ld1r {v0.8b}, [x0], #8"] {
            encode1_err(text);
        }
    }

    #[test]
    fn batch4_widening_arrangement_law() {
        for (text, want) in [
            // arr_d == arr_n, elem_d == 2 * elem_m, count preserved (base).
            ("saddw v0.8h, v1.8h, v2.8b", 0x0e221020u32),
            ("saddw v0.4s, v1.4s, v2.4h", 0x0e621020u32),
            ("saddw v0.2d, v1.2d, v2.2s", 0x0ea21020u32),
            // "2" form: Vm carries double the destination element count.
            ("saddw2 v0.4s, v1.4s, v2.8h", 0x4e621020u32),
            ("saddw2 v0.2d, v1.2d, v2.4s", 0x4ea21020u32),
            ("usubw2 v0.4s, v1.4s, v2.8h", 0x6e623020u32),
        ] {
            assert_eq!(encode1(text), want, "{text}");
        }
        // Destination and Vn must spell the same arrangement (GAS REJ).
        encode1_err("saddw v0.4h, v1.8h, v2.8b");
    }

    #[test]
    fn batch4_scalar_saturating_narrow_family_completes() {
        for (text, want) in [
            // Scalar SQSHRUN/SQRSHRUN (0x7f class, ops 100001/100011) --
            // residual (c): these had no scalar route at all before.
            ("sqshrun b31, h23, #2", 0x7f0e86ffu32),
            ("sqrshrun h7, s3, #9", 0x7f178c67u32),
            // Rounding scalar narrow regressions (pre-existing route).
            ("sqrshrn b1, h2, #3", 0x5f0d9c41u32),
            ("uqrshrn b1, h2, #3", 0x7f0d9c41u32),
        ] {
            assert_eq!(encode1(text), want, "{text}");
        }
        for text in [
            // Non-saturating narrow shifts have no scalar form (GAS REJ).
            "shrn b31, h23, #2",
            "rshrn b31, h23, #2",
        ] {
            encode1_err(text);
        }
    }

    #[test]
    fn batch4_fcmp_fcmpe_both_forms() {
        for (text, want) in [
            // Register form: Rt = 0b00000 (fcmpe adds bit 4 -> 0b01000).
            ("fcmp s8, s9", 0x1e292100u32),
            ("fcmpe s8, s9", 0x1e292110u32),
            // #0.0 form: Rt = 0b01000 (fcmpe -> 0b11000).  Every
            // positive-zero spelling GAS accepts maps here:
            ("fcmp d30, #0.0", 0x1e6023c8u32),
            ("fcmp s8, #0.0", 0x1e202108u32),
            ("fcmp d0, #0", 0x1e602008u32),
            ("fcmp d0, #+0.0", 0x1e602008u32),
            ("fcmp d0, #0e0", 0x1e602008u32),
            ("fcmpe d8, #0.0", 0x1e602118u32),
        ] {
            assert_eq!(encode1(text), want, "{text}");
        }
        for text in [
            // Exactly two operands, +0.0 only (GAS-measured rejects).
            "fcmp d0",
            "fcmpe d0",
            "fcmp d0, #1",
            "fcmp d0, #1.0",
            "fcmp d0, #-0",
            "fcmp d0, #-0.0",
            "fcmp d0, #(-0)",
            "fcmp d0, #(0.0)",
        ] {
            encode1_err(text);
        }
    }

    #[test]
    fn batch4_fmov_immediate_grammar() {
        for (text, want) in [
            // D-form: word = 0x1e601000 | imm8 << 13 (imm8 values measured).
            ("fmov d0, #1.0", 0x1e6e1000u32),
            ("fmov d0, #2.0", 0x1e601000u32),
            ("fmov d0, #4.0", 0x1e621000u32),
            ("fmov d0, #8.0", 0x1e641000u32),
            ("fmov d0, #16.0", 0x1e661000u32),
            ("fmov d0, #0.5", 0x1e6c1000u32),
            ("fmov d0, #0.25", 0x1e6a1000u32),
            ("fmov d0, #0.125", 0x1e681000u32),
            ("fmov d0, #1.5", 0x1e6f1000u32),
            ("fmov d0, #1.25", 0x1e6e9000u32),
            ("fmov d0, #1.75", 0x1e6f9000u32),
            ("fmov d0, #1.875", 0x1e6fd000u32),
            ("fmov d0, #1.9375", 0x1e6ff000u32),
            ("fmov d0, #1.0625", 0x1e6e3000u32),
            ("fmov d0, #3.0", 0x1e611000u32),
            ("fmov d0, #5.0", 0x1e629000u32),
            ("fmov d0, #7.0", 0x1e639000u32),
            ("fmov d0, #10.0", 0x1e649000u32),
            ("fmov d0, #12.0", 0x1e651000u32),
            ("fmov d0, #15.5", 0x1e65f000u32),
            ("fmov d0, #24.0", 0x1e671000u32),
            ("fmov d0, #0.4375", 0x1e6b9000u32),
            ("fmov d0, #-1.0", 0x1e7e1000u32),
            ("fmov d0, #-0.5", 0x1e7c1000u32),
            ("fmov d0, #-2.0", 0x1e701000u32),
            // Integer and scientific spellings (all measured ACCEPT):
            ("fmov d0, #1", 0x1e6e1000u32),
            ("fmov d0, #1e1", 0x1e649000u32),
            ("fmov d0, #1.0e0", 0x1e6e1000u32),
            // S-form shares imm8 (measured): f32 rounds then matches.
            ("fmov s0, #1.0", 0x1e2e1000u32),
            ("fmov s0, #1.00000001", 0x1e2e1000u32),
            // H-form (FEAT_FP16, armv9.4): measured words.
            ("fmov h0, #1.0", 0x1eee1000u32),
            ("fmov h0, #0.5", 0x1eec1000u32),
            ("fmov h0, #1e1", 0x1ee49000u32),
        ] {
            assert_eq!(encode1(text), want, "{text}");
        }
        for text in [
            // No imm8 decodes to these (all GAS-measured rejects):
            "fmov d0, #0.0",
            "fmov d0, #-0.0",
            "fmov d0, #0.1",
            "fmov d0, #1e-1",
            "fmov d0, #0.0625",
            "fmov d0, #0.09375",
            "fmov d0, #0.03125",
            "fmov d0, #256.0",
            "fmov d0, #65504.0",
            "fmov d0, #0.0009765625",
            "fmov d0, #(-1.0)",
            "fmov s0, #1.0000001",
            "fmov s0, #0.1",
            "fmov h0, #0.1",
        ] {
            encode1_err(text);
        }
    }

    /// The dispatcher ratchet: `tests/aarch64/dispatcher-goldens.tsv`
    /// carries exactly one GAS-verified instruction for EVERY
    /// `encode_instruction` arm (both directions checked -- a new arm
    /// without a row, a row for a deleted arm, or a drifted word all
    /// fail), and every row is re-encoded through the full
    /// parse -> dispatcher -> leaf path here, on a host with no cross
    /// tools.  Regenerate with `scripts/gen_aarch64_dispatcher_goldens.py`
    /// (exit 0 requires full coverage) and verify with `--check`.
    #[test]
    fn dispatcher_goldens_ratchet_one_row_per_arm() {
        // Arm set: same mechanical rule as the generator -- mnemonic keys
        // at line start inside the `encode_instruction` body.
        let src = include_str!("mod.rs");
        let a = src
            .find("pub fn encode_instruction(")
            .expect("encode_instruction found");
        let body_end = src[a + 10..].find("\npub fn ").map(|i| a + 10 + i);
        let body = &src[a..body_end.unwrap_or(src.len())];
        let mut arms: Vec<String> = Vec::new();
        for line in body.lines() {
            let t = line.trim_start();
            if let Some(rest) = t.strip_prefix('"') {
                if let Some(end) = rest.find('"') {
                    let mn = &rest[..end];
                    let after = &rest[end + 1..];
                    let key_ok = !mn.is_empty()
                        && mn.chars().all(|c| {
                            c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '.'
                        });
                    if key_ok
                        && (after.trim_start().starts_with("=>")
                            || after.trim_start().starts_with('|'))
                    {
                        if !arms.iter().any(|x| x == mn) {
                            arms.push(mn.to_string());
                        }
                    }
                }
            }
        }
        arms.sort();

        let tsv_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/aarch64/dispatcher-goldens.tsv");
        let tsv = std::fs::read_to_string(&tsv_path)
            .unwrap_or_else(|e| panic!("read {}: {e}", tsv_path.display()));
        let mut rows: Vec<(String, String, String)> = Vec::new();
        for line in tsv.lines() {
            if line.starts_with('#') || line.trim().is_empty() {
                continue;
            }
            let mut it = line.split('\t');
            let (mn, text, word) = (it.next(), it.next(), it.next());
            match (mn, text, word) {
                (Some(m), Some(t), Some(w)) if it.next().is_none() => {
                    rows.push((m.to_string(), t.to_string(), w.to_string()));
                }
                _ => panic!("malformed TSV row: {line:?}"),
            }
        }

        // Exact arm <-> row correspondence (>= 1 row per arm and no
        // stale rows), because the generator only writes the file at
        // full coverage and `--check` re-validates it.
        let mut row_mns: Vec<String> = rows.iter().map(|r| r.0.clone()).collect();
        row_mns.sort();
        row_mns.dedup();
        assert_eq!(
            arms, row_mns,
            "TSV arm set drifted from encode_instruction arms; \
             regenerate with scripts/gen_aarch64_dispatcher_goldens.py"
        );
        assert_eq!(
            rows.len(),
            arms.len(),
            "expected exactly one row per arm, got {} rows for {} arms",
            rows.len(),
            arms.len()
        );

        // Every row re-assembles to its recorded GAS word.
        for (mn, text, word) in &rows {
            // The TSV records the instruction's in-memory (little-endian)
            // bytes as hex -- the same `gdata.hex()` the generator compares
            // against lccc's output -- so unpack them LE.
            let raw = (0..word.len())
                .step_by(2)
                .map(|i| {
                    u8::from_str_radix(&word[i..i + 2], 16)
                        .unwrap_or_else(|e| panic!("bad hex {word:?} for {mn}: {e}"))
                })
                .collect::<Vec<u8>>();
            assert_eq!(raw.len(), 4, "{mn}: word {word:?} is not 4 bytes");
            let want = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]);
            let got = encode1_base(text);
            assert_eq!(got, want, "{mn}: `{text}` word drifted");
        }
    }

    /// Full GAS 2.47-measured law for the FP zero-comparisons and the
    /// vector rounding conversions (every (op, arrangement) cell probed
    /// under the pinned matrix law, including the 16-bit arrangements'
    /// bits 20:18 = 0b110 marker) plus the immediate grammar (`#-0`
    /// rejected; `#0`, `#0.0`, `#0e0`, `#+0.0` accepted).
    #[test]
    fn batch4_fp_zero_cmp_and_vector_rounding_law() {
        for (text, want) in [
            ("fcvtas v0.2s, v1.2s", 0x0e21c820u32),
            ("fcvtau v0.2s, v1.2s", 0x2e21c820u32),
            ("fcvtns v0.2s, v1.2s", 0x0e21a820u32),
            ("fcvtnu v0.2s, v1.2s", 0x2e21a820u32),
            ("fcvtms v0.2s, v1.2s", 0x0e21b820u32),
            ("fcvtmu v0.2s, v1.2s", 0x2e21b820u32),
            ("fcvtps v0.2s, v1.2s", 0x0ea1a820u32),
            ("fcvtpu v0.2s, v1.2s", 0x2ea1a820u32),
            ("fcvtzs v0.2s, v1.2s", 0x0ea1b820u32),
            ("fcvtzu v0.2s, v1.2s", 0x2ea1b820u32),
            ("scvtf v0.2s, v1.2s", 0x0e21d820u32),
            ("ucvtf v0.2s, v1.2s", 0x2e21d820u32),
            ("fcvtas v0.4s, v1.4s", 0x4e21c820u32),
            ("fcvtau v0.4s, v1.4s", 0x6e21c820u32),
            ("fcvtns v0.4s, v1.4s", 0x4e21a820u32),
            ("fcvtnu v0.4s, v1.4s", 0x6e21a820u32),
            ("fcvtms v0.4s, v1.4s", 0x4e21b820u32),
            ("fcvtmu v0.4s, v1.4s", 0x6e21b820u32),
            ("fcvtps v0.4s, v1.4s", 0x4ea1a820u32),
            ("fcvtpu v0.4s, v1.4s", 0x6ea1a820u32),
            ("fcvtzs v0.4s, v1.4s", 0x4ea1b820u32),
            ("fcvtzu v0.4s, v1.4s", 0x6ea1b820u32),
            ("scvtf v0.4s, v1.4s", 0x4e21d820u32),
            ("ucvtf v0.4s, v1.4s", 0x6e21d820u32),
            ("fcvtas v0.2d, v1.2d", 0x4e61c820u32),
            ("fcvtau v0.2d, v1.2d", 0x6e61c820u32),
            ("fcvtns v0.2d, v1.2d", 0x4e61a820u32),
            ("fcvtnu v0.2d, v1.2d", 0x6e61a820u32),
            ("fcvtms v0.2d, v1.2d", 0x4e61b820u32),
            ("fcvtmu v0.2d, v1.2d", 0x6e61b820u32),
            ("fcvtps v0.2d, v1.2d", 0x4ee1a820u32),
            ("fcvtpu v0.2d, v1.2d", 0x6ee1a820u32),
            ("fcvtzs v0.2d, v1.2d", 0x4ee1b820u32),
            ("fcvtzu v0.2d, v1.2d", 0x6ee1b820u32),
            ("scvtf v0.2d, v1.2d", 0x4e61d820u32),
            ("ucvtf v0.2d, v1.2d", 0x6e61d820u32),
            ("fcvtas v0.4h, v1.4h", 0x0e79c820u32),
            ("fcvtau v0.4h, v1.4h", 0x2e79c820u32),
            ("fcvtns v0.4h, v1.4h", 0x0e79a820u32),
            ("fcvtnu v0.4h, v1.4h", 0x2e79a820u32),
            ("fcvtms v0.4h, v1.4h", 0x0e79b820u32),
            ("fcvtmu v0.4h, v1.4h", 0x2e79b820u32),
            ("fcvtps v0.4h, v1.4h", 0x0ef9a820u32),
            ("fcvtpu v0.4h, v1.4h", 0x2ef9a820u32),
            ("fcvtzs v0.4h, v1.4h", 0x0ef9b820u32),
            ("fcvtzu v0.4h, v1.4h", 0x2ef9b820u32),
            ("scvtf v0.4h, v1.4h", 0x0e79d820u32),
            ("ucvtf v0.4h, v1.4h", 0x2e79d820u32),
            ("fcvtas v0.8h, v1.8h", 0x4e79c820u32),
            ("fcvtau v0.8h, v1.8h", 0x6e79c820u32),
            ("fcvtns v0.8h, v1.8h", 0x4e79a820u32),
            ("fcvtnu v0.8h, v1.8h", 0x6e79a820u32),
            ("fcvtms v0.8h, v1.8h", 0x4e79b820u32),
            ("fcvtmu v0.8h, v1.8h", 0x6e79b820u32),
            ("fcvtps v0.8h, v1.8h", 0x4ef9a820u32),
            ("fcvtpu v0.8h, v1.8h", 0x6ef9a820u32),
            ("fcvtzs v0.8h, v1.8h", 0x4ef9b820u32),
            ("fcvtzu v0.8h, v1.8h", 0x6ef9b820u32),
            ("scvtf v0.8h, v1.8h", 0x4e79d820u32),
            ("ucvtf v0.8h, v1.8h", 0x6e79d820u32),
            ("fcmeq v0.2s, v1.2s, #0", 0x0ea0d820u32),
            ("fcmge v0.2s, v1.2s, #0", 0x2ea0c820u32),
            ("fcmgt v0.2s, v1.2s, #0", 0x0ea0c820u32),
            ("fcmle v0.2s, v1.2s, #0", 0x2ea0d820u32),
            ("fcmlt v0.2s, v1.2s, #0", 0x0ea0e820u32),
            ("fcmeq v0.4s, v1.4s, #0", 0x4ea0d820u32),
            ("fcmge v0.4s, v1.4s, #0", 0x6ea0c820u32),
            ("fcmgt v0.4s, v1.4s, #0", 0x4ea0c820u32),
            ("fcmle v0.4s, v1.4s, #0", 0x6ea0d820u32),
            ("fcmlt v0.4s, v1.4s, #0", 0x4ea0e820u32),
            ("fcmeq v0.2d, v1.2d, #0", 0x4ee0d820u32),
            ("fcmge v0.2d, v1.2d, #0", 0x6ee0c820u32),
            ("fcmgt v0.2d, v1.2d, #0", 0x4ee0c820u32),
            ("fcmle v0.2d, v1.2d, #0", 0x6ee0d820u32),
            ("fcmlt v0.2d, v1.2d, #0", 0x4ee0e820u32),
            ("fcmeq v0.4h, v1.4h, #0", 0x0ef8d820u32),
            ("fcmge v0.4h, v1.4h, #0", 0x2ef8c820u32),
            ("fcmgt v0.4h, v1.4h, #0", 0x0ef8c820u32),
            ("fcmle v0.4h, v1.4h, #0", 0x2ef8d820u32),
            ("fcmlt v0.4h, v1.4h, #0", 0x0ef8e820u32),
            ("fcmeq v0.8h, v1.8h, #0", 0x4ef8d820u32),
            ("fcmge v0.8h, v1.8h, #0", 0x6ef8c820u32),
            ("fcmgt v0.8h, v1.8h, #0", 0x4ef8c820u32),
            ("fcmle v0.8h, v1.8h, #0", 0x6ef8d820u32),
            ("fcmlt v0.8h, v1.8h, #0", 0x4ef8e820u32),
            ("fcmle v0.4s, v1.4s, #0e0", 0x6ea0d820u32),
            ("fcmle v0.4s, v1.4s, #+0.0", 0x6ea0d820u32),
        ] {
            assert_eq!(encode1(text), want, "{text}");
        }
        for text in [
            "fcmle v0.4s, v1.4s, #-0",
            // The only immediate is +0.0 and only in the 3-operand vector
            // form: `#1` is not zero and the 2-operand scalar form does
            // not exist (all GAS-measured).
            "fcmle v0.4s, v1.4s, #1",
            "fcmle s0, s1",
            "fcmlt v0.4s, v1.4s, #1",
            "fcmle v0.4s, v1.4s, #(0.0)",
        ] {
            encode1_err(text);
        }
    }
}
