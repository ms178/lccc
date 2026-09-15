//! GNU hash table building for ELF32.
//!
//! Builds the `.gnu.hash` section for dynamically-linked i686 executables.
//! Uses 32-bit bloom filter words (ELF32 word size) and the GNU hash algorithm.

use crate::backend::linker_common;

/// Build a .gnu.hash section for ELF32.
///
/// `hashed_names`: names of symbols that go into the hash (at indices >= symoffset)
/// `symoffset`: first hashed symbol's index in .dynsym
///
/// Returns `(hash_data, sorted_indices)` where `sorted_indices` maps new position
/// to original index in `hashed_names`, so the caller can reorder dynsym entries.
pub(super) fn build_gnu_hash_32(hashed_names: &[String], symoffset: u32) -> (Vec<u8>, Vec<usize>) {
    let num_hashed = hashed_names.len();
    // Shared oracle-measured sizing (see linker_common/hash.rs): bloom words
    // = next_pow2(ceil(n/4)) — a hard glibc requirement is the power of two
    // (word-index mask = size-1) — shift = log2(bloom_bits), buckets = n/4.
    // The previous single 32-bit word at shift 5 saturated beyond ~30
    // exports and probed from a bit window overlapping the word-index bits;
    // at 20 000 symbols its lookup was a guaranteed chain walk per miss.
    let params = linker_common::gnu_hash_params(num_hashed, 32);
    let nbuckets = params.nbuckets;
    let bloom_size = params.bloom_size;
    let bloom_shift = params.bloom_shift;

    // Compute hashes
    let orig_hashes: Vec<u32> = hashed_names
        .iter()
        .map(|name| linker_common::gnu_hash(name.as_bytes()))
        .collect();

    // Sort by bucket for proper chain grouping (stable: deterministic output)
    let mut indices: Vec<usize> = (0..num_hashed).collect();
    indices.sort_by_key(|&i| orig_hashes[i] % nbuckets);
    let sym_hashes: Vec<u32> = indices.iter().map(|&i| orig_hashes[i]).collect();

    // Bloom filter: `bloom_size` 32-bit words (ELF32 class width).
    let bloom_words64 = linker_common::build_gnu_bloom(&sym_hashes, &params, 32);

    // Build buckets and chains
    let mut buckets = vec![0u32; nbuckets as usize];
    let mut chains = vec![0u32; num_hashed];
    for (i, &h) in sym_hashes.iter().enumerate() {
        let bucket = (h % nbuckets) as usize;
        if buckets[bucket] == 0 {
            buckets[bucket] = symoffset + i as u32;
        }
        chains[i] = h & !1;
    }

    // Mark the last symbol of each bucket chain with bit 0.  Symbols are
    // bucket-sorted, so entry i ends its chain exactly when the next entry
    // hashes into a different bucket — one linear pass.  (The previous
    // per-bucket rescan was O(buckets * symbols), quadratic at glibc scale.)
    for i in 0..sym_hashes.len() {
        let last = i + 1 == sym_hashes.len()
            || (sym_hashes[i + 1] % nbuckets) != (sym_hashes[i] % nbuckets);
        if last {
            chains[i] |= 1;
        }
    }

    // Serialize
    let mut data =
        Vec::with_capacity(16 + bloom_size as usize * 4 + nbuckets as usize * 4 + num_hashed * 4);
    data.extend_from_slice(&nbuckets.to_le_bytes());
    data.extend_from_slice(&symoffset.to_le_bytes());
    data.extend_from_slice(&bloom_size.to_le_bytes());
    data.extend_from_slice(&bloom_shift.to_le_bytes());
    for &w in &bloom_words64 {
        debug_assert_eq!(w >> 32, 0, "32-bit class bloom word overflow");
        data.extend_from_slice(&(w as u32).to_le_bytes());
    }
    for &b in &buckets {
        data.extend_from_slice(&b.to_le_bytes());
    }
    for &c in &chains {
        data.extend_from_slice(&c.to_le_bytes());
    }

    (data, indices)
}
