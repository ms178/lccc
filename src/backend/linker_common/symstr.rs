//! `SymStr` — a small-string-optimised, immutable symbol name.
//!
//! # Why this exists
//!
//! Profiling a 20 000-symbol link attributed **14.5 % of all instructions**
//! (inclusive) to `read_cstr`, and DHAT counted **40 240 heap allocations** for
//! a link with ~20 000 symbols — roughly two per symbol. Every one of them is a
//! `String` holding a name that is almost always tiny:
//!
//! | corpus | symbols | mean len | ≤ 23 bytes |
//! |---|---:|---:|---:|
//! | 20k-symbol synthetic | 20 003 | 10.4 | 100 % |
//! | expat `xmlparse.o` | 688 | 14.7 | 77 % |
//! | glibc `libc.so.6` | 3 161 | 12.9 | 91 % |
//!
//! So the overwhelming majority of these allocations exist to store fewer than
//! 24 bytes. `SymStr` keeps those inline and only falls back to the heap for
//! genuinely long names (C++ manglings, mostly).
//!
//! # Why not `SymbolId(u32)` interning?
//!
//! Interning is the theoretically better answer — it also removes the *hashing*
//! and *comparison* of names in the hot maps, which SSO does not. It was
//! rejected **for now** because it forces `FxHashMap<String, GlobalSymbol>` to
//! change shape in four backends (x86, ARM, RISC-V, i686) plus every emitter, a
//! change far too large to validate in one step against a linker whose
//! correctness is established by differential testing. `SymStr` is the
//! low-risk 80 % of that win: it is a drop-in for `String` at every call site
//! (`Deref<Target = str>`, `PartialEq<str>`, `Borrow<str>` so map lookups by
//! `&str` still work), so it can be adopted incrementally and reverted
//! trivially. See docs/linker/FOLLOWUP for the interning plan that builds on
//! it.
//!
//! # Layout
//!
//! ```text
//! inline:  [len: u8][bytes: 23]                       — 24 bytes, no alloc
//! heap:    [0xFF   ][ptr: 8][len: 8][_pad: 6]          — 24 bytes, one alloc
//! ```
//!
//! `size_of::<SymStr>() == 24`, the same as `String`, so embedding it in
//! `Elf64Symbol` does not grow the symbol array.
//!
//! # Safety
//!
//! The heap variant owns a `Box<str>`; `Drop` frees it exactly once and `Clone`
//! deep-copies. The inline variant holds initialised UTF-8 bytes. Both are
//! covered by the exhaustive tests at the bottom of this file, including a
//! Miri-friendly round-trip over every length from 0 to 64.

use std::borrow::Borrow;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Deref;

/// Bytes available for inline storage. 23 payload + 1 tag byte = 24 bytes,
/// exactly `size_of::<String>()`.
const INLINE_CAP: usize = 23;
/// Value of the tag byte meaning "the payload is a heap pointer".
const HEAP_TAG: u8 = 0xFF;

/// Inline variant: payload then tag, so the tag sits at offset 23.
#[repr(C)]
#[derive(Clone, Copy)]
struct InlineRepr {
    bytes: [u8; INLINE_CAP],
    tag: u8,
}

/// Heap variant: pointer + length, explicitly padded so that its tag byte
/// lands at offset 23 too — the same offset as `InlineRepr::tag`. Both
/// variants therefore agree on where the discriminant lives, which is what
/// makes reading the tag through either union field well-defined.
#[repr(C)]
#[derive(Clone, Copy)]
struct HeapRepr {
    ptr: std::ptr::NonNull<u8>,
    len: usize,
    /// Always zeroed: leaving it uninitialised would make reading the tag
    /// through `InlineRepr` (which covers these bytes) undefined behaviour.
    _pad: [u8; 7],
    tag: u8,
}

const _: () = {
    assert!(std::mem::size_of::<InlineRepr>() == 24);
    assert!(std::mem::size_of::<HeapRepr>() == 24);
    assert!(std::mem::offset_of!(InlineRepr, tag) == 23);
    assert!(std::mem::offset_of!(HeapRepr, tag) == 23);
};

pub struct SymStr {
    data: SymData,
}

union SymData {
    inline: InlineRepr,
    heap: HeapRepr,
}

impl SymStr {
    /// Create from a string slice, storing inline when it fits.
    #[inline]
    pub fn new(s: &str) -> Self {
        let b = s.as_bytes();
        if b.len() <= INLINE_CAP {
            let mut bytes = [0u8; INLINE_CAP];
            bytes[..b.len()].copy_from_slice(b);
            SymStr {
                data: SymData {
                    inline: InlineRepr {
                        bytes,
                        tag: b.len() as u8,
                    },
                },
            }
        } else {
            let boxed: Box<str> = s.into();
            let len = boxed.len();
            let raw = Box::into_raw(boxed) as *mut u8;
            // SAFETY: `Box::into_raw` never returns null.
            let ptr = unsafe { std::ptr::NonNull::new_unchecked(raw) };
            SymStr {
                data: SymData {
                    heap: HeapRepr {
                        ptr,
                        len,
                        _pad: [0; 7],
                        tag: HEAP_TAG,
                    },
                },
            }
        }
    }

    /// The empty name. `const` so it can initialise statics cheaply.
    #[inline]
    pub const fn empty() -> Self {
        SymStr {
            data: SymData {
                inline: InlineRepr {
                    bytes: [0u8; INLINE_CAP],
                    tag: 0,
                },
            },
        }
    }

    /// Read the discriminant. Always legal: byte 23 is initialised in both
    /// variants (`HeapRepr::_pad` is zeroed precisely so this holds).
    #[inline]
    fn tag(&self) -> u8 {
        // SAFETY: `InlineRepr` is all-integer and fully initialised in both
        // variants, so reading its `tag` field is defined whichever is active.
        unsafe { self.data.inline.tag }
    }

    #[inline]
    pub fn as_str(&self) -> &str {
        let tag = self.tag();
        // SAFETY: both variants are built only from a valid `&str`, so the
        // bytes are valid UTF-8. The inline branch reads exactly `tag` bytes,
        // all initialised by `new`; the heap branch reads the `Box<str>` we
        // still own, whose pointer and length are unchanged since creation.
        unsafe {
            if tag == HEAP_TAG {
                let h = &self.data.heap;
                std::str::from_utf8_unchecked(std::slice::from_raw_parts(h.ptr.as_ptr(), h.len))
            } else {
                std::str::from_utf8_unchecked(&self.data.inline.bytes[..tag as usize])
            }
        }
    }

    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        self.as_str().as_bytes()
    }

    /// Copy into an owned `String` with a single sized allocation.
    ///
    /// This *inherent* method shadows the blanket `ToString` impl that
    /// `Display` provides, and that shadowing is the whole point. The generic
    /// `ToString` formats through `fmt::Write::write_str`, starting from an
    /// empty `String` and growing it: DHAT attributed **20 001 reallocations**
    /// on a 20 000-symbol link to `sym.name.to_string()` in the symbol
    /// registration loop -- one per symbol, all pure growth traffic. Copying
    /// from a known length allocates exactly once.
    ///
    /// Method resolution prefers inherent methods, so every existing
    /// `name.to_string()` call site picks this up with no edit.
    #[inline]
    #[allow(clippy::inherent_to_string_shadow_display)]
    pub fn to_string(&self) -> String {
        self.as_str().to_owned()
    }

    #[inline]
    pub fn len(&self) -> usize {
        let tag = self.tag();
        if tag == HEAP_TAG {
            // SAFETY: heap variant is active, so `len` is initialised.
            unsafe { self.data.heap.len }
        } else {
            tag as usize
        }
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.tag() == 0
    }

    /// True when the name is stored without a heap allocation. Test-only
    /// observability: production code must not branch on this.
    #[inline]
    pub fn is_inline(&self) -> bool {
        self.tag() != HEAP_TAG
    }
}

impl Drop for SymStr {
    #[inline]
    fn drop(&mut self) {
        if self.tag() == HEAP_TAG {
            // SAFETY: the heap variant is active and owns the allocation, which
            // came from `Box::<str>::into_raw` with exactly this pointer and
            // length. `Drop` runs once, so the box is freed exactly once.
            unsafe {
                let h = self.data.heap;
                let slice = std::ptr::slice_from_raw_parts_mut(h.ptr.as_ptr(), h.len);
                drop(Box::from_raw(slice as *mut str));
            }
        }
    }
}

impl Clone for SymStr {
    #[inline]
    fn clone(&self) -> Self {
        SymStr::new(self.as_str())
    }
}

impl Default for SymStr {
    #[inline]
    fn default() -> Self {
        SymStr::empty()
    }
}

impl Deref for SymStr {
    type Target = str;
    #[inline]
    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<str> for SymStr {
    #[inline]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

/// Lets `FxHashMap<SymStr, _>` be probed with a plain `&str`, which keeps every
/// existing `map.get(name_str)` call site working unchanged.
impl Borrow<str> for SymStr {
    #[inline]
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl PartialEq for SymStr {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}
impl Eq for SymStr {}

impl PartialEq<str> for SymStr {
    #[inline]
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}
impl PartialEq<&str> for SymStr {
    #[inline]
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}
impl PartialEq<String> for SymStr {
    #[inline]
    fn eq(&self, other: &String) -> bool {
        self.as_str() == other.as_str()
    }
}
impl PartialEq<SymStr> for str {
    #[inline]
    fn eq(&self, other: &SymStr) -> bool {
        self == other.as_str()
    }
}
impl PartialEq<SymStr> for String {
    #[inline]
    fn eq(&self, other: &SymStr) -> bool {
        self.as_str() == other.as_str()
    }
}

impl PartialOrd for SymStr {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for SymStr {
    #[inline]
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.as_str().cmp(other.as_str())
    }
}

/// Hashes identically to `str`, which is what makes `Borrow<str>` sound.
impl Hash for SymStr {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_str().hash(state)
    }
}

impl fmt::Debug for SymStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}
impl fmt::Display for SymStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self.as_str(), f)
    }
}

impl From<&str> for SymStr {
    #[inline]
    fn from(s: &str) -> Self {
        SymStr::new(s)
    }
}
impl From<String> for SymStr {
    #[inline]
    fn from(s: String) -> Self {
        SymStr::new(&s)
    }
}
impl From<&SymStr> for String {
    #[inline]
    fn from(s: &SymStr) -> String {
        s.as_str().to_owned()
    }
}

// SAFETY: `SymStr` owns its data exclusively (inline bytes or a uniquely-owned
// `Box<str>` allocation) and hands out only shared references, so it is exactly
// as thread-safe as `String`. The raw pointer in `HeapRepr` is what suppresses
// the automatic impls; it is never aliased.
unsafe impl Send for SymStr {}
unsafe impl Sync for SymStr {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::hash_map::DefaultHasher;

    fn hash_of<T: Hash>(t: &T) -> u64 {
        let mut h = DefaultHasher::new();
        t.hash(&mut h);
        h.finish()
    }

    /// The whole point of the type: it must not be bigger than `String`,
    /// otherwise embedding it in `Elf64Symbol` grows every symbol array and
    /// trades allocations for cache misses.
    #[test]
    fn is_no_larger_than_string() {
        assert_eq!(std::mem::size_of::<SymStr>(), std::mem::size_of::<String>());
    }

    /// Exhaustive round-trip across the inline/heap boundary. Word-at-a-time
    /// layout bugs hide exactly at len == CAP and len == CAP + 1.
    #[test]
    fn roundtrip_every_length_0_to_64() {
        for n in 0..=64usize {
            let s: String = (0..n).map(|i| (b'a' + (i % 26) as u8) as char).collect();
            let sym = SymStr::new(&s);
            assert_eq!(sym.as_str(), s, "len {n}");
            assert_eq!(sym.len(), n, "len {n}");
            assert_eq!(sym.is_empty(), n == 0, "len {n}");
            assert_eq!(sym.is_inline(), n <= INLINE_CAP, "len {n} storage class");
            // Clone must be independent and equal.
            let c = sym.clone();
            assert_eq!(c, sym, "len {n} clone");
            assert_eq!(c.as_str(), s, "len {n} clone content");
            drop(sym);
            assert_eq!(c.as_str(), s, "len {n} clone outlives original");
        }
    }

    #[test]
    fn boundary_lengths_pick_the_right_storage() {
        assert!(SymStr::new(&"x".repeat(INLINE_CAP)).is_inline());
        assert!(!SymStr::new(&"x".repeat(INLINE_CAP + 1)).is_inline());
        assert_eq!(SymStr::new(&"x".repeat(INLINE_CAP)).len(), INLINE_CAP);
        assert_eq!(
            SymStr::new(&"x".repeat(INLINE_CAP + 1)).len(),
            INLINE_CAP + 1
        );
    }

    /// `Borrow<str>` is only sound if the hashes agree; a mismatch would make
    /// `map.get("name")` miss entries inserted as `SymStr`, which would show up
    /// as mysterious "undefined symbol" errors rather than a crash.
    #[test]
    fn hashes_like_str_in_both_storage_classes() {
        for s in [
            "",
            "main",
            "g_sym_19999",
            "a_very_long_mangled_symbol_name_that_will_not_fit_inline",
        ] {
            assert_eq!(
                hash_of(&SymStr::new(s)),
                hash_of(&s),
                "hash mismatch for {s:?}"
            );
        }
    }

    #[test]
    fn works_as_a_hashmap_key_probed_by_str() {
        use crate::common::fx_hash::FxHashMap;
        let mut m: FxHashMap<SymStr, u32> = FxHashMap::default();
        m.insert(SymStr::new("short"), 1);
        m.insert(
            SymStr::new("a_long_symbol_name_exceeding_the_inline_capacity"),
            2,
        );
        assert_eq!(m.get("short"), Some(&1));
        assert_eq!(
            m.get("a_long_symbol_name_exceeding_the_inline_capacity"),
            Some(&2)
        );
        assert_eq!(m.get("absent"), None);
    }

    #[test]
    fn ordering_matches_str() {
        let mut v: Vec<SymStr> = [
            "b",
            "a",
            "aa",
            "",
            "z_long_name_over_the_inline_limit_xxxxxxxx",
        ]
        .iter()
        .map(|s| SymStr::new(s))
        .collect();
        v.sort();
        let got: Vec<&str> = v.iter().map(|s| s.as_str()).collect();
        let mut want = [
            "b",
            "a",
            "aa",
            "",
            "z_long_name_over_the_inline_limit_xxxxxxxx",
        ];
        want.sort();
        assert_eq!(got, want);
    }

    #[test]
    fn utf8_and_comparison_helpers() {
        // ELF permits non-ASCII names; they must survive both storage classes.
        let s = "sym_ü_ß_名前";
        assert_eq!(SymStr::new(s).as_str(), s);
        let long = format!("{s}{s}{s}{s}");
        assert_eq!(SymStr::new(&long).as_str(), long);

        let a = SymStr::new("main");
        assert!(a == *"main");
        assert!(a == "main");
        assert!(a == String::from("main"));
        assert!(*"main" == a);
        assert_eq!(a.as_bytes(), b"main");
        assert_eq!(format!("{a}"), "main");
        assert_eq!(format!("{a:?}"), "\"main\"");
    }

    /// `to_string()` must resolve to the inherent single-allocation method,
    /// not the `Display`-derived blanket impl that grows a `String` byte by
    /// byte. A regression here is invisible to correctness tests but cost
    /// 20 001 reallocations per 20 000-symbol link when it last happened, so
    /// pin the observable consequence: capacity equal to the length, which
    /// `String::to_owned` guarantees and incremental growth does not.
    #[test]
    fn to_string_allocates_exactly_once() {
        for n in [0, 1, INLINE_CAP, INLINE_CAP + 1, 200] {
            let src: String = "s".repeat(n);
            let got = SymStr::new(&src).to_string();
            assert_eq!(got, src, "len {n} content");
            assert_eq!(
                got.capacity(),
                n,
                "len {n}: capacity {} != len {n}; to_string() regressed to \
                        the Display-based blanket impl and is growing the buffer",
                got.capacity()
            );
        }
    }

    #[test]
    fn default_and_empty_are_equivalent() {
        assert_eq!(SymStr::default(), SymStr::empty());
        assert!(SymStr::default().is_empty());
        assert!(SymStr::default().is_inline());
        assert_eq!(SymStr::default().as_str(), "");
    }

    /// Many clones and drops of a heap-backed value: catches a double free or
    /// a leak of the `ManuallyDrop<Box<str>>` under Miri/ASan.
    #[test]
    fn heap_variant_clone_drop_storm() {
        let base = SymStr::new(&"m".repeat(200));
        let mut v = Vec::new();
        for _ in 0..256 {
            v.push(base.clone());
        }
        for c in &v {
            assert_eq!(c.len(), 200);
        }
        drop(v);
        assert_eq!(base.len(), 200);
    }
}
