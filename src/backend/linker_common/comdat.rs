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
//! executable path never did. Symbol resolution hid the consequence for the
//! usual C++ copies, whose symbols are weak: only one definition ever *wins*,
//! so the program behaves correctly and every tool that looks at symbols agrees
//! with GNU ld. The duplicate section *bodies* were still laid out, though —
//! dead bytes reachable by nothing.
//!
//! A *strong* global defined in every copy (hand-written or C COMDAT code,
//! `-fno-weak`) was worse: resolution saw two strong definitions and failed
//! with "multiple definition". A discarded copy's definitions do not take
//! part in resolution in GNU ld or lld; `in_discarded_group` gives symbol
//! registration the same answer where a definition would displace another.
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

use crate::backend::elf::{GRP_COMDAT, SHT_GROUP, read_u32};
use crate::common::fx_hash::FxHashSet;

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

/// The COMDAT groups of `obj`: (signature, member section indices) per
/// `SHT_GROUP` section carrying `GRP_COMDAT` and a named signature. A plain
/// group is never deduplicated, and an unnamed signature identifies nothing.
fn comdat_groups(obj: &Elf64Object) -> impl Iterator<Item = (&str, &[u8])> {
    obj.sections.iter().enumerate().filter_map(|(si, sec)| {
        if sec.sh_type != SHT_GROUP {
            return None;
        }
        let data = obj.section_data[si].as_slice();
        // Word 0 is the flag word; the rest are section indices.
        if data.len() < 4 || read_u32(data, 0) & GRP_COMDAT == 0 {
            return None;
        }
        // sh_info indexes the signature symbol in the object's symtab.
        let sig = obj.symbols.get(sec.info as usize)?;
        (!sig.name.is_empty()).then(|| (sig.name.as_str(), &data[4..]))
    })
}

fn members(words: &[u8]) -> impl Iterator<Item = usize> + '_ {
    words.chunks_exact(4).map(|w| read_u32(w, 0) as usize)
}

/// The link's input objects in link order, together with the COMDAT group
/// signatures they claim.
///
/// Symbol registration asks, for a definition that would displace or clash
/// with an existing one, whether its section belongs to a group some earlier
/// object already claimed (`in_discarded_group`). Answering that by
/// rescanning every earlier object's groups made registration quadratic: a
/// link of N objects whose overriding definitions sit in COMDAT members took
/// 0.04 s at N = 500 and 1.75 s at N = 4000. Objects only ever enter the link
/// through `push`, which records their signatures, so the query is one hash
/// lookup. The set derefs to a SLICE, never to the `Vec`: element contents
/// may be edited by later link phases, but the object list can only grow
/// through `push`, so the claims can never go stale.
#[derive(Debug, Default)]
pub struct ObjectSet {
    objects: Vec<Elf64Object>,
    claimed: FxHashSet<String>,
}

impl ObjectSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append the next object in link order and record the group
    /// signatures it claims.
    pub fn push(&mut self, obj: Elf64Object) {
        for (sig, _) in comdat_groups(&obj) {
            if !self.claimed.contains(sig) {
                self.claimed.insert(sig.to_string());
            }
        }
        self.objects.push(obj);
    }

    /// Did an object in the set claim COMDAT signature `sig`?
    pub fn claims(&self, sig: &str) -> bool {
        self.claimed.contains(sig)
    }

    pub fn into_vec(self) -> Vec<Elf64Object> {
        self.objects
    }
}

impl std::ops::Deref for ObjectSet {
    type Target = [Elf64Object];
    fn deref(&self) -> &[Elf64Object] {
        &self.objects
    }
}

impl std::ops::DerefMut for ObjectSet {
    fn deref_mut(&mut self) -> &mut [Elf64Object] {
        &mut self.objects
    }
}

impl FromIterator<Elf64Object> for ObjectSet {
    fn from_iter<I: IntoIterator<Item = Elf64Object>>(iter: I) -> Self {
        let mut set = ObjectSet::new();
        for obj in iter {
            set.push(obj);
        }
        set
    }
}

impl<'a> IntoIterator for &'a ObjectSet {
    type Item = &'a Elf64Object;
    type IntoIter = std::slice::Iter<'a, Elf64Object>;
    fn into_iter(self) -> Self::IntoIter {
        self.objects.iter()
    }
}

impl<'a> IntoIterator for &'a mut ObjectSet {
    type Item = &'a mut Elf64Object;
    type IntoIter = std::slice::IterMut<'a, Elf64Object>;
    fn into_iter(self) -> Self::IntoIter {
        self.objects.iter_mut()
    }
}

/// Is section `shndx` of `obj` a member of a COMDAT group whose signature
/// an object in `prior` (the objects registered before it, in link order)
/// already claimed? Such a copy is discarded -- `plan_comdat` makes the same
/// first-wins decision over the same order -- so its definitions must not
/// compete with the kept copy's. O(groups of `obj`): the prior claims are
/// one hash lookup (see `ObjectSet`).
pub fn in_discarded_group(prior: &ObjectSet, obj: &Elf64Object, shndx: usize) -> bool {
    comdat_groups(obj)
        .find(|(_, words)| members(words).any(|m| m == shndx))
        .is_some_and(|(sig, _)| prior.claims(sig))
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
    // Signatures claimed so far (borrowed from the objects: no allocation).
    let mut winners: FxHashSet<&str> = FxHashSet::default();
    for (oi, obj) in objects.iter().enumerate() {
        for (sig, words) in comdat_groups(obj) {
            if winners.insert(sig) {
                continue;
            }
            plan.groups_discarded += 1;
            for member in members(words) {
                if member < obj.sections.len() && plan.dead.insert((oi, member)) {
                    plan.bytes_saved += obj.sections[member].size;
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
            flags: if sh_type == SHT_PROGBITS {
                SHF_ALLOC | SHF_EXECINSTR
            } else {
                0
            },
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
        assert!(
            plan.dead.is_empty(),
            "different signatures are different entities"
        );
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
        assert!(
            plan.dead.is_empty(),
            "only GRP_COMDAT groups are interchangeable"
        );
    }

    /// Symbol registration's view of the same first-wins decision: only a
    /// member of a later copy of a claimed signature is discarded.
    #[test]
    fn in_discarded_group_matches_the_plan() {
        let objs: ObjectSet = [
            comdat_object("sig_a", &[16, 8], true),
            comdat_object("sig_b", &[16], true),
            comdat_object("", &[16], true),
        ]
        .into_iter()
        .collect();
        let later_a = comdat_object("sig_a", &[16, 8], true);
        let later_plain = comdat_object("sig_a", &[16], false);
        let later_unnamed = comdat_object("", &[16], true);
        assert!(in_discarded_group(&objs, &later_a, 1));
        assert!(in_discarded_group(&objs, &later_a, 2));
        // The group section itself is not a member.
        assert!(!in_discarded_group(&objs, &later_a, 3));
        assert!(
            !in_discarded_group(&ObjectSet::new(), &later_a, 1),
            "first copy wins"
        );
        assert!(!in_discarded_group(&objs, &later_plain, 1));
        assert!(!in_discarded_group(&objs, &later_unnamed, 1));
    }

    /// An object with several groups: `(signature, member sizes)` each, the
    /// members of group k following those of groups 0..k.
    fn multi_group_object(groups: &[(&str, Vec<u64>)]) -> Elf64Object {
        let mut sections = vec![section("", 0, 0, 0)];
        let mut section_data = vec![SectionData::empty()];
        let mut symbols = vec![symbol("")];
        let mut group_secs = Vec::new();
        for (sig, sizes) in groups {
            let mut gdata = GRP_COMDAT.to_le_bytes().to_vec();
            for sz in sizes {
                gdata.extend_from_slice(&(sections.len() as u32).to_le_bytes());
                sections.push(section(".text.m", SHT_PROGBITS, *sz, 0));
                section_data.push(SectionData::owned(vec![0u8; *sz as usize]));
            }
            symbols.push(symbol(sig));
            group_secs.push((symbols.len() as u32 - 1, gdata));
        }
        for (sig_idx, gdata) in group_secs {
            sections.push(section(".group", SHT_GROUP, gdata.len() as u64, sig_idx));
            section_data.push(SectionData::owned(gdata));
        }
        Elf64Object {
            sections,
            symbols,
            section_data,
            relocations: Vec::new(),
            source_name: "<multi>".to_string(),
        }
    }

    /// Property: registration's incremental answer equals the whole-link
    /// plan, member by member, over random links (xorshift, fixed seeds:
    /// reproducible without a dependency). Objects draw 0..=3 groups from a
    /// pool of 6 signatures, so claims, repeats and fresh signatures mix.
    #[test]
    fn in_discarded_group_agrees_with_plan_comdat_on_random_links() {
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move |n: u64| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state % n
        };
        let pool = ["s0", "s1", "s2", "s3", "s4", "s5"];
        for _link in 0..200 {
            let n_objs = 1 + next(12) as usize;
            let mut objs = Vec::new();
            for _ in 0..n_objs {
                let mut sigs: Vec<&str> = Vec::new();
                for _ in 0..next(4) {
                    let s = pool[next(pool.len() as u64) as usize];
                    if !sigs.contains(&s) {
                        sigs.push(s);
                    }
                }
                let groups: Vec<(&str, Vec<u64>)> = sigs
                    .into_iter()
                    .map(|s| (s, (0..1 + next(3)).map(|k| 4 + k).collect()))
                    .collect();
                objs.push(multi_group_object(&groups));
            }
            let plan = plan_comdat(&objs);
            let mut prior = ObjectSet::new();
            for (oi, obj) in objs.into_iter().enumerate() {
                for shndx in 0..obj.sections.len() {
                    assert_eq!(
                        in_discarded_group(&prior, &obj, shndx),
                        plan.dead.contains(&(oi, shndx)),
                        "object {oi} section {shndx}"
                    );
                }
                prior.push(obj);
            }
        }
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
        let build = || {
            vec![
                comdat_object("s1", &[8, 8], true),
                comdat_object("s2", &[8], true),
                comdat_object("s1", &[8, 8], true),
                comdat_object("s2", &[8], true),
            ]
        };
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
