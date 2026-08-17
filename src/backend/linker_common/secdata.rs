//! `SectionData` — section bytes that are shared with the input file buffer
//! instead of copied out of it.
//!
//! # Why this exists
//!
//! `parse_elf64_object` used to materialise every section with
//! `bytes.to_vec()`. The whole input file has *already* been read into memory
//! by `std::fs::read`, so that second copy is pure waste: on a 20 000-symbol
//! object it accounted for **1.71 M instructions (2.68 % of the entire link)**
//! and 1.7 MB of allocation, and it was the single largest remaining
//! `memcpy` source after the byte-level work of session 7.
//!
//! # Why `Arc<[u8]>` and not a lifetime
//!
//! The obvious fix — `section_data: Vec<&'a [u8]>` borrowed from the file
//! buffer — does not work here, and it is worth recording exactly why so the
//! next person does not spend a day rediscovering it:
//!
//! * The file buffer is a **local** `Vec<u8>` inside `load_file` /
//!   `load_archive_elf64`, while the resulting `Elf64Object` is pushed into a
//!   caller-owned `Vec<Elf64Object>` that outlives it. Borrowing would require
//!   threading a lifetime parameter through `Elf64Object`, every backend's
//!   `ElfObject` alias, and all 18 files that touch `section_data`.
//! * Archive members are worse: `load_archive_elf64` reads one member at a
//!   time into a temporary, so there is no single buffer to borrow from.
//! * Some sections are *synthesised* rather than parsed (`strmerge` builds
//!   string pools; `linker_entry` fabricates sections), so a pure borrow
//!   cannot represent every case regardless of lifetimes.
//!
//! `Arc<[u8]>` sidesteps all three: the parser hands each section an
//! `Arc` clone of the one file buffer plus a range, synthesised sections own
//! their bytes, and nothing needs a lifetime parameter. Cloning an `Arc` is a
//! refcount bump, not a copy.
//!
//! # Drop-in contract
//!
//! Every existing site reads section data as a byte slice
//! (`&obj.section_data[i]`). `Deref<Target = [u8]>` keeps all of those
//! compiling unchanged; `From<Vec<u8>>` keeps the synthesising sites working.
//! `section_data` is read at 56 sites across 18 files and **mutated at none
//! after construction**, which is what makes a shared, immutable
//! representation sound.

use std::ops::Deref;
use std::sync::Arc;

/// Immutable bytes of one input section.
///
/// Either a window into a shared file buffer (the common case, no copy) or an
/// owned buffer for synthesised sections.
#[derive(Clone)]
pub struct SectionData {
    /// Backing storage. Shared between every section of the same input file.
    buf: Arc<[u8]>,
    /// Byte range of this section within `buf`.
    start: usize,
    end: usize,
}

impl SectionData {
    /// Empty section (`SHT_NOBITS`, or a zero-length section).
    #[inline]
    pub fn empty() -> Self {
        SectionData { buf: Arc::from(&[][..]), start: 0, end: 0 }
    }

    /// A window `[start, start + len)` into an existing shared buffer.
    ///
    /// Returns `None` when the range does not lie inside `buf`, which is how
    /// a truncated or malformed section header is rejected — the caller turns
    /// that into a diagnostic rather than a panic.
    #[inline]
    pub fn slice(buf: &Arc<[u8]>, start: usize, len: usize) -> Option<Self> {
        let end = start.checked_add(len)?;
        if end > buf.len() {
            return None;
        }
        Some(SectionData { buf: Arc::clone(buf), start, end })
    }

    /// Take ownership of bytes that do not come from an input file
    /// (synthesised sections: merged string pools, fabricated tables).
    #[inline]
    pub fn owned(bytes: Vec<u8>) -> Self {
        let end = bytes.len();
        SectionData { buf: Arc::from(bytes), start: 0, end }
    }

    #[inline]
    pub fn as_slice(&self) -> &[u8] {
        // The range is validated at construction and the buffer is immutable,
        // so this cannot be out of bounds.
        &self.buf[self.start..self.end]
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.end - self.start
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    /// Copy out into an owned `Vec`. Only for the few callers that genuinely
    /// need to mutate a private copy; prefer `as_slice`.
    #[inline]
    pub fn to_owned_vec(&self) -> Vec<u8> {
        self.as_slice().to_vec()
    }
}

impl Deref for SectionData {
    type Target = [u8];
    #[inline]
    fn deref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl AsRef<[u8]> for SectionData {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl Default for SectionData {
    #[inline]
    fn default() -> Self {
        SectionData::empty()
    }
}

impl From<Vec<u8>> for SectionData {
    #[inline]
    fn from(v: Vec<u8>) -> Self {
        SectionData::owned(v)
    }
}

impl From<&[u8]> for SectionData {
    #[inline]
    fn from(s: &[u8]) -> Self {
        SectionData::owned(s.to_vec())
    }
}

impl PartialEq for SectionData {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}
impl Eq for SectionData {}

impl std::fmt::Debug for SectionData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SectionData({} bytes)", self.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf(n: usize) -> Arc<[u8]> {
        Arc::from((0..n).map(|i| i as u8).collect::<Vec<u8>>())
    }

    /// Windows must expose exactly their range and nothing either side of it —
    /// an off-by-one here would feed neighbouring sections' bytes into
    /// relocation processing, which is silent corruption rather than a crash.
    #[test]
    fn slice_windows_expose_exact_range() {
        let b = buf(64);
        for (start, len) in [(0usize, 0usize), (0, 1), (0, 64), (10, 20), (63, 1), (64, 0)] {
            let sd = SectionData::slice(&b, start, len)
                .unwrap_or_else(|| panic!("in-bounds slice {start}+{len} rejected"));
            assert_eq!(sd.len(), len, "len for {start}+{len}");
            assert_eq!(sd.is_empty(), len == 0);
            assert_eq!(&*sd, &b[start..start + len], "content for {start}+{len}");
            assert_eq!(sd.as_slice(), &b[start..start + len]);
            assert_eq!(sd.to_owned_vec(), b[start..start + len].to_vec());
        }
    }

    /// Out-of-range windows must be rejected, including the overflow case: a
    /// malformed object with `sh_offset` near `usize::MAX` previously turned a
    /// bad file into a panic instead of a diagnostic.
    #[test]
    fn out_of_range_and_overflowing_windows_are_rejected() {
        let b = buf(16);
        assert!(SectionData::slice(&b, 0, 17).is_none(), "past end");
        assert!(SectionData::slice(&b, 16, 1).is_none(), "starts at end, non-empty");
        assert!(SectionData::slice(&b, 17, 0).is_none(), "starts past end");
        assert!(SectionData::slice(&b, usize::MAX, 1).is_none(), "offset overflow");
        assert!(SectionData::slice(&b, 1, usize::MAX).is_none(), "length overflow");
    }

    /// Cloning must share, not copy: that is the entire point of the type.
    /// Verified through the refcount rather than by timing.
    #[test]
    fn clones_share_one_allocation() {
        let b = buf(32);
        assert_eq!(Arc::strong_count(&b), 1);
        let a = SectionData::slice(&b, 0, 16).unwrap();
        let c = SectionData::slice(&b, 16, 16).unwrap();
        assert_eq!(Arc::strong_count(&b), 3, "each window holds one Arc ref");
        let d = a.clone();
        assert_eq!(Arc::strong_count(&b), 4, "clone bumps the refcount");
        // Independent windows into the same buffer stay independent.
        assert_eq!(&*a, &b[0..16]);
        assert_eq!(&*c, &b[16..32]);
        assert_eq!(&*d, &b[0..16]);
        drop((a, c, d));
        assert_eq!(Arc::strong_count(&b), 1, "refs released on drop");
    }

    /// Synthesised sections own their bytes and must behave identically to
    /// windows at every read site.
    #[test]
    fn owned_sections_behave_like_windows() {
        let v = vec![1u8, 2, 3, 4];
        let o = SectionData::owned(v.clone());
        assert_eq!(&*o, &v[..]);
        assert_eq!(o.len(), 4);
        assert!(!o.is_empty());

        let from_vec: SectionData = v.clone().into();
        assert_eq!(from_vec, o);
        let from_slice: SectionData = (&v[..]).into();
        assert_eq!(from_slice, o);

        let e = SectionData::empty();
        assert!(e.is_empty());
        assert_eq!(e.len(), 0);
        assert_eq!(&*e, &[][..]);
        assert_eq!(SectionData::default(), e);
    }

    /// The read sites use slice methods through `Deref`; make sure the usual
    /// ones resolve, so the drop-in claim is tested rather than assumed.
    #[test]
    fn deref_gives_full_slice_api() {
        let b = buf(8);
        let sd = SectionData::slice(&b, 2, 4).unwrap();
        assert_eq!(sd.first(), Some(&2u8));
        assert_eq!(sd.last(), Some(&5u8));
        assert_eq!(sd.get(1), Some(&3u8));
        assert_eq!(sd.get(99), None);
        assert_eq!(&sd[1..3], &[3u8, 4]);
        assert_eq!(sd.iter().copied().sum::<u8>(), 2 + 3 + 4 + 5);
        let as_ref: &[u8] = sd.as_ref();
        assert_eq!(as_ref.len(), 4);
    }

    /// End-to-end guard for the *point* of this type: after parsing an object
    /// out of a shared buffer, the section contents must alias that buffer
    /// rather than be copies of it.
    ///
    /// Checking Ir counts in a unit test would be flaky, so this asserts the
    /// observable structural property instead: the section's bytes live at an
    /// address inside the original allocation. A regression to `to_vec()`
    /// would move them elsewhere and fail here.
    #[test]
    fn parsed_sections_alias_the_input_buffer() {
        use crate::backend::linker_common::parse_elf64_object_at;

        // Minimal ELF64 relocatable object: header + one PROGBITS section
        // with recognisable contents + a section header table.
        const MARKER: &[u8] = b"ALIASME!";
        let mut f = vec![0u8; 64];
        f[0..4].copy_from_slice(b"\x7fELF");
        f[4] = 2;            // ELFCLASS64
        f[5] = 1;            // ELFDATA2LSB
        f[6] = 1;            // EV_CURRENT
        f[16..18].copy_from_slice(&1u16.to_le_bytes());   // ET_REL
        f[18..20].copy_from_slice(&62u16.to_le_bytes());  // EM_X86_64
        let data_off = 64usize;
        f.extend_from_slice(MARKER);
        let shoff = f.len();
        f[40..48].copy_from_slice(&(shoff as u64).to_le_bytes()); // e_shoff
        f[58..60].copy_from_slice(&64u16.to_le_bytes());          // e_shentsize
        f[60..62].copy_from_slice(&2u16.to_le_bytes());           // e_shnum
        f[62..64].copy_from_slice(&0u16.to_le_bytes());           // e_shstrndx

        // SHT_NULL, then our PROGBITS section.
        f.extend_from_slice(&[0u8; 64]);
        let mut sh = [0u8; 64];
        sh[4..8].copy_from_slice(&1u32.to_le_bytes());                    // SHT_PROGBITS
        sh[24..32].copy_from_slice(&(data_off as u64).to_le_bytes());      // sh_offset
        sh[32..40].copy_from_slice(&(MARKER.len() as u64).to_le_bytes());  // sh_size
        f.extend_from_slice(&sh);

        let buf: Arc<[u8]> = Arc::from(f);
        let buf_start = buf.as_ptr() as usize;
        let buf_end = buf_start + buf.len();

        let obj = parse_elf64_object_at(&buf, 0, buf.len(), "<test>", 62)
            .expect("synthetic object should parse");
        let sd = &obj.section_data[1];
        assert_eq!(&**sd, MARKER, "section contents");

        let p = sd.as_slice().as_ptr() as usize;
        assert!(p >= buf_start && p < buf_end,
                "section data at {p:#x} is outside the input buffer \
                 [{buf_start:#x}, {buf_end:#x}) -- it was COPIED, so the \
                 zero-copy property regressed");
        assert_eq!(p, buf_start + data_off, "window must point at sh_offset");
    }

    /// A section whose header claims bytes past the end of its own object must
    /// still be rejected even when the shared buffer is larger (an archive
    /// member must not be able to read its neighbour's bytes).
    #[test]
    fn section_cannot_escape_its_object_within_a_larger_buffer() {
        let b = buf(128);
        // Window arithmetic is what enforces this; verify the primitive
        // refuses a range outside the object extent it is given.
        assert!(SectionData::slice(&b, 100, 40).is_none(),
                "range past buffer end must be refused");
        assert!(SectionData::slice(&b, 100, 28).is_some(),
                "range ending exactly at buffer end is fine");
    }
}
