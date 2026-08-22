//! Identical Code Folding (ICF) for the x86-64 linker.
//!
//! Merges functions that are provably identical so only one copy reaches the
//! output. Enabled with `--icf=safe` / `--icf=all` (or `LCCC_LD_ICF=`).
//!
//! # What "identical" has to mean
//!
//! Comparing section *bytes* alone is wrong, and dangerously so. On x86-64 a
//! call is `e8 <rel32>` with the displacement supplied by a relocation, so
//!
//! ```c
//! int wrap_a(void) { return alpha(); }   // e8 00000000 c3
//! int wrap_b(void) { return beta();  }   // e8 00000000 c3
//! ```
//!
//! produce **byte-identical** sections that differ only in their relocation
//! targets. Folding them makes `wrap_b()` return `alpha()`'s value: a silent
//! miscompilation with no diagnostic. Two sections are therefore equal here
//! only when their bytes match **and** their relocation lists match
//! element-wise in offset, type, addend, and *resolved target identity*
//! (symbol name for globals; the referenced section for locals).
//!
//! # Safety classes
//!
//! * `safe` (default): additionally requires that no member has its address
//!   taken in a way that would let the program observe the fold. A function
//!   whose address escapes must keep a unique address, because C guarantees
//!   distinct functions compare unequal. Absolute relocations
//!   (`R_X86_64_64/32/32S`) targeting a member are treated as address-taking.
//! * `all`: folds regardless, matching `gold`/`lld`'s `--icf=all`. Faster and
//!   smaller, but only valid for programs that never compare function
//!   pointers for inequality.
//!
//! # Application
//!
//! `plan()` returns a redirection map `(obj, sec) -> (obj, sec)`. The emitter
//! drops folded sections from the layout and redirects every relocation and
//! symbol that pointed at them to the surviving representative, so no dangling
//! references remain.

use crate::backend::elf::{SHF_EXECINSTR, SHT_PROGBITS};
use crate::backend::linker_common::Elf64Object;
use crate::common::fx_hash::{FxHashMap, FxHashSet};

#[derive(Debug, Default, Clone)]
pub struct IcfResult {
    pub candidate_groups: usize,
    pub folded_sections: usize,
    pub bytes_saved: u64,
    pub rejected_unsafe: usize,
}

/// Section identity used for grouping: `(object, section)`.
pub type SecId = (usize, usize);

#[inline]
fn fnv1a64(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in data {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x1000_0000_01b3);
    }
    h
}

#[inline]
fn mix(h: u64, v: u64) -> u64 {
    (h ^ v).wrapping_mul(0x1000_0000_01b3)
}

/// Absolute relocation types: these capture the *address* of their target.
const ABS_RELOCS: &[u32] = &[1, 10, 11]; // R_X86_64_64, _32, _32S

/// How a relocation's target is identified when comparing two sections.
///
/// Local symbols carry no useful name, so they are identified by the section
/// they point into plus the in-section offset; globals by name, because the
/// same name from two different objects still denotes one entity after
/// resolution.
#[derive(PartialEq, Eq, Hash, Clone, Debug)]
enum RelTarget {
    Global(String),
    LocalSection { shndx: u16, value: u64 },
    Unknown(u32),
}

fn reloc_target(obj: &Elf64Object, sym_idx: u32) -> RelTarget {
    match obj.symbols.get(sym_idx as usize) {
        Some(sym) if !sym.name.is_empty() && !sym.is_local() => {
            RelTarget::Global(sym.name.to_string())
        }
        Some(sym) => RelTarget::LocalSection {
            shndx: sym.shndx,
            value: sym.value,
        },
        None => RelTarget::Unknown(sym_idx),
    }
}

/// Content hash covering bytes *and* relocations.
fn section_hash(obj: &Elf64Object, si: usize, data: &[u8]) -> u64 {
    let mut h = fnv1a64(data);
    h = mix(h, data.len() as u64);
    if let Some(relas) = obj.relocations.get(si) {
        h = mix(h, relas.len() as u64);
        for r in relas {
            h = mix(h, r.offset);
            h = mix(h, u64::from(r.rela_type));
            h = mix(h, r.addend as u64);
            match reloc_target(obj, r.sym_idx) {
                RelTarget::Global(name) => {
                    h = mix(h, 1);
                    h = mix(h, fnv1a64(name.as_bytes()));
                }
                RelTarget::LocalSection { shndx, value } => {
                    h = mix(h, 2);
                    h = mix(h, u64::from(shndx));
                    h = mix(h, value);
                }
                RelTarget::Unknown(i) => {
                    h = mix(h, 3);
                    h = mix(h, u64::from(i));
                }
            }
        }
    }
    h
}

/// Exact equality, used to confirm a hash bucket really is one fold group.
/// Hash collisions must never fold, so this is not optional.
fn sections_equal(objects: &[Elf64Object], a: SecId, b: SecId) -> bool {
    let (oa, sa) = a;
    let (ob, sb) = b;
    let da = objects[oa].section_data[sa].as_slice();
    let db = objects[ob].section_data[sb].as_slice();
    if da != db {
        return false;
    }
    let ea = objects[oa].sections[sa].entsize;
    let eb = objects[ob].sections[sb].entsize;
    let fa = objects[oa].sections[sa].flags;
    let fb = objects[ob].sections[sb].flags;
    if ea != eb || fa != fb {
        return false;
    }
    // Alignment must match: folding a 64-byte-aligned function onto a
    // 16-byte-aligned one silently weakens the guarantee the compiler relied on.
    if objects[oa].sections[sa].addralign != objects[ob].sections[sb].addralign {
        return false;
    }
    let empty = Vec::new();
    let ra = objects[oa].relocations.get(sa).unwrap_or(&empty);
    let rb = objects[ob].relocations.get(sb).unwrap_or(&empty);
    if ra.len() != rb.len() {
        return false;
    }
    for (x, y) in ra.iter().zip(rb.iter()) {
        if x.offset != y.offset || x.rela_type != y.rela_type || x.addend != y.addend {
            return false;
        }
        if reloc_target(&objects[oa], x.sym_idx) != reloc_target(&objects[ob], y.sym_idx) {
            return false;
        }
    }
    true
}

/// Group foldable executable sections by content hash.
pub fn collect_candidates(objects: &[Elf64Object]) -> FxHashMap<u64, Vec<SecId>> {
    let mut groups: FxHashMap<u64, Vec<SecId>> = FxHashMap::default();
    for (oi, obj) in objects.iter().enumerate() {
        for (si, sec) in obj.sections.iter().enumerate() {
            if sec.sh_type != SHT_PROGBITS || (sec.flags & SHF_EXECINSTR) == 0 || sec.size == 0 {
                continue;
            }
            let data = match obj.section_data.get(si) {
                Some(d) if !d.is_empty() => d.as_slice(),
                _ => continue,
            };
            // Pure padding is not worth folding and is a common accidental
            // match; skip tiny all-nop/all-zero sections.
            if data.len() < 16 && data.iter().all(|&b| b == 0x00 || b == 0x90) {
                continue;
            }
            groups
                .entry(section_hash(obj, si, data))
                .or_default()
                .push((oi, si));
        }
    }
    groups.retain(|_, v| v.len() > 1);
    groups
}

/// Sections whose address is observable, so folding them could change
/// program-visible behaviour.
///
/// A function referenced by an absolute relocation (a function-pointer table,
/// a vtable, an initialiser) must keep a distinct address. Scanning *all*
/// relocations in the link — not just those inside candidate sections — is
/// what makes this sound: the address is usually taken from somewhere else.
fn address_taken_sections(objects: &[Elf64Object]) -> FxHashSet<SecId> {
    // Map global symbol name -> defining (object, section), so an absolute
    // reference by name can be attributed back to the section it defines.
    let mut def_of: FxHashMap<&str, SecId> = FxHashMap::default();
    for (oi, obj) in objects.iter().enumerate() {
        for sym in &obj.symbols {
            if sym.name.is_empty() || sym.is_undefined() {
                continue;
            }
            let si = sym.shndx as usize;
            if si < obj.sections.len() {
                def_of.entry(sym.name.as_str()).or_insert((oi, si));
            }
        }
    }

    let mut taken: FxHashSet<SecId> = FxHashSet::default();
    for (oi, obj) in objects.iter().enumerate() {
        for relas in obj.relocations.iter() {
            for r in relas {
                if !ABS_RELOCS.contains(&r.rela_type) {
                    continue;
                }
                match obj.symbols.get(r.sym_idx as usize) {
                    Some(sym) if !sym.name.is_empty() && !sym.is_local() => {
                        if let Some(&id) = def_of.get(sym.name.as_str()) {
                            taken.insert(id);
                        }
                    }
                    Some(sym) => {
                        let si = sym.shndx as usize;
                        if si < obj.sections.len() {
                            taken.insert((oi, si));
                        }
                    }
                    None => {}
                }
            }
        }
    }
    taken
}

/// Legacy predicate retained for the older analysis entry point.
pub fn group_is_safe(objects: &[Elf64Object], members: &[SecId]) -> bool {
    let taken = address_taken_sections(objects);
    !members.iter().any(|m| taken.contains(m))
}

/// A concrete folding plan.
#[derive(Debug, Default, Clone)]
pub struct IcfPlan {
    /// Folded section -> surviving representative.
    pub redirect: FxHashMap<SecId, SecId>,
    pub result: IcfResult,
}

impl IcfPlan {
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.redirect.is_empty()
    }
    /// Follow the redirection for `id` (identity when not folded).
    #[inline]
    pub fn resolve(&self, id: SecId) -> SecId {
        self.redirect.get(&id).copied().unwrap_or(id)
    }
}

/// Compute which sections to fold.
///
/// `safe_only` excludes address-taken sections. The representative of each
/// group is the lexicographically smallest `(object, section)` so the result
/// is deterministic and independent of hash iteration order — a linker that
/// folds differently between runs is not reproducible.
pub fn plan(objects: &[Elf64Object], safe_only: bool) -> IcfPlan {
    let groups = collect_candidates(objects);
    let mut out = IcfPlan {
        result: IcfResult {
            candidate_groups: groups.len(),
            ..Default::default()
        },
        ..Default::default()
    };
    if groups.is_empty() {
        return out;
    }
    let taken = if safe_only {
        address_taken_sections(objects)
    } else {
        FxHashSet::default()
    };

    // Deterministic group order.
    let mut buckets: Vec<Vec<SecId>> = groups.into_values().collect();
    for b in buckets.iter_mut() {
        b.sort_unstable();
    }
    buckets.sort_unstable();

    for members in buckets {
        // Split a hash bucket into exact-equality classes: a collision must
        // never cause a fold.
        let mut remaining: Vec<SecId> = members;
        while remaining.len() > 1 {
            let rep = remaining[0];
            let mut same = vec![rep];
            let mut rest = Vec::new();
            for &m in &remaining[1..] {
                if sections_equal(objects, rep, m) {
                    same.push(m);
                } else {
                    rest.push(m);
                }
            }
            if same.len() > 1 {
                if safe_only && same.iter().any(|m| taken.contains(m)) {
                    out.result.rejected_unsafe += 1;
                } else {
                    for &m in &same[1..] {
                        out.redirect.insert(m, rep);
                        out.result.folded_sections += 1;
                        out.result.bytes_saved += objects[m.0].sections[m.1].size;
                    }
                }
            }
            remaining = rest;
        }
    }
    out
}

pub fn analyse(objects: &[Elf64Object], safe_only: bool) -> IcfResult {
    plan(objects, safe_only).result
}

/// `--icf=` / `LCCC_LD_ICF=` mode, validated.
pub fn icf_mode_from_env() -> Option<&'static str> {
    match std::env::var("LCCC_LD_ICF") {
        Ok(s) => parse_icf_mode(s.trim()),
        Err(_) => None,
    }
}

pub fn parse_icf_mode(s: &str) -> Option<&'static str> {
    if s.eq_ignore_ascii_case("safe") {
        Some("safe")
    } else if s.eq_ignore_ascii_case("all") {
        Some("all")
    } else {
        None // includes "none", which disables ICF
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::linker_common::{
        Elf64Object, Elf64Rela, Elf64Section, Elf64Symbol, SectionData, SymStr,
    };

    fn sec(name: &str, data: &[u8], align: u64) -> Elf64Section {
        Elf64Section {
            name_idx: 0,
            name: name.to_string(),
            sh_type: SHT_PROGBITS,
            flags: SHF_EXECINSTR | 2, /* SHF_ALLOC */
            addr: 0,
            offset: 0,
            size: data.len() as u64,
            link: 0,
            info: 0,
            addralign: align,
            entsize: 0,
        }
    }

    fn sym(name: &str, shndx: u16, value: u64, global: bool) -> Elf64Symbol {
        Elf64Symbol {
            name_idx: 0,
            name: SymStr::new(name),
            info: if global { 1 << 4 } else { 0 },
            other: 0,
            shndx,
            value,
            size: 0,
        }
    }

    fn rela(offset: u64, sym_idx: u32, rela_type: u32, addend: i64) -> Elf64Rela {
        Elf64Rela {
            offset,
            sym_idx,
            rela_type,
            addend,
        }
    }

    /// Build one object: sections[i] has data[i], relocs[i], symbols shared.
    fn obj(datas: &[&[u8]], relocs: Vec<Vec<Elf64Rela>>, symbols: Vec<Elf64Symbol>) -> Elf64Object {
        let mut sections = Vec::new();
        let mut section_data = Vec::new();
        for (i, d) in datas.iter().enumerate() {
            sections.push(sec(&format!(".text.f{i}"), d, 16));
            section_data.push(SectionData::owned(d.to_vec()));
        }
        Elf64Object {
            sections,
            symbols,
            section_data,
            relocations: relocs,
            source_name: "<test>".into(),
        }
    }

    const CALL_RET: &[u8] = &[0xe8, 0, 0, 0, 0, 0xc3];
    const PLT32: u32 = 4;

    /// THE bug this rewrite exists to prevent. Two sections with identical
    /// bytes whose relocations call *different* functions must NOT fold;
    /// doing so silently makes one function return the other's value.
    #[test]
    fn identical_bytes_different_call_targets_do_not_fold() {
        let o = obj(
            &[CALL_RET, CALL_RET],
            vec![vec![rela(1, 2, PLT32, -4)], vec![rela(1, 3, PLT32, -4)]],
            vec![
                sym("", 0, 0, false),
                sym("", 0, 0, false),
                sym("alpha", 0, 0, true),
                sym("beta", 0, 0, true),
            ],
        );
        let p = plan(&[o], true);
        assert!(
            p.is_empty(),
            "folded two sections that call different functions -- this is a \
             miscompilation, not an optimisation: {:?}",
            p.redirect
        );
    }

    /// The positive case: identical bytes calling the *same* function fold.
    #[test]
    fn identical_bytes_same_call_target_folds() {
        let o = obj(
            &[CALL_RET, CALL_RET],
            vec![vec![rela(1, 2, PLT32, -4)], vec![rela(1, 2, PLT32, -4)]],
            vec![
                sym("", 0, 0, false),
                sym("", 0, 0, false),
                sym("alpha", 0, 0, true),
            ],
        );
        let p = plan(&[o], true);
        assert_eq!(p.result.folded_sections, 1, "should fold one duplicate");
        assert_eq!(
            p.resolve((0, 1)),
            (0, 0),
            "duplicate redirects to the first"
        );
        assert_eq!(p.resolve((0, 0)), (0, 0), "representative maps to itself");
    }

    /// Addends participate in identity: `call foo+0` and `call foo+8` are
    /// different code even with identical bytes.
    #[test]
    fn differing_addends_do_not_fold() {
        let o = obj(
            &[CALL_RET, CALL_RET],
            vec![vec![rela(1, 2, PLT32, -4)], vec![rela(1, 2, PLT32, 4)]],
            vec![
                sym("", 0, 0, false),
                sym("", 0, 0, false),
                sym("alpha", 0, 0, true),
            ],
        );
        assert!(
            plan(&[o], true).is_empty(),
            "addend must be part of identity"
        );
    }

    /// Under `safe`, a function whose address is taken elsewhere keeps its own
    /// address: C requires distinct functions to compare unequal.
    #[test]
    fn address_taken_blocks_folding_in_safe_mode() {
        let mut o = obj(
            &[CALL_RET, CALL_RET],
            vec![
                vec![rela(1, 2, PLT32, -4)],
                vec![rela(1, 2, PLT32, -4)],
                Vec::new(),
            ],
            vec![
                sym("", 0, 0, false),
                sym("", 0, 0, false),
                sym("alpha", 0, 0, true),
                sym("dup", 1, 0, true), // names section 1
            ],
        );
        // A data section holding an absolute pointer to `dup`.
        o.sections.push(Elf64Section {
            name_idx: 0,
            name: ".data.ptr".into(),
            sh_type: SHT_PROGBITS,
            flags: 2,
            addr: 0,
            offset: 0,
            size: 8,
            link: 0,
            info: 0,
            addralign: 8,
            entsize: 0,
        });
        o.section_data.push(SectionData::owned(vec![0u8; 8]));
        o.relocations[2] = vec![rela(0, 3, 1 /* R_X86_64_64 */, 0)];

        assert!(
            plan(&[o.clone()], true).is_empty(),
            "safe ICF must not fold an address-taken function"
        );
        assert_eq!(
            plan(&[o], false).result.folded_sections,
            1,
            "--icf=all folds it anyway"
        );
    }

    /// Different alignment means the compiler asked for something stricter;
    /// folding onto a weaker section would silently break that.
    #[test]
    fn differing_alignment_does_not_fold() {
        let mut o = obj(
            &[CALL_RET, CALL_RET],
            vec![vec![rela(1, 2, PLT32, -4)], vec![rela(1, 2, PLT32, -4)]],
            vec![
                sym("", 0, 0, false),
                sym("", 0, 0, false),
                sym("alpha", 0, 0, true),
            ],
        );
        o.sections[1].addralign = 64;
        assert!(plan(&[o], true).is_empty(), "alignment is part of identity");
    }

    /// Folding must be deterministic: same input, same representative, every
    /// run. Hash-map iteration order must not leak into the output.
    #[test]
    fn plan_is_deterministic() {
        let build = || {
            obj(
                &[CALL_RET, CALL_RET, CALL_RET],
                vec![
                    vec![rela(1, 3, PLT32, -4)],
                    vec![rela(1, 3, PLT32, -4)],
                    vec![rela(1, 3, PLT32, -4)],
                ],
                vec![
                    sym("", 0, 0, false),
                    sym("", 0, 0, false),
                    sym("", 0, 0, false),
                    sym("alpha", 0, 0, true),
                ],
            )
        };
        let first = plan(&[build()], true);
        for _ in 0..8 {
            let again = plan(&[build()], true);
            assert_eq!(first.redirect, again.redirect, "ICF plan must be stable");
        }
        assert_eq!(first.result.folded_sections, 2);
        assert_eq!(first.resolve((0, 1)), (0, 0));
        assert_eq!(first.resolve((0, 2)), (0, 0));
    }

    #[test]
    fn mode_parsing_rejects_garbage() {
        assert_eq!(parse_icf_mode("safe"), Some("safe"));
        assert_eq!(parse_icf_mode("ALL"), Some("all"));
        assert_eq!(parse_icf_mode("none"), None);
        assert_eq!(parse_icf_mode(""), None);
        assert_eq!(parse_icf_mode("yes"), None);
    }

    #[test]
    fn empty_input_is_handled() {
        assert!(collect_candidates(&[]).is_empty());
        assert!(plan(&[], true).is_empty());
        assert_eq!(analyse(&[], true).candidate_groups, 0);
    }

    #[test]
    fn fnv_is_deterministic_and_sensitive() {
        assert_eq!(fnv1a64(b"identical"), fnv1a64(b"identical"));
        assert_ne!(fnv1a64(b"identical"), fnv1a64(b"different"));
    }
}
