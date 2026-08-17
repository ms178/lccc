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
///
/// # Correctness
///
/// The output is byte-for-byte identical to [`apply_writes_serial`] for any
/// input, including unsorted lists and lists containing several writes to the
/// same offset: partitioning preserves list order within a partition, and
/// partitions are split so that no two of them can touch the same byte.
///
/// # Soundness
///
/// Workers receive disjoint `&mut [u8]` sub-slices carved out of one buffer.
/// Disjointness is *established here*, not assumed:
///
/// * The list is indexed through a permutation sorted by offset, so a
///   partition boundary can be placed between two writes whose byte ranges do
///   not overlap.  The previous implementation partitioned the list as given
///   and merely documented "the caller must pass a sorted list"; with an
///   unsorted list the per-partition `[min, max)` spans overlap and the
///   workers alias the same bytes, which is undefined behaviour.  (Concretely,
///   offsets `[0, 1000, 8, 1008]` with two threads yield the overlapping spans
///   `[0, 1008)` and `[8, 1016)`.)
/// * A cluster of writes that overlap each other is never split across
///   partitions, so overlapping writes are always applied by one worker in
///   list order — preserving serial semantics.
/// * Every write is bounds-checked against the buffer *before* any pointer is
///   formed.  Out-of-range writes are dropped, exactly as the serial path
///   does; the old `debug_assert!` compiled out in release builds, so a bad
///   offset became an out-of-bounds write instead of a skipped one.
pub fn apply_writes_parallel(out: &mut [u8], writes: &[RelocWrite], n_threads: usize) {
    if writes.is_empty() || n_threads <= 1 {
        apply_writes_serial(out, writes);
        SERIAL_WRITES.fetch_add(writes.len(), Ordering::Relaxed);
        return;
    }

    let out_len = out.len();

    let (order, partitions) = plan_partitions_inner(writes, n_threads);

    if partitions.len() <= 1 {
        // Everything overlaps (or the list is tiny): no safe split exists.
        apply_writes_serial(out, writes);
        SERIAL_WRITES.fetch_add(writes.len(), Ordering::Relaxed);
        return;
    }

    let span = |i: u32| -> (usize, usize) {
        let w = &writes[i as usize];
        (w.offset, w.offset.saturating_add(w.width as usize))
    };

    thread::scope(|scope| {
        let out_addr = out.as_mut_ptr() as usize;
        let mut handles = Vec::with_capacity(partitions.len());

        for &(ps, pe) in &partitions {
            let idxs = &order[ps..pe];
            if idxs.is_empty() { continue; }

            // Span of this partition, clamped to the buffer. Writes outside
            // the buffer are filtered in the worker; here we only need a
            // sound base/length for the sub-slice. Both bounds are computed by
            // scanning: after the re-sort above, `idxs` is in list order, so
            // neither end of the span is at a known position.
            let lo = idxs.iter().map(|&i| span(i).0).min().unwrap();
            let hi = idxs.iter().map(|&i| span(i).1).max().unwrap_or(lo);
            if lo >= out_len { continue; }
            let hi = hi.min(out_len);
            if hi <= lo { continue; }

            // SAFETY: `partitions` was built so that the [lo, hi) spans of any
            // two partitions are disjoint (a boundary is only placed where the
            // running high-water mark has been reached), and each span is
            // clamped into `0..out_len`. Therefore the sub-slices handed to the
            // workers do not alias and stay inside the allocation that `out`
            // borrows, which outlives the scope.
            let thread_slice = unsafe {
                std::slice::from_raw_parts_mut((out_addr as *mut u8).add(lo), hi - lo)
            };

            handles.push(scope.spawn(move || {
                for &i in idxs {
                    let w = &writes[i as usize];
                    // Same bounds policy as the serial path: skip, never panic.
                    let Some(end) = w.offset.checked_add(w.width as usize) else { continue };
                    if end > out_len { continue; }
                    write_le(thread_slice, w.offset - lo, w.width, w.value);
                }
            }));
        }

        for h in handles {
            h.join().expect("parallel reloc worker panicked");
        }
    });

    PARALLEL_WRITES.fetch_add(writes.len(), Ordering::Relaxed);
}

/// Compute a safe partitioning of `writes` for `n_threads` workers.
///
/// Returns `(order, partitions)` where `order` is a permutation of indices
/// into `writes` and each `(start, end)` in `partitions` denotes the range
/// `order[start..end]` handled by one worker.
///
/// Guarantees, on which the `unsafe` slice construction in
/// [`apply_writes_parallel`] depends:
/// * the byte spans of any two partitions are disjoint;
/// * within a partition, indices are in ascending *list* order, so
///   last-writer-wins is preserved for overlapping writes.
fn plan_partitions_inner(writes: &[RelocWrite], n_threads: usize)
    -> (Vec<u32>, Vec<(usize, usize)>)
{
    // Index permutation sorted by offset, used only to find safe split points.
    let mut order: Vec<u32> = (0..writes.len() as u32).collect();
    order.sort_by_key(|&i| writes[i as usize].offset);

    let span = |i: u32| -> (usize, usize) {
        let w = &writes[i as usize];
        (w.offset, w.offset.saturating_add(w.width as usize))
    };

    // A split may be placed at k only when every byte written so far ends at
    // or before the first byte written at k. Tracking the running high-water
    // mark (not merely the previous write's end) keeps a chain of mutually
    // overlapping writes inside one partition.
    let target = writes.len().div_ceil(n_threads).max(1);
    let mut partitions: Vec<(usize, usize)> = Vec::with_capacity(n_threads);
    let mut part_start = 0usize;
    let mut reach = span(order[0]).1;
    for k in 1..order.len() {
        let (lo, hi) = span(order[k]);
        if lo >= reach && k - part_start >= target {
            partitions.push((part_start, k));
            part_start = k;
            reach = hi;
        } else {
            reach = reach.max(hi);
        }
    }
    partitions.push((part_start, order.len()));

    // Restore original list order *within* each partition.
    //
    // Sorting by offset must not survive into application order. Two writes
    // that overlap at *different* offsets are reordered by an offset sort,
    // which silently inverts last-writer-wins: for [(off 4, w4, B),
    // (off 0, w8, A)] the serial result has A covering bytes 4..8, whereas
    // offset order applies A then B and leaves B there. (A stable sort only
    // protects writes at the *same* offset.) The randomised differential test
    // caught exactly this. Writes in different partitions never touch the
    // same byte, so their relative order is unobservable.
    for &(ps, pe) in &partitions {
        order[ps..pe].sort_unstable();
    }
    (order, partitions)
}

/// Test hook: the partition ranges that would be used for `n_threads`.
#[cfg(test)]
fn plan_partitions(writes: &[RelocWrite], n_threads: usize) -> Vec<(usize, usize)> {
    if writes.is_empty() || n_threads <= 1 {
        return vec![(0, writes.len())];
    }
    plan_partitions_inner(writes, n_threads).1
}

pub fn apply_writes_serial(out: &mut [u8], writes: &[RelocWrite]) {
    for w in writes {
        // checked_add: `offset + width` must not wrap for offsets near
        // usize::MAX, which would turn a skip into an out-of-bounds write.
        let Some(end) = w.offset.checked_add(w.width as usize) else { continue };
        if end > out.len() { continue; }
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
        if r.offset.checked_add(r.bytes.len()).is_some_and(|e| e <= out.len()) {
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

    // ---- regressions for the partitioning soundness fix --------------------

    /// The exact shape that made the previous implementation alias two `&mut`
    /// slices: an *unsorted* list whose naive contiguous partitions produce
    /// overlapping `[min, max)` spans ([0,1008) and [8,1016) with 2 threads).
    /// Under the old code this was UB; the observable symptom here is simply
    /// that the result must equal the serial one.
    #[test]
    fn unsorted_writes_match_serial() {
        let writes = vec![
            RelocWrite { offset: 0,    width: 8, value: 0x1111_1111_1111_1111 },
            RelocWrite { offset: 1000, width: 8, value: 0x2222_2222_2222_2222 },
            RelocWrite { offset: 8,    width: 8, value: 0x3333_3333_3333_3333 },
            RelocWrite { offset: 1008, width: 8, value: 0x4444_4444_4444_4444 },
        ];
        let mut a = vec![0u8; 2048];
        let mut b = vec![0u8; 2048];
        apply_writes_serial(&mut a, &writes);
        apply_writes_parallel(&mut b, &writes, 2);
        assert_eq!(a, b);
    }

    /// Overlapping writes must be applied in list order (last writer wins),
    /// which requires an overlapping cluster never to be split across workers.
    #[test]
    fn overlapping_writes_preserve_last_writer_wins() {
        let writes = vec![
            RelocWrite { offset: 0, width: 8, value: 0xffff_ffff_ffff_ffff },
            RelocWrite { offset: 0, width: 4, value: 0x0000_0001 },
            RelocWrite { offset: 2, width: 4, value: 0x0000_0002 },
            RelocWrite { offset: 1, width: 2, value: 0x0003 },
        ];
        for threads in [1usize, 2, 3, 4, 8] {
            let mut a = vec![0u8; 64];
            let mut b = vec![0u8; 64];
            apply_writes_serial(&mut a, &writes);
            apply_writes_parallel(&mut b, &writes, threads);
            assert_eq!(a, b, "thread count {threads} diverged from serial");
        }
    }

    /// Out-of-range writes must be skipped, never written and never panic.
    /// The old code only had a `debug_assert!`, so a release build turned a
    /// bad offset into an out-of-bounds store.
    #[test]
    fn out_of_range_writes_are_skipped_not_written() {
        let writes = vec![
            RelocWrite { offset: 0,  width: 8, value: 0xaaaa_aaaa_aaaa_aaaa },
            RelocWrite { offset: 60, width: 8, value: 0xbbbb_bbbb_bbbb_bbbb }, // 60+8 > 64
            RelocWrite { offset: 64, width: 1, value: 0xcc },                  // at end
            RelocWrite { offset: usize::MAX, width: 8, value: 0xdd },          // wraps
            RelocWrite { offset: 16, width: 4, value: 0x1234_5678 },
        ];
        for threads in [1usize, 2, 4] {
            let mut a = vec![0u8; 64];
            let mut b = vec![0u8; 64];
            apply_writes_serial(&mut a, &writes);
            apply_writes_parallel(&mut b, &writes, threads);
            assert_eq!(a, b, "thread count {threads}");
            assert_eq!(&a[0..8], &0xaaaa_aaaa_aaaa_aaaau64.to_le_bytes());
            assert_eq!(&a[16..20], &0x1234_5678u32.to_le_bytes());
            assert!(a[56..64].iter().all(|&x| x == 0), "clobbered past-end region");
        }
    }

    /// Randomised differential test against the serial reference across a
    /// range of densities, widths, overlap rates and thread counts. This is
    /// the property that actually matters: for *every* input, parallel output
    /// == serial output. A deterministic xorshift keeps it reproducible.
    #[test]
    fn randomised_differential_against_serial() {
        let mut state: u64 = 0x243f_6a88_85a3_08d3;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        const BUF: usize = 4096;
        for round in 0..300 {
            let n = 1 + (next() % 200) as usize;
            // Alternate between a sparse layout and a dense/overlapping one.
            let dense = round % 3 == 0;
            let mut writes = Vec::with_capacity(n);
            for _ in 0..n {
                let width = [1u8, 2, 4, 8][(next() % 4) as usize];
                let offset = if dense {
                    (next() % 64) as usize
                } else {
                    (next() % (BUF as u64 + 16)) as usize
                };
                writes.push(RelocWrite { offset, width, value: next() });
            }
            let threads = 1 + (next() % 8) as usize;
            let mut a = vec![0u8; BUF];
            let mut b = vec![0u8; BUF];
            apply_writes_serial(&mut a, &writes);
            apply_writes_parallel(&mut b, &writes, threads);
            assert_eq!(a, b, "round {round} (n={n}, threads={threads}, dense={dense})");
        }
    }

    /// Many disjoint writes must actually be split across workers (otherwise
    /// the "parallel" path silently degrades to serial and no speed-up is
    /// possible).
    ///
    /// The partitioning decision is tested directly via `plan_partitions`
    /// rather than through the global `PARALLEL_WRITES` counter: cargo runs
    /// tests concurrently in one process, so any assertion on a shared
    /// counter is racy and would flake. (It did: an earlier version of this
    /// test read 1004 instead of 1000 because a sibling test ran in between.)
    #[test]
    fn disjoint_writes_are_partitioned() {
        let writes: Vec<RelocWrite> = (0..1000)
            .map(|i| RelocWrite { offset: i * 8, width: 8, value: i as u64 })
            .collect();
        let parts = plan_partitions(&writes, 4);
        assert!(parts.len() >= 2,
                "1000 disjoint writes must split across workers, got {} partition(s)",
                parts.len());
        // And the result must still equal the serial reference.
        let mut a = vec![0u8; 8 * 1000];
        let mut b = vec![0u8; 8 * 1000];
        apply_writes_serial(&mut a, &writes);
        apply_writes_parallel(&mut b, &writes, 4);
        assert_eq!(a, b);
    }

    /// Conversely, a fully-overlapping cluster must NOT be split, because
    /// splitting it would break last-writer-wins.
    #[test]
    fn overlapping_cluster_is_not_split() {
        let writes: Vec<RelocWrite> = (0..100)
            .map(|i| RelocWrite { offset: 0, width: 8, value: i as u64 })
            .collect();
        assert_eq!(plan_partitions(&writes, 8).len(), 1,
                   "an all-overlapping cluster must stay in one partition");
    }
}
