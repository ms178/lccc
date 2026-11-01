//! Phase 2: Section merging.
//!
//! Collects all allocatable sections from input objects, groups them by output
//! section name, and merges their data with proper alignment. Used by both
//! executable and shared library linking.

use std::rc::Rc;

use super::debug::{MERGE_FLAGS, gnu_merge_flags};
use super::elf_read::*;
use super::relocations::{InputSecRef, MergedSection, output_section_name};
use crate::backend::elf::{SHF_MERGE, SHF_STRINGS};
use crate::backend::linker_common::SectionData;
use crate::backend::linker_common::strmerge_gnu::{self, MergeInput, OffsetMap, Placement};
use crate::common::fx_hash::FxHashMap;

/// The members of merged `SHF_MERGE` groups, each mapped to the group's
/// representative: the first member in layout order, whose contents became
/// the group's pool (see [`merge_alloc_sections`]).
pub type MergeAliases = FxHashMap<(usize, usize), (usize, usize)>;

/// Input sections in the order GNU ld's default script lays them out: link
/// order, then grouped statement by statement (`input_rank`) within each
/// output section, output sections in the order link order first reaches
/// them. Sections that never reach an output section are left out.
fn layout_order(input_objs: &[(String, ElfObject)]) -> Vec<(usize, usize, String)> {
    let mut visit: Vec<(usize, usize, String)> = Vec::new();
    let mut first_seen: FxHashMap<String, usize> = FxHashMap::default();
    for (obj_idx, (_, obj)) in input_objs.iter().enumerate() {
        for (sec_idx, sec) in obj.sections.iter().enumerate() {
            // GNU ld discards every input `.note.gnu.build-id` (`ldelf.c`
            // `ldelf_after_open`), with or without `--build-id`: the output
            // carries its own note or none, never a stale input's ID.
            let stale_build_id = sec.name == ".note.gnu.build-id"
                && input_objs[obj_idx].0 != super::link::BUILD_ID_OBJECT;
            if sec.name.is_empty()
                || stale_build_id
                || sec.sh_type == SHT_SYMTAB
                || sec.sh_type == SHT_STRTAB
                || sec.sh_type == SHT_RELA
                || sec.sh_type == SHT_GROUP
            {
                continue;
            }
            if let Some(out_name) = output_section_name(&sec.name, sec.sh_type, sec.flags) {
                let next = first_seen.len();
                first_seen.entry(out_name.clone()).or_insert(next);
                visit.push((obj_idx, sec_idx, out_name));
            }
        }
    }
    visit.sort_by(|a, b| {
        let key = |(o, s, out): &(usize, usize, String)| {
            (
                first_seen[out],
                input_rank(out, &input_objs[*o].1.sections[*s].name),
            )
        };
        key(a).cmp(&key(b))
    });
    visit
}

/// GNU ld's merging of allocated `SHF_MERGE` inputs (`bfd/merge.c`):
/// `.rodata.str1.8` strings, `.rodata.cst16` and `.srodata.cst8` constants.
///
/// [`strmerge_gnu::merge`] decides, per output section and in layout order,
/// which inputs merge and what each group's pool holds. The pool replaces the
/// representative's contents; the other members leave the layout and resolve
/// through the representative (the returned aliases, which
/// [`merge_sections`] applies). Offsets into every merged input then follow
/// GNU ld's rule: a relocation against a section symbol resolves
/// `map(addend)`, any other symbol `map(value) + addend` -- so section-symbol
/// addends and symbol values are rewritten here, once, and every later
/// address computation works on ordinary sections.
///
/// Runs once per link, before relaxation: relaxation never touches these
/// inputs (they carry no relocations, or they would not merge), and every
/// relaxation probe lays the rewritten objects out the same way.
pub fn merge_alloc_sections(input_objs: &mut [(String, ElfObject)]) -> MergeAliases {
    let mut groups: Vec<(String, Vec<(usize, usize)>)> = Vec::new();
    let mut group_of: FxHashMap<String, usize> = FxHashMap::default();
    for (o, s, out) in layout_order(input_objs) {
        if out == ".note.GNU-stack" || out == ".riscv.attributes" {
            continue;
        }
        let idx = *group_of.entry(out.clone()).or_insert_with(|| {
            groups.push((out, Vec::new()));
            groups.len() - 1
        });
        groups[idx].1.push((o, s));
    }
    let mut aliases = MergeAliases::default();
    let mut maps: FxHashMap<(usize, usize), Rc<OffsetMap>> = FxHashMap::default();
    let mut pools: Vec<((usize, usize), Vec<u8>)> = Vec::new();
    for (_, members) in &groups {
        let mergeable = |&(o, s): &(usize, usize)| {
            let sec = &input_objs[o].1.sections[s];
            sec.flags & SHF_MERGE != 0 && sec.flags & SHF_ALLOC != 0 && sec.sh_type != SHT_NOBITS
        };
        if !members.iter().any(mergeable) {
            continue;
        }
        let inputs: Vec<MergeInput> = members
            .iter()
            .map(|&(o, s)| {
                let obj = &input_objs[o].1;
                let sec = &obj.sections[s];
                MergeInput {
                    data: obj.section_data[s].as_slice(),
                    merge: mergeable(&(o, s)),
                    strings: sec.flags & SHF_STRINGS != 0,
                    entsize: sec.entsize,
                    addralign: sec.addralign,
                    has_relocs: obj.relocations.get(s).is_some_and(|r| !r.is_empty()),
                }
            })
            .collect();
        for (k, placement) in strmerge_gnu::merge(&inputs).into_iter().enumerate() {
            if let Placement::Merged { repr, pool, map } = placement {
                match pool {
                    Some(pool) => pools.push((members[k], pool)),
                    None => {
                        aliases.insert(members[k], members[repr]);
                    }
                }
                maps.insert(members[k], map);
            }
        }
    }
    for ((o, s), pool) in pools {
        let obj = &mut input_objs[o].1;
        obj.sections[s].size = pool.len() as u64;
        obj.section_data[s] = SectionData::owned(pool);
    }
    if maps.is_empty() {
        return aliases;
    }
    for (o, (_, obj)) in input_objs.iter_mut().enumerate() {
        let merged_map = |shndx: u16| {
            (shndx != SHN_UNDEF && shndx < SHN_LORESERVE)
                .then(|| maps.get(&(o, usize::from(shndx))))
                .flatten()
        };
        for sym in &mut obj.symbols {
            if sym.sym_type() != STT_SECTION {
                if let Some(map) = merged_map(sym.shndx) {
                    sym.value = map.map(sym.value);
                }
            }
        }
        let symbols = &obj.symbols;
        for rela in obj.relocations.iter_mut().flatten() {
            let Some(sym) = symbols.get(rela.sym_idx as usize) else {
                continue;
            };
            if sym.sym_type() == STT_SECTION {
                if let Some(map) = merged_map(sym.shndx) {
                    // `_bfd_merged_section_offset` of the section offset the
                    // addend names (a negative one wraps past the end, as in
                    // bfd's unsigned arithmetic).
                    let offset = map.map(sym.value.wrapping_add_signed(rela.addend));
                    rela.addend = offset.cast_signed();
                }
            }
        }
    }
    aliases
}

/// Merge sections from all input objects into output sections.
///
/// Groups input sections by their canonical output name (e.g., `.text.foo` → `.text`),
/// concatenates their data with proper alignment, and returns the merged sections
/// along with a mapping of input section indices to merged section positions.
pub fn merge_sections(
    input_objs: &[(String, ElfObject)],
    aliases: &MergeAliases,
) -> (
    Vec<MergedSection>,
    FxHashMap<String, usize>,
    Vec<InputSecRef>,
) {
    let mut merged_sections: Vec<MergedSection> = Vec::new();
    let mut merged_map: FxHashMap<String, usize> = FxHashMap::default();
    let mut input_sec_refs: Vec<InputSecRef> = Vec::new();

    // Representatives' offsets, for the merged members aliased to them.
    let mut placed: FxHashMap<(usize, usize), (usize, u64)> = FxHashMap::default();
    let visit = layout_order(input_objs);
    for (obj_idx, sec_idx, out_name) in visit {
        let obj = &input_objs[obj_idx].1;
        let sec = &obj.sections[sec_idx];
        let sec_data = &obj.section_data[sec_idx];

        // Skip .note.GNU-stack (we generate our own)
        if out_name == ".note.GNU-stack" {
            continue;
        }

        // `.riscv.attributes`: one output section whose contents
        // `attributes::merge_into_output` replaces with the bfd merge of all
        // inputs. (`.note.*` inputs concatenate like any other section.)
        if out_name == ".riscv.attributes" {
            if !merged_map.contains_key(&out_name) {
                let idx = merged_sections.len();
                merged_map.insert(out_name.clone(), idx);
                merged_sections.push(MergedSection {
                    name: out_name.clone(),
                    sh_type: sec.sh_type,
                    sh_flags: sec.flags,
                    data: sec_data.to_owned_vec(),
                    vaddr: 0,
                    entsize: 0,
                    script_write: false,
                    align: sec.addralign.max(1),
                });
                input_sec_refs.push(InputSecRef {
                    obj_idx,
                    sec_idx,
                    merged_sec_idx: idx,
                    offset_in_merged: 0,
                });
            }
            continue;
        }

        let merged_idx = if let Some(&idx) = merged_map.get(&out_name) {
            idx
        } else {
            let idx = merged_sections.len();
            let is_bss = sec.sh_type == SHT_NOBITS || out_name == ".bss" || out_name == ".sbss";
            // `.sdata` is a data section even when only `.srodata`
            // inputs fill it: GNU ld's script places it in the RW segment.
            let forced_write = if out_name == ".sdata" { SHF_WRITE } else { 0 };
            merged_map.insert(out_name.clone(), idx);
            merged_sections.push(MergedSection {
                name: out_name.clone(),
                sh_type: if is_bss { SHT_NOBITS } else { sec.sh_type },
                // SHF_MERGE/SHF_STRINGS are decided once every input is
                // known (`gnu_merge_flags` below).
                sh_flags: (sec.flags & !MERGE_FLAGS) | forced_write,
                data: Vec::new(),
                vaddr: 0,
                entsize: 0,
                script_write: false,
                align: sec.addralign.max(1),
            });
            idx
        };

        let ms = &mut merged_sections[merged_idx];
        // Union flags from all input sections
        ms.sh_flags |= sec.flags & (SHF_WRITE | SHF_ALLOC | SHF_EXECINSTR | SHF_TLS);
        ms.align = ms.align.max(sec.addralign.max(1));

        // A merged member occupies no space of its own: it resolves through
        // its representative, laid out earlier in the same output section.
        if let Some(repr) = aliases.get(&(obj_idx, sec_idx)) {
            let (merged_sec_idx, offset_in_merged) = placed[repr];
            input_sec_refs.push(InputSecRef {
                obj_idx,
                sec_idx,
                merged_sec_idx,
                offset_in_merged,
            });
            continue;
        }

        // Align within merged section data
        let align = sec.addralign.max(1) as usize;
        let cur_len = ms.data.len();
        let aligned = (cur_len + align - 1) & !(align - 1);
        if aligned > cur_len {
            ms.data.resize(aligned, 0);
        }
        let offset_in_merged = ms.data.len() as u64;

        // Append section data
        if sec.sh_type == SHT_NOBITS {
            ms.data.resize(ms.data.len() + sec.size as usize, 0);
        } else {
            ms.data.extend_from_slice(sec_data);
        }

        placed.insert((obj_idx, sec_idx), (merged_idx, offset_in_merged));
        input_sec_refs.push(InputSecRef {
            obj_idx,
            sec_idx,
            merged_sec_idx: merged_idx,
            offset_in_merged,
        });
    }

    // GNU ld keeps an output's SHF_MERGE/SHF_STRINGS, with the inputs' entry
    // size, exactly while every input agrees (`lang_add_section`): a
    // `-shared` `.rodata` filled only by `.rodata.str1.8` stays `AMS`/1,
    // while plain `.rodata` next to string pools clears both.
    let mut inputs_of: Vec<Vec<(u64, u64)>> = vec![Vec::new(); merged_sections.len()];
    for r in &input_sec_refs {
        let sec = &input_objs[r.obj_idx].1.sections[r.sec_idx];
        inputs_of[r.merged_sec_idx].push((sec.flags, sec.entsize));
    }
    for (ms, inputs) in merged_sections.iter_mut().zip(inputs_of) {
        ms.script_write = ms.name == ".sdata" && inputs.iter().all(|&(f, _)| f & SHF_WRITE == 0);
        let (flags, entsize) = gnu_merge_flags(inputs.into_iter());
        ms.sh_flags = (ms.sh_flags & !MERGE_FLAGS) | flags;
        ms.entsize = entsize;
    }

    (merged_sections, merged_map, input_sec_refs)
}

/// Position of input section `name` within output section `out` under GNU
/// ld's default RISC-V script: the index of the first input statement that
/// matches it, and the name for `SORT(.text.sorted.*)`.  Sections no
/// statement distinguishes rank equal and stay in link order.
fn input_rank<'a>(out: &str, name: &'a str) -> (u8, &'a str) {
    let is =
        |base: &str| name == base || name.strip_prefix(base).is_some_and(|r| r.starts_with('.'));
    match out {
        ".text" => {
            if is(".text.unlikely") || (name.starts_with(".text.") && name.ends_with("_unlikely")) {
                (0, "")
            } else if is(".text.exit") {
                (1, "")
            } else if is(".text.startup") {
                (2, "")
            } else if is(".text.hot") {
                (3, "")
            } else if name.starts_with(".text.sorted.") {
                (4, name)
            } else {
                (5, "")
            }
        }
        ".data.rel.ro" => (
            u8::from(!is(".data.rel.ro.local") && !name.starts_with(".data.rel.ro.local")),
            "",
        ),
        ".sdata" => match name {
            ".srodata.cst16" => (0, ""),
            ".srodata.cst8" => (1, ""),
            ".srodata.cst4" => (2, ""),
            ".srodata.cst2" => (3, ""),
            _ if is(".srodata") => (4, ""),
            _ => (5, ""),
        },
        _ => (0, ""),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::elf::SHT_NULL;
    use crate::backend::linker_common::{Elf64Rela, Elf64Section, Elf64Symbol};

    const AMS: u64 = SHF_ALLOC | SHF_MERGE | SHF_STRINGS;

    fn section(
        name: &str,
        sh_type: u32,
        flags: u64,
        size: usize,
        align: u64,
        entsize: u64,
    ) -> Elf64Section {
        Elf64Section {
            name_idx: 0,
            name: name.to_string(),
            sh_type,
            flags,
            addr: 0,
            offset: 0,
            size: u64::try_from(size).unwrap(),
            link: 0,
            info: 0,
            addralign: align,
            entsize,
        }
    }

    fn symbol(name: &str, sym_type: u8, shndx: u16, value: u64) -> Elf64Symbol {
        Elf64Symbol {
            name_idx: 0,
            name: name.into(),
            info: (STB_LOCAL << 4) | sym_type,
            other: 0,
            shndx,
            value,
            size: 0,
        }
    }

    /// An object with a `.text` (index 1) whose relocations name `relocs`
    /// (symbol, addend), and a `.rodata.str1.1` (index 2) holding `strings`.
    fn object(strings: &[u8], syms: Vec<Elf64Symbol>, relocs: &[(u32, i64)]) -> ElfObject {
        ElfObject {
            sections: vec![
                section("", SHT_NULL, 0, 0, 0, 0),
                section(".text", SHT_PROGBITS, SHF_ALLOC | SHF_EXECINSTR, 16, 4, 0),
                section(".rodata.str1.1", SHT_PROGBITS, AMS, strings.len(), 1, 1),
            ],
            symbols: syms,
            section_data: vec![
                SectionData::owned(Vec::new()),
                SectionData::owned(vec![0; 16]),
                SectionData::owned(strings.to_vec()),
            ],
            relocations: vec![
                Vec::new(),
                relocs
                    .iter()
                    .map(|&(sym_idx, addend)| Elf64Rela {
                        offset: 0,
                        sym_idx,
                        rela_type: 0,
                        addend,
                    })
                    .collect(),
                Vec::new(),
            ],
            source_name: "<test>".to_string(),
            e_flags: 0,
        }
    }

    /// Duplicate and tail strings across two objects merge into the first
    /// object's input; section-symbol addends and symbol values follow the
    /// pool, and the second input aliases the first in the layout.
    #[test]
    fn allocated_strings_merge_like_gnu_ld() {
        let a = object(
            b"hello\0world\0",
            vec![symbol("", STT_SECTION, 2, 0)],
            &[(0, 0), (0, 6), (0, 12)],
        );
        let b = object(
            b"world\0lo\0new\0",
            vec![
                symbol("", STT_SECTION, 2, 0),
                symbol("tail", STT_OBJECT, 2, 6),
            ],
            // "world", "lo" (tail of "hello"), "new", one past the input's end.
            &[(0, 0), (0, 6), (0, 9), (0, 13), (1, 1)],
        );
        let mut objs = vec![("a.o".to_string(), a), ("b.o".to_string(), b)];
        let aliases = merge_alloc_sections(&mut objs);
        assert_eq!(aliases.len(), 1);
        assert_eq!(aliases[&(1, 2)], (0, 2));
        let pool = objs[0].1.section_data[2].as_slice();
        assert_eq!(pool, b"hello\0world\0new\0");
        assert_eq!(objs[0].1.sections[2].size, 16);
        let addends = |o: usize| {
            objs[o].1.relocations[1]
                .iter()
                .map(|r| r.addend)
                .collect::<Vec<_>>()
        };
        // 12 is the end of a's input: GNU ld maps it to the end of the pool.
        assert_eq!(addends(0), [0, 6, 16]);
        // Past the end maps to the end of the pool; a named symbol keeps its
        // own addend and moves itself.
        assert_eq!(addends(1), [6, 3, 12, 16, 1]);
        assert_eq!(objs[1].1.symbols[1].value, 3);

        let (merged, map, refs) = merge_sections(&objs, &aliases);
        let rodata = &merged[map[".rodata"]];
        assert_eq!(rodata.data, pool);
        let at = |o: usize| {
            refs.iter()
                .find(|r| (r.obj_idx, r.sec_idx) == (o, 2))
                .unwrap()
        };
        assert_eq!((at(0).offset_in_merged, at(1).offset_in_merged), (0, 0));
        assert_eq!(at(0).merged_sec_idx, at(1).merged_sec_idx);
        assert_eq!(
            (rodata.sh_flags & MERGE_FLAGS, rodata.entsize),
            (MERGE_FLAGS, 1)
        );
    }

    /// An input with relocations of its own stays as it is, and nothing is
    /// rewritten when no group forms.
    #[test]
    fn inputs_with_relocations_do_not_merge() {
        let mut a = object(b"x\0", vec![symbol("", STT_SECTION, 2, 0)], &[(0, 1)]);
        a.relocations[2].push(Elf64Rela {
            offset: 0,
            sym_idx: 0,
            rela_type: 0,
            addend: 0,
        });
        let b = object(b"x\0", vec![symbol("", STT_SECTION, 2, 0)], &[(0, 1)]);
        let mut objs = vec![("a.o".to_string(), a), ("b.o".to_string(), b)];
        let aliases = merge_alloc_sections(&mut objs);
        assert!(aliases.is_empty());
        assert_eq!(objs[0].1.relocations[1][0].addend, 1);
        let (merged, map, _) = merge_sections(&objs, &aliases);
        assert_eq!(merged[map[".rodata"]].data, b"x\0x\0");
    }

    /// `.sdata` holding only `.srodata` constants is placed as data (layout
    /// sees `SHF_WRITE`) but keeps the inputs' flags in its header.
    #[test]
    fn read_only_sdata_is_script_writable_only() {
        let mut a = object(b"", Vec::new(), &[]);
        a.sections[2] = section(
            ".srodata.cst8",
            SHT_PROGBITS,
            SHF_ALLOC | SHF_MERGE,
            8,
            8,
            8,
        );
        a.section_data[2] = SectionData::owned(vec![1; 8]);
        let objs = vec![("a.o".to_string(), a)];
        let (merged, map, _) = merge_sections(&objs, &MergeAliases::default());
        let sdata = &merged[map[".sdata"]];
        assert!(sdata.script_write);
        assert_ne!(sdata.sh_flags & SHF_WRITE, 0);
    }

    /// The statement order of GNU ld 2.47's `elf64lriscv` `.text`/`.sdata`.
    #[test]
    fn input_rank_follows_the_default_script() {
        let text = [
            ".text.unlikely",
            ".text.unlikely.f",
            ".text.f_unlikely",
            ".text.exit.f",
            ".text.startup",
            ".text.hot.f",
            ".text.sorted.a",
            ".text.sorted.b",
            ".text",
            ".text.f",
        ];
        let ranks: Vec<_> = text.iter().map(|n| input_rank(".text", n)).collect();
        assert!(ranks.windows(2).all(|w| w[0] <= w[1]), "{ranks:?}");
        assert_eq!(input_rank(".text", ".text.f"), input_rank(".text", ".text"));
        // `.text.hotter` is no `.text.hot` input.
        assert_eq!(input_rank(".text", ".text.hotter"), (5, ""));
        let sdata = [
            ".srodata.cst16",
            ".srodata.cst8",
            ".srodata.cst4",
            ".srodata.cst2",
            ".srodata",
            ".srodata.str1.8",
            ".sdata",
            ".sdata.x",
        ];
        let ranks: Vec<_> = sdata.iter().map(|n| input_rank(".sdata", n)).collect();
        assert!(ranks.windows(2).all(|w| w[0] <= w[1]), "{ranks:?}");
        assert!(input_rank(".sdata", ".srodata") < input_rank(".sdata", ".sdata"));
        assert_eq!(input_rank(".data", ".data.x"), input_rank(".data", ".data"));
        assert!(
            input_rank(".data.rel.ro", ".data.rel.ro.local.x")
                < input_rank(".data.rel.ro", ".data.rel.ro")
        );
        assert_eq!(input_rank(".data.rel.ro", ".data.rel.ro.local"), (0, ""));
    }
}
