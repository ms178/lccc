//! Core encoding helpers for i686 instruction encoding.
//!
//! ModR/M, SIB, segment prefixes, memory-operand encoding, and relocation
//! helpers for the 32-bit x86 backend. Supports legacy 16-bit and 32-bit
//! effective-address encodings; this is not a long-mode or EVEX encoder.
//!
//! # Hard invariants
//!
//! - **Validate before mutate.** Memory operands are fully validated and
//!   planned before any byte is appended to `self.bytes` or any relocation
//!   is pushed. A failed encoding leaves the encoder exactly as it was.
//! - **Never substitute a different register for an invalid one.** The old
//!   code routed base/index register names through the general `reg_num()`
//!   table, which silently reassigned `(%bx)` -> `(%ebx)`, `(%xmm0)` ->
//!   `(%eax)`, `(%ah)` -> `(%esp)`, and dropped `%esp` indices. Invalid
//!   names now produce a diagnostic, exactly like GNU as.
//! - **Never encode ESP as a SIB index.** SIB index field 4 is the "no
//!   index" sentinel; encoding `%esp` there silently drops the register.
//! - **Preserve base/index roles and their architectural default segment.**
//!   The one legitimate role change is the scale-1 index fold (below),
//!   which is restricted to registers whose default segment is DS in both
//!   roles.
//! - **Use full address-width displacement fields for symbolic operands.**
//!   GNU as emits `mod=10` with disp16/disp32 for relocatable displacements;
//!   the relocation points at the displacement field, never at ModR/M or
//!   SIB.
//! - **Numeric displacements follow GNU as's modular rule** (verified
//!   against binutils 2.44, `as --32`): a literal in
//!   `[iN::MIN, uN::MAX]` is reinterpreted as its signed N-bit value and
//!   encoded in the shortest form (so `0xffff(%eax)` is `disp8 -1`, three
//!   bytes shorter than the old code); a literal outside that union is
//!   taken modulo 2^N and forced to the full-width field. Nothing is
//!   silently truncated to a *different* address.
//!
//! # External contracts intentionally preserved
//!
//! - `pending_addr32` requests the 0x67 address-size prefix fixup in
//!   `InstructionEncoder::encode`, which routes it to
//!   `fixup_code16_prefixes` in `.code16` and to
//!   `fixup_code32_addr16_prefix` in `.code32` (see below).
//! - Symbolic displacement fields initially contain zero bytes;
//!   `Relocation::addend` carries the addend until the ELF writer patches
//!   it into the field (REL format, `ElfWriterCore`), with a patch width
//!   matching the relocation (`R_386_16` => 2 bytes).
//! - `@PLT` stripping and `PC32 -> PLT32` promotion in `add_relocation`
//!   are preserved; symbol-difference relocations keep the original symbol
//!   spelling for the existing resolver.
//! - TLS/GOT modifier *mapping* lives in
//!   `InstructionEncoder::tls_reloc_type` (x87.rs); this file only
//!   rejects modifier names the mapper does not know (GNU as rejects
//!   unknown modifiers, the mapper silently degraded them to `R_386_32`).
//! - `emit_segment_prefix` keeps its diagnostic-less `()` signature (every
//!   caller in this backend uses it as a statement). The parser only
//!   accepts the six segment names, so an unknown segment here is a
//!   programming error and panics instead of silently dropping an
//!   override.
//!
//! # Known, documented divergences from GNU as
//!
//! - Numeric literals with leading zeroes are decimal (`010` == 10), the
//!   historical contract of this helper. GNU as reads octal; no shipped
//!   source depends on either reading, and changing it now would be a
//!   gratuitous compatibility break.
//! - `symbol+offset` absolute labels pass through `split_label_offset`
//!   unchanged (symbol plus addend), matching the pre-existing pipeline.
//!
//! # Divergences closed by this rewrite
//!
//! - 16-bit register addressing (`%bx`/`%bp`/`%si`/`%di`) in `.code32` used
//!   to be **rejected** with a diagnostic, because this file cannot place
//!   the 0x67 address-size override itself: callers push the opcode before
//!   `encode_modrm_mem` runs. It is now supported, exactly as GNU as does
//!   it. `encode_modrm_mem` reports a 16-bit addressing form through
//!   `pending_addr32`, and `InstructionEncoder::fixup_code32_addr16_prefix`
//!   (`encoder/mod.rs`) splices 0x67 into the instruction's prefix run
//!   after FWAIT and any group-1/segment override, shifting the offsets of
//!   the relocations it displaces. The behavior this replaced was a silent
//!   miscompile (`(%bx)` encoded as `(%ebx)`).

use super::*;

// ===========================================================================
// Address value
// ===========================================================================

/// A numeric effective-address displacement or a relocatable expression.
///
/// Relocatable expressions deliberately carry no numeric payload here:
/// the field bytes stay zero and the addend lives in the relocation
/// record until the object writer materializes it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum I686AddressValue {
    Constant(i64),
    Relocatable,
}

impl I686AddressValue {
    fn is_relocatable(self) -> bool {
        matches!(self, Self::Relocatable)
    }

    /// Normalize a numeric displacement to the address width using GNU
    /// as's modular rule.
    ///
    /// Returns `(low_bits, in_union)`: `low_bits` is the value modulo
    /// 2^N, and `in_union` says whether the original literal belonged to
    /// the signed/unsigned N-bit union and may therefore use shortened
    /// (disp8/disp0) encodings. GAS-verified examples for N=16:
    /// `0xffff` -> (-1, true) => disp8; `0x10000` -> (0, false) =>
    /// disp16 forced. For N=32: `0xffffffff` -> (-1, true) => disp8;
    /// `4294967296` -> (0, false) => disp32 forced.
    fn normalize(self, address16: bool) -> (u32, bool) {
        let value = match self {
            Self::Constant(value) => value,
            Self::Relocatable => return (0, false),
        };

        if address16 {
            let in_union = (i16::MIN as i64..=u16::MAX as i64).contains(&value);
            (value as u16 as u32, in_union)
        } else {
            let in_union = (i32::MIN as i64..=u32::MAX as i64).contains(&value);
            (value as u32, in_union)
        }
    }
}

// ===========================================================================
// Address plan
// ===========================================================================

/// Complete ModR/M + optional SIB + displacement encoding.
///
/// The largest legacy address encoding is six bytes: one ModR/M byte, one
/// SIB byte, and four displacement bytes.
#[derive(Debug)]
struct I686AddressPlan {
    bytes: [u8; 6],
    /// Offset of the displacement field within `bytes` (1 or 2).
    header_len: u8,
    /// Total encoded length.
    len: u8,
    /// True when the 16-bit effective-address table was used.
    address16: bool,
}

impl I686AddressPlan {
    /// The encoded bytes, with or without the ModR/M (+SIB) header.
    fn encoded(&self, include_modrm: bool) -> &[u8] {
        let start = if include_modrm {
            0
        } else {
            usize::from(self.header_len)
        };
        &self.bytes[start..usize::from(self.len)]
    }

    /// Offset of the displacement field in the emitted slice.
    fn displacement_offset(&self, include_modrm: bool) -> usize {
        if include_modrm {
            usize::from(self.header_len)
        } else {
            0
        }
    }
}

// ===========================================================================
// Pending relocation
// ===========================================================================

/// Relocation information pending until the address field is emitted.
#[derive(Clone, Copy)]
struct I686PendingRelocation<'a> {
    symbol: &'a str,
    reloc_type: u32,
    addend: i64,
    diff_symbol: Option<&'a str>,
    modifier: Option<&'a str>,
}

fn i686_validate_reg_field(reg: u8) -> Result<(), String> {
    if reg < 8 {
        Ok(())
    } else {
        Err(format!("invalid i686 ModR/M register field: {}", reg))
    }
}

/// Strict 32-bit address-register lookup.
///
/// A general register-number lookup is insufficient here: `AL`, `AX`,
/// `XMM0` and registers of other modes share numerical register IDs with
/// `EAX`, and routing them through `reg_num` silently reassigns the
/// address to the wrong register (the old behavior).
fn i686_address_register32(name: &str) -> Result<u8, String> {
    match name {
        "eax" => Ok(0),
        "ecx" => Ok(1),
        "edx" => Ok(2),
        "ebx" => Ok(3),
        "esp" => Ok(4),
        "ebp" => Ok(5),
        "esi" => Ok(6),
        "edi" => Ok(7),
        _ => Err(format!("invalid i686 32-bit address register: {}", name)),
    }
}

fn i686_scale_bits(scale: u8) -> Result<u8, String> {
    match scale {
        1 => Ok(0),
        2 => Ok(1),
        4 => Ok(2),
        8 => Ok(3),
        _ => Err(format!(
            "invalid i686 address scale {}; expected 1, 2, 4, or 8",
            scale
        )),
    }
}

/// Segment-override prefix byte, exact lowercase spelling as parsed.
pub(super) fn i686_segment_prefix(segment: &str) -> Option<u8> {
    match segment {
        "es" => Some(0x26),
        "cs" => Some(0x2E),
        "ss" => Some(0x36),
        "ds" => Some(0x3E),
        "fs" => Some(0x64),
        "gs" => Some(0x65),
        _ => None,
    }
}

/// Default segment prefix for an addressing form (GAS-verified).
///
/// 32-bit: SS when the base is `%esp`/`%ebp`; DS otherwise (absolute,
/// base-less, index-only). 16-bit: SS when `%bp` participates; DS
/// otherwise, including the absolute `mod=00 rm=110` form.
///
/// GNU as drops an explicit override that equals the default segment
/// (`mov %ds:8(%eax),%ebx` -> `8b 58 08`, `mov %ss:8(%ebp),%eax` ->
/// `8b 45 08`) and emits it otherwise (`mov %ds:8(%ebp),%eax` ->
/// `3e 8b 45 08`). `emit_segment_prefix` mirrors that.
/// Default segment prefix for an addressing form (0x3E = DS, 0x36 = SS).
///
/// The rule is independent of the code mode: the register names select the
/// addressing table. 16-bit forms (`%bp` base or index, rm 2/3/6) default to
/// SS; 32-bit forms default to SS exactly for `%esp`/`%ebp` bases. Index-only
/// and absolute forms default to DS. Verified against GNU as 2.44 for both
/// `.code32` and `.code16` (including the 0x67 fallback inside `.code16`).
pub(super) fn i686_default_segment(base: Option<&str>, index: Option<&str>) -> u8 {
    let default_ss = match base {
        Some("ebp" | "esp" | "bp") => true,
        Some(_) => false,
        None => index == Some("bp"),
    };

    if default_ss { 0x36 } else { 0x3E }
}

/// Modifier names understood by `InstructionEncoder::tls_reloc_type`.
///
/// Keep in sync with the match in `x87.rs`. The mapper silently degrades
/// unknown modifiers to `R_386_32`; GNU as rejects them, so this file
/// checks membership before consulting the mapper.
fn i686_known_tls_modifier(modifier: &str) -> bool {
    matches!(
        modifier,
        "NTPOFF"
            | "TPOFF"
            | "TLSGD"
            | "TLSLDM"
            | "DTPOFF"
            | "GOT"
            | "GOTOFF"
            | "PLT"
            | "GOTPC"
            | "GOTNTPOFF"
            | "INDNTPOFF"
    )
}

// ===========================================================================
// Absolute numeric literals
// ===========================================================================

/// Recognize standalone absolute integer literals.
///
/// Decimal interpretation, including leading zeroes, preserves the
/// original helper's `parse::<i64>()` contract (see module docs).
/// Explicit hexadecimal (`0x`, `0X`) and binary (`0b`, `0B`) literals
/// with an optional sign are also accepted; the old code treated them as
/// symbol names and fabricated relocations against `"0x10"`.
///
/// GAS numeric local-label references remain labels: `1f`/`1b`/`0b` are
/// not parsed as numbers. Expressions containing `+`/`-` continue through
/// the existing `split_label_offset` path; this is not an expression
/// evaluator.
fn i686_parse_address_literal(label: &str) -> Result<Option<i64>, String> {
    let text = label.trim();
    if text.is_empty() {
        return Err("empty absolute address".to_string());
    }

    let (negative, body) = if let Some(rest) = text.strip_prefix('-') {
        (true, rest)
    } else if let Some(rest) = text.strip_prefix('+') {
        (false, rest)
    } else {
        (false, text)
    };

    if body.is_empty() {
        return Err(format!("invalid absolute address: {}", label));
    }

    // Numeric local-label reference: digits + `f`/`b` suffix.
    if let Some(digits) = body.strip_suffix('f').or_else(|| body.strip_suffix('b')) {
        if !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return Ok(None);
        }
    }

    // Preserve the external expression parser's responsibility.
    if body.contains('+') || body.contains('-') {
        return Ok(None);
    }

    let (radix, digits) =
        if let Some(rest) = body.strip_prefix("0x").or_else(|| body.strip_prefix("0X")) {
            (16, rest)
        } else if let Some(rest) = body.strip_prefix("0b").or_else(|| body.strip_prefix("0B")) {
            (2, rest)
        } else if body.bytes().all(|byte| byte.is_ascii_digit()) {
            (10, body)
        } else {
            return Ok(None);
        };

    let magnitude = u64::from_str_radix(digits, radix)
        .map_err(|_| format!("invalid or overflowing address literal: {}", label))?;

    // Every u64 fits i128; negation is safe here, including the
    // magnitude corresponding to i64::MIN.
    let signed = if negative {
        -(magnitude as i128)
    } else {
        magnitude as i128
    };

    if signed < i64::MIN as i128 || signed > i64::MAX as i128 {
        return Err(format!("overflowing address literal: {}", label));
    }

    Ok(Some(signed as i64))
}

// ===========================================================================
// Address planner
// ===========================================================================

fn i686_make_address_plan(
    modrm: u8,
    sib: Option<u8>,
    displacement: u32,
    displacement_len: u8,
    address16: bool,
) -> I686AddressPlan {
    debug_assert!(matches!(displacement_len, 0 | 1 | 2 | 4));

    let mut bytes = [0u8; 6];
    bytes[0] = modrm;

    let header_len = if let Some(sib) = sib {
        bytes[1] = sib;
        2u8
    } else {
        1u8
    };

    let len = header_len + displacement_len;
    bytes[usize::from(header_len)..usize::from(len)]
        .copy_from_slice(&displacement.to_le_bytes()[..usize::from(displacement_len)]);

    I686AddressPlan {
        bytes,
        header_len,
        len,
        address16,
    }
}

/// Fold a base-less, scale-1 index operand into a plain base operand.
///
/// `-1(,%ecx,1)` and `-1(%ecx)` compute the same effective address, but
/// the first needs a SIB byte and — because SIB with no base supports
/// only `mod=00` + disp32 — a full 4-byte displacement. Folding turns
/// GAS's `8b 04 0d ff ff ff ff` (8 bytes) into `8b 41 ff` (3 bytes).
/// ICC performs this fold; GAS 2.44, Clang and ICX all emit the long
/// form, so this is where the backend beats every oracle on this family.
///
/// The fold is deliberately restricted to registers whose default
/// segment is DS in both roles: `%ebp` as an index selects DS, but as a
/// base it selects SS — in 32-bit protected mode those segments can
/// have different bases, so folding `%ebp` would change the reference.
/// `%esp` can never be an index; it stays on the rejection path.
fn fold_index_into_base(mem: &MemoryOperand) -> Option<MemoryOperand> {
    if mem.base.is_some() || mem.scale.unwrap_or(1) != 1 {
        return None;
    }

    match mem.index.as_ref()?.name.as_str() {
        "eax" | "ecx" | "edx" | "ebx" | "esi" | "edi" => {}
        _ => return None,
    }

    Some(MemoryOperand {
        base: mem.index.clone(),
        index: None,
        scale: None,
        ..mem.clone()
    })
}

/// Validate and plan an effective address without mutating the encoder.
///
/// All validation happens here, before any byte is emitted, so a failed
/// operand leaves the instruction stream untouched. Base/index roles are
/// never rewritten (except by the segment-safe fold above); register
/// names are never resolved through the general `reg_num` table.
fn i686_plan_memory_address(
    code16: bool,
    reg_field: u8,
    base: Option<&str>,
    index: Option<&str>,
    scale: Option<u8>,
    displacement: I686AddressValue,
) -> Result<I686AddressPlan, String> {
    i686_validate_reg_field(reg_field)?;

    let scale = scale.unwrap_or(1);
    let scale_bits = i686_scale_bits(scale)?;

    if index.is_none() && scale != 1 {
        return Err("address scale requires an index register".to_string());
    }

    let base_is16 = base.is_some_and(|name| matches!(name, "bx" | "bp" | "si" | "di"));
    let index_is16 = index.is_some_and(|name| matches!(name, "bx" | "bp" | "si" | "di"));

    // Every name must be a real 16-bit or 32-bit address register.
    for name in [base, index].into_iter().flatten() {
        if !base_is16 && !index_is16 && i686_address_register32(name).is_err() {
            // `name` is neither a 16-bit address register nor a 32-bit
            // one: reject instead of silently reassigning it through
            // reg_num (which maps `ax`->`eax`, `ah`->`esp`, `xmm0`->`eax`).
            return Err(format!("invalid i686 address register: {}", name));
        }
    }

    let has_register16 = base_is16 || index_is16;

    // The 16-bit table serves 16-bit register addressing in BOTH code
    // modes (GNU as adds a 0x67 address-size override in .code32) and
    // absolute addresses in .code16 (mod=00 rm=110 + disp16). 32-bit
    // register names fall through to the 32-bit table below and request
    // a 0x67 override in .code16 instead.
    let address16 = has_register16 || (code16 && base.is_none() && index.is_none());

    if address16 {
        if scale != 1 {
            return Err("16-bit addressing does not support scaled indices".to_string());
        }

        let mixed = (base_is16 && index.is_some() && !index_is16)
            || (index_is16 && base.is_some() && !base_is16);
        if mixed {
            return Err(format!(
                "mixed-width 16-bit address: base={:?}, index={:?}",
                base, index
            ));
        }

        // 16-bit ModR/M table (GAS-verified rm assignments). GAS also
        // rejects reversed and other pairings; we mirror that.
        let rm = match (base, index) {
            (None, None) => 6,

            (Some("bx"), Some("si")) => 0,
            (Some("bx"), Some("di")) => 1,
            (Some("bp"), Some("si")) => 2,
            (Some("bp"), Some("di")) => 3,

            (Some("si"), None) | (None, Some("si")) => 4,
            (Some("di"), None) | (None, Some("di")) => 5,
            (Some("bp"), None) | (None, Some("bp")) => 6,
            (Some("bx"), None) | (None, Some("bx")) => 7,

            _ => {
                return Err(format!(
                    "invalid 16-bit address: base={:?}, index={:?}",
                    base, index
                ));
            }
        };

        let (mode, displacement_len) = if base.is_none() && index.is_none() {
            // Absolute: mod=00 rm=110 + disp16.
            (0, 2)
        } else if displacement.is_relocatable() {
            // Full-width field for the relocation.
            (2, 2)
        } else {
            let (bits, in_union) = displacement.normalize(true);
            if !in_union {
                (2, 2)
            } else {
                let signed = bits as u16 as i16;
                if signed == 0 && rm != 6 {
                    (0, 0)
                } else if (-128..=127).contains(&signed) {
                    (1, 1)
                } else {
                    (2, 2)
                }
            }
        };

        return Ok(i686_make_address_plan(
            (mode << 6) | (reg_field << 3) | rm,
            None,
            displacement.normalize(true).0,
            displacement_len,
            true,
        ));
    }

    let base_num = base.map(i686_address_register32).transpose()?;
    let index_num = index.map(i686_address_register32).transpose()?;

    if index_num == Some(4) {
        return Err("ESP cannot be a SIB index: index field 4 means no index".to_string());
    }

    let symbolic = displacement.is_relocatable();

    let (mode, rm, sib, displacement_len) = match (base_num, index_num) {
        (None, None) => (0, 5, None, 4),

        (None, Some(index_num)) => {
            // mod=00 with SIB.base=101 always requires disp32.
            let sib = (scale_bits << 6) | (index_num << 3) | 5;
            (0, 4, Some(sib), 4)
        }

        (Some(base_num), index_num) => {
            let (bits, in_union) = displacement.normalize(false);
            let (mode, displacement_len) = if symbolic || !in_union {
                (2, 4)
            } else {
                let signed = bits as i32;
                if signed == 0 && base_num != 5 {
                    (0, 0)
                } else if (-128..=127).contains(&signed) {
                    // [EBP] needs at least disp8=0.
                    (1, 1)
                } else {
                    (2, 4)
                }
            };

            let sib = if index_num.is_some() || base_num == 4 {
                Some((scale_bits << 6) | (index_num.unwrap_or(4) << 3) | base_num)
            } else {
                None
            };

            (
                mode,
                if sib.is_some() { 4 } else { base_num },
                sib,
                displacement_len,
            )
        }
    };

    Ok(i686_make_address_plan(
        (mode << 6) | (reg_field << 3) | rm,
        sib,
        displacement.normalize(false).0,
        displacement_len,
        false,
    ))
}

// ===========================================================================
// Relocations
// ===========================================================================

fn i686_memory_displacement(
    displacement: &Displacement,
) -> (I686AddressValue, Option<I686PendingRelocation<'_>>) {
    let (symbol, addend, diff_symbol, modifier) = match displacement {
        Displacement::None => {
            return (I686AddressValue::Constant(0), None);
        }
        Displacement::Integer(value) => {
            return (I686AddressValue::Constant(*value), None);
        }

        Displacement::Symbol(symbol) => (symbol.as_str(), 0, None, None),

        Displacement::SymbolAddend(symbol, addend)
        | Displacement::SymbolPlusOffset(symbol, addend) => (symbol.as_str(), *addend, None, None),

        Displacement::SymbolDiff(symbol, diff) => (symbol.as_str(), 0, Some(diff.as_str()), None),

        Displacement::SymbolDiffAddend(symbol, diff, addend) => {
            (symbol.as_str(), *addend, Some(diff.as_str()), None)
        }

        Displacement::SymbolMod(symbol, modifier) => {
            (symbol.as_str(), 0, None, Some(modifier.as_str()))
        }
    };

    (
        I686AddressValue::Relocatable,
        Some(I686PendingRelocation {
            symbol,
            reloc_type: R_386_32,
            addend,
            diff_symbol,
            modifier,
        }),
    )
}

/// Validate the relocation-width contract before emitting any bytes.
///
/// The addend itself need not fit the final relocation field: it is
/// patched by the object writer with the field's width, and whether the
/// evaluated expression fits belongs to the resolver/linker.
fn i686_validate_pending_relocation(
    relocation: &mut I686PendingRelocation<'_>,
    address16: bool,
) -> Result<(), String> {
    if relocation.symbol.is_empty()
        || matches!(relocation.diff_symbol, Some(""))
        || (relocation.diff_symbol.is_none() && relocation.symbol == "@PLT")
    {
        return Err("empty symbol in memory relocation".to_string());
    }

    if address16 {
        if relocation.diff_symbol.is_some() {
            return Err("symbol-difference displacement requires a verified \
                 16-bit difference-relocation resolver"
                .to_string());
        }

        if relocation.modifier.is_some() {
            return Err("relocation modifiers are unsupported in 16-bit \
                 address displacement fields"
                .to_string());
        }

        relocation.reloc_type = R_386_16;
    }

    Ok(())
}

/// Construct a relocation using the original normalization rules.
///
/// `@PLT` is stripped from the symbol name and promotes `R_386_PC32` to
/// `R_386_PLT32`. Difference relocations keep their original symbol
/// spelling; their resolver has a separate existing contract.
fn i686_make_relocation(
    offset: usize,
    symbol: &str,
    reloc_type: u32,
    addend: i64,
    diff_symbol: Option<&str>,
) -> Relocation {
    let (symbol, reloc_type) = if diff_symbol.is_none() {
        if let Some(base) = symbol.strip_suffix("@PLT") {
            (
                base,
                if reloc_type == R_386_PC32 {
                    R_386_PLT32
                } else {
                    reloc_type
                },
            )
        } else {
            (symbol, reloc_type)
        }
    } else {
        (symbol, reloc_type)
    };

    assert!(!symbol.is_empty(), "empty i686 relocation symbol");
    assert!(
        !matches!(diff_symbol, Some("")),
        "empty i686 relocation difference symbol"
    );

    Relocation {
        offset: offset as u64,
        symbol: symbol.to_owned(),
        reloc_type,
        addend,
        diff_symbol: diff_symbol.map(str::to_owned),
    }
}

/// Append a complete, validated address and its optional relocation.
///
/// This is also the emission path exercised by the unit tests; tests do
/// not substitute an independent serializer for production emission.
///
/// Allocation and relocation construction precede mutation of vector
/// contents, so an allocation failure cannot leave a half-written
/// instruction (it also cannot be converted into a recoverable encoder
/// error — that would require a cross-file interface change).
fn i686_write_address(
    bytes: &mut Vec<u8>,
    relocations: &mut Vec<Relocation>,
    plan: &I686AddressPlan,
    pending: Option<I686PendingRelocation<'_>>,
    include_modrm: bool,
) {
    let encoded = plan.encoded(include_modrm);

    let relocation = pending.map(|pending| {
        debug_assert!(pending.modifier.is_none());
        debug_assert!(matches!(plan.len - plan.header_len, 2 | 4));

        i686_make_relocation(
            bytes.len() + plan.displacement_offset(include_modrm),
            pending.symbol,
            pending.reloc_type,
            pending.addend,
            pending.diff_symbol,
        )
    });

    bytes.reserve(encoded.len());
    if relocation.is_some() {
        relocations.reserve(1);
    }

    bytes.extend_from_slice(encoded);

    if let Some(relocation) = relocation {
        relocations.push(relocation);
    }
}

// ===========================================================================
// InstructionEncoder methods
// ===========================================================================

impl super::InstructionEncoder {
    /// Encode validated ModR/M fields.
    ///
    /// This interface cannot return a diagnostic; invalid fields are
    /// therefore programming errors, not silently masked operands.
    pub(super) fn modrm(&self, mod_: u8, reg: u8, rm: u8) -> u8 {
        assert!(
            mod_ < 4 && reg < 8 && rm < 8,
            "invalid i686 ModR/M fields: mod={}, reg={}, rm={}",
            mod_,
            reg,
            rm
        );

        (mod_ << 6) | (reg << 3) | rm
    }

    /// Encode validated SIB fields.
    ///
    /// Index field 4 is the legitimate "no index" sentinel here; actual
    /// memory operands reject ESP as an index before this low-level
    /// representation is formed.
    pub(super) fn sib(&self, scale: u8, index: u8, base: u8) -> u8 {
        assert!(
            index < 8 && base < 8,
            "invalid i686 SIB fields: index={}, base={}",
            index,
            base
        );

        let scale_bits = match scale {
            1 => 0,
            2 => 1,
            4 => 2,
            8 => 3,
            _ => panic!("invalid i686 SIB scale: {}", scale),
        };

        (scale_bits << 6) | (index << 3) | base
    }

    // Segment overrides are emitted ONCE by `InstructionEncoder::encode`
    // (the operand-scan splice in mod.rs) — no per-arm emission exists
    // anymore. The per-arm calls were the open defect class (an arm that
    // forgot the call silently dropped the override); the default-segment
    // drop rule (SS default for ebp/esp/bp-based forms, DS otherwise,
    // GAS-parity: `%ds` on `(%eax)`-based forms and `%ss` on
    // `%ebp`/`%esp`-based forms are dropped) and the FWAIT ordering
    // (`9b 26 67 d9 7f 08`) live there, GAS 2.44-verified.

    fn emit_i686_address_plan(
        &mut self,
        plan: &I686AddressPlan,
        relocation: Option<I686PendingRelocation<'_>>,
        include_modrm: bool,
    ) {
        i686_write_address(
            &mut self.bytes,
            &mut self.relocations,
            plan,
            relocation,
            include_modrm,
        );

        if plan.address16 != self.code16 {
            self.pending_addr32 = true;
        }
    }

    fn encode_i686_absolute_address(
        &mut self,
        reg_field: Option<u8>,
        label: &str,
    ) -> Result<(), String> {
        if let Some(reg_field) = reg_field {
            i686_validate_reg_field(reg_field)?;
        }

        let literal = i686_parse_address_literal(label)?;

        let (displacement, mut relocation) = match literal {
            Some(value) => (I686AddressValue::Constant(value), None),
            None => {
                let (symbol, addend) = split_label_offset(label.trim());

                (
                    I686AddressValue::Relocatable,
                    Some(I686PendingRelocation {
                        symbol,
                        reloc_type: R_386_32,
                        addend,
                        diff_symbol: None,
                        modifier: None,
                    }),
                )
            }
        };

        let plan = i686_plan_memory_address(
            self.code16,
            reg_field.unwrap_or(0),
            None,
            None,
            None,
            displacement,
        )?;

        if let Some(relocation) = &mut relocation {
            i686_validate_pending_relocation(relocation, plan.address16)?;
        }

        self.emit_i686_address_plan(&plan, relocation, reg_field.is_some());
        Ok(())
    }

    /// Emit the address field for the accumulator moffs forms A0/A1/A2/A3.
    ///
    /// Width follows the current code mode, not the operand width
    /// (GAS-verified: `movb 0x12, %al` in `.code16` is `a0 12 00`).
    /// Explicit address-size overrides require a caller contract not
    /// represented by this method's arguments.
    pub(super) fn encode_abs_addr_disp_only(&mut self, label: &str) -> Result<(), String> {
        self.encode_i686_absolute_address(None, label)
    }

    /// Encode a bare-label or numeric absolute memory address.
    ///
    /// `.code16`: mod=00 rm=110 + disp16, optionally R_386_16.
    /// `.code32`: mod=00 rm=101 + disp32, optionally R_386_32.
    pub(super) fn encode_abs_addr_modrm(
        &mut self,
        reg_field: u8,
        label: &str,
    ) -> Result<(), String> {
        self.encode_i686_absolute_address(Some(reg_field), label)
    }

    /// Encode ModR/M, optional SIB, and a memory displacement.
    ///
    /// The whole operand is validated and planned before any byte is
    /// appended. Relocation offsets point at the displacement field, so
    /// a failed operand never leaves a dangling relocation either.
    pub(super) fn encode_modrm_mem(
        &mut self,
        reg_field: u8,
        mem: &MemoryOperand,
    ) -> Result<(), String> {
        let folded = fold_index_into_base(mem);
        let mem = folded.as_ref().unwrap_or(mem);

        let (displacement, relocation) = i686_memory_displacement(&mem.displacement);

        let plan = i686_plan_memory_address(
            self.code16,
            reg_field,
            mem.base.as_ref().map(|reg| reg.name.as_str()),
            mem.index.as_ref().map(|reg| reg.name.as_str()),
            mem.scale,
            displacement,
        )?;

        self.i686_resolve_and_emit(&plan, relocation, true)
    }

    /// Bare-address form (moffs) of an absolute memory operand: the
    /// accumulator opcodes A0–A3 take the address field with no ModR/M
    /// byte, which GNU as uses for `mov abs, %eax`-style moves. Width
    /// follows the code mode exactly like every other absolute address.
    pub(super) fn encode_i686_moffs(&mut self, displacement: &Displacement) -> Result<(), String> {
        let (displacement, relocation) = i686_memory_displacement(displacement);
        let plan = i686_plan_memory_address(self.code16, 0, None, None, None, displacement)?;

        self.i686_resolve_and_emit(&plan, relocation, false)
    }

    /// Shared tail of every address emission: validate the pending
    /// relocation (before any byte is written), resolve TLS modifiers,
    /// then append the planned bytes and request the address-size
    /// override when the plan's width differs from the mode default.
    fn i686_resolve_and_emit(
        &mut self,
        plan: &I686AddressPlan,
        mut relocation: Option<I686PendingRelocation>,
        include_modrm: bool,
    ) -> Result<(), String> {
        if let Some(relocation) = &mut relocation {
            i686_validate_pending_relocation(relocation, plan.address16)?;

            if let Some(modifier) = relocation.modifier.take() {
                // GNU as rejects unknown modifiers; the mapper in
                // tls_reloc_type would silently degrade them to R_386_32.
                if !i686_known_tls_modifier(modifier) {
                    return Err(format!("unknown i686 relocation modifier: @{}", modifier));
                }
                relocation.reloc_type = self.tls_reloc_type(modifier);
            }
        }

        self.emit_i686_address_plan(plan, relocation, include_modrm);
        Ok(())
    }

    /// Add a relocation at the current output position.
    pub(super) fn add_relocation(&mut self, symbol: &str, reloc_type: u32, addend: i64) {
        let relocation = i686_make_relocation(self.bytes.len(), symbol, reloc_type, addend, None);
        self.relocations.push(relocation);
    }

    /// Add a relocation for a label with an optional signed offset.
    pub(super) fn add_relocation_for_label(&mut self, label: &str, reloc_type: u32) {
        let (symbol, addend) = split_label_offset(label);
        self.add_relocation(symbol, reloc_type, addend);
    }

    /// Record a symbol difference for resolution after layout.
    pub(super) fn add_relocation_with_diff(
        &mut self,
        symbol: &str,
        reloc_type: u32,
        addend: i64,
        diff_sym: &str,
    ) {
        let relocation =
            i686_make_relocation(self.bytes.len(), symbol, reloc_type, addend, Some(diff_sym));
        self.relocations.push(relocation);
    }
}

#[cfg(test)]
mod i686_encoding_helper_tests {
    use super::*;

    // ------------------------------------------------------------- helpers

    fn register(name: &str) -> Register {
        Register {
            name: name.to_owned(),
            mask: None,
            zeroing: false,
            sae: false,
            rounding: None,
        }
    }

    fn memory_operand(
        displacement: Displacement,
        base: Option<&str>,
        index: Option<&str>,
        scale: Option<u8>,
        segment: Option<&str>,
    ) -> MemoryOperand {
        MemoryOperand {
            segment: segment.map(str::to_owned),
            displacement,
            base: base.map(register),
            index: index.map(register),
            scale,
            mask: None,
            zeroing: false,
            broadcast: None,
        }
    }

    fn numeric_plan(
        code16: bool,
        base: Option<&str>,
        index: Option<&str>,
        scale: Option<u8>,
        displacement: i64,
    ) -> I686AddressPlan {
        i686_plan_memory_address(
            code16,
            0,
            base,
            index,
            scale,
            I686AddressValue::Constant(displacement),
        )
        .unwrap()
    }

    fn pending(symbol: &str) -> I686PendingRelocation<'_> {
        I686PendingRelocation {
            symbol,
            reloc_type: R_386_32,
            addend: 0,
            diff_symbol: None,
            modifier: None,
        }
    }

    // ------------------------------------------------------- 32-bit goldens

    #[test]
    fn golden_32_bit_encodings() {
        // Byte shapes verified against GNU as 2.44 (`as --32`).
        let cases: &[(Option<&str>, Option<&str>, Option<u8>, i64, &[u8])] = &[
            (Some("eax"), None, None, 0, &[0x00]),
            (Some("esp"), None, None, 0, &[0x04, 0x24]),
            (Some("ebp"), None, None, 0, &[0x45, 0x00]),
            (
                None,
                None,
                None,
                0x1234_5678,
                &[0x05, 0x78, 0x56, 0x34, 0x12],
            ),
            (None, Some("ecx"), Some(4), 0, &[0x04, 0x8D, 0, 0, 0, 0]),
            (None, Some("ebp"), Some(1), 0, &[0x04, 0x2D, 0, 0, 0, 0]),
            (Some("ebp"), Some("ecx"), Some(4), 0, &[0x44, 0x8D, 0]),
            (Some("eax"), None, None, -128, &[0x40, 0x80]),
            (Some("eax"), None, None, 127, &[0x40, 0x7F]),
            (Some("eax"), None, None, 128, &[0x80, 0x80, 0, 0, 0]),
            (
                Some("eax"),
                None,
                None,
                -129,
                &[0x80, 0x7F, 0xFF, 0xFF, 0xFF],
            ),
            // Modular rule: u32::MAX is -1 -> disp8.
            (Some("eax"), None, None, 0xFFFF_FFFF, &[0x40, 0xFF]),
            // Modular rule: fits u32, signed is -0x80000000 -> disp32.
            (
                Some("eax"),
                None,
                None,
                0x8000_0000,
                &[0x80, 0x00, 0x00, 0x00, 0x80],
            ),
            // Outside the union: wrap + force full width.
            (Some("eax"), None, None, 0x1_0000_0000, &[0x80, 0, 0, 0, 0]),
        ];

        for &(base, index, scale, displacement, expected) in cases {
            let plan = numeric_plan(false, base, index, scale, displacement);
            assert_eq!(plan.encoded(true), expected);
            assert!(!plan.address16);
        }
    }

    // ------------------------------------------------------- 16-bit goldens

    #[test]
    fn golden_16_bit_encodings() {
        let cases: &[(Option<&str>, Option<&str>, i64, &[u8])] = &[
            (None, None, 0x1234, &[0x06, 0x34, 0x12]),
            (Some("bp"), None, 0, &[0x46, 0]),
            (Some("bx"), Some("si"), 0, &[0x00]),
            (Some("bx"), Some("di"), 0, &[0x01]),
            (Some("bp"), Some("si"), 0, &[0x02]),
            (Some("bp"), Some("di"), 0, &[0x03]),
            (None, Some("si"), 0, &[0x04]),
            (None, Some("di"), 0, &[0x05]),
            (Some("bx"), None, 0xFFFF, &[0x47, 0xFF]),
            (Some("bx"), None, 128, &[0x87, 0x80, 0]),
            // Outside the 16-bit union: wrap + force disp16.
            (Some("bx"), None, 0x1_0000, &[0x87, 0, 0]),
            (Some("bx"), None, -129, &[0x87, 0x7F, 0xFF]),
        ];

        for &(base, index, displacement, expected) in cases {
            let plan = numeric_plan(true, base, index, None, displacement);
            assert_eq!(plan.encoded(true), expected);
            assert!(plan.address16);
        }

        // .code32 with 16-bit registers: the same table, but the caller
        // (via pending_addr32) receives a 0x67 address-size override.
        // (The absolute (None, None) row is excluded: in .code32 an
        // absolute address keeps 32-bit width.)
        for &(base, index, displacement, expected) in cases {
            if base.is_none() && index.is_none() {
                continue;
            }
            let plan = i686_plan_memory_address(
                false,
                0,
                base,
                index,
                None,
                I686AddressValue::Constant(displacement),
            )
            .unwrap();
            assert_eq!(plan.encoded(true), expected);
            assert!(plan.address16);
        }

        let plan =
            i686_plan_memory_address(true, 2, None, None, None, I686AddressValue::Relocatable)
                .unwrap();

        assert_eq!(plan.encoded(true), &[0x16, 0, 0]);
        assert_eq!(plan.encoded(false), &[0, 0]);
    }

    // ------------------------------------------------------- reg field

    #[test]
    fn register_field_changes_only_reg_bits() {
        for reg_field in 0u8..8 {
            let plan = i686_plan_memory_address(
                false,
                reg_field,
                Some("esp"),
                Some("ecx"),
                Some(4),
                I686AddressValue::Constant(127),
            )
            .unwrap();

            assert_eq!(plan.encoded(true), &[0x44 | (reg_field << 3), 0x8C, 0x7F]);
        }

        for reg_field in 8u8..=u8::MAX {
            assert!(
                i686_plan_memory_address(
                    false,
                    reg_field,
                    Some("eax"),
                    None,
                    None,
                    I686AddressValue::Constant(0),
                )
                .is_err()
            );
        }
    }

    // ------------------------------------------------------- rejection

    #[test]
    fn rejects_invalid_registers_combinations_and_scales() {
        let invalid = [
            // Registers that reg_num would silently remap (old bugs).
            (false, Some("eax"), Some("esp"), Some(1)),
            (false, None, Some("esp"), Some(1)),
            (false, Some("esp"), Some("esp"), Some(2)),
            (false, Some("rax"), None, None),
            (false, Some("ax"), None, None),
            (false, Some("al"), None, None),
            (false, Some("ah"), None, None),
            (false, Some("xmm0"), None, None),
            (false, Some("unknown"), None, None),
            (false, None, Some("xmm1"), Some(2)),
            // Scales.
            (false, Some("eax"), None, Some(2)),
            (false, Some("eax"), Some("ecx"), Some(3)),
            (false, Some("eax"), Some("ecx"), Some(5)),
            (false, Some("eax"), Some("ecx"), Some(0)),
            // Mixed-width, illegal-combo and scaled 16-bit forms GAS
            // rejects (in either code mode).
            (false, Some("bx"), Some("eax"), None),
            (false, Some("eax"), Some("si"), None),
            (false, Some("bx"), None, Some(2)),
            (false, Some("bx"), Some("si"), Some(2)),
            (true, Some("bx"), Some("eax"), None),
            (true, Some("eax"), Some("si"), None),
            (true, Some("bx"), Some("si"), Some(2)),
            (true, Some("si"), Some("bx"), None),
            (true, Some("si"), Some("di"), None),
            (true, Some("bx"), Some("bp"), None),
            (true, Some("sp"), None, None),
            (true, Some("si"), None, Some(4)),
        ];

        for &(code16, base, index, scale) in &invalid {
            assert!(
                i686_plan_memory_address(
                    code16,
                    0,
                    base,
                    index,
                    scale,
                    I686AddressValue::Constant(0),
                )
                .is_err(),
                "expected rejection: code16={} base={:?} index={:?} \
                 scale={:?}",
                code16,
                base,
                index,
                scale
            );
        }

        for scale in 0u8..=u8::MAX {
            let result = i686_plan_memory_address(
                false,
                0,
                Some("eax"),
                Some("ecx"),
                Some(scale),
                I686AddressValue::Constant(0),
            );
            assert_eq!(result.is_ok(), matches!(scale, 1 | 2 | 4 | 8));
        }

        // Scale without an index is rejected like GNU as.
        assert!(
            i686_plan_memory_address(
                false,
                0,
                Some("eax"),
                None,
                Some(2),
                I686AddressValue::Constant(0),
            )
            .is_err()
        );
    }

    // ------------------------------------------------------- normalization

    #[test]
    fn displacement_modular_normalization_matches_gas() {
        // 16-bit width.
        for (value, expected) in [
            (0i64, (0u32, true)),
            (127, (127, true)),
            (-128, (0xFF80, true)),
            (32767, (0x7FFF, true)),
            (-32768, (0x8000, true)),
            (65535, (0xFFFF, true)),
            (65536, (0, false)),
            (-32769, (0x7FFF, false)),
            (i64::MIN, (0, false)),
            (i64::MAX, (0xFFFF, false)),
        ] {
            let got = I686AddressValue::Constant(value).normalize(true);
            assert_eq!(got, expected, "value {} (16-bit)", value);
        }

        // 32-bit width.
        for (value, expected) in [
            (0i64, (0u32, true)),
            (0xFFFF_FFFF, (0xFFFF_FFFF, true)),
            (0x8000_0000, (0x8000_0000, true)),
            (0x1_0000_0000, (0, false)),
            (-2147483649i64, (0x7FFF_FFFF, false)),
            (i64::MIN, (0, false)),
            (i64::MAX, (0xFFFF_FFFF, false)),
        ] {
            let got = I686AddressValue::Constant(value).normalize(false);
            assert_eq!(got, expected, "value {} (32-bit)", value);
        }

        assert_eq!(I686AddressValue::Relocatable.normalize(true), (0, false));
        assert_eq!(I686AddressValue::Relocatable.normalize(false), (0, false));
    }

    // ------------------------------------------------------- literals

    #[test]
    fn integer_literals_preserve_decimal_contract() {
        let cases = [
            ("0", 0),
            ("010", 10),
            ("08", 8),
            ("123", 123),
            ("-123", -123),
            ("+123", 123),
            ("0xff", 255),
            ("0XFFFF", 65535),
            ("0b101", 5),
            ("0B101", 5),
            ("-0x80", -128),
            (" 4294967295 ", 4294967295),
            ("-9223372036854775808", i64::MIN),
            ("9223372036854775807", i64::MAX),
        ];

        for (text, expected) in cases {
            assert_eq!(i686_parse_address_literal(text), Ok(Some(expected)));
        }

        for text in [
            "symbol", ".Ltmp0", "1f", "1b", "0b", "12f", "symbol+4", "symbol-4",
        ] {
            assert_eq!(i686_parse_address_literal(text), Ok(None));
        }

        for text in [
            "",
            "+",
            "-",
            "0x",
            "0X",
            "0B",
            "0xGG",
            "0b102",
            "9223372036854775808",
            "-9223372036854775809",
            "18446744073709551616",
        ] {
            assert!(i686_parse_address_literal(text).is_err());
        }
    }

    // ------------------------------------------------------- segments

    #[test]
    fn segment_prefixes() {
        for (name, prefix) in [
            ("es", 0x26),
            ("cs", 0x2E),
            ("ss", 0x36),
            ("ds", 0x3E),
            ("fs", 0x64),
            ("gs", 0x65),
        ] {
            assert_eq!(i686_segment_prefix(name), Some(prefix));
        }

        for name in ["", "eax", "ssx", " fs", "FS", "%fs", "fS"] {
            assert_eq!(i686_segment_prefix(name), None);
        }
    }

    #[test]
    fn default_segment_selection_matches_gas() {
        // 32-bit addressing: SS for esp/ebp bases, DS otherwise.
        assert_eq!(i686_default_segment(Some("ebp"), None), 0x36);
        assert_eq!(i686_default_segment(Some("esp"), None), 0x36);
        assert_eq!(i686_default_segment(Some("eax"), None), 0x3E);
        assert_eq!(i686_default_segment(None, Some("ebp")), 0x3E);
        assert_eq!(i686_default_segment(None, None), 0x3E);
        assert_eq!(i686_default_segment(Some("eax"), Some("ebp")), 0x3E);

        // 16-bit addressing: SS when %bp participates, DS otherwise.
        assert_eq!(i686_default_segment(Some("bp"), None), 0x36);
        assert_eq!(i686_default_segment(None, Some("bp")), 0x36);
        assert_eq!(i686_default_segment(Some("bp"), Some("si")), 0x36);
        assert_eq!(i686_default_segment(Some("bx"), None), 0x3E);
        assert_eq!(i686_default_segment(None, Some("si")), 0x3E);
    }

    fn encode_mov_from_segmented_mem(base: Option<&str>, segment: Option<&str>) -> Vec<u8> {
        // `mov <seg>:8(%<base>), %eax` through the FULL encode() path, so
        // the test exercises the dispatch-level operand-segment splice in
        // mod.rs (the single emission point) rather than any per-arm call.
        let mem = memory_operand(Displacement::Integer(8), base, None, None, segment);
        let instr = Instruction {
            prefix: None,
            mnemonic: "movl".to_string(),
            operands: vec![Operand::Memory(mem), Operand::Register(register("eax"))],
            nf: false,
            force_evex: false,
            force_rex2: false,
            dfv: 0,
        };
        let mut encoder = InstructionEncoder::new();
        encoder.encode(&instr).expect("encoding must succeed");
        encoder.bytes
    }

    #[test]
    fn encode_splices_segment_override_and_drops_redundant() {
        // GAS: `mov %ds:8(%eax),%eax` -> 8b 40 08 (ds is the default for
        // eax-based forms: dropped, no prefix byte at all; modrm 40 =
        // mod01 reg000 rm000 for dst=%eax base=%eax).
        let bytes = encode_mov_from_segmented_mem(Some("eax"), Some("ds"));
        assert_eq!(bytes, [0x8B, 0x40, 0x08]);

        // GAS: `mov %ss:8(%ebp),%eax` -> 8b 45 08 (ss is the default for
        // ebp-based forms: dropped; modrm 45 = mod01 reg000 rm101).
        let bytes = encode_mov_from_segmented_mem(Some("ebp"), Some("ss"));
        assert_eq!(bytes, [0x8B, 0x45, 0x08]);

        // GAS: `mov %ds:8(%ebp),%eax` -> 3e 8b 45 08 (ds differs from the
        // SS default: spliced as the OUTERMOST prefix).
        let bytes = encode_mov_from_segmented_mem(Some("ebp"), Some("ds"));
        assert_eq!(bytes, [0x3E, 0x8B, 0x45, 0x08]);

        // GAS: `mov %ss:8(%eax),%eax` -> 36 8b 40 08 (ss differs from the
        // DS default).
        let bytes = encode_mov_from_segmented_mem(Some("eax"), Some("ss"));
        assert_eq!(bytes, [0x36, 0x8B, 0x40, 0x08]);

        // All six names are emitted (verified byte values) for an
        // eax-based (DS-default) form; `ds` alone is dropped.
        for (name, prefix) in [
            ("es", 0x26),
            ("cs", 0x2E),
            ("ss", 0x36),
            ("fs", 0x64),
            ("gs", 0x65),
        ] {
            let bytes = encode_mov_from_segmented_mem(Some("eax"), Some(name));
            assert_eq!(bytes[0], prefix, "segment {}", name);
            assert_eq!(&bytes[1..], &[0x8B, 0x40, 0x08], "segment {}", name);
        }
    }

    // ------------------------------------------------------- displacement

    #[test]
    fn displacement_variants_preserve_expression_information() {
        let symbolic = [
            (Displacement::Symbol("s".to_owned()), 0, None, None),
            (
                Displacement::SymbolAddend("s".to_owned(), -7),
                -7,
                None,
                None,
            ),
            (
                Displacement::SymbolPlusOffset("s".to_owned(), 9),
                9,
                None,
                None,
            ),
            (
                Displacement::SymbolDiff("s".to_owned(), "d".to_owned()),
                0,
                Some("d"),
                None,
            ),
            (
                Displacement::SymbolDiffAddend("s".to_owned(), "d".to_owned(), -11),
                -11,
                Some("d"),
                None,
            ),
            (
                Displacement::SymbolMod("s".to_owned(), "TPOFF".to_owned()),
                0,
                None,
                Some("TPOFF"),
            ),
        ];

        for (displacement, addend, diff, modifier) in &symbolic {
            let (value, relocation) = i686_memory_displacement(displacement);
            let relocation = relocation.unwrap();

            assert_eq!(value, I686AddressValue::Relocatable);
            assert_eq!(relocation.symbol, "s");
            assert_eq!(relocation.reloc_type, R_386_32);
            assert_eq!(relocation.addend, *addend);
            assert_eq!(relocation.diff_symbol, *diff);
            assert_eq!(relocation.modifier, *modifier);
        }

        let (value, relocation) = i686_memory_displacement(&Displacement::None);
        assert_eq!(value, I686AddressValue::Constant(0));
        assert!(relocation.is_none());

        let (value, relocation) = i686_memory_displacement(&Displacement::Integer(-17));
        assert_eq!(value, I686AddressValue::Constant(-17));
        assert!(relocation.is_none());
    }

    // ------------------------------------------------------- relocation

    #[test]
    fn relocation_width_validation() {
        let mut ordinary = pending("s");
        ordinary.addend = i64::MAX;
        i686_validate_pending_relocation(&mut ordinary, true).unwrap();
        assert_eq!(ordinary.reloc_type, R_386_16);
        assert_eq!(ordinary.addend, i64::MAX);

        let mut difference = pending("s");
        difference.diff_symbol = Some("d");
        assert!(i686_validate_pending_relocation(&mut difference, true).is_err());
        assert!(i686_validate_pending_relocation(&mut difference, false).is_ok());

        let mut modified = pending("s");
        modified.modifier = Some("TPOFF");
        assert!(i686_validate_pending_relocation(&mut modified, true).is_err());
        assert!(i686_validate_pending_relocation(&mut modified, false).is_ok());

        for symbol in ["", "@PLT"] {
            let mut relocation = pending(symbol);
            assert!(i686_validate_pending_relocation(&mut relocation, false).is_err());
        }

        let mut empty_difference = pending("s");
        empty_difference.diff_symbol = Some("");
        assert!(i686_validate_pending_relocation(&mut empty_difference, false).is_err());
    }

    #[test]
    fn relocation_normalization_preserves_existing_contract() {
        let plt = i686_make_relocation(7, "target@PLT", R_386_PC32, -4, None);
        assert_eq!(plt.offset, 7);
        assert_eq!(plt.symbol, "target");
        assert_eq!(plt.reloc_type, R_386_PLT32);
        assert_eq!(plt.addend, -4);
        assert_eq!(plt.diff_symbol, None);

        let absolute = i686_make_relocation(9, "target@PLT", R_386_32, 5, None);
        assert_eq!(absolute.symbol, "target");
        assert_eq!(absolute.reloc_type, R_386_32);

        let difference = i686_make_relocation(11, "target@PLT", R_386_32, 6, Some("origin"));
        assert_eq!(difference.symbol, "target@PLT");
        assert_eq!(difference.reloc_type, R_386_32);
        assert_eq!(difference.diff_symbol.as_deref(), Some("origin"));
    }

    // ------------------------------------------------------- emission

    #[test]
    fn actual_emission_records_displacement_offsets_and_zero_fields() {
        let cases = [
            (false, None, None, None),
            (false, Some("eax"), None, None),
            (false, Some("ebp"), None, None),
            (false, Some("esp"), Some("ecx"), Some(4)),
            (false, None, Some("ebp"), Some(8)),
            (true, None, None, None),
            (true, Some("bx"), None, None),
            (true, Some("bp"), Some("di"), None),
        ];

        for (code16, base, index, scale) in cases {
            let plan = i686_plan_memory_address(
                code16,
                3,
                base,
                index,
                scale,
                I686AddressValue::Relocatable,
            )
            .unwrap();

            let mut relocation = pending("target");
            relocation.addend = -37;
            i686_validate_pending_relocation(&mut relocation, plan.address16).unwrap();

            let mut output = vec![0x66, 0x8B, 0xAA];
            let original_len = output.len();
            let mut relocations = Vec::new();

            i686_write_address(&mut output, &mut relocations, &plan, Some(relocation), true);

            assert_eq!(&output[..original_len], &[0x66, 0x8B, 0xAA]);
            assert_eq!(&output[original_len..], plan.encoded(true));
            assert_eq!(relocations.len(), 1);

            let relocation = &relocations[0];
            let offset = original_len + usize::from(plan.header_len);
            let width = if plan.address16 { 2 } else { 4 };

            assert_eq!(relocation.offset, offset as u64);
            assert_eq!(relocation.symbol, "target");
            assert_eq!(relocation.addend, -37);
            assert_eq!(relocation.diff_symbol, None);
            assert_eq!(
                relocation.reloc_type,
                if plan.address16 { R_386_16 } else { R_386_32 }
            );
            assert_eq!(output.len(), offset + width);
            assert!(output[offset..].iter().all(|&byte| byte == 0));

            // Later instruction bytes must not change the recorded offset.
            output.extend_from_slice(&[0x12, 0x34]);
            assert_eq!(relocations[0].offset, offset as u64);
        }
    }

    #[test]
    fn actual_moffs_emission_omits_modrm() {
        for code16 in [false, true] {
            let plan = i686_plan_memory_address(
                code16,
                0,
                None,
                None,
                None,
                I686AddressValue::Relocatable,
            )
            .unwrap();

            let mut relocation = pending("target");
            relocation.addend = 23;
            i686_validate_pending_relocation(&mut relocation, plan.address16).unwrap();

            let mut output = vec![0xA1];
            let mut relocations = Vec::new();

            i686_write_address(
                &mut output,
                &mut relocations,
                &plan,
                Some(relocation),
                false,
            );

            assert_eq!(relocations.len(), 1);
            assert_eq!(relocations[0].offset, 1);
            assert_eq!(relocations[0].addend, 23);
            assert_eq!(output.len(), if code16 { 3 } else { 5 });
            assert!(output[1..].iter().all(|&byte| byte == 0));
        }
    }

    #[test]
    fn numeric_emission_does_not_create_relocations() {
        let plan = numeric_plan(false, Some("eax"), Some("ecx"), Some(8), -128);

        let mut output = Vec::new();
        let mut relocations = Vec::new();

        i686_write_address(&mut output, &mut relocations, &plan, None, true);

        assert_eq!(output.as_slice(), &[0x44, 0xC8, 0x80]);
        assert!(relocations.is_empty());
    }

    #[test]
    fn actual_difference_emission_preserves_both_symbols() {
        let plan = i686_plan_memory_address(
            false,
            0,
            Some("ebp"),
            None,
            None,
            I686AddressValue::Relocatable,
        )
        .unwrap();

        let mut relocation = pending("target");
        relocation.diff_symbol = Some("origin");
        relocation.addend = -19;
        i686_validate_pending_relocation(&mut relocation, false).unwrap();

        let mut output = vec![0x8B];
        let mut relocations = Vec::new();

        i686_write_address(&mut output, &mut relocations, &plan, Some(relocation), true);

        assert_eq!(output.as_slice(), &[0x8B, 0x85, 0, 0, 0, 0]);
        assert_eq!(relocations.len(), 1);
        assert_eq!(relocations[0].offset, 2);
        assert_eq!(relocations[0].symbol, "target");
        assert_eq!(relocations[0].diff_symbol.as_deref(), Some("origin"));
        assert_eq!(relocations[0].addend, -19);
    }

    // ------------------------------------------------------- the fold

    #[test]
    fn fold_index_into_base_beats_gas_but_never_changes_semantics() {
        // GAS 2.44 emits `8b 04 0d ff ff ff ff` (8 bytes) for this; the
        // fold produces `8b 41 ff` (3 bytes). Round-trip verified via the
        // decoder below: both compute `ecx - 1` with DS default.
        let mem = memory_operand(Displacement::Integer(-1), None, Some("ecx"), Some(1), None);
        let folded = fold_index_into_base(&mem).unwrap();
        assert!(folded.base.is_some());
        assert_eq!(folded.base.as_ref().unwrap().name, "ecx");
        assert!(folded.index.is_none());
        assert!(folded.scale.is_none());

        let unfolded = i686_plan_memory_address(
            false,
            0,
            None,
            Some("ecx"),
            Some(1),
            I686AddressValue::Constant(-1),
        )
        .unwrap();
        assert_eq!(
            unfolded.encoded(true),
            &[0x04, 0x0D, 0xFF, 0xFF, 0xFF, 0xFF]
        );

        let folded_plan = i686_plan_memory_address(
            false,
            0,
            Some("ecx"),
            None,
            None,
            I686AddressValue::Constant(-1),
        )
        .unwrap();
        assert_eq!(folded_plan.encoded(true), &[0x41, 0xFF]);

        // No fold: a base is present.
        let mem = memory_operand(
            Displacement::Integer(-1),
            Some("eax"),
            Some("ecx"),
            Some(1),
            None,
        );
        assert!(fold_index_into_base(&mem).is_none());

        // No fold: scale != 1.
        let mem = memory_operand(Displacement::Integer(-1), None, Some("ecx"), Some(2), None);
        assert!(fold_index_into_base(&mem).is_none());

        // No fold: ebp would change the default segment DS -> SS.
        let mem = memory_operand(Displacement::Integer(-1), None, Some("ebp"), Some(1), None);
        assert!(fold_index_into_base(&mem).is_none());

        // No fold: esp is not a legal index (rejected downstream).
        let mem = memory_operand(Displacement::Integer(-1), None, Some("esp"), Some(1), None);
        assert!(fold_index_into_base(&mem).is_none());

        // Symbolic displacements fold too: GAS emits
        // `8b 04 0d 00000000` + R_386_32 (7 bytes); the folded form is
        // `8b 01 00000000` + R_386_32 (6 bytes) with the same relocation
        // semantics (absolute field, addend in the record).
        let symbolic = memory_operand(
            Displacement::Symbol("target".to_owned()),
            None,
            Some("ecx"),
            Some(1),
            None,
        );
        let mut encoder = InstructionEncoder::new();
        encoder.bytes.push(0x8B);
        encoder.encode_modrm_mem(0, &symbolic).unwrap();
        assert_eq!(&encoder.bytes[..], &[0x8B, 0x81, 0, 0, 0, 0]);
        assert_eq!(encoder.relocations.len(), 1);
        assert_eq!(encoder.relocations[0].offset, 2);
        assert_eq!(encoder.relocations[0].symbol, "target");
        assert_eq!(encoder.relocations[0].reloc_type, R_386_32);
    }

    // ------------------------------------------------------- encoder-level

    #[test]
    fn encoder_modrm_mem_validates_before_mutating() {
        // A failing operand must leave bytes and relocations untouched.
        let mut encoder = InstructionEncoder::new();
        encoder.bytes.push(0x8B);
        let bad = memory_operand(
            Displacement::Integer(0),
            Some("eax"),
            Some("esp"),
            Some(2),
            None,
        );
        assert!(encoder.encode_modrm_mem(3, &bad).is_err());
        assert_eq!(encoder.bytes, [0x8B]);
        assert!(encoder.relocations.is_empty());
        assert!(!encoder.pending_addr32);

        // Same for the absolute paths.
        assert!(encoder.encode_abs_addr_modrm(3, "").is_err());
        assert_eq!(encoder.bytes, [0x8B]);
        assert!(encoder.relocations.is_empty());

        // And for unknown relocation modifiers (GNU as rejects them).
        let unknown_mod = memory_operand(
            Displacement::SymbolMod("s".to_owned(), "BOGUS".to_owned()),
            Some("eax"),
            None,
            None,
            None,
        );
        assert!(encoder.encode_modrm_mem(3, &unknown_mod).is_err());
        assert_eq!(encoder.bytes, [0x8B]);
        assert!(encoder.relocations.is_empty());
    }

    #[test]
    fn known_tls_modifiers_reach_the_existing_mapper() {
        for (modifier, expected) in [
            ("NTPOFF", R_386_TLS_LE_32),
            ("TPOFF", R_386_32S),
            ("TLSGD", R_386_TLS_GD),
            ("TLSLDM", R_386_TLS_LDM),
            ("DTPOFF", R_386_TLS_LDO_32),
            ("GOT", R_386_GOT32),
            ("GOTOFF", R_386_GOTOFF),
            ("PLT", R_386_PLT32),
            ("GOTPC", R_386_GOTPC),
            ("GOTNTPOFF", R_386_TLS_GOTIE),
            ("INDNTPOFF", R_386_TLS_GOTIE),
        ] {
            assert!(i686_known_tls_modifier(modifier));

            let mut encoder = InstructionEncoder::new();
            encoder.bytes.push(0x8B);
            let mem = memory_operand(
                Displacement::SymbolMod("s".to_owned(), modifier.to_owned()),
                Some("eax"),
                None,
                None,
                None,
            );
            encoder.encode_modrm_mem(3, &mem).unwrap();

            assert_eq!(encoder.relocations.len(), 1);
            assert_eq!(encoder.relocations[0].reloc_type, expected);
            assert_eq!(encoder.relocations[0].symbol, "s");
            assert_eq!(encoder.relocations[0].offset, 2);
            assert_eq!(&encoder.bytes[..], &[0x8B, 0x98, 0, 0, 0, 0]);
        }
    }

    #[test]
    fn code16_pending_addr32_flag() {
        // 32-bit register in .code16: 0x67 requested, 32-bit table.
        let mut encoder = InstructionEncoder::new();
        encoder.code16 = true;
        encoder.bytes.push(0x8B);
        let mem = memory_operand(Displacement::Integer(0), Some("eax"), None, None, None);
        encoder.encode_modrm_mem(0, &mem).unwrap();
        assert_eq!(encoder.bytes, [0x8B, 0x00]);
        assert!(encoder.pending_addr32);

        // 16-bit register in .code16: no override, 16-bit table.
        let mut encoder = InstructionEncoder::new();
        encoder.code16 = true;
        encoder.bytes.push(0x8B);
        let mem = memory_operand(Displacement::Integer(0), Some("bx"), None, None, None);
        encoder.encode_modrm_mem(0, &mem).unwrap();
        assert_eq!(encoder.bytes, [0x8B, 0x07]);
        assert!(!encoder.pending_addr32);

        // Absolute in .code16 stays 16-bit.
        let mut encoder = InstructionEncoder::new();
        encoder.code16 = true;
        encoder.bytes.push(0xA1);
        encoder.encode_abs_addr_disp_only("target").unwrap();
        assert_eq!(encoder.bytes, [0xA1, 0, 0]);
        assert_eq!(encoder.relocations[0].reloc_type, R_386_16);
        assert_eq!(encoder.relocations[0].offset, 1);
        assert!(!encoder.pending_addr32);

        // Symbolic displacement on a 16-bit base: mod=10 + disp16
        // (GAS: `mov sym(%bx),%ax` -> 8b 87 0000 + R_386_16).
        let mut encoder = InstructionEncoder::new();
        encoder.code16 = true;
        encoder.bytes.push(0x8B);
        let mem = memory_operand(
            Displacement::Symbol("target".to_owned()),
            Some("bx"),
            None,
            None,
            None,
        );
        encoder.encode_modrm_mem(0, &mem).unwrap();
        assert_eq!(encoder.bytes, [0x8B, 0x87, 0, 0]);
        assert_eq!(encoder.relocations[0].reloc_type, R_386_16);
        assert_eq!(encoder.relocations[0].offset, 2);
        assert!(!encoder.pending_addr32);
    }

    #[test]
    fn absolute_address_paths_match_gas_widths() {
        // .code32: moffs disp32 for numeric and symbolic.
        let mut encoder = InstructionEncoder::new();
        encoder.bytes.push(0xA1);
        encoder.encode_abs_addr_disp_only("0x123456789").unwrap();
        assert_eq!(encoder.bytes, [0xA1, 0x89, 0x67, 0x45, 0x23]);
        assert!(encoder.relocations.is_empty());

        let mut encoder = InstructionEncoder::new();
        encoder.bytes.push(0xA1);
        encoder.encode_abs_addr_disp_only("target").unwrap();
        assert_eq!(encoder.bytes, [0xA1, 0, 0, 0, 0]);
        assert_eq!(encoder.relocations[0].reloc_type, R_386_32);

        // .code32: ModR/M form `mov sym, %ebx` -> 8b 1d disp32.
        let mut encoder = InstructionEncoder::new();
        encoder.bytes.push(0x8B);
        encoder.encode_abs_addr_modrm(3, "target").unwrap();
        assert_eq!(encoder.bytes, [0x8B, 0x1D, 0, 0, 0, 0]);
        assert_eq!(encoder.relocations[0].offset, 2);

        // .code16: ModR/M form `movw sym, %dx` -> 8b 16 disp16.
        let mut encoder = InstructionEncoder::new();
        encoder.code16 = true;
        encoder.bytes.push(0x8B);
        encoder.encode_abs_addr_modrm(2, "target").unwrap();
        assert_eq!(encoder.bytes, [0x8B, 0x16, 0, 0]);
        assert_eq!(encoder.relocations[0].reloc_type, R_386_16);
        assert_eq!(encoder.relocations[0].offset, 2);

        // .code16: numeric hex literal `mov 0x1234, %dx`.
        let mut encoder = InstructionEncoder::new();
        encoder.code16 = true;
        encoder.bytes.push(0x8B);
        encoder.encode_abs_addr_modrm(2, "0x1234").unwrap();
        assert_eq!(encoder.bytes, [0x8B, 0x16, 0x34, 0x12]);
        assert!(encoder.relocations.is_empty());
    }

    // ------------------------------------------------------- decoder

    /// Independent legacy-address decoder.
    ///
    /// Returns the effective offset and whether the default segment is
    /// SS. It does not inspect the planner's displacement-length
    /// metadata.
    fn decode_address(encoded: &[u8], address16: bool, registers: &[u32; 8]) -> (u32, bool) {
        let modrm = encoded[0];
        let mode = modrm >> 6;
        let rm = modrm & 7;
        assert_ne!(mode, 3);

        let mut cursor = 1;

        let (base, displacement_len, default_ss) = if address16 {
            if mode == 0 && rm == 6 {
                (0, 2, false)
            } else {
                let base = match rm {
                    0 => registers[3].wrapping_add(registers[6]),
                    1 => registers[3].wrapping_add(registers[7]),
                    2 => registers[5].wrapping_add(registers[6]),
                    3 => registers[5].wrapping_add(registers[7]),
                    4 => registers[6],
                    5 => registers[7],
                    6 => registers[5],
                    7 => registers[3],
                    _ => unreachable!(),
                };

                let len = match mode {
                    0 => 0,
                    1 => 1,
                    2 => 2,
                    _ => unreachable!(),
                };

                (base, len, matches!(rm, 2 | 3 | 6))
            }
        } else {
            let (base, forced_disp32, default_ss) = if rm == 4 {
                let sib = encoded[cursor];
                cursor += 1;

                let scale = 1u32 << (sib >> 6);
                let index = (sib >> 3) & 7;
                let base_register = sib & 7;
                let no_base = mode == 0 && base_register == 5;

                let index_value = if index == 4 {
                    0
                } else {
                    registers[usize::from(index)].wrapping_mul(scale)
                };

                let base_value = if no_base {
                    0
                } else {
                    registers[usize::from(base_register)]
                };

                (
                    base_value.wrapping_add(index_value),
                    no_base,
                    !no_base && matches!(base_register, 4 | 5),
                )
            } else if mode == 0 && rm == 5 {
                (0, true, false)
            } else {
                (registers[usize::from(rm)], false, matches!(rm, 4 | 5))
            };

            let len = if forced_disp32 {
                4
            } else {
                match mode {
                    0 => 0,
                    1 => 1,
                    2 => 4,
                    _ => unreachable!(),
                }
            };

            (base, len, default_ss)
        };

        let displacement = match displacement_len {
            0 => 0,
            1 => encoded[cursor] as i8 as i32 as u32,
            2 => u16::from_le_bytes([encoded[cursor], encoded[cursor + 1]]) as u32,
            4 => u32::from_le_bytes([
                encoded[cursor],
                encoded[cursor + 1],
                encoded[cursor + 2],
                encoded[cursor + 3],
            ]),
            _ => unreachable!(),
        };

        assert_eq!(cursor + displacement_len, encoded.len());

        let offset = base.wrapping_add(displacement);
        (if address16 { offset & 0xFFFF } else { offset }, default_ss)
    }

    #[test]
    fn generated_32_bit_addresses_preserve_offsets_and_segments() {
        let choices = [
            Some("eax"),
            Some("ecx"),
            Some("edx"),
            Some("ebx"),
            Some("esp"),
            Some("ebp"),
            Some("esi"),
            Some("edi"),
            None,
        ];

        let register_sets: [[u32; 8]; 2] = [
            [0; 8],
            [
                0xFFFF_FFF0,
                0x1234_5678,
                0x8000_0000,
                0x7FFF_FFFF,
                0x0102_0304,
                0xDEAD_BEEF,
                0xFFFF_0001,
                0x0000_FFFF,
            ],
        ];

        let displacements = [
            i32::MIN as i64,
            -129,
            -128,
            -1,
            0,
            1,
            127,
            128,
            i32::MAX as i64,
            0xFFFF_FF80,
            u32::MAX as i64,
        ];

        for (base_slot, &base) in choices.iter().enumerate() {
            for (index_slot, &index) in choices.iter().enumerate() {
                if index == Some("esp") {
                    continue;
                }

                for scale in [1u8, 2, 4, 8] {
                    if index.is_none() && scale != 1 {
                        continue;
                    }

                    for displacement in displacements {
                        let plan = numeric_plan(false, base, index, Some(scale), displacement);
                        assert!(!plan.address16);

                        for registers in &register_sets {
                            let base_value = registers.get(base_slot).copied().unwrap_or(0);

                            let index_value = registers
                                .get(index_slot)
                                .copied()
                                .unwrap_or(0)
                                .wrapping_mul(u32::from(scale));

                            let expected = base_value
                                .wrapping_add(index_value)
                                .wrapping_add(displacement as u32);

                            assert_eq!(
                                decode_address(plan.encoded(true), false, registers,),
                                (expected, matches!(base_slot, 4 | 5),)
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn all_16_bit_forms_preserve_offsets_and_segments() {
        let registers: [u32; 8] = [0, 0, 0, 0xFFF1, 0, 0x8123, 0x4321, 0xFEDC];

        let bx_si = registers[3].wrapping_add(registers[6]);
        let bx_di = registers[3].wrapping_add(registers[7]);
        let bp_si = registers[5].wrapping_add(registers[6]);
        let bp_di = registers[5].wrapping_add(registers[7]);

        let forms = [
            (Some("bx"), Some("si"), bx_si, false),
            (Some("bx"), Some("di"), bx_di, false),
            (Some("bp"), Some("si"), bp_si, true),
            (Some("bp"), Some("di"), bp_di, true),
            (Some("si"), None, registers[6], false),
            (None, Some("si"), registers[6], false),
            (Some("di"), None, registers[7], false),
            (None, Some("di"), registers[7], false),
            (Some("bp"), None, registers[5], true),
            (None, Some("bp"), registers[5], true),
            (Some("bx"), None, registers[3], false),
            (None, Some("bx"), registers[3], false),
            (None, None, 0, false),
        ];

        // Boundary displacements plus a full sweep of the bx form.
        let boundaries = [
            0i64, 1, 127, 128, 255, 256, 32767, -1, -128, -129, -32768, -32769, 65535, 65536,
        ];

        for (base, index, base_value, default_ss) in forms {
            for displacement in boundaries {
                let plan = numeric_plan(true, base, index, None, displacement);

                assert!(plan.address16);
                assert_eq!(
                    decode_address(plan.encoded(true), true, &registers,),
                    (
                        base_value.wrapping_add(displacement as u32) & 0xFFFF,
                        default_ss,
                    )
                );

                // Out-of-union literals (e.g. 65536) force a full-width
                // field but must decode to exactly the same effective
                // address as their wrapped value.
                let signed_plan = numeric_plan(true, base, index, None, displacement as i16 as i64);

                assert_eq!(
                    decode_address(plan.encoded(true), true, &registers,),
                    decode_address(signed_plan.encoded(true), true, &registers,)
                );
            }
        }

        // Full 65536-value sweep for one representative form, no heap
        // allocations in the loop.
        for displacement in 0u32..=u32::from(u16::MAX) {
            let plan = numeric_plan(true, Some("bx"), None, None, i64::from(displacement));

            assert!(plan.address16);
            assert_eq!(
                decode_address(plan.encoded(true), true, &registers),
                (registers[3].wrapping_add(displacement) & 0xFFFF, false,)
            );
        }
    }
}
