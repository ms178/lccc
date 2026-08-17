//! Identical Code Folding prototype.
#![allow(dead_code)]
use crate::common::fx_hash::FxHashMap;
use crate::backend::linker_common::Elf64Object;
use crate::backend::elf::{SHF_EXECINSTR, SHT_PROGBITS};

#[derive(Debug, Default)]
pub struct IcfResult {
    pub folded_sections: usize,
    pub redirected_symbols: usize,
    pub rewritten_relocs: usize,
    pub bytes_saved: u64,
}

fn fnv1a64(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in data { h ^= b as u64; h = h.wrapping_mul(0x100000001b3); }
    h
}

pub fn collect_candidates(objects: &[Elf64Object]) -> FxHashMap<u64, Vec<(usize, usize)>> {
    let mut groups: FxHashMap<u64, Vec<(usize, usize)>> = FxHashMap::default();
    for (oi, obj) in objects.iter().enumerate() {
        for (si, sec) in obj.sections.iter().enumerate() {
            if sec.sh_type != SHT_PROGBITS || sec.flags & SHF_EXECINSTR == 0 || sec.size == 0 { continue; }
            let data = match obj.section_data.get(si) { Some(d) if !d.is_empty() => d.as_slice(), _ => continue };
            groups.entry(fnv1a64(data)).or_default().push((oi, si));
        }
    }
    groups.retain(|_, v| v.len() > 1);
    groups
}

pub fn group_is_safe(objects: &[Elf64Object], members: &[(usize, usize)]) -> bool {
    const R_X86_64_64: u32 = 1;
    const R_X86_64_32: u32 = 10;
    const R_X86_64_32S: u32 = 11;
    for &(oi, si) in members {
        for rela in &objects[oi].relocations[si] {
            match rela.rela_type {
                R_X86_64_64 | R_X86_64_32 | R_X86_64_32S => return false,
                _ => {}
            }
        }
    }
    true
}

pub fn analyse(objects: &[Elf64Object], safe_only: bool) -> IcfResult {
    let groups = collect_candidates(objects);
    let mut result = IcfResult::default();
    for (_hash, members) in &groups {
        if safe_only && !group_is_safe(objects, members) { continue; }
        for &m in &members[1..] {
            result.folded_sections += 1;
            result.bytes_saved += objects[m.0].sections[m.1].size;
        }
    }
    result
}

pub fn icf_mode_from_env() -> Option<&'static str> {
    match std::env::var("LCCC_LD_ICF") {
        Ok(s) if s.eq_ignore_ascii_case("safe") => Some("safe"),
        Ok(s) if s.eq_ignore_ascii_case("all") => Some("all"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fnv_deterministic() {
        assert_eq!(fnv1a64(b"hello"), fnv1a64(b"hello"));
        assert_ne!(fnv1a64(b"hello"), fnv1a64(b"world"));
    }
}
