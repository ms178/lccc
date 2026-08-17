//! COMDAT (`SHT_GROUP` / `GRP_COMDAT`) deduplication.
//!
//! # What a COMDAT group is
//!
//! C++ semantics require that an inline function, a template instantiation, a
//! vtable or a static data member may be *defined* in every translation unit
//! that uses it, yet exist exactly once in the program. The compiler emits each
//! such definition into its own section and records the set in an `SHT_GROUP`
//! section carrying `GRP_COMDAT` and a *signature symbol*. The linker keeps the
//! first group of a given signature and discards the members of every later
//! one.
//!
//! # Why the executable path needed this
//!
//! `emit_rel.rs` (the `ld -r` path) has done this since it was written, but the
//! executable path never did. Symbol resolution hid the consequence: only one
//! definition ever *wins*, so the program behaves correctly and every tool that
//! looks at symbols agrees with GNU ld. The duplicate section *bodies* were
//! still laid out, though — dead bytes reachable by nothing.
//!
//! Measured on three small C++ TUs sharing one header
//! (`g++ -O0 -fno-inline`), searching the linked image for the exact byte
//! pattern of `Widget<long>::twice`:
//!
//! ```text
//! lccc (before)  3 occurrences   exec bytes 780
//! ld.bfd         1 occurrence    exec bytes 579
//! ```
//!
//! On real C++ this scales with the number of translation units that include a
//! given header, which is why it is the main remaining *size* lever.
//!
//! # Relationship to ICF
//!
//! ICF folds sections that happen to be byte-identical and must prove it.
//! COMDAT dedup discards sections the *compiler* already declared
//! interchangeable, so it needs no content comparison and is always safe. They
//! are complementary, and COMDAT runs first because it is cheaper and strictly
//! more reliable.

use crate::backend::elf::{read_u32, GRP_COMDAT, SHT_GROUP};
use crate::common::fx_hash::{FxHashMap, FxHashSet};

use super::types::Elf64Object;

/// Result of a COMDAT scan.
#[derive(Debug, Default)]
pub struct ComdatPlan {
    /// Input sections to drop: members of a group whose signature was already
    /// claimed by an earlier group.
    pub dead: FxHashSet<(usize, usize)>,
    /// Number of duplicate groups discarded (not sections).
    pub groups_discarded: usize,
    /// Total bytes of discarded section content.
    pub bytes_saved: u64,
}

/// Decide which COMDAT group members lose.
///
/// The winner is the first group with a given signature in link order, which
/// is what GNU ld does and what makes the result reproducible: the plan depends
/// only on the order the inputs were given, never on hash iteration.
///
/// `SHT_GROUP` sections themselves are never emitted into an executable, so
/// they do not need to be marked dead here; the emitter already skips them.
pub fn plan_comdat(objects: &[Elf64Object]) -> ComdatPlan {
    let mut plan = ComdatPlan::default();
    // signature -> (object, section) of the winning group, for diagnostics.
    let mut winners: FxHashMap<String, (usize, usize)> = FxHashMap::default();

    for (oi, obj) in objects.iter().enumerate() {
        for (si, sec) in obj.sections.iter().enumerate() {
            if sec.sh_type != SHT_GROUP {
                continue;
            }
            let data = obj.section_data[si].as_slice();
            if data.len() < 4 {
                continue;
            }
            // Word 0 is the flag word; the rest are section indices.
            if read_u32(data, 0) & GRP_COMDAT == 0 {
                continue; // a plain (non-COMDAT) group is never deduplicated
            }
            // sh_info indexes the signature symbol in the object's symtab.
            let Some(sig_sym) = obj.symbols.get(sec.info as usize) else {
                continue;
            };
            if sig_sym.name.is_empty() {
                continue;
            }
            let sig = sig_sym.name.to_string();

            match winners.get(&sig) {
                None => {
                    winners.insert(sig, (oi, si));
                }
                Some(_) => {
                    plan.groups_discarded += 1;
                    for k in (4..data.len()).step_by(4) {
                        if k + 4 > data.len() {
                            break;
                        }
                        let member = read_u32(data, k) as usize;
                        if member < obj.sections.len() && plan.dead.insert((oi, member)) {
                            plan.bytes_saved += obj.sections[member].size;
                        }
                    }
                }
            }
        }
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::elf::{SHF_ALLOC, SHF_EXECINSTR, SHT_PROGBITS};
    use crate::backend::linker_common::{Elf64Section, Elf64Symbol, SectionData, SymStr};

    fn section(name: &str, sh_type: u32, size: u64, info: u32) -> Elf64Section {
        Elf64Section {
            name_idx: 0,
            name: name.to_string(),
            sh_type,
            flags: if sh_type == SHT_PROGBITS { SHF_ALLOC | SHF_EXECINSTR } else { 0 },
            addr: 0,
            offset: 0,
            size,
            link: 0,
            info,
            addralign: 1,
            entsize: 0,
        }
    }

    fn symbol(name: &str) -> Elf64Symbol {
        Elf64Symbol {
            name_idx: 0,
            name: SymStr::new(name),
            info: 1 << 4,
            other: 0,
            shndx: 1,
            value: 0,
            size: 0,
        }
    }

    /// Build an object holding one COMDAT group whose members are sections
    /// 1..=n, with the signature symbol at symtab index 1.
    fn comdat_object(sig: &str, member_sizes: &[u64], comdat: bool) -> Elf64Object {
        let mut sections = vec![section("", 0, 0, 0)]; // SHT_NULL
        let mut section_data = vec![SectionData::empty()];
        for (i, sz) in member_sizes.iter().enumerate() {
            sections.push(section(&format!(".text.m{i}"), SHT_PROGBITS, *sz, 0));
            section_data.push(SectionData::owned(vec![0u8; *sz as usize]));
        }
        // The group section lists its members' indices.
        let group_idx = sections.len();
        let mut gdata = Vec::new();
        gdata.extend_from_slice(&(if comdat { GRP_COMDAT } else { 0u32 }).to_le_bytes());
        for i in 0..member_sizes.len() {
            gdata.extend_from_slice(&((1 + i) as u32).to_le_bytes());
        }
        sections.push(section(".group", SHT_GROUP, gdata.len() as u64, 1));
        section_data.push(SectionData::owned(gdata));
        let _ = group_idx;

        Elf64Object {
            sections,
            symbols: vec![symbol(""), symbol(sig)],
            section_data,
            relocations: Vec::new(),
            source_name: format!("<{sig}>"),
        }
    }

    #[test]
    fn first_group_wins_and_later_duplicates_die() {
        let objs = vec![
            comdat_object("_ZNK6WidgetIlE5twiceEv", &[20, 8], true),
            comdat_object("_ZNK6WidgetIlE5twiceEv", &[20, 8], true),
            comdat_object("_ZNK6WidgetIlE5twiceEv", &[20, 8], true),
        ];
        let plan = plan_comdat(&objs);
        assert_eq!(plan.groups_discarded, 2, "two later groups must lose");
        // Object 0 keeps everything; objects 1 and 2 lose both members each.
        assert!(!plan.dead.contains(&(0, 1)) && !plan.dead.contains(&(0, 2)));
        assert!(plan.dead.contains(&(1, 1)) && plan.dead.contains(&(1, 2)));
        assert!(plan.dead.contains(&(2, 1)) && plan.dead.contains(&(2, 2)));
        assert_eq!(plan.bytes_saved, 2 * (20 + 8));
    }

    #[test]
    fn distinct_signatures_are_all_kept() {
        let objs = vec![
            comdat_object("sig_a", &[16], true),
            comdat_object("sig_b", &[16], true),
        ];
        let plan = plan_comdat(&objs);
        assert_eq!(plan.groups_discarded, 0);
        assert!(plan.dead.is_empty(), "different signatures are different entities");
    }

    /// A group without `GRP_COMDAT` is a plain section group (used for
    /// `.debug_*` grouping, among other things). Discarding one would delete
    /// live data.
    #[test]
    fn non_comdat_groups_are_never_discarded() {
        let objs = vec![
            comdat_object("same_sig", &[16], false),
            comdat_object("same_sig", &[16], false),
        ];
        let plan = plan_comdat(&objs);
        assert_eq!(plan.groups_discarded, 0);
        assert!(plan.dead.is_empty(),
                "only GRP_COMDAT groups are interchangeable");
    }

    #[test]
    fn empty_input_is_handled() {
        let plan = plan_comdat(&[]);
        assert!(plan.dead.is_empty());
        assert_eq!(plan.groups_discarded, 0);
        assert_eq!(plan.bytes_saved, 0);
    }

    /// The plan must not depend on hash iteration: the winner is decided by
    /// link order alone, so repeated runs agree.
    #[test]
    fn plan_is_deterministic() {
        let build = || vec![
            comdat_object("s1", &[8, 8], true),
            comdat_object("s2", &[8], true),
            comdat_object("s1", &[8, 8], true),
            comdat_object("s2", &[8], true),
        ];
        let first = plan_comdat(&build());
        for _ in 0..8 {
            let again = plan_comdat(&build());
            let mut a: Vec<_> = first.dead.iter().copied().collect();
            let mut b: Vec<_> = again.dead.iter().copied().collect();
            a.sort_unstable();
            b.sort_unstable();
            assert_eq!(a, b, "COMDAT plan must be stable across runs");
        }
        assert_eq!(first.groups_discarded, 2);
    }
}
