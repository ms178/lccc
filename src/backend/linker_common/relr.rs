//! ELF `SHT_RELR` / `DT_RELR` compact relative-relocation encoding (`-z pack-relative-relocs`).
//!
//! Per the ELF gABI `RELR` specification:
//! - Each entry in `.relr.dyn` is an `Elf64_Relr` (`u64`) on ELFCLASS64 or
//!   `Elf32_Relr` (`u32`) on ELFCLASS32.
//! - An **even** entry (`entry & 1 == 0`) encodes a relocation at `entry` and
//!   sets the bitmap cursor `where = entry + WORD_SIZE`.
//! - An **odd** entry (`entry & 1 == 1`) is a `(WORD_BITS - 1)`-bit bitmap:
//!   for each bit `k` in `1..WORD_BITS`, if `(entry >> k) & 1 != 0`, a relative
//!   relocation applies at `where + (k - 1) * WORD_SIZE`; after processing the
//!   word, `where += (WORD_BITS - 1) * WORD_SIZE`.
//! - Every location covered by `RELR` carries its implicit link-time addend in
//!   the target word itself (`*p += load_base`).
//!
//! Unaligned relative relocations (`addr % WORD_SIZE != 0`, e.g. pointers inside
//! `__attribute__((packed))` structs) cannot be represented in `RELR` without
//! unaligned RMWs or odd address words, so [`partition_relative_64`] keeps them
//! in `.rela.dyn` while packing all 8-byte-aligned entries into `.relr.dyn`.

/// Partition `(vaddr, addend)` relative relocation pairs into:
/// - `relr_addrs`: sorted, deduplicated 8-byte-aligned addresses for `.relr.dyn`,
/// - `rela_leftovers`: unaligned `(vaddr, addend)` pairs that must stay in `.rela.dyn`.
pub fn partition_relative_64(entries: &[(u64, u64)]) -> (Vec<u64>, Vec<(u64, u64)>) {
    let mut relr_addrs = Vec::with_capacity(entries.len());
    let mut rela_leftovers = Vec::new();
    for &(addr, addend) in entries {
        if addr & 7 == 0 {
            relr_addrs.push(addr);
        } else {
            rela_leftovers.push((addr, addend));
        }
    }
    relr_addrs.sort_unstable();
    relr_addrs.dedup();
    (relr_addrs, rela_leftovers)
}

/// Encode sorted, deduplicated 8-byte-aligned addresses into `Elf64_Relr` words,
/// serialized as little-endian bytes for `.relr.dyn`.
///
/// Provably minimal word count: a continuation bitmap word covers 63 consecutive
/// 8-byte slots (`504` bytes). Emitting an all-zero bitmap word (`0x1`) to bridge
/// an empty 504-byte gap costs 1 word for 0 relocations, whereas starting a fresh
/// address word at the next relocation costs 1 word, encodes 1 relocation, and
/// resets `where` further right — strictly dominating any zero-bitmap bridge.
pub fn encode_relr_64_bytes(sorted_aligned_addrs: &[u64]) -> Vec<u8> {
    let words = encode_relr_64_words(sorted_aligned_addrs);
    let mut bytes = Vec::with_capacity(words.len() * 8);
    for w in words {
        bytes.extend_from_slice(&w.to_le_bytes());
    }
    bytes
}

/// Encode sorted, deduplicated 8-byte-aligned addresses into `Elf64_Relr` words.
pub fn encode_relr_64_words(addrs: &[u64]) -> Vec<u64> {
    const WORD_SIZE: u64 = 8;
    const BITMAP_SLOTS: u64 = 63;
    const WINDOW_BYTES: u64 = WORD_SIZE * BITMAP_SLOTS; // 504 bytes

    let mut out = Vec::new();
    let mut i = 0;
    while i < addrs.len() {
        let addr = addrs[i];
        debug_assert_eq!(addr & 1, 0, "RELR address word must be even");
        out.push(addr);
        i += 1;
        let mut cursor = addr + WORD_SIZE;

        while i < addrs.len() {
            let delta = addrs[i].saturating_sub(cursor);
            if delta >= WINDOW_BYTES || delta % WORD_SIZE != 0 {
                break;
            }
            let mut bitmap: u64 = 1; // bit 0 marks a bitmap entry
            while i < addrs.len() {
                let d = addrs[i].saturating_sub(cursor);
                if d >= WINDOW_BYTES || d % WORD_SIZE != 0 {
                    break;
                }
                let bit = (d / WORD_SIZE) + 1;
                bitmap |= 1u64 << bit;
                i += 1;
            }
            out.push(bitmap);
            cursor += WINDOW_BYTES;
        }
    }
    out
}

/// Decode `Elf64_Relr` words back into the exact sequence of relocated virtual
/// addresses (matching glibc `elf_machine_lazy_rel` / `_dl_relocate_object`).
#[cfg(test)]
pub fn decode_relr_64_words(words: &[u64]) -> Vec<u64> {
    let mut out = Vec::new();
    let mut cursor = 0u64;
    for &w in words {
        if w & 1 == 0 {
            out.push(w);
            cursor = w + 8;
        } else {
            for bit in 1..64u64 {
                if (w >> bit) & 1 != 0 {
                    out.push(cursor + (bit - 1) * 8);
                }
            }
            cursor += 63 * 8;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_and_singleton_roundtrip() {
        assert!(encode_relr_64_words(&[]).is_empty());
        let w = encode_relr_64_words(&[0x4000]);
        assert_eq!(w, vec![0x4000]);
        assert_eq!(decode_relr_64_words(&w), vec![0x4000]);
    }

    #[test]
    fn dense_64_and_65_slots_pack_to_2_and_3_words() {
        // 1 base + 63 bitmap slots = 64 relocations in just 2 words (16 bytes vs 1536 bytes in RELA!).
        let addrs64: Vec<u64> = (0..64).map(|k| 0x4000 + k * 8).collect();
        let w64 = encode_relr_64_words(&addrs64);
        assert_eq!(w64.len(), 2);
        assert_eq!(w64[0], 0x4000);
        assert_eq!(w64[1], u64::MAX);
        assert_eq!(decode_relr_64_words(&w64), addrs64);

        // 65th contiguous slot spills into a second continuation bitmap word (3 words total).
        let addrs65: Vec<u64> = (0..65).map(|k| 0x4000 + k * 8).collect();
        let w65 = encode_relr_64_words(&addrs65);
        assert_eq!(w65.len(), 3);
        assert_eq!(w65[2], 0b11);
        assert_eq!(decode_relr_64_words(&w65), addrs65);
    }

    #[test]
    fn sparse_gaps_and_unaligned_partition() {
        let raw = vec![
            (0x3dc0, 0x1100),
            (0x3dc8, 0x1120),
            (0x4001, 0x2200), // packed unaligned: stays in RELA
            (0x5000, 0x3300), // > 504 bytes away: fresh RELR base word
            (0x5010, 0x3310), // +16 bytes: bit 2 of bitmap word
        ];
        let (relr, rela) = partition_relative_64(&raw);
        assert_eq!(rela, vec![(0x4001, 0x2200)]);
        let words = encode_relr_64_words(&relr);
        assert_eq!(words.len(), 4); // [0x3dc0, bitmap, 0x5000, bitmap]
        assert_eq!(
            decode_relr_64_words(&words),
            vec![0x3dc0, 0x3dc8, 0x5000, 0x5010]
        );
    }
}
