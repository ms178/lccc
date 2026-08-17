//! Identical Code Folding (ICF) for the x86-64 linker.
//!
//! Analysis + safety classification of identical executable sections.
//! Integration into the main emit path is gated by `LCCC_LD_ICF=safe|all`.
//!
//! Safety (`safe`): every relocation inside each member must be relative.
//! Absolute relocs (R_X86_64_64/32/32S) that capture the section address are
//! rejected. `--icf=all` skips that check.

#![allow(dead_code)]

use crate::backend::elf::{SHF_EXECINSTR, SHT_PROGBITS};
use crate::backend::linker_common::Elf64Object;
use crate::common::fx_hash::FxHashMap;

#[derive(Debug, Default, Clone)]
pub struct IcfResult {
    pub candidate_groups: usize,
    pub folded_sections: usize,
    pub bytes_saved: u64,
    pub rejected_unsafe: usize,
}

#[inline]
fn fnv1a64(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in data {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

pub fn collect_candidates(objects: &[Elf64Object]) -> FxHashMap<u64, Vec<(usize, usize)>> {
    let mut groups: FxHashMap<u64, Vec<(usize, usize)>> = FxHashMap::default();
    for (oi, obj) in objects.iter().enumerate() {
        for (si, sec) in obj.sections.iter().enumerate() {
            if sec.sh_type != SHT_PROGBITS || (sec.flags & SHF_EXECINSTR) == 0 || sec.size == 0 {
                continue;
            }
            let data = match obj.section_data.get(si) {
                Some(d) if !d.is_empty() => d.as_slice(),
                _ => continue,
            };
            if data.iter().all(|&b| b == 0x00 || b == 0x90) && data.len() < 16 {
                continue;
            }
            groups.entry(fnv1a64(data)).or_default().push((oi, si));
        }
    }
    groups.retain(|_, v| v.len() > 1);
    groups
}

const ABS_RELOCS: &[u32] = &[1, 10, 11]; // 64, 32, 32S

pub fn group_is_safe(objects: &[Elf64Object], members: &[(usize, usize)]) -> bool {
    for &(oi, si) in members {
        let obj = &objects[oi];
        if si >= obj.relocations.len() { continue; }
        for rela in &obj.relocations[si] {
            if ABS_RELOCS.contains(&rela.rela_type) {
                return false;
            }
        }
    }
    true
}

pub fn analyse(objects: &[Elf64Object], safe_only: bool) -> IcfResult {
    let groups = collect_candidates(objects);
    let mut result = IcfResult { candidate_groups: groups.len(), ..IcfResult::default() };
    for (_hash, members) in &groups {
        if safe_only && !group_is_safe(objects, members) {
            result.rejected_unsafe += 1;
            continue;
        }
        for &m in &members[1..] {
            result.folded_sections += 1;
            result.bytes_saved += objects[m.0].sections[m.1].size;
        }
    }
    result
}

pub fn icf_mode_from_env() -> Option<&'static str> {
    match std::env::var("LCCC_LD_ICF") {
        Ok(s) => {
            let s = s.trim();
            if s.eq_ignore_ascii_case("safe") { Some("safe") }
            else if s.eq_ignore_ascii_case("all") { Some("all") }
            else { None }
        }
        Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fnv_is_deterministic_and_sensitive() {
        assert_eq!(fnv1a64(b"identical"), fnv1a64(b"identical"));
        assert_ne!(fnv1a64(b"identical"), fnv1a64(b"different"));
    }
    #[test]
    fn empty_input_yields_no_candidates() {
        assert!(collect_candidates(&[]).is_empty());
    }
    #[test]
    fn analyse_on_empty_is_zero() {
        let r = analyse(&[], true);
        assert_eq!(r.candidate_groups, 0);
        assert_eq!(r.folded_sections, 0);
    }
}
