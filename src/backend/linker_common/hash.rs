//! ELF hash functions for `.gnu.hash` and `.hash` section generation.
//!
//! Provides GNU and SysV hash computations used by linkers when building
//! the dynamic symbol hash tables.

/// Compute the GNU hash of a symbol name.
pub fn gnu_hash(name: &[u8]) -> u32 {
    let mut h: u32 = 5381;
    for &b in name {
        h = h.wrapping_mul(33).wrapping_add(b as u32);
    }
    h
}

/// Compute the SysV ELF hash of a symbol name.
pub fn sysv_hash(name: &[u8]) -> u32 {
    let mut h: u32 = 0;
    for &b in name {
        h = (h << 4).wrapping_add(b as u32);
        let g = h & 0xf0000000;
        if g != 0 {
            h ^= g >> 24;
        }
        h &= !g;
    }
    h
}

/// A built SysV (`.hash` / `SHT_HASH`) table.
///
/// Shared by the executable and shared-object emitters.  It used to be spelled
/// out inline in each, and the two copies are exactly the kind of thing that
/// drifts: the format is small enough that a mistake still produces an image
/// that loads, it just resolves the wrong symbol or fails a lookup that a
/// loader without `.gnu.hash` support has to fall back to.
///
/// Unlike the GNU table this indexes `.dynsym` directly, so it needs no
/// reordering of the symbol table and can be built from the final dynsym order.
pub struct SysvHash {
    pub nbucket: u32,
    /// Number of `.dynsym` entries *including* the NULL symbol at index 0;
    /// `chain[]` is indexed by symbol index, so the ABI fixes this value.
    pub nchain: u32,
    pub buckets: Vec<u32>,
    pub chains: Vec<u32>,
}

impl SysvHash {
    /// Byte size of the on-disk table: header + buckets + chains.
    pub fn size(&self) -> u64 {
        8 + self.nbucket as u64 * 4 + self.nchain as u64 * 4
    }
}

/// Build a SysV hash table over `names`, which must be the exported symbol
/// names in final `.dynsym` order *excluding* the NULL entry at index 0.
///
/// `nbucket` is at the linker's discretion; one bucket per chain slot gives
/// O(1) expected chain length and keeps the table a simple function of the
/// symbol count rather than a tuned constant.
pub fn build_sysv_hash(names: &[&str]) -> SysvHash {
    let nchain = (1 + names.len()) as u32;
    let nbucket = nchain.max(1);
    let mut buckets = vec![0u32; nbucket as usize];
    let mut chains = vec![0u32; nchain as usize];
    for (i, name) in names.iter().enumerate() {
        let symidx = (i + 1) as u32; // +1: index 0 is the NULL symbol
        let b = (sysv_hash(name.as_bytes()) % nbucket) as usize;
        chains[symidx as usize] = buckets[b];
        buckets[b] = symidx;
    }
    SysvHash {
        nbucket,
        nchain,
        buckets,
        chains,
    }
}

/// Sizing parameters for a `.gnu.hash` (`SHT_GNU_HASH`) table.
///
/// Evidence base (measured on Debian oracles, `-shared` links with N
/// exported symbols, `scripts` TSV in the commit message history):
///
/// ```text
/// nsyms  bfd(nbuckets/bloom/shift)  lld             mold
///    64        37 /  8 /  9         16 / 16 / 26     9 / 16 / 26
///  1024       521 /128 / 13        256 /256 / 26   129 /256 / 26
/// 20000     16411 /2048/ 17       5000 /4096/ 26  2501 /4096/ 26
/// ```
///
/// lld's and mold's sizing strictly dominates bfd's at equal table bytes:
/// the bloom filter carries the false-positive load (measured FPR ~0.4-0.6%
/// vs bfd ~1-2%), the bucket array only shortens chains.  Previous lccc
/// sizing (1 bloom word of 64 bits at shift 6, `next_pow2(n)` buckets) was
/// strictly worse than all three on both axes: at 20 000 symbols its bloom
/// saturated (FPR ~1.0, every miss falls back to a chain walk) while its
/// bucket array alone was as large as lld's entire table.
///
/// Rules implemented here:
/// * `bloom_size = max(1, next_pow2(ceil(n/4)))` words — a power of two is
///   **mandatory**: glibc derives the word-index mask as `bloom_size - 1`
///   (dl-lookup.c), so a non-power-of-two word count silently reads the
///   wrong word and every lookup would be wrong.
/// * `bloom_shift = log2(bloom_bits)` where `bloom_bits = bloom_size *
///   class_bits` — the second probe bit then hashes from a bit-window of
///   `h` disjoint from both probe-1's low bits and the word-index window.
/// * `nbuckets = max(1, n/4)` (lld parity: expected chain <= 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GnuHashParams {
    pub nbuckets: u32,
    pub bloom_size: u32, // in class words (power of two, >= 1)
    pub bloom_shift: u32,
}

/// Compute the `.gnu.hash` sizing for `num_hashed` hashed symbols on an
/// ELF class with `class_bits`-bit bloom words (64 for ELFCLASS64, 32 for
/// ELFCLASS32).  `class_bits` must be 32 or 64.
pub fn gnu_hash_params(num_hashed: usize, class_bits: u32) -> GnuHashParams {
    debug_assert!(class_bits == 32 || class_bits == 64);
    if num_hashed == 0 {
        return GnuHashParams {
            nbuckets: 1,
            bloom_size: 1,
            bloom_shift: class_bits.trailing_zeros(),
        };
    }
    let n = num_hashed as u32;
    let bloom_size = n.div_ceil(4).next_power_of_two().max(1);
    let bloom_bits = bloom_size * class_bits;
    GnuHashParams {
        nbuckets: (n / 4).max(1),
        bloom_size,
        bloom_shift: bloom_bits.trailing_zeros(),
    }
}

/// Build the bloom filter words for `hashes` under `params`.
///
/// Returns `bloom_size` words, each `class_bits` wide, stored as `u64`
/// (callers on 32-bit classes write the low 32 bits).  The probe bits are:
///
/// ```text
/// word  = bloom[(h / class_bits)  % bloom_size]
/// bit1  = h % class_bits
/// bit2  = (h >> bloom_shift) % class_bits
/// ```
///
/// matching glibc's `dl-lookup.c` walk exactly.  Insertion is linear in
/// the number of hashed symbols with no temporary allocation beyond the
/// word vector itself (the previous single-word version collapsed all
/// probes into 64 bits and saturated for >~30 exported symbols).
pub fn build_gnu_bloom(hashes: &[u32], params: &GnuHashParams, class_bits: u32) -> Vec<u64> {
    debug_assert!(class_bits == 32 || class_bits == 64);
    let words = params.bloom_size as usize;
    let class_mask = (class_bits - 1) as u64; // class_bits is a power of two
    let mut bloom = vec![0u64; words.max(1)];
    if words == 0 {
        return bloom;
    }
    let word_mask = (words - 1) as u64; // bloom_size is a power of two
    for &h in hashes {
        let h = h as u64;
        let w = ((h / class_bits as u64) & word_mask) as usize;
        let bit1 = h & class_mask;
        let bit2 = (h >> params.bloom_shift) & class_mask;
        bloom[w] |= (1u64 << bit1) | (1u64 << bit2);
        // Shift-check only the 32-bit class: `bloom[w] >> 64` overflows the
        // word width and panics in debug builds (the 64-bit class trivially
        // fits in the word — nothing above bit 63 exists to check).
        if class_bits == 32 {
            debug_assert_eq!(bloom[w] >> 32, 0, "bits must stay in class width");
        }
    }
    bloom
}

/// Write a SysV hash table at `off` in `out`.
pub fn write_sysv_hash(out: &mut [u8], off: usize, h: &SysvHash) {
    fn w32(o: &mut [u8], p: usize, v: u32) {
        o[p..p + 4].copy_from_slice(&v.to_le_bytes());
    }
    w32(out, off, h.nbucket);
    w32(out, off + 4, h.nchain);
    let sb = off + 8;
    for (i, &b) in h.buckets.iter().enumerate() {
        w32(out, sb + i * 4, b);
    }
    let sc = sb + h.nbucket as usize * 4;
    for (i, &c) in h.chains.iter().enumerate() {
        w32(out, sc + i * 4, c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ground truth captured from the Debian oracles (`-shared` links with N
    /// exported `{s0..sN}` symbols; see the `GnuHashParams` doc comment).
    /// lld's choices are the parity target; mold uses the same bloom sizing.
    #[test]
    fn gnu_hash_params_match_lld_sizing() {
        // (nsyms, lld_nbuckets, lld_bloom_words)
        for (n, want_buckets, want_bloom) in [
            (64usize, 16u32, 16u32),
            (512, 128, 128),
            (1024, 256, 256),
            (4096, 1024, 1024),
        ] {
            let p = gnu_hash_params(n, 64);
            assert_eq!(p.nbuckets, want_buckets, "nbuckets for n={n}");
            assert_eq!(p.bloom_size, want_bloom, "bloom for n={n}");
            assert_eq!(
                p.bloom_shift,
                (p.bloom_size * 64).trailing_zeros(),
                "shift must be log2(bloom_bits) for n={n}"
            );
        }
    }

    /// glibc derives the word-index mask as bloom_size - 1; a word count
    /// that is not a power of two silently corrupts every lookup.
    #[test]
    fn bloom_word_count_is_always_a_power_of_two() {
        for n in 0..600usize {
            let p = gnu_hash_params(n, 64);
            assert!(p.bloom_size.is_power_of_two(), "n={n}");
            assert!(p.bloom_size >= 1, "n={n}");
            assert!(p.nbuckets >= 1, "n={n}");
            let p32 = gnu_hash_params(n, 32);
            assert!(p32.bloom_size.is_power_of_two(), "32-bit n={n}");
        }
    }

    /// The bloom probe formulas must match glibc's dl-lookup.c semantics:
    /// a symbol inserted at build time must pass the two-bit filter at
    /// lookup time, in the exact word glibc would select.
    #[test]
    fn bloom_passes_every_inserted_symbol() {
        let names: Vec<String> = (0..1000).map(|i| format!("sym_{i}")).collect();
        let hashes: Vec<u32> = names.iter().map(|s| gnu_hash(s.as_bytes())).collect();
        for class_bits in [32u32, 64] {
            let p = gnu_hash_params(names.len(), class_bits);
            let bloom = build_gnu_bloom(&hashes, &p, class_bits);
            assert_eq!(bloom.len(), p.bloom_size as usize);
            let class_mask = (class_bits - 1) as u64;
            for &h in &hashes {
                let h = h as u64;
                let w = ((h / class_bits as u64) & (p.bloom_size as u64 - 1)) as usize;
                let bit1 = h & class_mask;
                let bit2 = (h >> p.bloom_shift) & class_mask;
                let word = bloom[w];
                assert!(
                    word & (1u64 << bit1) != 0 && word & (1u64 << bit2) != 0,
                    "miss for h={h:#x} on {class_bits}-bit class"
                );
            }
        }
    }

    /// The previous single-word implementation saturated: beyond ~30
    /// symbols almost every miss passed the filter.  The sized filter must
    /// keep the observable false-positive rate in the single-digit percent
    /// range at glibc scale (5 000 symbols).
    #[test]
    fn bloom_false_positive_rate_stays_low() {
        let hashes: Vec<u32> = (0..5000)
            .map(|i| gnu_hash(format!("g_sym_{i}").as_bytes()))
            .collect();
        let p = gnu_hash_params(hashes.len(), 64);
        let bloom = build_gnu_bloom(&hashes, &p, 64);
        let probe = |h: u32| {
            let h = h as u64;
            let w = ((h / 64) & (p.bloom_size as u64 - 1)) as usize;
            let word = bloom[w];
            word & (1u64 << (h & 63)) != 0 && word & (1u64 << ((h >> p.bloom_shift) & 63)) != 0
        };
        let misses = (0..20000u32)
            .filter(|&i| {
                let h = gnu_hash(format!("absent_{i}").as_bytes());
                !hashes.contains(&h) && probe(h)
            })
            .count();
        let fpr = misses as f64 / 20000.0;
        assert!(fpr < 0.05, "FPR {fpr} too high at 5 000 symbols");
    }
}
