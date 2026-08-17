//! SHF_MERGE section deduplication (string and fixed-size constant pools).
//!
//! GCC/Clang emit mergeable sections (`.rodata.str1.1` with SHF_MERGE|
//! SHF_STRINGS entsize 1, `.rodata.cst8` with SHF_MERGE entsize 8, ...) whose
//! entries the linker is expected to deduplicate across all input objects.
//! Without dedup every duplicated format string/FP constant is emitted once
//! per referencing object — typically 10-20 % of `.rodata` in real programs
//! (GNU ld, mold, lld all deduplicate).
//!
//! Strategy: before section merging, build one pool per *output* section name
//! from all eligible input sections, interning entries. Eligible input
//! sections are then excluded from normal layout (size forced to 0) and a
//! synthetic object carrying the pools is appended. During relocation
//! application the emitter maps `(input_section, input_offset)` through
//! [`StringMergePlan::remap`] to the pool location.
//!
//! An input section is *disqualified* (left un-deduplicated) when any ALLOC
//! section references it with a relocation type whose target offset cannot be
//! recovered (bias unknown), or when its payload does not cleanly divide into
//! entries. Correctness always wins over size.

use super::SectionData;
use super::SymStr;
use crate::common::fx_hash::FxHashMap;

use crate::backend::elf::{
    SHT_PROGBITS, SHF_ALLOC, SHF_MERGE, SHF_STRINGS,
    STT_SECTION,
};
use super::types::Elf64Object;

/// x86-64 relocation types with a recoverable target offset, and their
/// instruction bias: `target = S + A + bias`.
fn reloc_bias_x86(rtype: u32) -> Option<i64> {
    match rtype {
        1 /* 64 */ | 10 /* 32 */ | 11 /* 32S */ | 24 /* PC64 */ => Some(0),
        2 /* PC32 */ | 4 /* PLT32 */ => Some(4),
        _ => None, // GOT-relative & TLS types: disqualify the target section
    }
}

/// Per-input-section remap: sorted entry starts + new offsets in the pool.
struct SecRemap {
    /// Sorted (old_entry_start, new_pool_offset, entry_len).
    entries: Vec<(u64, u64, u64)>,
    /// Which pool (index into `pools`) this section was merged into.
    pool_idx: usize,
}

/// One deduplicated output pool.
pub struct MergePool {
    /// Output section name (e.g. ".rodata.str1.1").
    pub name: String,
    pub data: Vec<u8>,
    pub align: u64,
    pub flags: u64,
    pub entsize: u64,
}

pub struct StringMergePlan {
    remaps: FxHashMap<(usize, usize), SecRemap>,
    pub pools: Vec<MergePool>,
}

impl StringMergePlan {
    /// Is `(obj, sec)` a deduplicated section?
    pub fn contains(&self, obj: usize, sec: usize) -> bool {
        self.remaps.contains_key(&(obj, sec))
    }

    /// All input sections consumed by the plan.
    pub fn merged_sections(&self) -> impl Iterator<Item = &(usize, usize)> {
        self.remaps.keys()
    }

    /// Recoverable-bias lookup for the emitter.
    pub fn bias(rtype: u32) -> Option<i64> { reloc_bias_x86(rtype) }

    /// Map an offset inside an input merge section to
    /// `(pool_idx, new_offset_in_pool)`. Offsets that point into the middle
    /// of an entry keep their intra-entry delta.
    pub fn remap(&self, obj: usize, sec: usize, off: u64) -> Option<(usize, u64)> {
        let r = self.remaps.get(&(obj, sec))?;
        // binary search for the entry containing `off`
        let i = r.entries.partition_point(|&(start, _, _)| start <= off);
        if i == 0 { return None; }
        let (start, new_off, len) = r.entries[i - 1];
        let delta = off - start;
        if delta > len { return None; } // past entry end (== len allowed: end pointer)
        Some((r.pool_idx, new_off + delta))
    }
}

/// Build the dedup plan. `dead` marks gc-eliminated sections (skipped).
/// `map_name` is the input→output section name mapping used by the linker.
pub fn plan_string_merge(
    objects: &[Elf64Object],
    dead: &crate::common::fx_hash::FxHashSet<(usize, usize)>,
    map_name: fn(&str) -> &str,
) -> Option<StringMergePlan> {
    // 1. Candidates: ALLOC|MERGE PROGBITS, sane entsize, not gc'd, read-only.
    let mut candidates: Vec<(usize, usize)> = Vec::new();
    for (oi, obj) in objects.iter().enumerate() {
        for (si, sec) in obj.sections.iter().enumerate() {
            if sec.sh_type != SHT_PROGBITS { continue; }
            if sec.flags & SHF_MERGE == 0 || sec.flags & SHF_ALLOC == 0 { continue; }
            if sec.flags & 0x1 != 0 { continue; } // SHF_WRITE: never dedup writable
            if dead.contains(&(oi, si)) { continue; }
            if sec.size == 0 { continue; }
            let data = &obj.section_data[si];
            if data.len() != sec.size as usize { continue; }
            if sec.flags & SHF_STRINGS != 0 {
                if sec.entsize != 1 { continue; }         // wide strings: keep as-is
                if *data.last().unwrap_or(&1) != 0 { continue; } // must end with NUL
            } else {
                let ent = sec.entsize;
                if ent == 0 || ent > 32 || data.len() as u64 % ent != 0 { continue; }
            }
            // A merge section that itself HAS relocations cannot be reordered
            // safely (entry contents would change) — extremely rare; skip.
            if obj.relocations.get(si).map(|r| !r.is_empty()).unwrap_or(false) { continue; }
            // Global/weak named symbols inside a merge section would need
            // their (value+addend) remapped through the global table across
            // objects; compilers only emit local .LC* labels here, so
            // disqualify the exotic case instead of risking corruption.
            let has_global_sym = obj.symbols.iter().any(|sym|
                !sym.is_local() && sym.shndx as usize == si);
            if has_global_sym { continue; }
            candidates.push((oi, si));
        }
    }
    if candidates.is_empty() { return None; }

    // 2. Disqualify sections referenced by un-recoverable relocation types
    //    from ALLOC sections (their relocs must be remappable).
    let cand_set: crate::common::fx_hash::FxHashSet<(usize, usize)> =
        candidates.iter().copied().collect();
    let mut disqualified: crate::common::fx_hash::FxHashSet<(usize, usize)> =
        crate::common::fx_hash::FxHashSet::default();
    for (oi, obj) in objects.iter().enumerate() {
        for (from_sec, relas) in obj.relocations.iter().enumerate() {
            if relas.is_empty() { continue; }
            let from_alloc = obj.sections.get(from_sec)
                .map(|s| s.flags & SHF_ALLOC != 0).unwrap_or(false);
            if !from_alloc { continue; }
            for rela in relas {
                let si = rela.sym_idx as usize;
                let Some(sym) = obj.symbols.get(si) else { continue };
                let tsec = sym.shndx as usize;
                if !cand_set.contains(&(oi, tsec)) { continue; }
                if reloc_bias_x86(rela.rela_type).is_none() {
                    disqualified.insert((oi, tsec));
                }
                // Non-section named symbols into merge sections are fine:
                // the emitter remaps sym.value+addend the same way.
                let _ = STT_SECTION;
            }
        }
    }

    // 3. Build pools per output section name, interning entries.
    let mut pools: Vec<MergePool> = Vec::new();
    let mut pool_by_name: FxHashMap<String, usize> = FxHashMap::default();
    // Per-pool intern table: entry bytes -> pool offset.
    let mut interns: Vec<FxHashMap<Vec<u8>, u64>> = Vec::new();
    let mut remaps: FxHashMap<(usize, usize), SecRemap> = FxHashMap::default();

    for &(oi, si) in &candidates {
        if disqualified.contains(&(oi, si)) { continue; }
        let sec = &objects[oi].sections[si];
        let data = &objects[oi].section_data[si];
        // Key pools by the INPUT section name: .rodata.str1.1 and
        // .rodata.str1.8 have different alignment contracts and must not
        // share a pool (entries in .str1.8 are 8-aligned).
        let out_name = sec.name.clone();
        let _ = map_name; // output placement still uses map_section_name later
        let pool_idx = match pool_by_name.get(&out_name) {
            Some(&i) => i,
            None => {
                let i = pools.len();
                pool_by_name.insert(out_name.clone(), i);
                pools.push(MergePool {
                    name: out_name,
                    data: Vec::new(),
                    align: sec.addralign.max(1),
                    flags: sec.flags,
                    entsize: sec.entsize,
                });
                interns.push(FxHashMap::default());
                i
            }
        };
        let align = sec.addralign.max(1);
        if align > pools[pool_idx].align { pools[pool_idx].align = align; }
        if pools[pool_idx].entsize != sec.entsize { pools[pool_idx].entsize = 0; }

        let mut entries: Vec<(u64, u64, u64)> = Vec::new();
        let is_strings = sec.flags & SHF_STRINGS != 0;
        let mut pos = 0usize;
        while pos < data.len() {
            let end = if is_strings {
                match data[pos..].iter().position(|&b| b == 0) {
                    Some(n) => pos + n + 1, // include NUL
                    None => data.len(),
                }
            } else {
                pos + sec.entsize as usize
            };
            let entry = &data[pos..end];
            let pool = &mut pools[pool_idx];
            let new_off = match interns[pool_idx].get(entry) {
                Some(&o) => o,
                None => {
                    // Respect the section's alignment contract for every
                    // entry (e.g. .rodata.str1.8 aligns strings to 8).
                    let a = align;
                    if a > 1 {
                        while pool.data.len() as u64 % a != 0 { pool.data.push(0); }
                    }
                    let o = pool.data.len() as u64;
                    pool.data.extend_from_slice(entry);
                    interns[pool_idx].insert(entry.to_vec(), o);
                    o
                }
            };
            entries.push((pos as u64, new_off, (end - pos) as u64));
            pos = end;
        }
        remaps.insert((oi, si), SecRemap { entries, pool_idx });
    }

    if remaps.is_empty() { return None; }
    Some(StringMergePlan { remaps, pools })
}

/// Apply a merge plan to the link state:
/// 1. Append a synthetic object carrying the deduplicated pools.
/// 2. Register one (linker-internal) global symbol per pool.
/// 3. Rewrite every relocation that targets a merged input section to
///    reference the pool symbol with a rebased addend.
/// 4. Mark the merged input sections dead so layout skips them.
///
/// All-or-nothing: if ANY relocation cannot be safely rewritten the plan is
/// abandoned and the link proceeds without dedup (correctness > size).
/// Returns true if the plan was applied.
pub fn apply_string_merge<G: super::symbols::GlobalSymbolOps>(
    objects: &mut Vec<Elf64Object>,
    globals: &mut FxHashMap<String, G>,
    dead_sections: &mut crate::common::fx_hash::FxHashSet<(usize, usize)>,
    plan: &StringMergePlan,
) -> bool {
    use crate::backend::elf::{STB_GLOBAL, STT_OBJECT};
    use super::types::{Elf64Section, Elf64Symbol};

    let synth_idx = objects.len();
    let pool_sym_name = |i: usize| format!("__lccc.strmerge.{}", i);

    // ── Dry-run: compute every rewrite, disqualifying unmappable target
    // sections individually (GNU ld behavior: a section with an out-of-entry
    // reference — e.g. `lea str-1(%rip)` loop tricks — stays un-merged while
    // every other section still deduplicates). Iterate to a fixed point;
    // failures are rare so this converges in 1-2 passes.
    let debug = std::env::var("LCCC_DEBUG_STRMERGE").is_ok();
    let mut excluded: crate::common::fx_hash::FxHashSet<(usize, usize)> =
        crate::common::fx_hash::FxHashSet::default();
    let mut rewrites: Vec<(usize, usize, usize, usize, i64)> = Vec::new();
    loop {
        rewrites.clear();
        let mut newly_excluded = false;
        'objs: for (oi, obj) in objects.iter().enumerate() {
            for (from_sec, relas) in obj.relocations.iter().enumerate() {
                let from_alloc = obj.sections.get(from_sec)
                    .map(|s| s.flags & SHF_ALLOC != 0).unwrap_or(false);
                for (ri, rela) in relas.iter().enumerate() {
                    let Some(sym) = obj.symbols.get(rela.sym_idx as usize) else { continue };
                    let tsec = sym.shndx as usize;
                    if !plan.contains(oi, tsec) || excluded.contains(&(oi, tsec)) { continue; }
                    let Some(bias) = reloc_bias_x86(rela.rela_type) else {
                        if from_alloc {
                            // Unexpected reloc type (pre-scan should have
                            // caught it): exclude this target section.
                            excluded.insert((oi, tsec));
                            newly_excluded = true;
                            continue 'objs;
                        }
                        continue; // non-alloc referrer (debug): not emitted
                    };
                    let old_off = sym.value as i64 + rela.addend + bias;
                    let mapped = if old_off < 0 { None }
                        else { plan.remap(oi, tsec, old_off as u64) };
                    match mapped {
                        Some((pool_idx, new_off)) => {
                            rewrites.push((oi, from_sec, ri, pool_idx,
                                           new_off as i64 - bias));
                        }
                        None => {
                            if debug {
                                eprintln!("[strmerge] exclude {} sec {}: off {} (rtype {})",
                                    obj.source_name, tsec, old_off, rela.rela_type);
                            }
                            excluded.insert((oi, tsec));
                            newly_excluded = true;
                            continue 'objs;
                        }
                    }
                }
            }
        }
        if !newly_excluded { break; }
    }

    // ── Commit ──────────────────────────────────────────────────────────
    // Synthetic object: NULL section at index 0 (shndx 0 == SHN_UNDEF), pools after.
    let mut sections = vec![Elf64Section {
        name_idx: 0, name: String::new(), sh_type: 0, flags: 0, addr: 0,
        offset: 0, size: 0, link: 0, info: 0, addralign: 0, entsize: 0,
    }];
    let mut section_data = vec![SectionData::empty()];
    let mut symbols = Vec::new();
    for (i, pool) in plan.pools.iter().enumerate() {
        let shndx = sections.len() as u16;
        sections.push(Elf64Section {
            name_idx: 0, name: pool.name.clone(), sh_type: SHT_PROGBITS,
            // Strip SHF_MERGE|SHF_STRINGS from the output: the pool is already
            // deduplicated and may mix entry sizes, so the mergeable contract
            // no longer holds (mold/lld emit plain A here too).
            flags: pool.flags & !(SHF_MERGE | SHF_STRINGS),
            addr: 0, offset: 0,
            size: pool.data.len() as u64, link: 0, info: 0,
            addralign: pool.align, entsize: 0,
        });
        section_data.push(SectionData::owned(pool.data.clone()));
        let sym = Elf64Symbol {
            name_idx: 0, name: SymStr::new(&pool_sym_name(i)),
            // LOCAL binding in the DEFINITION keeps the pool symbol out of
            // --export-dynamic / dynsym; lookups still work because they go
            // through the globals map by name.
            info: STT_OBJECT, other: 2 /* STV_HIDDEN */,
            shndx, value: 0, size: pool.data.len() as u64,
        };
        globals.insert(pool_sym_name(i), G::new_defined(synth_idx, &sym));
        symbols.push(sym);
    }
    let n_secs = sections.len();
    objects.push(Elf64Object {
        sections, symbols, section_data,
        relocations: vec![Vec::new(); n_secs],
        source_name: "<string-merge>".into(),
    });

    // Rewrite relocations: point at a per-object GLOBAL undefined symbol
    // named after the pool (resolved through the globals map like any
    // cross-object reference).
    let mut per_obj_sym: FxHashMap<(usize, usize), u32> = FxHashMap::default();
    for (oi, from_sec, ri, pool_idx, new_addend) in rewrites {
        let sym_idx = match per_obj_sym.get(&(oi, pool_idx)) {
            Some(&i) => i,
            None => {
                let i = objects[oi].symbols.len() as u32;
                objects[oi].symbols.push(Elf64Symbol {
                    name_idx: 0, name: SymStr::new(&pool_sym_name(pool_idx)),
                    info: (STB_GLOBAL << 4) | STT_OBJECT, other: 0,
                    shndx: 0 /* SHN_UNDEF */, value: 0, size: 0,
                });
                per_obj_sym.insert((oi, pool_idx), i);
                i
            }
        };
        let rela = &mut objects[oi].relocations[from_sec][ri];
        rela.sym_idx = sym_idx;
        rela.addend = new_addend;
    }

    // Retire the merged input sections from layout (excluded ones stay).
    for &(oi, si) in plan.merged_sections() {
        if !excluded.contains(&(oi, si)) {
            dead_sections.insert((oi, si));
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::linker_common::types::{Elf64Object, Elf64Section, Elf64Symbol, Elf64Rela};

    fn mk_obj(secs: Vec<(&str, u64, u64, Vec<u8>)>) -> Elf64Object {
        // (name, flags, entsize, data)
        let mut sections = Vec::new();
        let mut section_data = Vec::new();
        for (name, flags, entsize, data) in secs {
            sections.push(Elf64Section {
                name_idx: 0, name: name.to_string(), sh_type: SHT_PROGBITS,
                flags, addr: 0, offset: 0, size: data.len() as u64,
                link: 0, info: 0, addralign: 1, entsize,
            });
            section_data.push(SectionData::owned(data));
        }
        let n = sections.len();
        Elf64Object {
            sections, symbols: Vec::<Elf64Symbol>::new(), section_data,
            relocations: vec![Vec::<Elf64Rela>::new(); n],
            source_name: "test.o".into(),
        }
    }

    #[test]
    fn dedups_identical_strings_across_objects() {
        const MF: u64 = SHF_ALLOC | SHF_MERGE | SHF_STRINGS;
        let o1 = mk_obj(vec![(".rodata.str1.1", MF, 1, b"hi\0shared\0".to_vec())]);
        let o2 = mk_obj(vec![(".rodata.str1.1", MF, 1, b"shared\0yo\0".to_vec())]);
        let objs = vec![o1, o2];
        let dead = crate::common::fx_hash::FxHashSet::default();
        let plan = plan_string_merge(&objs, &dead, |n| n).expect("plan");
        assert_eq!(plan.pools.len(), 1);
        // "hi\0shared\0yo\0" = 13 bytes (shared deduplicated)
        assert_eq!(plan.pools[0].data.len(), 13);
        // o1 "shared" at old off 3 and o2 "shared" at old off 0 map to same place
        let (p1, a) = plan.remap(0, 0, 3).unwrap();
        let (p2, b) = plan.remap(1, 0, 0).unwrap();
        assert_eq!((p1, a), (p2, b));
        // interior offset keeps delta
        let (_, c) = plan.remap(1, 0, 2).unwrap();
        assert_eq!(c, b + 2);
    }

    #[test]
    fn fixed_size_entries() {
        const MF: u64 = SHF_ALLOC | SHF_MERGE;
        let o1 = mk_obj(vec![(".rodata.cst8", MF, 8,
            vec![1,0,0,0,0,0,0,0, 2,0,0,0,0,0,0,0])]);
        let o2 = mk_obj(vec![(".rodata.cst8", MF, 8,
            vec![2,0,0,0,0,0,0,0])]);
        let objs = vec![o1, o2];
        let dead = crate::common::fx_hash::FxHashSet::default();
        let plan = plan_string_merge(&objs, &dead, |n| n).expect("plan");
        assert_eq!(plan.pools[0].data.len(), 16); // 2 unique constants
        let (_, x) = plan.remap(0, 0, 8).unwrap();
        let (_, y) = plan.remap(1, 0, 0).unwrap();
        assert_eq!(x, y);
    }
}
