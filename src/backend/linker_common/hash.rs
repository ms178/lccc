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
