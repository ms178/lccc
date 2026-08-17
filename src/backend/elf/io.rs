//! Binary read/write helpers for little-endian ELF fields, plus section header,
//! program header, symbol table entry, and relocation entry writers.

// ── Overflow-safe range helpers ──────────────────────────────────────────────
//
// ELF offset/size fields are attacker-controlled 32/64-bit values.  The naive
// bounds check
//
//     if off + size <= data.len() { &data[off..off + size] }
//
// is **wrong**: `off + size` wraps in release builds (where overflow checks are
// off), so a section header claiming `sh_offset = 2^64 - 100, sh_size = 200`
// passes the guard and then panics — or, worse, silently reads the wrong
// memory — inside the slice operation.  A fuzzer found exactly this against
// `parse_object.rs` (`range start index 18446744073692774483 out of range`).
//
// These helpers make the correct form as short as the incorrect one, so there
// is no incentive to hand-roll a check again.  All arithmetic is checked; the
// happy path compiles to the same single compare + slice as the naive version
// because `checked_add` on values the optimizer can bound folds away.

/// Return `data[off .. off + len]`, or `None` if the range is not fully
/// contained in `data`.  Overflow-safe: `off + len` is computed with
/// `checked_add`, so wrap-around cannot produce a false "in bounds" verdict.
#[inline]
pub fn slice_at(data: &[u8], off: usize, len: usize) -> Option<&[u8]> {
    let end = off.checked_add(len)?;
    data.get(off..end)
}

/// Overflow-safe containment test for `[off, off + len)` within `data`.
///
/// Prefer [`slice_at`] when the slice itself is wanted; use this when only the
/// predicate is needed (e.g. before a loop that indexes incrementally).
#[inline]
pub fn range_ok(data: &[u8], off: usize, len: usize) -> bool {
    match off.checked_add(len) {
        Some(end) => end <= data.len(),
        None => false,
    }
}

/// Byte range of ELF table entry `idx` given a table base, entry size and
/// entry count-independent bounds check, or `None` on overflow / out of range.
///
/// Used for section-header, program-header, symbol and relocation tables,
/// where the naive `base + idx * entsize` multiplication is itself an overflow
/// hazard (`e_shentsize` and `e_shnum` are both attacker-controlled u16, and
/// `e_shoff` a full u64).
#[inline]
pub fn table_entry(data: &[u8], base: usize, idx: usize, entsize: usize) -> Option<&[u8]> {
    let off = idx.checked_mul(entsize)?.checked_add(base)?;
    slice_at(data, off, entsize)
}

// ── Binary read helpers (little-endian) ──────────────────────────────────────

/// Read a little-endian u16 from `data` at `offset`.
#[inline]
pub fn read_u16(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([data[offset], data[offset + 1]])
}

/// Read a little-endian u32 from `data` at `offset`.
#[inline]
pub fn read_u32(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        data[offset], data[offset + 1], data[offset + 2], data[offset + 3],
    ])
}

/// Read a little-endian u64 from `data` at `offset`.
#[inline]
pub fn read_u64(data: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        data[offset], data[offset + 1], data[offset + 2], data[offset + 3],
        data[offset + 4], data[offset + 5], data[offset + 6], data[offset + 7],
    ])
}

/// Read a little-endian i32 from `data` at `offset`.
#[inline]
pub fn read_i32(data: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes([
        data[offset], data[offset + 1], data[offset + 2], data[offset + 3],
    ])
}

/// Read a little-endian i64 from `data` at `offset`.
#[inline]
pub fn read_i64(data: &[u8], offset: usize) -> i64 {
    i64::from_le_bytes([
        data[offset], data[offset + 1], data[offset + 2], data[offset + 3],
        data[offset + 4], data[offset + 5], data[offset + 6], data[offset + 7],
    ])
}

/// Read a null-terminated string from a byte slice starting at `offset`.
///
/// Returns an empty string when `offset` is out of bounds.  An unterminated
/// tail is accepted and returned up to the end of `data` (GNU ld tolerates a
/// truncated final string table entry rather than rejecting the object).
pub fn read_cstr(data: &[u8], offset: usize) -> String {
    let tail = match data.get(offset..) {
        Some(t) => t,
        None => return String::new(),
    };
    let end = memchr0(tail);
    let bytes = &tail[..end];

    // Fast path: ELF symbol and section names are ASCII in every real
    // toolchain, and `str::from_utf8` on ASCII is a cheap validation scan,
    // whereas `String::from_utf8_lossy(..).into_owned()` drags in the
    // `Utf8Chunks` iterator and an extra copy.  Profiling a 20k-symbol link
    // attributed 3.8% of all instructions to `read_cstr` and a further 3.4%
    // to `Utf8Chunks::next`; this removes the latter entirely.
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_owned(),
        // Non-UTF-8 names are legal in ELF (the format stores bytes), so keep
        // the lossy fallback rather than rejecting the object.
        Err(_) => String::from_utf8_lossy(bytes).into_owned(),
    }
}

/// Borrowing counterpart of [`read_cstr`].
///
/// Returns a `&str` pointing directly into the string table instead of
/// allocating. Callers that only need to *inspect* a name, or that copy it
/// into a small-string type such as `SymStr`, must use this: allocating a
/// `String` just to immediately copy out of it doubles the allocation count of
/// symbol parsing (measured: 40 240 allocations for a 20 000-symbol link, ~2
/// per symbol).
///
/// Non-UTF-8 names are legal in ELF but vanishingly rare; because a borrowed
/// return cannot own a lossy conversion, this reports them as `None` and the
/// caller falls back to [`read_cstr`].
#[inline]
pub fn read_cstr_ref(data: &[u8], offset: usize) -> Option<&str> {
    let tail = data.get(offset..)?;
    let end = memchr0(tail);
    std::str::from_utf8(&tail[..end]).ok()
}

/// Index of the first NUL byte in `hay`, or `hay.len()`.
///
/// A byte-at-a-time `position()` costs one compare + one branch per byte and
/// dominated string-table parsing on symbol-heavy links.  This scans a machine
/// word at a time using the classic SWAR zero-byte test
/// (Mycroft's `(w - 0x01..01) & !w & 0x80..80`), which is portable, has no
/// `unsafe`, and needs no external crate (`memchr` would be a new dependency,
/// which linker policy avoids).
#[inline]
fn memchr0(hay: &[u8]) -> usize {
    const LO: usize = usize::from_ne_bytes([0x01; core::mem::size_of::<usize>()]);
    const HI: usize = usize::from_ne_bytes([0x80; core::mem::size_of::<usize>()]);
    const W: usize = core::mem::size_of::<usize>();

    let n = hay.len();
    let mut i = 0;

    // Align to a word boundary so the word loads never straddle a page.
    let align = (W - (hay.as_ptr() as usize % W)) % W;
    let head = align.min(n);
    while i < head {
        if hay[i] == 0 {
            return i;
        }
        i += 1;
    }

    while i + W <= n {
        // Aligned, in-bounds: this is a plain slice read, no unsafe needed.
        let chunk = usize::from_ne_bytes(hay[i..i + W].try_into().unwrap());
        if chunk.wrapping_sub(LO) & !chunk & HI != 0 {
            // A zero byte is somewhere in this word; find it exactly.
            for k in 0..W {
                if hay[i + k] == 0 {
                    return i + k;
                }
            }
        }
        i += W;
    }

    while i < n {
        if hay[i] == 0 {
            return i;
        }
        i += 1;
    }
    n
}

// ── Fallible readers for untrusted input ─────────────────────────────────────
//
// The `read_uN` family above indexes directly and therefore panics on a short
// buffer.  That is acceptable for buffers the linker itself just built, but not
// for parsing attacker-controlled files.  These variants return `None` instead
// and let the caller emit a proper diagnostic.

/// Read a little-endian u16 at `offset`, or `None` if out of bounds.
#[inline]
pub fn try_read_u16(data: &[u8], offset: usize) -> Option<u16> {
    slice_at(data, offset, 2).map(|b| u16::from_le_bytes([b[0], b[1]]))
}

/// Read a little-endian u32 at `offset`, or `None` if out of bounds.
#[inline]
pub fn try_read_u32(data: &[u8], offset: usize) -> Option<u32> {
    slice_at(data, offset, 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// Read a little-endian u64 at `offset`, or `None` if out of bounds.
#[inline]
pub fn try_read_u64(data: &[u8], offset: usize) -> Option<u64> {
    slice_at(data, offset, 8).map(|b| {
        u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
    })
}

// ── Binary write helpers (little-endian, in-place) ───────────────────────────

/// Write a little-endian u16 into `buf` at `offset`. No-op if out of bounds.
#[inline]
pub fn w16(buf: &mut [u8], off: usize, val: u16) {
    if off + 2 <= buf.len() {
        buf[off..off + 2].copy_from_slice(&val.to_le_bytes());
    }
}

/// Write a little-endian u32 into `buf` at `offset`. No-op if out of bounds.
#[inline]
pub fn w32(buf: &mut [u8], off: usize, val: u32) {
    if off + 4 <= buf.len() {
        buf[off..off + 4].copy_from_slice(&val.to_le_bytes());
    }
}

/// Write a little-endian u64 into `buf` at `offset`. No-op if out of bounds.
#[inline]
pub fn w64(buf: &mut [u8], off: usize, val: u64) {
    if off + 8 <= buf.len() {
        buf[off..off + 8].copy_from_slice(&val.to_le_bytes());
    }
}

/// Copy `data` into `buf` starting at `off`. No-op if out of bounds.
#[inline]
pub fn write_bytes(buf: &mut [u8], off: usize, data: &[u8]) {
    let end = off + data.len();
    if end <= buf.len() {
        buf[off..end].copy_from_slice(data);
    }
}

// ── Section header writing ───────────────────────────────────────────────────

/// Append an ELF64 section header to `buf`.
pub fn write_shdr64(
    buf: &mut Vec<u8>,
    sh_name: u32, sh_type: u32, sh_flags: u64,
    sh_addr: u64, sh_offset: u64, sh_size: u64,
    sh_link: u32, sh_info: u32, sh_addralign: u64, sh_entsize: u64,
) {
    buf.extend_from_slice(&sh_name.to_le_bytes());
    buf.extend_from_slice(&sh_type.to_le_bytes());
    buf.extend_from_slice(&sh_flags.to_le_bytes());
    buf.extend_from_slice(&sh_addr.to_le_bytes());
    buf.extend_from_slice(&sh_offset.to_le_bytes());
    buf.extend_from_slice(&sh_size.to_le_bytes());
    buf.extend_from_slice(&sh_link.to_le_bytes());
    buf.extend_from_slice(&sh_info.to_le_bytes());
    buf.extend_from_slice(&sh_addralign.to_le_bytes());
    buf.extend_from_slice(&sh_entsize.to_le_bytes());
}

/// Append an ELF32 section header to `buf`.
pub fn write_shdr32(
    buf: &mut Vec<u8>,
    sh_name: u32, sh_type: u32, sh_flags: u32,
    sh_addr: u32, sh_offset: u32, sh_size: u32,
    sh_link: u32, sh_info: u32, sh_addralign: u32, sh_entsize: u32,
) {
    buf.extend_from_slice(&sh_name.to_le_bytes());
    buf.extend_from_slice(&sh_type.to_le_bytes());
    buf.extend_from_slice(&sh_flags.to_le_bytes());
    buf.extend_from_slice(&sh_addr.to_le_bytes());
    buf.extend_from_slice(&sh_offset.to_le_bytes());
    buf.extend_from_slice(&sh_size.to_le_bytes());
    buf.extend_from_slice(&sh_link.to_le_bytes());
    buf.extend_from_slice(&sh_info.to_le_bytes());
    buf.extend_from_slice(&sh_addralign.to_le_bytes());
    buf.extend_from_slice(&sh_entsize.to_le_bytes());
}

/// Write an ELF64 program header to `buf` at offset `off`.
pub fn write_phdr64(
    buf: &mut [u8], off: usize,
    p_type: u32, p_flags: u32, p_offset: u64,
    p_vaddr: u64, p_paddr: u64, p_filesz: u64, p_memsz: u64, p_align: u64,
) {
    w32(buf, off, p_type);
    w32(buf, off + 4, p_flags);
    w64(buf, off + 8, p_offset);
    w64(buf, off + 16, p_vaddr);
    w64(buf, off + 24, p_paddr);
    w64(buf, off + 32, p_filesz);
    w64(buf, off + 40, p_memsz);
    w64(buf, off + 48, p_align);
}

/// Write an ELF64 program header with `p_paddr = p_vaddr` (the common case).
/// This is a convenience wrapper around `write_phdr64` used by multiple linker
/// backends to avoid repeating the vaddr twice.
#[inline]
pub fn wphdr(buf: &mut [u8], off: usize, pt: u32, flags: u32, foff: u64, va: u64, fsz: u64, msz: u64, align: u64) {
    write_phdr64(buf, off, pt, flags, foff, va, va, fsz, msz, align);
}

/// Write an ELF64 symbol table entry to `buf`.
pub fn write_sym64(
    buf: &mut Vec<u8>,
    st_name: u32, st_info: u8, st_other: u8, st_shndx: u16,
    st_value: u64, st_size: u64,
) {
    buf.extend_from_slice(&st_name.to_le_bytes());
    buf.push(st_info);
    buf.push(st_other);
    buf.extend_from_slice(&st_shndx.to_le_bytes());
    buf.extend_from_slice(&st_value.to_le_bytes());
    buf.extend_from_slice(&st_size.to_le_bytes());
}

/// Write an ELF32 symbol table entry to `buf`.
pub fn write_sym32(
    buf: &mut Vec<u8>,
    st_name: u32, st_value: u32, st_size: u32,
    st_info: u8, st_other: u8, st_shndx: u16,
) {
    buf.extend_from_slice(&st_name.to_le_bytes());
    buf.extend_from_slice(&st_value.to_le_bytes());
    buf.extend_from_slice(&st_size.to_le_bytes());
    buf.push(st_info);
    buf.push(st_other);
    buf.extend_from_slice(&st_shndx.to_le_bytes());
}

/// Write an ELF64 RELA relocation entry to `buf`.
pub fn write_rela64(buf: &mut Vec<u8>, r_offset: u64, r_sym: u32, r_type: u32, r_addend: i64) {
    buf.extend_from_slice(&r_offset.to_le_bytes());
    let r_info: u64 = ((r_sym as u64) << 32) | (r_type as u64);
    buf.extend_from_slice(&r_info.to_le_bytes());
    buf.extend_from_slice(&r_addend.to_le_bytes());
}

/// Write an ELF32 REL relocation entry to `buf`.
pub fn write_rel32(buf: &mut Vec<u8>, r_offset: u32, r_sym: u32, r_type: u8) {
    buf.extend_from_slice(&r_offset.to_le_bytes());
    let r_info: u32 = (r_sym << 8) | (r_type as u32);
    buf.extend_from_slice(&r_info.to_le_bytes());
}

/// Write an ELF32 RELA relocation entry to `buf`.
/// Used by architectures that require RELA even in 32-bit mode (e.g., RISC-V).
pub fn write_rela32(buf: &mut Vec<u8>, r_offset: u32, r_sym: u32, r_type: u8, r_addend: i32) {
    buf.extend_from_slice(&r_offset.to_le_bytes());
    let r_info: u32 = (r_sym << 8) | (r_type as u32);
    buf.extend_from_slice(&r_info.to_le_bytes());
    buf.extend_from_slice(&r_addend.to_le_bytes());
}

#[cfg(test)]
mod bounds_tests {
    use super::*;

    /// The exact defect a mutation fuzzer found in `parse_object.rs`: a section
    /// header whose `sh_offset` is near `u64::MAX` makes `off + size` wrap to a
    /// small value, so the naive guard `off + size <= len` succeeds and the
    /// subsequent slice panics.  `slice_at` must reject it.
    #[test]
    fn slice_at_rejects_wrapping_range() {
        let data = [0u8; 64];
        let off = usize::MAX - 16;
        // Demonstrate the hazard: wrapping arithmetic claims the range fits.
        assert!(off.wrapping_add(32) <= data.len());
        // The safe helper must not be fooled.
        assert!(slice_at(&data, off, 32).is_none());
        assert!(!range_ok(&data, off, 32));
    }

    #[test]
    fn slice_at_exact_and_boundary() {
        let data = [1u8, 2, 3, 4];
        assert_eq!(slice_at(&data, 0, 4), Some(&data[..]));
        assert_eq!(slice_at(&data, 4, 0), Some(&[] as &[u8])); // empty at end is valid
        assert_eq!(slice_at(&data, 2, 2), Some(&[3u8, 4][..]));
        assert_eq!(slice_at(&data, 3, 2), None);               // one past the end
        assert_eq!(slice_at(&data, 5, 0), None);               // start beyond end
    }

    #[test]
    fn table_entry_rejects_multiplication_overflow() {
        let data = [0u8; 4096];
        // e_shnum * e_shentsize overflows usize; must not wrap into a valid range.
        assert!(table_entry(&data, 0, usize::MAX / 8, 64).is_none());
        assert!(table_entry(&data, usize::MAX, 1, 64).is_none());
        // Normal indexing still works.
        assert_eq!(table_entry(&data, 64, 2, 64).map(<[u8]>::len), Some(64));
        // Last fully-contained entry is accepted, the next is not.
        assert!(table_entry(&data, 0, 63, 64).is_some());
        assert!(table_entry(&data, 0, 64, 64).is_none());
    }

    #[test]
    fn try_readers_are_total() {
        let data = [0xefu8, 0xbe, 0xad, 0xde, 0, 0, 0, 0];
        assert_eq!(try_read_u16(&data, 0), Some(0xbeef));
        assert_eq!(try_read_u32(&data, 0), Some(0xdeadbeef));
        assert_eq!(try_read_u64(&data, 0), Some(0x00000000deadbeef));
        // Out of bounds and overflowing offsets yield None, never a panic.
        assert_eq!(try_read_u32(&data, 5), None);
        assert_eq!(try_read_u64(&data, 1), None);
        assert_eq!(try_read_u16(&data, usize::MAX), None);
    }

    /// Exhaustive check of the SWAR NUL scanner against the naive reference,
    /// across every alignment and every NUL position in a multi-word buffer.
    /// Word-at-a-time scanning is easy to get subtly wrong at the head/tail
    /// boundaries, so this brute-forces the whole space rather than sampling.
    #[test]
    fn memchr0_matches_naive_for_all_alignments_and_positions() {
        const N: usize = 40;
        // A backing buffer big enough to slice at every alignment offset.
        let mut buf = vec![0xffu8; N + 16];
        for align in 0..16usize {
            for pos in 0..N {
                for b in buf.iter_mut() { *b = 0xff; }
                buf[align + pos] = 0;
                let hay = &buf[align..align + N];
                let naive = hay.iter().position(|&b| b == 0).unwrap_or(hay.len());
                assert_eq!(memchr0(hay), naive,
                           "align={align} pos={pos}");
            }
            // No NUL at all: must report the full length.
            for b in buf.iter_mut() { *b = 0xff; }
            let hay = &buf[align..align + N];
            assert_eq!(memchr0(hay), N, "align={align} (no NUL)");
        }
        // Degenerate sizes.
        assert_eq!(memchr0(b""), 0);
        assert_eq!(memchr0(b"\0"), 0);
        assert_eq!(memchr0(b"a"), 1);
    }

    /// Non-UTF-8 names are legal in ELF; the fast ASCII path must not change
    /// the result for them.
    #[test]
    fn read_cstr_preserves_lossy_behaviour_for_non_utf8() {
        let data = b"ok\0\xff\xfe bad\0";
        assert_eq!(read_cstr(data, 0), "ok");
        let got = read_cstr(data, 3);
        assert_eq!(got, String::from_utf8_lossy(b"\xff\xfe bad"));
        assert!(got.contains("bad"));
    }

    #[test]
    fn read_cstr_handles_unterminated_and_oob() {
        // Unterminated tail: return what is there rather than panicking.
        assert_eq!(read_cstr(b"abc", 0), "abc");
        assert_eq!(read_cstr(b"abc\0def", 4), "def");
        assert_eq!(read_cstr(b"abc\0def", 0), "abc");
        // Offset at and beyond the end.
        assert_eq!(read_cstr(b"abc", 3), "");
        assert_eq!(read_cstr(b"abc", 99), "");
        assert_eq!(read_cstr(b"", 0), "");
        assert_eq!(read_cstr(b"abc", usize::MAX), "");
    }
}
