//! Linker-script-driven ELF executable emission (x86-64).
//!
//! Implements the layout semantics of GNU ld for full `SECTIONS` scripts:
//! the location counter walks the script top-to-bottom, input sections are
//! assigned to output sections by glob pattern (first match wins, in script
//! order), symbols are defined at their point of assignment, PHDRS are
//! honored, and `AT()` gives independent load addresses (LMA).
//!
//! This is what links the Linux kernel's `vmlinux.lds`. It produces a fully
//! static ET_EXEC image with no dynamic sections; relocation types are the
//! static x86-64 set (PC32/PLT32/32/32S/64 and TLS LE forms).

use crate::backend::elf::{push_strtab_name, elf64_sym_entry};
use crate::common::fx_hash::{FxHashMap, FxHashSet};

use super::elf::*;
use crate::backend::linker_common::{
    self,
    linker_script::{
        self, LinkerScript, SectionsItem, SecItem, OutputSecDef, Assignment,
        AssignOp, EvalCtx, EvalError, SortKind, glob_match, eval_expr,
    },
};

type Object = linker_common::Elf64Object;

const PT_LOAD_: u32 = 1;
const PT_TLS_: u32 = 7;
const PT_NOTE_: u32 = 4;
const ELF64_EHDR_SIZE: u64 = 64;
const ELF64_PHDR_SIZE: u64 = 56;

/// Bytes occupied by the ELF header and the script-declared program-header
/// table. GNU ld exposes this value to scripts as `SIZEOF_HEADERS`.
/// Bytes occupied by the ELF header plus all program headers.
///
/// This is what `SIZEOF_HEADERS` evaluates to, so it must agree exactly with
/// the number of program headers actually written. `extra_phdrs` accounts for
/// headers the linker synthesises rather than the script declaring them (today:
/// PT_TLS). Undercounting places the first section on top of the last program
/// header, and the entry point then lands in header bytes -- observed as an
/// immediate SIGILL.
fn script_header_size_with(script: &LinkerScript, extra_phdrs: usize) -> u64 {
    ELF64_EHDR_SIZE
        + ELF64_PHDR_SIZE * (script.phdrs.len().max(1) + extra_phdrs) as u64
}

fn script_header_size(script: &LinkerScript) -> u64 {
    script_header_size_with(script, 0)
}

/// Synthetic dynamic-link information required by an ET_DYN linker-script
/// image. Linux's vDSO script names `.dynsym`, `.dynstr`, `.hash`,
/// `.gnu.hash`, `.gnu.version*`, and `.dynamic`, but those sections are linker
/// products rather than compiler inputs. Treating unmatched patterns as empty
/// creates an ET_DYN file that looks linked yet cannot be resolved by ld.so.
struct ScriptDynamic {
    object_index: usize,
    exports: Vec<String>,
    name_offsets: Vec<u32>,
    version_name: String,
    soname_offset: Option<u32>,
    verdef_count: u64,
}

fn push_u32(buf: &mut Vec<u8>, value: u32) {
    buf.extend_from_slice(&value.to_le_bytes());
}

fn build_sysv_hash(names: &[String]) -> Vec<u8> {
    let buckets = names.len().next_power_of_two().max(1);
    let mut heads = vec![0u32; buckets];
    let mut chains = vec![0u32; names.len() + 1];
    for (i, name) in names.iter().enumerate() {
        let index = (i + 1) as u32;
        let bucket = linker_common::sysv_hash(name.as_bytes()) as usize % buckets;
        if heads[bucket] == 0 {
            heads[bucket] = index;
        } else {
            let mut tail = heads[bucket] as usize;
            while chains[tail] != 0 { tail = chains[tail] as usize; }
            chains[tail] = index;
        }
    }
    let mut out = Vec::with_capacity(8 + (buckets + chains.len()) * 4);
    push_u32(&mut out, buckets as u32);
    push_u32(&mut out, chains.len() as u32);
    for value in heads { push_u32(&mut out, value); }
    for value in chains { push_u32(&mut out, value); }
    out
}

/// Build a GNU hash table and return the required dynsym order. GNU hash
/// requires all symbols belonging to a bucket to be contiguous.
fn build_gnu_hash(mut names: Vec<String>) -> (Vec<u8>, Vec<String>) {
    // Bucket count: a power of two near the symbol count, so chains stay
    // short. GNU ld uses a load factor around 1 symbol per bucket for exactly
    // this reason -- every ld.so lookup walks a chain, so an undersized table
    // is a permanent runtime tax on the consumer, not a link-time saving.
    //
    // A fixed cap (an earlier revision clamped this to 64) is a trap: it looks
    // harmless on a vDSO with four exports and silently degrades to ~78-symbol
    // chains at 5 000 exports, versus ~1.5 uncapped -- measured. The table
    // costs 4 bytes per bucket, so sizing it properly is cheap.
    let nbuckets = names.len().next_power_of_two().max(1);
    names.sort_by_key(|name| {
        let hash = linker_common::gnu_hash(name.as_bytes());
        (hash % nbuckets as u32, hash, name.clone())
    });
    let hashes: Vec<u32> = names.iter()
        .map(|name| linker_common::gnu_hash(name.as_bytes()))
        .collect();
    let bloom_size = ((names.len() + 63) / 64).next_power_of_two().max(1);
    let bloom_shift = 6u32;
    let mut bloom = vec![0u64; bloom_size];
    for &hash in &hashes {
        let word = (hash as usize / 64) % bloom_size;
        bloom[word] |= 1u64 << (hash % 64);
        bloom[word] |= 1u64 << ((hash >> bloom_shift) % 64);
    }
    let mut buckets = vec![0u32; nbuckets];
    let mut chains = vec![0u32; names.len()];
    for (i, &hash) in hashes.iter().enumerate() {
        let bucket = hash as usize % nbuckets;
        if buckets[bucket] == 0 { buckets[bucket] = (i + 1) as u32; }
        let last = i + 1 == hashes.len()
            || hashes[i + 1] % nbuckets as u32 != hash % nbuckets as u32;
        chains[i] = (hash & !1) | u32::from(last);
    }
    let mut out = Vec::new();
    push_u32(&mut out, nbuckets as u32);
    push_u32(&mut out, 1); // first hashed dynsym follows the null entry
    push_u32(&mut out, bloom_size as u32);
    push_u32(&mut out, bloom_shift);
    for value in bloom { out.extend_from_slice(&value.to_le_bytes()); }
    for value in buckets { push_u32(&mut out, value); }
    for value in chains { push_u32(&mut out, value); }
    (out, names)
}

fn push_verdef(buf: &mut Vec<u8>, index: u16, name: &str, name_off: u32, next: u32) {
    // Elf64_Verdef followed by one Elf64_Verdaux.
    buf.extend_from_slice(&1u16.to_le_bytes());
    buf.extend_from_slice(&0u16.to_le_bytes());
    buf.extend_from_slice(&index.to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes());
    push_u32(buf, linker_common::sysv_hash(name.as_bytes()));
    push_u32(buf, 20);
    push_u32(buf, next);
    push_u32(buf, name_off);
    push_u32(buf, 0);
}

fn synthetic_section(name: &str, sh_type: u32, flags: u64,
                     align: u64, entsize: u64, data: Vec<u8>) -> (linker_common::Elf64Section, Vec<u8>) {
    let size = data.len() as u64;
    (linker_common::Elf64Section {
        name_idx: 0, name: name.into(), sh_type, flags, addr: 0, offset: 0,
        size, link: 0, info: 0, addralign: align, entsize,
    }, data)
}

fn make_script_dynamic_object(
    object_index: usize,
    mut export_candidates: Vec<String>,
    version: &linker_common::VersionScript,
    soname: Option<&str>,
    base_name: &str,
    bsymbolic: bool,
) -> (Object, ScriptDynamic) {
    // GNU ld exposes the version-node name itself as an absolute dynamic
    // symbol.  Besides matching its ABI, this lets consumers identify the
    // default version through either ELF hash table.
    if !export_candidates.iter().any(|name| name == &version.version_name) {
        export_candidates.push(version.version_name.clone());
    }
    let (gnu_hash, exports) = build_gnu_hash(export_candidates);
    let mut dynstr = vec![0u8];
    let mut name_offsets = Vec::with_capacity(exports.len());
    for name in &exports {
        name_offsets.push(dynstr.len() as u32);
        dynstr.extend_from_slice(name.as_bytes());
        dynstr.push(0);
    }
    let soname_offset = soname.map(|name| {
        let off = dynstr.len() as u32;
        dynstr.extend_from_slice(name.as_bytes());
        dynstr.push(0);
        off
    });
    let version_name_offset = name_offsets[exports.iter()
        .position(|name| name == &version.version_name)
        .expect("version symbol was inserted")];

    let sysv_hash = build_sysv_hash(&exports);
    let dynsym = vec![0u8; (exports.len() + 1) * 24];
    let mut versym = Vec::with_capacity((exports.len() + 1) * 2);
    versym.extend_from_slice(&0u16.to_le_bytes());
    for _ in &exports { versym.extend_from_slice(&2u16.to_le_bytes()); }
    let mut verdef = Vec::new();
    let base_off = if soname == Some(base_name) {
        soname_offset.expect("SONAME was inserted in dynstr")
    } else {
        let off = dynstr.len() as u32;
        dynstr.extend_from_slice(base_name.as_bytes());
        dynstr.push(0);
        off
    };
    push_verdef(&mut verdef, 1, base_name, base_off, 28);
    push_verdef(&mut verdef, 2, &version.version_name, version_name_offset, 0);
    let verdef_count = 2u64;

    // Fixed tags plus optional SONAME and symbolic binding. Values that depend
    // on layout are patched after sections receive final virtual addresses.
    let dynamic_entries = 10usize + usize::from(soname.is_some())
        + if bsymbolic { 2 } else { 0 };
    let dynamic = vec![0u8; dynamic_entries * 16];

    let mut sections = Vec::new();
    let mut section_data = Vec::new();
    let pairs = [
        synthetic_section("", SHT_NULL_, 0, 0, 0, Vec::new()),
        synthetic_section(".hash", 5, SHF_ALLOC_, 8, 4, sysv_hash),
        synthetic_section(".gnu.hash", 0x6fff_fff6, SHF_ALLOC_, 8, 0, gnu_hash),
        synthetic_section(".dynsym", 11, SHF_ALLOC_, 8, 24, dynsym),
        synthetic_section(".dynstr", SHT_STRTAB_, SHF_ALLOC_, 1, 0, dynstr),
        synthetic_section(".gnu.version", 0x6fff_ffff, SHF_ALLOC_, 2, 2, versym),
        synthetic_section(".gnu.version_d", 0x6fff_fffd, SHF_ALLOC_, 8, 0, verdef),
        synthetic_section(".dynamic", 6, SHF_ALLOC_ | SHF_WRITE_, 8, 16, dynamic),
    ];
    for (section, data) in pairs {
        sections.push(section);
        // Synthesised sections own their bytes; `SectionData::owned` is the
        // constructor for that case (input-file sections are Arc windows).
        section_data.push(linker_common::SectionData::owned(data));
    }
    let section_count = sections.len();
    let object = Object {
        sections, symbols: Vec::new(), section_data,
        relocations: vec![Vec::new(); section_count],
        source_name: "<script-dynamic>".into(),
    };
    let plan = ScriptDynamic {
        object_index, exports, name_offsets, version_name: version.version_name.clone(),
        soname_offset, verdef_count,
    };
    (object, plan)
}

/// One placed input section.
struct Placed {
    obj_idx: usize,
    sec_idx: usize,
    vaddr: u64,
    size: u64,
}

/// One output section after layout.
struct OutSec {
    name: String,
    vaddr: u64,
    lma: u64,
    size: u64,
    align: u64,
    sh_type: u32,
    flags: u64,
    file_offset: u64,
    is_alloc: bool,
    nobits: bool,
    phdrs: Vec<String>,
    fill: Option<u64>,
    placed: Vec<Placed>,
}

pub fn link_with_script(
    objects: &[Object],
    script_src: &str,
    output_path: &str,
    emit_symtab: bool,
    is_pie: bool,
    // `--emit-relocs`: retain the (already applied) relocations as `.rela.*`
    // sections in the output. Required by the Linux kernel's `arch/x86/tools/
    // relocs` pass for CONFIG_RELOCATABLE / KASLR.
    emit_relocs: bool,
    soname: Option<&str>,
    bsymbolic: bool,
    max_page_size: u64,
) -> Result<(), String> {
    let script: LinkerScript = linker_script::parse_linker_script(script_src)?;

    // ── Global symbol table from objects (defined globals + weak) ──
    // name -> (obj_idx, sec_idx(SHN), value, size, info)
    let mut def_syms: FxHashMap<String, (usize, u16, u64, u64, u8)> = FxHashMap::default();
    for (oi, obj) in objects.iter().enumerate() {
        for sym in &obj.symbols {
            if sym.name.is_empty() || sym.is_local() { continue; }
            if sym.is_undefined() || sym.shndx == SHN_COMMON { continue; }
            let replace = match def_syms.get(sym.name.as_str()) {
                None => true,
                Some(&(_, _, _, _, info)) => (info >> 4) == STB_WEAK && sym.is_global(),
            };
            if replace {
                def_syms.insert(sym.name.to_string(),
                    (oi, sym.shndx, sym.value, sym.size, sym.info));
            }
        }
    }

    // Symbols the *inputs* declared STV_HIDDEN/STV_INTERNAL. Visibility is an
    // ABI property decided by the compiler; the linker must honour it before
    // any version-script globbing is applied.
    let mut hidden_object_syms: FxHashSet<String> = FxHashSet::default();
    for obj in objects.iter() {
        for sym in &obj.symbols {
            if sym.name.is_empty() || sym.is_undefined() { continue; }
            // STV_HIDDEN = 2, STV_INTERNAL = 1, STV_PROTECTED = 3 (still exported).
            let vis = sym.visibility();
            if vis == 1 || vis == 2 {
                hidden_object_syms.insert(sym.name.to_string());
            }
        }
    }

    // ET_DYN script links (notably Linux's vDSO) require linker-created
    // dynamic metadata. Materialise it as a synthetic input object so the
    // script's ordinary `*(.dynsym)` / `*(.dynamic)` rules decide placement,
    // exactly as they do in GNU ld.
    let embedded_version = linker_common::VersionScript::parse_text(script_src);
    let wants_dynamic = is_pie && script.sections.iter().any(|item|
        matches!(item, SectionsItem::Output(def) if def.name == ".dynamic"));
    let mut owned_objects: Option<Vec<Object>> = None;
    let mut script_dynamic: Option<ScriptDynamic> = None;
    if wants_dynamic {
        if let Some(version) = embedded_version.as_ref() {
            // STV_HIDDEN / STV_INTERNAL symbols are deliberately not part of
            // the ABI: they must stay out of .dynsym even when the version
            // script says `global: *`. GNU ld applies visibility first and the
            // version script second; doing it the other way round exports the
            // vDSO's internal helpers and every -fvisibility=hidden symbol in
            // a script-linked shared object.
            let mut exports: Vec<String> = def_syms.iter()
                .filter(|(name, _)| version.matches_global(name))
                .filter(|(name, _)| !hidden_object_syms.contains(name.as_str()))
                .map(|(name, _)| name.clone())
                .collect();
            exports.sort();
            let mut all = objects.to_vec();
            let object_index = all.len();
            let output_base_name = soname.map(str::to_owned).unwrap_or_else(|| {
                std::path::Path::new(output_path).file_name()
                    .and_then(|name| name.to_str()).unwrap_or("a.out").to_owned()
            });
            let (dynamic_object, plan) = make_script_dynamic_object(
                object_index, exports, version, soname, &output_base_name,
                bsymbolic);
            all.push(dynamic_object);
            owned_objects = Some(all);
            script_dynamic = Some(plan);
        }
    }
    let objects: &[Object] = owned_objects.as_deref().unwrap_or(objects);

    // ── Assign input sections to output sections ──
    // Collect allocatable + INFO-listed input sections.
    let mut unassigned: Vec<(usize, usize)> = Vec::new();
    for (oi, obj) in objects.iter().enumerate() {
        for (si, sec) in obj.sections.iter().enumerate() {
            // Ordinary object string/symbol/relocation tables are linker
            // metadata rather than input sections.  An allocated string table
            // is different: .dynstr must participate in script matching.
            if matches!(sec.sh_type, SHT_NULL_ | SHT_SYMTAB_ | SHT_RELA_ | SHT_REL_ | SHT_GROUP_)
                || (sec.sh_type == SHT_STRTAB_ && sec.flags & SHF_ALLOC_ == 0)
            { continue; }
            if sec.size == 0 && sec.name.is_empty() { continue; }
            unassigned.push((oi, si));
        }
    }

    // Output section definitions in script order.
    let out_defs: Vec<&OutputSecDef> = script.sections.iter().filter_map(|it| match it {
        SectionsItem::Output(o) => Some(o),
        _ => None,
    }).collect();

    // For each (output def, item order), record which input sections match.
    // First-match-wins across the whole script.
    let mut assigned: FxHashMap<(usize, usize), usize> = FxHashMap::default(); // input -> out_def index
    let mut assigned_item: FxHashMap<(usize, usize), usize> = FxHashMap::default(); // input -> item index
    let mut discard: FxHashSet<(usize, usize)> = FxHashSet::default();

    for (di, def) in out_defs.iter().enumerate() {
        for (ii, item) in def.items.iter().enumerate() {
            let SecItem::Input(spec) = item else { continue };
            for &(oi, si) in unassigned.iter() {
                if assigned.contains_key(&(oi, si)) || discard.contains(&(oi, si)) { continue; }
                let name = &objects[oi].sections[si].name;
                if spec.patterns.iter().any(|p| glob_match(p, name)) {
                    if def.name == "/DISCARD/" {
                        discard.insert((oi, si));
                    } else {
                        assigned.insert((oi, si), di);
                        assigned_item.insert((oi, si), ii);
                    }
                }
            }
        }
    }

    // Orphan sections: allocatable sections not matched by any pattern get
    // appended after the last section (GNU --orphan-handling=place). For the
    // kernel script every real section is matched; orphans are mostly
    // .comment/.note.* metadata. Non-alloc orphans are dropped from the image
    // (we still emit .symtab/.strtab/.shstrtab ourselves).
    let mut orphans: Vec<(usize, usize)> = Vec::new();
    for &(oi, si) in unassigned.iter() {
        if assigned.contains_key(&(oi, si)) || discard.contains(&(oi, si)) { continue; }
        let sec = &objects[oi].sections[si];
        if sec.flags & SHF_ALLOC_ != 0 && sec.size > 0 {
            orphans.push((oi, si));
        }
    }

    // ── Layout: walk the script, maintaining dot ──
    let mut symbols: FxHashMap<String, u64> = FxHashMap::default();
    // The parser represents SIZEOF_HEADERS as this reserved symbol because the
    // value depends on PHDRS and therefore is not a context-free constant.
    // Seed it before the first assignment is evaluated: Linux's vDSO script
    // starts with `. = SIZEOF_HEADERS`, before any output section exists.
    // A synthesised PT_TLS adds a program header, so it must be counted here
    // too or SIZEOF_HEADERS under-reserves and the first section overlaps the
    // header table. Predict it from the inputs, since output sections do not
    // exist yet: any allocated SHF_TLS input section will produce one unless
    // the script already declares PT_TLS.
    let will_add_tls_phdr = objects.iter().any(|o| o.sections.iter()
            .any(|sec| (sec.flags & SHF_TLS_) != 0 && (sec.flags & SHF_ALLOC_) != 0
                       && sec.size > 0))
        && !script.phdrs.iter().any(|d| d.ptype == PT_TLS_);
    symbols.insert("__SIZEOF_HEADERS".into(),
                   script_header_size_with(&script, usize::from(will_add_tls_phdr)));
    // GNU ld attributes a script symbol to the output section that was
    // current at its point of definition (e.g. `_end` after .brk belongs to
    // .brk even though .modinfo starts at the same address).
    let mut sym_home: FxHashMap<String, String> = FxHashMap::default();
    // Symbols defined by HIDDEN()/PROVIDE_HIDDEN(): emitted STV_HIDDEN and
    // kept out of .dynsym. Losing this bit exports internal markers from every
    // shared object built with a custom script.
    let mut hidden_syms: FxHashSet<String> = FxHashSet::default();
    let mut cur_out_name: Option<String> = None;
    let mut sections_meta: FxHashMap<String, (u64, u64, u64, u64)> = FxHashMap::default();
    let mut out_secs: Vec<OutSec> = Vec::new();
    let mut dot: u64 = 0;

    // MEMORY regions: base/limit for the overflow check, plus a per-region
    // allocation cursor so `> region` sections pack independently of `dot`.
    let mut region_bounds: FxHashMap<String, (u64, u64)> = FxHashMap::default();
    let mut region_cursor: FxHashMap<String, u64> = FxHashMap::default();
    for r in &script.memory {
        let empty_syms: FxHashMap<String, u64> = FxHashMap::default();
        let empty_secs: FxHashMap<String, (u64, u64, u64, u64)> = FxHashMap::default();
        let ctx = linker_script::EvalCtx {
            dot: 0, symbols: &empty_syms, sections: &empty_secs, segment_starts: None,
        };
        let origin = linker_script::eval_expr(&r.origin, &ctx)
            .map_err(|_| format!("MEMORY region '{}': ORIGIN is not a constant", r.name))?;
        let length = linker_script::eval_expr(&r.length, &ctx)
            .map_err(|_| format!("MEMORY region '{}': LENGTH is not a constant", r.name))?;
        region_bounds.insert(r.name.clone(), (origin, origin.saturating_add(length)));
        region_cursor.insert(r.name.clone(), origin);
    }

    // Deferred assignments that referenced not-yet-defined symbols; re-run to
    // fixed point after layout.
    let mut deferred: Vec<(Assignment, u64)> = Vec::new(); // (assignment, dot at that point)

    // Symbol values that live inside output sections get resolved lazily:
    // we need input-section vaddrs first. We do a single forward pass, which
    // works because GNU scripts define symbols before use except for
    // cross-references handled via deferral.
    //
    // Pre-pass: seed `symbols` with per-object section-symbol placeholders? No —
    // symbol lookups during layout come from assignments only. References to
    // object symbols (e.g. `phys_startup_64 = ABSOLUTE(startup_64 - X)`) are
    // resolved through def_syms once their section has been placed.

    // Helper to resolve an object symbol to its final vaddr if its section is
    // already placed. Uses a by-(obj,sec) lookup of placed sections.
    let mut placed_map: FxHashMap<(usize, usize), u64> = FxHashMap::default();
    // Owning output-section index for each placed input section. Used for
    // relocation application: an address-based lookup is ambiguous because
    // empty marker sections (.init.begin) share addresses with real ones.
    let mut placed_owner: FxHashMap<(usize, usize), usize> = FxHashMap::default();

    // Resolve symbol via assignments or object definitions.
    fn lookup_sym(
        name: &str,
        symbols: &FxHashMap<String, u64>,
        def_syms: &FxHashMap<String, (usize, u16, u64, u64, u8)>,
        placed_map: &FxHashMap<(usize, usize), u64>,
    ) -> Option<u64> {
        if let Some(&v) = symbols.get(name) { return Some(v); }
        if let Some(&(oi, shndx, value, _, _)) = def_syms.get(name) {
            if shndx == SHN_ABS { return Some(value); }
            if let Some(&base) = placed_map.get(&(oi, shndx as usize)) {
                return Some(base + value);
            }
        }
        None
    }

    // Evaluate with object-symbol fallback; returns Err(name) on undefined.
    fn eval_full(
        e: &linker_script::Expr,
        dot: u64,
        symbols: &FxHashMap<String, u64>,
        sections_meta: &FxHashMap<String, (u64, u64, u64, u64)>,
        def_syms: &FxHashMap<String, (usize, u16, u64, u64, u8)>,
        placed_map: &FxHashMap<(usize, usize), u64>,
    ) -> Result<u64, String> {
        // Fast path: try direct eval; on undefined symbol, augment.
        let ctx = EvalCtx { dot, symbols, sections: sections_meta, segment_starts: None };
        match eval_expr(e, &ctx) {
            Ok(v) => Ok(v),
            Err(EvalError::UndefinedSymbol(n)) => {
                if let Some(v) = lookup_sym(&n, symbols, def_syms, placed_map) {
                    // clone-augment (rare path)
                    let mut aug = symbols.clone();
                    aug.insert(n, v);
                    // may still hit more undefined symbols; recurse via loop
                    let mut aug2 = aug;
                    loop {
                        let ctx2 = EvalCtx { dot, symbols: &aug2, sections: sections_meta, segment_starts: None };
                        match eval_expr(e, &ctx2) {
                            Ok(v) => return Ok(v),
                            Err(EvalError::UndefinedSymbol(n2)) => {
                                match lookup_sym(&n2, &aug2, def_syms, placed_map) {
                                    Some(v2) => { aug2.insert(n2, v2); }
                                    None => return Err(n2),
                                }
                            }
                            Err(EvalError::UnknownSection(s)) => return Err(format!("section {}", s)),
                            Err(EvalError::AssertFailed(m)) => return Err(format!("ASSERT failed: {}", m)),
                        }
                    }
                } else {
                    Err(n)
                }
            }
            Err(EvalError::UnknownSection(s)) => Err(format!("section {}", s)),
            Err(EvalError::AssertFailed(m)) => Err(format!("ASSERT failed: {}", m)),
        }
    }

    for item in &script.sections {
        match item {
            SectionsItem::Assign(a) => {
                match eval_full(&a.expr, dot, &symbols, &sections_meta, &def_syms, &placed_map) {
                    Ok(v) => {
                        if a.symbol == "." {
                            dot = if a.op == AssignOp::Add { dot.wrapping_add(v) } else { v };
                        } else {
                            let nv = if a.op == AssignOp::Add {
                                symbols.get(&a.symbol).copied().unwrap_or(0).wrapping_add(v)
                            } else { v };
                            if !(a.provide && (symbols.contains_key(&a.symbol) || def_syms.contains_key(&a.symbol))) {
                                symbols.insert(a.symbol.clone(), nv);
                                if a.hidden { hidden_syms.insert(a.symbol.clone()); }
                                if let Some(ref n) = cur_out_name {
                                    sym_home.insert(a.symbol.clone(), n.clone());
                                }
                            }
                        }
                    }
                    Err(_) => deferred.push((a.clone(), dot)),
                }
            }
            SectionsItem::Assert(e, msg) => {
                // Defer asserts until after layout (sections sized).
                deferred.push((Assignment {
                    symbol: "__assert__".into(), op: AssignOp::Set,
                    expr: linker_script::Expr::Assert(Box::new(e.clone()), msg.clone()),
                    provide: false,
                    hidden: false,
                }, dot));
            }
            SectionsItem::Output(def) => {
                if def.name == "/DISCARD/" { continue; }
                let di = out_defs.iter().position(|d| std::ptr::eq(*d, def)).unwrap();

                // Non-alloc convention: explicit address 0 (debug sections) or (INFO).
                let explicit_zero = matches!(def.address, Some(linker_script::Expr::Num(0)));
                let is_alloc = !def.info && !explicit_zero;

                // Section start: explicit address, region base, or dot.
                //
                // `> region` sets the VMA from the region's allocation cursor,
                // so consecutive sections assigned to a region pack into it
                // independently of the global dot. An explicit address still
                // wins, matching GNU ld.
                let mut sec_start = if let Some(ref ae) = def.address {
                    eval_full(ae, dot, &symbols, &sections_meta, &def_syms, &placed_map)
                        .map_err(|e| format!("cannot evaluate address of {}: {}", def.name, e))?
                } else if let Some(ref rname) = def.region {
                    *region_cursor.get(rname.as_str()).ok_or_else(|| format!(
                        "linker script: section {} assigned to undefined MEMORY region '{}'",
                        def.name, rname))?
                } else {
                    dot
                };
                if let Some(ref al) = def.align {
                    let a = eval_full(al, sec_start, &symbols, &sections_meta, &def_syms, &placed_map)
                        .unwrap_or(1).max(1);
                    sec_start = (sec_start + a - 1) & !(a - 1);
                } else if def.address.is_none() {
                    // GNU ld aligns the OUTPUT section start to the largest
                    // alignment among its (first-item) input sections, not
                    // just each input within the section. Without this,
                    // .text can start at an unaligned dot (observed as a
                    // 0x35-byte skew vs GNU ld on the kernel decompressor).
                    let di_self = di;
                    let max_in_align = unassigned.iter()
                        .filter(|k| assigned.get(k) == Some(&di_self))
                        .map(|&(oi, si)| objects[oi].sections[si].addralign.max(1))
                        .max().unwrap_or(1);
                    if max_in_align > 1 {
                        sec_start = (sec_start + max_in_align - 1) & !(max_in_align - 1);
                    }
                }

                let mut cur = sec_start;
                let mut placed: Vec<Placed> = Vec::new();
                let mut max_align: u64 = 1;
                let mut sh_type = SHT_PROGBITS_;
                let mut flags: u64 = 0;
                let mut any_progbits = false;

                // Gather matched inputs per item, in item order.
                for (ii, sitem) in def.items.iter().enumerate() {
                    match sitem {
                        SecItem::Assign(a) => {
                            // In-section assignments evaluate against the walking dot.
                            match eval_full(&a.expr, cur, &symbols, &sections_meta, &def_syms, &placed_map) {
                                Ok(v) => {
                                    if a.symbol == "." {
                                        let nv = if a.op == AssignOp::Add { cur.wrapping_add(v) } else { v };
                                        if nv < cur && a.op == AssignOp::Set {
                                            // moving dot backwards is an error in ld; clamp
                                            cur = nv.max(sec_start);
                                        } else {
                                            cur = nv;
                                        }
                                    } else {
                                        let nv = if a.op == AssignOp::Add {
                                            symbols.get(&a.symbol).copied().unwrap_or(0).wrapping_add(v)
                                        } else { v };
                                        if !(a.provide && (symbols.contains_key(&a.symbol) || def_syms.contains_key(&a.symbol))) {
                                            symbols.insert(a.symbol.clone(), nv);
                                            if a.hidden { hidden_syms.insert(a.symbol.clone()); }
                                            sym_home.insert(a.symbol.clone(), def.name.clone());
                                        }
                                    }
                                }
                                Err(_) => deferred.push((a.clone(), cur)),
                            }
                        }
                        SecItem::Assert(e, msg) => {
                            deferred.push((Assignment {
                                symbol: "__assert__".into(), op: AssignOp::Set,
                                expr: linker_script::Expr::Assert(Box::new(e.clone()), msg.clone()),
                                provide: false,
                    hidden: false,
                            }, cur));
                        }
                        SecItem::Constructors => { /* ELF: no-op */ }
                        SecItem::Input(spec) => {
                            // Collect matching inputs assigned to (di, ii).
                            let mut ins: Vec<(usize, usize)> = unassigned.iter()
                                .filter(|k| assigned.get(k) == Some(&di)
                                    && assigned_item.get(k) == Some(&ii))
                                .copied().collect();
                            match spec.sort {
                                SortKind::ByName | SortKind::ByInitPriority => {
                                    ins.sort_by(|a, b| {
                                        objects[a.0].sections[a.1].name
                                            .cmp(&objects[b.0].sections[b.1].name)
                                            .then(a.cmp(b))
                                    });
                                }
                                SortKind::ByAlignment => {
                                    ins.sort_by(|a, b| {
                                        objects[b.0].sections[b.1].addralign
                                            .cmp(&objects[a.0].sections[a.1].addralign)
                                            .then(a.cmp(b))
                                    });
                                }
                                SortKind::None => {
                                    // GNU ld: command-line (object) order, section order
                                    ins.sort();
                                }
                            }
                            for (oi, si) in ins {
                                let sec = &objects[oi].sections[si];
                                let a = sec.addralign.max(1);
                                cur = (cur + a - 1) & !(a - 1);
                                placed_map.insert((oi, si), cur);
                                placed_owner.insert((oi, si), out_secs.len());
                                placed.push(Placed { obj_idx: oi, sec_idx: si, vaddr: cur, size: sec.size });
                                cur += sec.size;
                                if a > max_align { max_align = a; }
                                if sec.sh_type == SHT_PROGBITS_ { any_progbits = true; }
                                if sec.sh_type != SHT_NOBITS_ && sec.sh_type != SHT_PROGBITS_ && sh_type == SHT_PROGBITS_ && !any_progbits {
                                    sh_type = sec.sh_type;
                                }
                                flags |= sec.flags & (SHF_WRITE_ | SHF_ALLOC_ | SHF_EXECINSTR_ | SHF_TLS_);
                            }
                        }
                    }
                }

                if std::env::var("LCCC_LD_MAP").is_ok() {
                    for p in &placed {
                        eprintln!("MAP {} 0x{:x} 0x{:x} {}({})",
                            def.name, p.vaddr, p.size,
                            objects[p.obj_idx].source_name,
                            objects[p.obj_idx].sections[p.sec_idx].name);
                    }
                }

                let size = cur.saturating_sub(sec_start);
                // NOBITS only if ALL inputs are NOBITS and there was no ". += N" gap
                // requiring file content. Kernel .bss/.brk are file-backed zeros in
                // GNU ld only when fill is demanded; standard behavior: keep NOBITS.
                let nobits = placed.iter().all(|p|
                    objects[p.obj_idx].sections[p.sec_idx].sh_type == SHT_NOBITS_)
                    && def.fill.is_none()
                    && (def.name.starts_with(".bss") || def.name.starts_with(".brk")
                        || placed.iter().len() > 0 && placed.iter().all(|p|
                            objects[p.obj_idx].sections[p.sec_idx].sh_type == SHT_NOBITS_)
                        && !placed.is_empty());
                let nobits = nobits && !placed.is_empty() || (placed.is_empty()
                    && (def.name.starts_with(".bss") || def.name.starts_with(".brk")) && size > 0);

                if is_alloc { flags |= SHF_ALLOC_; }

                let lma = if let Some(ref at) = def.at_lma {
                    // AT() may reference ADDR(this-section): publish addr first.
                    sections_meta.insert(def.name.clone(), (sec_start, size, max_align, sec_start));
                    eval_full(at, sec_start, &symbols, &sections_meta, &def_syms, &placed_map)
                        .map_err(|e| format!("cannot evaluate AT() of {}: {}", def.name, e))?
                } else {
                    sec_start
                };
                sections_meta.insert(def.name.clone(), (sec_start, size, max_align, lma));

                // MEMORY region accounting. Overrunning a region is the single
                // most common linker-script mistake, and an unchecked overrun
                // produces an image whose sections silently overlap. Diagnose
                // it by name, with the overflow amount, like GNU ld does.
                if let Some(rname) = def.region.as_deref() {
                    let end = sec_start.saturating_add(size);
                    if let Some(&(origin, limit)) = region_bounds.get(rname) {
                        if end > limit {
                            return Err(format!(
                                "linker script: section {} overflows MEMORY region '{}' by \
                                 {} bytes (region {:#x}..{:#x}, section {:#x}..{:#x})",
                                def.name, rname, end - limit, origin, limit, sec_start, end));
                        }
                    }
                    region_cursor.insert(rname.to_string(), end);
                }
                if let Some(rname) = def.lma_region.as_deref() {
                    // AT> region: the load image must fit too.
                    if let Some(&(origin, limit)) = region_bounds.get(rname) {
                        let lend = lma.saturating_add(size);
                        if lend > limit {
                            return Err(format!(
                                "linker script: load image of {} overflows MEMORY region \
                                 '{}' by {} bytes (region {:#x}..{:#x})",
                                def.name, rname, lend - limit, origin, limit));
                        }
                    }
                }

                out_secs.push(OutSec {
                    name: def.name.clone(),
                    vaddr: sec_start,
                    lma,
                    size,
                    align: max_align,
                    sh_type: if nobits { SHT_NOBITS_ } else { sh_type },
                    flags,
                    file_offset: 0,
                    is_alloc,
                    nobits,
                    phdrs: def.phdrs.clone(),
                    fill: def.fill,
                    placed,
                });

                if is_alloc {
                    dot = cur;
                    if size > 0 {
                        cur_out_name = Some(def.name.clone());
                    }
                }
            }
        }
    }

    // Append orphan alloc sections at the end (page-aligned).
    if !orphans.is_empty() {
        for (oi, si) in orphans {
            let sec = &objects[oi].sections[si];
            let a = sec.addralign.max(1);
            dot = (dot + a - 1) & !(a - 1);
            placed_map.insert((oi, si), dot);
            placed_owner.insert((oi, si), out_secs.len());
            let start = dot;
            dot += sec.size;
            sections_meta.insert(sec.name.clone(), (start, sec.size, a, start));
            out_secs.push(OutSec {
                name: sec.name.clone(), vaddr: start, lma: start, size: sec.size,
                align: a, sh_type: sec.sh_type, flags: sec.flags & !SHF_GROUP_,
                file_offset: 0, is_alloc: true,
                nobits: sec.sh_type == SHT_NOBITS_,
                phdrs: Vec::new(), fill: None,
                placed: vec![Placed { obj_idx: oi, sec_idx: si, vaddr: start, size: sec.size }],
            });
        }
    }

    // ── Re-run deferred assignments to fixed point ──
    let mut made_progress = true;
    let mut pending = deferred;
    while made_progress && !pending.is_empty() {
        made_progress = false;
        let mut still: Vec<(Assignment, u64)> = Vec::new();
        for (a, adot) in pending.into_iter() {
            match eval_full(&a.expr, adot, &symbols, &sections_meta, &def_syms, &placed_map) {
                Ok(v) => {
                    made_progress = true;
                    if a.symbol == "__assert__" || a.symbol == "." { continue; }
                    if a.provide && (symbols.contains_key(&a.symbol) || def_syms.contains_key(&a.symbol)) { continue; }
                    let nv = if a.op == AssignOp::Add {
                        symbols.get(&a.symbol).copied().unwrap_or(0).wrapping_add(v)
                    } else { v };
                    symbols.insert(a.symbol.clone(), nv);
                }
                Err(_) => still.push((a, adot)),
            }
        }
        pending = still;
    }
    // Evaluate top-level assigns/asserts (PROVIDE etc.) the same way.
    for a in &script.top_assigns {
        if let Ok(v) = eval_full(&a.expr, dot, &symbols, &sections_meta, &def_syms, &placed_map) {
            if a.symbol == "." { continue; }
            if a.provide && (symbols.contains_key(&a.symbol) || def_syms.contains_key(&a.symbol)) { continue; }
            symbols.insert(a.symbol.clone(), v);
            if a.hidden { hidden_syms.insert(a.symbol.clone()); }
        }
    }
    for (e, msg) in &script.top_asserts {
        if let Err(err) = eval_full(e, dot, &symbols, &sections_meta, &def_syms, &placed_map) {
            return Err(format!("linker script assert: {} ({})", msg, err));
        }
    }
    for (a, adot) in &pending {
        // Remaining failures: asserts get reported, plain assigns error out.
        match eval_full(&a.expr, *adot, &symbols, &sections_meta, &def_syms, &placed_map) {
            Ok(_) => {}
            Err(e) => {
                if a.symbol == "__assert__" {
                    return Err(format!("linker script ASSERT failed: {}", e));
                }
                // PROVIDE of an unresolvable symbol that nothing references is OK.
                if !a.provide {
                    return Err(format!(
                        "linker script: cannot resolve assignment {} = ... ({})", a.symbol, e));
                }
            }
        }
    }

    // ── Resolve every symbol needed by relocations ──
    // resolve(obj, sym) -> Option<vaddr>
    let resolve = |oi: usize, sym: &linker_common::Elf64Symbol| -> Option<u64> {
        if sym.sym_type() == STT_SECTION {
            return placed_map.get(&(oi, sym.shndx as usize)).copied();
        }
        if !sym.name.is_empty() {
            if sym.is_local() {
                if sym.shndx == SHN_ABS { return Some(sym.value); }
                return placed_map.get(&(oi, sym.shndx as usize)).map(|b| b + sym.value);
            }
            if let Some(&v) = symbols.get(sym.name.as_str()) { return Some(v); }
            if let Some(&(doi, shndx, value, _, _)) = def_syms.get(sym.name.as_str()) {
                if shndx == SHN_ABS { return Some(value); }
                return placed_map.get(&(doi, shndx as usize)).map(|b| b + value);
            }
            if sym.is_weak() { return Some(0); }
            return None;
        }
        if sym.shndx == SHN_ABS { return Some(sym.value); }
        placed_map.get(&(oi, sym.shndx as usize)).map(|b| b + sym.value)
    };

    // ── File layout ──
    // Compute program headers from PHDRS + section->phdr assignments.
    // Sections inherit the previous section's phdr list when unspecified (GNU rule).
    let mut cur_phdrs: Vec<String> = Vec::new();
    let mut sec_phdrs: Vec<Vec<String>> = Vec::with_capacity(out_secs.len());
    for os in &out_secs {
        if !os.phdrs.is_empty() {
            cur_phdrs = os.phdrs.clone();
        }
        if os.is_alloc {
            sec_phdrs.push(cur_phdrs.clone());
        } else {
            sec_phdrs.push(Vec::new());
        }
    }

    // ELF header + phdrs at file start.
    let declared_phdrs: Vec<&linker_script::PhdrDecl> = script.phdrs.iter().collect();
    // A script that places SHF_TLS sections but declares no PT_TLS gets one
    // synthesised. Without it the loader never allocates the thread block, so
    // every %fs-relative access reads whatever happens to precede the TCB --
    // the relocations are correct and the program still misbehaves.
    //
    // This must agree with `will_add_tls_phdr` above, which fed SIZEOF_HEADERS.
    let needs_tls_phdr = out_secs.iter().any(|o| (o.flags & SHF_TLS_) != 0 && o.is_alloc)
        && !script.phdrs.iter().any(|d| d.ptype == PT_TLS_);
    debug_assert_eq!(needs_tls_phdr, will_add_tls_phdr,
        "PT_TLS prediction disagreed with the final layout; SIZEOF_HEADERS would be wrong");
    let n_phdrs = declared_phdrs.len().max(1) + usize::from(needs_tls_phdr);
    let mut file_off = script_header_size_with(
        &script, usize::from(needs_tls_phdr));

    // Assign file offsets: alloc PROGBITS sections in vaddr order get offsets
    // congruent to their LMA modulo the requested maximum page size. The
    // kernel proper uses 2 MiB; its vDSO explicitly requests 4 KiB.
    let page = if max_page_size.is_power_of_two() { max_page_size } else { 0x200000 };
    let mut order: Vec<usize> = (0..out_secs.len()).collect();
    order.sort_by_key(|&i| (!out_secs[i].is_alloc, out_secs[i].lma));

    for &i in &order {
        let os = &mut out_secs[i];
        if !os.is_alloc || os.nobits || os.size == 0 {
            continue;
        }
        // congruence: file_off ≡ lma (mod page)
        let want = os.lma % page;
        let have = file_off % page;
        if want != have {
            file_off += (want + page - have) % page;
        }
        os.file_offset = file_off;
        file_off += os.size;
    }
    // NOBITS sections point at the current file offset (no data).
    for os in out_secs.iter_mut() {
        if os.nobits || os.size == 0 || !os.is_alloc {
            os.file_offset = file_off;
        }
    }

    // ── Build output image ──
    let mut out = vec![0u8; file_off as usize];

    // Fill patterns + section data
    for os in &out_secs {
        if os.nobits || !os.is_alloc || os.size == 0 { continue; }
        let base = os.file_offset as usize;
        if let Some(fill) = os.fill {
            // GNU ld uses the fill value as a (big-endian) byte pattern; the
            // kernel's 0xcccccccc means "fill gaps with int3".
            let b = [(fill >> 24) as u8, (fill >> 16) as u8, (fill >> 8) as u8, fill as u8];
            let pat: &[u8] = if fill <= 0xff { &b[3..] } else { &b[..] };
            for k in 0..os.size as usize {
                out[base + k] = pat[k % pat.len()];
            }
        }
        for p in &os.placed {
            let data = &objects[p.obj_idx].section_data[p.sec_idx];
            if data.is_empty() { continue; }
            let off = base + (p.vaddr - os.vaddr) as usize;
            let end = off + data.len();
            if end <= out.len() {
                out[off..end].copy_from_slice(data);
            }
        }
    }

    /// Relax a GOT-relative reference into direct addressing.
    ///
    /// A linker-script link has no GOT: every address is known, so the
    /// *indirection through a GOT slot* that the compiler emitted must be
    /// removed rather than satisfied. The x86-64 psABI ("GOTPCRELX
    /// relaxations") specifies the legal rewrites; this implements the forms
    /// GCC and Clang actually emit for `-fPIC` code:
    ///
    /// | before                              | after                        |
    /// |-------------------------------------|------------------------------|
    /// | `mov  sym@GOTPCREL(%rip), %reg` 8b  | `lea sym(%rip), %reg`    8d  |
    /// | `cmp  %reg, sym@GOTPCREL(%rip)` 3b  | `cmp $sym, %reg`   81 /7 imm |
    /// | `call *sym@GOTPCREL(%rip)`   ff /2  | `call sym` (e8) + nop        |
    /// | `jmp  *sym@GOTPCREL(%rip)`   ff /4  | `jmp  sym` (e9) + nop        |
    ///
    /// `mov` -> `lea` keeps the operand PC-relative, so it is position
    /// independent and needs no range beyond the usual +/-2 GiB. The `cmp`
    /// and indirect-branch forms become absolute/direct and are therefore
    /// only valid when the target fits the encoding; that is checked.
    ///
    /// Returns `Err(<opcode bytes>)` when the form is not one of the above,
    /// so the caller can produce a precise diagnostic instead of emitting a
    /// silently corrupt image.
    fn relax_gotpcrel(
        out: &mut [u8], fp: usize, s: u64, a: i64, p: u64,
    ) -> Result<(), String> {
        // Displacement for a rewritten PC-relative operand. The instruction
        // length is unchanged by every rewrite below, so `p` still names the
        // end of the 4-byte field.
        let pcrel = s as i64 + a - p as i64;
        let describe = |o: &[u8]| o.iter().map(|b| format!("{:02x}", b)).collect::<Vec<_>>().join(" ");

        if fp < 2 || fp + 4 > out.len() {
            return Err("<truncated>".to_string());
        }
        let modrm = out[fp - 1];
        let op = out[fp - 2];

        // mov m64, r64  ->  lea m64, r64   (opcode 8b -> 8d, ModRM unchanged)
        if op == 0x8b {
            out[fp - 2] = 0x8d;
            w32(out, fp, pcrel as u32);
            return Ok(());
        }

        // cmp r64, m64  ->  cmp r64, imm32   (3b /r -> 81 /7 id)
        // Only valid when the absolute address fits in a sign-extended imm32,
        // which is the normal case for a -T image below 2 GiB.
        if op == 0x3b {
            let abs = s as i64 + a + 4; // addend is -4 for a rip operand
            if (i32::MIN as i64..=i32::MAX as i64).contains(&abs) {
                let reg = (modrm >> 3) & 7;
                out[fp - 2] = 0x81;
                out[fp - 1] = 0xf8 | reg; // mod=11, /7 (cmp), rm=reg
                w32(out, fp, abs as u32);
                return Ok(());
            }
            return Err(describe(&out[fp - 2..fp]));
        }

        // call/jmp *m64 -> direct call/jmp. ff /2 = call, ff /4 = jmp.
        // The indirect form is 6 bytes (ff /r + disp32) and the direct form is
        // 5 (e8/e9 + rel32), so a leading nop keeps the length identical --
        // exactly what GNU ld does.
        if op == 0xff && fp >= 2 {
            let ext = (modrm >> 3) & 7;
            if ext == 2 || ext == 4 {
                // The rel32 is measured from the end of the 5-byte direct
                // instruction, which now starts one byte later.
                let rel = s as i64 + a + 1 - p as i64;
                if !(i32::MIN as i64..=i32::MAX as i64).contains(&rel) {
                    return Err(describe(&out[fp - 2..fp]));
                }
                out[fp - 2] = 0x90; // nop
                out[fp - 1] = if ext == 2 { 0xe8 } else { 0xe9 };
                w32(out, fp, rel as u32);
                return Ok(());
            }
        }

        Err(describe(&out[fp - 2..fp]))
    }

    // ── TLS segment bounds ──
    //
    // Initial-Exec TLS offsets are measured from the *end* of the TLS block,
    // because %fs:0 points just past it on x86-64. The block spans every
    // SHF_TLS output section; .tbss contributes to the memory size but not to
    // the file image, exactly as in a PT_TLS program header.
    let (tls_addr, tls_mem_size) = {
        let mut lo = u64::MAX;
        let mut hi = 0u64;
        for os in out_secs.iter().filter(|o| (o.flags & SHF_TLS_) != 0 && o.is_alloc) {
            lo = lo.min(os.vaddr);
            hi = hi.max(os.vaddr + os.size);
        }
        if lo == u64::MAX { (0u64, 0u64) } else { (lo, hi - lo) }
    };

    // ── Relocation helpers ──
    //
    // A 32-bit displacement that does not fit is the classic way a large image
    // gets silently corrupted: the truncated value points somewhere plausible
    // and the failure surfaces much later as a wild jump. Diagnose instead.
    fn reloc_range_err(kind: &str, v: i64, sym: &str, src: &str) -> String {
        format!("script link: {} against '{}' in {} does not fit: value {:#x} \
                 is out of range (image too large or wrong load address?)",
                kind, sym, src, v)
    }
    fn check_pcrel32(v: i64, sym: &str, src: &str) -> Result<(), String> {
        if !(i32::MIN as i64..=i32::MAX as i64).contains(&v) {
            return Err(reloc_range_err("32-bit PC-relative reference", v, sym, src));
        }
        Ok(())
    }

    // ── Apply relocations ──
    let mut undefined: Vec<String> = Vec::new();
    for (oi, obj) in objects.iter().enumerate() {
        for (si, relas) in obj.relocations.iter().enumerate() {
            if relas.is_empty() { continue; }
            let Some(&sec_vaddr) = placed_map.get(&(oi, si)) else { continue };
            let Some(&owner) = placed_owner.get(&(oi, si)) else { continue };
            let os = &out_secs[owner];
            if os.nobits || !os.is_alloc { continue; }
            let sec_foff = os.file_offset + (sec_vaddr - os.vaddr);

            for rela in relas {
                let sidx = rela.sym_idx as usize;
                if sidx >= obj.symbols.len() { continue; }
                let sym = &obj.symbols[sidx];
                let p = sec_vaddr + rela.offset;
                let fp = (sec_foff + rela.offset) as usize;
                let a = rela.addend;
                let s = match resolve(oi, sym) {
                    Some(v) => v,
                    None => {
                        if undefined.len() < 20 && !undefined.iter().any(|u| u == sym.name.as_str()) {
                            undefined.push(sym.name.to_string());
                        }
                        continue;
                    }
                };
                match rela.rela_type {
                    R_X86_64_64 => w64(&mut out, fp, (s as i64 + a) as u64),
                    R_X86_64_PC32 | R_X86_64_PLT32 => {
                        // A -T link resolves every call directly; there is no
                        // PLT, so PLT32 degenerates to PC32.
                        let v = s as i64 + a - p as i64;
                        check_pcrel32(v, &sym.name, &obj.source_name)?;
                        w32(&mut out, fp, v as u32)
                    }
                    R_X86_64_32 => {
                        let v = s as i64 + a;
                        if !(0..=u32::MAX as i64).contains(&v) {
                            return Err(reloc_range_err("R_X86_64_32", v, &sym.name, &obj.source_name));
                        }
                        w32(&mut out, fp, v as u32)
                    }
                    R_X86_64_32S => {
                        let v = s as i64 + a;
                        if !(i32::MIN as i64..=i32::MAX as i64).contains(&v) {
                            return Err(reloc_range_err("R_X86_64_32S", v, &sym.name, &obj.source_name));
                        }
                        w32(&mut out, fp, v as u32)
                    }
                    R_X86_64_16 => w16(&mut out, fp, (s as i64 + a) as u16),
                    R_X86_64_PC16 => w16(&mut out, fp, (s as i64 + a - p as i64) as u16),
                    R_X86_64_8 => {
                        if fp < out.len() { out[fp] = (s as i64 + a) as u8; }
                    }
                    R_X86_64_PC8 => {
                        if fp < out.len() { out[fp] = (s as i64 + a - p as i64) as u8; }
                    }
                    R_X86_64_PC64 => w64(&mut out, fp, (s as i64 + a - p as i64) as u64),
                    // Symbol size, not address: used by some hand-written asm
                    // and by __builtin_object_size lowering.
                    R_X86_64_SIZE32 => w32(&mut out, fp, (sym.size as i64 + a) as u32),
                    R_X86_64_SIZE64 => w64(&mut out, fp, (sym.size as i64 + a) as u64),

                    // ── GOT-relative forms ──
                    //
                    // A linker-script link produces a fully-resolved image with
                    // no dynamic loader and no GOT of its own, so every GOT
                    // reference must be relaxed into direct addressing. This is
                    // exactly what GNU ld does for -no-pie/static links, and it
                    // is what the kernel's vDSO and early-boot objects rely on:
                    // they are compiled -fPIC but linked to fixed addresses.
                    R_X86_64_GOTPCREL | R_X86_64_GOTPCRELX | R_X86_64_REX_GOTPCRELX => {
                        // A -T link produces a fully-resolved image with no
                        // dynamic loader and no GOT, so every GOT reference
                        // must be relaxed into direct addressing. The x86-64
                        // psABI defines exactly which instruction forms may be
                        // rewritten; anything else is a hard error, because
                        // pointing a LOAD at the symbol would read the bytes
                        // stored there and treat them as a pointer.
                        match relax_gotpcrel(&mut out, fp, s, a, p) {
                            Ok(()) => {}
                            Err(op) => {
                                return Err(format!(
                                    "script link: GOTPCREL against '{}' in {} uses an \
                                     instruction form this linker cannot relax \
                                     (opcode bytes {}); a -T link has no GOT, and \
                                     pointing the load at the symbol would read its \
                                     bytes instead of its address",
                                    sym.name, obj.source_name, op));
                            }
                        }
                    }
                    // GOTPC32: distance from the reference to the GOT origin.
                    // With no GOT, GNU ld still defines _GLOBAL_OFFSET_TABLE_;
                    // treat its address as the value so `lea _GLOBAL_OFFSET_TABLE_(%rip)`
                    // sequences stay self-consistent.
                    R_X86_64_GOTPC32 => {
                        let got_base = symbols.get("_GLOBAL_OFFSET_TABLE_").copied().unwrap_or(p);
                        w32(&mut out, fp, (got_base as i64 + a - p as i64) as u32)
                    }
                    R_X86_64_GOTOFF64 => {
                        let got_base = symbols.get("_GLOBAL_OFFSET_TABLE_").copied().unwrap_or(0);
                        w64(&mut out, fp, (s as i64 + a - got_base as i64) as u64)
                    }
                    // ── Thread-local storage ──
                    //
                    // A -T link is always static with a fixed TLS block, so
                    // the Initial-Exec / Local-Exec forms resolve directly and
                    // no GOT slot or __tls_get_addr call is needed.
                    R_X86_64_TPOFF32 => {
                        if tls_mem_size == 0 {
                            return Err(format!(
                                "script link: TLS relocation against '{}' in {} but the \
                                 script places no SHF_TLS section (add .tdata/.tbss)",
                                sym.name, obj.source_name));
                        }
                        // %fs:0 is the end of the block: offsets are negative.
                        let tpoff = (s as i64 - tls_addr as i64) - tls_mem_size as i64;
                        let v = tpoff + a;
                        if !(i32::MIN as i64..=i32::MAX as i64).contains(&v) {
                            return Err(reloc_range_err("R_X86_64_TPOFF32", v,
                                                       &sym.name, &obj.source_name));
                        }
                        w32(&mut out, fp, v as u32);
                    }
                    R_X86_64_TPOFF64 => {
                        let tpoff = (s as i64 - tls_addr as i64) - tls_mem_size as i64;
                        w64(&mut out, fp, (tpoff + a) as u64);
                    }
                    // Offset within the TLS block, used by Local-Dynamic
                    // sequences; measured from the block start, not its end.
                    R_X86_64_DTPOFF32 => w32(&mut out, fp, (s as i64 - tls_addr as i64 + a) as u32),
                    R_X86_64_DTPOFF64 => w64(&mut out, fp, (s as i64 - tls_addr as i64 + a) as u64),
                    R_X86_64_NONE => {}
                    other => {
                        return Err(format!(
                            "script link: unsupported relocation type {} for '{}' in {}",
                            other, sym.name, obj.source_name));
                    }
                }
            }
        }
    }
    if !undefined.is_empty() {
        undefined.sort();
        return Err(format!("undefined symbols: {}", undefined.join(", ")));
    }

    // ── ELF + program headers ──
    let entry = script.entry.as_ref()
        .and_then(|e| {
            symbols.get(e).copied().or_else(|| {
                def_syms.get(e).and_then(|&(oi, shndx, value, _, _)| {
                    if shndx == SHN_ABS { Some(value) }
                    else { placed_map.get(&(oi, shndx as usize)).map(|b| b + value) }
                })
            })
        })
        // A shared object without an explicit ENTRY has no process entry
        // point.  Choosing its first allocated section would incorrectly make
        // the ELF hash table look executable (and differs from GNU ld).
        .unwrap_or_else(|| if is_pie { 0 } else {
            out_secs.iter().find(|o| o.is_alloc).map(|o| o.vaddr).unwrap_or(0)
        });

    out[0..4].copy_from_slice(&ELF_MAGIC);
    out[4] = ELFCLASS64; out[5] = ELFDATA2LSB; out[6] = 1;
    w16(&mut out, 16, if is_pie { ET_DYN } else { ET_EXEC });
    w16(&mut out, 18, EM_X86_64); w32(&mut out, 20, 1);
    w64(&mut out, 24, entry);
    w64(&mut out, 32, 64);
    w32(&mut out, 48, 0); w16(&mut out, 52, 64); w16(&mut out, 54, 56);
    w16(&mut out, 56, n_phdrs as u16); w16(&mut out, 58, 64);

    // Program headers: for each declared phdr, span the sections assigned to it.
    let mut ph_off = 64usize;
    for decl in &declared_phdrs {
        let mut min_v = u64::MAX; let mut max_v = 0u64;
        let mut min_f = u64::MAX; let mut max_f = 0u64;
        let mut max_mem = 0u64;
        let mut min_lma = u64::MAX;
        let mut segment_align = 1u64;
        for (i, os) in out_secs.iter().enumerate() {
            if !os.is_alloc || os.size == 0 { continue; }
            if !sec_phdrs[i].contains(&decl.name) { continue; }
            segment_align = segment_align.max(os.align);
            min_v = min_v.min(os.vaddr);
            max_mem = max_mem.max(os.vaddr + os.size);
            min_lma = min_lma.min(os.lma);
            if !os.nobits {
                min_f = min_f.min(os.file_offset);
                max_f = max_f.max(os.file_offset + os.size);
                max_v = max_v.max(os.vaddr + os.size);
            }
        }
        if min_v == u64::MAX {
            // Empty phdr
            wphdr(&mut out, ph_off, decl.ptype, decl.flags.unwrap_or(0) as u32, 0, 0, 0, 0, 8);
            ph_off += 56;
            continue;
        }
        if min_f == u64::MAX { min_f = 0; max_f = 0; max_v = min_v; }
        // FILEHDR/PHDRS make the ELF/program headers part of this segment.
        // Linux's vDSO verifier requires its sole PT_LOAD to begin exactly at
        // file offset and virtual address zero.
        if decl.has_filehdr {
            // Preserve the VMA-to-file-offset delta while extending the
            // segment back over every byte preceding its first section.
            min_v = min_v.saturating_sub(min_f);
            min_lma = min_lma.saturating_sub(min_f);
            min_f = 0;
        } else if decl.has_phdrs {
            let prefix = min_f.saturating_sub(ELF64_EHDR_SIZE);
            min_v = min_v.saturating_sub(prefix);
            min_lma = min_lma.saturating_sub(prefix);
            min_f = ELF64_EHDR_SIZE;
        }
        let filesz = max_f.saturating_sub(min_f);
        let memsz = max_mem - min_v;
        let flags = decl.flags.unwrap_or(match decl.ptype {
            PT_LOAD_ => 5,
            _ => 4,
        }) as u32;
        let align = if decl.ptype == PT_LOAD_ { page } else { segment_align };
        // p_paddr = LMA
        wphdr_paddr(&mut out, ph_off, decl.ptype, flags, min_f, min_v,
                    min_lma, filesz, memsz, align);
        ph_off += 56;
        let _ = max_v;
    }
    if declared_phdrs.is_empty() {
        // Single LOAD spanning everything.
        //
        // The span is taken over LOAD addresses, not virtual ones. With
        // overlays (or any AT()) several sections share a VMA while occupying
        // distinct LMAs, so a VMA-derived memsz can come out smaller than the
        // file image and produce `filesz > memsz`, which readelf rejects and
        // loaders treat as corrupt.
        let alloc = |o: &&OutSec| o.is_alloc && o.size > 0;
        let min_v = out_secs.iter().filter(alloc).map(|o| o.lma).min().unwrap_or(0);
        let max_m = out_secs.iter().filter(alloc)
            .map(|o| o.lma + o.size).max().unwrap_or(0);
        let min_f = out_secs.iter().filter(|o| alloc(o) && !o.nobits)
            .map(|o| o.file_offset).min().unwrap_or(0);
        let max_f = out_secs.iter().filter(|o| alloc(o) && !o.nobits)
            .map(|o| o.file_offset + o.size).max().unwrap_or(0);
        let filesz = max_f.saturating_sub(min_f);
        let memsz = (max_m.saturating_sub(min_v)).max(filesz);
        wphdr(&mut out, 64, PT_LOAD_, 7, min_f, min_v, filesz, memsz, page);
        ph_off = 64 + 56;
    }
    if needs_tls_phdr {
        // p_filesz covers .tdata only (.tbss is NOBITS); p_memsz covers both.
        let tls: Vec<&OutSec> = out_secs.iter()
            .filter(|o| (o.flags & SHF_TLS_) != 0 && o.is_alloc).collect();
        let vlo = tls.iter().map(|o| o.vaddr).min().unwrap_or(0);
        let vhi = tls.iter().map(|o| o.vaddr + o.size).max().unwrap_or(0);
        let flo = tls.iter().filter(|o| !o.nobits)
            .map(|o| o.file_offset).min().unwrap_or(0);
        let fhi = tls.iter().filter(|o| !o.nobits)
            .map(|o| o.file_offset + o.size).max().unwrap_or(flo);
        let talign = tls.iter().map(|o| o.align).max().unwrap_or(1).max(1);
        wphdr(&mut out, ph_off, PT_TLS_, 4 /* PF_R */, flo, vlo,
              fhi - flo, vhi - vlo, talign);
    }

    // ── Section headers (+ optional symtab) ──
    let mut shstrtab: Vec<u8> = vec![0];
    let mut shname = |t: &mut Vec<u8>, n: &str| -> u32 {
        let off = t.len() as u32;
        t.extend_from_slice(n.as_bytes());
        t.push(0);
        off
    };

    // symtab: linker-script symbols + object globals + locals (for kallsyms/System.map)
    let mut symtab: Vec<[u8; 24]> = vec![[0u8; 24]];
    let mut strtab: Vec<u8> = vec![0];
    let n_local_syms;

    // Map vaddr -> output section header index for st_shndx.
    // Header order: NULL + emitted sections (alloc first as laid out) + symtab/strtab/shstrtab
    let mut hdr_secs: Vec<usize> = Vec::new(); // indices into out_secs
    for (i, os) in out_secs.iter().enumerate() {
        // GNU ld drops zero-size output sections from the final image;
        // keeping them (e.g. the kernel's ASSERT-guard .got/.plt/.rela.dyn)
        // confuses objcopy and inflates the header table.
        if os.size > 0 {
            hdr_secs.push(i);
        }
    }
    let find_shndx = |vaddr: u64| -> u16 {
        // Two passes: prefer strict containment, then inclusive end.
        // End-of-section symbols like `_end` (== .brk end) must be attributed
        // to the PRECEDING section, not a follow-on section that happens to
        // start at the same address (the kernel strips .modinfo from the
        // final image; a symbol attributed to it would be stripped too).
        for (h, &i) in hdr_secs.iter().enumerate() {
            let os = &out_secs[i];
            if os.is_alloc && vaddr >= os.vaddr && vaddr < os.vaddr + os.size.max(1) {
                return (h + 1) as u16;
            }
        }
        for (h, &i) in hdr_secs.iter().enumerate() {
            let os = &out_secs[i];
            if os.is_alloc && vaddr >= os.vaddr && vaddr <= os.vaddr + os.size {
                return (h + 1) as u16;
            }
        }
        SHN_ABS
    };

    // Finish linker-created ET_DYN metadata now that every exported symbol and
    // synthetic section has a final address.
    if let Some(plan) = script_dynamic.as_ref() {
        let synthetic_file_offset = |section_index: usize| -> Option<usize> {
            let &vaddr = placed_map.get(&(plan.object_index, section_index))?;
            let &owner = placed_owner.get(&(plan.object_index, section_index))?;
            let os = &out_secs[owner];
            Some((os.file_offset + vaddr - os.vaddr) as usize)
        };

        // .dynsym: null entry followed by the GNU-hash bucket order.
        if let Some(base) = synthetic_file_offset(3) {
            for (i, name) in plan.exports.iter().enumerate() {
                let (value, size, info, shndx) = if let Some(&(oi, shndx, value, size, info))
                    = def_syms.get(name)
                {
                    let value = if shndx == SHN_ABS { value } else {
                        placed_map.get(&(oi, shndx as usize)).copied().unwrap_or(0) + value
                    };
                    (value, size, info, find_shndx(value))
                } else if name == &plan.version_name {
                    (0, 0, (STB_GLOBAL << 4) | STT_OBJECT, SHN_ABS)
                } else {
                    continue;
                };
                let off = base + (i + 1) * 24;
                w32(&mut out, off, plan.name_offsets[i]);
                out[off + 4] = info;
                out[off + 5] = 0;
                w16(&mut out, off + 6, shndx);
                w64(&mut out, off + 8, value);
                w64(&mut out, off + 16, size);
            }
        }

        let section_addr = |name: &str| -> u64 {
            sections_meta.get(name).map(|meta| meta.0).unwrap_or(0)
        };
        if let Some(mut off) = synthetic_file_offset(7) {
            let mut emit = |tag: i64, value: u64| {
                w64(&mut out, off, tag as u64);
                w64(&mut out, off + 8, value);
                off += 16;
            };
            if let Some(soname_offset) = plan.soname_offset {
                emit(DT_SONAME, soname_offset as u64);
            }
            if bsymbolic {
                emit(16, 0); // DT_SYMBOLIC
                emit(DT_FLAGS, 0x2); // DF_SYMBOLIC
            }
            emit(DT_HASH, section_addr(".hash"));
            emit(DT_GNU_HASH, section_addr(".gnu.hash"));
            emit(DT_STRTAB, section_addr(".dynstr"));
            emit(DT_SYMTAB, section_addr(".dynsym"));
            emit(DT_STRSZ, out_secs.iter().find(|s| s.name == ".dynstr")
                .map(|s| s.size).unwrap_or(0));
            emit(DT_SYMENT, 24);
            emit(DT_VERSYM, section_addr(".gnu.version"));
            emit(DT_VERDEF, section_addr(".gnu.version_d"));
            emit(DT_VERDEFNUM, plan.verdef_count);
            emit(DT_NULL, 0);
        }
    }

    // --emit-relocs bookkeeping: input symbol -> output .symtab index.
    let mut local_sym_index: FxHashMap<(usize, usize), u32> = FxHashMap::default();
    let mut global_sym_index: FxHashMap<String, u32> = FxHashMap::default();
    // out_secs index -> index of that section's STT_SECTION symbol in .symtab
    let mut out_sec_sym_index: FxHashMap<usize, u32> = FxHashMap::default();

    // STV_* visibility lives in st_other; `elf64_sym_entry` takes it directly.
    fn add_sym_vis(name: &str, value: u64, size: u64, info: u8, other: u8,
                   shndx: u16, symtab: &mut Vec<[u8;24]>, strtab: &mut Vec<u8>) {
        let noff = push_strtab_name(strtab, name.as_bytes());
        symtab.push(elf64_sym_entry(noff, info, other, shndx, value, size));
    }

    if emit_symtab {
        let mut add_sym = |name: &str, value: u64, size: u64, info: u8, shndx: u16,
                           symtab: &mut Vec<[u8;24]>, strtab: &mut Vec<u8>| {
            add_sym_vis(name, value, size, info, 0, shndx, symtab, strtab);
        };

        // --emit-relocs: one STT_SECTION symbol per output section, first in
        // the local range. GNU ld emits relocations against section symbols
        // (`.text + 0x9`) rather than folding the address into the addend,
        // and the kernel's relocs pass and objtool both expect that form.
        // Without these the records still describe the right bytes, but they
        // lose the "which section" information a consumer needs.
        if emit_relocs {
            for (h, &i) in hdr_secs.iter().enumerate() {
                out_sec_sym_index.insert(i, symtab.len() as u32);
                add_sym("", out_secs[i].vaddr, 0,
                        // STB_LOCAL = 0, so the binding nibble is zero.
                        STT_SECTION,
                        (h + 1) as u16, &mut symtab, &mut strtab);
            }
        }

        // Locals first (ELF requirement).
        for (oi, obj) in objects.iter().enumerate() {
            for (sidx, sym) in obj.symbols.iter().enumerate() {
                if !sym.is_local() || sym.name.is_empty() { continue; }
                if sym.sym_type() == STT_SECTION || sym.sym_type() == STT_FILE { continue; }
                if sym.shndx == SHN_UNDEF || sym.shndx == SHN_ABS || sym.shndx == SHN_COMMON { continue; }
                let Some(&base) = placed_map.get(&(oi, sym.shndx as usize)) else { continue };
                let v = base + sym.value;
                // --emit-relocs: remember where this input symbol landed in the
                // output .symtab so retained relocations can point at it.
                local_sym_index.insert((oi, sidx), symtab.len() as u32);
                add_sym(&sym.name, v, sym.size, sym.info, find_shndx(v), &mut symtab, &mut strtab);
            }
        }
        n_local_syms = symtab.len();
        // Object globals
        let mut names: Vec<&String> = def_syms.keys().collect();
        names.sort();
        for name in names {
            let &(oi, shndx, value, size, info) = def_syms.get(name).unwrap();
            let v = if shndx == SHN_ABS { value } else {
                match placed_map.get(&(oi, shndx as usize)) {
                    Some(&b) => b + value,
                    None => continue,
                }
            };
            let sx = if shndx == SHN_ABS { SHN_ABS } else { find_shndx(v) };
            global_sym_index.insert(name.clone(), symtab.len() as u32);
            add_sym(name, v, size, info, sx, &mut symtab, &mut strtab);
        }
        // Script-defined symbols
        let mut snames: Vec<&String> = symbols.keys().collect();
        snames.sort();
        // hdr index by section name for home attribution
        let hdr_by_name: FxHashMap<&str, u16> = hdr_secs.iter().enumerate()
            .map(|(h, &i)| (out_secs[i].name.as_str(), (h + 1) as u16))
            .collect();
        for name in snames {
            if def_syms.contains_key(name) { continue; }
            let v = symbols[name];
            let sx = sym_home.get(name)
                .and_then(|h| hdr_by_name.get(h.as_str()).copied())
                .unwrap_or_else(|| find_shndx(v));
            // STV_HIDDEN = 2. A hidden symbol keeps GLOBAL binding in .symtab
            // (so debuggers still see it) but is excluded from .dynsym.
            let vis = if hidden_syms.contains(name.as_str()) { 2u8 } else { 0u8 };
            add_sym_vis(name, v, 0, (STB_GLOBAL << 4) | STT_NOTYPE_, vis, sx,
                        &mut symtab, &mut strtab);
        }
    } else {
        n_local_syms = 1;
    }

    // ── --emit-relocs: retain relocations as .rela.<section> ───────────────
    //
    // The Linux kernel's arch/x86/tools/relocs pass reads these from a fully
    // linked vmlinux to build the table that lets the boot code relocate the
    // image at runtime (CONFIG_RELOCATABLE / CONFIG_RANDOMIZE_BASE, i.e.
    // KASLR). Without them the kernel links "successfully" and then fails to
    // boot, which is why accepting-and-ignoring the flag was worse than
    // rejecting it.
    //
    // Semantics (matching GNU ld):
    //   * relocations are still APPLIED to the section contents above; this
    //     only preserves a record of them;
    //   * r_offset is rebased from section-relative to the final VIRTUAL
    //     ADDRESS (bfd emits absolute addresses here for a linked image);
    //   * r_info's symbol index is remapped from the input object's symtab to
    //     the merged output .symtab;
    //   * the addend is carried through unchanged;
    //   * one .rela.NAME section per output section that has any, with
    //     sh_info = target section header index and sh_link = .symtab index.
    //
    // Relocations against symbols that did not make it into the output symtab
    // are emitted with symbol index 0 (STN_UNDEF) and the resolved value
    // folded into the addend, so the record stays self-consistent rather than
    // dangling — the same trick bfd uses for section-relative references.
    struct RetainedRelocs {
        /// index into out_secs of the section these apply to
        target: usize,
        /// raw Elf64_Rela entries, already in output form
        entries: Vec<[u8; 24]>,
    }
    let mut retained: Vec<RetainedRelocs> = Vec::new();
    if emit_relocs {
        // Group by target output section, preserving output-section order so
        // the .rela.* headers appear in a deterministic sequence.
        let mut by_target: FxHashMap<usize, Vec<[u8; 24]>> = FxHashMap::default();
        for (oi, obj) in objects.iter().enumerate() {
            for (si, relas) in obj.relocations.iter().enumerate() {
                if relas.is_empty() { continue; }
                let Some(&sec_vaddr) = placed_map.get(&(oi, si)) else { continue };
                let Some(&owner) = placed_owner.get(&(oi, si)) else { continue };
                if !out_secs[owner].is_alloc { continue; }
                for rela in relas {
                    if rela.rela_type == R_X86_64_NONE { continue; }
                    let sidx = rela.sym_idx as usize;
                    let Some(sym) = obj.symbols.get(sidx) else { continue };

                    // Map the input symbol to its output .symtab index.
                    let (out_sym, extra_addend) = if sym.is_local() {
                        match local_sym_index.get(&(oi, sidx)) {
                            Some(&ix) => (ix, 0i64),
                            None => {
                                // A reference through a section symbol (the
                                // common case for .eh_frame and switch tables).
                                // Point at the OUTPUT section's symbol and
                                // rebase the addend by how far this input
                                // section sits inside it — exactly what GNU ld
                                // emits, e.g. `.text + 0x9`.
                                let secsym = out_sec_sym_index.get(&owner).copied();
                                match secsym {
                                    Some(ix) => {
                                        let delta = sec_vaddr
                                            .wrapping_sub(out_secs[owner].vaddr)
                                            as i64;
                                        (ix, delta)
                                    }
                                    // No section symbol available (symtab
                                    // suppressed): keep the record
                                    // self-consistent by folding the resolved
                                    // address into the addend.
                                    None => (0u32, resolve(oi, sym).unwrap_or(0) as i64),
                                }
                            }
                        }
                    } else {
                        match global_sym_index.get(sym.name.as_str()) {
                            Some(&ix) => (ix, 0i64),
                            None => (0u32, resolve(oi, sym).unwrap_or(0) as i64),
                        }
                    };

                    let r_offset = sec_vaddr + rela.offset;
                    let r_info = ((out_sym as u64) << 32) | (rela.rela_type as u64);
                    let r_addend = rela.addend + extra_addend;

                    let mut e = [0u8; 24];
                    e[0..8].copy_from_slice(&r_offset.to_le_bytes());
                    e[8..16].copy_from_slice(&r_info.to_le_bytes());
                    e[16..24].copy_from_slice(&r_addend.to_le_bytes());
                    by_target.entry(owner).or_default().push(e);
                }
            }
        }
        // Deterministic order: follow the output-section header order, and
        // sort each group by r_offset (bfd emits ascending offsets, and the
        // kernel's relocs tool is easier to diff against when we match).
        for (h, &i) in hdr_secs.iter().enumerate() {
            let _ = h;
            if let Some(mut entries) = by_target.remove(&i) {
                entries.sort_by_key(|e| u64::from_le_bytes(e[0..8].try_into().unwrap()));
                retained.push(RetainedRelocs { target: i, entries });
            }
        }
    }

    // Append symtab/strtab/shstrtab data
    while out.len() % 8 != 0 { out.push(0); }
    let symtab_off = out.len() as u64;
    for e in &symtab { out.extend_from_slice(e); }
    let strtab_off = out.len() as u64;
    out.extend_from_slice(&strtab);

    // --emit-relocs payloads (8-byte aligned, Elf64_Rela is 24 bytes).
    let mut rela_offsets: Vec<u64> = Vec::with_capacity(retained.len());
    for rr in &retained {
        while out.len() % 8 != 0 { out.push(0); }
        rela_offsets.push(out.len() as u64);
        for e in &rr.entries { out.extend_from_slice(e); }
    }

    // section header name offsets
    let mut name_offs: Vec<u32> = Vec::new();
    for &i in &hdr_secs {
        let n = out_secs[i].name.clone();
        name_offs.push(shname(&mut shstrtab, &n));
    }
    // ".rela" + target section name, e.g. ".rela.text".
    let rela_names: Vec<u32> = retained.iter()
        .map(|rr| {
            let n = format!(".rela{}", out_secs[rr.target].name);
            shname(&mut shstrtab, &n)
        })
        .collect();
    let symtab_name = shname(&mut shstrtab, ".symtab");
    let strtab_name = shname(&mut shstrtab, ".strtab");
    let shstrtab_name = shname(&mut shstrtab, ".shstrtab");

    while out.len() % 8 != 0 { out.push(0); }
    let shstrtab_off = out.len() as u64;
    out.extend_from_slice(&shstrtab);

    while out.len() % 8 != 0 { out.push(0); }
    let shoff = out.len() as u64;

    let write_shdr = linker_common::write_elf64_shdr;
    // NULL
    write_shdr(&mut out, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0);
    let dynamic_header_index: FxHashMap<&str, u32> = hdr_secs.iter().enumerate()
        .map(|(h, &i)| (out_secs[i].name.as_str(), (h + 1) as u32))
        .collect();
    for (h, &i) in hdr_secs.iter().enumerate() {
        let os = &out_secs[i];
        let link = match os.name.as_str() {
            ".hash" | ".gnu.hash" | ".gnu.version" =>
                dynamic_header_index.get(".dynsym").copied().unwrap_or(0),
            ".dynsym" | ".dynamic" | ".gnu.version_d" =>
                dynamic_header_index.get(".dynstr").copied().unwrap_or(0),
            _ => 0,
        };
        let info = if os.name == ".gnu.version_d" {
            script_dynamic.as_ref().map(|p| p.verdef_count as u32).unwrap_or(0)
        } else if os.name == ".dynsym" { 1 } else { 0 };
        let entsize = match os.name.as_str() {
            ".hash" => 4,
            ".dynsym" => 24,
            ".gnu.version" => 2,
            ".dynamic" => 16,
            _ => 0,
        };
        write_shdr(&mut out, name_offs[h], os.sh_type, os.flags,
                   os.vaddr, os.file_offset, os.size, link, info,
                   os.align.max(1), entsize);
    }
    let n_hdrs = 1 + hdr_secs.len();
    // Header index layout from here on:
    //   [0]              NULL
    //   [1 .. n_hdrs)    output sections (hdr_secs order)
    //   [n_hdrs ..]      .rela.* (one per retained group), if --emit-relocs
    //   then             .symtab, .strtab, .shstrtab
    let symtab_idx = n_hdrs + retained.len();
    let strtab_idx = symtab_idx + 1;

    // Map an out_secs index to its section-header index.
    let hdr_index_of = |target: usize| -> u32 {
        hdr_secs.iter().position(|&i| i == target).map(|p| p as u32 + 1).unwrap_or(0)
    };
    for (k, rr) in retained.iter().enumerate() {
        // sh_link = .symtab, sh_info = section these relocations apply to.
        write_shdr(&mut out, rela_names[k], SHT_RELA_, 0, 0, rela_offsets[k],
                   (rr.entries.len() * 24) as u64,
                   symtab_idx as u32, hdr_index_of(rr.target), 8, 24);
    }

    write_shdr(&mut out, symtab_name, SHT_SYMTAB_, 0, 0, symtab_off,
               (symtab.len() * 24) as u64, strtab_idx as u32, n_local_syms as u32, 8, 24);
    write_shdr(&mut out, strtab_name, SHT_STRTAB_, 0, 0, strtab_off,
               strtab.len() as u64, 0, 0, 1, 0);
    write_shdr(&mut out, shstrtab_name, SHT_STRTAB_, 0, 0, shstrtab_off,
               shstrtab.len() as u64, 0, 0, 1, 0);

    let sh_count = (n_hdrs + retained.len() + 3) as u16;
    out[40..48].copy_from_slice(&shoff.to_le_bytes());
    out[60..62].copy_from_slice(&sh_count.to_le_bytes());
    out[62..64].copy_from_slice(&((sh_count - 1)).to_le_bytes());

    // --build-id=sha1: the synthetic "<build-id>" object carries a
    // .note.gnu.build-id section that the script's *(.note.*) pattern
    // placed like any input. Fill the header now and hash the COMPLETE
    // image with a zeroed descriptor, then patch the digest in.
    {
        let bid = objects.iter().enumerate()
            .filter(|(_, o)| o.source_name == "<build-id>")
            .flat_map(|(oi, o)| o.sections.iter().enumerate()
                .filter(|(_, sec)| sec.name == ".note.gnu.build-id")
                .map(move |(si, _)| (oi, si)))
            .next();
        if let Some((oi, si)) = bid {
            if let (Some(&vaddr), Some(&owner)) =
                (placed_map.get(&(oi, si)), placed_owner.get(&(oi, si))) {
                let os = &out_secs[owner];
                if !os.nobits && os.is_alloc {
                    let foff = (os.file_offset + (vaddr - os.vaddr)) as usize;
                    if foff + linker_common::build_id::BUILD_ID_NOTE_SIZE as usize <= out.len() {
                        linker_common::build_id::write_build_id_skeleton(&mut out, foff);
                        linker_common::build_id::patch_build_id(&mut out, foff);
                    }
                }
            }
        }
    }

    std::fs::write(output_path, &out)
        .map_err(|e| format!("failed to write '{}': {}", output_path, e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(output_path, std::fs::Permissions::from_mode(0o755));
    }
    Ok(())
}

// phdr writer that supports distinct p_paddr (LMA)
#[allow(clippy::too_many_arguments)]
fn wphdr_paddr(out: &mut [u8], off: usize, ptype: u32, flags: u32,
               offset: u64, vaddr: u64, paddr: u64, filesz: u64, memsz: u64, align: u64) {
    w32(out, off, ptype);
    w32(out, off + 4, flags);
    w64(out, off + 8, offset);
    w64(out, off + 16, vaddr);
    w64(out, off + 24, paddr);
    w64(out, off + 32, filesz);
    w64(out, off + 40, memsz);
    w64(out, off + 48, align);
}

// Local aliases for constants used with underscore suffix to avoid clashes
const SHT_NULL_: u32 = 0;
const SHT_PROGBITS_: u32 = 1;
const SHT_SYMTAB_: u32 = 2;
const SHT_STRTAB_: u32 = 3;
const SHT_RELA_: u32 = 4;
const SHT_NOBITS_: u32 = 8;
const SHT_REL_: u32 = 9;
const SHT_GROUP_: u32 = 17;
const SHF_WRITE_: u64 = 0x1;
const SHF_ALLOC_: u64 = 0x2;
const SHF_EXECINSTR_: u64 = 0x4;
const SHF_MERGE_: u64 = 0x10;
const SHF_GROUP_: u64 = 0x200;
const SHF_TLS_: u64 = 0x400;
const STT_NOTYPE_: u8 = 0;
const STT_FILE: u8 = 4;
