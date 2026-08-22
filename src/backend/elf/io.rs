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
///
/// Slicing once and converting a fixed-size array (rather than indexing each
/// byte) emits a single bounds check and a single load. The byte-at-a-time
/// form costs one check *per byte*: `read_u64` alone was 3.85 M Ir (5.13 %)
/// over 80 150 calls while parsing a 20 000-symbol object.
#[inline]
pub fn read_u16(data: &[u8], offset: usize) -> u16 {
    let b: [u8; 2] = data[offset..offset + 2].try_into().unwrap();
    u16::from_le_bytes(b)
}

/// Read a little-endian u32 from `data` at `offset`.
#[inline]
pub fn read_u32(data: &[u8], offset: usize) -> u32 {
    let b: [u8; 4] = data[offset..offset + 4].try_into().unwrap();
    u32::from_le_bytes(b)
}

/// Read a little-endian u64 from `data` at `offset`.
#[inline]
pub fn read_u64(data: &[u8], offset: usize) -> u64 {
    let b: [u8; 8] = data[offset..offset + 8].try_into().unwrap();
    u64::from_le_bytes(b)
}

/// Read a little-endian i32 from `data` at `offset`.
#[inline]
pub fn read_i32(data: &[u8], offset: usize) -> i32 {
    let b: [u8; 4] = data[offset..offset + 4].try_into().unwrap();
    i32::from_le_bytes(b)
}

/// Read a little-endian i64 from `data` at `offset`.
#[inline]
pub fn read_i64(data: &[u8], offset: usize) -> i64 {
    let b: [u8; 8] = data[offset..offset + 8].try_into().unwrap();
    i64::from_le_bytes(b)
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
    let bytes = &tail[..end];

    // Fast path: symbol and section names are pure ASCII in every mainstream
    // toolchain. `str::from_utf8` runs the general multi-byte validator, which
    // walks the string a second time after `memchr0` already scanned it and
    // cost 1.80 M Ir (2.74 %) on a 20 000-symbol link. A word-at-a-time
    // high-bit test settles the common case in a few instructions, and any
    // string containing a non-ASCII byte still falls through to the full
    // validator, so non-UTF-8 input is rejected exactly as before.
    if is_ascii_fast(bytes) {
        // SAFETY: every byte is < 0x80, so the slice is valid UTF-8.
        return Some(unsafe { std::str::from_utf8_unchecked(bytes) });
    }
    std::str::from_utf8(bytes).ok()
}

/// True when every byte is ASCII (< 0x80), tested a machine word at a time.
///
/// `<[u8]>::is_ascii` is already word-wise, but goes through a slower generic
/// path at `-O1`; this mirrors the SWAR style used by `memchr0` below and
/// keeps the two scans symmetrical.
#[inline]
fn is_ascii_fast(hay: &[u8]) -> bool {
    const HI: usize = usize::from_ne_bytes([0x80; core::mem::size_of::<usize>()]);
    const W: usize = core::mem::size_of::<usize>();
    let mut i = 0;
    while i + W <= hay.len() {
        let mut buf = [0u8; W];
        buf.copy_from_slice(&hay[i..i + W]);
        if usize::from_ne_bytes(buf) & HI != 0 {
            return false;
        }
        i += W;
    }
    while i < hay.len() {
        if hay[i] >= 0x80 {
            return false;
        }
        i += 1;
    }
    true
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
    slice_at(data, offset, 8)
        .map(|b| u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
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

/// Append a NUL-terminated name to a string table, returning its offset.
///
/// The obvious spelling of this is
/// ```ignore
/// let off = tab.len();
/// tab.extend_from_slice(name.as_bytes());
/// tab.push(0);
/// ```
/// which costs two calls per symbol, and `extend_from_slice` bottoms out in a
/// `memcpy` call even for the ~10-byte names that dominate real symbol tables.
/// Callgrind measured **40 075 calls / 1.9 M Ir (2.54 %)** for exactly this
/// pattern while building `.strtab` on a 20 000-symbol link.
///
/// Reserving once and doing a single `copy_nonoverlapping` plus a direct
/// terminator write removes the second call and the second capacity check.
///
/// Measured alternatives on the 20 000-symbol link (fastbuild, callgrind Ir):
/// the obvious `extend_from_slice` + `push` baseline is 74.96 M; an
/// `extend(iter.chain(once(0)))` formulation is **79.30 M -- 5.8 % worse**,
/// because the byte-at-a-time iterator defeats the vectorised copy. Only the
/// explicit copy below is actually faster, which is why this function carries
/// `unsafe` rather than a tidy iterator chain.
///
/// The caller is expected to have pre-sized `tab` (the reserve below is then
/// a predictable no-op); correctness does not depend on it.
#[inline]
pub fn push_strtab_name(tab: &mut Vec<u8>, name: &[u8]) -> u32 {
    let off = tab.len() as u32;
    let n = name.len();
    tab.reserve(n + 1);
    let len = tab.len();
    // SAFETY: `reserve` guarantees at least `n + 1` bytes of spare capacity,
    // so the region `[len, len + n]` is allocated and writable. `name` is a
    // separate borrow, so source and destination cannot overlap. `set_len`
    // runs only after every byte in the new range has been initialised.
    unsafe {
        let dst = tab.as_mut_ptr().add(len);
        core::ptr::copy_nonoverlapping(name.as_ptr(), dst, n);
        dst.add(n).write(0);
        tab.set_len(len + n + 1);
    }
    off
}

/// Build one 24-byte `Elf64_Sym` table entry.
///
/// The open-coded form assembles the entry with four `copy_from_slice` calls
/// into subranges of a `[u8; 24]`. Each is a slice copy with its own length
/// check, and callgrind counted **200 256 calls / 3.86 M Ir (5.94 %)** of
/// `copy_from_slice_impl::<u8>` on a 20 000-symbol link -- about ten per
/// symbol across the local and global passes.
///
/// Writing the fields into a fixed-size array with constant indices lets the
/// compiler emit plain stores instead: the layout is known at compile time,
/// so there is nothing to check.
///
/// Field order is the ELF64 ABI's: `st_name`, `st_info`, `st_other`,
/// `st_shndx`, `st_value`, `st_size`.
#[inline]
pub fn elf64_sym_entry(
    name_off: u32,
    info: u8,
    other: u8,
    shndx: u16,
    value: u64,
    size: u64,
) -> [u8; 24] {
    let n = name_off.to_le_bytes();
    let x = shndx.to_le_bytes();
    let v = value.to_le_bytes();
    let z = size.to_le_bytes();
    [
        n[0], n[1], n[2], n[3], info, other, x[0], x[1], v[0], v[1], v[2], v[3], v[4], v[5], v[6],
        v[7], z[0], z[1], z[2], z[3], z[4], z[5], z[6], z[7],
    ]
}

// ── Section header writing ───────────────────────────────────────────────────

/// Append an ELF64 section header to `buf`.
pub fn write_shdr64(
    buf: &mut Vec<u8>,
    sh_name: u32,
    sh_type: u32,
    sh_flags: u64,
    sh_addr: u64,
    sh_offset: u64,
    sh_size: u64,
    sh_link: u32,
    sh_info: u32,
    sh_addralign: u64,
    sh_entsize: u64,
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
    sh_name: u32,
    sh_type: u32,
    sh_flags: u32,
    sh_addr: u32,
    sh_offset: u32,
    sh_size: u32,
    sh_link: u32,
    sh_info: u32,
    sh_addralign: u32,
    sh_entsize: u32,
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
    buf: &mut [u8],
    off: usize,
    p_type: u32,
    p_flags: u32,
    p_offset: u64,
    p_vaddr: u64,
    p_paddr: u64,
    p_filesz: u64,
    p_memsz: u64,
    p_align: u64,
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
pub fn wphdr(
    buf: &mut [u8],
    off: usize,
    pt: u32,
    flags: u32,
    foff: u64,
    va: u64,
    fsz: u64,
    msz: u64,
    align: u64,
) {
    write_phdr64(buf, off, pt, flags, foff, va, va, fsz, msz, align);
}

/// Write an ELF64 symbol table entry to `buf`.
pub fn write_sym64(
    buf: &mut Vec<u8>,
    st_name: u32,
    st_info: u8,
    st_other: u8,
    st_shndx: u16,
    st_value: u64,
    st_size: u64,
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
    st_name: u32,
    st_value: u32,
    st_size: u32,
    st_info: u8,
    st_other: u8,
    st_shndx: u16,
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
        assert_eq!(slice_at(&data, 3, 2), None); // one past the end
        assert_eq!(slice_at(&data, 5, 0), None); // start beyond end
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
                for b in buf.iter_mut() {
                    *b = 0xff;
                }
                buf[align + pos] = 0;
                let hay = &buf[align..align + N];
                let naive = hay.iter().position(|&b| b == 0).unwrap_or(hay.len());
                assert_eq!(memchr0(hay), naive, "align={align} pos={pos}");
            }
            // No NUL at all: must report the full length.
            for b in buf.iter_mut() {
                *b = 0xff;
            }
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

    /// `push_strtab_name` contains `unsafe` pointer writes, so pin its whole
    /// contract: returned offsets, NUL termination, exact final length, and
    /// that the bytes are byte-for-byte what the safe formulation produces.
    #[test]
    fn push_strtab_name_matches_safe_formulation() {
        let names: Vec<&[u8]> = vec![
            b"",
            b"a",
            b"main",
            b"_ZTI1E",
            b"a_very_long_mangled_symbol_name_that_exceeds_any_inline_buffer",
            b"\xff\xfe non utf8", // ELF names are bytes, not necessarily UTF-8
        ];

        let mut fast: Vec<u8> = Vec::new();
        let mut fast_offs = Vec::new();
        for n in &names {
            fast_offs.push(push_strtab_name(&mut fast, n));
        }

        // Reference: the obvious two-call spelling this replaced.
        let mut safe: Vec<u8> = Vec::new();
        let mut safe_offs = Vec::new();
        for n in &names {
            safe_offs.push(safe.len() as u32);
            safe.extend_from_slice(n);
            safe.push(0);
        }

        assert_eq!(
            fast, safe,
            "string table bytes differ from the safe version"
        );
        assert_eq!(fast_offs, safe_offs, "returned offsets differ");

        // Each returned offset must address exactly the name it was given,
        // terminated by a NUL -- this is what every sh_name/st_name points at.
        for (off, n) in fast_offs.iter().zip(&names) {
            let start = *off as usize;
            let end = start + n.len();
            assert_eq!(&fast[start..end], *n);
            assert_eq!(fast[end], 0, "missing NUL terminator");
        }
        let total: usize = names.iter().map(|n| n.len() + 1).sum();
        assert_eq!(fast.len(), total, "unexpected final length");
    }

    /// Appending into a table that already has content (and spare capacity)
    /// must not disturb what is already there -- `set_len` juggling is exactly
    /// where an off-by-one would corrupt earlier entries.
    #[test]
    fn push_strtab_name_preserves_existing_content() {
        let mut tab: Vec<u8> = Vec::with_capacity(4); // deliberately too small
        let a = push_strtab_name(&mut tab, b"first");
        let b = push_strtab_name(&mut tab, b"second");
        let c = push_strtab_name(&mut tab, b"");
        assert_eq!((a, b, c), (0, 6, 13));
        assert_eq!(tab, b"first\0second\0\0");
        assert_eq!(read_cstr(&tab, a as usize), "first");
        assert_eq!(read_cstr(&tab, b as usize), "second");
        assert_eq!(read_cstr(&tab, c as usize), "");
    }

    /// The little-endian readers were rewritten from byte-at-a-time indexing
    /// to a single slice + `try_into`. That is only a safe refactor if the
    /// *values* and the *panic boundary* are both unchanged: malformed ELF
    /// reaches these helpers, and the parser relies on a panic (caught
    /// upstream) rather than silent truncation for a short buffer.
    #[test]
    fn le_readers_values_and_panic_boundary() {
        let d: Vec<u8> = (0u8..=15).collect();
        assert_eq!(read_u16(&d, 0), 0x0100);
        assert_eq!(read_u16(&d, 14), 0x0f0e);
        assert_eq!(read_u32(&d, 0), 0x03020100);
        assert_eq!(read_u32(&d, 12), 0x0f0e0d0c);
        assert_eq!(read_u64(&d, 0), 0x0706050403020100);
        assert_eq!(read_u64(&d, 8), 0x0f0e0d0c0b0a0908);
        assert_eq!(read_i32(&d, 0), 0x03020100);
        assert_eq!(read_i64(&d, 8), 0x0f0e0d0c0b0a0908);

        // Sign-extension must still work through the array conversion.
        let neg = [0xffu8; 8];
        assert_eq!(read_i32(&neg, 0), -1);
        assert_eq!(read_i64(&neg, 0), -1);

        // Exactly-fitting reads are in bounds; one byte short must panic.
        assert_eq!(read_u64(&d[..8], 0), 0x0706050403020100);
        for f in [
            (|| {
                read_u16(&[0u8; 1], 0);
            }) as fn(),
            (|| {
                read_u32(&[0u8; 3], 0);
            }) as fn(),
            (|| {
                read_u64(&[0u8; 7], 0);
            }) as fn(),
            (|| {
                read_i32(&[0u8; 3], 0);
            }) as fn(),
            (|| {
                read_i64(&[0u8; 7], 0);
            }) as fn(),
            (|| {
                read_u64(&[0u8; 8], 1);
            }) as fn(),
        ] {
            assert!(
                std::panic::catch_unwind(f).is_err(),
                "short read must panic, not read out of bounds"
            );
        }
    }

    /// `read_cstr_ref` takes an ASCII fast path that skips UTF-8 validation
    /// via `from_utf8_unchecked`. That is sound only if `is_ascii_fast` is
    /// exact, so probe the word-boundary cases (the SWAR loop handles 8 bytes
    /// at a time, so a non-ASCII byte in the tail remainder is the classic
    /// miss) and confirm the slow path still *rejects* invalid UTF-8.
    #[test]
    fn read_cstr_ref_ascii_fastpath_matches_from_utf8() {
        // Pure ASCII of every length across the word boundary.
        for n in 0..40usize {
            let mut buf: Vec<u8> = (0..n).map(|i| b'a' + (i % 26) as u8).collect();
            buf.push(0);
            let got = read_cstr_ref(&buf, 0);
            let want = std::str::from_utf8(&buf[..n]).ok();
            assert_eq!(got, want, "ascii len {n}");
        }

        // A non-ASCII byte at every position: must still decode identically to
        // from_utf8 (valid UTF-8 sequences) or be rejected (invalid ones).
        for pos in 0..20usize {
            let mut buf = vec![b'x'; 20];
            buf[pos] = 0xC3;
            buf[(pos + 1) % 20] = 0xA9; // 'é' when adjacent
            buf.push(0);
            let end = buf.len() - 1;
            assert_eq!(
                read_cstr_ref(&buf, 0),
                std::str::from_utf8(&buf[..end]).ok(),
                "non-ascii at {pos}"
            );
        }

        // Invalid UTF-8 must be rejected, not silently reinterpreted.
        let bad = [b'a', 0xFF, b'b', 0];
        assert_eq!(read_cstr_ref(&bad, 0), None);
        let lone_cont = [0x80u8, 0];
        assert_eq!(read_cstr_ref(&lone_cont, 0), None);

        // Valid multi-byte UTF-8 must survive the slow path intact.
        let utf8 = "näme_ünïcode_名前";
        let mut buf = utf8.as_bytes().to_vec();
        buf.push(0);
        assert_eq!(read_cstr_ref(&buf, 0), Some(utf8));

        // Out-of-range offset stays None.
        assert_eq!(read_cstr_ref(b"abc\0", 99), None);
    }

    /// Direct exhaustive check of the ASCII predicate against the std one,
    /// including all lengths around the word size and a high bit in each slot.
    #[test]
    fn is_ascii_fast_agrees_with_std() {
        for n in 0..24usize {
            let base: Vec<u8> = vec![b'q'; n];
            assert_eq!(is_ascii_fast(&base), base.is_ascii(), "len {n} all ascii");
            for pos in 0..n {
                let mut v = base.clone();
                v[pos] = 0x80;
                assert_eq!(
                    is_ascii_fast(&v),
                    v.is_ascii(),
                    "len {n}, high bit at {pos}"
                );
                let mut w = base.clone();
                w[pos] = 0x7f; // highest still-ASCII byte
                assert_eq!(is_ascii_fast(&w), w.is_ascii(), "len {n}, 0x7f at {pos}");
            }
        }
    }

    /// `elf64_sym_entry` replaced four `copy_from_slice` calls into a
    /// `[u8; 24]`. Any transposed field would corrupt every symbol table the
    /// linker writes, so compare it byte-for-byte against the open-coded form
    /// it replaced, and verify each field lands at its ABI-mandated offset.
    #[test]
    fn elf64_sym_entry_matches_open_coded_layout() {
        let cases = [
            (0u32, 0u8, 0u8, 0u16, 0u64, 0u64),
            (1, 0x12, 0x03, 0xfff1, 0x0000_0000_0040_1000, 24),
            (u32::MAX, u8::MAX, u8::MAX, u16::MAX, u64::MAX, u64::MAX),
            (
                0xdead_beef,
                0x21,
                0x02,
                0x000e,
                0x1122_3344_5566_7788,
                0x99aa_bbcc_ddee_ff00,
            ),
        ];
        for (name_off, info, other, shndx, value, size) in cases {
            let got = elf64_sym_entry(name_off, info, other, shndx, value, size);

            let mut want = [0u8; 24];
            want[0..4].copy_from_slice(&name_off.to_le_bytes());
            want[4] = info;
            want[5] = other;
            want[6..8].copy_from_slice(&shndx.to_le_bytes());
            want[8..16].copy_from_slice(&value.to_le_bytes());
            want[16..24].copy_from_slice(&size.to_le_bytes());
            assert_eq!(got, want, "entry bytes differ for {name_off:#x}");

            // Independent decode at the ABI offsets, so a self-consistent but
            // wrong layout in both versions would still be caught.
            assert_eq!(read_u32(&got, 0), name_off, "st_name at 0");
            assert_eq!(got[4], info, "st_info at 4");
            assert_eq!(got[5], other, "st_other at 5");
            assert_eq!(read_u16(&got, 6), shndx, "st_shndx at 6");
            assert_eq!(read_u64(&got, 8), value, "st_value at 8");
            assert_eq!(read_u64(&got, 16), size, "st_size at 16");
        }
    }
}
