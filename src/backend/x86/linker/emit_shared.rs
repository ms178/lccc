//! Shared library (.so) emission for the x86-64 linker.
//!
//! Emits an ELF64 shared library (ET_DYN) with PIC relocations, PLT stubs,
//! `.dynamic` section, and GNU hash tables.

use crate::backend::elf::push_strtab_name;
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use std::collections::BTreeSet;

use super::elf::*;
use super::emit_exec::resolve_sym;
use super::types::{GlobalSymbol, PAGE_SIZE};
use crate::backend::linker_common::VersionScript;
use crate::backend::linker_common::{self, DynStrTab, OutputSection};

/// Strip the .symver version suffix from a linker symbol name:
/// "foo@@GLIBC_2.34" / "foo@GLIBC_2.2.5" -> "foo".
fn sym_base(name: &str) -> String {
    if let Some(pos) = name.find('@') {
        name[..pos].to_string()
    } else {
        name.to_string()
    }
}

fn push_verdef_entry(buf: &mut Vec<u8>, index: u16, name: &str, name_off: usize, next: u32) {
    push_verdef_entry_with_parent(buf, index, name, name_off, next, None);
}

/// Emit one `Elf64_Verdef` plus its `Elf64_Verdaux` chain.
///
/// `parent` adds a second verdaux naming the inherited version. That chain is
/// how `LIBV_2.0 { ... } LIBV_1.0;` tells the loader a LIBV_2.0 provider also
/// satisfies a LIBV_1.0 dependency; without it the hierarchy is lost and an
/// older consumer fails to bind even though the symbols are present.
///
/// `vd_cnt` must count *all* verdaux entries, not just the name, or the loader
/// stops reading after the first one.
fn push_verdef_entry_with_parent(
    buf: &mut Vec<u8>,
    index: u16,
    name: &str,
    name_off: usize,
    next: u32,
    parent: Option<(&str, usize)>,
) {
    let start = buf.len();
    buf.resize(start + 20, 0);
    let name_off32 = name_off as u32;
    debug_assert_eq!(name_off32 as usize, name_off);
    let cnt: u16 = 1 + u16::from(parent.is_some());
    // Elf64_Verdef: vd_version, vd_flags, vd_ndx, vd_cnt, vd_hash, vd_aux, vd_next
    w16(buf, start, 1);
    w16(buf, start + 2, if index == 1 { 1 } else { 0 }); // VER_FLG_BASE for base
    w16(buf, start + 4, index);
    w16(buf, start + 6, cnt);
    w32(buf, start + 8, elf_hash_name(name));
    w32(buf, start + 12, 20);
    w32(buf, start + 16, next);
    let aux = buf.len();
    buf.resize(aux + 8, 0);
    // Elf64_Verdaux: vda_name, vda_next
    w32(buf, aux, name_off32);
    w32(buf, aux + 4, if parent.is_some() { 8 } else { 0 });
    if let Some((_, poff)) = parent {
        let paux = buf.len();
        buf.resize(paux + 8, 0);
        w32(buf, paux, poff as u32);
        w32(buf, paux + 4, 0);
    }
}

fn elf_hash_name(name: &str) -> u32 {
    let mut h: u32 = 0;
    for b in name.bytes() {
        h = (h << 4).wrapping_add(b as u32);
        let g = h & 0xf0000000;
        if g != 0 {
            h ^= g >> 24;
        }
        h &= !g;
    }
    h
}

pub(super) fn emit_shared_library(
    objects: &[ElfObject],
    globals: &mut FxHashMap<String, GlobalSymbol>,
    output_sections: &mut [OutputSection],
    section_map: &FxHashMap<(usize, usize), (usize, u64)>,
    needed_sonames: &[String],
    output_path: &str,
    soname: Option<String>,
    rpath_entries: &[String],
    use_runpath: bool,
    version_script_path: Option<&str>,
    bsymbolic: bool,
    // `--exclude-libs`: archives whose symbols must not be re-exported.
    exclude_libs: &[String],
    // `-Map=FILE`: write a GNU-ld-compatible link map. Previously reachable
    // only for executables, because link_shared's private argument parser
    // never recognised -Map at all.
    map_path: Option<&str>,
) -> Result<(), String> {
    let base_addr: u64 = 0;

    // Congruent segment packing — see the extended rationale in
    // emit_exec.rs. File offsets stay dense; virtual addresses advance one
    // page per PT_LOAD. The gABI only requires
    // `p_offset === p_vaddr (mod p_align)`, which holds by construction
    // because `vaddr_bias` is always a multiple of PAGE_SIZE. Rounding the
    // *file offset* up at each segment boundary (what this function used to
    // do) wastes up to a page per segment: measured 19 568 B for a trivial
    // .so versus 7 792 B from mold and 5 974 B from wild.
    // Shared with emit_exec.rs via layout_plan::SegmentPacker so the invariant
    // is stated and tested exactly once (see that type's documentation).
    let mut packer = super::layout_plan::SegmentPacker::new(base_addr, PAGE_SIZE);
    macro_rules! vaddr {
        ($off:expr) => {
            packer.vaddr($off)
        };
    }
    macro_rules! new_segment {
        () => {
            packer.new_segment();
        };
    }

    let mut dynstr = DynStrTab::new();
    for lib in needed_sonames {
        dynstr.add(lib);
    }
    if let Some(ref sn) = soname {
        dynstr.add(sn);
    }
    let rpath_string = if rpath_entries.is_empty() {
        None
    } else {
        let s = rpath_entries.join(":");
        dynstr.add(&s);
        Some(s)
    };
    let version_script = version_script_path.and_then(VersionScript::parse);

    // `--exclude-libs`: a symbol that came from one of the named archives is
    // linked in but must not appear in .dynsym.
    //
    // This predicate is used in TWO places and both are load-bearing:
    //   1. the export filter, which is the visible effect, and
    //   2. the PLT scan below.
    // Missing (2) produces a subtly broken library rather than an error: the
    // symbol keeps its PLT slot and JUMP_SLOT relocation, but the dynsym entry
    // it referred to is gone, so the relocation ends up pointing at symbol
    // index 0 and the loader aborts with
    // `symbol lookup error: ...: undefined symbol: ` (empty name).
    // An excluded symbol is by definition not interposable, so suppressing the
    // PLT is also the semantically correct thing to do — the call binds
    // directly, exactly as for a hidden or version-script-local symbol.
    // Precomputed as a set rather than a closure over `globals`: the export
    // filter and the PLT scan run at points where `globals` is mutably
    // borrowed, and a set lookup is O(1) instead of re-walking objects per
    // query. Objects are classified once (there are far fewer objects than
    // symbols).
    let excluded_syms: FxHashSet<String> = if exclude_libs.is_empty() {
        FxHashSet::default()
    } else {
        let excluded_objs: Vec<bool> = objects
            .iter()
            .map(|o| linker_common::exclude_libs_matches(exclude_libs, &o.source_name))
            .collect();
        globals
            .iter()
            .filter(|(_, g)| {
                g.defined_in
                    .is_some_and(|oi| excluded_objs.get(oi).copied().unwrap_or(false))
            })
            .map(|(n, _)| n.clone())
            .collect()
    };

    // Identify symbols that need PLT entries: any symbol referenced via
    // R_X86_64_PLT32 or R_X86_64_PC32 that is not defined locally.
    // In shared libraries, undefined symbols are resolved at runtime by the
    // dynamic linker, so we need PLT entries for all of them.
    let mut plt_names: Vec<String> = Vec::new();
    let mut plt_seen: FxHashSet<String> = FxHashSet::default();
    for obj in objects.iter() {
        for sec_relas in &obj.relocations {
            for rela in sec_relas {
                let si = rela.sym_idx as usize;
                if si >= obj.symbols.len() {
                    continue;
                }
                let sym = &obj.symbols[si];
                if sym.name.is_empty() {
                    continue;
                }
                // Skip local symbols - they don't need PLT entries
                if sym.is_local() {
                    continue;
                }
                // Layout-anchor symbols (__ehdr_start, _DYNAMIC, _end, ...)
                // are link-time constants the linker itself defines during
                // layout; they are UNDEFINED at this collection point, which
                // previously classified them "external" and gave them PLT
                // slots + JUMP_SLOT relocations. glibc ld.so then computed
                // its own load base from an unrelocated GOT slot and crashed
                // in _dl_start before the first LD_DEBUG line (LK-24).
                if linker_common::is_layout_anchor_symbol(&sym.name) {
                    continue;
                }
                match rela.rela_type {
                    R_X86_64_PLT32 | R_X86_64_PC32 => {
                        if let Some(gsym) = globals.get(sym.name.as_str()) {
                            let locally_defined = gsym.defined_in.is_some() && !gsym.is_dynamic;
                            // External references always need a stub.
                            let external = gsym.is_dynamic
                                || (gsym.defined_in.is_none() && gsym.section_idx == SHN_UNDEF);
                            // GNU semantics: calls to our own EXPORTED functions
                            // also route through the PLT so LD_PRELOAD /
                            // earlier-DSO interposition works. Direct binding
                            // only for hidden/protected visibility (not
                            // exported), version-script-locals, or -Bsymbolic.
                            let hidden = sym.visibility() != 0; // STV_HIDDEN/PROTECTED/INTERNAL
                            let version_local = version_script.as_ref().is_some_and(|vs| {
                                vs.any_local_star()
                                    && !vs.matches_global(&sym_base(&sym.name))
                            }) || excluded_syms.contains(sym.name.as_str());
                            let is_func = (gsym.info & 0xf) == STT_FUNC
                                || sym.sym_type() == STT_FUNC
                                || rela.rela_type == R_X86_64_PLT32;
                            let interposable = locally_defined
                                && is_func
                                && !hidden
                                && !version_local
                                && !bsymbolic;
                            if (external || interposable) && plt_seen.insert(sym.name.to_string()) {
                                plt_names.push(sym.name.to_string());
                            }
                        }
                        // Don't create PLT for symbols not in globals - they are
                        // local/section symbols resolved directly
                    }
                    _ => {}
                }
            }
        }
    }

    // Ensure PLT symbols that are not yet in globals get entries (e.g. libc symbols
    // when libc is not explicitly linked). Create global entries for them so they
    // appear in dynsym and can be resolved by the dynamic linker at runtime.
    for name in &plt_names {
        if !globals.contains_key(name) {
            globals.insert(
                name.clone(),
                GlobalSymbol {
                    value: 0,
                    size: 0,
                    info: (STB_GLOBAL << 4) | STT_FUNC,
                    defined_in: None,
                    from_lib: None,
                    section_idx: SHN_UNDEF,
                    is_dynamic: true,
                    copy_reloc: false,
                    lib_sym_value: 0,
                    version: None,
                    plt_idx: None,
                    got_idx: None,
                },
            );
        }
    }

    // Assign PLT indices to global symbols
    for (plt_idx, name) in plt_names.iter().enumerate() {
        if let Some(gsym) = globals.get_mut(name) {
            gsym.plt_idx = Some(plt_idx);
        }
    }

    // Collect symbols that need GOT entries (GOTPCREL references).
    // For undefined symbols, these need R_X86_64_GLOB_DAT relocations
    // (or R_X86_64_TPOFF64 for TLS symbols referenced via GOTTPOFF).
    let mut got_needed_names: Vec<String> = Vec::new();
    let mut got_needed_seen: FxHashSet<String> = FxHashSet::default();
    let mut tlsgd_seen: FxHashSet<String> = FxHashSet::default();
    let mut tls_got_names: FxHashSet<String> = FxHashSet::default();
    // TLS General-Dynamic symbols: each needs a GOT slot PAIR
    // (DTPMOD64 at slot, DTPOFF64 at slot+8).
    let mut tlsgd_names: Vec<String> = Vec::new();
    // TLS Local-Dynamic: one shared GOT pair (DTPMOD64, 0) per module.
    let mut needs_tlsld_slot = false;
    for obj in objects.iter() {
        for sec_relas in &obj.relocations {
            for rela in sec_relas {
                let si = rela.sym_idx as usize;
                if si >= obj.symbols.len() {
                    continue;
                }
                let sym = &obj.symbols[si];
                if rela.rela_type == R_X86_64_TLSLD {
                    needs_tlsld_slot = true;
                    continue;
                }
                if sym.name.is_empty() {
                    continue;
                }
                // Skip local symbols - they don't need GOT entries in dynsym
                // (e.g. static _Thread_local variables referenced via GOTTPOFF)
                if sym.is_local() {
                    // ...but a local symbol referenced via TLSGD still needs a
                    // GOT pair; use the symbol's mangled per-object identity.
                    if rela.rela_type == R_X86_64_TLSGD && tlsgd_seen.insert(sym.name.to_string()) {
                        tlsgd_names.push(sym.name.to_string());
                    }
                    continue;
                }
                match rela.rela_type {
                    R_X86_64_GOTPCREL
                    | R_X86_64_GOTPCRELX
                    | R_X86_64_REX_GOTPCRELX
                    | R_X86_64_GOTTPOFF => {
                        if got_needed_seen.insert(sym.name.to_string()) {
                            got_needed_names.push(sym.name.to_string());
                        }
                        // Track TLS symbols for proper dynamic relocation emission
                        if sym.sym_type() == STT_TLS {
                            tls_got_names.insert(sym.name.to_string());
                        }
                    }
                    R_X86_64_TLSGD => {
                        if tlsgd_seen.insert(sym.name.to_string()) {
                            tlsgd_names.push(sym.name.to_string());
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    // Ensure GOT-referenced undefined symbols are in globals for dynsym
    for name in &got_needed_names {
        if !globals.contains_key(name) {
            // Use STT_TLS for TLS symbols so the dynamic symbol table has the
            // correct type, allowing the dynamic linker to resolve them properly.
            let stype = if tls_got_names.contains(name) {
                STT_TLS
            } else {
                STT_FUNC
            };
            globals.insert(
                name.clone(),
                GlobalSymbol {
                    value: 0,
                    size: 0,
                    info: (STB_GLOBAL << 4) | stype,
                    defined_in: None,
                    from_lib: None,
                    section_idx: SHN_UNDEF,
                    is_dynamic: true,
                    copy_reloc: false,
                    lib_sym_value: 0,
                    version: None,
                    plt_idx: None,
                    got_idx: None,
                },
            );
        }
    }

    // Pre-scan: collect named global symbols referenced by R_X86_64_64 relocations.
    // These must appear in the dynamic symbol table so the dynamic linker can
    // resolve them (supporting symbol interposition at runtime).
    let mut abs64_sym_names: BTreeSet<String> = BTreeSet::new();
    for obj in objects.iter() {
        for sec_relas in &obj.relocations {
            for rela in sec_relas {
                if rela.rela_type == R_X86_64_64 {
                    let si = rela.sym_idx as usize;
                    if si >= obj.symbols.len() {
                        continue;
                    }
                    let sym = &obj.symbols[si];
                    if !sym.name.is_empty() && !sym.is_local() && sym.sym_type() != STT_SECTION {
                        abs64_sym_names.insert(sym.name.to_string());
                    }
                }
            }
        }
    }

    // Collect all defined global symbols for export
    let mut dyn_sym_names: Vec<String> = Vec::new();
    let mut dyn_sym_seen: FxHashSet<String> = FxHashSet::default();
    let mut exported: Vec<String> = globals
        .iter()
        .filter(|(name, g)| {
            if !(g.defined_in.is_some() && !g.is_dynamic
                && (g.info >> 4) != 0 // not STB_LOCAL
                && g.section_idx != SHN_UNDEF)
            {
                return false;
            }
            // GNU version scripts commonly use `local: *;` to hide all symbols
            // except listed API patterns.  Honor that for shared libraries so
            // FFmpeg-style DSOs expose the intended ABI and produce versioned
            // dynamic symbols for consumers like mpv.
            if let Some(ref vs) = version_script {
                // .symver-derived globals are keyed "base@@VER"/"base@VER";
                // the script's patterns name the BASE. Matching the composed
                // string dropped every explicitly-versioned export: glibc's
                // libc.so lost fdopen/fopen/... (defined as
                // _IO_new_fdopen + `.symver fdopen@@GLIBC_2.2.5`) and
                // sotruss-lib.so failed with `undefined reference to fdopen`
                // even though the DSO was right there on the command line.
                if vs.any_local_star() && !vs.matches_global(&sym_base(name)) {
                    return false;
                }
            }
            // --exclude-libs: symbols pulled in from the named static
            // archives are linked in but NOT re-exported. This is how a
            // shared library statically absorbs a helper archive (OpenSSL's
            // libcrypto.a inside a plugin .so is the canonical case) without
            // leaking that archive's entire symbol table into its ABI, where
            // it would collide with a different version loaded elsewhere in
            // the process.
            if excluded_syms.contains(name.as_str()) {
                return false;
            }
            true
        })
        .map(|(n, _)| n.clone())
        .collect();
    exported.sort();
    for name in exported {
        if dyn_sym_seen.insert(name.clone()) {
            dyn_sym_names.push(name);
        }
    }

    // Also add undefined/dynamic symbols (from -l libs and PLT imports)
    for (name, gsym) in globals.iter() {
        if (gsym.is_dynamic || (gsym.defined_in.is_none() && gsym.section_idx == SHN_UNDEF))
            && !dyn_sym_seen.contains(name)
        {
            dyn_sym_seen.insert(name.clone());
            dyn_sym_names.push(name.clone());
        }
    }

    // Ensure externally-versioned symbols referenced by R_X86_64_64 data
    // relocations are in dynsym.  Version-script-local definitions are resolved
    // with RELATIVE relocations below and must not be re-exported here.
    for name in &abs64_sym_names {
        let is_version_local = version_script
            .as_ref()
            .is_some_and(|vs| vs.any_local_star() && !vs.matches_global(&sym_base(name)));
        if !is_version_local && dyn_sym_seen.insert(name.clone()) {
            dyn_sym_names.push(name.clone());
        }
    }

    // Split .symver-derived names ("foo@@GLIBC_2.34", "foo@GLIBC_2.2.5")
    // into (base name, version, is_default). The .dynstr holds base names;
    // versions become verdef nodes. Without this, ld cannot bind unversioned
    // references (e.g. __libc_start_main) against the DSO.
    let mut sym_versions: Vec<(String, Option<String>, bool)> = Vec::new();
    let mut version_set: Vec<String> = Vec::new();
    for name in &dyn_sym_names {
        if let Some(pos) = name.find("@@") {
            let base = name[..pos].to_string();
            let ver = name[pos + 2..].to_string();
            if !version_set.contains(&ver) {
                version_set.push(ver.clone());
            }
            dynstr.add(&base);
            sym_versions.push((base, Some(ver), true));
        } else if let Some(pos) = name.find('@') {
            let base = name[..pos].to_string();
            let ver = name[pos + 1..].to_string();
            if !version_set.contains(&ver) {
                version_set.push(ver.clone());
            }
            dynstr.add(&base);
            sym_versions.push((base, Some(ver), false));
        } else {
            dynstr.add(name);
            sym_versions.push((name.clone(), None, false));
        }
    }
    // GNU ld emits a Verdef node for EVERY named node of the version script,
    // whether or not a symbol currently binds to it. glibc depends on this:
    // /bin/echo's verneed asks libc.so.6 for GLIBC_2.34/GLIBC_2.14/... and
    // ld.so answers from the Verdef table alone — a missing node is a fatal
    // "version `GLIBC_2.34' not found" even if no exported symbol uses it.
    if let Some(vs) = version_script.as_ref() {
        for node in &vs.nodes {
            if !node.name.is_empty() && !version_set.contains(&node.name) {
                version_set.push(node.name.clone());
            }
        }
    }
    // Version node names must be in .dynstr BEFORE dynstr_size is computed.
    for v in &version_set {
        dynstr.add(v);
    }
    let versioned_name = version_script.as_ref().map(|vs| vs.version_name.clone());
    let base_version_name = soname.clone().unwrap_or_else(|| {
        output_path
            .rsplit('/')
            .next()
            .unwrap_or(output_path)
            .to_string()
    });
    // Named nodes from the version script, in declaration order. An anonymous
    // node ({ global: ...; local: *; }) only restricts visibility and gets no
    // verdef, so it is filtered out here.
    let script_nodes: Vec<linker_common::VersionNode> = version_script
        .as_ref()
        .map(|vs| {
            vs.nodes
                .iter()
                .filter(|n| !n.name.is_empty())
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    if let Some(ref vn) = versioned_name {
        dynstr.add(&base_version_name);
        dynstr.add(vn);
    }
    // Every node name AND every parent name must be in .dynstr before its size
    // is fixed; a parent may name a node that appears later in the file.
    for n in &script_nodes {
        dynstr.add(&n.name);
        if let Some(ref p) = n.parent {
            dynstr.add(p);
        }
    }

    let dynsym_count = 1 + dyn_sym_names.len();
    let dynsym_size = dynsym_count as u64 * 24;
    let dynstr_size = dynstr.as_bytes().len() as u64;

    // Build .gnu.hash
    // Separate defined (hashed) from undefined (unhashed) symbols.
    // .gnu.hash only includes defined symbols; undefined symbols must come
    // first in the symbol table (before symoffset).
    let mut undef_syms: Vec<String> = Vec::new();
    let mut defined_syms: Vec<String> = Vec::new();
    for name in &dyn_sym_names {
        if let Some(g) = globals.get(name) {
            if g.defined_in.is_some() && g.section_idx != SHN_UNDEF {
                defined_syms.push(name.clone());
            } else {
                undef_syms.push(name.clone());
            }
        } else {
            undef_syms.push(name.clone());
        }
    }
    // Reorder: undefined first, then defined
    dyn_sym_names.clear();
    dyn_sym_names.extend(undef_syms.iter().cloned());
    dyn_sym_names.extend(defined_syms.iter().cloned());

    let gnu_hash_symoffset: usize = 1 + undef_syms.len(); // 1 for null entry + undefs
    let num_hashed = defined_syms.len();
    let gnu_hash_nbuckets = if num_hashed == 0 {
        1
    } else {
        num_hashed.next_power_of_two().max(1)
    } as u32;
    // Scale bloom filter size with number of symbols for efficient lookup.
    // Each 64-bit bloom word can effectively track ~32 symbols (2 bits each).
    // Use next power of two for the number of words needed, minimum 1.
    let gnu_hash_bloom_size: u32 = if num_hashed <= 32 {
        1
    } else {
        num_hashed.div_ceil(32).next_power_of_two() as u32
    };
    let gnu_hash_bloom_shift: u32 = 6;

    let hashed_sym_hashes: Vec<u32> = defined_syms
        .iter()
        .map(|name| linker_common::gnu_hash(sym_base(name).as_bytes()))
        .collect();

    let mut bloom_words: Vec<u64> = vec![0u64; gnu_hash_bloom_size as usize];
    for &h in &hashed_sym_hashes {
        let word_idx = ((h / 64) % gnu_hash_bloom_size) as usize;
        bloom_words[word_idx] |= 1u64 << (h as u64 % 64);
        bloom_words[word_idx] |= 1u64 << ((h >> gnu_hash_bloom_shift) as u64 % 64);
    }

    // Sort hashed (defined) symbols by bucket
    if num_hashed > 0 {
        let mut hashed_with_hash: Vec<(String, u32)> = defined_syms
            .iter()
            .zip(hashed_sym_hashes.iter())
            .map(|(n, &h)| (n.clone(), h))
            .collect();
        hashed_with_hash.sort_by_key(|(_, h)| h % gnu_hash_nbuckets);
        // Update defined portion of dyn_sym_names
        for (i, (name, _)) in hashed_with_hash.iter().enumerate() {
            dyn_sym_names[undef_syms.len() + i] = name.clone();
        }
    }

    let hashed_sym_hashes: Vec<u32> = dyn_sym_names[undef_syms.len()..]
        .iter()
        .map(|name| linker_common::gnu_hash(sym_base(name).as_bytes()))
        .collect();

    // O(1) name -> dynsym index (1-based), valid from this point on (after the
    // .gnu.hash bucket sort has frozen the final dynsym order).
    let dyn_sym_index: FxHashMap<&str, u64> = dyn_sym_names
        .iter()
        .enumerate()
        .map(|(i, n)| (n.as_str(), (i + 1) as u64))
        .collect();

    let mut gnu_hash_buckets = vec![0u32; gnu_hash_nbuckets as usize];
    let mut gnu_hash_chains = vec![0u32; num_hashed];
    for (i, &h) in hashed_sym_hashes.iter().enumerate() {
        let bucket = (h % gnu_hash_nbuckets) as usize;
        if gnu_hash_buckets[bucket] == 0 {
            gnu_hash_buckets[bucket] = (gnu_hash_symoffset + i) as u32;
        }
        gnu_hash_chains[i] = h & !1;
    }
    // Single-pass end-of-chain marking (entries are bucket-sorted); the old
    // per-bucket rescan was O(buckets * symbols).
    for i in 0..hashed_sym_hashes.len() {
        let last = i + 1 == hashed_sym_hashes.len()
            || (hashed_sym_hashes[i + 1] % gnu_hash_nbuckets)
                != (hashed_sym_hashes[i] % gnu_hash_nbuckets);
        if last {
            gnu_hash_chains[i] |= 1;
        }
    }

    let (versym_data, verdef_data, verdef_count): (Vec<u8>, Vec<u8>, u64) = if !version_set
        .is_empty()
    {
        // Proper GNU versioning from the objects' .symver names: one verdef
        // node per version; each dynsym entry's versym index selects its node.
        // The base node (1) carries the SONAME.
        let mut versym = Vec::with_capacity(dynsym_count as usize * 2);
        versym.extend_from_slice(&0u16.to_le_bytes()); // dynsym[0]
        // Emit versym in dyn_sym_names' FINAL order. sym_versions was built
        // before the undef-first reorder and the .gnu.hash bucket sort;
        // iterating it here assigned version indices to the WRONG symbols
        // once any reordering happened (glibc libc.so: every .symver export
        // landed on versym 1 "*global*", so versioned references like
        // fdopen@GLIBC_2.2.5 failed at load time). Re-derive each entry's
        // version from its (still composed) name.
        for name in &dyn_sym_names {
            let (ver, hidden): (Option<&str>, bool) = if let Some(pos) = name.find("@@") {
                (Some(&name[pos + 2..]), false)
            } else if let Some(pos) = name.find('@') {
                (Some(&name[pos + 1..]), true)
            } else {
                (None, false)
            };
            let mut idx: u16 = match ver {
                Some(v) => 2 + version_set.iter().position(|x| x == v).unwrap_or(0) as u16,
                None => 1,
            };
            // "name@VER" (single @): non-default version — hidden bit set.
            if hidden {
                idx |= 0x8000;
            }
            versym.extend_from_slice(&idx.to_le_bytes());
        }
        let mut verdef = Vec::new();
        let base_off = dynstr.get_offset(&base_version_name);
        push_verdef_entry(&mut verdef, 1, &base_version_name, base_off, 28);
        for (i, v) in version_set.iter().enumerate() {
            let voff = dynstr.get_offset(v);
            // vd_next chains EVERY node (28 = Verdef 20B + one Verdaux 8B);
            // only the final node terminates with 0. The old unconditional 0
            // ended the chain at the second entry: VERDEFNUM said 27 but
            // ld.so's version lookup walked 2, and every binary needing
            // GLIBC_2.3/2.14/2.34/... aborted with "version not found".
            let next = if i + 1 == version_set.len() { 0 } else { 28 };
            push_verdef_entry(&mut verdef, (2 + i) as u16, v, voff, next);
        }
        (versym, verdef, 1 + version_set.len() as u64)
    } else if script_nodes.len() > 1 {
        // Multi-node version script: one verdef per named node, in declaration
        // order, with the inheritance chain preserved. A defined symbol takes
        // the index of the FIRST node whose `global:` list matches it, which is
        // how GNU ld resolves a symbol named in several nodes.
        let mut versym = Vec::with_capacity(dynsym_count as usize * 2);
        versym.extend_from_slice(&0u16.to_le_bytes());
        for name in &dyn_sym_names {
            let defined = globals
                .get(name)
                .is_some_and(|g| g.defined_in.is_some() && g.section_idx != SHN_UNDEF);
            let idx: u16 = if !defined {
                1
            } else {
                script_nodes
                    .iter()
                    .position(|n| {
                        n.global_patterns
                            .iter()
                            .any(|p| linker_common::wildcard_match_pattern(p, name))
                    })
                    .map_or(1, |i| (2 + i) as u16)
            };
            versym.extend_from_slice(&idx.to_le_bytes());
        }
        let mut verdef = Vec::new();
        let base_off = dynstr.get_offset(&base_version_name);
        // vd_next for a parentless node is 28 (20 verdef + 8 verdaux); a node
        // carrying a parent adds another 8-byte verdaux.
        let node_span =
            |n: &linker_common::VersionNode| -> u32 { 28 + if n.parent.is_some() { 8 } else { 0 } };
        push_verdef_entry(&mut verdef, 1, &base_version_name, base_off, 28);
        for (i, n) in script_nodes.iter().enumerate() {
            let is_last = i + 1 == script_nodes.len();
            let next = if is_last { 0 } else { node_span(n) };
            let noff = dynstr.get_offset(&n.name);
            let parent = n
                .parent
                .as_ref()
                .map(|p| (p.as_str(), dynstr.get_offset(p)));
            push_verdef_entry_with_parent(&mut verdef, (2 + i) as u16, &n.name, noff, next, parent);
        }
        (versym, verdef, 1 + script_nodes.len() as u64)
    } else if let Some(ref vn) = versioned_name {
        let mut versym = Vec::with_capacity(dynsym_count as usize * 2);
        versym.extend_from_slice(&0u16.to_le_bytes());
        for name in &dyn_sym_names {
            let idx: u16 = if let Some(g) = globals.get(name) {
                if g.defined_in.is_some() && g.section_idx != SHN_UNDEF {
                    2
                } else {
                    1
                }
            } else {
                1
            };
            versym.extend_from_slice(&idx.to_le_bytes());
        }
        let mut verdef = Vec::new();
        let base_off = dynstr.get_offset(&base_version_name);
        let ver_off = dynstr.get_offset(vn);
        push_verdef_entry(&mut verdef, 1, &base_version_name, base_off, 28);
        push_verdef_entry(&mut verdef, 2, vn, ver_off, 0);
        (versym, verdef, 2)
    } else {
        (Vec::new(), Vec::new(), 0)
    };
    let versym_size = versym_data.len() as u64;
    let verdef_size = verdef_data.len() as u64;

    let gnu_hash_size: u64 = 16
        + (gnu_hash_bloom_size as u64 * 8)
        + (gnu_hash_nbuckets as u64 * 4)
        + (num_hashed as u64 * 4);

    let plt_size = if plt_names.is_empty() {
        0u64
    } else {
        16 + 16 * plt_names.len() as u64
    };
    let got_plt_count = if plt_names.is_empty() {
        0
    } else {
        3 + plt_names.len()
    };
    let got_plt_size = got_plt_count as u64 * 8;
    let rela_plt_size = plt_names.len() as u64 * 24;

    // Count R_X86_64_RELATIVE relocations needed (for internal absolute addresses)
    // We'll collect them during relocation processing
    let has_init_array = output_sections
        .iter()
        .any(|s| s.name == ".init_array" && s.mem_size > 0);
    let has_fini_array = output_sections
        .iter()
        .any(|s| s.name == ".fini_array" && s.mem_size > 0);
    let mut dyn_count = needed_sonames.len() as u64 + 10; // 9 fixed entries + DT_NULL
    if soname.is_some() {
        dyn_count += 1;
    }
    if has_init_array {
        dyn_count += 2;
    }
    if has_fini_array {
        dyn_count += 2;
    }
    if !plt_names.is_empty() {
        dyn_count += 4;
    } // DT_PLTGOT, DT_PLTRELSZ, DT_PLTREL, DT_JMPREL
    if rpath_string.is_some() {
        dyn_count += 1;
    } // DT_RUNPATH or DT_RPATH
    if bsymbolic {
        dyn_count += 2;
    } // DT_SYMBOLIC + DT_FLAGS(DF_SYMBOLIC)
    if verdef_count > 0 {
        dyn_count += 3;
    } // DT_VERSYM, DT_VERDEF, DT_VERDEFNUM
    let dynamic_size = dyn_count * 16;

    let has_tls_sections = output_sections
        .iter()
        .any(|s| s.flags & SHF_TLS != 0 && s.flags & SHF_ALLOC != 0);

    // Identify output sections that have R_X86_64_64 relocations (need RELATIVE
    // relocations at load time). These must go in a writable segment so the
    // dynamic linker can patch them. We track them by output section index.
    let mut sections_with_abs_relocs: crate::common::fx_hash::FxHashSet<usize> =
        crate::common::fx_hash::FxHashSet::default();
    for obj in objects.iter() {
        for (sec_idx, sec_relas) in obj.relocations.iter().enumerate() {
            for rela in sec_relas {
                if rela.rela_type == R_X86_64_64 {
                    // Find which output section this input section maps to
                    let obj_idx_search = objects.iter().position(|o| std::ptr::eq(o, obj));
                    if let Some(oi) = obj_idx_search {
                        if let Some(&(out_idx, _)) = section_map.get(&(oi, sec_idx)) {
                            sections_with_abs_relocs.insert(out_idx);
                        }
                    }
                }
            }
        }
    }

    // A section is "pure rodata" if it's read-only and has no absolute relocations.
    // Sections with absolute relocations go in the RW segment (as .data.rel.ro).
    let is_pure_rodata = |idx: usize, sec: &OutputSection| -> bool {
        sec.flags & SHF_ALLOC != 0
            && sec.flags & SHF_EXECINSTR == 0
            && sec.flags & SHF_WRITE == 0
            && sec.flags & SHF_TLS == 0
            && sec.sh_type != SHT_NOBITS
            && !sections_with_abs_relocs.contains(&idx)
    };
    let is_relro_rodata = |idx: usize, sec: &OutputSection| -> bool {
        sec.flags & SHF_ALLOC != 0
            && sec.flags & SHF_EXECINSTR == 0
            && sec.flags & SHF_WRITE == 0
            && sec.flags & SHF_TLS == 0
            && sec.sh_type != SHT_NOBITS
            && sections_with_abs_relocs.contains(&idx)
    };

    // phdrs: PHDR, LOAD(ro), LOAD(text), LOAD(rodata), LOAD(rw), DYNAMIC, GNU_STACK, [GNU_RELRO], [TLS]
    let has_relro = !sections_with_abs_relocs.is_empty();
    let mut phdr_count: u64 = 7; // base count
    if has_tls_sections {
        phdr_count += 1;
    }
    if has_relro {
        phdr_count += 1;
    }
    let phdr_total_size = phdr_count * 56;

    // === Layout ===
    let mut offset = 64 + phdr_total_size;

    offset = (offset + 7) & !7;
    let gnu_hash_offset = offset;
    let gnu_hash_addr = vaddr!(offset);
    offset += gnu_hash_size;
    offset = (offset + 7) & !7;
    let dynsym_offset = offset;
    let dynsym_addr = vaddr!(offset);
    offset += dynsym_size;
    let dynstr_offset = offset;
    let dynstr_addr = vaddr!(offset);
    offset += dynstr_size;
    offset = (offset + 1) & !1;
    let versym_offset = offset;
    let versym_addr = vaddr!(offset);
    offset += versym_size;
    offset = (offset + 7) & !7;
    let verdef_offset = offset;
    let verdef_addr = vaddr!(offset);
    offset += verdef_size;

    // Text segment
    new_segment!();
    let text_page_offset = offset;
    let text_page_addr = vaddr!(offset);
    for sec in output_sections.iter_mut() {
        if sec.flags & SHF_EXECINSTR != 0 && sec.flags & SHF_ALLOC != 0 {
            let a = sec.alignment.max(1);
            offset = (offset + a - 1) & !(a - 1);
            sec.addr = vaddr!(offset);
            sec.file_offset = offset;
            offset += sec.mem_size;
        }
    }
    // PLT goes at the end of the text segment
    let (plt_addr, plt_offset) = if plt_size > 0 {
        offset = (offset + 15) & !15;
        let a = vaddr!(offset);
        let o = offset;
        offset += plt_size;
        (a, o)
    } else {
        (0u64, 0u64)
    };
    let text_total_size = offset - text_page_offset;

    // Rodata segment - only pure rodata (no absolute relocations)
    new_segment!();
    let rodata_page_offset = offset;
    let rodata_page_addr = vaddr!(offset);
    for (idx, sec) in output_sections.iter_mut().enumerate() {
        if is_pure_rodata(idx, sec) {
            let a = sec.alignment.max(1);
            offset = (offset + a - 1) & !(a - 1);
            sec.addr = vaddr!(offset);
            sec.file_offset = offset;
            offset += sec.mem_size;
        }
    }
    let rodata_total_size = offset - rodata_page_offset;

    // RW segment - includes RELRO sections (rodata with abs relocs), then linker
    // data structures, then actual writable data
    new_segment!();
    let rw_page_offset = offset;
    let rw_page_addr = vaddr!(offset);

    // First: RELRO sections (rodata that needs dynamic relocations)
    let _relro_start_offset = offset;
    for (idx, sec) in output_sections.iter_mut().enumerate() {
        if is_relro_rodata(idx, sec) {
            let a = sec.alignment.max(1);
            offset = (offset + a - 1) & !(a - 1);
            sec.addr = vaddr!(offset);
            sec.file_offset = offset;
            offset += sec.mem_size;
        }
    }

    let mut init_array_addr = 0u64;
    let mut init_array_size = 0u64;
    let mut fini_array_addr = 0u64;
    let mut fini_array_size = 0u64;

    for sec in output_sections.iter_mut() {
        if sec.name == ".init_array" {
            let a = sec.alignment.max(8);
            offset = (offset + a - 1) & !(a - 1);
            sec.addr = vaddr!(offset);
            sec.file_offset = offset;
            init_array_addr = sec.addr;
            init_array_size = sec.mem_size;
            offset += sec.mem_size;
            break;
        }
    }
    for sec in output_sections.iter_mut() {
        if sec.name == ".fini_array" {
            let a = sec.alignment.max(8);
            offset = (offset + a - 1) & !(a - 1);
            sec.addr = vaddr!(offset);
            sec.file_offset = offset;
            fini_array_addr = sec.addr;
            fini_array_size = sec.mem_size;
            offset += sec.mem_size;
            break;
        }
    }

    // GOT entries were already collected into got_needed_names above.
    let got_needed = &got_needed_names;

    // Reserve space for .rela.dyn (will be filled later)
    offset = (offset + 7) & !7;
    let rela_dyn_offset = offset;
    let rela_dyn_addr = vaddr!(offset);
    // Each R_X86_64_64 reloc in input becomes one R_X86_64_RELATIVE entry.
    let mut max_rela_count: usize = 0;
    for obj in objects.iter() {
        for sec_relas in &obj.relocations {
            for rela in sec_relas {
                if rela.rela_type == R_X86_64_64 {
                    max_rela_count += 1;
                }
            }
        }
    }
    // Also init_array/fini_array entries are pointers
    for sec in output_sections.iter() {
        if sec.name == ".init_array" || sec.name == ".fini_array" {
            max_rela_count += (sec.mem_size / 8) as usize;
        }
    }
    // GOT entries need either RELATIVE (local) or GLOB_DAT (external) relocations
    max_rela_count += got_needed.len();
    // Each TLSGD pair may emit DTPMOD64 + DTPOFF64; the TLSLD slot emits DTPMOD64.
    max_rela_count += tlsgd_names.len() * 2 + if needs_tlsld_slot { 1 } else { 0 };
    let rela_dyn_max_size = max_rela_count as u64 * 24;
    offset += rela_dyn_max_size;

    // .rela.plt (JMPREL) for PLT GOT entries
    offset = (offset + 7) & !7;
    let rela_plt_offset = offset;
    let rela_plt_addr = vaddr!(offset);
    offset += rela_plt_size;

    offset = (offset + 7) & !7;
    let dynamic_offset = offset;
    let dynamic_addr = vaddr!(offset);
    offset += dynamic_size;

    // End of RELRO region (page-aligned up for PT_GNU_RELRO).
    // Everything after this must be on a new page so that mprotect(PROT_READ)
    // on the RELRO region doesn't affect writable data (GOT.PLT, GOT, .data, .bss).
    // With densely packed file offsets the RELRO end must be aligned in
    // ADDRESS space: ld.so mprotects page-rounded [vaddr, vaddr+memsz), so
    // aligning the file offset would leave the boundary mid-page and
    // write-protect the head of the following section.
    let relro_end_addr = vaddr!(offset) + packer.padding_to_page(offset);
    if has_relro {
        offset += packer.padding_to_page(offset); // advance to page boundary
    }

    // .got.plt entries - MUST be after RELRO boundary since dynamic linker
    // needs to write to them during lazy PLT resolution
    offset = (offset + 7) & !7;
    let got_plt_offset = offset;
    let got_plt_addr = vaddr!(offset);
    offset += got_plt_size;

    // GOT for locally-resolved symbols, followed by TLS GD pairs and the
    // optional LD pair. Layout:
    //   [got_needed slots][GD pair 0][GD pair 1]...[LD pair]
    let got_offset = offset;
    let got_addr = vaddr!(offset);
    let tlsgd_got_base = got_needed.len() as u64 * 8; // offset of first GD pair within .got
    let tlsld_got_off = tlsgd_got_base + tlsgd_names.len() as u64 * 16;
    let got_size = tlsld_got_off + if needs_tlsld_slot { 16 } else { 0 };
    offset += got_size;

    for sec in output_sections.iter_mut() {
        if sec.flags & SHF_ALLOC != 0
            && sec.flags & SHF_WRITE != 0
            && sec.sh_type != SHT_NOBITS
            && sec.name != ".init_array"
            && sec.name != ".fini_array"
            && sec.flags & SHF_TLS == 0
        {
            let a = sec.alignment.max(1);
            offset = (offset + a - 1) & !(a - 1);
            sec.addr = vaddr!(offset);
            sec.file_offset = offset;
            offset += sec.mem_size;
        }
    }

    // TLS sections
    let mut tls_addr = 0u64;
    let mut tls_file_offset = 0u64;
    let mut tls_file_size = 0u64;
    let mut tls_mem_size = 0u64;
    let mut tls_align = 1u64;
    for sec in output_sections.iter_mut() {
        if sec.flags & SHF_TLS != 0 && sec.flags & SHF_ALLOC != 0 && sec.sh_type != SHT_NOBITS {
            let a = sec.alignment.max(1);
            offset = (offset + a - 1) & !(a - 1);
            sec.addr = vaddr!(offset);
            sec.file_offset = offset;
            if tls_addr == 0 {
                tls_addr = sec.addr;
                tls_file_offset = offset;
                tls_align = a;
            }
            tls_file_size += sec.mem_size;
            tls_mem_size += sec.mem_size;
            offset += sec.mem_size;
        }
    }
    if tls_addr == 0 && has_tls_sections {
        tls_addr = vaddr!(offset);
        tls_file_offset = offset;
    }
    for sec in output_sections.iter_mut() {
        if sec.flags & SHF_TLS != 0 && sec.sh_type == SHT_NOBITS {
            let a = sec.alignment.max(1);
            let aligned = (tls_mem_size + a - 1) & !(a - 1);
            sec.addr = tls_addr + aligned;
            sec.file_offset = offset;
            tls_mem_size = aligned + sec.mem_size;
            if a > tls_align {
                tls_align = a;
            }
        }
    }
    tls_mem_size = (tls_mem_size + tls_align - 1) & !(tls_align - 1);
    let has_tls = tls_addr != 0;

    let bss_addr = vaddr!(offset);
    let mut bss_size = 0u64;
    for sec in output_sections.iter_mut() {
        if sec.sh_type == SHT_NOBITS && sec.flags & SHF_ALLOC != 0 && sec.flags & SHF_TLS == 0 {
            let a = sec.alignment.max(1);
            let aligned = (bss_addr + bss_size + a - 1) & !(a - 1);
            bss_size = aligned - bss_addr + sec.mem_size;
            sec.addr = aligned;
            sec.file_offset = offset;
        }
    }

    // Merge section data
    for sec in output_sections.iter_mut() {
        if sec.sh_type == SHT_NOBITS {
            continue;
        }
        let mut data = vec![0u8; sec.mem_size as usize];
        for input in &sec.inputs {
            let sd = &objects[input.object_idx].section_data[input.section_idx];
            let s = input.output_offset as usize;
            let e = s + sd.len();
            if e <= data.len() && !sd.is_empty() {
                data[s..e].copy_from_slice(sd);
            }
        }
        sec.data = data;
    }

    // Update global symbol addresses
    for (_, gsym) in globals.iter_mut() {
        if let Some(obj_idx) = gsym.defined_in {
            if gsym.section_idx == SHN_COMMON || gsym.section_idx == 0xffff {
                if let Some(bss_sec) = output_sections.iter().find(|s| s.name == ".bss") {
                    gsym.value += bss_sec.addr;
                }
            } else if gsym.section_idx != SHN_UNDEF && gsym.section_idx != SHN_ABS {
                let si = gsym.section_idx as usize;
                if let Some(&(oi, so)) = section_map.get(&(obj_idx, si)) {
                    gsym.value += output_sections[oi].addr + so;
                }
            }
        }
    }

    // Define linker-provided symbols
    let linker_addrs = LinkerSymbolAddresses {
        base_addr,
        got_addr,
        dynamic_addr,
        bss_addr,
        bss_size,
        text_end: text_page_addr + text_total_size,
        data_start: rw_page_addr,
        init_array_start: init_array_addr,
        init_array_size,
        fini_array_start: fini_array_addr,
        fini_array_size,
        preinit_array_start: 0,
        preinit_array_size: 0,
        rela_iplt_start: 0,
        rela_iplt_size: 0,
    };
    for sym in &get_standard_linker_symbols(&linker_addrs) {
        let entry = globals.entry(sym.name.to_string()).or_insert(GlobalSymbol {
            value: 0,
            size: 0,
            info: (sym.binding << 4),
            defined_in: None,
            from_lib: None,
            plt_idx: None,
            got_idx: None,
            section_idx: SHN_ABS,
            is_dynamic: false,
            copy_reloc: false,
            lib_sym_value: 0,
            version: None,
        });
        if entry.defined_in.is_none() && !entry.is_dynamic {
            entry.value = sym.value;
            entry.defined_in = Some(usize::MAX);
            entry.section_idx = SHN_ABS;
        }
    }

    // Auto-generate __start_<section> / __stop_<section> symbols (GNU ld feature)
    for (name, addr) in linker_common::resolve_start_stop_symbols(output_sections) {
        if let Some(entry) = globals.get_mut(&name) {
            if entry.defined_in.is_none() && !entry.is_dynamic {
                entry.value = addr;
                entry.defined_in = Some(usize::MAX);
                entry.section_idx = SHN_ABS;
            }
        }
    }

    // === Build output buffer ===
    let file_size = offset as usize;
    let mut out = vec![0u8; file_size];

    // ELF header
    out[0..4].copy_from_slice(&ELF_MAGIC);
    out[4] = ELFCLASS64;
    out[5] = ELFDATA2LSB;
    out[6] = 1;
    w16(&mut out, 16, ET_DYN); // Shared object
    w16(&mut out, 18, EM_X86_64);
    w32(&mut out, 20, 1);
    // e_entry: GNU ld sets the entry point for ET_DYN outputs too — to the
    // `-e` symbol if given, else to `_start` when the output defines it,
    // else 0. glibc's ld.so DEPENDS on this: it is linked `-shared` with no
    // `-e`, defines `_start` (RTLD_START in rtld.c), and is EXECUTED
    // directly (`./ld.so --library-path ... prog`); the kernel jumps to
    // e_entry, so a hardcoded 0 made the kernel execute the ELF header
    // bytes at the map base (SIGSEGV before any LD_DEBUG output).
    let e_entry = globals
        .get("_start")
        .filter(|s| s.defined_in.is_some())
        .map(|s| s.value)
        .unwrap_or(0);
    w64(&mut out, 24, e_entry);
    w64(&mut out, 32, 64); // e_phoff
    w64(&mut out, 40, 0); // e_shoff = 0 (no section headers for now)
    w32(&mut out, 48, 0);
    w16(&mut out, 52, 64);
    w16(&mut out, 54, 56);
    w16(&mut out, 56, phdr_count as u16);
    w16(&mut out, 58, 64);
    w16(&mut out, 60, 0);
    w16(&mut out, 62, 0);

    // Program headers
    let mut ph = 64usize;
    wphdr(
        &mut out,
        ph,
        PT_PHDR,
        PF_R,
        64,
        base_addr + 64,
        phdr_total_size,
        phdr_total_size,
        8,
    );
    ph += 56;
    // Initial read-only metadata segment: ELF/PHDR + dynamic lookup tables.
    // Keep provider-side version sections inside this LOAD segment; linkers and
    // dynamic loaders expect DT_VERSYM/DT_VERDEF virtual addresses to be mapped.
    let ro_seg_end = if verdef_size > 0 {
        verdef_offset + verdef_size
    } else if versym_size > 0 {
        versym_offset + versym_size
    } else {
        dynstr_offset + dynstr_size
    };
    wphdr(
        &mut out, ph, PT_LOAD, PF_R, 0, base_addr, ro_seg_end, ro_seg_end, PAGE_SIZE,
    );
    ph += 56;
    if text_total_size > 0 {
        wphdr(
            &mut out,
            ph,
            PT_LOAD,
            PF_R | PF_X,
            text_page_offset,
            text_page_addr,
            text_total_size,
            text_total_size,
            PAGE_SIZE,
        );
        ph += 56;
    } else {
        wphdr(
            &mut out,
            ph,
            PT_LOAD,
            PF_R | PF_X,
            text_page_offset,
            text_page_addr,
            0,
            0,
            PAGE_SIZE,
        );
        ph += 56;
    }
    wphdr(
        &mut out,
        ph,
        PT_LOAD,
        PF_R,
        rodata_page_offset,
        rodata_page_addr,
        rodata_total_size,
        rodata_total_size,
        PAGE_SIZE,
    );
    ph += 56;
    let rw_filesz = offset - rw_page_offset;
    let rw_memsz = if bss_size > 0 {
        (bss_addr + bss_size) - rw_page_addr
    } else {
        rw_filesz
    };
    wphdr(
        &mut out,
        ph,
        PT_LOAD,
        PF_R | PF_W,
        rw_page_offset,
        rw_page_addr,
        rw_filesz,
        rw_memsz,
        PAGE_SIZE,
    );
    ph += 56;
    wphdr(
        &mut out,
        ph,
        PT_DYNAMIC,
        PF_R | PF_W,
        dynamic_offset,
        dynamic_addr,
        dynamic_size,
        dynamic_size,
        8,
    );
    ph += 56;
    wphdr(&mut out, ph, PT_GNU_STACK, PF_R | PF_W, 0, 0, 0, 0, 0x10);
    ph += 56;
    if has_relro {
        let relro_filesz = relro_end_addr - rw_page_addr;
        wphdr(
            &mut out,
            ph,
            PT_GNU_RELRO,
            PF_R,
            rw_page_offset,
            rw_page_addr,
            relro_filesz,
            relro_filesz,
            1,
        );
        ph += 56;
    }
    if has_tls {
        wphdr(
            &mut out,
            ph,
            PT_TLS,
            PF_R,
            tls_file_offset,
            tls_addr,
            tls_file_size,
            tls_mem_size,
            tls_align,
        );
    }

    // .gnu.hash
    let gh = gnu_hash_offset as usize;
    w32(&mut out, gh, gnu_hash_nbuckets);
    w32(&mut out, gh + 4, gnu_hash_symoffset as u32);
    w32(&mut out, gh + 8, gnu_hash_bloom_size);
    w32(&mut out, gh + 12, gnu_hash_bloom_shift);
    let bloom_off = gh + 16;
    for (i, &bw) in bloom_words.iter().enumerate() {
        w64(&mut out, bloom_off + i * 8, bw);
    }
    let buckets_off = bloom_off + (gnu_hash_bloom_size as usize * 8);
    for (i, &b) in gnu_hash_buckets.iter().enumerate() {
        w32(&mut out, buckets_off + i * 4, b);
    }
    let chains_off = buckets_off + (gnu_hash_nbuckets as usize * 4);
    for (i, &c) in gnu_hash_chains.iter().enumerate() {
        w32(&mut out, chains_off + i * 4, c);
    }

    // .dynsym
    let mut ds = dynsym_offset as usize + 24; // skip null entry
    for name in &dyn_sym_names {
        let no = dynstr.get_offset(&sym_base(name)) as u32;
        w32(&mut out, ds, no);
        if let Some(gsym) = globals.get(name) {
            if gsym.defined_in.is_some() && !gsym.is_dynamic && gsym.section_idx != SHN_UNDEF {
                // Exported defined symbol: preserve original st_info (type + binding)
                if ds + 5 < out.len() {
                    out[ds + 4] = gsym.info;
                    out[ds + 5] = 0;
                }
                // shndx=1: marks symbol as defined (non-UNDEF). The dynamic linker
                // only checks UNDEF vs defined, not the actual section index.
                w16(&mut out, ds + 6, 1);
                // For TLS symbols, the value must be the offset within the TLS segment,
                // not the virtual address. The dynamic linker uses this offset to
                // compute the thread-pointer-relative address.
                let sym_val = if (gsym.info & 0xf) == STT_TLS && tls_mem_size > 0 {
                    gsym.value - tls_addr
                } else {
                    gsym.value
                };
                w64(&mut out, ds + 8, sym_val);
                w64(&mut out, ds + 16, gsym.size);
            } else {
                // Undefined symbol (from -l dependencies or weak refs)
                // Preserve original binding (STB_WEAK vs STB_GLOBAL) and type
                let bind = gsym.info >> 4;
                let stype = gsym.info & 0xf;
                let st_info = (bind << 4) | if stype != 0 { stype } else { STT_FUNC };
                if ds + 5 < out.len() {
                    out[ds + 4] = st_info;
                    out[ds + 5] = 0;
                }
                w16(&mut out, ds + 6, 0);
                w64(&mut out, ds + 8, 0);
                w64(&mut out, ds + 16, 0);
            }
        } else {
            if ds + 5 < out.len() {
                out[ds + 4] = (STB_GLOBAL << 4) | STT_FUNC;
                out[ds + 5] = 0;
            }
            w16(&mut out, ds + 6, 0);
            w64(&mut out, ds + 8, 0);
            w64(&mut out, ds + 16, 0);
        }
        ds += 24;
    }

    // .dynstr and provider-side GNU symbol versioning tables.
    write_bytes(&mut out, dynstr_offset as usize, dynstr.as_bytes());
    if !versym_data.is_empty() {
        write_bytes(&mut out, versym_offset as usize, &versym_data);
    }
    if !verdef_data.is_empty() {
        write_bytes(&mut out, verdef_offset as usize, &verdef_data);
    }

    // Section data
    for sec in output_sections.iter() {
        if sec.sh_type == SHT_NOBITS || sec.data.is_empty() {
            continue;
        }
        write_bytes(&mut out, sec.file_offset as usize, &sec.data);
    }

    // .plt - PLT stubs for external dynamic symbols
    if plt_size > 0 {
        let po = plt_offset as usize;
        // PLT[0] - the resolver stub (16 bytes)
        out[po] = 0xff;
        out[po + 1] = 0x35; // push [GOT+8] (link_map)
        w32(
            &mut out,
            po + 2,
            ((got_plt_addr + 8) as i64 - (plt_addr + 6) as i64) as u32,
        );
        out[po + 6] = 0xff;
        out[po + 7] = 0x25; // jmp [GOT+16] (resolver)
        w32(
            &mut out,
            po + 8,
            ((got_plt_addr + 16) as i64 - (plt_addr + 12) as i64) as u32,
        );
        for i in 12..16 {
            out[po + i] = 0x90;
        } // nop padding

        // PLT[1..N] - per-symbol stubs (16 bytes each)
        for (i, _) in plt_names.iter().enumerate() {
            let ep = po + 16 + i * 16;
            let pea = plt_addr + 16 + i as u64 * 16;
            let gea = got_plt_addr + 24 + i as u64 * 8;
            out[ep] = 0xff;
            out[ep + 1] = 0x25; // jmp [GOT.PLT slot]
            w32(&mut out, ep + 2, (gea as i64 - (pea + 6) as i64) as u32);
            out[ep + 6] = 0x68;
            w32(&mut out, ep + 7, i as u32); // push <plt_index>
            out[ep + 11] = 0xe9; // jmp PLT[0]
            w32(
                &mut out,
                ep + 12,
                (plt_addr as i64 - (pea + 16) as i64) as u32,
            );
        }
    }

    // .got.plt
    if got_plt_size > 0 {
        let gp = got_plt_offset as usize;
        w64(&mut out, gp, dynamic_addr); // GOT[0] = _DYNAMIC
        w64(&mut out, gp + 8, 0); // GOT[1] = 0 (link_map, filled by ld.so)
        w64(&mut out, gp + 16, 0); // GOT[2] = 0 (resolver, filled by ld.so)
        for (i, _) in plt_names.iter().enumerate() {
            // GOT[3+i] = address of "push <index>" in PLT stub (lazy binding)
            w64(&mut out, gp + 24 + i * 8, plt_addr + 16 + i as u64 * 16 + 6);
        }
    }

    // .rela.plt - JMPREL relocations for GOT.PLT entries
    if rela_plt_size > 0 {
        let mut rp = rela_plt_offset as usize;
        let gpb = got_plt_addr + 24; // base of per-symbol GOT.PLT slots
        for (i, name) in plt_names.iter().enumerate() {
            let gea = gpb + i as u64 * 8;
            // Find symbol index in dynsym
            let si = dyn_sym_index.get(name.as_str()).copied().unwrap_or(0);
            w64(&mut out, rp, gea); // r_offset = GOT.PLT slot address
            w64(&mut out, rp + 8, (si << 32) | R_X86_64_JUMP_SLOT as u64);
            w64(&mut out, rp + 16, 0); // r_addend = 0
            rp += 24;
        }
    }

    // Build GOT entries map
    let mut got_sym_addrs: FxHashMap<String, u64> = FxHashMap::default();
    for (i, name) in got_needed.iter().enumerate() {
        let gea = got_addr + i as u64 * 8;
        got_sym_addrs.insert(name.clone(), gea);
        // Fill GOT with resolved symbol value (skip TLS - handled in reloc loop below)
        if !tls_got_names.contains(name) {
            if let Some(gsym) = globals.get(name) {
                if gsym.defined_in.is_some() && !gsym.is_dynamic {
                    w64(&mut out, (got_offset + i as u64 * 8) as usize, gsym.value);
                }
            }
        }
    }

    // Apply relocations and collect dynamic relocation entries
    let globals_snap: FxHashMap<String, GlobalSymbol> = globals.clone();
    let mut rela_dyn_entries: Vec<(u64, u64)> = Vec::new(); // (offset, value) for RELATIVE relocs
    let mut glob_dat_entries: Vec<(u64, String)> = Vec::new(); // (offset, sym_name) for GLOB_DAT relocs
    let mut tpoff64_entries: Vec<(u64, String)> = Vec::new(); // (offset, sym_name) for R_X86_64_TPOFF64 relocs
    let mut abs64_entries: Vec<(u64, String, i64)> = Vec::new(); // (offset, sym_name, addend) for R_X86_64_64 relocs
                                                                 // TLS module-id relocations: (got_slot_vaddr, sym_name_or_empty).
                                                                 // DTPMOD64 always needed (module id known only at load time). DTPOFF64
                                                                 // needed only for symbols that may be interposed (named globals).
    let mut dtpmod64_entries: Vec<(u64, String)> = Vec::new();
    let mut dtpoff64_entries: Vec<(u64, String)> = Vec::new();

    // TLS GD/LD GOT pair setup
    let mut tlsgd_slot_addr: FxHashMap<String, u64> = FxHashMap::default();
    for (i, name) in tlsgd_names.iter().enumerate() {
        let slot = got_addr + tlsgd_got_base + i as u64 * 16;
        tlsgd_slot_addr.insert(name.clone(), slot);
        dtpmod64_entries.push((slot, String::new())); // module id of THIS module
                                                      // DTPOFF: statically known when the symbol is defined here; store it.
        let mut static_off: Option<u64> = None;
        if let Some(g) = globals_snap.get(name) {
            if g.defined_in.is_some() && !g.is_dynamic && g.section_idx != SHN_UNDEF {
                static_off = Some(g.value.wrapping_sub(tls_addr));
            }
        }
        // Local TLS symbols (not in globals): resolve from object symtabs.
        if static_off.is_none() && !globals_snap.contains_key(name) {
            'outer: for (oi, obj) in objects.iter().enumerate() {
                for sym in &obj.symbols {
                    if sym.name == *name && sym.shndx != 0 {
                        if let Some(&(out_i, so)) = section_map.get(&(oi, sym.shndx as usize)) {
                            static_off = Some(
                                (output_sections[out_i].addr + so + sym.value)
                                    .wrapping_sub(tls_addr),
                            );
                            break 'outer;
                        }
                    }
                }
            }
        }
        match static_off {
            Some(off) => {
                w64(
                    &mut out,
                    (got_offset + tlsgd_got_base + i as u64 * 16 + 8) as usize,
                    off,
                );
            }
            None => dtpoff64_entries.push((slot + 8, name.clone())),
        }
    }
    let tlsld_slot = if needs_tlsld_slot {
        let slot = got_addr + tlsld_got_off;
        dtpmod64_entries.push((slot, String::new()));
        // slot+8 stays 0 (offsets computed via DTPOFF32 addends)
        Some(slot)
    } else {
        None
    };

    // Add RELATIVE entries for GOT entries that point to local symbols,
    // GLOB_DAT entries for GOT entries that point to external non-TLS symbols,
    // and TPOFF64 entries for GOT entries that point to external TLS symbols.
    for (i, name) in got_needed.iter().enumerate() {
        let gea = got_addr + i as u64 * 8;
        let is_tls = tls_got_names.contains(name);
        if let Some(gsym) = globals_snap.get(name) {
            if gsym.defined_in.is_some() && !gsym.is_dynamic && gsym.section_idx != SHN_UNDEF {
                if is_tls {
                    // Locally-defined TLS symbol: compute TPOFF and store in GOT
                    let tpoff = (gsym.value as i64 - tls_addr as i64) - tls_mem_size as i64;
                    w64(&mut out, (got_offset + i as u64 * 8) as usize, tpoff as u64);
                    // No dynamic relocation needed - statically resolved
                } else {
                    rela_dyn_entries.push((gea, gsym.value));
                }
            } else if is_tls {
                // External TLS symbol - needs R_X86_64_TPOFF64 dynamic relocation
                tpoff64_entries.push((gea, name.clone()));
            } else {
                // External non-TLS symbol - needs GLOB_DAT
                glob_dat_entries.push((gea, name.clone()));
            }
        } else if is_tls {
            // Unknown TLS symbol - needs R_X86_64_TPOFF64
            tpoff64_entries.push((gea, name.clone()));
        } else {
            // Unknown symbol - needs GLOB_DAT
            glob_dat_entries.push((gea, name.clone()));
        }
    }

    for obj_idx in 0..objects.len() {
        for sec_idx in 0..objects[obj_idx].sections.len() {
            let relas = &objects[obj_idx].relocations[sec_idx];
            if relas.is_empty() {
                continue;
            }
            let (out_idx, sec_off) = match section_map.get(&(obj_idx, sec_idx)) {
                Some(&v) => v,
                None => continue,
            };
            let sa = output_sections[out_idx].addr;
            let sfo = output_sections[out_idx].file_offset;

            for rela in relas {
                let si = rela.sym_idx as usize;
                if si >= objects[obj_idx].symbols.len() {
                    continue;
                }
                let sym = &objects[obj_idx].symbols[si];
                let p = sa + sec_off + rela.offset;
                let fp = (sfo + sec_off + rela.offset) as usize;
                let a = rela.addend;
                let s = resolve_sym(
                    obj_idx,
                    sym,
                    &globals_snap,
                    section_map,
                    output_sections,
                    plt_addr,
                );

                match rela.rela_type {
                    R_X86_64_64 => {
                        let val = (s as i64 + a) as u64;
                        w64(&mut out, fp, val);
                        // Determine what kind of dynamic relocation to emit.
                        // Named global/weak symbols need R_X86_64_64 dynamic relocs
                        // (with symbol index) to support symbol interposition.
                        // Section symbols and local symbols use R_X86_64_RELATIVE.
                        let is_version_local = version_script
                            .as_ref()
                            .is_some_and(|vs| vs.any_local_star() && !vs.matches_global(&sym.name));
                        let locally_defined = globals_snap
                            .get(sym.name.as_str())
                            .map(|g| g.defined_in.is_some() && !g.is_dynamic)
                            .unwrap_or(false);
                        let is_named_global = !sym.name.is_empty()
                            && !sym.is_local()
                            && sym.sym_type() != STT_SECTION
                            && !is_version_local
                            && !(bsymbolic && locally_defined);
                        if is_named_global {
                            abs64_entries.push((p, sym.name.to_string(), a));
                        } else if s != 0 {
                            rela_dyn_entries.push((p, val));
                        }
                    }
                    R_X86_64_PC32 | R_X86_64_PLT32 => {
                        // For dynamic symbols, redirect through PLT
                        let t = if !sym.name.is_empty() && !sym.is_local() {
                            if let Some(g) = globals_snap.get(sym.name.as_str()) {
                                if let Some(pi) = g.plt_idx {
                                    plt_addr + 16 + pi as u64 * 16
                                } else {
                                    s
                                }
                            } else {
                                s
                            }
                        } else {
                            s
                        };
                        w32(&mut out, fp, (t as i64 + a - p as i64) as u32);
                    }
                    // TODO: R_X86_64_32/32S are not position-independent and should
                    // ideally emit a diagnostic when used in shared libraries. For now
                    // we apply them statically which works for simple cases but may fail
                    // if the library is loaded at a high address.
                    R_X86_64_32 => {
                        w32(&mut out, fp, (s as i64 + a) as u32);
                    }
                    R_X86_64_32S => {
                        w32(&mut out, fp, (s as i64 + a) as u32);
                    }
                    R_X86_64_GOTPCREL | R_X86_64_GOTPCRELX | R_X86_64_REX_GOTPCRELX => {
                        if let Some(&gea) = got_sym_addrs.get(sym.name.as_str()) {
                            w32(&mut out, fp, (gea as i64 + a - p as i64) as u32);
                        } else if (rela.rela_type == R_X86_64_GOTPCRELX
                            || rela.rela_type == R_X86_64_REX_GOTPCRELX)
                            && !sym.name.is_empty()
                        {
                            // GOT relaxation: convert to LEA
                            if let Some(g) = globals_snap.get(sym.name.as_str()) {
                                if g.defined_in.is_some() {
                                    if fp >= 2 && fp < out.len() && out[fp - 2] == 0x8b {
                                        out[fp - 2] = 0x8d;
                                    }
                                    w32(&mut out, fp, (s as i64 + a - p as i64) as u32);
                                    continue;
                                }
                            }
                            w32(&mut out, fp, (s as i64 + a - p as i64) as u32);
                        } else {
                            w32(&mut out, fp, (s as i64 + a - p as i64) as u32);
                        }
                    }
                    R_X86_64_PC64 => {
                        w64(&mut out, fp, (s as i64 + a - p as i64) as u64);
                    }
                    R_X86_64_GOTTPOFF => {
                        // TLS Initial-Exec: point the instruction at the GOT entry.
                        // For locally-defined TLS symbols, the GOT entry was already
                        // filled with the static TPOFF value above. For external TLS
                        // symbols, the dynamic linker fills the GOT slot at load time
                        // via R_X86_64_TPOFF64.
                        if let Some(&gea) = got_sym_addrs.get(sym.name.as_str()) {
                            // Only fill the GOT entry statically for locally-defined symbols
                            let is_local_tls = if let Some(g) = globals_snap.get(sym.name.as_str())
                            {
                                g.defined_in.is_some()
                                    && !g.is_dynamic
                                    && g.section_idx != SHN_UNDEF
                            } else {
                                false
                            };
                            if is_local_tls {
                                let tpoff = (s as i64 - tls_addr as i64) - tls_mem_size as i64;
                                w64(
                                    &mut out,
                                    (got_offset + (gea - got_addr)) as usize,
                                    tpoff as u64,
                                );
                            }
                            // Patch the instruction to reference the GOT entry
                            w32(&mut out, fp, (gea as i64 + a - p as i64) as u32);
                        } else {
                            // No GOT entry: IE-to-LE relaxation for locally-resolved symbols.
                            // Handles mov (8b) and add (03); transplants REX.R
                            // into REX.B since the register moves from ModRM.reg
                            // to ModRM.rm (see emit_exec.rs for details).
                            let tpoff = (s as i64 - tls_addr as i64) - tls_mem_size as i64;
                            let opc = if fp >= 2 { out[fp - 2] } else { 0 };
                            if fp >= 3 && fp + 4 <= out.len() && (opc == 0x8b || opc == 0x03) {
                                let modrm = out[fp - 1];
                                let reg = (modrm >> 3) & 7;
                                out[fp - 2] = if opc == 0x8b { 0xc7 } else { 0x81 };
                                out[fp - 1] = 0xc0 | reg;
                                let rex = out[fp - 3];
                                if (rex & 0xf0) == 0x40 {
                                    out[fp - 3] = (rex & 0b1111_1010) | ((rex >> 2) & 1);
                                }
                                w32(&mut out, fp, (tpoff + a) as u32);
                            }
                        }
                    }
                    R_X86_64_TPOFF32 => {
                        let tpoff = (s as i64 - tls_addr as i64) - tls_mem_size as i64;
                        w32(&mut out, fp, (tpoff + a) as u32);
                    }
                    R_X86_64_TLSGD => {
                        // Point the lea at the (DTPMOD64, DTPOFF64) GOT pair.
                        if let Some(&slot) = tlsgd_slot_addr.get(sym.name.as_str()) {
                            w32(&mut out, fp, (slot as i64 + a - p as i64) as u32);
                        } else {
                            eprintln!("warning: TLSGD without GOT pair for '{}'", sym.name);
                        }
                    }
                    R_X86_64_TLSLD => {
                        if let Some(slot) = tlsld_slot {
                            w32(&mut out, fp, (slot as i64 + a - p as i64) as u32);
                        }
                    }
                    R_X86_64_DTPOFF32 => {
                        // Offset of the symbol within this module's TLS block.
                        let dtpoff = s as i64 - tls_addr as i64;
                        w32(&mut out, fp, (dtpoff + a) as u32);
                    }
                    R_X86_64_DTPOFF64 => {
                        let dtpoff = s as i64 - tls_addr as i64;
                        w64(&mut out, fp, (dtpoff + a) as u64);
                    }
                    R_X86_64_NONE => {}
                    other => {
                        eprintln!(
                            "warning: unsupported relocation type {} for '{}' in shared library",
                            other, sym.name
                        );
                    }
                }
            }
        }
    }

    // Write .rela.dyn entries
    let relative_count = rela_dyn_entries.len();
    let total_rela_count = relative_count
        + glob_dat_entries.len()
        + tpoff64_entries.len()
        + abs64_entries.len()
        + dtpmod64_entries.len()
        + dtpoff64_entries.len();
    let rela_dyn_size = total_rela_count as u64 * 24;
    let mut rd = rela_dyn_offset as usize;
    // First: R_X86_64_RELATIVE entries (type 8, no symbol)
    for (rel_offset, rel_value) in &rela_dyn_entries {
        if rd + 24 <= out.len() {
            w64(&mut out, rd, *rel_offset); // r_offset
            w64(&mut out, rd + 8, R_X86_64_RELATIVE as u64); // r_info (sym 0)
            w64(&mut out, rd + 16, *rel_value); // r_addend = runtime value
            rd += 24;
        }
    }
    // Then: R_X86_64_GLOB_DAT entries (type 6, with symbol index)
    for (rel_offset, sym_name) in &glob_dat_entries {
        let si = dyn_sym_index.get(sym_name.as_str()).copied().unwrap_or(0);
        if rd + 24 <= out.len() {
            w64(&mut out, rd, *rel_offset); // r_offset = GOT entry address
            w64(&mut out, rd + 8, (si << 32) | R_X86_64_GLOB_DAT as u64);
            w64(&mut out, rd + 16, 0); // r_addend = 0
            rd += 24;
        }
    }
    // Then: R_X86_64_TPOFF64 entries (type 18, with symbol index) for TLS GOT entries.
    // The dynamic linker fills these GOT slots with the thread-pointer offset of the
    // TLS symbol, so that `%fs:0 + GOT[n]` gives the correct address.
    for (rel_offset, sym_name) in &tpoff64_entries {
        let si = dyn_sym_index.get(sym_name.as_str()).copied().unwrap_or(0);
        if rd + 24 <= out.len() {
            w64(&mut out, rd, *rel_offset); // r_offset = GOT entry address
            w64(&mut out, rd + 8, (si << 32) | R_X86_64_TPOFF64 as u64);
            w64(&mut out, rd + 16, 0); // r_addend = 0
            rd += 24;
        }
    }
    // TLS module/offset relocations for General/Local-Dynamic GOT pairs.
    // DTPMOD64 with sym 0 = "this module"; ld.so writes the module id.
    for (rel_offset, sym_name) in &dtpmod64_entries {
        let si = if sym_name.is_empty() {
            0
        } else {
            dyn_sym_index.get(sym_name.as_str()).copied().unwrap_or(0)
        };
        if rd + 24 <= out.len() {
            w64(&mut out, rd, *rel_offset);
            w64(&mut out, rd + 8, (si << 32) | R_X86_64_DTPMOD64 as u64);
            w64(&mut out, rd + 16, 0);
            rd += 24;
        }
    }
    for (rel_offset, sym_name) in &dtpoff64_entries {
        let si = dyn_sym_index.get(sym_name.as_str()).copied().unwrap_or(0);
        if rd + 24 <= out.len() {
            w64(&mut out, rd, *rel_offset);
            w64(&mut out, rd + 8, (si << 32) | R_X86_64_DTPOFF64 as u64);
            w64(&mut out, rd + 16, 0);
            rd += 24;
        }
    }
    // Then: R_X86_64_64 entries (type 1, with symbol index) for named symbol
    // references in data sections (function pointer tables, vtables, etc.)
    for (rel_offset, sym_name, addend) in &abs64_entries {
        let si = dyn_sym_index.get(sym_name.as_str()).copied().unwrap_or(0);
        if rd + 24 <= out.len() {
            w64(&mut out, rd, *rel_offset); // r_offset
            w64(&mut out, rd + 8, (si << 32) | R_X86_64_64 as u64);
            w64(&mut out, rd + 16, *addend as u64); // r_addend
            rd += 24;
        }
    }

    // .dynamic
    let mut dd = dynamic_offset as usize;
    for lib in needed_sonames {
        let so = dynstr.get_offset(lib);
        w64(&mut out, dd, DT_NEEDED as u64);
        w64(&mut out, dd + 8, so as u64);
        dd += 16;
    }
    if let Some(ref sn) = soname {
        let so = dynstr.get_offset(sn);
        w64(&mut out, dd, DT_SONAME as u64);
        w64(&mut out, dd + 8, so as u64);
        dd += 16;
    }
    for &(tag, val) in &[
        (DT_STRTAB, dynstr_addr),
        (DT_SYMTAB, dynsym_addr),
        (DT_STRSZ, dynstr_size),
        (DT_SYMENT, 24),
        (DT_RELA, rela_dyn_addr),
        (DT_RELASZ, rela_dyn_size),
        (DT_RELAENT, 24),
        (DT_RELACOUNT, relative_count as u64),
        (DT_GNU_HASH, gnu_hash_addr),
        // DT_TEXTREL not needed since we use PIC
    ] {
        w64(&mut out, dd, tag as u64);
        w64(&mut out, dd + 8, val);
        dd += 16;
    }
    if bsymbolic {
        // DT_SYMBOLIC (legacy) + DT_FLAGS:DF_SYMBOLIC (modern): search the
        // library itself before the global scope at run time.
        w64(&mut out, dd, 16u64);
        w64(&mut out, dd + 8, 0);
        dd += 16; // DT_SYMBOLIC
        w64(&mut out, dd, DT_FLAGS as u64);
        w64(&mut out, dd + 8, 0x2);
        dd += 16; // DF_SYMBOLIC
    }
    if verdef_count > 0 {
        w64(&mut out, dd, DT_VERSYM as u64);
        w64(&mut out, dd + 8, versym_addr);
        dd += 16;
        w64(&mut out, dd, DT_VERDEF as u64);
        w64(&mut out, dd + 8, verdef_addr);
        dd += 16;
        w64(&mut out, dd, DT_VERDEFNUM as u64);
        w64(&mut out, dd + 8, verdef_count);
        dd += 16;
    }
    if has_init_array {
        w64(&mut out, dd, DT_INIT_ARRAY as u64);
        w64(&mut out, dd + 8, init_array_addr);
        dd += 16;
        w64(&mut out, dd, DT_INIT_ARRAYSZ as u64);
        w64(&mut out, dd + 8, init_array_size);
        dd += 16;
    }
    if has_fini_array {
        w64(&mut out, dd, DT_FINI_ARRAY as u64);
        w64(&mut out, dd + 8, fini_array_addr);
        dd += 16;
        w64(&mut out, dd, DT_FINI_ARRAYSZ as u64);
        w64(&mut out, dd + 8, fini_array_size);
        dd += 16;
    }
    if !plt_names.is_empty() {
        w64(&mut out, dd, DT_PLTGOT as u64);
        w64(&mut out, dd + 8, got_plt_addr);
        dd += 16;
        w64(&mut out, dd, DT_PLTRELSZ as u64);
        w64(&mut out, dd + 8, rela_plt_size);
        dd += 16;
        w64(&mut out, dd, DT_PLTREL as u64);
        w64(&mut out, dd + 8, DT_RELA as u64);
        dd += 16;
        w64(&mut out, dd, DT_JMPREL as u64);
        w64(&mut out, dd + 8, rela_plt_addr);
        dd += 16;
    }
    if let Some(ref rp) = rpath_string {
        let rp_off = dynstr.get_offset(rp) as u64;
        let tag = if use_runpath { DT_RUNPATH } else { DT_RPATH };
        w64(&mut out, dd, tag as u64);
        w64(&mut out, dd + 8, rp_off);
        dd += 16;
    }
    w64(&mut out, dd, DT_NULL as u64);
    w64(&mut out, dd + 8, 0);

    // === Append section headers ===
    // Build .shstrtab string table
    let mut shstrtab = vec![0u8]; // null byte at offset 0
    let mut shstr_offsets: FxHashMap<String, u32> = FxHashMap::default();
    let known_names = [
        ".gnu.hash",
        ".dynsym",
        ".dynstr",
        ".gnu.version",
        ".gnu.version_d",
        ".rela.dyn",
        ".rela.plt",
        ".plt",
        ".dynamic",
        ".got",
        ".got.plt",
        ".init_array",
        ".fini_array",
        ".tdata",
        ".tbss",
        ".bss",
        ".symtab",
        ".strtab",
        ".shstrtab",
    ];
    for name in &known_names {
        let off = shstrtab.len() as u32;
        shstr_offsets.insert(name.to_string(), off);
        shstrtab.extend_from_slice(name.as_bytes());
        shstrtab.push(0);
    }
    // Add merged section names not already in known list
    for sec in output_sections.iter() {
        if !sec.name.is_empty() && !shstr_offsets.contains_key(&sec.name) {
            let off = shstrtab.len() as u32;
            shstr_offsets.insert(sec.name.clone(), off);
            shstrtab.extend_from_slice(sec.name.as_bytes());
            shstrtab.push(0);
        }
    }

    let get_shname = |n: &str| -> u32 { shstr_offsets.get(n).copied().unwrap_or(0) };

    // Helper: write a 64-byte ELF64 section header
    // Use shared write_elf64_shdr from linker_common (aliased locally for brevity)
    let write_shdr_so = linker_common::write_elf64_shdr;

    // Pre-count section indices for cross-references
    let dynsym_shidx: u32 = 2; // NULL=0, .gnu.hash=1, .dynsym=2
    let dynstr_shidx: u32 = 3; // .dynstr=3

    // Map merged output sections to their final section-header indices.
    let mut out_sec_to_hdr: FxHashMap<usize, u16> = FxHashMap::default();
    let mut next_hdr = 4usize;
    if versym_size > 0 {
        next_hdr += 1;
    }
    if verdef_size > 0 {
        next_hdr += 1;
    }
    if rela_dyn_size > 0 {
        next_hdr += 1;
    }
    if rela_plt_size > 0 {
        next_hdr += 1;
    }
    if plt_size > 0 {
        next_hdr += 1;
    }
    for (i, sec) in output_sections.iter().enumerate() {
        if sec.flags & SHF_ALLOC != 0
            && sec.sh_type != SHT_NOBITS
            && sec.flags & SHF_TLS == 0
            && sec.name != ".init_array"
            && sec.name != ".fini_array"
        {
            out_sec_to_hdr.insert(i, next_hdr as u16);
            next_hdr += 1;
        }
    }
    for (i, sec) in output_sections.iter().enumerate() {
        if sec.flags & SHF_TLS != 0 && sec.flags & SHF_ALLOC != 0 && sec.sh_type != SHT_NOBITS {
            out_sec_to_hdr.insert(i, next_hdr as u16);
            next_hdr += 1;
        }
    }
    for (i, sec) in output_sections.iter().enumerate() {
        if sec.flags & SHF_TLS != 0 && sec.sh_type == SHT_NOBITS {
            out_sec_to_hdr.insert(i, next_hdr as u16);
            next_hdr += 1;
        }
    }
    if has_init_array {
        next_hdr += 1;
    }
    if has_fini_array {
        next_hdr += 1;
    }
    next_hdr += 1; // .dynamic
    if got_plt_size > 0 {
        next_hdr += 1;
    }
    if got_size > 0 {
        next_hdr += 1;
    }
    for (i, sec) in output_sections.iter().enumerate() {
        if sec.sh_type == SHT_NOBITS && sec.flags & SHF_ALLOC != 0 && sec.flags & SHF_TLS == 0 {
            out_sec_to_hdr.insert(i, next_hdr as u16);
            next_hdr += 1;
        }
    }
    let symtab_shidx = next_hdr as u16;
    let strtab_shidx = symtab_shidx + 1;

    // Full static symbol table for GDB, perf, and Callgrind. ELF requires local
    // entries before globals and sh_info to name the first global index.
    let mut symtab_entries: Vec<[u8; 24]> = vec![[0u8; 24]];
    let mut symtab_names: Vec<u8> = vec![0];
    let mut locals: Vec<(usize, &Symbol)> = objects
        .iter()
        .enumerate()
        .flat_map(|(obj_idx, obj)| {
            obj.symbols.iter().filter_map(move |sym| {
                if sym.is_local()
                    && !sym.name.is_empty()
                    && sym.shndx != SHN_UNDEF
                    && sym.shndx != SHN_ABS
                    && section_map.contains_key(&(obj_idx, sym.shndx as usize))
                {
                    Some((obj_idx, sym))
                } else {
                    None
                }
            })
        })
        .collect();
    locals.sort_by(|(oa, a), (ob, b)| {
        oa.cmp(ob)
            .then_with(|| a.value.cmp(&b.value))
            .then_with(|| a.name.cmp(&b.name))
    });
    for (obj_idx, sym) in locals {
        let (oi, sec_off) = section_map[&(obj_idx, sym.shndx as usize)];
        let Some(&shndx) = out_sec_to_hdr.get(&oi) else {
            continue;
        };
        let name_off = push_strtab_name(&mut symtab_names, sym.name.as_bytes());
        let mut entry = [0u8; 24];
        entry[0..4].copy_from_slice(&name_off.to_le_bytes());
        entry[4] = sym.info;
        entry[5] = sym.other;
        entry[6..8].copy_from_slice(&shndx.to_le_bytes());
        let value = output_sections[oi].addr + sec_off + sym.value;
        entry[8..16].copy_from_slice(&value.to_le_bytes());
        entry[16..24].copy_from_slice(&sym.size.to_le_bytes());
        symtab_entries.push(entry);
    }
    let first_global = symtab_entries.len() as u32;
    let mut global_names: Vec<(&String, &GlobalSymbol)> = globals
        .iter()
        .filter(|(_, sym)| sym.defined_in.is_some() && !sym.is_dynamic)
        .collect();
    global_names.sort_by(|a, b| a.0.cmp(b.0));
    for (name, sym) in global_names {
        let name_off = push_strtab_name(&mut symtab_names, name.as_bytes());
        let shndx = if sym.section_idx == SHN_ABS {
            SHN_ABS
        } else if sym.section_idx == SHN_COMMON {
            SHN_COMMON
        } else if let Some(obj_idx) = sym.defined_in {
            section_map
                .get(&(obj_idx, sym.section_idx as usize))
                .and_then(|(oi, _)| out_sec_to_hdr.get(oi))
                .copied()
                .unwrap_or(SHN_ABS)
        } else {
            SHN_ABS
        };
        let mut entry = [0u8; 24];
        entry[0..4].copy_from_slice(&name_off.to_le_bytes());
        entry[4] = sym.info;
        entry[6..8].copy_from_slice(&shndx.to_le_bytes());
        entry[8..16].copy_from_slice(&sym.value.to_le_bytes());
        entry[16..24].copy_from_slice(&sym.size.to_le_bytes());
        symtab_entries.push(entry);
    }

    // Count total sections to determine .shstrtab index
    let mut sh_count: u16 = 4; // NULL + .gnu.hash + .dynsym + .dynstr
    if versym_size > 0 {
        sh_count += 1;
    }
    if verdef_size > 0 {
        sh_count += 1;
    }
    if rela_dyn_size > 0 {
        sh_count += 1;
    }
    if rela_plt_size > 0 {
        sh_count += 1;
    }
    if plt_size > 0 {
        sh_count += 1;
    }
    // Merged output sections (non-BSS, non-TLS, non-init/fini)
    for sec in output_sections.iter() {
        if sec.flags & SHF_ALLOC != 0
            && sec.sh_type != SHT_NOBITS
            && sec.flags & SHF_TLS == 0
            && sec.name != ".init_array"
            && sec.name != ".fini_array"
        {
            sh_count += 1;
        }
    }
    // TLS data + TLS BSS
    for sec in output_sections.iter() {
        if sec.flags & SHF_TLS != 0 && sec.flags & SHF_ALLOC != 0 && sec.sh_type != SHT_NOBITS {
            sh_count += 1;
        }
    }
    for sec in output_sections.iter() {
        if sec.flags & SHF_TLS != 0 && sec.sh_type == SHT_NOBITS {
            sh_count += 1;
        }
    }
    if has_init_array {
        sh_count += 1;
    }
    if has_fini_array {
        sh_count += 1;
    }
    sh_count += 1; // .dynamic
    if got_plt_size > 0 {
        sh_count += 1;
    }
    if got_size > 0 {
        sh_count += 1;
    }
    // BSS sections (non-TLS)
    for sec in output_sections.iter() {
        if sec.sh_type == SHT_NOBITS && sec.flags & SHF_ALLOC != 0 && sec.flags & SHF_TLS == 0 {
            sh_count += 1;
        }
    }
    sh_count += 2; // .symtab + .strtab
    debug_assert_eq!(symtab_shidx, sh_count - 2);
    debug_assert_eq!(strtab_shidx, sh_count - 1);
    let shstrtab_shidx = sh_count; // .shstrtab is the last section
    sh_count += 1;

    while out.len() % 8 != 0 {
        out.push(0);
    }
    let symtab_data_offset = out.len() as u64;
    for entry in &symtab_entries {
        out.extend_from_slice(entry);
    }
    let symtab_data_size = (symtab_entries.len() * 24) as u64;
    let strtab_data_offset = out.len() as u64;
    out.extend_from_slice(&symtab_names);

    while out.len() % 8 != 0 {
        out.push(0);
    }
    let shstrtab_data_offset = out.len() as u64;
    out.extend_from_slice(&shstrtab);

    // Align section header table to 8 bytes
    while out.len() % 8 != 0 {
        out.push(0);
    }
    let shdr_offset = out.len() as u64;

    // Write section headers
    // [0] NULL
    write_shdr_so(&mut out, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0);
    // .gnu.hash
    write_shdr_so(
        &mut out,
        get_shname(".gnu.hash"),
        SHT_GNU_HASH,
        SHF_ALLOC,
        gnu_hash_addr,
        gnu_hash_offset,
        gnu_hash_size,
        dynsym_shidx,
        0,
        8,
        0,
    );
    // .dynsym
    write_shdr_so(
        &mut out,
        get_shname(".dynsym"),
        SHT_DYNSYM,
        SHF_ALLOC,
        dynsym_addr,
        dynsym_offset,
        dynsym_size,
        dynstr_shidx,
        1,
        8,
        24,
    );
    // .dynstr
    write_shdr_so(
        &mut out,
        get_shname(".dynstr"),
        SHT_STRTAB,
        SHF_ALLOC,
        dynstr_addr,
        dynstr_offset,
        dynstr_size,
        0,
        0,
        1,
        0,
    );
    if versym_size > 0 {
        write_shdr_so(
            &mut out,
            get_shname(".gnu.version"),
            SHT_GNU_VERSYM,
            SHF_ALLOC,
            versym_addr,
            versym_offset,
            versym_size,
            dynsym_shidx,
            0,
            2,
            2,
        );
    }
    if verdef_size > 0 {
        write_shdr_so(
            &mut out,
            get_shname(".gnu.version_d"),
            SHT_GNU_VERDEF,
            SHF_ALLOC,
            verdef_addr,
            verdef_offset,
            verdef_size,
            dynstr_shidx,
            verdef_count as u32,
            8,
            0,
        );
    }
    // .rela.dyn
    if rela_dyn_size > 0 {
        write_shdr_so(
            &mut out,
            get_shname(".rela.dyn"),
            SHT_RELA,
            SHF_ALLOC,
            rela_dyn_addr,
            rela_dyn_offset,
            rela_dyn_size,
            dynsym_shidx,
            0,
            8,
            24,
        );
    }
    // .rela.plt
    if rela_plt_size > 0 {
        write_shdr_so(
            &mut out,
            get_shname(".rela.plt"),
            SHT_RELA,
            SHF_ALLOC | 0x40,
            rela_plt_addr,
            rela_plt_offset,
            rela_plt_size,
            dynsym_shidx,
            0,
            8,
            24,
        );
    }
    // .plt
    if plt_size > 0 {
        write_shdr_so(
            &mut out,
            get_shname(".plt"),
            SHT_PROGBITS,
            SHF_ALLOC | SHF_EXECINSTR,
            plt_addr,
            plt_offset,
            plt_size,
            0,
            0,
            16,
            16,
        );
    }
    // Merged output sections (text/rodata/data, excluding BSS/TLS/init_array/fini_array)
    for sec in output_sections.iter() {
        if sec.flags & SHF_ALLOC != 0
            && sec.sh_type != SHT_NOBITS
            && sec.flags & SHF_TLS == 0
            && sec.name != ".init_array"
            && sec.name != ".fini_array"
        {
            write_shdr_so(
                &mut out,
                get_shname(&sec.name),
                sec.sh_type,
                sec.flags,
                sec.addr,
                sec.file_offset,
                sec.mem_size,
                0,
                0,
                sec.alignment.max(1),
                0,
            );
        }
    }
    // TLS data sections (.tdata)
    for sec in output_sections.iter() {
        if sec.flags & SHF_TLS != 0 && sec.flags & SHF_ALLOC != 0 && sec.sh_type != SHT_NOBITS {
            write_shdr_so(
                &mut out,
                get_shname(&sec.name),
                sec.sh_type,
                sec.flags,
                sec.addr,
                sec.file_offset,
                sec.mem_size,
                0,
                0,
                sec.alignment.max(1),
                0,
            );
        }
    }
    // TLS BSS sections (.tbss)
    for sec in output_sections.iter() {
        if sec.flags & SHF_TLS != 0 && sec.sh_type == SHT_NOBITS {
            write_shdr_so(
                &mut out,
                get_shname(&sec.name),
                SHT_NOBITS,
                sec.flags,
                sec.addr,
                sec.file_offset,
                sec.mem_size,
                0,
                0,
                sec.alignment.max(1),
                0,
            );
        }
    }
    // .init_array
    if has_init_array {
        if let Some(ia_sec) = output_sections.iter().find(|s| s.name == ".init_array") {
            write_shdr_so(
                &mut out,
                get_shname(".init_array"),
                SHT_INIT_ARRAY,
                SHF_ALLOC | SHF_WRITE,
                init_array_addr,
                ia_sec.file_offset,
                init_array_size,
                0,
                0,
                8,
                8,
            );
        }
    }
    // .fini_array
    if has_fini_array {
        if let Some(fa_sec) = output_sections.iter().find(|s| s.name == ".fini_array") {
            write_shdr_so(
                &mut out,
                get_shname(".fini_array"),
                SHT_FINI_ARRAY,
                SHF_ALLOC | SHF_WRITE,
                fini_array_addr,
                fa_sec.file_offset,
                fini_array_size,
                0,
                0,
                8,
                8,
            );
        }
    }
    // .dynamic
    write_shdr_so(
        &mut out,
        get_shname(".dynamic"),
        SHT_DYNAMIC,
        SHF_ALLOC | SHF_WRITE,
        dynamic_addr,
        dynamic_offset,
        dynamic_size,
        dynstr_shidx,
        0,
        8,
        16,
    );
    // .got.plt
    if got_plt_size > 0 {
        write_shdr_so(
            &mut out,
            get_shname(".got.plt"),
            SHT_PROGBITS,
            SHF_ALLOC | SHF_WRITE,
            got_plt_addr,
            got_plt_offset,
            got_plt_size,
            0,
            0,
            8,
            8,
        );
    }
    // .got
    if got_size > 0 {
        write_shdr_so(
            &mut out,
            get_shname(".got"),
            SHT_PROGBITS,
            SHF_ALLOC | SHF_WRITE,
            got_addr,
            got_offset,
            got_size,
            0,
            0,
            8,
            8,
        );
    }
    // BSS sections (non-TLS)
    for sec in output_sections.iter() {
        if sec.sh_type == SHT_NOBITS && sec.flags & SHF_ALLOC != 0 && sec.flags & SHF_TLS == 0 {
            write_shdr_so(
                &mut out,
                get_shname(&sec.name),
                SHT_NOBITS,
                sec.flags,
                sec.addr,
                sec.file_offset,
                sec.mem_size,
                0,
                0,
                sec.alignment.max(1),
                0,
            );
        }
    }
    // Non-allocated static symbols used by profilers and debuggers.
    write_shdr_so(
        &mut out,
        get_shname(".symtab"),
        SHT_SYMTAB,
        0,
        0,
        symtab_data_offset,
        symtab_data_size,
        strtab_shidx as u32,
        first_global,
        8,
        24,
    );
    write_shdr_so(
        &mut out,
        get_shname(".strtab"),
        SHT_STRTAB,
        0,
        0,
        strtab_data_offset,
        symtab_names.len() as u64,
        0,
        0,
        1,
        0,
    );
    // .shstrtab (last section)
    write_shdr_so(
        &mut out,
        get_shname(".shstrtab"),
        SHT_STRTAB,
        0,
        0,
        shstrtab_data_offset,
        shstrtab.len() as u64,
        0,
        0,
        1,
        0,
    );

    // Patch ELF header with section header info
    out[40..48].copy_from_slice(&shdr_offset.to_le_bytes()); // e_shoff
    out[58..60].copy_from_slice(&64u16.to_le_bytes()); // e_shentsize
    out[60..62].copy_from_slice(&sh_count.to_le_bytes()); // e_shnum
    out[62..64].copy_from_slice(&shstrtab_shidx.to_le_bytes()); // e_shstrndx

    // === -Map=FILE ===
    // Written after layout so every address is final, and built from the same
    // `output_sections` / `section_map` state the ELF was emitted from -- the
    // map is authoritative, not a reconstruction that can drift.
    if let Some(mp) = map_path {
        let object_names: Vec<String> = objects.iter().map(|o| o.source_name.clone()).collect();
        let mut map_syms: Vec<(String, usize, usize, u64)> = Vec::new();
        for (obj_idx, obj) in objects.iter().enumerate() {
            for sym in &obj.symbols {
                if sym.name.is_empty() {
                    continue;
                }
                let st = sym.sym_type();
                if st == STT_SECTION || st == 4 {
                    continue;
                }
                if sym.is_undefined() {
                    continue;
                }
                let si = sym.shndx as usize;
                if section_map.contains_key(&(obj_idx, si)) {
                    map_syms.push((sym.name.to_string(), obj_idx, si, sym.value));
                }
            }
        }
        // A shared library has no entry point, so pass none rather than
        // inventing `_start`, which would put a bogus line in the map.
        let lm = linker_common::build_link_map(output_sections, &object_names, &map_syms, None, 0);
        lm.write_to_path(std::path::Path::new(mp))
            .map_err(|e| format!("failed to write map file '{}': {}", mp, e))?;
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
