//! `FileMap` — read an input file without copying its contents.
//!
//! # Why
//!
//! Every input reaches the linker through one `std::fs::read`, which allocates
//! a buffer and copies the whole file into it. `SectionData` already avoids a
//! *second* copy by windowing into that buffer (see `secdata.rs`), but the
//! first copy remained, and it is not small: on the gzip workload (12 objects
//! plus an archive) `load_file` accounts for **3.46 M instructions, 6.6 % of
//! the entire link**, essentially all of it `__memcpy_avx_unaligned_erms`.
//!
//! `mmap(PROT_READ, MAP_PRIVATE)` removes it outright. The kernel maps the
//! page cache into the address space; the bytes are never copied, and pages
//! that the linker does not touch are never even faulted in. That last point
//! matters for archives, where the linker typically reads the symbol index and
//! a handful of members out of a much larger file.
//!
//! # Why not the `memmap2` crate
//!
//! This crate has **zero external dependencies**, which keeps the build fast on
//! constrained hardware and keeps the supply chain empty. The needed surface is
//! three libc calls, so they are declared directly.
//!
//! # Safety
//!
//! A memory mapping is only sound while the underlying file is not truncated
//! from underneath us: shrinking a mapped file turns the affected pages into
//! `SIGBUS` on access. A linker reads inputs that a build system is not
//! concurrently rewriting, and GNU ld, gold, lld and mold all mmap their inputs
//! on this same assumption. Two guards are applied anyway:
//!
//! * Zero-length files are never mapped (`mmap` rejects a zero length).
//! * Any failure — unmappable filesystem, `/proc`, a pipe, ENOMEM — falls back
//!   to `std::fs::read`, so correctness never depends on the mapping working.
//!
//! `LCCC_NO_MMAP=1` forces the fallback for A/B measurement and debugging.

use std::os::unix::io::AsRawFd;
use std::sync::Arc;

#[cfg(unix)]
mod sys {
    extern "C" {
        pub fn mmap(
            addr: *mut core::ffi::c_void,
            length: usize,
            prot: i32,
            flags: i32,
            fd: i32,
            offset: i64,
        ) -> *mut core::ffi::c_void;
        pub fn munmap(addr: *mut core::ffi::c_void, length: usize) -> i32;
        pub fn madvise(addr: *mut core::ffi::c_void, length: usize, advice: i32) -> i32;
    }
    pub const PROT_READ: i32 = 1;
    pub const MAP_PRIVATE: i32 = 2;
    pub const MAP_FAILED: *mut core::ffi::c_void = usize::MAX as *mut core::ffi::c_void;
    /// `MADV_WILLNEED` — start reading ahead. Object files are consumed almost
    /// end to end, so the kernel may as well fetch them in one go rather than
    /// fault them in a page at a time.
    pub const MADV_WILLNEED: i32 = 3;
}

/// An immutable, read-only mapping of a whole file.
///
/// `Send`/`Sync` because the mapping is read-only and the pointer is valid for
/// the lifetime of the value.
struct Mapping {
    ptr: *mut core::ffi::c_void,
    len: usize,
}

unsafe impl Send for Mapping {}
unsafe impl Sync for Mapping {}

impl Drop for Mapping {
    fn drop(&mut self) {
        // SAFETY: `ptr`/`len` come from a successful `mmap` in `map_file` and
        // are unmapped exactly once, here.
        unsafe {
            sys::munmap(self.ptr, self.len);
        }
    }
}

/// The bytes of one input file.
///
/// Either a memory mapping (no copy) or an owned buffer (fallback). Both are
/// exposed as `&[u8]`, and both can hand out an `Arc<[u8]>`-equivalent view for
/// `SectionData`.
pub struct FileMap {
    inner: FileMapInner,
}

enum FileMapInner {
    Mapped(Arc<Mapping>),
    Owned(Arc<[u8]>),
}

impl FileMap {
    /// Map `path`, falling back to a plain read when mapping is unavailable.
    ///
    /// Never fails for a reason the fallback could have avoided: a mapping
    /// error is not reported to the caller, only an unreadable file is.
    pub fn open(path: &str) -> Result<Self, String> {
        if std::env::var_os("LCCC_NO_MMAP").is_none() {
            if let Some(m) = Self::try_map(path) {
                return Ok(m);
            }
        }
        let data = std::fs::read(path)
            .map_err(|e| format!("failed to read '{}': {}", path, e))?;
        Ok(Self { inner: FileMapInner::Owned(data.into()) })
    }

    #[cfg(unix)]
    fn try_map(path: &str) -> Option<Self> {
        let file = std::fs::File::open(path).ok()?;
        let len = file.metadata().ok()?.len();
        // mmap(2) rejects a zero length, and a zero-byte input has no content
        // worth mapping anyway.
        if len == 0 || len > usize::MAX as u64 {
            return None;
        }
        let len = len as usize;
        // SAFETY: `fd` is a valid open descriptor for the duration of the call;
        // a null hint lets the kernel choose the address; PROT_READ/MAP_PRIVATE
        // gives a private read-only view that cannot alias writes elsewhere.
        let ptr = unsafe {
            sys::mmap(
                std::ptr::null_mut(),
                len,
                sys::PROT_READ,
                sys::MAP_PRIVATE,
                file.as_raw_fd(),
                0,
            )
        };
        if ptr == sys::MAP_FAILED || ptr.is_null() {
            return None;
        }
        // Advisory only; failure is irrelevant to correctness.
        unsafe {
            sys::madvise(ptr, len, sys::MADV_WILLNEED);
        }
        // The mapping stays valid after the descriptor is closed.
        drop(file);
        Some(Self { inner: FileMapInner::Mapped(Arc::new(Mapping { ptr, len })) })
    }

    #[cfg(not(unix))]
    fn try_map(_path: &str) -> Option<Self> {
        None
    }

    #[inline]
    pub fn as_slice(&self) -> &[u8] {
        match &self.inner {
            // SAFETY: the mapping is live for as long as `self` holds the
            // `Arc`, covers exactly `len` readable bytes, and is never written.
            FileMapInner::Mapped(m) => unsafe {
                std::slice::from_raw_parts(m.ptr as *const u8, m.len)
            },
            FileMapInner::Owned(v) => v,
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.as_slice().len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// True when the bytes came from a mapping rather than a copy.
    ///
    /// Exposed so a regression test can assert the fast path is actually taken
    /// instead of silently falling back forever.
    #[inline]
    pub fn is_mapped(&self) -> bool {
        matches!(self.inner, FileMapInner::Mapped(_))
    }

    /// A shared handle to the backing storage, for `SectionData`.
    #[inline]
    pub fn backing(&self) -> FileBacking {
        match &self.inner {
            FileMapInner::Mapped(m) => FileBacking::mapped(Arc::clone(m)),
            FileMapInner::Owned(v) => FileBacking::owned(Arc::clone(v)),
        }
    }
}

impl std::ops::Deref for FileMap {
    type Target = [u8];
    #[inline]
    fn deref(&self) -> &[u8] {
        self.as_slice()
    }
}

/// Shared ownership of an input file's bytes.
///
/// Cloning is a refcount bump in both variants, so every section of a file can
/// hold one without copying anything.
///
/// The representation is deliberately opaque, mirroring `FileMap`/
/// `FileMapInner` above. `Mapping` is a raw-pointer RAII type whose only
/// invariant is "unmapped exactly once"; exposing it in a public enum variant
/// would both leak that detail into the crate's API and make the private type
/// reachable at `pub` visibility (a `private_interfaces` warning). Callers only
/// ever need `as_slice`, plus `owned` to wrap bytes they already hold.
#[derive(Clone)]
pub struct FileBacking {
    inner: FileBackingInner,
}

#[derive(Clone)]
enum FileBackingInner {
    Mapped(Arc<Mapping>),
    Owned(Arc<[u8]>),
}

impl FileBacking {
    /// Wrap bytes the caller already owns (synthesised sections, and the
    /// non-mappable fallback).
    #[inline]
    pub fn owned(buf: Arc<[u8]>) -> Self {
        FileBacking { inner: FileBackingInner::Owned(buf) }
    }

    #[inline]
    fn mapped(m: Arc<Mapping>) -> Self {
        FileBacking { inner: FileBackingInner::Mapped(m) }
    }

    #[inline]
    pub fn as_slice(&self) -> &[u8] {
        match &self.inner {
            // SAFETY: as in `FileMap::as_slice`; the `Arc` keeps the mapping
            // alive for at least as long as this borrow.
            FileBackingInner::Mapped(m) => unsafe {
                std::slice::from_raw_parts(m.ptr as *const u8, m.len)
            },
            FileBackingInner::Owned(v) => v,
        }
    }

    /// True when these bytes are a mapping rather than a copy.
    ///
    /// Only meaningful to tests asserting the zero-copy path is taken.
    #[inline]
    pub fn is_mapped(&self) -> bool {
        matches!(self.inner, FileBackingInner::Mapped(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};

    // `cargo test` runs tests in one binary concurrently, so a PID-keyed
    // temporary directory is shared and torn down early by whichever test
    // finishes first. Key per call.
    static SEQ: AtomicU64 = AtomicU64::new(0);
    fn tmp_path(tag: &str) -> std::path::PathBuf {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("lccc_filemap_{}_{}_{}", tag, std::process::id(), n))
    }

    fn write_file(tag: &str, bytes: &[u8]) -> std::path::PathBuf {
        let p = tmp_path(tag);
        let mut f = std::fs::File::create(&p).unwrap();
        f.write_all(bytes).unwrap();
        f.sync_all().unwrap();
        p
    }

    #[test]
    fn maps_a_file_and_reads_its_bytes() {
        let content: Vec<u8> = (0..4096u32).map(|i| (i % 251) as u8).collect();
        let p = write_file("map", &content);
        let m = FileMap::open(p.to_str().unwrap()).unwrap();
        assert_eq!(m.len(), content.len());
        assert_eq!(m.as_slice(), &content[..]);
        assert!(m.is_mapped(), "expected the mmap path to be taken on Linux");
        std::fs::remove_file(&p).ok();
    }

    /// The mapping must outlive the descriptor: `try_map` drops the `File`
    /// immediately, and reading afterwards must still be valid.
    #[test]
    fn mapping_outlives_the_file_descriptor() {
        let content = b"the mapping stays valid after close(2)".to_vec();
        let p = write_file("fdclose", &content);
        let m = FileMap::open(p.to_str().unwrap()).unwrap();
        // Also unlink: on Unix the mapping survives removal of the directory entry.
        std::fs::remove_file(&p).ok();
        assert_eq!(m.as_slice(), &content[..]);
    }

    /// A backing handle keeps the bytes alive after the `FileMap` is dropped,
    /// which is exactly what `SectionData` relies on.
    #[test]
    fn backing_outlives_the_filemap() {
        let content: Vec<u8> = (0..1024u32).map(|i| (i % 97) as u8).collect();
        let p = write_file("backing", &content);
        let backing = {
            let m = FileMap::open(p.to_str().unwrap()).unwrap();
            m.backing()
        }; // FileMap dropped here
        assert_eq!(backing.as_slice(), &content[..]);
        std::fs::remove_file(&p).ok();
    }

    /// A zero-length file cannot be mapped; the fallback must still produce a
    /// valid (empty) slice rather than an error.
    #[test]
    fn empty_file_falls_back_to_read() {
        let p = write_file("empty", b"");
        let m = FileMap::open(p.to_str().unwrap()).unwrap();
        assert!(m.is_empty());
        assert!(!m.is_mapped(), "a zero-length file must not be mapped");
        std::fs::remove_file(&p).ok();
    }

    /// The fallback must yield byte-identical content to the mapping, or the
    /// LCCC_NO_MMAP escape hatch would change link output.
    #[test]
    fn fallback_matches_mapped_bytes() {
        let content: Vec<u8> = (0..8192u32).map(|i| (i.wrapping_mul(7) % 253) as u8).collect();
        let p = write_file("ab", &content);
        let mapped = FileMap::open(p.to_str().unwrap()).unwrap();
        let owned = FileMap {
            inner: FileMapInner::Owned(std::fs::read(&p).unwrap().into()),
        };
        assert_eq!(mapped.as_slice(), owned.as_slice());
        assert!(mapped.is_mapped() && !owned.is_mapped());
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn missing_file_is_an_error_not_a_panic() {
        let p = tmp_path("nope");
        let e = match FileMap::open(p.to_str().unwrap()) {
            Ok(_) => panic!("opening a nonexistent path must fail"),
            Err(e) => e,
        };
        assert!(e.contains("failed to read"), "got {e}");
    }
}
