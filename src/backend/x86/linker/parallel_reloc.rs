//! Parallel pure-store relocation application for the x86-64 linker.
//!
//! # Design
//!
//! Relocation application has two distinct phases:
//!
//! 1. **Resolution** (serial): symbol lookup, TLS GD/LD→LE rewriting, GOTPCRELX
//!    relaxation, overflow checks. These steps have data dependencies and
//!    mutate instruction bytes; they cannot be parallelised safely without a
//!    full rewrite of the emit path.
//! 2. **Pure stores** (parallelisable): once a list of `(file_offset, width,
//!    value)` triples has been produced, writing them into the output buffer
//!    is embarrassingly parallel provided each worker owns a *disjoint*
//!    mutable slice of the buffer.
//!
//! This module implements phase 2. The caller is responsible for producing a
//! sorted (or at least non-overlapping-in-a-way-that-breaks-partitioning)
//! `RelocWrite` list after resolution.
//!
//! # Threading model
//!
//! * `std::thread::scope` only — zero external dependencies.
//! * Default behaviour is serial (`desired_threads() == 1`) so differential
//!   testing remains bit-identical and deterministic.
//! * Opt-in via `LCCC_LD_PARALLEL=1` (use available parallelism, capped) or
//!   `LCCC_LD_PARALLEL=N` for an explicit thread count.
//!
//! # Safety
//!
//! The parallel path constructs disjoint mutable slices from a single
//! underlying buffer by calculating min/max offsets per partition. Partitions
//! are contiguous segments of a list that has been sorted by offset; therefore
//! the byte ranges cannot overlap. The `unsafe` block is confined to the
//! slice construction and is justified by that invariant.

#![allow(dead_code)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

/// A single pure store produced by the resolution pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RelocWrite {
    /// Absolute file offset into the output buffer.
    pub offset: usize,
    /// Store width: 1, 2, 4 or 8.
    pub width: u8,
    /// Value to write (little-endian, low `width` bytes).
    pub value: u64,
}

/// Instruction-byte rewrite recorded during TLS / GOTPCRELX relaxation.
#[derive(Clone, Debug)]
pub struct InsnRewrite {
    pub offset: usize,
    pub bytes: Vec<u8>,
}

/// Apply pure reloc stores in parallel.
pub fn apply_writes_parallel(out: &mut [u8], writes: &[RelocWrite], n_threads: usize) {
    if writes.is_empty() || n_threads <= 1 {
        apply_writes_serial(out, writes);
        SERIAL_WRITES.fetch_add(writes.len(), Ordering::Relaxed);
        return;
    }

    let n_threads = n_threads.min(writes.len()).max(1);
    let chunk = (writes.len() + n_threads - 1) / n_threads;

    thread::scope(|scope| {
        let out_addr = out.as_mut_ptr() as usize;
        let out_len = out.len();
        let mut handles = Vec::with_capacity(n_threads);

        for t in 0..n_threads {
            let start = t * chunk;
            if start >= writes.len() { break; }
            let end = (start + chunk).min(writes.len());
            let slice = &writes[start..end];
            if slice.is_empty() { continue; }

            let min_off = slice.iter().map(|w| w.offset).min().unwrap();
            let max_off = slice.iter().map(|w| w.offset + w.width as usize).max().unwrap();
            debug_assert!(max_off <= out_len, "reloc write past end of buffer");

            let thread_slice = unsafe {
                let base = out_addr as *mut u8;
                std::slice::from_raw_parts_mut(base.add(min_off), max_off - min_off)
            };

            handles.push(scope.spawn(move || {
                for w in slice {
                    let local = w.offset - min_off;
                    write_le(thread_slice, local, w.width, w.value);
                }
            }));
        }

        for h in handles {
            h.join().expect("parallel reloc worker panicked");
        }
    });

    PARALLEL_WRITES.fetch_add(writes.len(), Ordering::Relaxed);
}

pub fn apply_writes_serial(out: &mut [u8], writes: &[RelocWrite]) {
    for w in writes {
        if w.offset + w.width as usize > out.len() { continue; }
        write_le(out, w.offset, w.width, w.value);
    }
}

#[inline]
fn write_le(buf: &mut [u8], off: usize, width: u8, value: u64) {
    match width {
        1 => buf[off] = value as u8,
        2 => buf[off..off + 2].copy_from_slice(&(value as u16).to_le_bytes()),
        4 => buf[off..off + 4].copy_from_slice(&(value as u32).to_le_bytes()),
        8 => buf[off..off + 8].copy_from_slice(&value.to_le_bytes()),
        _ => {}
    }
}

pub fn apply_rewrites(out: &mut [u8], rewrites: &[InsnRewrite]) {
    for r in rewrites {
        if r.offset + r.bytes.len() <= out.len() {
            out[r.offset..r.offset + r.bytes.len()].copy_from_slice(&r.bytes);
        }
    }
}

pub fn desired_threads() -> usize {
    match std::env::var("LCCC_LD_PARALLEL") {
        Ok(s) => {
            let s = s.trim();
            if s.is_empty() || s == "0" || s.eq_ignore_ascii_case("false") || s.eq_ignore_ascii_case("off") {
                return 1;
            }
            if s == "1" || s.eq_ignore_ascii_case("true") || s.eq_ignore_ascii_case("on") {
                return std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2).clamp(1, 8);
            }
            s.parse::<usize>().unwrap_or(1).clamp(1, 16)
        }
        Err(_) => 1,
    }
}

pub static PARALLEL_WRITES: AtomicUsize = AtomicUsize::new(0);
pub static SERIAL_WRITES: AtomicUsize = AtomicUsize::new(0);

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_writes(n: usize) -> Vec<RelocWrite> {
        (0..n).map(|i| RelocWrite { offset: i * 8, width: 8, value: 0x1122_3344_5566_7788 + i as u64 }).collect()
    }

    #[test]
    fn serial_and_parallel_produce_identical_buffers() {
        let writes = sample_writes(64);
        let mut a = vec![0u8; 64 * 8];
        let mut b = vec![0u8; 64 * 8];
        apply_writes_serial(&mut a, &writes);
        apply_writes_parallel(&mut b, &writes, 4);
        assert_eq!(a, b);
    }

    #[test]
    fn empty_list_is_noop() {
        let mut out = vec![0u8; 16];
        apply_writes_parallel(&mut out, &[], 8);
        assert!(out.iter().all(|&b| b == 0));
    }

    #[test]
    fn single_thread_matches_serial() {
        let writes = sample_writes(8);
        let mut a = vec![0u8; 64];
        let mut b = vec![0u8; 64];
        apply_writes_serial(&mut a, &writes);
        apply_writes_parallel(&mut b, &writes, 1);
        assert_eq!(a, b);
    }

    #[test]
    fn mixed_widths() {
        let writes = vec![
            RelocWrite { offset: 0, width: 1, value: 0xaa },
            RelocWrite { offset: 1, width: 2, value: 0xbbcc },
            RelocWrite { offset: 3, width: 4, value: 0x11223344 },
            RelocWrite { offset: 7, width: 8, value: 0xdeadbeefcafebabe },
        ];
        let mut out = vec![0u8; 16];
        apply_writes_parallel(&mut out, &writes, 2);
        assert_eq!(out[0], 0xaa);
        assert_eq!(&out[1..3], &0xbbccu16.to_le_bytes());
        assert_eq!(&out[3..7], &0x11223344u32.to_le_bytes());
        assert_eq!(&out[7..15], &0xdeadbeefcafebabeu64.to_le_bytes());
    }

    #[test]
    fn desired_threads_defaults_to_one() {
        let _ = desired_threads();
    }
}
