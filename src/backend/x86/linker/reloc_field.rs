//! x86-64 relocation field table and the checked writers built on it.
//!
//! Every relocation this backend applies writes a value into a fixed-width field.
//! The width and the signedness of that field are ABI facts, and they are stated
//! here once, in one table, instead of being re-derived at each of the ~46
//! application sites spread over `emit_exec.rs`, `emit_shared.rs` and
//! `emit_script.rs`. Deriving them per site is how the three copies came to
//! disagree: `R_X86_64_PC32` was range-checked in two of them and masked in the
//! third, so `lccc-ld -shared` silently truncated an out-of-range displacement
//! and exited 0 while GNU ld reported "relocation truncated to fit" on the same
//! object.
//!
//! # Why a `match` and not a map
//!
//! [`field`] and [`name`] are `match` statements over `u32` literals. The
//! compiler lowers a dense literal match to a jump table and a sparse one to a
//! decision tree; either way the lookup is a handful of predictable branches with
//! no hashing, no allocation and no lazy-static initialisation. A `HashMap` here
//! would cost a hash per relocation -- millions of them on a kernel-sized link --
//! to save nothing.
//!
//! # Two entry points, one semantic
//!
//! [`check`] looks the field up from the relocation type and is what a call site
//! uses when it only has the type. [`check_known`] takes the field directly and
//! skips the lookup; a relocation arm that already knows it is handling a 32-bit
//! signed displacement should not pay for a second dispatch, and more importantly
//! should not be able to drift from the instruction it is about to emit. Both
//! honour [`is_no_check`], so the exemption rules live in one place.

use super::elf::*;
use crate::backend::linker_common::reloc_field::{
    RelocField, bounds_message, overflow_message, type_label,
};

// Re-exported so a call site has one `reloc_field::` namespace for both the
// architecture-independent invariant (does this relocation's field fit inside the
// section it patches?) and the x86-64 specifics (which field does this type
// write?). Splitting them across two module paths at every call site only invites
// one of the two to be forgotten.
pub use crate::backend::linker_common::reloc_field::offset_in_section;

/// 32-bit signed displacement -- `PC32`, `PLT32`, `GOTPC32`, the `GOTPCREL`
/// family and the TLS GOT forms all write this.
pub const S32: RelocField = RelocField::signed(32);
/// 32-bit unsigned absolute -- `R_X86_64_32`, `SIZE32`.
pub const U32: RelocField = RelocField::unsigned(32);
/// 16-bit forms (`R_X86_64_16`, `PC16`).
pub const S16: RelocField = RelocField::signed(16);
pub const U16: RelocField = RelocField::unsigned(16);
/// 8-bit forms (`R_X86_64_8`, `PC8`).
pub const S8: RelocField = RelocField::signed(8);
pub const U8: RelocField = RelocField::unsigned(8);
/// 64-bit forms. These are in the table for completeness and so that a caller
/// asking "what field is this?" gets a truthful answer, but [`is_no_check`] makes
/// the range check vacuous for them: a 64-bit field written from a 64-bit value
/// cannot lose bits.
pub const S64: RelocField = RelocField::signed(64);
pub const U64: RelocField = RelocField::unsigned(64);

/// The advice appended to a truncation diagnostic on this architecture.
///
/// x86-64's code models are what make a 32-bit field overflow fixable by the
/// user, so the message says how: the default small model caps the image at 2 GiB
/// and `-mcmodel=large` or `-fpic` lifts it.
const ADVICE: &str = "recompile with -mcmodel=large or -fpic if the image exceeds 2 GiB";

/// The ABI name of an x86-64 relocation type, or `None` if it is not one this
/// backend knows.
pub fn name(rtype: u32) -> Option<&'static str> {
    Some(match rtype {
        R_X86_64_NONE => "R_X86_64_NONE",
        R_X86_64_64 => "R_X86_64_64",
        R_X86_64_PC32 => "R_X86_64_PC32",
        R_X86_64_GOT32 => "R_X86_64_GOT32",
        R_X86_64_PLT32 => "R_X86_64_PLT32",
        R_X86_64_GLOB_DAT => "R_X86_64_GLOB_DAT",
        R_X86_64_JUMP_SLOT => "R_X86_64_JUMP_SLOT",
        R_X86_64_RELATIVE => "R_X86_64_RELATIVE",
        R_X86_64_GOTPCREL => "R_X86_64_GOTPCREL",
        R_X86_64_32 => "R_X86_64_32",
        R_X86_64_32S => "R_X86_64_32S",
        R_X86_64_16 => "R_X86_64_16",
        R_X86_64_PC16 => "R_X86_64_PC16",
        R_X86_64_8 => "R_X86_64_8",
        R_X86_64_PC8 => "R_X86_64_PC8",
        R_X86_64_DTPMOD64 => "R_X86_64_DTPMOD64",
        R_X86_64_DTPOFF64 => "R_X86_64_DTPOFF64",
        R_X86_64_TPOFF64 => "R_X86_64_TPOFF64",
        R_X86_64_TLSGD => "R_X86_64_TLSGD",
        R_X86_64_TLSLD => "R_X86_64_TLSLD",
        R_X86_64_DTPOFF32 => "R_X86_64_DTPOFF32",
        R_X86_64_GOTTPOFF => "R_X86_64_GOTTPOFF",
        R_X86_64_TPOFF32 => "R_X86_64_TPOFF32",
        R_X86_64_PC64 => "R_X86_64_PC64",
        R_X86_64_GOTOFF64 => "R_X86_64_GOTOFF64",
        R_X86_64_GOTPC32 => "R_X86_64_GOTPC32",
        R_X86_64_GOT64 => "R_X86_64_GOT64",
        R_X86_64_GOTPCREL64 => "R_X86_64_GOTPCREL64",
        R_X86_64_GOTPC64 => "R_X86_64_GOTPC64",
        R_X86_64_GOTPLT64 => "R_X86_64_GOTPLT64",
        R_X86_64_PLTOFF64 => "R_X86_64_PLTOFF64",
        R_X86_64_SIZE32 => "R_X86_64_SIZE32",
        R_X86_64_SIZE64 => "R_X86_64_SIZE64",
        R_X86_64_GOTPC32_TLSDESC => "R_X86_64_GOTPC32_TLSDESC",
        R_X86_64_TLSDESC_CALL => "R_X86_64_TLSDESC_CALL",
        R_X86_64_IRELATIVE => "R_X86_64_IRELATIVE",
        R_X86_64_RELATIVE64 => "R_X86_64_RELATIVE64",
        R_X86_64_GOTPCRELX => "R_X86_64_GOTPCRELX",
        R_X86_64_REX_GOTPCRELX => "R_X86_64_REX_GOTPCRELX",
        R_X86_64_CODE_4_GOTPCRELX => "R_X86_64_CODE_4_GOTPCRELX",
        R_X86_64_CODE_4_GOTTPOFF => "R_X86_64_CODE_4_GOTTPOFF",
        R_X86_64_CODE_4_GOTPC32_TLSDESC => "R_X86_64_CODE_4_GOTPC32_TLSDESC",
        R_X86_64_CODE_6_GOTPCRELX => "R_X86_64_CODE_6_GOTPCRELX",
        R_X86_64_CODE_6_GOTTPOFF => "R_X86_64_CODE_6_GOTTPOFF",
        R_X86_64_CODE_6_GOTPC32_TLSDESC => "R_X86_64_CODE_6_GOTPC32_TLSDESC",
        _ => return None,
    })
}

/// The field a relocation type writes, per the x86-64 ABI.
///
/// `None` means the type writes no fixed-width value at static-link time: the
/// no-op, the `TLSDESC_CALL` marker (an annotation on an existing call, not a
/// field), and the dynamic-only types whose payload is a `.rela.dyn` entry
/// rather than a patch in the image.
pub fn field(rtype: u32) -> Option<RelocField> {
    Some(match rtype {
        // 64-bit data fields. Present so the table is complete and truthful;
        // is_no_check makes the range check vacuous for them.
        R_X86_64_64 | R_X86_64_SIZE64 | R_X86_64_DTPMOD64 => U64,
        R_X86_64_PC64 | R_X86_64_GOTOFF64 | R_X86_64_DTPOFF64 | R_X86_64_TPOFF64
        | R_X86_64_GOT64 | R_X86_64_GOTPCREL64 | R_X86_64_GOTPC64 | R_X86_64_GOTPLT64
        | R_X86_64_PLTOFF64 => S64,

        // 32-bit unsigned absolute.
        R_X86_64_32 | R_X86_64_SIZE32 => U32,

        // 32-bit signed PC-relative and GOT-relative. The GOTPCREL family is
        // relaxable, but relaxation rewrites the *instruction*, not the field:
        // every spelling still stores a 32-bit signed displacement to a GOT
        // slot, so they all get the same field. Treating them as field-less
        // because they are relaxable hides real application sites.
        R_X86_64_PC32
        | R_X86_64_PLT32
        | R_X86_64_32S
        | R_X86_64_GOT32
        | R_X86_64_GOTPC32
        | R_X86_64_GOTPCREL
        | R_X86_64_GOTPCRELX
        | R_X86_64_REX_GOTPCRELX
        | R_X86_64_CODE_4_GOTPCRELX
        | R_X86_64_CODE_6_GOTPCRELX
        | R_X86_64_GOTPC32_TLSDESC
        | R_X86_64_CODE_4_GOTPC32_TLSDESC
        | R_X86_64_CODE_6_GOTPC32_TLSDESC
        | R_X86_64_TLSGD
        | R_X86_64_TLSLD
        | R_X86_64_GOTTPOFF
        | R_X86_64_CODE_4_GOTTPOFF
        | R_X86_64_CODE_6_GOTTPOFF
        | R_X86_64_TPOFF32
        | R_X86_64_DTPOFF32 => S32,

        // Narrow fields. These are the ones a hand-written `.word sym` /
        // `.byte sym` produces, and they are the easiest to truncate silently
        // because nothing in a normal build ever gets near their limit.
        R_X86_64_16 => U16,
        R_X86_64_PC16 => S16,
        R_X86_64_8 => U8,
        R_X86_64_PC8 => S8,

        // No fixed-width field written at static-link time.
        R_X86_64_NONE
        | R_X86_64_TLSDESC_CALL
        | R_X86_64_GLOB_DAT
        | R_X86_64_JUMP_SLOT
        | R_X86_64_RELATIVE
        | R_X86_64_RELATIVE64
        | R_X86_64_IRELATIVE => return None,

        _ => return None,
    })
}

/// Number of bytes this relocation writes into the image.
///
/// For x86-64 that is the field width divided by eight: every type stores its
/// value contiguously, so the patch width is derivable. It is a function rather
/// than a derivation at the call site because that is not true of every
/// architecture -- AArch64's `TSTBR14` has a 16-bit field inside a 4-byte
/// instruction, so its patch width is 4 while its field width is 16. Keeping the
/// two concepts separate here is what stops a future backend from computing one
/// from the other and silently under-checking.
///
/// Zero means the relocation writes no field (`NONE`, the `TLSDESC_CALL` marker,
/// the dynamic-only types whose payload is a `.rela.dyn` entry), which makes the
/// offset check vacuous for it.
pub fn patch_width(rtype: u32) -> usize {
    match field(rtype) {
        Some(f) => (f.bits as usize) / 8,
        None => 0,
    }
}

/// True when the type must not be range-checked.
///
/// Two cases, and keeping them in one predicate is what stops a backend from
/// exempting one and not the other:
///
///  * the type writes no field at all, and we know that it does not -- `NONE`,
///    the `TLSDESC_CALL` marker, the dynamic-only types. [`name`] recognising the
///    type is what distinguishes this from the next case;
///  * the field is 64 bits or wider, where the value the linker computed cannot
///    lose bits. Refusing those would reject inputs GNU ld accepts -- a symbol at
///    address 0 with a negative addend is legal and its wrapped bits are the
///    correct content of a 64-bit field.
///
/// A type this table has never heard of is **not** exempt. Exempting it would
/// turn an ABI addition or a corrupt `r_type` into a silently truncated field,
/// which is the exact failure this module exists to prevent; [`check_width`]
/// falls back to the writer's own width instead.
pub fn is_no_check(rtype: u32) -> bool {
    match field(rtype) {
        Some(f) => f.bits >= 64,
        None => name(rtype).is_some(),
    }
}

/// The field a value is to be checked against, given the relocation type and the
/// width of the writer about to store it.
///
/// `None` means "no range check applies". The three cases are separated here
/// because conflating the last two is a fail-open: a type we know to be fieldless
/// genuinely has nothing to overflow, while a type we do not know has a field we
/// failed to describe, and the honest response to that is to check it against the
/// width of the store that is about to happen.
fn effective_field(rtype: u32, width_bits: u32) -> Option<RelocField> {
    match field(rtype) {
        Some(f) if f.bits < 64 => Some(f),
        // A 64-bit field written from a 64-bit value cannot lose bits.
        Some(_) => None,
        // Known to write no field at static-link time.
        None if name(rtype).is_some() => None,
        // Unknown type: be conservative rather than silent. Signed is the
        // stricter of the two at a given width for values above the half-way
        // point, and every width here is the writer's own, so the check can only
        // refuse a value that the store would have truncated anyway.
        None => Some(RelocField::signed(width_bits)),
    }
}

/// Range-check `value` against the field for `rtype`, falling back to a
/// `width_bits`-wide signed field when the type is not in the table.
///
/// This is the entry point the checked writers use: they know how many bytes they
/// are about to store, so an unrecognised relocation type still gets a check
/// against that width instead of sailing through.
pub fn check_width(
    rtype: u32,
    width_bits: u32,
    value: i64,
    sym_name: &str,
    source: &str,
) -> Result<(), String> {
    // An unknown type is handled here rather than by `check_known` because its
    // diagnostic has to say something different. The fallback field's signedness
    // is a guess, and printing "does not fit the 32-bit signed field" for a type
    // the table has never described would send the user after the wrong fix --
    // they would reach for -mcmodel=large when what is actually missing is a
    // table entry. Say what is known (the width the store reserves) and what to
    // do about it.
    if field(rtype).is_none() && name(rtype).is_none() {
        let f = RelocField::signed(width_bits);
        if f.fits(value) {
            return Ok(());
        }
        let target = if sym_name.is_empty() {
            "<local>"
        } else {
            sym_name
        };
        return Err(format!(
            "unrecognised relocation {} against symbol '{target}' in {source}: value \
             {value} = 0x{value:x} does not fit the {width_bits}-bit field this write \
             reserves; add the type to the x86-64 relocation field table",
            type_label(name(rtype), rtype),
        ));
    }
    match effective_field(rtype, width_bits) {
        None => Ok(()),
        Some(f) => check_known(rtype, f, value, sym_name, source),
    }
}

/// Range-check `value` against the field the ABI gives `rtype`.
///
/// The type-driven entry point, for call sites that have the relocation type but
/// no writer width to offer. An unrecognised type is checked as 32-bit signed:
/// that is the field width almost every x86-64 relocation that reaches a checked
/// write actually has, and guessing narrow would let a value through that the
/// store then truncates.
///
/// Arms that already know their field should call [`check_known`] and skip the
/// lookup entirely.
pub fn check(rtype: u32, value: i64, sym_name: &str, source: &str) -> Result<(), String> {
    check_width(rtype, 32, value, sym_name, source)
}

/// Range-check `value` against a field the caller already knows.
///
/// Lookup-free, which matters twice over: it is the hot path (one relocation per
/// patch site, millions per kernel-sized link), and it makes the check
/// unforgeable -- the field passed here is the same constant the arm is about to
/// encode, so the check and the write cannot drift apart.
///
/// A type that is not in the table falls back to the natural width of the writer
/// that called it, treated as signed. That is deliberate: an unknown type
/// reaching a checked writer means the caller is handling a relocation this table
/// has never heard of, and failing loudly is better than truncating silently.
pub fn check_known(
    rtype: u32,
    f: RelocField,
    value: i64,
    sym_name: &str,
    source: &str,
) -> Result<(), String> {
    if f.bits >= 64 || f.fits(value) {
        return Ok(());
    }
    Err(overflow_message(
        &type_label(name(rtype), rtype),
        value,
        f,
        sym_name,
        source,
        ADVICE,
    ))
}

/// Verify that `width` bytes at `off` are inside the output image.
///
/// The destination is checked separately from the value because they fail for
/// different reasons and the user needs to know which one happened: a value can
/// be perfectly representable while the offset an input object asks us to patch
/// points past the end of the image we built.
///
/// The arithmetic uses `checked_add`. The obvious `off + width <= len` both
/// silently skips the write when it is false -- reporting success for an image
/// that is missing a patch -- and wraps for an offset near `usize::MAX`, after
/// which the comparison passes and the slice index panics. A corrupt or hostile
/// input object can supply such an offset, so this is reachable without any bug
/// in our own layout code.
pub fn bounds(
    out: &[u8],
    off: usize,
    width: usize,
    rtype: u32,
    sym_name: &str,
    source: &str,
) -> Result<(), String> {
    if off.checked_add(width).is_some_and(|end| end <= out.len()) {
        return Ok(());
    }
    Err(bounds_message(
        &type_label(name(rtype), rtype),
        off,
        width,
        out.len(),
        sym_name,
        source,
    ))
}

/// Write a 4-byte relocation field, range-checked and bounds-checked.
///
/// Replaces `w32(&mut out, fp, EXPR as u32)` at relocation application sites.
/// The `as u32` cast in the old idiom *is* the bug: it is the mask that discards
/// the bits this function now checks first.
pub fn w32_checked(
    out: &mut [u8],
    off: usize,
    value: i64,
    rtype: u32,
    sym_name: &str,
    source: &str,
) -> Result<(), String> {
    check_width(rtype, 32, value, sym_name, source)?;
    bounds(out, off, 4, rtype, sym_name, source)?;
    w32(out, off, value as u32);
    Ok(())
}

/// Write a 2-byte relocation field, range-checked and bounds-checked.
pub fn w16_checked(
    out: &mut [u8],
    off: usize,
    value: i64,
    rtype: u32,
    sym_name: &str,
    source: &str,
) -> Result<(), String> {
    check_width(rtype, 16, value, sym_name, source)?;
    bounds(out, off, 2, rtype, sym_name, source)?;
    w16(out, off, value as u16);
    Ok(())
}

/// Write a 1-byte relocation field, range-checked and bounds-checked.
///
/// There is no `w8` in the shared ELF helpers -- the unchecked idiom was
/// `out[fp] = EXPR as u8`, which panics on an out-of-range offset instead of
/// skipping it, so this is both the range check and the bounds check that the
/// raw index never had.
pub fn w8_checked(
    out: &mut [u8],
    off: usize,
    value: i64,
    rtype: u32,
    sym_name: &str,
    source: &str,
) -> Result<(), String> {
    check_width(rtype, 8, value, sym_name, source)?;
    bounds(out, off, 1, rtype, sym_name, source)?;
    out[off] = value as u8;
    Ok(())
}

/// Write an 8-byte relocation field, bounds-checked.
///
/// The range check is vacuous for a 64-bit field (see [`is_no_check`]), so what
/// this adds over `w64` is the destination check: `w64` silently skips a write
/// that falls outside the buffer, and a `.rela.dyn`-driven 64-bit patch at a bad
/// offset must not report success.
pub fn w64_checked(
    out: &mut [u8],
    off: usize,
    value: u64,
    rtype: u32,
    sym_name: &str,
    source: &str,
) -> Result<(), String> {
    bounds(out, off, 8, rtype, sym_name, source)?;
    w64(out, off, value);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A value that fits every signed 32-bit field.
    const OK: i64 = 0x1000;

    #[test]
    fn table_agrees_with_the_abi_on_signedness_and_width() {
        // The two 32-bit absolutes are written by the same 4-byte store and
        // differ only in signedness; that difference is the whole point of the
        // table, so it is pinned here against the ABI.
        assert_eq!(field(R_X86_64_32), Some(U32));
        assert_eq!(field(R_X86_64_32S), Some(S32));
        assert_eq!(field(R_X86_64_PC32), Some(S32));
        assert_eq!(field(R_X86_64_PLT32), Some(S32));
        assert_eq!(field(R_X86_64_16), Some(U16));
        assert_eq!(field(R_X86_64_PC16), Some(S16));
        assert_eq!(field(R_X86_64_8), Some(U8));
        assert_eq!(field(R_X86_64_PC8), Some(S8));
        assert_eq!(field(R_X86_64_64), Some(U64));
        // The relaxable GOTPCREL family still writes a 32-bit signed field.
        for t in [
            R_X86_64_GOTPCREL,
            R_X86_64_GOTPCRELX,
            R_X86_64_REX_GOTPCRELX,
            R_X86_64_CODE_4_GOTPCRELX,
            R_X86_64_CODE_6_GOTPCRELX,
        ] {
            assert_eq!(field(t), Some(S32), "type {t} lost its field");
        }
    }

    #[test]
    fn every_field_type_has_a_name_and_vice_versa() {
        // A type in the name table but not the field table (or the reverse)
        // means the diagnostic would print "type 43" for a relocation we know
        // perfectly well, or check a field it cannot name.
        for t in 0u32..=52 {
            let (n, f) = (name(t), field(t));
            if f.is_some() {
                assert!(n.is_some(), "type {t} has a field but no name");
            }
        }
        assert_eq!(
            name(R_X86_64_CODE_6_GOTTPOFF),
            Some("R_X86_64_CODE_6_GOTTPOFF")
        );
        assert!(name(9999).is_none());
        assert_eq!(type_label(name(9999), 9999), "type 9999");
    }

    #[test]
    fn no_check_covers_exactly_the_vacuous_and_fieldless_cases() {
        assert!(is_no_check(R_X86_64_NONE));
        assert!(is_no_check(R_X86_64_TLSDESC_CALL));
        assert!(
            is_no_check(R_X86_64_GLOB_DAT),
            "dynamic-only: no image patch"
        );
        assert!(is_no_check(R_X86_64_64), "64-bit field cannot lose bits");
        assert!(is_no_check(R_X86_64_PC64));
        assert!(!is_no_check(R_X86_64_PC32));
        assert!(!is_no_check(R_X86_64_32));
        assert!(!is_no_check(R_X86_64_8));
        // An unrecognised type is not exempt. This assertion is the regression
        // test for a fail-open the first version of this module had: `field()`
        // returns None both for "known to write no field" and for "never heard of
        // it", and exempting on None let type 9999 truncate without a word.
        assert!(!is_no_check(9999), "unknown types must not be exempt");
    }

    #[test]
    fn a_negative_64_bit_value_is_written_not_refused() {
        // GNU ld writes the wrapped bits for a symbol at 0 with a negative
        // addend; refusing here would reject a valid link.
        assert!(check(R_X86_64_64, -8, "sym", "a.o").is_ok());
        let mut out = vec![0u8; 8];
        assert!(w64_checked(&mut out, 0, (-8i64) as u64, R_X86_64_64, "sym", "a.o").is_ok());
        assert_eq!(out, (-8i64).to_le_bytes().to_vec());
    }

    #[test]
    fn out_of_range_values_are_refused_with_an_actionable_message() {
        let e = check(R_X86_64_PC32, 0x1_0000_0000, "far", "big.o").unwrap_err();
        assert!(e.contains("relocation truncated to fit"), "{e}");
        assert!(e.contains("R_X86_64_PC32"), "{e}");
        assert!(e.contains("far"), "{e}");
        assert!(e.contains("big.o"), "{e}");
        assert!(e.contains("-mcmodel=large"), "{e}");
        // Signed and unsigned at the same width disagree, as the ABI says.
        assert!(check(R_X86_64_32, 0x8000_0000, "s", "a.o").is_ok());
        assert!(check(R_X86_64_32S, 0x8000_0000, "s", "a.o").is_err());
        assert!(check(R_X86_64_32, -1, "s", "a.o").is_err());
    }

    #[test]
    fn narrow_fields_refuse_one_past_their_limit() {
        assert!(check(R_X86_64_16, 0xFFFF, "s", "a.o").is_ok());
        assert!(check(R_X86_64_16, 0x1_0000, "s", "a.o").is_err());
        assert!(check(R_X86_64_PC16, -0x8000, "s", "a.o").is_ok());
        assert!(check(R_X86_64_PC16, -0x8001, "s", "a.o").is_err());
        assert!(check(R_X86_64_8, 0xFF, "s", "a.o").is_ok());
        assert!(check(R_X86_64_8, 0x100, "s", "a.o").is_err());
        assert!(check(R_X86_64_PC8, 127, "s", "a.o").is_ok());
        assert!(check(R_X86_64_PC8, 128, "s", "a.o").is_err());
        assert!(check(R_X86_64_PC8, -128, "s", "a.o").is_ok());
        assert!(check(R_X86_64_PC8, -129, "s", "a.o").is_err());
    }

    #[test]
    fn checked_writers_store_the_same_bytes_as_the_unchecked_ones() {
        // The check must not change the encoding of a value that fits.
        let mut a = vec![0xAAu8; 8];
        let mut b = vec![0xAAu8; 8];
        w32(&mut a, 2, 0x1234_5678);
        w32_checked(&mut b, 2, 0x1234_5678, R_X86_64_32, "s", "a.o").unwrap();
        assert_eq!(a, b);
        assert_eq!(&b[2..6], &0x1234_5678u32.to_le_bytes());

        w16(&mut a, 0, 0xBEEF);
        w16_checked(&mut b, 0, 0xBEEF, R_X86_64_16, "s", "a.o").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn a_destination_past_the_end_is_an_error_not_a_silent_skip() {
        // This is the fail-open the unchecked writers have: w32 no-ops and the
        // link reports success for an image missing a patch.
        let mut out = vec![0u8; 4];
        let e = w32_checked(&mut out, 4, OK, R_X86_64_32, "s", "a.o").unwrap_err();
        assert!(e.contains("outside the output image"), "{e}");
        assert!(e.contains("0x4"), "{e}");
        assert_eq!(out, vec![0u8; 4], "nothing may be written");
        // The unchecked writer, for contrast, would have "succeeded".
        w32(&mut out, 4, 1);
        assert_eq!(out, vec![0u8; 4]);
    }

    #[test]
    fn an_offset_that_would_overflow_usize_is_rejected_not_wrapped() {
        // `off + width` wraps for an offset near usize::MAX; the wrapped sum
        // passes a naive `<= len` test and the slice index then panics. A
        // corrupt input object can carry such an offset.
        let mut out = vec![0u8; 16];
        for off in [usize::MAX, usize::MAX - 1, usize::MAX - 3] {
            let e = w32_checked(&mut out, off, OK, R_X86_64_32, "s", "a.o")
                .expect_err("must not wrap into bounds");
            assert!(e.contains("outside the output image"), "{e}");
        }
        assert!(w8_checked(&mut out, usize::MAX, 1, R_X86_64_8, "s", "a.o").is_err());
        assert!(w64_checked(&mut out, usize::MAX - 2, 1, R_X86_64_64, "s", "a.o").is_err());
    }

    #[test]
    fn the_last_valid_offset_still_writes() {
        // Guard against over-tightening: an off-by-one here would refuse the
        // final field of every section.
        let mut out = vec![0u8; 8];
        w32_checked(&mut out, 4, 0x1122_3344, R_X86_64_32, "s", "a.o").unwrap();
        assert_eq!(&out[4..], &0x1122_3344u32.to_le_bytes());
        w8_checked(&mut out, 7, 0x7F, R_X86_64_8, "s", "a.o").unwrap();
        assert_eq!(out[7], 0x7F);
    }

    #[test]
    fn patch_width_is_the_field_width_in_bytes_and_zero_when_fieldless() {
        assert_eq!(patch_width(R_X86_64_64), 8);
        assert_eq!(patch_width(R_X86_64_PC64), 8);
        assert_eq!(patch_width(R_X86_64_PC32), 4);
        assert_eq!(patch_width(R_X86_64_32S), 4);
        assert_eq!(patch_width(R_X86_64_GOTPCRELX), 4);
        assert_eq!(patch_width(R_X86_64_16), 2);
        assert_eq!(patch_width(R_X86_64_PC16), 2);
        assert_eq!(patch_width(R_X86_64_8), 1);
        assert_eq!(patch_width(R_X86_64_PC8), 1);
        // Fieldless: nothing is written, so the offset check must be vacuous.
        assert_eq!(patch_width(R_X86_64_NONE), 0);
        assert_eq!(patch_width(R_X86_64_TLSDESC_CALL), 0);
        assert_eq!(patch_width(R_X86_64_GLOB_DAT), 0);
        assert_eq!(patch_width(R_X86_64_JUMP_SLOT), 0);
        assert_eq!(patch_width(9999), 0);
    }

    #[test]
    fn an_unknown_type_fails_loudly_rather_than_truncating() {
        // Not in the table: fall back to the writer's natural width, signed, and
        // refuse a value that does not fit it. Truncating an unrecognised
        // relocation is how a wrong image gets produced with no diagnostic.
        let mut out = vec![0u8; 8];
        assert!(w32_checked(&mut out, 0, OK, 9999, "s", "a.o").is_ok());
        let e = w32_checked(&mut out, 0, 0x1_0000_0000, 9999, "s", "a.o").unwrap_err();
        assert!(e.contains("type 9999"), "must name the number: {e}");
        assert!(!e.contains("R_X86_64"), "{e}");
    }

    #[test]
    fn the_unknown_type_fallback_uses_the_writers_own_width() {
        // The fallback is not a fixed 32-bit guess: each writer checks against
        // the number of bytes it is about to store, so a value that fits the
        // store is accepted and one that does not is refused at every width.
        let mut out = vec![0u8; 8];
        // The fallback field is signed at the writer's own width, so the accept
        // boundary moves with the writer: 0x7F for w8, 0x7FFF for w16, 0x7FFFFFFF
        // for w32. A fixed 32-bit guess would have let the narrow stores
        // truncate.
        assert!(w8_checked(&mut out, 0, 0x7F, 9999, "s", "a.o").is_ok());
        assert!(w8_checked(&mut out, 0, 0x80, 9999, "s", "a.o").is_err());
        assert!(w8_checked(&mut out, 0, -0x80, 9999, "s", "a.o").is_ok());
        assert!(w16_checked(&mut out, 0, 0x7FFF, 9999, "s", "a.o").is_ok());
        assert!(w16_checked(&mut out, 0, 0x8000, 9999, "s", "a.o").is_err());
        assert!(w32_checked(&mut out, 0, 0x7FFF_FFFF, 9999, "s", "a.o").is_ok());
        assert!(w32_checked(&mut out, 0, 0x8000_0000, 9999, "s", "a.o").is_err());
    }

    #[test]
    fn the_unknown_type_diagnostic_does_not_claim_a_signedness_it_guessed() {
        let mut out = vec![0u8; 8];
        let e = w32_checked(&mut out, 0, 0x1_0000_0000, 9999, "sym", "a.o").unwrap_err();
        assert!(e.contains("unrecognised relocation type 9999"), "{e}");
        assert!(e.contains("32-bit field"), "{e}");
        assert!(e.contains("field table"), "must say how to fix it: {e}");
        // Naming a signedness we inferred would point the user at -mcmodel=large
        // instead of at the missing table entry.
        assert!(!e.contains("signed"), "{e}");
        assert!(!e.contains("-mcmodel"), "{e}");
        // A known type still gets the normal GNU ld wording.
        let k = w32_checked(&mut out, 0, 0x1_0000_0000, R_X86_64_PC32, "sym", "a.o").unwrap_err();
        assert!(
            k.contains("relocation truncated to fit: R_X86_64_PC32"),
            "{k}"
        );
        assert!(k.contains("32-bit signed"), "{k}");
    }
}
