//! Executable emission for the x86-64 linker.
//!
//! Emits either a statically-linked or dynamically-linked ELF64 executable,
//! depending on whether dynamic symbols are present. Handles PLT/GOT,
//! `.dynamic` section, TLS, IFUNC/IRELATIVE, and copy relocations.

use crate::common::fx_hash::FxHashMap;
use std::collections::BTreeSet;

use super::elf::*;
use super::reloc_field::{self, w8_checked, w16_checked, w32_checked};
use super::types::{BASE_ADDR, GlobalSymbol, PAGE_SIZE};
use crate::backend::elf::{elf64_sym_entry, push_strtab_name};
use crate::backend::linker_common::{self, DynStrTab, OutputSection};

/// The dynsym NAME for a dynamic symbol reference. Versioned references are
/// keyed in the global table by their full object-file name ("memcpy@GLIBC_2.2.5"),
/// but the emitted .dynstr name must be the BARE symbol name: the dynamic
/// linker hashes the raw dynstr name against the library's .gnu.hash, and
/// libc's hash contains "memcpy", not "memcpy@GLIBC_2.2.5". The requested
/// version is conveyed by the .gnu.version (versym) index into .gnu.version_r
/// (verneed), exactly as GNU ld represents versioned references.
fn dynsym_emit_name(name: &str) -> &str {
    match name.find('@') {
        Some(at) => &name[..at],
        None => name,
    }
}

pub(super) fn emit_executable(
    objects: &[ElfObject],
    interp: &[u8],
    globals: &mut FxHashMap<String, GlobalSymbol>,
    output_sections: &mut [OutputSection],
    section_map: &FxHashMap<(usize, usize), (usize, u64)>,
    // Sections dropped by `--gc-sections`, COMDAT dedup, ICF folding or dead-FDE
    // pruning, keyed `(object, section)`.  They are still present in
    // `section_map` -- the layout pass assigns them a slot before collection
    // decides they are unreachable -- so a symbol filter that only consults
    // `section_map` emits a `.symtab` full of entries pointing at address 0.
    dead_sections: &crate::common::fx_hash::FxHashSet<(usize, usize)>,
    // ICF-folded sections (a subset of `dead_sections`, keyed the same way):
    // unlike GC/COMDAT losers these still EXIST — `section_map` remaps them
    // onto the surviving representative — so their symbols must be emitted
    // as aliases at the survivor's address, not dropped (lld keeps
    // `dup_two` at `dup_one`'s address under `--icf=all`; dropping it also
    // contradicts the aliasing `link.rs` installs just above).  Only the two
    // `.symtab` filters below consult this; the merge must still skip
    // folded sections — eliding the bytes is the point of folding.
    folded_sections: &FxHashMap<(usize, usize), (usize, usize)>,
    // `-s` / `--strip-all`: omit `.symtab` and `.strtab` entirely.
    plt_names: &[String],
    got_entries: &[(String, bool)],
    // Absolute 64-bit relocations against dynamic data symbols; each becomes a
    // dynamic R_X86_64_64 in .rela.dyn (see AbsDynReloc).
    abs_dyn_relocs: &[super::plt_got::AbsDynReloc],
    // Absolute 64-bit relocations that a PIE hands to the loader as
    // R_X86_64_RELATIVE so it can slide them by the load base.  Collected by
    // the same relocation walk that produces `abs_dyn_relocs`; see
    // `plt_got::PieRelative` for why the count and the bytes cannot drift.
    pie_relative: &[super::plt_got::PieRelative],
    needed_sonames: &[String],
    output_path: &str,
    export_dynamic: bool,
    rpath_entries: &[String],
    use_runpath: bool,
    is_static: bool,
    ifunc_symbols: &[String],
    entry_symbol: Option<&str>,
    z_now: bool,
    z_relro: bool,
    // `-Map=FILE`: write a GNU-ld-compatible link map after layout.
    map_path: Option<&str>,
    // `--version-script=FILE`: restrict which symbols --export-dynamic puts
    // into .dynsym. Previously honoured only for shared objects, so a
    // `local: *;` script silently exported everything from an executable.
    version_script_path: Option<&str>,
    // `-pie`: emit `ET_DYN` based at 0 so the kernel may map the image
    // anywhere, and describe every internal absolute address to `ld.so` with
    // `R_X86_64_RELATIVE`.
    is_pie: bool,
    strip_all: bool,
    // Local IFUNCs as `(object, symbol index)`.  They get IPLT slots numbered
    // after the global ones, so every IPLT/GOT/IRELATIVE table stays a single
    // index space.
    local_ifuncs: &[(usize, usize)],
    hash_style: crate::backend::linker_common::HashStyle,
    // `--defsym` expressions deferred from `apply_defsyms`: their value
    // depends on final addresses, so they are evaluated here, after section
    // addresses and the linker-provided symbols exist.  See
    // `link::evaluate_pending_defsyms` for the full rationale.
    pending_defsyms: &[(String, String, usize)],
) -> Result<(), String> {
    let ld_time = std::env::var("LCCC_LD_TIME").is_ok();
    let mut t_zone = std::time::Instant::now();
    macro_rules! zone {
        ($name:expr_2021) => {
            if ld_time {
                eprintln!(
                    "[ldtime]   emit/{:<18} {:>7.1} ms",
                    $name,
                    t_zone.elapsed().as_secs_f64() * 1e3
                );
                t_zone = std::time::Instant::now();
            }
        };
    }

    let mut dynstr = DynStrTab::new();
    for lib in needed_sonames {
        dynstr.add(lib);
    }
    let rpath_string = if rpath_entries.is_empty() {
        None
    } else {
        let s = rpath_entries.join(":");
        dynstr.add(&s);
        Some(s)
    };

    // Build dyn_sym_names in two parts:
    // 1. Non-hashed symbols (PLT imports, GLOB_DAT imports) - these are undefined
    // 2. Hashed symbols (copy-reloc symbols) - these are defined and must be
    //    findable through .gnu.hash so the dynamic linker can redirect references
    let mut dyn_sym_names: Vec<String> = Vec::new();
    let mut dyn_sym_seen: crate::common::fx_hash::FxHashSet<String> =
        crate::common::fx_hash::FxHashSet::default();
    for name in plt_names {
        if dyn_sym_seen.insert(name.clone()) {
            dyn_sym_names.push(name.clone());
        }
    }
    for (name, is_plt) in got_entries {
        if !name.is_empty() && !*is_plt && !dyn_sym_seen.contains(name) {
            if let Some(gsym) = globals.get(name) {
                if gsym.is_dynamic && !gsym.copy_reloc {
                    dyn_sym_seen.insert(name.clone());
                    dyn_sym_names.push(name.clone());
                }
            }
        }
    }
    // symoffset = index of first hashed symbol (1-indexed: null symbol is index 0)
    let gnu_hash_symoffset = 1 + dyn_sym_names.len(); // +1 for null entry

    // Collect copy relocation symbols - these go AFTER non-hashed symbols
    // and are included in the .gnu.hash table
    let copy_reloc_syms: Vec<(String, u64)> = globals
        .iter()
        .filter(|(_, g)| g.copy_reloc)
        .map(|(n, g)| (n.clone(), g.size))
        .collect();
    for (name, _) in &copy_reloc_syms {
        if dyn_sym_seen.insert(name.clone()) {
            dyn_sym_names.push(name.clone());
        }
    }

    // Symbols targeted by an absolute dynamic relocation must appear in
    // .dynsym: the R_X86_64_64 entry we emit for them carries a symbol index,
    // and index 0 (the null symbol) would make ld.so resolve the slot to the
    // addend alone -- reproducing the very bug this fixes.
    for r in abs_dyn_relocs {
        if dyn_sym_seen.insert(r.name.clone()) {
            dyn_sym_names.push(r.name.clone());
        }
    }

    // When --export-dynamic is used, add all defined global symbols to the
    // dynamic symbol table so shared libraries loaded at runtime (via dlopen)
    // can find symbols from this executable.
    if export_dynamic {
        // A version script narrows the --export-dynamic set exactly as it does
        // for shared objects; `{ global: a; b; local: *; }` is the common
        // spelling used to keep an executable's plugin ABI small.
        let version_script = version_script_path.and_then(linker_common::VersionScript::parse);
        let mut exported: Vec<String> = globals
            .iter()
            .filter(|(name, g)| {
                // Export defined, non-dynamic (local to this executable) global
                // symbols.  The predicate is shared with `--gc-sections`'s root
                // set so the two can never drift apart.
                if !linker_common::is_exported_dynamic_symbol(*g) {
                    return false;
                }
                if let Some(ref vs) = version_script {
                    if vs.any_local_star() && !vs.matches_global(name) {
                        return false;
                    }
                }
                true
            })
            .map(|(n, _)| n.clone())
            .collect();
        exported.sort(); // deterministic output
        for name in exported {
            if dyn_sym_seen.insert(name.clone()) {
                dyn_sym_names.push(name);
            }
        }
    }

    for name in &dyn_sym_names {
        dynstr.add(dynsym_emit_name(name));
    }

    // ── Build .gnu.version (versym) and .gnu.version_r (verneed) data ──
    //
    // Collect version requirements from dynamic symbols, grouped by library.
    let mut lib_versions: FxHashMap<String, BTreeSet<String>> = FxHashMap::default();
    for name in &dyn_sym_names {
        if let Some(gs) = globals.get(name) {
            if gs.is_dynamic {
                if let Some(ref ver) = gs.version {
                    if let Some(ref lib) = gs.from_lib {
                        lib_versions
                            .entry(lib.clone())
                            .or_default()
                            .insert(ver.clone());
                    }
                }
            }
        }
    }

    // Build version index mapping: (library, version_string) -> version index (starting at 2)
    let mut ver_index_map: FxHashMap<(String, String), u16> = FxHashMap::default();
    let mut ver_idx: u16 = 2;
    let mut lib_ver_list: Vec<(String, Vec<String>)> = Vec::new();
    let mut sorted_libs: Vec<String> = lib_versions.keys().cloned().collect();
    sorted_libs.sort();
    for lib in &sorted_libs {
        let vers: Vec<String> = lib_versions[lib].iter().cloned().collect();
        for v in &vers {
            ver_index_map.insert((lib.clone(), v.clone()), ver_idx);
            ver_idx += 1;
            // Add version string to dynstr
            dynstr.add(v);
        }
        lib_ver_list.push((lib.clone(), vers));
    }

    // Build .gnu.version_r (verneed) section
    let mut verneed_data: Vec<u8> = Vec::new();
    let mut verneed_count: u32 = 0;
    // Only include libraries that are in our needed list
    let lib_ver_needed: Vec<(String, Vec<String>)> = lib_ver_list
        .iter()
        .filter(|(lib, _)| needed_sonames.contains(lib))
        .cloned()
        .collect();
    for (lib_i, (lib, vers)) in lib_ver_needed.iter().enumerate() {
        let lib_name_off = dynstr.get_offset(lib);
        let is_last_lib = lib_i == lib_ver_needed.len() - 1;

        // Verneed entry header (16 bytes)
        verneed_data.extend_from_slice(&1u16.to_le_bytes()); // vn_version = 1
        verneed_data.extend_from_slice(&(vers.len() as u16).to_le_bytes()); // vn_cnt
        verneed_data.extend_from_slice(&(lib_name_off as u32).to_le_bytes()); // vn_file
        verneed_data.extend_from_slice(&16u32.to_le_bytes()); // vn_aux (right after header)
        let next_off = if is_last_lib {
            0u32
        } else {
            16 + vers.len() as u32 * 16
        };
        verneed_data.extend_from_slice(&next_off.to_le_bytes()); // vn_next
        verneed_count += 1;

        // Vernaux entries for each version (16 bytes each)
        for (v_i, ver) in vers.iter().enumerate() {
            let ver_name_off = dynstr.get_offset(ver);
            let vidx = ver_index_map[&(lib.clone(), ver.clone())];
            let is_last_ver = v_i == vers.len() - 1;

            let vna_hash = linker_common::sysv_hash(ver.as_bytes());
            verneed_data.extend_from_slice(&vna_hash.to_le_bytes()); // vna_hash
            verneed_data.extend_from_slice(&0u16.to_le_bytes()); // vna_flags
            verneed_data.extend_from_slice(&vidx.to_le_bytes()); // vna_other
            verneed_data.extend_from_slice(&(ver_name_off as u32).to_le_bytes()); // vna_name
            let vna_next: u32 = if is_last_ver { 0 } else { 16 };
            verneed_data.extend_from_slice(&vna_next.to_le_bytes()); // vna_next
        }
    }

    let verneed_size = verneed_data.len() as u64;

    let dynsym_count = 1 + dyn_sym_names.len();
    let dynsym_size = dynsym_count as u64 * 24;
    let dynstr_size = dynstr.as_bytes().len() as u64;
    let rela_plt_size = plt_names.len() as u64 * 24;
    let rela_dyn_glob_count = got_entries
        .iter()
        .filter(|(n, p)| {
            !n.is_empty()
                && !*p
                && globals
                    .get(n)
                    .map(|g| g.is_dynamic && !g.copy_reloc && g.plt_idx.is_none())
                    .unwrap_or(false)
        })
        .count();
    // Dynamic executables carry IFUNC IRELATIVE relocations at the END of
    // .rela.dyn (ld.so applies IRELATIVE last, matching GNU ld/mold layout).
    // Static executables use the separate .rela.iplt + __rela_iplt_start/end
    // protocol handled by glibc's static startup instead.
    // Every IFUNC slot needs an IRELATIVE, local ones included -- counting only
    // the globals here left local IFUNCs with an IPLT stub whose GOT entry was
    // never resolved, so the dynamic case kept binding to the resolver while
    // the static case (which uses `num_ifunc` for `.rela.iplt`) worked.
    let dyn_irelative_count = if is_static {
        0
    } else {
        ifunc_symbols.len() + local_ifuncs.len()
    };
    // Absolute relocations against dynamic data symbols also live in .rela.dyn.
    // Symbols that ended up copy-relocated are excluded: for those the storage
    // is a local BSS copy that ld.so fills via R_X86_64_COPY, so an additional
    // R_X86_64_64 would double-apply.
    let abs_dyn_count = abs_dyn_relocs
        .iter()
        .filter(|r| {
            globals
                .get(r.name.as_str())
                .map(|g| !g.copy_reloc)
                .unwrap_or(false)
        })
        .count();
    // RELATIVE entries come first in .rela.dyn (the order bfd/mold use, and the
    // one that keeps the loader's common case -- a run of RELATIVE at the head
    // of the table -- cache-friendly).  The count is `pie_relative.len()`: the
    // very same list the emitter below walks, so DT_RELASZ is exact by
    // construction rather than by a second predicate happening to agree.
    // Filter once, here, and use the *same* filtered list for the count and for
    // the emission below.  `create_plt_got` walks every section of every input,
    // including ones that never reach the output (non-alloc sections, and
    // anything GC/COMDAT dropped), so a `RELATIVE` is only real if its storage
    // was actually laid out.  Deriving the count from the filtered list rather
    // than from `pie_relative.len()` is what makes `DT_RELASZ` exact: if the
    // filter ever rejects an entry, the count shrinks with it.
    let pie_relative: Vec<&super::plt_got::PieRelative> = if is_static || !is_pie {
        Vec::new()
    } else {
        pie_relative
            .iter()
            .filter(|pr| section_map.contains_key(&(pr.obj_idx, pr.sec_idx)))
            .collect()
    };
    // A PIE also has to slide the GOT slots that hold the address of a symbol
    // *defined in this output*.  crt1.o reaches `main` through
    // R_X86_64_REX_GOTPCRELX, so `_start` loads it out of a GOT slot; in an
    // ET_EXEC that slot is simply pre-filled, but in a PIE it must carry a
    // RELATIVE or the program jumps to the unslid link-time address of main.
    // Collected by ordinal (position among the non-PLT GOT entries, which is
    // how the emitter numbers the slots) so the count needs no addresses.
    let pie_got_relative: Vec<usize> = if is_static || !is_pie {
        Vec::new()
    } else {
        let mut v = Vec::new();
        let mut ord = 0usize;
        for (name, is_plt) in got_entries {
            if name.is_empty() || *is_plt {
                continue;
            }
            // Same rule as the data relocations: the slot needs sliding when
            // what we put in it is one of our own addresses.  That covers both
            // a locally-defined symbol and a dynamic function's PLT entry (the
            // GLOB_DAT writer below deliberately skips those, filling the slot
            // statically instead).
            let needs_slide = globals
                .get(name.as_str())
                .map(super::plt_got::stored_value_is_local)
                .unwrap_or(false);
            if needs_slide {
                v.push(ord);
            }
            ord += 1;
        }
        v
    };
    let pie_relative_count = pie_relative.len() + pie_got_relative.len();
    let rela_dyn_count = rela_dyn_glob_count
        + copy_reloc_syms.len()
        + dyn_irelative_count
        + abs_dyn_count
        + pie_relative_count;
    let rela_dyn_size = rela_dyn_count as u64 * 24;

    // Build .gnu.hash table for hashed symbols (copy-reloc + exported)
    // Number of hashed symbols = total symbols after the non-hashed imports
    let num_hashed = dyn_sym_names.len() - (gnu_hash_symoffset - 1);
    let gnu_hp = linker_common::gnu_hash_params(num_hashed, 64);
    let gnu_hash_nbuckets = gnu_hp.nbuckets;
    let gnu_hash_bloom_size: u32 = gnu_hp.bloom_size;
    let gnu_hash_bloom_shift: u32 = gnu_hp.bloom_shift;

    // Compute hashes for hashed symbols ONCE, over the *emitted* dynsym
    // names (version suffix stripped).  A `@`-suffixed name hashes
    // differently from its emitted form; computing the pre-sort hash from
    // the emitted name but the post-sort bucket/chain hashes from the raw
    // name placed versioned exports into buckets ld.so never consults (it
    // looks up the stripped name — a latent runtime resolution failure for
    // versioned exports).  One vector now feeds the bucket sort, the bloom
    // filter, and the chain table, so a mismatch is impossible by
    // construction (and the duplicate pass is gone).
    let mut hashed_sym_hashes: Vec<u32> = dyn_sym_names[gnu_hash_symoffset - 1..]
        .iter()
        .map(|name| linker_common::gnu_hash(dynsym_emit_name(name).as_bytes()))
        .collect();
    let bloom_words = linker_common::build_gnu_bloom(&hashed_sym_hashes, &gnu_hp, 64);

    // Sort hashed symbols by bucket (hash % nbuckets) for proper chain
    // grouping, via an index permutation so the hash vector tracks the name
    // reordering exactly (no second hash pass, no drift between the two).
    // Stable: same-bucket order keeps input order, so output is
    // deterministic across runs and machines.
    if num_hashed > 0 {
        let hashed_start = gnu_hash_symoffset - 1;
        let mut perm: Vec<usize> = (0..num_hashed).collect();
        perm.sort_by_key(|&i| hashed_sym_hashes[i] % gnu_hash_nbuckets);
        let names_before = dyn_sym_names[hashed_start..].to_vec();
        let hashes_before = hashed_sym_hashes.clone();
        for (new_i, &old_i) in perm.iter().enumerate() {
            dyn_sym_names[hashed_start + new_i] = names_before[old_i].clone();
            hashed_sym_hashes[new_i] = hashes_before[old_i];
        }
    }

    // Build .gnu.version (versym) - one u16 per dynsym entry
    // Must be built AFTER gnu_hash bucket sort so versym indices match final dynsym order
    let mut versym_data: Vec<u8> = Vec::new();
    // Entry 0: VER_NDX_LOCAL for the null symbol
    versym_data.extend_from_slice(&0u16.to_le_bytes());
    for name in &dyn_sym_names {
        if let Some(gs) = globals.get(name) {
            if gs.is_dynamic {
                if let Some(ref ver) = gs.version {
                    if let Some(ref lib) = gs.from_lib {
                        let idx = ver_index_map
                            .get(&(lib.clone(), ver.clone()))
                            .copied()
                            .unwrap_or(1);
                        versym_data.extend_from_slice(&idx.to_le_bytes());
                    } else {
                        versym_data.extend_from_slice(&1u16.to_le_bytes()); // VER_NDX_GLOBAL
                    }
                } else {
                    versym_data.extend_from_slice(&1u16.to_le_bytes()); // VER_NDX_GLOBAL
                }
            } else if gs.section_idx != SHN_UNDEF && gs.value != 0 {
                // Defined/exported symbol: VER_NDX_GLOBAL
                versym_data.extend_from_slice(&1u16.to_le_bytes());
            } else {
                versym_data.extend_from_slice(&0u16.to_le_bytes()); // VER_NDX_LOCAL
            }
        } else {
            versym_data.extend_from_slice(&0u16.to_le_bytes());
        }
    }

    let versym_size = versym_data.len() as u64;

    // O(1) name -> dynsym index (1-based; 0 is the null symbol). Built AFTER
    // the .gnu.hash bucket sort so indices match the final table order.
    let dyn_sym_index: FxHashMap<&str, u64> = dyn_sym_names
        .iter()
        .enumerate()
        .map(|(i, n)| (n.as_str(), (i + 1) as u64))
        .collect();

    // `hashed_sym_hashes` is already in final dynsym order (the index
    // permutation above tracked the bucket sort) — reuse it for the chain
    // table.  This is the single-hash-pass contract.

    // Build buckets and chains
    // SysV `.hash` sizing.  `nbucket` is at the linker's discretion; `nchain`
    // is fixed by the ABI at the number of `.dynsym` entries *including* the
    // NULL symbol at index 0, because `chain[]` is indexed by symbol index.
    let want_gnu_hash = !is_static && hash_style.wants_gnu();
    let want_sysv_hash = !is_static && hash_style.wants_sysv();
    let sysv_hash_size: u64 = if want_sysv_hash {
        let names: Vec<&str> = dyn_sym_names.iter().map(|n| dynsym_emit_name(n)).collect();
        linker_common::build_sysv_hash(&names).size()
    } else {
        0
    };

    // SysV table, built by the helper shared with the shared-object emitter so
    // the two cannot drift apart.
    let sysv_hash: Option<linker_common::SysvHash> = if want_sysv_hash {
        let names: Vec<&str> = dyn_sym_names.iter().map(|n| dynsym_emit_name(n)).collect();
        Some(linker_common::build_sysv_hash(&names))
    } else {
        None
    };
    let mut gnu_hash_buckets = vec![0u32; gnu_hash_nbuckets as usize];
    let mut gnu_hash_chains = vec![0u32; num_hashed];
    for (i, &h) in hashed_sym_hashes.iter().enumerate() {
        let bucket = (h % gnu_hash_nbuckets) as usize;
        if gnu_hash_buckets[bucket] == 0 {
            gnu_hash_buckets[bucket] = (gnu_hash_symoffset + i) as u32;
        }
        // Chain value = hash with bit 0 indicating end of chain
        gnu_hash_chains[i] = h & !1; // clear bit 0 (will set later for last in chain)
    }
    // Mark the last symbol of each bucket chain (bit 0). Symbols are already
    // bucket-sorted, so a single linear pass suffices: entry i ends its chain
    // when the next entry hashes into a different bucket. The previous
    // per-bucket rescan was O(buckets * symbols) - quadratic for large
    // export tables (glibc-sized .so builds).
    for i in 0..hashed_sym_hashes.len() {
        let last = i + 1 == hashed_sym_hashes.len()
            || (hashed_sym_hashes[i + 1] % gnu_hash_nbuckets)
                != (hashed_sym_hashes[i] % gnu_hash_nbuckets);
        if last {
            gnu_hash_chains[i] |= 1;
        }
    }

    // gnu_hash_size = header(16) + bloom(bloom_size*8) + buckets(nbuckets*4) + chains(num_hashed*4)
    let gnu_hash_size: u64 = if is_static {
        0
    } else {
        16 + (gnu_hash_bloom_size as u64 * 8)
            + (gnu_hash_nbuckets as u64 * 4)
            + (num_hashed as u64 * 4)
    };
    // `--hash-style=sysv` means no `.gnu.hash` at all: zero size, no section
    // header, no DT_GNU_HASH.  Leaving an empty table behind would make the
    // loader prefer a hash that contains nothing.
    let gnu_hash_size = if want_gnu_hash { gnu_hash_size } else { 0 };

    let plt_size = if is_static || plt_names.is_empty() {
        0u64
    } else {
        16 + 16 * plt_names.len() as u64
    };
    let got_plt_size = if is_static {
        0u64
    } else {
        (3 + plt_names.len()) as u64 * 8
    };
    let got_globdat_count = got_entries
        .iter()
        .filter(|(n, p)| !n.is_empty() && !*p)
        .count();
    let got_size = got_globdat_count as u64 * 8; // GOT needed even for static (TLS, GOTPCREL)

    let has_init_array = output_sections
        .iter()
        .any(|s| s.name == ".init_array" && s.mem_size > 0);
    let has_preinit_array = output_sections
        .iter()
        .any(|s| s.name == ".preinit_array" && s.mem_size > 0);
    let has_fini_array = output_sections
        .iter()
        .any(|s| s.name == ".fini_array" && s.mem_size > 0);
    // `.init`/`.fini` (from crti/crtn): present with content on every normal
    // dynamic link.  The addresses are NOT final yet (layout runs below), so
    // only the predicates live here; emission looks the addresses up by name.
    let has_init = output_sections
        .iter()
        .any(|s| s.name == ".init" && s.mem_size > 0);
    let has_fini = output_sections
        .iter()
        .any(|s| s.name == ".fini" && s.mem_size > 0);
    // DT_RELACOUNT: the count of leading R_X86_64_RELATIVE entries in
    // .rela.dyn, which ld.so uses to bound the slide pass.  The writer
    // below emits all RELATIVE entries first (pie_relative, then the GOT
    // range), so the count is exact.  GNU emits the tag only when the
    // count is non-zero (a non-PIE executable's .dynamic has no
    // RELACOUNT); match that rather than ARM's unconditional form.
    let relacount = pie_relative_count;
    let dynamic_size = if is_static {
        0u64
    } else {
        // 13 fixed entries + NULL.  DT_GNU_HASH moved out of the fixed set when
        // --hash-style gained a sysv mode, so both hash tags are added here.
        let mut dyn_count = needed_sonames.len() as u64 + 14; // fixed entries + NULL
        dyn_count -= 1; // DT_GNU_HASH is no longer unconditional
        if want_gnu_hash {
            dyn_count += 1;
        }
        if want_sysv_hash {
            dyn_count += 1;
        }
        if has_init_array {
            dyn_count += 2;
        }
        if has_fini_array {
            dyn_count += 2;
        }
        if has_preinit_array {
            dyn_count += 2;
        }
        // Spelled identically to the emission below (see the DT_FLAGS note):
        // DT_INIT/DT_FINI when the sections exist with content, DT_RELACOUNT
        // when the .rela.dyn head run is non-empty.
        if has_init {
            dyn_count += 1;
        }
        if has_fini {
            dyn_count += 1;
        }
        if relacount > 0 {
            dyn_count += 1;
        }
        // DT_FLAGS carries BIND_NOW; DT_FLAGS_1 carries NOW and/or PIE.  A PIE
        // needs DF_1_PIE even without `-z now`, so the two are counted
        // independently -- and both conditions are spelled identically to the
        // emission below, which is the only thing keeping DT_* count and bytes
        // in agreement.
        if z_now {
            dyn_count += 1; // DT_FLAGS
        }
        if z_now || is_pie {
            dyn_count += 1; // DT_FLAGS_1
        }
        if rpath_string.is_some() {
            dyn_count += 1;
        }
        if verneed_size > 0 {
            dyn_count += 3;
        } // DT_VERSYM + DT_VERNEED + DT_VERNEEDNUM
        dyn_count * 16
    };
    // Override other dynamic sizes for static linking
    let dynsym_size = if is_static { 0u64 } else { dynsym_size };
    let dynstr_size = if is_static { 0u64 } else { dynstr_size };
    let rela_plt_size = if is_static { 0u64 } else { rela_plt_size };
    let rela_dyn_size = if is_static { 0u64 } else { rela_dyn_size };
    let versym_size = if is_static { 0u64 } else { versym_size };
    let verneed_size = if is_static { 0u64 } else { verneed_size };

    let has_tls_sections = output_sections
        .iter()
        .any(|s| s.flags & SHF_TLS != 0 && s.flags & SHF_ALLOC != 0);
    // .eh_frame_hdr + PT_GNU_EH_FRAME: required for libgcc/libunwind binary-
    // search unwinding (C++ exceptions, backtrace()) instead of linear scans.
    let eh_frame_fde_count: usize = output_sections
        .iter()
        .filter(|s| s.name == ".eh_frame" && s.mem_size > 0)
        .flat_map(|s| s.inputs.iter())
        .map(|input| {
            linker_common::count_eh_frame_fdes(
                &objects[input.object_idx].section_data[input.section_idx],
            )
        })
        .sum();
    let eh_frame_hdr_size: u64 = if eh_frame_fde_count > 0 {
        (12 + 8 * eh_frame_fde_count) as u64
    } else {
        0
    };
    // Static: PHDR, LOAD(ro), LOAD(text), LOAD(rodata), LOAD(rw), GNU_STACK, [TLS]
    // Dynamic: PHDR, INTERP, LOAD(ro), LOAD(text), LOAD(rodata), LOAD(rw), DYNAMIC, GNU_STACK, [TLS]
    let mut phdr_count: u64 = if is_static {
        if has_tls_sections { 7 } else { 6 }
    } else if has_tls_sections {
        9
    } else {
        8
    };
    if eh_frame_hdr_size > 0 {
        phdr_count += 1;
    }
    // PT_GNU_RELRO: covers the head of the RW segment (init/fini arrays,
    // .data.rel.ro, .dynamic, .got — plus .got.plt under -z now). Static
    // executables get it too: their .got/arrays/.data.rel.ro are
    // link-time-final (startup writes only .data/.bss/ifunc_got, all past
    // the boundary), and static glibc does mprotect the range.
    let has_relro = z_relro;
    if has_relro {
        phdr_count += 1;
    }
    // Split-RW predicate: anything (file bytes OR pure memory) that lands
    // after the RELRO boundary in the layout pass below — lazy .got.plt, the
    // IFUNC GOT (+ static .rela.iplt), writable PROGBITS, TLS, .bss, or
    // copy-reloc slots.  When it exists, the RELRO window gets its own
    // PT_LOAD so the page pad after const-after-relocation data can be
    // NOBITS (virtual address space only) instead of a run of file zeros —
    // the same trick lld and mold play with their synthetic `.relro_padding`
    // section.  Measured: ~1.8 KiB saved on a hello-world PIE (7 872 →
    // 6 064 B), up to one page per binary.  This predicate MUST match the
    // layout pass exactly, or phdr_count and the emitted headers disagree.
    let has_post_relro_content = (got_plt_size > 0 && !(has_relro && z_now))
        || !ifunc_symbols.is_empty()
        || !local_ifuncs.is_empty()
        || !copy_reloc_syms.is_empty()
        || output_sections.iter().any(|s| {
            s.flags & SHF_ALLOC != 0
                && s.flags & SHF_WRITE != 0
                && s.mem_size > 0
                && s.name != ".init_array"
                && s.name != ".fini_array"
                && s.name != ".preinit_array"
                && s.name != ".data.rel.ro"
        });
    let split_relro_load = has_relro && has_post_relro_content;
    if split_relro_load {
        phdr_count += 1; // second RW PT_LOAD (writable tail after RELRO)
    }
    // One PT_NOTE segment per contiguous RUN of allocated note sections
    // (property, build-id, ABI tag), plus a PT_GNU_PROPERTY alias for
    // `.note.gnu.property`.  lld proves a merged note segment is fully
    // compatible (its hello carries exactly one PT_NOTE) and each merged
    // note saves a 56-byte phdr; GNU ld's three PT_NOTE entries exist only
    // because ITS notes land in three different places.  A "run" =
    // order-adjacent allocated note sections: the rodata layout loop places
    // sections in `output_sections` order, so order-adjacent notes are also
    // file-contiguous (section alignment inside a run is fine — a PT_NOTE
    // span may cover padding bytes).  Sections that the flags place into a
    // different segment break the walk conservatively (an extra PT_NOTE is
    // wasted, never a wrong one).  The identical walk runs again at phdr
    // WRITE time over the same order+predicates, so count and emission can
    // never drift.  `mem_size` is the pre-layout size; section `data` is
    // only filled during the layout pass, so it must not be the condition
    // here (it is still empty at this point).
    let is_alloc_note =
        |s: &OutputSection| s.sh_type == SHT_NOTE && s.flags & SHF_ALLOC != 0 && s.mem_size > 0;
    let mut note_phdr_count = 0u64;
    {
        let mut prev_note = false;
        for s in output_sections.iter() {
            if is_alloc_note(s) && !prev_note {
                note_phdr_count += 1;
            }
            prev_note = is_alloc_note(s);
        }
    }
    let has_gnu_property_phdr = output_sections
        .iter()
        .any(|s| s.name == ".note.gnu.property" && s.flags & SHF_ALLOC != 0 && s.mem_size > 0);
    phdr_count += note_phdr_count + has_gnu_property_phdr as u64;
    let phdr_total_size = phdr_count * 56;

    zone!("pre-layout");
    // === Layout ===
    //
    // Segment packing: file offsets stay *dense*, virtual addresses advance in
    // whole pages.
    //
    // A PT_LOAD segment does not need its file offset page-aligned. The ELF
    // gABI only requires `p_offset ≡ p_vaddr (mod p_align)`, because mmap maps
    // `p_offset & ~(pagesize-1)` to `p_vaddr & ~(pagesize-1)`. Rounding the
    // *file offset* up to a page at every segment boundary — which this
    // linker used to do, since `addr` was hardwired to `BASE_ADDR + offset` —
    // inserts up to one page of zero padding per segment.
    //
    // Measured on a zlib-ng test binary: lccc 20 640 bytes vs bfd 16 400 and
    // wild 6 773, with the entire difference being inter-segment padding
    // (7 568 + 3 504 bytes of holes) rather than content. bfd and wild both
    // emit congruent, non-page-aligned offsets (e.g. off=0x2d70/vaddr=0x3d70).
    //
    // `vaddr_bias` is always a multiple of PAGE_SIZE, so congruence
    // `(offset + bias) ≡ offset (mod PAGE_SIZE)` holds by construction and
    // every `p_offset ≡ p_vaddr (mod PAGE_SIZE)` requirement is satisfied
    // automatically. Bumping the bias by one page at a segment boundary gives
    // the new segment a fresh page of address space (so permissions never
    // share a page) while costing zero bytes in the file.
    // The packing invariant itself lives in `layout_plan::SegmentPacker`, which
    // is unit-tested independently. It used to be a pair of local macros here
    // and a second, identical pair in emit_shared.rs — and that duplication is
    // precisely why the shared-library path stayed broken after the executable
    // path was fixed.
    // A PIE is laid out from 0: every address in the image is then an offset
    // from whatever base the kernel picks, and R_X86_64_RELATIVE tells ld.so
    // which stored values need that base added.  A non-PIE keeps the
    // traditional fixed base.
    let base_addr = if is_pie { 0 } else { BASE_ADDR };
    let mut packer = super::layout_plan::SegmentPacker::new(base_addr, PAGE_SIZE);
    macro_rules! vaddr {
        ($off:expr_2021) => {
            packer.vaddr($off)
        };
    }
    macro_rules! new_segment {
        () => {
            packer.new_segment();
        };
    }
    let mut offset = 64 + phdr_total_size;
    let interp_offset = offset;
    let interp_addr = vaddr!(offset);
    if !is_static {
        offset += interp.len() as u64;
    }

    offset = (offset + 7) & !7;
    let gnu_hash_offset = offset;
    let gnu_hash_addr = vaddr!(offset);
    offset += gnu_hash_size;
    // SysV `.hash`, for `--hash-style=sysv|both`.  Placed immediately after
    // `.gnu.hash` so it lands inside the same read-only PT_LOAD (which runs
    // from 0 to the end of `.rela.plt`) with no extra segment bookkeeping.
    offset = (offset + 7) & !7;
    let sysv_hash_offset = offset;
    let sysv_hash_addr = vaddr!(offset);
    offset += sysv_hash_size;
    offset = (offset + 7) & !7;
    let dynsym_offset = offset;
    let dynsym_addr = vaddr!(offset);
    offset += dynsym_size;
    let dynstr_offset = offset;
    let dynstr_addr = vaddr!(offset);
    offset += dynstr_size;
    // .gnu.version (versym) - right after dynstr, aligned to 2
    offset = (offset + 1) & !1;
    let versym_offset = offset;
    let versym_addr = vaddr!(offset);
    if versym_size > 0 {
        offset += versym_size;
    }
    // .gnu.version_r (verneed) - aligned to 4
    offset = (offset + 3) & !3;
    let verneed_offset = offset;
    let verneed_addr = vaddr!(offset);
    if verneed_size > 0 {
        offset += verneed_size;
    }
    offset = (offset + 7) & !7;
    let rela_dyn_offset = offset;
    let rela_dyn_addr = vaddr!(offset);
    offset += rela_dyn_size;
    offset = (offset + 7) & !7;
    let rela_plt_offset = offset;
    let rela_plt_addr = vaddr!(offset);
    offset += rela_plt_size;

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
    let (plt_addr, plt_offset) = if plt_size > 0 {
        offset = (offset + 15) & !15;
        let a = vaddr!(offset);
        let o = offset;
        offset += plt_size;
        (a, o)
    } else {
        (0u64, 0u64)
    };

    // .iplt (IFUNC PLT entries for static linking)
    let num_ifunc = ifunc_symbols.len() + local_ifuncs.len();
    let iplt_entry_size: u64 = 16; // each IPLT entry: jmp *got(%rip) + padding
    let iplt_total_size = num_ifunc as u64 * iplt_entry_size;
    let (iplt_addr, iplt_offset) = if iplt_total_size > 0 {
        offset = (offset + 15) & !15;
        let a = vaddr!(offset);
        let o = offset;
        offset += iplt_total_size;
        (a, o)
    } else {
        (0u64, 0u64)
    };

    let text_total_size = offset - text_page_offset;

    // Rodata segment
    new_segment!();
    let rodata_page_offset = offset;
    let rodata_page_addr = vaddr!(offset);
    // .eh_frame_hdr leads the rodata segment (data filled after relocation).
    let (eh_frame_hdr_offset, eh_frame_hdr_vaddr) = if eh_frame_hdr_size > 0 {
        let o = offset;
        let v = vaddr!(offset);
        offset += eh_frame_hdr_size;
        (o, v)
    } else {
        (0u64, 0u64)
    };
    for sec in output_sections.iter_mut() {
        if sec.flags & SHF_ALLOC != 0
            && sec.flags & SHF_EXECINSTR == 0
            && sec.flags & SHF_WRITE == 0
            && sec.sh_type != SHT_NOBITS
        {
            let a = sec.alignment.max(1);
            offset = (offset + a - 1) & !(a - 1);
            sec.addr = vaddr!(offset);
            sec.file_offset = offset;
            offset += sec.mem_size;
        }
    }
    let rodata_total_size = offset - rodata_page_offset;

    // RW segment
    new_segment!();
    let rw_page_offset = offset;
    let rw_page_addr = vaddr!(offset);

    let mut init_array_addr = 0u64;
    let mut init_array_size = 0u64;
    let mut fini_array_addr = 0u64;
    let mut fini_array_size = 0u64;
    let mut preinit_array_addr = 0u64;
    let mut preinit_array_size = 0u64;

    for sec in output_sections.iter_mut() {
        if sec.name == ".preinit_array" {
            let a = sec.alignment.max(8);
            offset = (offset + a - 1) & !(a - 1);
            sec.addr = vaddr!(offset);
            sec.file_offset = offset;
            preinit_array_addr = sec.addr;
            preinit_array_size = sec.mem_size;
            offset += sec.mem_size;
            break;
        }
    }
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

    // .data.rel.ro joins the RELRO window: it is const-after-relocation by
    // construction, so leaving it in the generic RW loop below kept it
    // writable at runtime for no reason (its entire purpose defeated).
    for sec in output_sections.iter_mut() {
        if sec.name == ".data.rel.ro" {
            let a = sec.alignment.max(1);
            offset = (offset + a - 1) & !(a - 1);
            sec.addr = vaddr!(offset);
            sec.file_offset = offset;
            offset += sec.mem_size;
            break;
        }
    }

    offset = (offset + 7) & !7;
    let dynamic_offset = offset;
    let dynamic_addr = vaddr!(offset);
    offset += dynamic_size;
    offset = (offset + 7) & !7;
    let got_offset = offset;
    let got_addr = vaddr!(offset);
    offset += got_size;
    // Under -z now the .got.plt is never written after startup, so it can be
    // protected too (Full RELRO). Under lazy binding it must stay writable
    // and is placed after the RELRO page boundary below.
    let (mut got_plt_offset, mut got_plt_addr) = (0u64, 0u64);
    if has_relro && z_now {
        offset = (offset + 7) & !7;
        got_plt_offset = offset;
        got_plt_addr = vaddr!(offset);
        offset += got_plt_size;
    }
    // RELRO boundary: everything before this is mprotect(PROT_READ)ed by
    // ld.so after relocation; the boundary must be page-aligned.
    let relro_start = rw_page_offset;
    let relro_start_addr = rw_page_addr;
    let mut relro_size = 0u64;
    // File offset where the RELRO content ends (= file size of the RELRO
    // LOAD, and the start of the writable-tail LOAD when split_relro_load).
    let mut relro_file_end = rw_page_offset;
    // Virtual address assigned to `relro_file_end` after the boundary: the
    // p_vaddr of the writable-tail LOAD.
    let mut rw2_addr = rw_page_addr;
    if has_relro {
        // ld.so mprotects [vaddr, vaddr+memsz) rounded out to page bounds, so
        // it is the *virtual address* that must reach a page boundary, not the
        // file offset. The pad up to that page boundary is NOBITS: it must
        // cost address space (ld.so will mprotect the tail page, so the
        // writable tail must start on a fresh page) but NOT file space —
        // advancing `offset` here used to bake up to one page of zeros into
        // every linked binary.
        relro_file_end = offset;
        let relro_file_end_addr = vaddr!(offset);
        let relro_pad = packer.padding_to_page(offset);
        relro_size = relro_file_end_addr + relro_pad - relro_start_addr;
        if split_relro_load && relro_pad > 0 {
            // The writable tail continues at the same dense file offset but
            // on a fresh page. Congruence is automatic (both file offset and
            // address keep their mod-page residue — verified against lld's
            // own two-RW-LOAD layout). The kernel side-effect mapping of the
            // tail's first file page inside the RELRO page absorbs the
            // mprotect; every real reference goes through the new bias.
            packer.new_segment();
        }
        rw2_addr = vaddr!(offset);
    }
    if !(has_relro && z_now) {
        offset = (offset + 7) & !7;
        got_plt_offset = offset;
        got_plt_addr = vaddr!(offset);
        offset += got_plt_size;
    }

    // IFUNC GOT (8 bytes per entry, stores resolver addresses initially)
    offset = (offset + 7) & !7;
    let ifunc_got_offset = offset;
    let ifunc_got_addr = vaddr!(offset);
    let ifunc_got_size = num_ifunc as u64 * 8;
    offset += ifunc_got_size;

    // .rela.iplt (24 bytes per RELA entry for R_X86_64_IRELATIVE).
    // Only emitted for STATIC executables: dynamic executables put their
    // IRELATIVE entries in .rela.dyn instead (ld.so ignores .rela.iplt).
    offset = (offset + 7) & !7;
    let rela_iplt_offset = offset;
    let rela_iplt_addr = vaddr!(offset);
    let rela_iplt_size = if is_static { num_ifunc as u64 * 24 } else { 0 };
    offset += rela_iplt_size;

    for sec in output_sections.iter_mut() {
        if sec.flags & SHF_ALLOC != 0
            && sec.flags & SHF_WRITE != 0
            && sec.sh_type != SHT_NOBITS
            && sec.name != ".init_array"
            && sec.name != ".fini_array"
            && sec.name != ".preinit_array"
            && sec.name != ".data.rel.ro"
            && sec.flags & SHF_TLS == 0
        {
            let a = sec.alignment.max(1);
            offset = (offset + a - 1) & !(a - 1);
            sec.addr = vaddr!(offset);
            sec.file_offset = offset;
            offset += sec.mem_size;
        }
    }

    // TLS sections (.tdata, .tbss) - place in RW segment, track for PT_TLS
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
    // If only .tbss (NOBITS TLS) exists with no .tdata, we still need a TLS segment.
    // Set tls_addr/tls_file_offset to the current position so TPOFF calculations work.
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
    // Align TLS size to TLS alignment
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

    // Allocate BSS space for copy-relocated symbols.
    // Symbols that are aliases (same from_lib + lib_sym_value) share the same BSS slot.
    let mut copy_reloc_addr_map: FxHashMap<(String, u64), u64> = FxHashMap::default(); // (lib, lib_value) -> bss_addr
    for (name, size) in &copy_reloc_syms {
        let gsym = globals.get(name).cloned();
        let key = gsym.as_ref().and_then(|g| {
            g.from_lib
                .as_ref()
                .map(|lib| (lib.clone(), g.lib_sym_value))
        });
        let addr = if let Some(ref k) = key {
            if let Some(&existing_addr) = copy_reloc_addr_map.get(k) {
                existing_addr // reuse existing BSS slot for alias
            } else {
                let aligned = (bss_addr + bss_size + 7) & !7;
                bss_size = aligned - bss_addr + size;
                copy_reloc_addr_map.insert(k.clone(), aligned);
                aligned
            }
        } else {
            let aligned = (bss_addr + bss_size + 7) & !7;
            bss_size = aligned - bss_addr + size;
            aligned
        };
        if let Some(gsym) = globals.get_mut(name) {
            gsym.value = addr;
            gsym.defined_in = Some(usize::MAX); // sentinel: defined via copy reloc
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

    // Define linker-provided symbols using shared infrastructure (consistent
    // with i686/ARM/RISC-V backends via get_standard_linker_symbols)
    let text_seg_end = text_page_addr + text_total_size;
    let data_seg_start = rw_page_addr;
    let linker_addrs = LinkerSymbolAddresses {
        base_addr,
        got_addr: got_plt_addr,
        dynamic_addr,
        bss_addr,
        bss_size,
        text_end: text_seg_end,
        data_start: data_seg_start,
        init_array_start: init_array_addr,
        init_array_size,
        fini_array_start: fini_array_addr,
        fini_array_size,
        preinit_array_start: preinit_array_addr,
        preinit_array_size,
        rela_iplt_start: rela_iplt_addr,
        rela_iplt_size,
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
            entry.defined_in = Some(usize::MAX); // sentinel: linker-defined
            entry.section_idx = SHN_ABS;
        }
    }

    // Auto-generate __start_<section> / __stop_<section> symbols (GNU ld feature).
    // These are created for output sections whose names are valid C identifiers,
    // when there are undefined references to those symbols.
    for (name, addr) in linker_common::resolve_start_stop_symbols(output_sections) {
        if let Some(entry) = globals.get_mut(&name) {
            if entry.defined_in.is_none() && !entry.is_dynamic {
                entry.value = addr;
                entry.defined_in = Some(usize::MAX);
                entry.section_idx = SHN_ABS;
            }
        }
    }

    // Finalise deferred `--defsym` expressions.  This is the first point at
    // which every address they may reference is final: section addresses are
    // assigned, and `_end`/`_etext`/`__bss_start` and the `__start_`/`__stop_`
    // symbols exist.  Running it here (not in `link_builtin`, where the values
    // are still section-relative) is what makes `--defsym x=_start+4` agree
    // with GNU ld.
    super::link::evaluate_pending_defsyms(globals, pending_defsyms)?;

    // Override IFUNC symbol addresses to point to IPLT entries.
    // Save the original (resolver) addresses for IFUNC GOT initialization.
    let mut ifunc_resolver_addrs: Vec<u64> = Vec::new();
    for (i, name) in ifunc_symbols.iter().enumerate() {
        if let Some(gsym) = globals.get_mut(name) {
            ifunc_resolver_addrs.push(gsym.value);
            gsym.value = iplt_addr + (i as u64) * iplt_entry_size;
        }
    }
    // Local IFUNCs: an IFUNC symbol's `st_value` *is* the resolver's address
    // (the compiler points both `pick` and its `res` at the same offset), so
    // resolving the symbol through the ordinary section path yields exactly the
    // resolver address the IRELATIVE addend needs.  Unlike the globals above,
    // these are not redirected in `globals` -- the applier is handed an
    // explicit slot map below instead, because a local symbol must never be
    // resolved by name.
    let global_ifunc_slots = ifunc_resolver_addrs.len();
    let mut local_ifunc_slots: FxHashMap<(usize, usize), u64> = FxHashMap::default();
    for (i, &(obj_idx, sym_idx)) in local_ifuncs.iter().enumerate() {
        let slot = global_ifunc_slots + i;
        let Some(sym) = objects[obj_idx].symbols.get(sym_idx) else {
            continue;
        };
        let resolver = resolve_sym(
            obj_idx,
            sym,
            globals,
            section_map,
            output_sections,
            plt_addr,
        );
        ifunc_resolver_addrs.push(resolver);
        local_ifunc_slots.insert(
            (obj_idx, sym_idx),
            iplt_addr + slot as u64 * iplt_entry_size,
        );
    }

    let entry_name = entry_symbol.unwrap_or("_start");
    let entry_addr = globals
        .get(entry_name)
        .map(|s| s.value)
        .or_else(|| globals.get("_start").map(|s| s.value))
        .unwrap_or(text_page_addr);

    zone!("layout");
    // === .symtab / .strtab (debug symbol table) ===
    // ms178: previously the executable emitted ONLY .dynsym, so nm/gdb/perf
    // saw no symbol table at all. Every defined global symbol (function or
    // data) is now emitted into a full SHT_SYMTAB/.strtab pair, giving
    // profilers and debuggers address → name resolution. Section indices are
    // mapped to the OUTPUT section header order (mirrors the write loop
    // below). Symbols that resolve to linker-provided addresses (or common/
    // absolute) use SHN_ABS/SHN_COMMON as appropriate.
    // Pre-size: every defined global plus locals lands here; growth doubling
    // copies 24-byte entries repeatedly (measured 8.8 ms of a 60 ms link in
    // this zone for a 40k-symbol object).
    let est_syms: usize =
        globals.len() + objects.iter().map(|o| o.symbols.len()).sum::<usize>() / 4 + 2;
    let mut symtab_entries: Vec<[u8; 24]> = Vec::with_capacity(est_syms);
    let mut symtab_names: Vec<u8> = Vec::with_capacity(est_syms * 12);
    symtab_names.push(0u8); // strtab with leading NUL
    symtab_entries.push([0u8; 24]); // NULL symbol at index 0

    // Map output_sections index → section-header index, in write order.
    // Section-header index assignment.
    //
    // This walk defines the order in which section headers are written, and
    // three things must agree exactly: the index recorded for each output
    // section, the indices of .symtab/.strtab that follow them, and the write
    // loop further down. It used to be spelled out TWICE -- once assigning
    // `out_sec_to_hdr`, once re-counting to derive symtab_shidx -- with the
    // two copies kept in step by hand. That is the duplication that produced
    // the historical ordering bugs `layout_plan.rs` was created to prevent, so
    // it is now a single pass whose count is reused.
    // Merged `.comment` (compiler version strings, deduplicated).  Non-alloc
    // metadata: it needs a section header and file bytes but no address and
    // no segment.  Empty when no input carries a comment (or all are empty).
    // Computed up here because the header walk below must count it.
    let comment_data = linker_common::merge_comment_sections(objects);

    let mut out_sec_to_hdr: FxHashMap<usize, u16> = FxHashMap::default();
    // `-s` / `--strip-all`: no `.symtab`/`.strtab` at all.  Filtering the
    // builders below (rather than building and discarding) means a stripped
    // link also skips the sort and the string-table construction.
    let emit_symtab = !strip_all;
    let symtab_shidx;
    let strtab_shidx;
    let dynsym_shidx: u32;
    let dynstr_shidx: u32;
    // Number of headers before the output sections; `sh_count` continues from
    // here rather than re-deriving the same arithmetic.
    let linker_hdr_count: u16;
    {
        // Linker-created headers that precede the output sections, in the
        // exact order the write loop emits them.
        //
        // Each one is *named* here instead of skipped over with `h += N`.  The
        // previous form let `.dynsym`'s index be hardcoded as 3 further down,
        // which silently broke the moment `--hash-style=sysv` replaced
        // `.gnu.hash` with `.hash`: the header count stayed the same but the
        // cross-references did not.  Assigning the index where the header is
        // counted makes that class of bug unrepresentable.
        let mut h = 1usize; // [0] = NULL
        if !is_static {
            h += 1; // .interp
            if want_gnu_hash {
                h += 1; // .gnu.hash
            }
            if want_sysv_hash {
                h += 1; // .hash
            }
            dynsym_shidx = h as u32;
            h += 1; // .dynsym
            dynstr_shidx = h as u32;
            h += 1; // .dynstr
        } else {
            dynsym_shidx = 0;
            dynstr_shidx = 0;
        }
        if !is_static && verneed_size > 0 {
            h += 2;
        }
        if !is_static && rela_dyn_size > 0 {
            h += 1;
        }
        if !is_static && rela_plt_size > 0 {
            h += 1;
        }
        if !is_static && plt_size > 0 {
            h += 1;
        }
        if eh_frame_hdr_size > 0 {
            h += 1;
        } // .eh_frame_hdr
        // Captured here, before the output sections are numbered: `sh_count`
        // adds those itself, and taking the value after `assign` would count
        // them twice.
        linker_hdr_count = h as u16;

        // Output sections, in four ordered groups.
        let mut assign = |pred: &dyn Fn(&OutputSection) -> bool,
                          h: &mut usize,
                          map: &mut FxHashMap<usize, u16>| {
            for (i, sec) in output_sections.iter().enumerate() {
                if pred(sec) {
                    map.insert(i, *h as u16);
                    *h += 1;
                }
            }
        };
        // 1. ordinary allocated PROGBITS (init/fini arrays are placed later)
        assign(
            &|sec: &OutputSection| {
                sec.flags & SHF_ALLOC != 0
                    && sec.sh_type != SHT_NOBITS
                    && sec.flags & SHF_TLS == 0
                    && sec.name != ".init_array"
                    && sec.name != ".fini_array"
                    && sec.name != ".preinit_array"
            },
            &mut h,
            &mut out_sec_to_hdr,
        );
        // 2. TLS PROGBITS (.tdata)
        assign(
            &|sec: &OutputSection| {
                sec.flags & SHF_TLS != 0 && sec.flags & SHF_ALLOC != 0 && sec.sh_type != SHT_NOBITS
            },
            &mut h,
            &mut out_sec_to_hdr,
        );
        // 3. TLS NOBITS (.tbss)
        assign(
            &|sec: &OutputSection| sec.flags & SHF_TLS != 0 && sec.sh_type == SHT_NOBITS,
            &mut h,
            &mut out_sec_to_hdr,
        );

        // Linker-created headers between the alloc sections and .bss.
        if has_init_array {
            h += 1;
        }
        if has_fini_array {
            h += 1;
        }
        if has_preinit_array {
            h += 1;
        }
        if !is_static {
            h += 1;
        } // .dynamic
        if got_size > 0 {
            h += 1;
        } // .got
        if !is_static {
            h += 1;
        } // .got.plt
        if iplt_total_size > 0 {
            h += 1;
        }
        if rela_iplt_size > 0 {
            h += 1;
        }

        // 4. non-TLS NOBITS (.bss)
        assign(
            &|sec: &OutputSection| {
                sec.sh_type == SHT_NOBITS && sec.flags & SHF_ALLOC != 0 && sec.flags & SHF_TLS == 0
            },
            &mut h,
            &mut out_sec_to_hdr,
        );

        // .comment precedes .symtab (as in bfd), shifting both indices.
        if !comment_data.is_empty() {
            h += 1;
        }

        // .symtab and .strtab follow, then .shstrtab.
        symtab_shidx = if emit_symtab { h as u16 } else { 0 };
        strtab_shidx = if emit_symtab { h as u16 + 1 } else { 0 };
        if emit_symtab {
            h += 2;
        }
    }

    // ELF requires every STB_LOCAL entry before the first global entry.
    // Preserve named local FUNC/OBJECT/NOTYPE symbols from each input object so
    // perf, gdb, addr2line-like tools and callgrind can resolve static hot
    // functions (e.g. gzip's longest_match). Local names may repeat across
    // objects, which is legal; their object-local identity is retained here.
    let mut locals: Vec<(usize, &Symbol)> = objects
        .iter()
        .enumerate()
        .flat_map(|(obj_idx, obj)| {
            obj.symbols
                .iter()
                .filter(move |sym| {
                    emit_symtab
                        && sym.is_local()
                        && !sym.name.is_empty()
                        && sym.shndx != SHN_UNDEF
                        && sym.shndx != SHN_ABS
                        && section_map.contains_key(&(obj_idx, sym.shndx as usize))
                        // Collected away: the section was never laid out, so the
                        // symbol has no address.  Emitting it anyway produced a
                        // `.symtab` dominated by value-0/size!=0 entries -- on a
                        // 61-object `-ffunction-sections --gc-sections` link,
                        // 2341 of 2551 symbols, 61 KB against bfd's 6 KB.
                        // ICF-folded is the exception: the section was laid
                        // out once, under the representative, and
                        // `section_map` already points there — the symbol is
                        // alive as an alias and must be emitted (lld parity).
                        && (!dead_sections.contains(&(obj_idx, sym.shndx as usize))
                            || folded_sections
                                .contains_key(&(obj_idx, sym.shndx as usize)))
                })
                .map(move |sym| (obj_idx, sym))
        })
        .collect();
    locals.sort_unstable_by(|(oa, a), (ob, b)| {
        oa.cmp(ob)
            .then_with(|| a.value.cmp(&b.value))
            .then_with(|| a.name.cmp(&b.name))
    });
    for (obj_idx, sym) in locals {
        let (oi, sec_off) = section_map[&(obj_idx, sym.shndx as usize)];
        let Some(&shndx) = out_sec_to_hdr.get(&oi) else {
            continue;
        };
        let off = push_strtab_name(&mut symtab_names, sym.name.as_bytes());
        let value = output_sections[oi].addr + sec_off + sym.value;
        symtab_entries.push(elf64_sym_entry(
            off, sym.info, sym.other, shndx, value, sym.size,
        ));
    }

    let mut sym_names: Vec<(&String, &GlobalSymbol)> = globals
        .iter()
        .filter(|(_, g)| {
            emit_symtab
                && g.defined_in.is_some()
                && !g.is_dynamic
                // Same rule as the locals above: a global defined in a section
                // that was collected away has no address to report.  Folded
                // globals are the exception — they alias the representative.
                && !matches!(g.defined_in,
                    Some(oi) if g.section_idx != SHN_ABS
                        && g.section_idx != SHN_COMMON
                        && dead_sections.contains(&(oi, g.section_idx as usize))
                        && !folded_sections
                            .contains_key(&(oi, g.section_idx as usize)))
                // A LOCAL-binding entry that is section-defined and laid out
                // is already emitted by the locals loop above (synthetic
                // `<string-merge>` pool symbols live in BOTH the object
                // symbol lists and the resolved map — that is how lookups
                // find them). Emitting it here too duplicates the symbol AND
                // breaks the locals-before-globals order (a STB_LOCAL entry
                // after `sh_info` is a hard ELF violation `readelf` warns
                // about). Skip exactly the covered case; an unmapped local
                // still emits below and the partition step places it.
                && !matches!(g.defined_in,
                    Some(oi)
                        if (g.info >> 4) == STB_LOCAL
                            && g.section_idx != SHN_ABS
                            && g.section_idx != SHN_COMMON
                            && section_map.contains_key(&(oi, g.section_idx as usize)))
        })
        .collect();
    // Sort by a cached big-endian 8-byte prefix first: for symbol-heavy
    // objects the names share long prefixes ("F1", "F12", ...), so plain
    // byte-wise cmp walks the common prefix on every comparison. The u64
    // prefix decides almost every comparison in one instruction; ties fall
    // back to the full byte compare (total order preserved, identical
    // resulting symtab order).
    //
    // The prefix is 16 bytes, not 8. Eight bytes is too narrow for real symbol
    // tables: names routinely share a longer prefix than that (`g_sym_12345`,
    // `_ZNSt3__1`, `__pthread_`, `nghttp2_session_`), so the u64 key ties and
    // every comparison falls through to the byte compare. Profiling a
    // 20k-symbol link showed 7.0% of all instructions in `__memcmp_avx2_movbe`
    // for exactly that reason. A u128 covers the whole name for the vast
    // majority of symbols, so the fallback almost never runs; the fallback is
    // retained so the total order — and therefore the emitted symtab — is
    // byte-for-byte identical to the previous implementation.
    //
    // Measured alternative, rejected: an *index* sort (sort u32 indices into a
    // separate key array) moves 4 bytes per swap instead of 32, but the double
    // indirection on every comparison cost more than the wider swap saved:
    // 57.8M instructions vs 53.7M for this version on the 20k-symbol profile.
    // Keeping the tuple sort.
    let mut keyed: Vec<(u128, &String, &GlobalSymbol)> = sym_names
        .iter()
        .map(|(n, g)| {
            let b = n.as_bytes();
            let mut p = [0u8; 16];
            let l = b.len().min(16);
            p[..l].copy_from_slice(&b[..l]);
            (u128::from_be_bytes(p), *n, *g)
        })
        .collect();
    keyed.sort_unstable_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.as_bytes().cmp(b.1.as_bytes()))
    });
    let sym_names: Vec<(&String, &GlobalSymbol)> =
        keyed.into_iter().map(|(_, n, g)| (n, g)).collect();
    for (name, gsym) in &sym_names {
        let off = push_strtab_name(&mut symtab_names, name.as_bytes());
        let shndx: u16 = if gsym.defined_in == Some(usize::MAX) || gsym.section_idx == SHN_ABS {
            SHN_ABS
        } else if gsym.section_idx == SHN_COMMON {
            SHN_COMMON
        } else if let Some(obj_idx) = gsym.defined_in {
            match section_map.get(&(obj_idx, gsym.section_idx as usize)) {
                Some(&(oi, _)) => out_sec_to_hdr.get(&oi).copied().unwrap_or(SHN_ABS),
                None => SHN_ABS,
            }
        } else {
            SHN_ABS
        };
        symtab_entries.push(elf64_sym_entry(
            off, gsym.info, 0, /* st_other */
            shndx, gsym.value, gsym.size,
        ));
    }
    // ELF mandates every STB_LOCAL entry before the first global, and
    // `sh_info` must equal the number of STB_LOCAL entries, counting the
    // NULL symbol at index 0.  The globals filter above keeps covered
    // locals out, but any future path that appends a STB_LOCAL entry here
    // (this class has recurred: `sh_info` used to be snapshotted between
    // the loops, then derived by counting — both still wrong when a local
    // lands after a global) would silently re-break the order.  Enforce it
    // structurally instead: a STABLE partition keeps the NULL at index 0
    // and preserves each loop's order within its run, so the invariant
    // holds no matter what the loops append.  Cost is one stable sort over
    // a few hundred cache-resident 24-byte entries — noise next to the
    // u128-prefixed global sort above.
    symtab_entries.sort_by_key(|e| (e[4] >> 4) != STB_LOCAL);
    // `sh_info` must equal the number of STB_LOCAL entries, counting the NULL
    // symbol at index 0.  Derived from the finished (now partitioned) table:
    // correct by construction, and one linear scan over data already in cache.
    let n_local = symtab_entries
        .iter()
        .filter(|e| (e[4] >> 4) == STB_LOCAL)
        .count();
    zone!("symtab");
    // === Build output buffer ===
    let file_size = offset as usize;
    // Defence in depth against malformed input driving a gigantic layout.
    // `parse_elf64_object` already rejects non-power-of-two sh_addralign (the
    // fuzzer-found path that demanded a 1 TiB buffer), but any future arithmetic
    // slip here would abort the process inside the allocator, which
    // `catch_unwind` cannot intercept.  Fail with a diagnostic instead.
    //
    // The cap is deliberately far above any real link (the Linux kernel's
    // vmlinux is ~1 GiB at the extreme) and is a guard, not a policy limit.
    const MAX_OUTPUT_BYTES: usize = 64 << 30; // 64 GiB
    if file_size > MAX_OUTPUT_BYTES {
        return Err(format!(
            "output would be {} bytes ({:.1} GiB), which exceeds the {} GiB sanity limit; \
             this usually means an input object has corrupt section offsets or sizes",
            file_size,
            file_size as f64 / (1u64 << 30) as f64,
            MAX_OUTPUT_BYTES >> 30
        ));
    }
    let mut out = vec![0u8; file_size];

    // ELF header
    out[0..4].copy_from_slice(&ELF_MAGIC);
    out[4] = ELFCLASS64;
    out[5] = ELFDATA2LSB;
    out[6] = 1;
    w16(&mut out, 16, if is_pie { ET_DYN } else { ET_EXEC });
    w16(&mut out, 18, EM_X86_64);
    w32(&mut out, 20, 1);
    w64(&mut out, 24, entry_addr);
    w64(&mut out, 32, 64);
    w64(&mut out, 40, 0);
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
    if !is_static {
        wphdr(
            &mut out,
            ph,
            PT_INTERP,
            PF_R,
            interp_offset,
            interp_addr,
            interp.len() as u64,
            interp.len() as u64,
            1,
        );
        ph += 56;
    }
    let ro_seg_end = rela_plt_offset + rela_plt_size;
    wphdr(
        &mut out, ph, PT_LOAD, PF_R, 0, base_addr, ro_seg_end, ro_seg_end, PAGE_SIZE,
    );
    ph += 56;
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
    if has_relro {
        // RELRO LOAD: const-after-relocation window. filesz covers only the
        // file content; memsz reaches across the NOBITS page pad so the
        // loader materialises the tail page that mprotect will cover.
        wphdr(
            &mut out,
            ph,
            PT_LOAD,
            PF_R | PF_W,
            rw_page_offset,
            rw_page_addr,
            relro_file_end - rw_page_offset,
            relro_size,
            PAGE_SIZE,
        );
        ph += 56;
        if split_relro_load {
            // Writable tail (lazy .got.plt / IFUNC GOT / .data / TLS / .bss):
            // same dense file offset, one fresh page of address space.
            // p_filesz can be 0 (pure-bss tail): an anonymous-zero segment.
            let rw2_filesz = offset - relro_file_end;
            let rw2_memsz = if bss_size > 0 {
                (bss_addr + bss_size) - rw2_addr
            } else {
                rw2_filesz
            };
            wphdr(
                &mut out,
                ph,
                PT_LOAD,
                PF_R | PF_W,
                relro_file_end,
                rw2_addr,
                rw2_filesz,
                rw2_memsz,
                PAGE_SIZE,
            );
            ph += 56;
        }
    } else {
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
    }
    if !is_static {
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
    }
    // One PT_NOTE per contiguous run of allocated note sections — the same
    // walk that counted them before layout (identical predicate and order,
    // so emission can never disagree with e_phnum).  The span runs from the
    // first note's start to the last note's end; p_align is the maximum
    // member alignment (minimum 4, as in GNU ld).  Written after DYNAMIC,
    // before the synthetic segments, matching GNU ld's program header
    // order.
    {
        let mut run_start: Option<(u64, u64, u64)> = None; // (file_off, addr, align)
        let mut run_end: Option<(u64, u64)> = None; // (file_end, addr_end)
        let mut flush = |run: Option<(u64, u64, u64)>,
                         end: Option<(u64, u64)>,
                         out: &mut Vec<u8>,
                         ph: &mut usize| {
            if let (Some((fo, va, al)), Some((fe, ae))) = (run, end) {
                wphdr(out, *ph, PT_NOTE, PF_R, fo, va, fe - fo, ae - va, al);
                *ph += 56;
            }
        };
        for sec in output_sections.iter() {
            if is_alloc_note(sec) {
                if run_start.is_none() {
                    run_start = Some((sec.file_offset, sec.addr, sec.alignment.max(4)));
                } else if let Some(r) = &mut run_start {
                    r.2 = r.2.max(sec.alignment.max(4));
                }
                run_end = Some((
                    sec.file_offset + sec.data.len() as u64,
                    sec.addr + sec.mem_size,
                ));
            } else {
                flush(run_start.take(), run_end.take(), &mut out, &mut ph);
            }
        }
        flush(run_start.take(), run_end.take(), &mut out, &mut ph);
    }
    if has_gnu_property_phdr {
        if let Some(sec) = output_sections
            .iter()
            .find(|s| s.name == ".note.gnu.property" && !s.data.is_empty())
        {
            wphdr(
                &mut out,
                ph,
                PT_GNU_PROPERTY,
                PF_R,
                sec.file_offset,
                sec.addr,
                sec.data.len() as u64,
                sec.mem_size,
                8,
            );
            ph += 56;
        }
    }
    if eh_frame_hdr_size > 0 {
        wphdr(
            &mut out,
            ph,
            PT_GNU_EH_FRAME,
            PF_R,
            eh_frame_hdr_offset,
            eh_frame_hdr_vaddr,
            eh_frame_hdr_size,
            eh_frame_hdr_size,
            4,
        );
        ph += 56;
    }
    // Executable stack when ANY input object requests it (GNU semantics:
    // .note.GNU-stack with SHF_EXECINSTR — set by nested-function
    // trampolines). GNU ld ORs the flag across all inputs.
    let exec_stack = objects.iter().any(|obj| {
        obj.sections
            .iter()
            .any(|sec| sec.name == ".note.GNU-stack" && sec.flags & SHF_EXECINSTR != 0)
    });
    let stack_flags = if exec_stack {
        PF_R | PF_W | PF_X
    } else {
        PF_R | PF_W
    };
    wphdr(&mut out, ph, PT_GNU_STACK, stack_flags, 0, 0, 0, 0, 0x10);
    ph += 56;
    if has_relro && relro_size > 0 {
        wphdr(
            &mut out,
            ph,
            PT_GNU_RELRO,
            PF_R,
            relro_start,
            relro_start_addr,
            // filesz covers the file content only; ld.so reads vaddr+memsz
            // and mprotects the page-rounded range. (lld emits the same pair.)
            relro_file_end - relro_start,
            relro_size,
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

    // Section data (needed for both static and dynamic)
    for sec in output_sections.iter() {
        if sec.sh_type == SHT_NOBITS || sec.data.is_empty() {
            continue;
        }
        write_bytes(&mut out, sec.file_offset as usize, &sec.data);
    }

    // Dynamic linking sections (skipped for static executables)
    if !is_static {
        // .interp
        write_bytes(&mut out, interp_offset as usize, interp);

        // .gnu.hash - proper hash table so dynamic linker can find copy-reloc
        // symbols.  Gated on `want_gnu_hash`: at zero size it aliases
        // `sysv_hash_offset`, and writing here would clobber the SysV table.
        let gh = gnu_hash_offset as usize;
        if want_gnu_hash {
            w32(&mut out, gh, gnu_hash_nbuckets);
            w32(&mut out, gh + 4, gnu_hash_symoffset as u32);
            w32(&mut out, gh + 8, gnu_hash_bloom_size);
            w32(&mut out, gh + 12, gnu_hash_bloom_shift);
            // Bloom filter: `bloom_size` 64-bit words.
            let bloom_off = gh + 16;
            for (i, &w) in bloom_words.iter().enumerate() {
                w64(&mut out, bloom_off + i * 8, w);
            }
            // Buckets
            let buckets_off = bloom_off + (gnu_hash_bloom_size as usize * 8);
            for (i, &b) in gnu_hash_buckets.iter().enumerate() {
                w32(&mut out, buckets_off + i * 4, b);
            }
            // Chains
            let chains_off = buckets_off + (gnu_hash_nbuckets as usize * 4);
            for (i, &c) in gnu_hash_chains.iter().enumerate() {
                w32(&mut out, chains_off + i * 4, c);
            }
        }

        // .hash (SysV): nbucket, nchain, bucket[nbucket], chain[nchain]
        if let Some(sh) = &sysv_hash {
            linker_common::write_sysv_hash(&mut out, sysv_hash_offset as usize, sh);
        }

        // .dynsym
        let mut ds = dynsym_offset as usize + 24; // skip null entry
        for name in &dyn_sym_names {
            let no = dynstr.get_offset(dynsym_emit_name(name)) as u32;
            w32(&mut out, ds, no);
            if let Some(gsym) = globals.get(name) {
                if gsym.copy_reloc {
                    if ds + 5 < out.len() {
                        out[ds + 4] = (STB_GLOBAL << 4) | STT_OBJECT;
                        out[ds + 5] = 0;
                    }
                    w16(&mut out, ds + 6, 1);
                    w64(&mut out, ds + 8, gsym.value);
                    w64(&mut out, ds + 16, gsym.size);
                } else if !gsym.is_dynamic && gsym.section_idx != SHN_UNDEF && gsym.value != 0 {
                    let stt = gsym.info & 0xf;
                    let stb = gsym.info >> 4;
                    let st_info = (stb << 4) | stt;
                    if ds + 5 < out.len() {
                        out[ds + 4] = st_info;
                        out[ds + 5] = 0;
                    }
                    w16(&mut out, ds + 6, 1);
                    // For TLS symbols, the dynsym value must be the offset within
                    // the TLS segment, not the virtual address.
                    let sym_val = if stt == STT_TLS && tls_addr != 0 {
                        gsym.value - tls_addr
                    } else {
                        gsym.value
                    };
                    w64(&mut out, ds + 8, sym_val);
                    w64(&mut out, ds + 16, gsym.size);
                } else {
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

        // .dynstr
        write_bytes(&mut out, dynstr_offset as usize, dynstr.as_bytes());

        // .gnu.version (versym)
        if !versym_data.is_empty() {
            write_bytes(&mut out, versym_offset as usize, &versym_data);
        }

        // .gnu.version_r (verneed)
        if !verneed_data.is_empty() {
            write_bytes(&mut out, verneed_offset as usize, &verneed_data);
        }

        // .rela.dyn (GLOB_DAT for dynamic GOT symbols, R_X86_64_COPY for copy relocs)
        let mut rd = rela_dyn_offset as usize;
        // PIE slide entries first: ld.so's common case is a run of RELATIVE at
        // the head of the table, and this is the order bfd and mold emit.
        //
        // Values come from the same `resolve_sym` the relocation applier below
        // uses, so a target reached through a section symbol -- how the
        // compiler points `const char *msgs[]` at merged string literals --
        // resolves to its merged address rather than to 0.  Because
        // `base_addr` is 0 for a PIE, that value is also what the applier
        // stores in the file, so the addend and the stored bytes agree.
        for pr in &pie_relative {
            // Unwrap is safe: the list was filtered on exactly this key above.
            let (out_idx, sec_off) = section_map[&(pr.obj_idx, pr.sec_idx)];
            let obj = &objects[pr.obj_idx];
            let s = resolve_sym(
                pr.obj_idx,
                &obj.symbols[pr.sym_idx],
                globals,
                section_map,
                output_sections,
                plt_addr,
            );
            w64(
                &mut out,
                rd,
                output_sections[out_idx].addr + sec_off + pr.offset,
            );
            w64(&mut out, rd + 8, R_X86_64_RELATIVE as u64);
            w64(&mut out, rd + 16, (s as i64 + pr.addend) as u64);
            rd += 24;
        }
        // ...then the GOT slots holding locally-defined addresses.  Slot N of
        // the non-PLT range lives at got_addr + N*8, matching the numbering the
        // GLOB_DAT writer below uses.
        for &ord in &pie_got_relative {
            let name = &got_entries
                .iter()
                .filter(|(n, p)| !n.is_empty() && !*p)
                .nth(ord)
                .map(|(n, _)| n.clone())
                .unwrap_or_default();
            let addend = globals
                .get(name.as_str())
                .map(|g| {
                    if g.is_dynamic && !g.copy_reloc {
                        // The slot was filled with our PLT entry, not g.value.
                        g.plt_idx
                            .map(|pi| plt_addr + 16 + pi as u64 * 16)
                            .unwrap_or(g.value)
                    } else {
                        g.value
                    }
                })
                .unwrap_or(0);
            w64(&mut out, rd, got_addr + ord as u64 * 8);
            w64(&mut out, rd + 8, R_X86_64_RELATIVE as u64);
            w64(&mut out, rd + 16, addend);
            rd += 24;
        }
        let mut gd_a = got_addr;
        for (name, is_plt) in got_entries {
            if name.is_empty() || *is_plt {
                continue;
            }
            let gsym_info = globals.get(name);
            let is_dynamic = gsym_info
                .map(|g| g.is_dynamic && !g.copy_reloc)
                .unwrap_or(false);
            let has_plt = gsym_info.map(|g| g.plt_idx.is_some()).unwrap_or(false);
            // Skip GLOB_DAT for dynamic symbols that also have a PLT entry:
            // their GOT entry is statically filled with the PLT address to match
            // the canonical address used by R_X86_64_64 data relocations.
            if is_dynamic && !has_plt {
                let si = dyn_sym_index.get(name.as_str()).copied().unwrap_or(0);
                // TLS symbols get R_X86_64_TPOFF64 (ld.so stores the TP offset);
                // everything else gets GLOB_DAT.
                let is_tls = globals
                    .get(name)
                    .map(|g| (g.info & 0xf) == STT_TLS)
                    .unwrap_or(false);
                let rtype = if is_tls {
                    R_X86_64_TPOFF64
                } else {
                    R_X86_64_GLOB_DAT
                } as u64;
                w64(&mut out, rd, gd_a);
                w64(&mut out, rd + 8, (si << 32) | rtype);
                w64(&mut out, rd + 16, 0);
                rd += 24;
            }
            gd_a += 8;
        }
        // Absolute R_X86_64_64 relocations against dynamic data symbols.
        // ld.so writes `symbol_address + addend` into the storage at link time
        // of loading; we leave the storage itself zero (see the R_X86_64_64
        // emit arm) so a stale addend cannot be mistaken for a real pointer.
        for r in abs_dyn_relocs {
            if globals
                .get(r.name.as_str())
                .map(|g| g.copy_reloc)
                .unwrap_or(true)
            {
                continue; // copy-relocated: R_X86_64_COPY handles it below
            }
            let Some(&(out_idx, sec_off)) = section_map.get(&(r.obj_idx, r.sec_idx)) else {
                continue;
            };
            let addr = output_sections[out_idx].addr + sec_off + r.offset;
            let si = dyn_sym_index.get(r.name.as_str()).copied().unwrap_or(0);
            debug_assert!(
                si != 0,
                "absolute dynamic reloc against symbol absent from .dynsym"
            );
            w64(&mut out, rd, addr);
            w64(&mut out, rd + 8, (si << 32) | R_X86_64_64 as u64);
            w64(&mut out, rd + 16, r.addend as u64);
            rd += 24;
        }

        // R_X86_64_COPY relocations for copy-relocated symbols
        for (name, _) in &copy_reloc_syms {
            if let Some(gsym) = globals.get(name) {
                let si = dyn_sym_index.get(name.as_str()).copied().unwrap_or(0);
                let copy_addr = gsym.value;
                w64(&mut out, rd, copy_addr);
                w64(&mut out, rd + 8, (si << 32) | 5);
                w64(&mut out, rd + 16, 0);
                rd += 24;
            }
        }
        // R_X86_64_IRELATIVE for IFUNCs in dynamic executables: ld.so calls
        // the resolver (addend) and stores the result into the IFUNC GOT slot.
        for i in 0..dyn_irelative_count {
            let slot = ifunc_got_addr + i as u64 * 8;
            w64(&mut out, rd, slot);
            w64(&mut out, rd + 8, R_X86_64_IRELATIVE as u64);
            w64(&mut out, rd + 16, ifunc_resolver_addrs[i]);
            rd += 24;
        }

        // .rela.plt
        let mut rp = rela_plt_offset as usize;
        let gpb = got_plt_addr + 24;
        for (i, name) in plt_names.iter().enumerate() {
            let gea = gpb + i as u64 * 8;
            let si = dyn_sym_index.get(name.as_str()).copied().unwrap_or(0);
            w64(&mut out, rp, gea);
            w64(&mut out, rp + 8, (si << 32) | R_X86_64_JUMP_SLOT as u64);
            w64(&mut out, rp + 16, 0);
            rp += 24;
        }

        // .plt
        if plt_size > 0 {
            let po = plt_offset as usize;
            out[po] = 0xff;
            out[po + 1] = 0x35;
            w32(
                &mut out,
                po + 2,
                ((got_plt_addr + 8) as i64 - (plt_addr + 6) as i64) as u32,
            );
            out[po + 6] = 0xff;
            out[po + 7] = 0x25;
            w32(
                &mut out,
                po + 8,
                ((got_plt_addr + 16) as i64 - (plt_addr + 12) as i64) as u32,
            );
            for i in 12..16 {
                out[po + i] = 0x90;
            }

            for (i, _) in plt_names.iter().enumerate() {
                let ep = po + 16 + i * 16;
                let pea = plt_addr + 16 + i as u64 * 16;
                let gea = got_plt_addr + 24 + i as u64 * 8;
                out[ep] = 0xff;
                out[ep + 1] = 0x25;
                w32(&mut out, ep + 2, (gea as i64 - (pea + 6) as i64) as u32);
                out[ep + 6] = 0x68;
                w32(&mut out, ep + 7, i as u32);
                out[ep + 11] = 0xe9;
                w32(
                    &mut out,
                    ep + 12,
                    (plt_addr as i64 - (pea + 16) as i64) as u32,
                );
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
        // DT_INIT/DT_FINI: the .init/.fini section addresses (crti/crtn put
        // _init/_fini at the section starts, so this equals the symbol
        // values on every normal link — verified against bfd).  Placed
        // right after NEEDED as in bfd.  The lookups cannot miss: `has_init`
        // / `has_fini` above test the same predicate.
        if has_init {
            let init_addr = output_sections
                .iter()
                .find(|s| s.name == ".init")
                .map(|s| s.addr)
                .unwrap_or(0);
            w64(&mut out, dd, DT_INIT as u64);
            w64(&mut out, dd + 8, init_addr);
            dd += 16;
        }
        if has_fini {
            let fini_addr = output_sections
                .iter()
                .find(|s| s.name == ".fini")
                .map(|s| s.addr)
                .unwrap_or(0);
            w64(&mut out, dd, DT_FINI as u64);
            w64(&mut out, dd + 8, fini_addr);
            dd += 16;
        }
        for &(tag, val) in &[
            (DT_STRTAB, dynstr_addr),
            (DT_SYMTAB, dynsym_addr),
            (DT_STRSZ, dynstr_size),
            (DT_SYMENT, 24),
            (DT_DEBUG, 0),
            (DT_PLTGOT, got_plt_addr),
            (DT_PLTRELSZ, rela_plt_size),
            (DT_PLTREL, DT_RELA as u64),
            (DT_JMPREL, rela_plt_addr),
            (DT_RELA, rela_dyn_addr),
            (DT_RELASZ, rela_dyn_size),
            (DT_RELAENT, 24),
        ] {
            w64(&mut out, dd, tag as u64);
            w64(&mut out, dd + 8, val);
            dd += 16;
        }
        // DT_RELACOUNT: size of the leading R_X86_64_RELATIVE run in
        // .rela.dyn (both RELATIVE loops above run before any GLOB_DAT, so
        // `relacount` is exact).  Omitted when zero, as GNU does.
        if relacount > 0 {
            w64(&mut out, dd, DT_RELACOUNT as u64);
            w64(&mut out, dd + 8, relacount as u64);
            dd += 16;
        }
        // Hash tables, in the order the loader probes them: DT_GNU_HASH first,
        // because a loader that understands it prefers it and ignores DT_HASH.
        if want_gnu_hash {
            w64(&mut out, dd, DT_GNU_HASH as u64);
            w64(&mut out, dd + 8, gnu_hash_addr);
            dd += 16;
        }
        if want_sysv_hash {
            w64(&mut out, dd, DT_HASH as u64);
            w64(&mut out, dd + 8, sysv_hash_addr);
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
        if has_preinit_array {
            w64(&mut out, dd, DT_PREINIT_ARRAY as u64);
            w64(&mut out, dd + 8, preinit_array_addr);
            dd += 16;
            w64(&mut out, dd, DT_PREINIT_ARRAYSZ as u64);
            w64(&mut out, dd + 8, preinit_array_size);
            dd += 16;
        }
        if let Some(ref rp) = rpath_string {
            let rp_off = dynstr.get_offset(rp) as u64;
            let tag = if use_runpath { DT_RUNPATH } else { DT_RPATH };
            w64(&mut out, dd, tag as u64);
            w64(&mut out, dd + 8, rp_off);
            dd += 16;
        }
        if verneed_size > 0 {
            w64(&mut out, dd, DT_VERSYM as u64);
            w64(&mut out, dd + 8, versym_addr);
            dd += 16;
            w64(&mut out, dd, DT_VERNEED as u64);
            w64(&mut out, dd + 8, verneed_addr);
            dd += 16;
            w64(&mut out, dd, DT_VERNEEDNUM as u64);
            w64(&mut out, dd + 8, verneed_count as u64);
            dd += 16;
        }
        if z_now {
            w64(&mut out, dd, DT_FLAGS as u64);
            w64(&mut out, dd + 8, DF_BIND_NOW as u64);
            dd += 16;
        }
        if z_now || is_pie {
            let mut flags1: i64 = 0;
            if z_now {
                flags1 |= DF_1_NOW;
            }
            if is_pie {
                // Without DF_1_PIE an ET_DYN is treated as a shared object:
                // ld.so would let the global scope interpose its symbols and
                // would not apply the executable's lookup rules.
                flags1 |= DF_1_PIE;
            }
            w64(&mut out, dd, DT_FLAGS_1 as u64);
            w64(&mut out, dd + 8, flags1 as u64);
            dd += 16;
        }
        w64(&mut out, dd, DT_NULL as u64);
        w64(&mut out, dd + 8, 0);

        // .got.plt
        let gp = got_plt_offset as usize;
        w64(&mut out, gp, dynamic_addr);
        w64(&mut out, gp + 8, 0);
        w64(&mut out, gp + 16, 0);
        for (i, _) in plt_names.iter().enumerate() {
            w64(&mut out, gp + 24 + i * 8, plt_addr + 16 + i as u64 * 16 + 6);
        }
    } // end if !is_static

    // .got (needed for both static and dynamic: TLS GOTTPOFF, GOTPCREL entries)
    if got_size > 0 {
        let mut go = got_offset as usize;
        for (name, is_plt) in got_entries {
            if name.is_empty() || *is_plt {
                continue;
            }
            if let Some(gsym) = globals.get(name) {
                if gsym.defined_in.is_some() && !gsym.is_dynamic {
                    let sym_val = gsym.value;
                    if has_tls && (gsym.info & 0xf) == STT_TLS {
                        // TLS GOT entry: store the TPOFF value
                        let tpoff = (sym_val as i64 - tls_addr as i64) - tls_mem_size as i64;
                        w64(&mut out, go, tpoff as u64);
                    } else {
                        w64(&mut out, go, sym_val);
                    }
                } else if gsym.copy_reloc && gsym.value != 0 {
                    w64(&mut out, go, gsym.value);
                } else if gsym.is_dynamic {
                    if let Some(plt_idx) = gsym.plt_idx {
                        // Dynamic function with both PLT and GOTPCREL: fill GOT with
                        // PLT entry address so address-of via GOTPCREL matches the
                        // canonical PLT address used by R_X86_64_64 data relocations.
                        let plt_entry_addr = plt_addr + 16 + plt_idx as u64 * 16;
                        w64(&mut out, go, plt_entry_addr);
                    }
                }
            }
            go += 8;
        }
    }

    // IFUNC: write .iplt, IFUNC GOT, and .rela.iplt data
    if num_ifunc > 0 {
        // .iplt - each entry is: jmp *ifunc_got_entry(%rip); nop padding
        for i in 0..num_ifunc {
            let ep = iplt_offset as usize + i * iplt_entry_size as usize;
            let pea = iplt_addr + i as u64 * iplt_entry_size; // address of this IPLT entry
            let gea = ifunc_got_addr + i as u64 * 8; // address of IFUNC GOT entry
            // ff 25 XX XX XX XX = jmp *disp32(%rip)
            out[ep] = 0xff;
            out[ep + 1] = 0x25;
            w32(&mut out, ep + 2, (gea as i64 - (pea + 6) as i64) as u32);
            // Pad remaining 10 bytes with NOPs
            for j in 6..iplt_entry_size as usize {
                out[ep + j] = 0x90;
            }
        }

        // IFUNC GOT - initialized to resolver function addresses
        for (i, &resolver_addr) in ifunc_resolver_addrs.iter().enumerate() {
            let go = ifunc_got_offset as usize + i * 8;
            w64(&mut out, go, resolver_addr);
        }

        // .rela.iplt - R_X86_64_IRELATIVE relocations (static executables only;
        // dynamic executables emit IRELATIVE into .rela.dyn above)
        if rela_iplt_size > 0 {
            for i in 0..num_ifunc {
                let rp = rela_iplt_offset as usize + i * 24;
                let r_offset = ifunc_got_addr + i as u64 * 8;
                // r_info: (0 << 32) | R_X86_64_IRELATIVE
                w64(&mut out, rp, r_offset);
                w64(&mut out, rp + 8, R_X86_64_IRELATIVE as u64);
                w64(&mut out, rp + 16, ifunc_resolver_addrs[i]); // r_addend = resolver address
            }
        }
    }

    zone!("buffer");

    // TLS-consumed ranges: file offsets overwritten by a prior TLS GD/LD→LE
    // rewrite. Subsequent relocations whose file offset falls inside any range
    // must be skipped. Small lists use linear scan; past 32 entries the structure
    // sorts+merges in place and switches to binary search permanently.
    struct TlsConsumed {
        ranges: Vec<(usize, usize)>,
        sorted: bool,
    }
    impl TlsConsumed {
        fn new() -> Self {
            Self {
                ranges: Vec::new(),
                sorted: true,
            }
        }
        fn push(&mut self, start: usize, end: usize) {
            self.ranges.push((start, end));
            self.sorted = false;
        }
        fn contains(&mut self, fp: usize) -> bool {
            if self.ranges.is_empty() {
                return false;
            }
            if self.ranges.len() <= 32 && !self.sorted {
                return self.ranges.iter().any(|&(s, e)| fp >= s && fp < e);
            }
            if !self.sorted {
                self.ranges.sort_unstable_by_key(|&(s, _)| s);
                let mut merged: Vec<(usize, usize)> = Vec::with_capacity(self.ranges.len());
                for (s, e) in self.ranges.drain(..) {
                    if let Some(last) = merged.last_mut() {
                        if s <= last.1 {
                            last.1 = last.1.max(e);
                            continue;
                        }
                    }
                    merged.push((s, e));
                }
                self.ranges = merged;
                self.sorted = true;
            }
            self.ranges
                .binary_search_by(|&(s, e)| {
                    if fp < s {
                        std::cmp::Ordering::Greater
                    } else if fp >= e {
                        std::cmp::Ordering::Less
                    } else {
                        std::cmp::Ordering::Equal
                    }
                })
                .is_ok()
        }
    }
    let mut tls_consumed = TlsConsumed::new();

    // === Apply relocations ===
    // `globals` is never touched again after this point, so a plain reborrow
    // suffices. The old `.clone()` deep-copied the entire map — 40k String
    // keys + values — on EVERY link (several ms and a peak-RSS spike on
    // symbol-heavy inputs) for no semantic benefit.
    let globals_snap: &FxHashMap<String, GlobalSymbol> = globals;

    // Precomputed ordinal of each non-PLT GOT entry (index into the .got
    // section). Replaces per-relocation prefix scans that were O(GOT^2).
    let got_slot_ordinal: Vec<usize> = {
        let mut v = Vec::with_capacity(got_entries.len());
        let mut nb = 0usize;
        for (n, p) in got_entries.iter() {
            v.push(nb);
            if !n.is_empty() && !*p {
                nb += 1;
            }
        }
        v
    };

    // Byte ranges consumed by TLS GD/LD -> LE relaxation. The __tls_get_addr
    // call that follows a relaxed TLSGD/TLSLD sequence has its own PLT32 /
    // GOTPCRELX relocation which must NOT be applied (the call bytes have been
    // overwritten). Ranges are file offsets.

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

            // Loop-invariant parts of the offset check below, hoisted: the
            // section's size and name and the object's name do not change per
            // relocation, and this loop runs once per relocation in the link.
            let in_sec = &objects[obj_idx].sections[sec_idx];
            let (sec_size, sec_name) = (in_sec.size, in_sec.name.as_str());
            let obj_name = objects[obj_idx].source_name.as_str();

            for rela in relas {
                let si = rela.sym_idx as usize;
                if si >= objects[obj_idx].symbols.len() {
                    continue;
                }
                let sym = &objects[obj_idx].symbols[si];
                let p = sa + sec_off + rela.offset;
                let fp = (sfo + sec_off + rela.offset) as usize;
                let a = rela.addend;
                // GNU ld refuses a relocation whose field crosses the end of the
                // section it patches (bfd_reloc_outofrange, reported as "error
                // 4"). The parser already rejects an offset outside the section
                // altogether; this is the width-aware half of the same rule, and
                // it lives here because the field width is arch-specific while the
                // parser is shared by four backends.
                reloc_field::offset_in_section(
                    rela.rela_type,
                    reloc_field::name(rela.rela_type),
                    rela.offset,
                    reloc_field::patch_width(rela.rela_type),
                    sec_size,
                    sec_name,
                    obj_name,
                )?;

                // Skip relocations whose bytes were consumed by a TLS GD/LD->LE
                // rewrite (the __tls_get_addr call no longer exists).
                if tls_consumed.contains(fp) {
                    continue;
                }
                let s = resolve_sym(
                    obj_idx,
                    sym,
                    globals_snap,
                    section_map,
                    output_sections,
                    plt_addr,
                );
                // A reference to a local IFUNC must land on its IPLT stub, not
                // on the resolver.  This has to apply to every relocation type
                // that can name the symbol (PLT32, PC32, 64), so it happens
                // once here rather than per arm.
                let s = match local_ifunc_slots.get(&(obj_idx, si)) {
                    Some(&slot) => slot,
                    None => s,
                };

                match rela.rela_type {
                    R_X86_64_64 => {
                        let mut deferred_to_loader = false;
                        let t = if !sym.name.is_empty() && !sym.is_local() {
                            if let Some(g) = globals_snap.get(sym.name.as_str()) {
                                if g.is_dynamic && !g.copy_reloc {
                                    if let Some(pi) = g.plt_idx {
                                        plt_addr + 16 + pi as u64 * 16
                                    } else {
                                        // Dynamic DATA symbol with no PLT: its
                                        // address is unknown until ld.so maps
                                        // the library, so a dynamic R_X86_64_64
                                        // was emitted into .rela.dyn for this
                                        // storage. Leave the bytes zero -- the
                                        // loader adds `symbol + addend`, and
                                        // pre-writing the addend here would make
                                        // ld.so's RELA (not REL) semantics
                                        // irrelevant while leaving a bogus
                                        // pointer if the reloc is ever skipped.
                                        deferred_to_loader = true;
                                        0
                                    }
                                } else {
                                    s
                                }
                            } else {
                                s
                            }
                        } else {
                            s
                        };
                        if deferred_to_loader {
                            w64(&mut out, fp, 0);
                        } else {
                            w64(&mut out, fp, (t as i64 + a) as u64);
                        }
                    }
                    R_X86_64_PC32 | R_X86_64_PLT32 => {
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
                        let v = t as i64 + a - p as i64;
                        if v > i32::MAX as i64 || v < i32::MIN as i64 {
                            return Err(reloc_truncated("R_X86_64_PC32", v, sym, obj_idx, objects));
                        }
                        w32(&mut out, fp, v as u32);
                    }
                    R_X86_64_32 => {
                        // Zero-extended 32-bit absolute: value must fit unsigned.
                        let v = s as i64 + a;
                        if v < 0 || v > u32::MAX as i64 {
                            return Err(reloc_truncated("R_X86_64_32", v, sym, obj_idx, objects));
                        }
                        w32(&mut out, fp, v as u32);
                    }
                    // 16- and 8-bit zero-extended absolutes. These go through the
                    // checked writers rather than a hand-rolled comparison so the
                    // range comes from the one table that also names the type:
                    // `field(R_X86_64_16)` is U16, so the check and the store
                    // cannot drift apart, and an unsigned 0xffff is accepted
                    // instead of being rejected by a signed 16-bit test.
                    R_X86_64_16 => {
                        w16_checked(
                            &mut out,
                            fp,
                            s as i64 + a,
                            rela.rela_type,
                            &sym.name,
                            &objects[obj_idx].source_name,
                        )?;
                    }
                    R_X86_64_8 => {
                        w8_checked(
                            &mut out,
                            fp,
                            s as i64 + a,
                            rela.rela_type,
                            &sym.name,
                            &objects[obj_idx].source_name,
                        )?;
                    }
                    R_X86_64_32S => {
                        // Sign-extended 32-bit absolute: value must fit signed.
                        let v = s as i64 + a;
                        if v > i32::MAX as i64 || v < i32::MIN as i64 {
                            return Err(reloc_truncated("R_X86_64_32S", v, sym, obj_idx, objects));
                        }
                        w32(&mut out, fp, v as u32);
                    }
                    R_X86_64_GOTTPOFF | R_X86_64_CODE_4_GOTTPOFF | R_X86_64_CODE_6_GOTTPOFF => {
                        // Initial Exec TLS via GOT: GOT entry contains TPOFF value
                        let mut resolved = false;
                        if !sym.name.is_empty() && !sym.is_local() {
                            if let Some(g) = globals_snap.get(sym.name.as_str()) {
                                if let Some(gi) = g.got_idx {
                                    let entry = &got_entries[gi];
                                    let gea = if entry.1 {
                                        got_plt_addr + 24 + g.plt_idx.unwrap_or(0) as u64 * 8
                                    } else {
                                        let nb = got_slot_ordinal[gi];
                                        got_addr + nb as u64 * 8
                                    };
                                    w32_checked(
                                        &mut out,
                                        fp,
                                        gea as i64 + a - p as i64,
                                        rela.rela_type,
                                        &sym.name,
                                        &objects[obj_idx].source_name,
                                    )?;
                                    resolved = true;
                                }
                            }
                        }
                        if !resolved {
                            // IE-to-LE rewrites a REX-prefixed movq/addq. REX2
                            // (CODE_4) and APX EVEX (CODE_6) keep a GOT slot
                            // when one exists; without a slot we refuse rather
                            // than corrupt the prefix.
                            if rela.rela_type != R_X86_64_GOTTPOFF {
                                return Err(format!(
                                    "GOTTPOFF IE-to-LE relaxation failed: APX/REX2 form of '{}' has no GOT slot",
                                    sym.name
                                ));
                            }
                            // IE-to-LE relaxation: convert GOT-indirect to immediate TPOFF.
                            //   movq  sym@GOTTPOFF(%rip), %reg  ->  movq $tpoff, %reg
                            //   addq  sym@GOTTPOFF(%rip), %reg  ->  addq $tpoff, %reg
                            // Encodings (fp points at the disp32):
                            //   REX 8b /r disp32  ->  REX' c7 (0xc0|reg) imm32
                            //   REX 03 /r disp32  ->  REX' 81 (0xc0|reg) imm32
                            // CRITICAL: the destination register moves from the
                            // ModRM.reg field to the ModRM.rm field, so the REX.R
                            // bit must be transplanted to REX.B. Without this,
                            // e.g. %r12 (REX.R + reg=100) silently becomes %rsp,
                            // corrupting the stack pointer (observed as glibc
                            // static-TLS crashes).
                            let tpoff = (s as i64 - tls_addr as i64) - tls_mem_size as i64;
                            let opc = if fp >= 2 { out[fp - 2] } else { 0 };
                            if fp >= 3 && fp + 4 <= out.len() && (opc == 0x8b || opc == 0x03) {
                                let modrm = out[fp - 1];
                                let reg = (modrm >> 3) & 7;
                                out[fp - 2] = if opc == 0x8b { 0xc7 } else { 0x81 };
                                out[fp - 1] = 0xc0 | reg;
                                let rex = out[fp - 3];
                                if (rex & 0xf0) == 0x40 {
                                    // Keep W and X; move R into B.
                                    out[fp - 3] = (rex & 0b1111_1010) | ((rex >> 2) & 1);
                                }
                                w32_checked(
                                    &mut out,
                                    fp,
                                    tpoff + a,
                                    rela.rela_type,
                                    &sym.name,
                                    &objects[obj_idx].source_name,
                                )?;
                            } else {
                                return Err(format!(
                                    "GOTTPOFF IE-to-LE relaxation failed: unrecognized instruction pattern at offset 0x{:x} for symbol '{}' (expected movq/addq GOT(%rip), %reg)",
                                    fp, sym.name
                                ));
                            }
                        }
                    }
                    R_X86_64_GOTPCREL
                    | R_X86_64_GOTPCRELX
                    | R_X86_64_REX_GOTPCRELX
                    | R_X86_64_CODE_4_GOTPCRELX
                    | R_X86_64_CODE_6_GOTPCRELX => {
                        if !sym.name.is_empty() && !sym.is_local() {
                            if let Some(g) = globals_snap.get(sym.name.as_str()) {
                                if let Some(gi) = g.got_idx {
                                    let entry = &got_entries[gi];
                                    let gea = if entry.1 {
                                        got_plt_addr + 24 + g.plt_idx.unwrap_or(0) as u64 * 8
                                    } else {
                                        let nb = got_slot_ordinal[gi];
                                        got_addr + nb as u64 * 8
                                    };
                                    if std::env::var("LCCC_DEBUG_GOT").is_ok() {
                                        let nb = got_slot_ordinal[gi];
                                        eprintln!(
                                            "[GOTREL] name={:?} gi={} is_plt={} nb={} gea=0x{:x} got_addr=0x{:x} p=0x{:x} addend={}",
                                            sym.name, gi, entry.1, nb, gea, got_addr, p, a
                                        );
                                    }
                                    w32_checked(
                                        &mut out,
                                        fp,
                                        gea as i64 + a - p as i64,
                                        rela.rela_type,
                                        &sym.name,
                                        &objects[obj_idx].source_name,
                                    )?;
                                    continue;
                                }
                                if is_gotpcrelx_relaxable(rela.rela_type) && g.defined_in.is_some()
                                {
                                    if fp >= 2 && fp < out.len() && out[fp - 2] == 0x8b {
                                        out[fp - 2] = 0x8d;
                                    }
                                    w32_checked(
                                        &mut out,
                                        fp,
                                        s as i64 + a - p as i64,
                                        rela.rela_type,
                                        &sym.name,
                                        &objects[obj_idx].source_name,
                                    )?;
                                    continue;
                                }
                            }
                        }
                        // No GOT slot exists for this symbol (typically a
                        // LOCAL asm label reached via sym@GOTPCREL). The
                        // instruction still DEREFERENCES its memory operand
                        // (`movq sym@GOTPCREL(%rip), %reg` loads the slot's
                        // CONTENTS), so pointing it straight at the symbol
                        // loads the bytes AT the symbol instead of its
                        // address — silent wrong code (an asm label's first
                        // instruction bytes masqueraded as a pointer). Do
                        // what GNU ld does: relax mov -> lea so the operand
                        // becomes an address computation. Any other opcode
                        // shape with a slotless GOT reference cannot be
                        // fixed up locally — fail loudly rather than emit a
                        // silently corrupt binary.
                        if fp >= 2 && fp < out.len() && out[fp - 2] == 0x8b {
                            out[fp - 2] = 0x8d;
                            w32_checked(
                                &mut out,
                                fp,
                                s as i64 + a - p as i64,
                                rela.rela_type,
                                &sym.name,
                                &objects[obj_idx].source_name,
                            )?;
                        } else {
                            return Err(format!(
                                "GOTPCREL against '{}' has no GOT entry and the \
                                 instruction is not a relaxable mov (opcode 0x{:02x}); \
                                 refusing to emit a load of the symbol's bytes",
                                sym.name,
                                if fp >= 2 { out[fp - 2] } else { 0 }
                            ));
                        }
                    }
                    R_X86_64_PC64 => {
                        w64(&mut out, fp, (s as i64 + a - p as i64) as u64);
                    }
                    R_X86_64_TPOFF32 => {
                        // Initial Exec TLS: value = (sym_addr - tls_addr) - tls_mem_size
                        // %fs:0 points past end of TLS block on x86-64
                        let tpoff = (s as i64 - tls_addr as i64) - tls_mem_size as i64;
                        w32_checked(
                            &mut out,
                            fp,
                            tpoff + a,
                            rela.rela_type,
                            &sym.name,
                            &objects[obj_idx].source_name,
                        )?;
                    }
                    R_X86_64_TLSGD => {
                        // General-Dynamic TLS in an executable. Canonical sequence:
                        //   66 48 8d 3d <disp32>      data16 lea sym@tlsgd(%rip),%rdi
                        //   66 66 48 e8 <disp32>      data16 data16 rex.W call __tls_get_addr
                        // fp points at the lea's disp32, so the sequence spans
                        // [fp-4, fp+12) = 16 bytes.
                        let seq = fp.checked_sub(4).filter(|&st| {
                            st + 16 <= out.len()
                                && out[st] == 0x66
                                && out[st + 1] == 0x48
                                && out[st + 2] == 0x8d
                                && out[st + 3] == 0x3d
                                && out[fp + 4] == 0x66
                                && out[fp + 5] == 0x66
                                && out[fp + 6] == 0x48
                                && out[fp + 7] == 0xe8
                        });
                        let Some(st) = seq else {
                            return Err(format!(
                                "TLSGD relaxation failed: unrecognized code sequence for '{}' in {}",
                                sym.name, objects[obj_idx].source_name
                            ));
                        };
                        let is_dyn_tls = globals_snap
                            .get(sym.name.as_str())
                            .map(|g| g.is_dynamic)
                            .unwrap_or(false);
                        if !is_dyn_tls {
                            // GD -> LE:  mov %fs:0,%rax ; lea tpoff(%rax),%rax
                            let tpoff = (s as i64 - tls_addr as i64) - tls_mem_size as i64;
                            out[st..st + 9]
                                .copy_from_slice(&[0x64, 0x48, 0x8b, 0x04, 0x25, 0, 0, 0, 0]);
                            out[st + 9] = 0x48;
                            out[st + 10] = 0x8d;
                            out[st + 11] = 0x80;
                            w32_checked(
                                &mut out,
                                st + 12,
                                tpoff,
                                rela.rela_type,
                                &sym.name,
                                &objects[obj_idx].source_name,
                            )?;
                        } else {
                            // GD -> IE:  mov %fs:0,%rax ; add got(%rip),%rax
                            // Requires a GOT slot with an R_X86_64_TPOFF64 dynamic
                            // relocation (created by the got_entries scan).
                            let gi = globals_snap.get(sym.name.as_str()).and_then(|g| g.got_idx);
                            let Some(gi) = gi else {
                                return Err(format!(
                                    "TLSGD->IE: no GOT entry for dynamic TLS symbol '{}'",
                                    sym.name
                                ));
                            };
                            let nb = got_slot_ordinal[gi];
                            let gea = got_addr + nb as u64 * 8;
                            let seq_addr = sa + sec_off + rela.offset - 4;
                            out[st..st + 9]
                                .copy_from_slice(&[0x64, 0x48, 0x8b, 0x04, 0x25, 0, 0, 0, 0]);
                            out[st + 9] = 0x48;
                            out[st + 10] = 0x03;
                            out[st + 11] = 0x05;
                            w32_checked(
                                &mut out,
                                st + 12,
                                gea as i64 - (seq_addr as i64 + 16),
                                rela.rela_type,
                                &sym.name,
                                &objects[obj_idx].source_name,
                            )?;
                        }
                        tls_consumed.push(fp + 4, fp + 12);
                    }
                    R_X86_64_TLSLD => {
                        // Local-Dynamic TLS in an executable -> LE. Sequence:
                        //   48 8d 3d <disp32>    lea sym@tlsld(%rip),%rdi
                        //   e8 <disp32>          call __tls_get_addr
                        // spans [fp-3, fp+9) = 12 bytes. Replaced by a 12-byte
                        //   66 66 66 64 48 8b 04 25 00 00 00 00   mov %fs:0,%rax
                        // after which DTPOFF32 values are TP-relative.
                        let seq = fp.checked_sub(3).filter(|&st| {
                            st + 12 <= out.len()
                                && out[st] == 0x48
                                && out[st + 1] == 0x8d
                                && out[st + 2] == 0x3d
                                && out[fp + 4] == 0xe8
                        });
                        let Some(st) = seq else {
                            return Err(format!(
                                "TLSLD relaxation failed: unrecognized code sequence in {}",
                                objects[obj_idx].source_name
                            ));
                        };
                        out[st..st + 12].copy_from_slice(&[
                            0x66, 0x66, 0x66, 0x64, 0x48, 0x8b, 0x04, 0x25, 0, 0, 0, 0,
                        ]);
                        tls_consumed.push(fp + 4, fp + 9);
                    }
                    R_X86_64_DTPOFF32 => {
                        // After LD->LE relaxation %rax holds TP, so DTPOFF becomes
                        // a TP-relative offset (same formula as TPOFF32).
                        let tpoff = (s as i64 - tls_addr as i64) - tls_mem_size as i64;
                        w32_checked(
                            &mut out,
                            fp,
                            tpoff + a,
                            rela.rela_type,
                            &sym.name,
                            &objects[obj_idx].source_name,
                        )?;
                    }
                    R_X86_64_DTPOFF64 => {
                        let tpoff = (s as i64 - tls_addr as i64) - tls_mem_size as i64;
                        w64(&mut out, fp, (tpoff + a) as u64);
                    }
                    R_X86_64_GOTPC32_TLSDESC
                    | R_X86_64_CODE_4_GOTPC32_TLSDESC
                    | R_X86_64_CODE_6_GOTPC32_TLSDESC => {
                        // TLSDESC -> LE relaxation:
                        //   [REX|REX2|EVEX] 8d 05 <disp32>  lea sym@tlsdesc(%rip),%rax
                        // becomes
                        //   [same prefix]   c7 c0 <tpoff32> mov $tpoff,%rax
                        if fp >= 2 && out[fp - 2] == 0x8d && out[fp - 1] == 0x05 {
                            let tpoff = (s as i64 - tls_addr as i64) - tls_mem_size as i64;
                            out[fp - 2] = 0xc7;
                            out[fp - 1] = 0xc0;
                            w32_checked(
                                &mut out,
                                fp,
                                tpoff,
                                rela.rela_type,
                                &sym.name,
                                &objects[obj_idx].source_name,
                            )?;
                        } else {
                            return Err(format!(
                                "TLSDESC relaxation failed: unrecognized sequence for '{}'",
                                sym.name
                            ));
                        }
                    }
                    R_X86_64_TLSDESC_CALL => {
                        // call *(%rax) [ff 10] -> xchg %ax,%ax [66 90] after LE relax
                        if fp + 2 <= out.len() && out[fp] == 0xff && out[fp + 1] == 0x10 {
                            out[fp] = 0x66;
                            out[fp + 1] = 0x90;
                        }
                    }
                    R_X86_64_NONE => {}
                    other => {
                        return Err(format!(
                            "unsupported x86-64 relocation type {} for '{}' in {}",
                            other, sym.name, objects[obj_idx].source_name
                        ));
                    }
                }
            }
        }
    }

    // Build .eh_frame_hdr from the RELOCATED .eh_frame bytes (initial_location
    // fields are only meaningful after R_X86_64_PC32 application).
    if eh_frame_hdr_size > 0 {
        if let Some(ef) = output_sections
            .iter()
            .find(|s| s.name == ".eh_frame" && s.mem_size > 0)
        {
            let ef_start = ef.file_offset as usize;
            let ef_end = ef_start + ef.mem_size as usize;
            if ef_end <= out.len() {
                // Build the header from a *borrow* of the already-relocated
                // .eh_frame bytes. Copying the section first (`to_vec()`) was
                // only ever needed to satisfy the borrow checker before the
                // `write_bytes` below, but .eh_frame is large -- 400 KB on a
                // 20 000-function link -- so that copy was pure waste. The
                // immutable borrow ends when `hdr` is produced, which is
                // before `out` is borrowed mutably, so NLL accepts this.
                let hdr = linker_common::build_eh_frame_hdr(
                    &out[ef_start..ef_end],
                    ef.addr,
                    eh_frame_hdr_vaddr,
                    true,
                );
                if !hdr.is_empty() && eh_frame_hdr_offset as usize + hdr.len() <= out.len() {
                    write_bytes(&mut out, eh_frame_hdr_offset as usize, &hdr);
                }
            }
        }
    }

    zone!("relocs");
    // === Append section headers ===
    // Build .shstrtab string table
    let mut shstrtab = vec![0u8]; // null byte at offset 0
    let mut shstr_offsets: FxHashMap<String, u32> = FxHashMap::default();
    let known_names = [
        ".eh_frame_hdr",
        ".interp",
        ".gnu.hash",
        ".hash",
        ".dynsym",
        ".dynstr",
        ".gnu.version",
        ".gnu.version_r",
        ".rela.dyn",
        ".rela.plt",
        ".plt",
        ".dynamic",
        ".got",
        ".got.plt",
        ".init_array",
        ".fini_array",
        ".preinit_array",
        ".tdata",
        ".tbss",
        ".bss",
        ".shstrtab",
        ".iplt",
        ".rela.iplt",
        ".symtab",
        ".strtab",
        ".comment",
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

    // Use shared write_elf64_shdr from linker_common (aliased locally for brevity)
    let write_shdr = linker_common::write_elf64_shdr;

    // Continue the count from the single header walk above.  This used to
    // re-derive the same five conditionals a second time, and the two copies
    // had to be kept in step by hand -- which is how `.dynsym`'s index ended up
    // hardcoded as 3 and broke when `--hash-style=sysv` swapped `.gnu.hash` for
    // `.hash`.  There is now exactly one place that numbers headers.
    let mut sh_count: u16 = linker_hdr_count;
    // Merged output sections (non-BSS, non-TLS, non-init/fini)
    for sec in output_sections.iter() {
        if sec.flags & SHF_ALLOC != 0
            && sec.sh_type != SHT_NOBITS
            && sec.flags & SHF_TLS == 0
            && sec.name != ".init_array"
            && sec.name != ".fini_array"
            && sec.name != ".preinit_array"
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
    if has_preinit_array {
        sh_count += 1;
    }
    if !is_static {
        sh_count += 1;
    } // .dynamic
    if got_size > 0 {
        sh_count += 1;
    } // .got (needed for static too: TLS, GOTPCREL)
    if !is_static {
        sh_count += 1;
    } // .got.plt
    if iplt_total_size > 0 {
        sh_count += 1;
    } // .iplt
    if rela_iplt_size > 0 {
        sh_count += 1;
    } // .rela.iplt
    // BSS sections (non-TLS)
    for sec in output_sections.iter() {
        if sec.sh_type == SHT_NOBITS && sec.flags & SHF_ALLOC != 0 && sec.flags & SHF_TLS == 0 {
            sh_count += 1;
        }
    }
    // .comment sits before .symtab, as in bfd.
    if !comment_data.is_empty() {
        sh_count += 1;
    }
    sh_count += if emit_symtab { 2 } else { 0 }; // .symtab + .strtab
    let shstrtab_shidx = sh_count; // .shstrtab is the last section
    sh_count += 1;

    // .comment data first (alignment 1, no padding needed), then the
    // 8-aligned .symtab + .strtab data (before .shstrtab).
    let comment_data_offset = out.len() as u64;
    out.extend_from_slice(&comment_data);
    while out.len() % 8 != 0 {
        out.push(0);
    }
    let symtab_data_offset = out.len() as u64;
    let symtab_data_size = (symtab_entries.len() * 24) as u64;
    for e in &symtab_entries {
        out.extend_from_slice(e);
    }
    let strtab_data_offset = out.len() as u64;
    out.extend_from_slice(&symtab_names);
    // When stripping, both are empty and no headers point at them, so the
    // alignment padding above is the only residue (at most 7 bytes).

    // Align and append .shstrtab data
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
    write_shdr(&mut out, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0);
    // Dynamic linking section headers (skipped for static executables)
    if !is_static {
        // .interp
        write_shdr(
            &mut out,
            get_shname(".interp"),
            SHT_PROGBITS,
            SHF_ALLOC,
            interp_addr,
            interp_offset,
            interp.len() as u64,
            0,
            0,
            1,
            0,
        );
        // .gnu.hash
        if want_gnu_hash {
            write_shdr(
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
        }
        // .hash (SysV).  sh_link points at .dynsym and sh_info is the index of
        // the first *global* symbol, exactly as for SHT_DYNSYM.
        if want_sysv_hash {
            write_shdr(
                &mut out,
                get_shname(".hash"),
                SHT_HASH,
                SHF_ALLOC,
                sysv_hash_addr,
                sysv_hash_offset,
                sysv_hash_size,
                dynsym_shidx,
                // sh_info is unspecified for SHT_HASH by the ELF spec; GNU ld
                // and every loader in the wild use 0, and readelf warns on
                // anything else.  (The "first global symbol index" reading of
                // this field belongs to SHT_DYNSYM, not here.)
                0,
                4,
                0,
            );
        }
        // .dynsym
        write_shdr(
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
        write_shdr(
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
        // .gnu.version (versym)
        if verneed_size > 0 {
            write_shdr(
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
        // .gnu.version_r (verneed)
        if verneed_size > 0 {
            write_shdr(
                &mut out,
                get_shname(".gnu.version_r"),
                SHT_GNU_VERNEED,
                SHF_ALLOC,
                verneed_addr,
                verneed_offset,
                verneed_size,
                dynstr_shidx,
                verneed_count,
                4,
                0,
            );
        }
        // .rela.dyn
        if rela_dyn_size > 0 {
            write_shdr(
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
            write_shdr(
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
            write_shdr(
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
    }
    // .eh_frame_hdr (before the merged sections; matches the count walks above)
    if eh_frame_hdr_size > 0 {
        write_shdr(
            &mut out,
            get_shname(".eh_frame_hdr"),
            SHT_PROGBITS,
            SHF_ALLOC,
            eh_frame_hdr_vaddr,
            eh_frame_hdr_offset,
            eh_frame_hdr_size,
            0,
            0,
            4,
            0,
        );
    }
    // Merged output sections (text/rodata/data, excluding BSS/TLS/init_array/fini_array)
    for sec in output_sections.iter() {
        if sec.flags & SHF_ALLOC != 0
            && sec.sh_type != SHT_NOBITS
            && sec.flags & SHF_TLS == 0
            && sec.name != ".init_array"
            && sec.name != ".fini_array"
            && sec.name != ".preinit_array"
        {
            write_shdr(
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
            write_shdr(
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
            write_shdr(
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
            write_shdr(
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
            write_shdr(
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
    // .preinit_array
    if has_preinit_array {
        if let Some(pa_sec) = output_sections.iter().find(|s| s.name == ".preinit_array") {
            write_shdr(
                &mut out,
                get_shname(".preinit_array"),
                SHT_PREINIT_ARRAY,
                SHF_ALLOC | SHF_WRITE,
                preinit_array_addr,
                pa_sec.file_offset,
                preinit_array_size,
                0,
                0,
                8,
                8,
            );
        }
    }
    if !is_static {
        // .dynamic
        write_shdr(
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
    }
    // .got (needed for both static and dynamic: TLS GOTTPOFF, GOTPCREL)
    if got_size > 0 {
        write_shdr(
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
    if !is_static {
        // .got.plt
        write_shdr(
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
    // .iplt (IFUNC PLT for static linking)
    if iplt_total_size > 0 {
        write_shdr(
            &mut out,
            get_shname(".iplt"),
            SHT_PROGBITS,
            SHF_ALLOC | SHF_EXECINSTR,
            iplt_addr,
            iplt_offset,
            iplt_total_size,
            0,
            0,
            16,
            16,
        );
    }
    // .rela.iplt (IRELATIVE relocations for static linking)
    if rela_iplt_size > 0 {
        write_shdr(
            &mut out,
            get_shname(".rela.iplt"),
            SHT_RELA,
            SHF_ALLOC,
            rela_iplt_addr,
            rela_iplt_offset,
            rela_iplt_size,
            0,
            0,
            8,
            24,
        );
    }
    // BSS sections (non-TLS)
    for sec in output_sections.iter() {
        if sec.sh_type == SHT_NOBITS && sec.flags & SHF_ALLOC != 0 && sec.flags & SHF_TLS == 0 {
            write_shdr(
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
    // .comment (merged compiler version strings; non-alloc, addr 0).
    if !comment_data.is_empty() {
        write_shdr(
            &mut out,
            get_shname(".comment"),
            SHT_PROGBITS,
            SHF_MERGE | SHF_STRINGS,
            0,
            comment_data_offset,
            comment_data.len() as u64,
            0,
            0,
            1,
            1,
        );
    }
    if emit_symtab {
        // .symtab (defined symbols for profilers/debuggers)
        write_shdr(
            &mut out,
            get_shname(".symtab"),
            SHT_SYMTAB,
            0,
            0,
            symtab_data_offset,
            symtab_data_size,
            strtab_shidx as u32,
            n_local as u32,
            8,
            24,
        );
        // .strtab
        write_shdr(
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
    }
    // .shstrtab (last section)
    write_shdr(
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
    // e_shoff at offset 40 (8 bytes)
    out[40..48].copy_from_slice(&shdr_offset.to_le_bytes());
    // e_shnum at offset 60 (2 bytes)
    out[60..62].copy_from_slice(&sh_count.to_le_bytes());
    // e_shstrndx at offset 62 (2 bytes)
    out[62..64].copy_from_slice(&shstrtab_shidx.to_le_bytes());

    zone!("headers");

    // === -Map=FILE ===
    // Written after layout, so every address in the map is the final one.
    // Built from the same `output_sections` / `section_map` state the ELF
    // itself was emitted from, which makes the map authoritative rather than
    // a reconstruction that can drift from reality.
    if let Some(mp) = map_path {
        let object_names: Vec<String> = objects.iter().map(|o| o.source_name.clone()).collect();

        // (name, object_idx, section_idx, offset-within-input-section)
        let mut map_syms: Vec<(String, usize, usize, u64)> = Vec::new();
        for (obj_idx, obj) in objects.iter().enumerate() {
            for sym in &obj.symbols {
                if sym.name.is_empty() {
                    continue;
                }
                let st = sym.sym_type();
                // STT_SECTION (3) and STT_FILE (4) are bookkeeping entries,
                // not addresses a map reader cares about.
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

        let lm = linker_common::build_link_map(
            output_sections,
            &object_names,
            &map_syms,
            entry_symbol.or(Some("_start")),
            entry_addr,
        );
        lm.write_to_path(std::path::Path::new(mp))
            .map_err(|e| format!("failed to write map file '{}': {}", mp, e))?;
    }

    // === .note.gnu.build-id ===
    // Last thing before the write, because the digest covers the whole image
    // (section headers included) with the descriptor field zeroed.  The note
    // was laid out from the synthetic input object `link_builtin` appends when
    // --build-id was requested; find where it landed and fill it in.
    let build_id_offset = output_sections
        .iter()
        .find(|s| s.name == ".note.gnu.build-id" && s.mem_size > 0)
        .map(|s| s.file_offset);
    linker_common::build_id::patch_output_build_id(&mut out, build_id_offset);

    std::fs::write(output_path, &out)
        .map_err(|e| format!("failed to write '{}': {}", output_path, e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(output_path, std::fs::Permissions::from_mode(0o755));
    }
    Ok(())
}

pub(super) fn resolve_sym(
    obj_idx: usize,
    sym: &Symbol,
    globals: &FxHashMap<String, GlobalSymbol>,
    section_map: &FxHashMap<(usize, usize), (usize, u64)>,
    output_sections: &[OutputSection],
    plt_addr: u64,
) -> u64 {
    if sym.sym_type() == STT_SECTION {
        let si = sym.shndx as usize;
        return section_map
            .get(&(obj_idx, si))
            .map(|&(oi, so)| output_sections[oi].addr + so)
            .unwrap_or(0);
    }
    // Local (STB_LOCAL) symbols must NOT be resolved via globals, since a
    // local symbol named e.g. "opts" must not be confused with a global "opts"
    // from another object file.
    if !sym.name.is_empty() && !sym.is_local() {
        if let Some(g) = globals.get(sym.name.as_str()) {
            if g.defined_in.is_some() {
                return g.value;
            }
            if g.is_dynamic {
                return g
                    .plt_idx
                    .map(|pi| plt_addr + 16 + pi as u64 * 16)
                    .unwrap_or(0);
            }
        }
        if sym.is_weak() {
            return 0;
        }
    }
    if sym.is_undefined() {
        return 0;
    }
    if sym.shndx == SHN_ABS {
        return sym.value;
    }
    section_map
        .get(&(obj_idx, sym.shndx as usize))
        .map(|&(oi, so)| output_sections[oi].addr + so + sym.value)
        .unwrap_or(sym.value)
}

/// Format a "relocation truncated to fit" error in the GNU ld style,
/// with the referencing object file for actionable diagnostics.
pub(super) fn reloc_truncated(
    rtype: &str,
    value: i64,
    sym: &Symbol,
    obj_idx: usize,
    objects: &[ElfObject],
) -> String {
    let target = if sym.name.is_empty() {
        "<local>"
    } else {
        sym.name.as_str()
    };
    format!(
        "relocation truncated to fit: {} against symbol '{}' in {} (value 0x{:x} out of range); \
         recompile with -mcmodel=large or -fpic if the image exceeds 2 GiB",
        rtype, target, objects[obj_idx].source_name, value
    )
}
