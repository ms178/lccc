//! The one invariant every relocation shares, stated once instead of once per
//! backend.
//!
//! A relocation computes a value -- an address, a displacement, a symbol size --
//! and stores it in a field of fixed width inside an instruction or a data slot.
//! When the value does not fit, there are exactly two acceptable outcomes: a
//! diagnostic, or a relaxation that emits a longer instruction sequence. The
//! third outcome is what this module exists to prevent: masking the value to the
//! field width and carrying on. That produces an image which links cleanly,
//! exits 0, and then reads or branches to the wrong address -- a miscompile with
//! no diagnostic anywhere, which is the worst failure a linker can have because
//! the user's next observation is a wrong result rather than an error.
//!
//! Masking is not a hypothetical. On x86-64 a `CALL` writes a 32-bit signed
//! displacement and a target 200 MiB *forward* becomes, after the mask and the
//! CPU's sign extension of the field, a jump 56 MiB *backward*: the bits are
//! preserved, the meaning is inverted. The check therefore has to look at the
//! value before it is shifted or masked, never at the masked result.
//!
//! # What is deliberately *not* checked
//!
//! A field of 64 bits or wider cannot lose information, because the value the
//! linker computes is itself 64 bits wide: every bit of it lands in the field.
//! [`RelocField::fits`] is defined to be about lost bits, so it returns `true`
//! for those fields. Checking them anyway would only invent refusals that GNU ld
//! does not make (a symbol at 0 with a negative addend is legal and GNU ld
//! writes the wrapped bits), and a linker that refuses valid inputs is as broken
//! as one that accepts invalid ones.
//!
//! # Cost
//!
//! [`RelocField::fits`] is two comparisons against immediates once the field is
//! a `const`, which every call site in a relocation table is: the bounds fold at
//! compile time and the branch is never taken on a well-formed link. There is no
//! table lookup, no allocation and no formatting on the success path -- the
//! `format!` only runs when the link is about to fail anyway.

/// A fixed-width relocation field: how many bits the instruction or data slot
/// carries, and whether those bits are two's complement.
///
/// `Copy` and `const`-constructible so a backend's relocation table can be a
/// `match` returning literals, which the compiler folds into the comparison
/// sequence at each call site.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RelocField {
    /// Number of bits the field carries.
    pub bits: u32,
    /// Whether those bits are two's complement (`true`) or plain magnitude
    /// (`false`). Getting this wrong is not cosmetic: `R_X86_64_32` and
    /// `R_X86_64_32S` are both written by the same 4-byte store, and only the
    /// signedness of the field tells them apart.
    pub signed: bool,
}

impl RelocField {
    /// An unsigned field of `bits` bits.
    pub const fn unsigned(bits: u32) -> Self {
        Self {
            bits,
            signed: false,
        }
    }

    /// A two's-complement field of `bits` bits.
    pub const fn signed(bits: u32) -> Self {
        Self { bits, signed: true }
    }

    /// The inclusive range this field can represent.
    ///
    /// `i128` rather than `i64` because an unsigned 64-bit field's maximum
    /// (`2^64 - 1`) is not representable in `i64`; widening the return type is
    /// cheaper than special-casing every caller.
    pub fn bounds(self) -> (i128, i128) {
        if self.bits == 0 {
            // A zero-width field is a table bug, not a value the linker can
            // produce. Returning an empty range makes every value fail with a
            // diagnostic that names the field, which is a louder and more
            // debuggable outcome than the shift underflow below.
            return (1, 0);
        }
        if self.signed {
            let half = 1i128 << (self.bits - 1);
            (-half, half - 1)
        } else {
            (0, (1i128 << self.bits) - 1)
        }
    }

    /// Does `value` fit this field, i.e. would storing it lose any bits?
    pub fn fits(self, value: i64) -> bool {
        // Fields of 64 bits or more cannot lose information: see the module
        // docs. This is also the fast path for the widest data relocations.
        if self.bits >= 64 {
            return true;
        }
        let (lo, hi) = self.bounds();
        // The widening cast is free: `value as i128` is a sign extension, and
        // with `bits < 64` both bounds fit in i64, so the comparison could be
        // narrowed further -- but i128 keeps a single code path for signed and
        // unsigned fields and the extra register pair costs nothing here.
        (value as i128) >= lo && (value as i128) <= hi
    }
}

/// The name to show for a relocation type: its ABI name when the backend knows
/// it, otherwise its number.
///
/// A number is actionable -- `readelf -r` prints the same number, so the user can
/// look the type up in one step -- where a placeholder such as `<unknown>` sends
/// them nowhere. This exists because a diagnostic refactor once replaced the
/// number with a placeholder and the resulting message could not be acted on.
pub fn type_label(name: Option<&str>, rtype: u32) -> String {
    match name {
        Some(name) if !name.is_empty() => name.to_string(),
        _ => format!("type {rtype}"),
    }
}

/// The "relocation truncated to fit" diagnostic, in GNU ld's wording plus the
/// field's own range.
///
/// GNU ld stops at naming the type, the symbol and the object; adding the range
/// costs nothing (this path only runs when the link is failing) and turns "out of
/// range" into "here is how far out", which is what decides whether the fix is
/// `-mcmodel=large`, a linker script change or a genuine bug.
///
/// `advice` is architecture-specific and appended verbatim; pass `""` for none.
pub fn overflow_message(
    rtype: &str,
    value: i64,
    field: RelocField,
    sym_name: &str,
    source: &str,
    advice: &str,
) -> String {
    let (lo, hi) = field.bounds();
    let target = if sym_name.is_empty() {
        "<local>"
    } else {
        sym_name
    };
    let kind = if field.signed { "signed" } else { "unsigned" };
    // Line continuations, not wrapped literals: a `format!` string that is
    // wrapped without `\` emits the indentation as literal runs of spaces, and
    // the message ends up with "value  0x10" in it.
    let mut msg = format!(
        "relocation truncated to fit: {rtype} against symbol '{target}' in {source} \
         (value {value} = 0x{value:x} does not fit the {}-bit {kind} field [{lo}, {hi}])",
        field.bits,
    );
    if !advice.is_empty() {
        msg.push_str("; ");
        msg.push_str(advice);
    }
    msg
}

/// The diagnostic for a relocation whose *destination* lies outside the output
/// image.
///
/// This is a separate failure from an out-of-range value and gets its own
/// message: the value can be perfectly representable while the offset the input
/// object asks us to patch points past the end of what we built. Every backend
/// used to guard these writes with `if off + width <= out.len() { write } else {
/// carry on }`, which reports success for an image that is missing a patch. Both
/// the silent skip and the `off + width` overflow it hides (a hostile offset near
/// `usize::MAX` wraps, the comparison then passes, and the write either panics or
/// lands nowhere) are what this diagnostic replaces.
pub fn bounds_message(
    rtype: &str,
    off: usize,
    width: usize,
    len: usize,
    sym_name: &str,
    source: &str,
) -> String {
    let target = if sym_name.is_empty() {
        "<local>"
    } else {
        sym_name
    };
    format!(
        "relocation {rtype} against symbol '{target}' in {source} points outside the \
         output image: it would write {width} byte(s) at offset 0x{off:x}, but the \
         image is 0x{len:x} byte(s) long"
    )
}

/// Validate a relocation's offset against the section it patches, given the
/// number of bytes it is about to write.
///
/// This is the width-aware half of GNU ld's rule, and the boundary was measured
/// rather than assumed: with a 9-byte `.text` and a 4-byte `R_X86_64_PC32`, bfd
/// accepts `r_offset = 5` (5 + 4 == 9) and refuses `r_offset = 6` with
/// `bfd_reloc_outofrange` ("error 4"). So the field may end exactly at the
/// section boundary but not cross it.
///
/// Why this is not in the parser, which already rejects `r_offset >= size`: the
/// width of the field a relocation writes is an architecture-specific fact, and
/// the parser is shared by four of them. Splitting the rule this way keeps the
/// generic invariant (a relocation points inside its section) in one place that
/// every backend inherits, and the precise one where the widths are known.
///
/// Without this half, a relocation near the end of a section writes into whatever
/// the layout put next to it -- the bytes belong to a different input section, so
/// the image is corrupt while every bounds check against the *image* passes.
///
/// `width` is 0 for a relocation that writes no field, which makes the check
/// vacuous: the parser's bound already applies.
pub fn offset_in_section(
    rtype: u32,
    type_name: Option<&str>,
    offset: u64,
    width: usize,
    sec_size: u64,
    sec_name: &str,
    source: &str,
) -> Result<(), String> {
    // checked_add because `offset` is input-controlled and `width` is added to
    // it: the parser guarantees `offset < sec_size`, but this function is also
    // correct on its own, without relying on that having run.
    let crosses = match offset.checked_add(width as u64) {
        Some(end) => end > sec_size,
        None => true,
    };
    if !crosses {
        return Ok(());
    }
    Err(format!(
        "{source}: relocation {} in section '{sec_name}' at offset 0x{offset:x} would \
         write {width} byte(s) across the end of that section (size 0x{sec_size:x})",
        type_label(type_name, rtype),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const U32F: RelocField = RelocField::unsigned(32);
    const S32F: RelocField = RelocField::signed(32);

    #[test]
    fn signed_and_unsigned_32_differ_exactly_where_the_abi_says() {
        // R_X86_64_32 is unsigned, R_X86_64_32S is signed; conflating them
        // either refuses a valid high-half address or accepts a negative one.
        assert!(U32F.fits(0));
        assert!(U32F.fits(u32::MAX as i64));
        assert!(
            !U32F.fits(-1),
            "unsigned field must reject a negative value"
        );
        assert!(!U32F.fits(u32::MAX as i64 + 1));

        assert!(S32F.fits(i32::MIN as i64));
        assert!(S32F.fits(i32::MAX as i64));
        assert!(!S32F.fits(i32::MAX as i64 + 1));
        assert!(!S32F.fits(i32::MIN as i64 - 1));
        // The asymmetry, stated explicitly: this value is fine for _32 and
        // truncated for _32S.
        assert!(U32F.fits(0x8000_0000));
        assert!(!S32F.fits(0x8000_0000));
    }

    #[test]
    fn narrow_fields_check_both_ends() {
        for bits in [8u32, 16, 32] {
            let s = RelocField::signed(bits);
            let u = RelocField::unsigned(bits);
            let lo = -(1i64 << (bits - 1));
            let hi = (1i64 << (bits - 1)) - 1;
            // Both ends of the signed field, inclusive, and one past each end.
            assert!(s.fits(lo), "{bits}-bit signed must accept {lo}");
            assert!(s.fits(hi), "{bits}-bit signed must accept {hi}");
            assert!(!s.fits(lo - 1), "{bits}-bit signed must reject {}", lo - 1);
            assert!(!s.fits(hi + 1), "{bits}-bit signed must reject {}", hi + 1);
            // The unsigned field shares the top end but not the bottom.
            assert!(u.fits((1i64 << bits) - 1));
            assert!(!u.fits(1i64 << bits));
            assert!(!u.fits(-1));
        }
    }

    #[test]
    fn wide_fields_are_vacuous_by_design() {
        // A 64-bit field written from an i64 loses no bits, so the check is
        // defined to pass -- including for a negative value, which GNU ld writes
        // as the wrapped bits rather than refusing.
        assert!(RelocField::unsigned(64).fits(i64::MIN));
        assert!(RelocField::unsigned(64).fits(i64::MAX));
        assert!(RelocField::signed(64).fits(i64::MIN));
        assert!(RelocField::signed(64).fits(i64::MAX));
    }

    #[test]
    fn bounds_are_exact_and_do_not_overflow() {
        assert_eq!(S32F.bounds(), (i32::MIN as i128, i32::MAX as i128));
        assert_eq!(U32F.bounds(), (0, u32::MAX as i128));
        // The case that needs i128: an unsigned 64-bit maximum is not an i64.
        assert_eq!(
            RelocField::unsigned(64).bounds(),
            (0, (1i128 << 64) - 1),
            "u64 max must be representable"
        );
        assert_eq!(
            RelocField::signed(64).bounds(),
            (i64::MIN as i128, i64::MAX as i128)
        );
    }

    #[test]
    fn a_zero_width_field_rejects_everything_instead_of_underflowing() {
        // `1 << (0 - 1)` would underflow; the guard turns a table typo into a
        // diagnostic instead of a panic in a release build.
        let f = RelocField::unsigned(0);
        assert_eq!(f.bounds(), (1, 0));
        assert!(!f.fits(0));
    }

    #[test]
    fn overflow_message_carries_every_field_the_user_needs() {
        let msg = overflow_message(
            "R_X86_64_PC32",
            0x1_0000_0000,
            S32F,
            "far_target",
            "big.o",
            "recompile with -mcmodel=large",
        );
        assert!(msg.starts_with("relocation truncated to fit: "), "{msg}");
        for needle in [
            "R_X86_64_PC32",
            "far_target",
            "big.o",
            "0x100000000",
            "32-bit signed",
            "[-2147483648, 2147483647]",
            "-mcmodel=large",
        ] {
            assert!(msg.contains(needle), "missing {needle:?} in: {msg}");
        }
        // Wrapped format strings are how literal space runs get into messages.
        assert!(!msg.contains("  "), "double space in: {msg}");
    }

    #[test]
    fn overflow_message_without_advice_has_no_dangling_separator() {
        let msg = overflow_message("R_X86_64_32", 1 << 40, U32F, "", "a.o", "");
        assert!(!msg.ends_with(';'), "{msg}");
        assert!(!msg.contains("; "), "{msg}");
        // An unnamed (section-relative) symbol still has to read as a symbol.
        assert!(msg.contains("'<local>'"), "{msg}");
    }

    #[test]
    fn bounds_message_names_the_offset_and_the_image_size() {
        let msg = bounds_message("R_X86_64_PC32", 0x1000, 4, 0x800, "sym", "x.o");
        for needle in [
            "0x1000",
            "0x800",
            "4 byte(s)",
            "sym",
            "x.o",
            "R_X86_64_PC32",
        ] {
            assert!(msg.contains(needle), "missing {needle:?} in: {msg}");
        }
        assert!(!msg.contains("  "), "double space in: {msg}");
    }

    #[test]
    fn offset_in_section_matches_the_measured_bfd_boundary() {
        // .text of 9 bytes, a 4-byte field: bfd accepts 5, refuses 6.
        let (size, w) = (9u64, 4usize);
        for off in 0..=5u64 {
            assert!(
                offset_in_section(2, Some("R_X86_64_PC32"), off, w, size, ".text", "a.o").is_ok(),
                "offset {off} must be accepted (ends at {})",
                off + w as u64
            );
        }
        for off in 6..=12u64 {
            assert!(
                offset_in_section(2, Some("R_X86_64_PC32"), off, w, size, ".text", "a.o").is_err(),
                "offset {off} must be refused"
            );
        }
    }

    #[test]
    fn offset_in_section_is_vacuous_for_a_fieldless_relocation() {
        // Width 0 writes nothing, so only the parser's own bound applies.
        assert!(offset_in_section(0, Some("R_X86_64_NONE"), 8, 0, 9, ".text", "a.o").is_ok());
    }

    #[test]
    fn offset_in_section_survives_an_offset_that_would_overflow() {
        // Input-controlled offset plus a width must not wrap into acceptance.
        assert!(
            offset_in_section(2, Some("R_X86_64_PC32"), u64::MAX, 4, 9, ".text", "a.o").is_err()
        );
        assert!(offset_in_section(2, None, u64::MAX - 1, 8, 4096, ".text", "a.o").is_err());
    }

    #[test]
    fn offset_in_section_message_names_everything_needed_to_act() {
        let e = offset_in_section(2, Some("R_X86_64_PC32"), 6, 4, 9, ".text", "f.o").unwrap_err();
        for needle in ["f.o", "R_X86_64_PC32", ".text", "0x6", "4 byte(s)", "0x9"] {
            assert!(e.contains(needle), "missing {needle:?} in: {e}");
        }
        assert!(!e.contains("  "), "double space in: {e}");
        // An unknown type still gets a number the user can look up.
        let u = offset_in_section(9999, None, 6, 4, 9, ".text", "f.o").unwrap_err();
        assert!(u.contains("type 9999"), "{u}");
    }

    #[test]
    fn type_label_falls_back_to_a_number_that_readelf_also_prints() {
        assert_eq!(type_label(Some("R_X86_64_PC32"), 2), "R_X86_64_PC32");
        assert_eq!(type_label(None, 2), "type 2");
        // An empty name is the same case as no name: a placeholder string would
        // leave the user with nothing to look up.
        assert_eq!(type_label(Some(""), 37), "type 37");
    }
}
