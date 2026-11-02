//! i386 TLS access-model transitions for executables.
//!
//! An executable's own TLS block sits at a link-time-constant offset from
//! the thread pointer (`%gs:0`), and a shared library's TLS variable
//! reached from the executable has a load-time-constant one.  So every
//! dynamic access model in an executable can be rewritten into a cheaper
//! one, exactly as GNU ld's `elf_i386_tls_transition` does:
//!
//! | input model              | main-image symbol | DSO symbol          |
//! |--------------------------|-------------------|---------------------|
//! | general dynamic (GD)     | local exec (LE)   | initial exec (IE)   |
//! | local dynamic (LDM/LDO)  | local exec        | (not applicable)    |
//! | IE, `@indntpoff`         | LE                | IE (GOT slot)       |
//! | IE, `@gotntpoff`         | LE                | IE (GOT slot)       |
//! | TLS descriptors (GNU2)   | LE                | IE                  |
//!
//! Every rewrite keeps the instruction footprint byte-for-byte and writes
//! the complete new sequence, including the value, so the caller must not
//! apply the generic 32-bit patch afterwards.  A sequence whose shape is
//! not one of the ABI-sanctioned forms is an error, never a silent
//! mislink: the recognisers check every byte they rely on.
//!
//! Values: `ntpoff` is the (negative) offset of the variable from the
//! thread pointer, `S + A - end of the TLS block` (variant II TLS).
//! `gotoff` is a GOT slot's offset from `_GLOBAL_OFFSET_TABLE_`.

/// Where the `___tls_get_addr` call of a GD/LDM sequence is, and how long
/// the whole sequence is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct TlsCallSeq {
    /// First byte of the `lea`.
    pub start: usize,
    /// Total length of `lea` + call (+ trailing `nop` for the 11-byte form).
    pub len: usize,
    /// The register holding `_GLOBAL_OFFSET_TABLE_` (the lea's base/index);
    /// `None` for the non-PIC absolute `lea x@tlsgd(,%reg... )`-free form.
    pub got_reg: Option<u8>,
    /// Offset of the call's relocated 32-bit field (its PLT32 / PC32 /
    /// GOT32X relocation must be dropped once the call is rewritten away).
    pub call_field: usize,
}

const EAX: u8 = 0;
const ESP: u8 = 4;

fn byte(data: &[u8], i: usize) -> Option<u8> {
    data.get(i).copied()
}

/// Recognise the call that follows a GD/LDM `lea` ending at `lea_end`.
/// Returns (call length, offset of the call's 32-bit field).
fn tls_get_addr_call(data: &[u8], lea_end: usize) -> Option<(usize, usize)> {
    match (byte(data, lea_end)?, byte(data, lea_end + 1)?) {
        // call rel32 (`call ___tls_get_addr@PLT`)
        (0xe8, _) => Some((5, lea_end + 1)),
        // addr32 call rel32 (GNU ld's relaxation of the GOT form)
        (0x67, 0xe8) => Some((6, lea_end + 2)),
        // call *disp32(%reg)   (`call *___tls_get_addr@GOT(%reg)`, -fno-plt)
        (0xff, m) if m & 0xf8 == 0x90 && m & 7 != ESP => Some((6, lea_end + 2)),
        // call *disp32         (non-PIC -fno-plt)
        (0xff, 0x15) => Some((6, lea_end + 2)),
        _ => None,
    }
}

/// Recognise a general-dynamic sequence whose `@tlsgd` field is at `off`.
///
/// Accepted (i386 psABI, and what GNU ld transitions):
/// ```text
///   8d 04 rr  disp32   leal x@tlsgd(,%reg,1), %eax     (7)  + call (5/6)
///   8d 80+r   disp32   leal x@tlsgd(%reg), %eax        (6)  + call (6)
///   8d 80+r   disp32   leal x@tlsgd(%reg), %eax        (6)  + call rel32 (5) [+ nop]
/// ```
/// The destination must be `%eax` (the `___tls_get_addr` argument).
pub(super) fn gd_sequence(data: &[u8], off: usize) -> Option<TlsCallSeq> {
    lea_call_sequence(data, off)
}

/// Recognise a local-dynamic `leal x@tlsldm(%reg), %eax` + call sequence.
pub(super) fn ldm_sequence(data: &[u8], off: usize) -> Option<TlsCallSeq> {
    lea_call_sequence(data, off)
}

fn lea_call_sequence(data: &[u8], off: usize) -> Option<TlsCallSeq> {
    let lea_end = off + 4;
    if lea_end > data.len() {
        return None;
    }
    // SIB form: 8d 04 <sib>: modrm mod=00 reg=eax rm=100; SIB base=101
    // (no base, disp32), scale 1, index = the GOT register.
    let sib = off
        .checked_sub(3)
        .filter(|&s| data[s] == 0x8d && data[s + 1] == 0x04 && data[s + 2] & 0xc7 == 0x05)
        .map(|s| (s, Some((data[s + 2] >> 3) & 7)));
    // Plain form: 8d <modrm> with mod=10 reg=eax rm=GOT register (not SIB).
    let plain = off
        .checked_sub(2)
        .filter(|&s| data[s] == 0x8d && data[s + 1] & 0xf8 == 0x80 && data[s + 1] & 7 != ESP)
        .map(|s| (s, Some(data[s + 1] & 7)));
    let (start, got_reg) = sib.or(plain)?;
    if got_reg == Some(ESP) {
        return None; // `(,%esp,1)` is not encodable as an index
    }
    let (call_len, call_field) = tls_get_addr_call(data, lea_end)?;
    if lea_end + call_len > data.len() {
        // The call's 32-bit field runs past the section: not a sequence
        // (and rewriting it would write out of bounds).
        return None;
    }
    let mut len = lea_end - start + call_len;
    if len == 11 && byte(data, start + 11) == Some(0x90) {
        len = 12; // the `nop` GNU as-level code pads the short form with
    }
    Some(TlsCallSeq {
        start,
        len,
        got_reg,
        call_field,
    })
}

/// Offsets, within one input section, of the `___tls_get_addr` call fields
/// of its GD/LDM sequences.  An executable transitions every such sequence,
/// so the call — and the PLT32/PC32/GOT32(X) relocation on its field —
/// disappears.  Derived from the untouched input bytes, so the PLT/GOT
/// scan and relocation application agree, and neither depends on whether
/// the object lists the call's relocation before or after the `@tlsgd`
/// one.
pub(super) fn transitioned_call_fields<'a>(
    data: &'a [u8],
    relocations: &'a [(u32, u32, u32, i32)],
) -> impl Iterator<Item = u32> + 'a {
    use super::types::{R_386_TLS_GD, R_386_TLS_LDM};
    relocations
        .iter()
        .filter(|r| r.1 == R_386_TLS_GD || r.1 == R_386_TLS_LDM)
        .filter_map(move |r| lea_call_sequence(data, r.0 as usize))
        .map(|seq| seq.call_field as u32)
}

fn put32(out: &mut [u8], v: u32) {
    out.copy_from_slice(&v.to_le_bytes());
}

/// GD → LE: `%eax = %gs:0 + ntpoff`.
///
/// ```text
///   65 a1 00000000   movl %gs:0, %eax
///   8d 80 imm32      leal ntpoff(%eax), %eax      (12-byte sequences)
///   05 imm32         addl $ntpoff, %eax           (11-byte sequences)
/// ```
pub(super) fn gd_to_le(data: &mut [u8], seq: &TlsCallSeq, ntpoff: i32) {
    let s = seq.start;
    data[s..s + 6].copy_from_slice(&[0x65, 0xa1, 0, 0, 0, 0]);
    if seq.len == 12 {
        data[s + 6] = 0x8d;
        data[s + 7] = 0x80;
        put32(&mut data[s + 8..s + 12], ntpoff as u32);
    } else {
        debug_assert_eq!(seq.len, 11);
        data[s + 6] = 0x05;
        put32(&mut data[s + 7..s + 11], ntpoff as u32);
    }
}

/// GD → IE (DSO symbol from an executable): `%eax = %gs:0 + *slot`.
///
/// ```text
///   65 a1 00000000   movl %gs:0, %eax
///   03 80+r disp32   addl slot@gotntpoff(%reg), %eax
/// ```
/// Needs the 12-byte form and a GOT register other than `%eax` (the
/// `movl %gs:0, %eax` would overwrite the GOT pointer before the `addl`
/// reads it, and no 12-byte IE form addresses the GOT through `%eax`);
/// returns false otherwise and leaves `data` untouched — the caller reports
/// the failed transition.
pub(super) fn gd_to_ie(data: &mut [u8], seq: &TlsCallSeq, slot_gotoff: i32) -> bool {
    let Some(reg) = seq.got_reg else {
        return false;
    };
    if seq.len != 12 || reg == EAX {
        return false;
    }
    let s = seq.start;
    data[s..s + 6].copy_from_slice(&[0x65, 0xa1, 0, 0, 0, 0]);
    data[s + 6] = 0x03;
    data[s + 7] = 0x80 | reg;
    put32(&mut data[s + 8..s + 12], slot_gotoff as u32);
    true
}

/// LD → LE: `%eax = %gs:0` (the `@dtpoff` offsets then become `ntpoff`s).
///
/// ```text
///   65 a1 00000000             movl %gs:0, %eax
///   90 8d 74 26 00             nop; leal 0(%esi,%eiz,1), %esi   (11)
///   8d b6 00000000             leal 0(%esi), %esi               (12)
/// ```
pub(super) fn ldm_to_le(data: &mut [u8], seq: &TlsCallSeq) {
    let s = seq.start;
    data[s..s + 6].copy_from_slice(&[0x65, 0xa1, 0, 0, 0, 0]);
    if seq.len == 12 {
        data[s + 6..s + 12].copy_from_slice(&[0x8d, 0xb6, 0, 0, 0, 0]);
    } else {
        data[s + 6..s + 11].copy_from_slice(&[0x90, 0x8d, 0x74, 0x26, 0x00]);
    }
}

/// IE (`@indntpoff`, absolute slot address) → LE for a main-image symbol.
///
/// ```text
///   a1 disp32             movl x@indntpoff, %eax  ->  b8 imm32     movl $ntpoff, %eax
///   8b 05+8r disp32       movl x@indntpoff, %reg  ->  c7 c0+r imm32
///   03 05+8r disp32       addl x@indntpoff, %reg  ->  81 c0+r imm32
/// ```
pub(super) fn ie_to_le(data: &mut [u8], off: usize, ntpoff: i32) -> Result<(), String> {
    if off + 4 > data.len() {
        return Err("R_386_TLS_IE field is outside its section".to_string());
    }
    if off >= 1 && data[off - 1] == 0xa1 {
        // A 5-byte `movl moffs32, %eax`: only valid if it is not the tail
        // of a longer instruction whose modrm happens to be 0xa1.  The
        // psABI form is exactly this opcode, and the alternative 2-byte
        // encodings below never end in 0xa1 (their modrm has mod=00
        // rm=101, i.e. the low three bits are 101 and 0xa1 & 7 == 1).
        data[off - 1] = 0xb8;
    } else if off >= 2 && data[off - 1] & 0xc7 == 0x05 {
        let reg = (data[off - 1] >> 3) & 7;
        match data[off - 2] {
            0x8b => {
                data[off - 2] = 0xc7;
                data[off - 1] = 0xc0 | reg;
            }
            0x03 => {
                data[off - 2] = 0x81;
                data[off - 1] = 0xc0 | reg;
            }
            op => {
                return Err(format!(
                    "R_386_TLS_IE on unsupported instruction (opcode 0x{op:02x})"
                ));
            }
        }
    } else {
        return Err(
            "R_386_TLS_IE on an instruction that is not `movl`/`addl x@indntpoff`".to_string(),
        );
    }
    put32(&mut data[off..off + 4], ntpoff as u32);
    Ok(())
}

/// GOTIE (`@gotntpoff`, GOT-relative slot) → LE for a main-image symbol.
///
/// ```text
///   8b 80+8r+b disp32   movl x@gotntpoff(%b), %reg  ->  c7 c0+r imm32
///   03 80+8r+b disp32   addl x@gotntpoff(%b), %reg  ->  81 c0+r imm32
/// ```
/// (`C7 /0` puts the destination in r/m: the modrm is `0xc0 | reg`, not
/// `0xc0 | reg << 3`.)
pub(super) fn gotie_to_le(data: &mut [u8], off: usize, ntpoff: i32) -> Result<(), String> {
    if off < 2 || off + 4 > data.len() {
        return Err("R_386_TLS_GOTIE field is outside its section".to_string());
    }
    let modrm = data[off - 1];
    if modrm & 0xc0 != 0x80 || modrm & 7 == ESP {
        return Err(format!(
            "R_386_TLS_GOTIE on an unsupported addressing form (modrm 0x{modrm:02x}); expected `x@gotntpoff(%reg)`"
        ));
    }
    let reg = (modrm >> 3) & 7;
    match data[off - 2] {
        0x8b => data[off - 2] = 0xc7,
        0x03 => data[off - 2] = 0x81,
        op => {
            return Err(format!(
                "R_386_TLS_GOTIE on unsupported instruction (opcode 0x{op:02x}); expected movl/addl"
            ));
        }
    }
    data[off - 1] = 0xc0 | reg;
    put32(&mut data[off..off + 4], ntpoff as u32);
    Ok(())
}

/// TLS descriptor `leal x@tlsdesc(%b), %eax` (`8d 80+b disp32`) → LE:
/// `leal ntpoff, %eax` (`8d 05 imm32`), i.e. `%eax = ntpoff`, which the
/// following `call *x@tlscall(%eax)` (rewritten by [`desc_call_to_nop`])
/// would have returned.
pub(super) fn gotdesc_to_le(data: &mut [u8], off: usize, ntpoff: i32) -> Result<(), String> {
    check_gotdesc(data, off)?;
    data[off - 1] = 0x05;
    put32(&mut data[off..off + 4], ntpoff as u32);
    Ok(())
}

/// TLS descriptor → IE: `movl slot@gotntpoff(%b), %eax` (`8b 80+b`).
pub(super) fn gotdesc_to_ie(data: &mut [u8], off: usize, slot_gotoff: i32) -> Result<(), String> {
    check_gotdesc(data, off)?;
    data[off - 2] = 0x8b;
    put32(&mut data[off..off + 4], slot_gotoff as u32);
    Ok(())
}

fn check_gotdesc(data: &[u8], off: usize) -> Result<(), String> {
    let ok = off >= 2
        && off + 4 <= data.len()
        && data[off - 2] == 0x8d
        && data[off - 1] & 0xf8 == 0x80
        && data[off - 1] & 7 != ESP
        && (data[off - 1] >> 3) & 7 == EAX;
    if ok {
        Ok(())
    } else {
        Err(
            "R_386_TLS_GOTDESC on an instruction that is not `leal x@tlsdesc(%reg), %eax`"
                .to_string(),
        )
    }
}

/// `call *x@tlscall(%eax)` (`ff 10`) → `xchg %ax, %ax` (`66 90`).
/// `off` is the R_386_TLS_DESC_CALL offset, i.e. the call's first byte.
pub(super) fn desc_call_to_nop(data: &mut [u8], off: usize) -> Result<(), String> {
    match data.get(off..off + 2) {
        Some([0xff, 0x10]) => {
            data[off] = 0x66;
            data[off + 1] = 0x90;
            Ok(())
        }
        _ => Err("R_386_TLS_DESC_CALL on an instruction that is not `call *(%eax)`".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Sequences below are GNU as encodings (`as --32`) of the psABI forms.

    /// leal x@tlsgd(,%ebx,1),%eax; call ___tls_get_addr@PLT
    const GD_SIB: [u8; 12] = [0x8d, 0x04, 0x1d, 0, 0, 0, 0, 0xe8, 0, 0, 0, 0];
    /// leal x@tlsgd(%edx),%eax; call *___tls_get_addr@GOT(%edx)
    const GD_IND: [u8; 12] = [0x8d, 0x82, 0, 0, 0, 0, 0xff, 0x92, 0, 0, 0, 0];
    /// leal x@tlsgd(%ebx),%eax; call ___tls_get_addr@PLT
    const GD_SHORT: [u8; 11] = [0x8d, 0x83, 0, 0, 0, 0, 0xe8, 0, 0, 0, 0];

    #[test]
    fn recognises_gd_forms() {
        assert_eq!(
            gd_sequence(&GD_SIB, 3),
            Some(TlsCallSeq {
                start: 0,
                len: 12,
                got_reg: Some(3),
                call_field: 8
            })
        );
        assert_eq!(
            gd_sequence(&GD_IND, 2),
            Some(TlsCallSeq {
                start: 0,
                len: 12,
                got_reg: Some(2),
                call_field: 8
            })
        );
        assert_eq!(
            gd_sequence(&GD_SHORT, 2),
            Some(TlsCallSeq {
                start: 0,
                len: 11,
                got_reg: Some(3),
                call_field: 7
            })
        );
        let mut padded = GD_SHORT.to_vec();
        padded.push(0x90);
        assert_eq!(gd_sequence(&padded, 2).map(|s| s.len), Some(12));
    }

    #[test]
    fn rejects_non_eax_destination_and_foreign_calls() {
        // leal x@tlsgd(%ebx),%ecx
        let mut bad = GD_SHORT;
        bad[1] = 0x8b;
        assert_eq!(gd_sequence(&bad, 2), None);
        // not followed by a call
        let mut nocall = GD_SIB;
        nocall[7] = 0x90;
        assert_eq!(gd_sequence(&nocall, 3), None);
        // truncated
        assert_eq!(gd_sequence(&GD_SIB[..9], 3), None);
        // the call's rel32 field would run past the section end
        assert_eq!(gd_sequence(&GD_SIB[..11], 3), None);
        assert_eq!(gd_sequence(&GD_SHORT[..10], 2), None);
    }

    #[test]
    fn gd_to_le_encodings() {
        let mut d = GD_SIB;
        let seq = gd_sequence(&d, 3).unwrap();
        gd_to_le(&mut d, &seq, -8);
        assert_eq!(
            d,
            [0x65, 0xa1, 0, 0, 0, 0, 0x8d, 0x80, 0xf8, 0xff, 0xff, 0xff]
        );
        let mut d = GD_SHORT;
        let seq = gd_sequence(&d, 2).unwrap();
        gd_to_le(&mut d, &seq, -8);
        assert_eq!(d, [0x65, 0xa1, 0, 0, 0, 0, 0x05, 0xf8, 0xff, 0xff, 0xff]);
    }

    #[test]
    fn gd_to_ie_uses_the_lea_got_register() {
        let mut d = GD_IND;
        let seq = gd_sequence(&d, 2).unwrap();
        assert!(gd_to_ie(&mut d, &seq, 0x10));
        // addl 0x10(%edx),%eax
        assert_eq!(d, [0x65, 0xa1, 0, 0, 0, 0, 0x03, 0x82, 0x10, 0, 0, 0]);
        let mut d = GD_SHORT;
        let seq = gd_sequence(&d, 2).unwrap();
        assert!(
            !gd_to_ie(&mut d, &seq, 0x10),
            "11 bytes cannot hold the IE form"
        );
        assert_eq!(d, GD_SHORT, "a refused transition leaves the bytes alone");
        // leal x@tlsgd(%eax),%eax; call ___tls_get_addr@PLT; nop — valid GD,
        // but `movl %gs:0,%eax` would clobber the GOT register.
        let mut d = [0x8d, 0x80, 0, 0, 0, 0, 0xe8, 0, 0, 0, 0, 0x90];
        let orig = d;
        let seq = gd_sequence(&d, 2).unwrap();
        assert_eq!(seq.got_reg, Some(EAX));
        assert!(
            !gd_to_ie(&mut d, &seq, 0x10),
            "%eax GOT register must be refused"
        );
        assert_eq!(d, orig, "a refused transition must not modify the code");
    }

    #[test]
    fn ldm_to_le_encodings() {
        let mut d = GD_SHORT;
        let seq = ldm_sequence(&d, 2).unwrap();
        ldm_to_le(&mut d, &seq);
        assert_eq!(d, [0x65, 0xa1, 0, 0, 0, 0, 0x90, 0x8d, 0x74, 0x26, 0x00]);
        let mut d = GD_IND;
        let seq = ldm_sequence(&d, 2).unwrap();
        ldm_to_le(&mut d, &seq);
        assert_eq!(d, [0x65, 0xa1, 0, 0, 0, 0, 0x8d, 0xb6, 0, 0, 0, 0]);
    }

    #[test]
    fn ie_to_le_encodings() {
        // movl x@indntpoff,%eax
        let mut d = [0xa1, 0, 0, 0, 0];
        ie_to_le(&mut d, 1, -4).unwrap();
        assert_eq!(d, [0xb8, 0xfc, 0xff, 0xff, 0xff]);
        // movl x@indntpoff,%ecx  (8b 0d)
        let mut d = [0x8b, 0x0d, 0, 0, 0, 0];
        ie_to_le(&mut d, 2, -4).unwrap();
        assert_eq!(d, [0xc7, 0xc1, 0xfc, 0xff, 0xff, 0xff]);
        // addl x@indntpoff,%edx  (03 15)
        let mut d = [0x03, 0x15, 0, 0, 0, 0];
        ie_to_le(&mut d, 2, -4).unwrap();
        assert_eq!(d, [0x81, 0xc2, 0xfc, 0xff, 0xff, 0xff]);
        // subl is not an IE form
        let mut d = [0x2b, 0x15, 0, 0, 0, 0];
        assert!(ie_to_le(&mut d, 2, -4).is_err());
    }

    #[test]
    fn gotie_to_le_encodings() {
        // movl x@gotntpoff(%ebx),%esi  (8b b3)
        let mut d = [0x8b, 0xb3, 0, 0, 0, 0];
        gotie_to_le(&mut d, 2, -12).unwrap();
        assert_eq!(d, [0xc7, 0xc6, 0xf4, 0xff, 0xff, 0xff]);
        // addl x@gotntpoff(%ecx),%eax  (03 81)
        let mut d = [0x03, 0x81, 0, 0, 0, 0];
        gotie_to_le(&mut d, 2, -12).unwrap();
        assert_eq!(d, [0x81, 0xc0, 0xf4, 0xff, 0xff, 0xff]);
        // base-less absolute form is not a GOT access
        let mut d = [0x8b, 0x05, 0, 0, 0, 0];
        assert!(gotie_to_le(&mut d, 2, 0).is_err());
    }

    #[test]
    fn tlsdesc_encodings() {
        // leal x@tlsdesc(%ebx),%eax; call *x@tlscall(%eax)
        let seq = [0x8d, 0x83, 0, 0, 0, 0, 0xff, 0x10];
        let mut d = seq;
        gotdesc_to_le(&mut d, 2, -16).unwrap();
        desc_call_to_nop(&mut d, 6).unwrap();
        assert_eq!(d, [0x8d, 0x05, 0xf0, 0xff, 0xff, 0xff, 0x66, 0x90]);
        let mut d = seq;
        gotdesc_to_ie(&mut d, 2, 0x20).unwrap();
        assert_eq!(&d[..6], &[0x8b, 0x83, 0x20, 0, 0, 0]);
        // destination other than %eax is not a descriptor sequence
        let mut d = [0x8d, 0x8b, 0, 0, 0, 0];
        assert!(gotdesc_to_le(&mut d, 2, 0).is_err());
    }
}
