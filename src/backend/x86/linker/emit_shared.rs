//! Shared library (.so) emission for the x86-64 linker.
//!
//! Emits an ELF64 shared library (ET_DYN) with PIC relocations, PLT stubs,
//! `.dynamic` section, and GNU hash tables.

use crate::backend::elf::{STT_NOTYPE, STV_DEFAULT, STV_PROTECTED, push_strtab_name};
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use std::collections::BTreeSet;

use super::elf::*;
use super::emit_exec::resolve_sym;
use super::plt_got::local_got_target;
use super::reloc_field::{self, w8_checked, w16_checked, w32_checked};
use super::types::{GlobalSymbol, LocalSlots, PAGE_SIZE};
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

/// Emit one GNU ELF64 `Verneed` node and its `Vernaux` entries.
///
/// Undefined references in a shared object carry a version index in
/// `.gnu.version`; the index is resolved through `.gnu.version_r`, not the
/// provider-side `.gnu.version_d`. Omitting this table makes glibc's internal
/// `*_rtld_global*@GLIBC_PRIVATE` references look unversioned at load time and
/// fails before `main` with a misleading undefined-symbol diagnostic.
///
/// `last` terminates the Verneed chain. Every other node's `vn_next` is the
/// byte distance to the following node, which (Vernaux records being laid out
/// directly behind their Verneed) is `16 + 16 * versions.len()`. Writing 0 for
/// every node -- as this emitter once did -- truncates the chain after the
/// first provider: `readelf -V` warns "Invalid vn_next field of 0", ld.so
/// never sees the later providers' versions, and GNU ld refuses to link
/// against the library (`_Unwind_Resume: invalid needed version 3` for a C++
/// DSO that needs libc.so.6, libgcc_s.so.1 and libstdc++.so.6).
fn push_verneed_entry(
    buf: &mut Vec<u8>,
    soname_off: usize,
    versions: &[(String, u16, usize)],
    last: bool,
) {
    let start = buf.len();
    buf.resize(start + 16, 0);
    w16(buf, start, 1); // vn_version
    w16(buf, start + 2, versions.len() as u16);
    w32(buf, start + 4, soname_off as u32);
    w32(buf, start + 8, 16); // vn_aux: first Vernaux
    let next = if last { 0 } else { 16 + 16 * versions.len() };
    w32(buf, start + 12, next as u32);
    for (idx, (version, other, name_off)) in versions.iter().enumerate() {
        let aux = buf.len();
        buf.resize(aux + 16, 0);
        w32(buf, aux, elf_hash_name(version));
        w16(buf, aux + 4, 0); // vna_flags
        w16(buf, aux + 6, *other);
        w32(buf, aux + 8, *name_off as u32);
        w32(
            buf,
            aux + 12,
            if idx + 1 == versions.len() { 0 } else { 16 },
        );
    }
}

/// Linkage symbols GNU ld defines with hidden visibility: they name this
/// image's own GOT and dynamic section and are never published in `.dynsym`
/// (a reference from another module would bind to the wrong image's table).
/// GNU ld's diagnostic for a non-PIC relocation in a shared object.
fn non_pic_reloc_error(obj_name: &str, rela_type: u32, sym_name: &str) -> String {
    format!(
        "{obj_name}: relocation {} against '{sym_name}' can not be used when making a shared \
         object; recompile with -fPIC",
        reloc_field::name(rela_type).unwrap_or("of this type")
    )
}

/// Hand a PC-relative field whose value depends on the load address to
/// ld.so, as GNU ld does: in writable storage it becomes a dynamic
/// R_X86_64_PC32/PC64 -- against the symbol when ld.so may bind it
/// elsewhere, else against symbol 0 with the absolute target folded into
/// the addend.  In read-only storage that would be a text relocation, which
/// this linker does not produce (nor does GNU ld for x86-64 PC32 in a shared
/// object): the object was compiled without -fPIC.
#[allow(clippy::too_many_arguments)]
fn pc_rel_to_loader(
    entries: &mut Vec<(u64, u32, String, i64)>,
    sec_flags: u64,
    rela_type: u32,
    p: u64,
    sym: &Symbol,
    symbolic: bool,
    s: u64,
    a: i64,
    obj_name: &str,
) -> Result<(), String> {
    if sec_flags & SHF_WRITE == 0 {
        return Err(non_pic_reloc_error(obj_name, rela_type, &sym.name));
    }
    if symbolic {
        entries.push((p, rela_type, sym.name.to_string(), a));
    } else {
        entries.push((p, rela_type, String::new(), s as i64 + a));
    }
    Ok(())
}

fn is_hidden_linkage_symbol(name: &str) -> bool {
    matches!(name, "_GLOBAL_OFFSET_TABLE_" | "_DYNAMIC")
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
    hash_style: crate::backend::linker_common::HashStyle,
    section_map: &FxHashMap<(usize, usize), (usize, u64)>,
    needed_sonames: &[String],
    output_path: &str,
    soname: Option<String>,
    rpath_entries: &[String],
    use_runpath: bool,
    version_script_path: Option<&str>,
    symbolic: crate::backend::linker_common::Symbolic,
    // `(DT_FLAGS, DT_FLAGS_1)` bits the options request
    // (`LinkerArgs::requested_dyn_flags`); DF_SYMBOLIC is added here.
    requested_dyn_flags: (u64, u64),
    // `-z relro` (the default) / `-z norelro`.
    z_relro: bool,
    // `-z now`: the lazy-binding `.got.plt` joins the RELRO window.
    z_now: bool,
    // `--exclude-libs`: archives whose symbols must not be re-exported.
    exclude_libs: &[String],
    // `-Map=FILE`: write a GNU-ld-compatible link map. Previously reachable
    // only for executables, because link_shared's private argument parser
    // never recognised -Map at all.
    map_path: Option<&str>,
    // `--defsym` expressions deferred from `apply_defsyms`; evaluated here
    // once section addresses and the linker-provided symbols are final, so the
    // shared path and the executable path agree (see `link::evaluate_pending_defsyms`).
    pending_defsyms: &[(String, String, usize)],
) -> Result<(), String> {
    let base_addr: u64 = 0;
    // Option-derived dynamic flags, fixed up front: the `.dynamic` sizing
    // and the writer below read these same three values, so the entry count
    // and the bytes cannot disagree.
    let dt_symbolic = symbolic == crate::backend::linker_common::Symbolic::All;
    let (dt_flags, dt_flags_1) = {
        let (mut f, f1) = requested_dyn_flags;
        if dt_symbolic {
            f |= crate::backend::linker_common::dyn_flags::DF_SYMBOLIC;
        }
        (f, f1)
    };

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
        ($off:expr_2021) => {
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

    // One definition of "exported" and "preemptible" serves every decision
    // below: `.dynsym` membership, PLT routing, the relocation a GOT slot
    // gets, and the flavour of an `R_X86_64_64`.  They used to be re-derived
    // at each site from different inputs -- the PLT scan read the
    // *referencing* object's `st_other`, the export filter read no
    // visibility at all, and the GOT pass treated every local definition as
    // non-preemptible -- so hidden symbols were exported while exported data
    // was bound with `R_X86_64_RELATIVE`, which splits a copy-relocated
    // variable in two (the executable reads its copy, this library writes
    // the original).
    //
    // exported:    a definition in this output that `.dynsym` publishes --
    //              not HIDDEN/INTERNAL (merged gABI visibility), not
    //              version-script local, not from an `--exclude-libs`
    //              archive, and not one of the linkage symbols GNU ld
    //              defines hidden (`_GLOBAL_OFFSET_TABLE_`, `_DYNAMIC`).
    // preemptible: exported with DEFAULT visibility and no `-Bsymbolic`:
    //              ld.so may bind it to another module's definition, so
    //              every reference from this library must go through a
    //              symbolic dynamic relocation (or the PLT).
    let is_version_local = |name: &str| {
        version_script
            .as_ref()
            .is_some_and(|vs| vs.any_local_star() && !vs.matches_global(&sym_base(name)))
    };
    let exports_def = |name: &str, g: &GlobalSymbol| -> bool {
        g.defined_in.is_some()
            && !g.is_dynamic
            && (g.info >> 4) != STB_LOCAL
            && g.section_idx != SHN_UNDEF
            && linker_common::is_dynamic_visibility(g.visibility)
            && !is_hidden_linkage_symbol(name)
            && !is_version_local(name)
            && !excluded_syms.contains(name)
    };
    // Symbols this emitter defines itself once the layout is known
    // (`_end`, `__bss_start`, `__start_SEC`, ...; see "Define
    // linker-provided symbols" below).  They are ADDRESSES in this image even
    // though they are recorded with SHN_ABS and no defining object: GOT
    // slots and R_X86_64_64 fields holding them need R_X86_64_RELATIVE, and
    // their `.dynsym` entries need a real section index -- an SHN_ABS entry
    // is not relocated by ld.so.  They bind locally (GNU ld gives
    // `__start_`/`__stop_` protected visibility), so they are never
    // preemptible.  Only names still undefined here are provided.
    let linker_provided: FxHashSet<String> =
        get_standard_linker_symbols(&LinkerSymbolAddresses::default())
            .iter()
            .map(|s| s.name.to_string())
            .chain(
                linker_common::resolve_start_stop_symbols(output_sections)
                    .into_iter()
                    .map(|(n, _)| n),
            )
            .filter(|n| {
                globals
                    .get(n)
                    .is_none_or(|g| g.defined_in.is_none() && !g.is_dynamic)
            })
            .collect();
    let preemptible_def = |name: &str, g: &GlobalSymbol| -> bool {
        exports_def(name, g)
            && g.visibility == STV_DEFAULT
            && !symbolic.binds_locally(g.info & 0xf)
            && !linker_provided.contains(name)
    };
    // An undefined, non-DSO symbol is imported through `.dynsym` unless its
    // merged visibility is HIDDEN/INTERNAL: a hidden undefined weak symbol
    // resolves to 0 inside this component and must never become a dynamic
    // reference (it could be satisfied by some other module's definition).
    let imports_undef = |g: &GlobalSymbol| -> bool {
        g.is_dynamic || linker_common::is_dynamic_visibility(g.visibility)
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
                    R_X86_64_PLT32 | R_X86_64_PC32 | R_X86_64_PLTOFF64 => {
                        if let Some(gsym) = globals.get(sym.name.as_str()) {
                            let locally_defined = gsym.defined_in.is_some() && !gsym.is_dynamic;
                            // External references always need a stub.
                            let external = gsym.is_dynamic
                                || (gsym.defined_in.is_none()
                                    && gsym.section_idx == SHN_UNDEF
                                    && imports_undef(gsym));
                            // GNU semantics: calls to our own EXPORTED functions
                            // also route through the PLT so LD_PRELOAD /
                            // earlier-DSO interposition works. Direct binding
                            // for everything `preemptible_def` rejects: merged
                            // HIDDEN/PROTECTED/INTERNAL visibility, version-
                            // script locals, --exclude-libs, -Bsymbolic.
                            let is_func = (gsym.info & 0xf) == STT_FUNC
                                || sym.sym_type() == STT_FUNC
                                || matches!(rela.rela_type, R_X86_64_PLT32 | R_X86_64_PLTOFF64);
                            let interposable = locally_defined
                                && is_func
                                && preemptible_def(sym.name.as_str(), gsym);
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
                    canonical_plt: false,
                    visibility: 0,
                    lib_sym_value: 0,
                    version: None,
                    plt_idx: None,
                    got_idx: None,
                    absolute: false,
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

    // IFUNCs bound inside this library: STB_LOCAL ones, and global ones
    // ld.so cannot rebind (hidden/protected, version-script local,
    // -Bsymbolic).  A preemptible IFUNC needs nothing special -- its
    // symbolic JUMP_SLOT/GLOB_DAT/R_X86_64_64 makes ld.so run the resolver.
    // A bound one has no link-time address at all: its symbol value is the
    // RESOLVER.  Each referenced one gets an IPLT entry, `jmp *slot(%rip)`
    // through a `.got.plt` slot that an R_X86_64_IRELATIVE (appended to
    // `.rela.plt`, after the JUMP_SLOTs, where GNU ld puts them, so the
    // resolver runs once the rest of the library is relocated) fills with
    // the resolver's answer.  That entry then IS the function's address for
    // every reference in the library -- calls, `lea`, GOT slots, pointers in
    // data -- so function-pointer equality holds, and GOT-indirect
    // references relax to it like to any local function.  This emitter
    // used to refuse such references outright.
    let bound_ifunc_global = |name: &str, g: &GlobalSymbol| -> bool {
        (g.info & 0xf) == STT_GNU_IFUNC
            && g.defined_in.is_some()
            && !g.is_dynamic
            && g.section_idx != SHN_UNDEF
            && !preemptible_def(name, g)
    };
    let mut iplt_globals: Vec<String> = Vec::new();
    let mut iplt_locals: Vec<(usize, usize)> = Vec::new();
    {
        let mut seen_globals: FxHashSet<&str> = FxHashSet::default();
        let mut seen_locals: FxHashSet<(usize, usize)> = FxHashSet::default();
        for (obj_idx, obj) in objects.iter().enumerate() {
            for (sec_idx, sec_relas) in obj.relocations.iter().enumerate() {
                // References from unloaded storage (DWARF) need no entry.
                if obj.sections[sec_idx].flags & SHF_ALLOC == 0 {
                    continue;
                }
                for rela in sec_relas {
                    let si = rela.sym_idx as usize;
                    let Some(sym) = obj.symbols.get(si) else {
                        continue;
                    };
                    if rela.rela_type == R_X86_64_NONE {
                        continue;
                    }
                    if sym.is_local() {
                        if sym.sym_type() == STT_GNU_IFUNC
                            && sym.shndx != SHN_UNDEF
                            && seen_locals.insert((obj_idx, si))
                        {
                            iplt_locals.push((obj_idx, si));
                        }
                    } else if !sym.name.is_empty()
                        && globals
                            .get(sym.name.as_str())
                            .is_some_and(|g| bound_ifunc_global(&sym.name, g))
                        && seen_globals.insert(sym.name.as_str())
                    {
                        iplt_globals.push(sym.name.to_string());
                    }
                }
            }
        }
    }
    let n_iplt = iplt_globals.len() + iplt_locals.len();

    // A GOT-indirect reference to a non-preemptible definition can address
    // it directly (`elf::gotpcrelx_relaxation` decides for the instruction)
    // -- a bound IFUNC included, whose address is its IPLT entry (every one
    // referenced from loaded storage has one, see above).  PIC rules: an
    // address in this object only through `lea`, `addr32 call` and `jmp`;
    // an absolute value (`GlobalSymbol::absolute`) only through the
    // immediate forms.  A linker-created absolute (`--defsym`) keeps its
    // slot: an expression's value is computed after layout, and planner and
    // applier must decide on the same value (see `exec_got_target`).  The
    // scan below and the relocation pass ask the same question of the same
    // original bytes, so every reference left unrelaxed has a slot.
    let got_target = |name: &str, g: &GlobalSymbol| -> Option<GotTarget> {
        if g.defined_in.is_none()
            || g.is_dynamic
            || g.section_idx == SHN_UNDEF
            || preemptible_def(name, g)
        {
            return None;
        }
        if !g.absolute {
            return Some(GotTarget::Image);
        }
        (g.defined_in != Some(usize::MAX)).then_some(GotTarget::Absolute(g.value))
    };

    // GOT slots.  Global symbols are keyed by name (one slot per symbol, and
    // their dynamic relocations name the `.dynsym` entry); LOCAL symbols by
    // (object, symbol index): two translation units may each have a
    // `static __thread int x`, and keying those by name -- as this emitter
    // once did for TLSGD -- hands both the same slot.
    //   got_needed_names  GOTPCREL* (non-relaxed) and GOTTPOFF of globals
    //   tlsgd_names       TLSGD pairs of globals
    //   tlsdesc_names     TLSDESC pairs of globals
    //   local_gd / local_desc / local_ie / local_got: the same for locals
    let mut got_needed_names: Vec<String> = Vec::new();
    let mut got_needed_seen: FxHashSet<String> = FxHashSet::default();
    let mut tlsgd_seen: FxHashSet<String> = FxHashSet::default();
    let mut tls_got_names: FxHashSet<String> = FxHashSet::default();
    // TLS General-Dynamic symbols: each needs a GOT slot PAIR
    // (DTPMOD64 at slot, DTPOFF64 at slot+8).
    let mut tlsgd_names: Vec<String> = Vec::new();
    // TLS descriptors: a 16-byte pair each, filled by ld.so from one
    // R_X86_64_TLSDESC.
    let mut tlsdesc_names: Vec<String> = Vec::new();
    let mut tlsdesc_seen: FxHashSet<String> = FxHashSet::default();
    let mut local_gd = LocalSlots::default();
    let mut local_desc = LocalSlots::default();
    let mut local_ie = LocalSlots::default();
    let mut local_got = LocalSlots::default();
    // TLS Local-Dynamic: one shared GOT pair (DTPMOD64, 0) per module.
    let mut needs_tlsld_slot = false;
    for (obj_idx, obj) in objects.iter().enumerate() {
        for (sec_idx, sec_relas) in obj.relocations.iter().enumerate() {
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
                let relaxable = |target: GotTarget| {
                    gotpcrelx_relaxation(
                        rela.rela_type,
                        rela.addend,
                        obj.section_data[sec_idx].as_slice(),
                        rela.offset as usize,
                        true,
                        target,
                    )
                    .is_some()
                };
                // Locals first: a section symbol has an empty name and is
                // exactly what `mov .Lc@GOTPCREL(%rip)` references.
                if sym.is_local() {
                    // Never preemptible and never in `.dynsym`; its slots are
                    // filled with module-relative values (RELATIVE, or TLS
                    // relocations against symbol 0 with the TLS-block offset
                    // as addend).
                    let key = (obj_idx, si);
                    match rela.rela_type {
                        R_X86_64_TLSGD => local_gd.insert(key),
                        t if is_tlsdesc_gotpc(t) => local_desc.insert(key),
                        t if is_gottpoff_family(t) => local_ie.insert(key),
                        t if is_gotpcrel_family(t) => {
                            if !relaxable(local_got_target(sym)) {
                                local_got.insert(key);
                            }
                        }
                        t if is_got64_family(t) => local_got.insert(key),
                        _ => {}
                    }
                    continue;
                }
                if sym.name.is_empty() {
                    continue;
                }
                match rela.rela_type {
                    t if is_got64_family(t) => {
                        if got_needed_seen.insert(sym.name.to_string()) {
                            got_needed_names.push(sym.name.to_string());
                        }
                    }
                    t if is_gotpcrel_family(t) || is_gottpoff_family(t) => {
                        let relaxed = is_gotpcrel_family(t)
                            && globals
                                .get(sym.name.as_str())
                                .and_then(|g| got_target(&sym.name, g))
                                .is_some_and(relaxable);
                        if !relaxed && got_needed_seen.insert(sym.name.to_string()) {
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
                    t if is_tlsdesc_gotpc(t) => {
                        if tlsdesc_seen.insert(sym.name.to_string()) {
                            tlsdesc_names.push(sym.name.to_string());
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    // Ensure GOT-referenced undefined symbols are in globals for dynsym
    for name in got_needed_names
        .iter()
        .chain(tlsgd_names.iter())
        .chain(tlsdesc_names.iter())
    {
        if !globals.contains_key(name) {
            // Use STT_TLS for TLS symbols so the dynamic symbol table has the
            // correct type, allowing the dynamic linker to resolve them properly.
            let stype = if tls_got_names.contains(name)
                || tlsgd_seen.contains(name)
                || tlsdesc_seen.contains(name)
            {
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
                    canonical_plt: false,
                    visibility: 0,
                    lib_sym_value: 0,
                    version: None,
                    plt_idx: None,
                    got_idx: None,
                    absolute: false,
                },
            );
        }
    }

    // Pre-scan: collect named global symbols referenced by R_X86_64_64 relocations.
    // These must appear in the dynamic symbol table so the dynamic linker can
    // resolve them (supporting symbol interposition at runtime).
    // A PC32/PC64 in writable storage against such a symbol becomes a
    // dynamic PC-relative relocation that names it just the same.
    let mut abs64_sym_names: BTreeSet<String> = BTreeSet::new();
    for obj in objects.iter() {
        for (sec_idx, sec_relas) in obj.relocations.iter().enumerate() {
            let writable =
                obj.sections[sec_idx].flags & (SHF_ALLOC | SHF_WRITE) == SHF_ALLOC | SHF_WRITE;
            for rela in sec_relas {
                if rela.rela_type == R_X86_64_64
                    || (writable && matches!(rela.rela_type, R_X86_64_PC32 | R_X86_64_PC64))
                {
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
        // `exports_def` above.  Two of its clauses deserve their history:
        // * version scripts: `.symver`-derived globals are keyed
        //   "base@@VER"/"base@VER" while the script's patterns name the BASE;
        //   matching the composed string dropped every explicitly-versioned
        //   export (glibc's libc.so lost fdopen/fopen/..., defined as
        //   _IO_new_fdopen + `.symver fdopen@@GLIBC_2.2.5`).
        // * --exclude-libs: symbols pulled in from the named static archives
        //   are linked in but NOT re-exported -- how a shared library absorbs
        //   a helper archive (OpenSSL's libcrypto.a inside a plugin .so)
        //   without leaking it into its ABI.
        .filter(|(name, g)| exports_def(name, g))
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
        if (gsym.is_dynamic
            || (gsym.defined_in.is_none()
                && gsym.section_idx == SHN_UNDEF
                && imports_undef(gsym)
                && !is_hidden_linkage_symbol(name)))
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
        // A local definition is published only if `exports_def` says so (a
        // hidden or version-local one is bound with R_X86_64_RELATIVE); an
        // undefined one only if it is really imported.
        let publish = match globals.get(name) {
            Some(g) if g.defined_in.is_some() && !g.is_dynamic => exports_def(name, g),
            Some(g) => imports_undef(g),
            None => true,
        } && !is_hidden_linkage_symbol(name);
        if publish && dyn_sym_seen.insert(name.clone()) {
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
    // Oracle-measured sizing shared with the executable emitter: bloom
    // words = next_pow2(n/4) (lld parity, FPR ~2% at glibc scale; the
    // previous n/32 words saturated ~63% full, passing ~40% of misses)
    // and buckets = n/4 expected chain length 4.
    let gnu_hp = linker_common::gnu_hash_params(num_hashed, 64);
    let gnu_hash_nbuckets = gnu_hp.nbuckets;
    let gnu_hash_bloom_size: u32 = gnu_hp.bloom_size;
    let gnu_hash_bloom_shift: u32 = gnu_hp.bloom_shift;

    // Single hash pass over the *emitted* (version-stripped) names: the
    // vector feeds the bloom filter, the bucket sort, and the chain table,
    // so a pre/post-sort disagreement is impossible by construction.
    let mut hashed_sym_hashes: Vec<u32> = defined_syms
        .iter()
        .map(|name| linker_common::gnu_hash(sym_base(name).as_bytes()))
        .collect();
    let bloom_words = linker_common::build_gnu_bloom(&hashed_sym_hashes, &gnu_hp, 64);

    // Sort hashed (defined) symbols by bucket, via an index permutation so
    // the hash vector tracks the name reordering exactly.
    if num_hashed > 0 {
        let mut perm: Vec<usize> = (0..num_hashed).collect();
        perm.sort_by_key(|&i| hashed_sym_hashes[i] % gnu_hash_nbuckets);
        let names_before = defined_syms.clone();
        let hashes_before = hashed_sym_hashes.clone();
        for (new_i, &old_i) in perm.iter().enumerate() {
            dyn_sym_names[undef_syms.len() + new_i] = names_before[old_i].clone();
            hashed_sym_hashes[new_i] = hashes_before[old_i];
        }
    }

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

    // Dynamic imports have their own version-index namespace. Allocate it
    // after all provider-side verdef nodes so a `.gnu.version` entry can be
    // decoded unambiguously by ld.so. Keep the allocation deterministic: both
    // library names and version names are sorted before serialization.
    let mut needed_versions: FxHashMap<String, BTreeSet<String>> = FxHashMap::default();
    for name in &dyn_sym_names {
        if let Some(g) = globals.get(name) {
            if g.is_dynamic {
                if let (Some(lib), Some(version)) = (&g.from_lib, &g.version) {
                    needed_versions
                        .entry(lib.clone())
                        .or_default()
                        .insert(version.clone());
                }
            }
        }
    }
    let provider_verdef_count = if !version_set.is_empty() {
        1 + version_set.len() as u64
    } else if script_nodes.len() > 1 {
        1 + script_nodes.len() as u64
    } else if versioned_name.is_some() {
        2
    } else {
        0
    };
    let first_verneed_index = if provider_verdef_count > 0 {
        provider_verdef_count + 1
    } else {
        2
    };
    let mut version_need_indices: FxHashMap<(String, String), u16> = FxHashMap::default();
    let mut next_verneed_index = first_verneed_index as u16;
    let mut sorted_needed_libs: Vec<String> = needed_versions.keys().cloned().collect();
    sorted_needed_libs.sort();
    for lib in &sorted_needed_libs {
        let mut versions: Vec<String> = needed_versions[lib].iter().cloned().collect();
        versions.sort();
        for version in versions {
            version_need_indices.insert((lib.clone(), version), next_verneed_index);
            next_verneed_index = next_verneed_index
                .checked_add(1)
                .ok_or("too many shared-library symbol versions")?;
        }
    }
    for ((_, version), _) in version_need_indices.iter() {
        dynstr.add(version);
    }
    // A needs-only DSO still gets the base verdef node emitted by the
    // version-table path below; make its SONAME/string-table name available
    // before capturing offsets.
    if !version_need_indices.is_empty() {
        dynstr.add(&base_version_name);
    }
    // This must be computed after importing version names.  Otherwise the
    // VERNEED name offsets point past the end of `.dynstr`; ld.so reports the
    // first byte at that offset as a bogus version such as `G`.
    let dynstr_size = dynstr.as_bytes().len() as u64;

    let (versym_data, verdef_data, verdef_count): (Vec<u8>, Vec<u8>, u64) = if !version_set
        .is_empty()
        && script_nodes.len() <= 1
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
            let dynamic_import = globals.get(name).is_some_and(|g| g.is_dynamic);
            let import_idx = globals.get(name).and_then(|g| {
                let lib = g.from_lib.as_ref()?;
                let version = g.version.as_ref()?;
                version_need_indices
                    .get(&(lib.clone(), version.clone()))
                    .copied()
            });
            let mut idx: u16 = match ver {
                Some(_v) if dynamic_import => import_idx.unwrap_or(1),
                Some(v) => 2 + version_set.iter().position(|x| x == v).unwrap_or(0) as u16,
                // Plain (un-@-suffixed) DEFINED name: GNU ld assigns the
                // version of the FIRST version-script node whose `global:`
                // patterns match — that is how the vast majority of glibc's
                // exports (e.g. __progname@GLIBC_2.2.5) are versioned; only
                // compat symbols carry explicit .symver suffixes. Leaving
                // these at versym 1 (*global*) made every versioned
                // reference from an external binary fail check_match
                // ("undefined symbol __progname, version GLIBC_2.2.5").
                // Undefined symbols stay 1: imports are versioned via
                // verneed, not verdef.
                None => {
                    if let Some(idx) = import_idx {
                        idx
                    } else {
                        let defined = globals
                            .get(name)
                            .is_some_and(|g| g.defined_in.is_some() && g.section_idx != SHN_UNDEF);
                        let from_script = if defined {
                            version_script.as_ref().and_then(|vs| {
                                vs.nodes.iter().find_map(|node| {
                                    node.global_patterns
                                        .iter()
                                        .any(|pat| linker_common::wildcard_match_pattern(pat, name))
                                        .then(|| {
                                            version_set
                                                .iter()
                                                .position(|x| x == &node.name)
                                                .map(|p| (2 + p) as u16)
                                        })
                                        .flatten()
                                })
                            })
                        } else {
                            None
                        };
                        from_script.unwrap_or(1)
                    }
                }
            };
            // "name@VER" (single @): non-default version — hidden bit set.
            // The high bit is only valid for a provider-side hidden version;
            // versioned undefined imports use VERNEED's plain index.
            if hidden && !dynamic_import {
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
            let import_idx = globals.get(name).and_then(|g| {
                let lib = g.from_lib.as_ref()?;
                let version = g.version.as_ref()?;
                version_need_indices
                    .get(&(lib.clone(), version.clone()))
                    .copied()
            });
            let idx: u16 = if let Some(import_idx) = import_idx {
                import_idx
            } else if !defined {
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
    } else if !version_need_indices.is_empty() {
        // A DSO with only versioned imports has no provider version nodes, but
        // it still needs a versym entry for every dynamic symbol and a base
        // verdef node so the section remains a valid GNU version table.
        let mut versym = Vec::with_capacity(dynsym_count as usize * 2);
        versym.extend_from_slice(&0u16.to_le_bytes());
        for name in &dyn_sym_names {
            let idx = globals
                .get(name)
                .and_then(|g| {
                    let lib = g.from_lib.as_ref()?;
                    let version = g.version.as_ref()?;
                    version_need_indices
                        .get(&(lib.clone(), version.clone()))
                        .copied()
                })
                .unwrap_or(1);
            versym.extend_from_slice(&idx.to_le_bytes());
        }
        let mut verdef = Vec::new();
        let base_off = dynstr.get_offset(&base_version_name);
        push_verdef_entry(&mut verdef, 1, &base_version_name, base_off, 0);
        (versym, verdef, 1)
    } else {
        (Vec::new(), Vec::new(), 0)
    };
    let versym_size = versym_data.len() as u64;
    let verdef_size = verdef_data.len() as u64;

    let mut verneed_data = Vec::new();
    for (lib_i, lib) in sorted_needed_libs.iter().enumerate() {
        let mut versions: Vec<(String, u16, usize)> = needed_versions[lib]
            .iter()
            .filter_map(|version| {
                let index = version_need_indices
                    .get(&(lib.clone(), version.clone()))
                    .copied()?;
                Some((version.clone(), index, dynstr.get_offset(version)))
            })
            .collect();
        versions.sort_by(|a, b| a.1.cmp(&b.1));
        let soname_off = dynstr.get_offset(lib);
        let last = lib_i + 1 == sorted_needed_libs.len();
        push_verneed_entry(&mut verneed_data, soname_off, &versions, last);
    }
    let verneed_size = verneed_data.len() as u64;
    let verneed_count = sorted_needed_libs.len() as u64;

    let want_gnu_hash = hash_style.wants_gnu();
    let want_sysv_hash = hash_style.wants_sysv();
    // Built here, after the GNU-hash bucket sort has fixed the final dynsym
    // order: the SysV table indexes `.dynsym` positions directly, so it must
    // see the order the symbols are actually written in.  Same `sym_base`
    // spelling the dynsym/GNU-hash paths use, so the two tables agree on names.
    let sysv_hash: Option<crate::backend::linker_common::SysvHash> = if want_sysv_hash {
        let owned: Vec<String> = dyn_sym_names.iter().map(|n| sym_base(n)).collect();
        let names: Vec<&str> = owned.iter().map(|n| n.as_str()).collect();
        Some(crate::backend::linker_common::build_sysv_hash(&names))
    } else {
        None
    };
    let sysv_hash_size: u64 = sysv_hash.as_ref().map(|h| h.size()).unwrap_or(0);
    // `--hash-style=sysv` means no `.gnu.hash` at all: zero size, no section
    // header, no DT_GNU_HASH.  An empty table left behind would be preferred by
    // a loader that understands it, and would then resolve nothing.
    let gnu_hash_size: u64 = if !want_gnu_hash {
        0
    } else {
        16 + (gnu_hash_bloom_size as u64 * 8)
            + (gnu_hash_nbuckets as u64 * 4)
            + (num_hashed as u64 * 4)
    };

    // PLT entries: PLT0, one per imported/preemptible function, then one
    // per bound IFUNC (IPLT); `.got.plt` and `.rela.plt` follow the same
    // index order.
    let n_plt_total = plt_names.len() + n_iplt;
    let plt_size = if n_plt_total == 0 {
        0u64
    } else {
        16 + 16 * n_plt_total as u64
    };
    let got_plt_count = if n_plt_total == 0 { 0 } else { 3 + n_plt_total };
    let got_plt_size = got_plt_count as u64 * 8;
    let rela_plt_size = n_plt_total as u64 * 24;

    // Count R_X86_64_RELATIVE relocations needed (for internal absolute addresses)
    // We'll collect them during relocation processing
    let has_init_array = output_sections
        .iter()
        .any(|s| s.name == ".init_array" && s.mem_size > 0);
    let has_fini_array = output_sections
        .iter()
        .any(|s| s.name == ".fini_array" && s.mem_size > 0);
    // `.init`/`.fini` (crti/crtn): addresses are not final yet (layout runs
    // below), so only the predicates live here; emission looks them up.
    let has_init = output_sections
        .iter()
        .any(|s| s.name == ".init" && s.mem_size > 0);
    let has_fini = output_sections
        .iter()
        .any(|s| s.name == ".fini" && s.mem_size > 0);
    // 8 fixed entries + DT_NULL.  DT_GNU_HASH is no longer unconditional (see
    // --hash-style), so both hash tags are counted below instead.
    let mut dyn_count = needed_sonames.len() as u64 + 9;
    if want_gnu_hash {
        dyn_count += 1;
    }
    if want_sysv_hash {
        dyn_count += 1;
    }
    if soname.is_some() {
        dyn_count += 1;
    }
    if has_init_array {
        dyn_count += 2;
    }
    if has_fini_array {
        dyn_count += 2;
    }
    // Spelled identically to the emission below: DT_INIT/DT_FINI when the
    // sections exist with content.
    if has_init {
        dyn_count += 1;
    }
    if has_fini {
        dyn_count += 1;
    }
    if rela_plt_size > 0 {
        dyn_count += 4;
    } // DT_PLTGOT, DT_PLTRELSZ, DT_PLTREL, DT_JMPREL
    if rpath_string.is_some() {
        dyn_count += 1;
    } // DT_RUNPATH or DT_RPATH
    dyn_count += u64::from(dt_symbolic) + u64::from(dt_flags != 0) + u64::from(dt_flags_1 != 0);
    if verdef_count > 0 || verneed_count > 0 {
        dyn_count += 1; // DT_VERSYM
    }
    if verdef_count > 0 {
        dyn_count += 2; // DT_VERDEF, DT_VERDEFNUM
    }
    if verneed_count > 0 {
        dyn_count += 2; // DT_VERNEED, DT_VERNEEDNUM
    }
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

    // phdrs: PHDR, LOAD(ro), LOAD(text), LOAD(rodata), LOAD(rw), DYNAMIC,
    // NOTE* + GNU_PROPERTY, GNU_STACK, [GNU_RELRO], [TLS]
    // RELRO follows `-z relro` (the default), as in `emit_exec`.  It used to
    // exist only when some read-only section carried absolute relocations,
    // so a typical -fPIC library -- whose relocated constants the compiler
    // already puts in `.data.rel.ro` -- had NO PT_GNU_RELRO: `.dynamic`,
    // `.init_array`, `.data.rel.ro` and every GOT slot stayed writable for
    // the life of the process.  The window is now what GNU ld protects:
    // those plus the whole `.got`, and `.got.plt` too under `-z now`.
    let has_relro = z_relro;
    // .eh_frame_hdr + PT_GNU_EH_FRAME.  Not optional for a shared library:
    // modern crtbeginS.o no longer registers `.eh_frame` with
    // `__register_frame_info`, so libgcc's unwinder finds a DSO's FDEs ONLY
    // through `dl_iterate_phdr` -> PT_GNU_EH_FRAME.  Without it a C++
    // exception thrown through this library reaches std::terminate and
    // backtrace() stops at its first frame.  Same sizing as `emit_exec`.
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
    let mut phdr_count: u64 = 7; // base count
    if eh_frame_hdr_size > 0 {
        phdr_count += 1; // PT_GNU_EH_FRAME
    }
    if has_tls_sections {
        phdr_count += 1;
    }
    if has_relro {
        phdr_count += 1;
    }
    // Split-RW predicate: anything (file bytes OR pure memory) laid out
    // after the RELRO boundary below — .got.plt, the local/TLS GOT, writable
    // sections, TLS, .bss.  Same contract as `emit_exec`: the RELRO window
    // gets its own PT_LOAD and the page pad becomes NOBITS instead of a run
    // of file zeros.  MUST match the layout pass exactly.
    let has_post_relro_content = (got_plt_size > 0 && !z_now)
        || output_sections.iter().any(|s| {
            s.flags & SHF_ALLOC != 0
                && s.flags & SHF_WRITE != 0
                && s.mem_size > 0
                && s.name != ".init_array"
                && s.name != ".fini_array"
                && s.name != ".data.rel.ro"
        });
    let split_relro_load = has_relro && has_post_relro_content;
    if split_relro_load {
        phdr_count += 1; // second RW PT_LOAD (writable tail after RELRO)
    }
    // One PT_NOTE segment per contiguous RUN of allocated note sections plus
    // the PT_GNU_PROPERTY alias — merged exactly like `emit_exec` (one phdr
    // saved per adjacent note; count and write run the identical walk, so
    // they cannot drift).  The count must use `mem_size`, which is set
    // before layout; section `data` is only filled during the layout pass.
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

    // === Layout ===
    let mut offset = 64 + phdr_total_size;

    offset = (offset + 7) & !7;
    let gnu_hash_offset = offset;
    let gnu_hash_addr = vaddr!(offset);
    offset += gnu_hash_size;
    offset = (offset + 3) & !3;
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
    offset = (offset + 1) & !1;
    let versym_offset = offset;
    let versym_addr = vaddr!(offset);
    offset += versym_size;
    offset = (offset + 7) & !7;
    let verdef_offset = offset;
    let verdef_addr = vaddr!(offset);
    offset += verdef_size;
    offset = (offset + 7) & !7;
    let verneed_offset = offset;
    let verneed_addr = vaddr!(offset);
    offset += verneed_size;

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
    // .eh_frame_hdr leads the rodata segment (filled after relocation).
    let (eh_frame_hdr_offset, eh_frame_hdr_vaddr) = if eh_frame_hdr_size > 0 {
        let o = offset;
        let v = vaddr!(offset);
        offset += eh_frame_hdr_size;
        (o, v)
    } else {
        (0u64, 0u64)
    };
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
    // A PC32/PC64 in writable storage may become a dynamic PC-relative
    // relocation (see the relocation pass), so it is counted too.
    let mut max_rela_count: usize = 0;
    for obj in objects.iter() {
        for (sec_idx, sec_relas) in obj.relocations.iter().enumerate() {
            let writable =
                obj.sections[sec_idx].flags & (SHF_ALLOC | SHF_WRITE) == SHF_ALLOC | SHF_WRITE;
            for rela in sec_relas {
                if rela.rela_type == R_X86_64_64
                    || (writable && matches!(rela.rela_type, R_X86_64_PC32 | R_X86_64_PC64))
                {
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
    // One relocation per TLS descriptor, local GD pair (DTPMOD64 only: the
    // offset is static), local IE slot (TPOFF64) and local GOT slot
    // (RELATIVE, none for an absolute symbol).
    max_rela_count +=
        tlsdesc_names.len() + local_desc.len() + local_gd.len() + local_ie.len() + local_got.len();
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

    // End of RELRO region (page-aligned up for PT_GNU_RELRO).
    // Everything after this must be on a new page so that mprotect(PROT_READ)
    // on the RELRO region doesn't affect writable data (GOT.PLT, GOT, .data, .bss).
    // With densely packed file offsets the RELRO end must be aligned in
    // ADDRESS space: ld.so mprotects page-rounded [vaddr, vaddr+memsz), so
    // aligning the file offset would leave the boundary mid-page and
    // write-protect the head of the following section.  The pad up to the
    // boundary page is NOBITS — address space only, zero file bytes (same
    // contract as the executable emitter; before, up to one page of zeros
    // was baked into every shared object with a .data.rel.ro).
    // `relro_file_end` / `rw2_addr` are the two halves of the split-LOAD
    // interface: file offset where RELRO content ends, and the address the
    // writable tail's PT_LOAD starts at (post-bias-update).
    // .got layout (offsets within the section):
    //   [got_needed slots][GD pairs][local GD pairs][TLSDESC pairs]
    //   [local TLSDESC pairs][LD pair][local IE slots][local GOT slots]
    // Pairs come first so every 16-byte pair stays 8-aligned (all entries
    // are 8 bytes; nothing needs more).  Every slot is written by ld.so
    // during relocation only (TLS descriptors too: they are resolved
    // eagerly from .rela.dyn), so the section belongs to the RELRO window.
    offset = (offset + 7) & !7;
    let got_offset = offset;
    let got_addr = vaddr!(offset);
    let tlsgd_got_base = got_needed.len() as u64 * 8; // offset of first GD pair within .got
    let local_gd_base = tlsgd_got_base + tlsgd_names.len() as u64 * 16;
    let tlsdesc_base = local_gd_base + local_gd.len() as u64 * 16;
    let local_desc_base = tlsdesc_base + tlsdesc_names.len() as u64 * 16;
    let tlsld_got_off = local_desc_base + local_desc.len() as u64 * 16;
    let local_ie_base = tlsld_got_off + if needs_tlsld_slot { 16 } else { 0 };
    let local_got_base = local_ie_base + local_ie.len() as u64 * 8;
    let got_size = local_got_base + local_got.len() as u64 * 8;
    offset += got_size;

    // Under `-z now` nothing writes `.got.plt` after startup either (Full
    // RELRO); under lazy binding ld.so's resolver patches it on every
    // first call, so it goes after the boundary.
    let got_plt_in_relro = has_relro && z_now;
    let (mut got_plt_offset, mut got_plt_addr) = (0u64, 0u64);
    if got_plt_in_relro {
        offset = (offset + 7) & !7;
        got_plt_offset = offset;
        got_plt_addr = vaddr!(offset);
        offset += got_plt_size;
    }

    let mut relro_file_end = offset;
    let mut relro_mem_size = 0u64;
    let mut rw2_addr = vaddr!(offset);
    if has_relro {
        relro_file_end = offset;
        let relro_pad = packer.padding_to_page(offset);
        relro_mem_size = vaddr!(offset) + relro_pad - rw_page_addr;
        if split_relro_load && relro_pad > 0 {
            packer.new_segment();
        }
        rw2_addr = vaddr!(offset);
    }

    // Lazy `.got.plt`: after the RELRO boundary (see above).
    if !got_plt_in_relro {
        offset = (offset + 7) & !7;
        got_plt_offset = offset;
        got_plt_addr = vaddr!(offset);
        offset += got_plt_size;
    }

    for sec in output_sections.iter_mut() {
        if sec.flags & SHF_ALLOC != 0
            && sec.flags & SHF_WRITE != 0
            && sec.sh_type != SHT_NOBITS
            && sec.name != ".init_array"
            && sec.name != ".fini_array"
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

    // TLS sections
    let mut tls_addr = 0u64;
    let mut tls_file_offset = 0u64;
    let mut tls_file_size = 0u64;
    let mut tls_mem_size = 0u64;
    // PT_TLS must start at a multiple of its p_align (the largest TLS
    // section alignment): the loader puts the block at thread-pointer
    // offset `roundup(p_memsz, p_align)` only then — glibc otherwise also
    // accounts for `p_vaddr % p_align` — and that is the offset every
    // link-time TPOFF value assumes.  Aligning to the first section's
    // alignment alone shifted all variables when `.tbss` was more aligned.
    let mut tls_align = output_sections
        .iter()
        .filter(|sec| sec.flags & SHF_TLS != 0 && sec.flags & SHF_ALLOC != 0)
        .map(|sec| sec.alignment.max(1))
        .max()
        .unwrap_or(1);
    if has_tls_sections {
        offset = (offset + tls_align - 1) & !(tls_align - 1);
    }
    for sec in output_sections.iter_mut() {
        if sec.flags & SHF_TLS != 0 && sec.flags & SHF_ALLOC != 0 && sec.sh_type != SHT_NOBITS {
            let a = sec.alignment.max(1);
            offset = (offset + a - 1) & !(a - 1);
            sec.addr = vaddr!(offset);
            sec.file_offset = offset;
            if tls_addr == 0 {
                tls_addr = sec.addr;
                tls_file_offset = offset;
            }
            offset += sec.mem_size;
            // Inter-section alignment padding is part of the TLS image.
            tls_file_size = offset - tls_file_offset;
            tls_mem_size = tls_file_size;
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
            canonical_plt: false,
            visibility: 0,
            lib_sym_value: 0,
            version: None,
            absolute: false,
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

    // Finalise deferred `--defsym` expressions now that every address they may
    // reference is final (section addresses + linker-provided symbols).
    super::link::evaluate_pending_defsyms(globals, pending_defsyms)?;

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
    let ro_seg_end = if verneed_size > 0 {
        verneed_offset + verneed_size
    } else if verdef_size > 0 {
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
    if has_relro {
        // RELRO LOAD (filesz = file content, memsz covering the NOBITS pad),
        // then the writable tail at the same dense file offset on a fresh
        // page — the executable emitter's split, see its rationale there.
        wphdr(
            &mut out,
            ph,
            PT_LOAD,
            PF_R | PF_W,
            rw_page_offset,
            rw_page_addr,
            relro_file_end - rw_page_offset,
            relro_mem_size,
            PAGE_SIZE,
        );
        ph += 56;
        if split_relro_load {
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
    // One PT_NOTE per contiguous run of allocated note sections (p_align
    // the maximum member alignment, minimum 4) — the same walk that counted
    // them before layout, identical predicate and order.
    {
        let mut run_start: Option<(u64, u64, u64)> = None;
        let mut run_end: Option<(u64, u64)> = None;
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
    wphdr(&mut out, ph, PT_GNU_STACK, PF_R | PF_W, 0, 0, 0, 0, 0x10);
    ph += 56;
    if has_relro {
        // filesz covers the RELRO file content; memsz reaches across the
        // NOBITS page pad (ld.so rounds vaddr+memsz out to pages).
        wphdr(
            &mut out,
            ph,
            PT_GNU_RELRO,
            PF_R,
            rw_page_offset,
            rw_page_addr,
            relro_file_end - rw_page_offset,
            relro_mem_size,
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
    if let Some(sh) = &sysv_hash {
        crate::backend::linker_common::write_sysv_hash(&mut out, sysv_hash_offset as usize, sh);
    }
    // Gated: with `--hash-style=sysv` the GNU table has zero size, so
    // `gnu_hash_offset` and `sysv_hash_offset` are the *same* address and this
    // writer would overwrite the SysV table that was just laid down.
    if want_gnu_hash {
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
    }

    // An import's binding is STB_WEAK iff every reference to it is weak,
    // exactly as in GNU ld: glibc defines `puts` as a weak alias, and
    // copying that STB_WEAK would let ld.so silently bind an ordinary call
    // to 0 on a library lacking it; conversely `__cxa_finalize`, referenced
    // only weakly by crtbeginS.o, must stay weak although libc defines it
    // GLOBAL.  The type is STT_TLS iff some reference says so.
    let mut import_has_strong_ref: FxHashMap<&str, bool> = FxHashMap::default();
    let mut import_ref_type: FxHashMap<&str, u8> = FxHashMap::default();
    for obj in objects.iter() {
        for sym in &obj.symbols {
            if sym.is_undefined() && !sym.is_local() && !sym.name.is_empty() {
                *import_has_strong_ref
                    .entry(sym.name.as_str())
                    .or_insert(false) |= !sym.is_weak();
                if sym.sym_type() == STT_TLS {
                    import_ref_type.insert(sym.name.as_str(), STT_TLS);
                }
            }
        }
    }

    // Bound IFUNCs take their IPLT entry as address (see the scan above);
    // the resolver's address goes to the IRELATIVE.  A global one's
    // `.dynsym` entry (-Bsymbolic, protected) then exports that canonical
    // address as a plain function, as GNU ld does for pointer equality.
    let n_plt = plt_names.len();
    let iplt_entry = |j: usize| plt_addr + 16 + (n_plt + j) as u64 * 16;
    let mut iplt_resolvers: Vec<u64> = Vec::with_capacity(n_iplt);
    for (j, name) in iplt_globals.iter().enumerate() {
        let g = globals
            .get_mut(name)
            .ok_or_else(|| format!("internal error: IFUNC '{name}' vanished"))?;
        iplt_resolvers.push(g.value);
        g.value = iplt_entry(j);
        g.info = (g.info & 0xf0) | STT_FUNC;
    }
    let mut local_ifunc_addr: FxHashMap<(usize, usize), u64> = FxHashMap::default();
    for (k, &(obj_idx, si)) in iplt_locals.iter().enumerate() {
        iplt_resolvers.push(resolve_sym(
            obj_idx,
            &objects[obj_idx].symbols[si],
            globals,
            section_map,
            output_sections,
            plt_addr,
        ));
        local_ifunc_addr.insert((obj_idx, si), iplt_entry(iplt_globals.len() + k));
    }

    // .dynsym
    let mut ds = dynsym_offset as usize + 24; // skip null entry
    for name in &dyn_sym_names {
        let no = dynstr.get_offset(&sym_base(name)) as u32;
        w32(&mut out, ds, no);
        if let Some(gsym) = globals.get(name) {
            if gsym.defined_in.is_some() && !gsym.is_dynamic && gsym.section_idx != SHN_UNDEF {
                // Exported defined symbol: preserve original st_info (type
                // + binding); st_other carries PROTECTED (the only
                // non-default visibility an exported symbol can have).
                if ds + 5 < out.len() {
                    out[ds + 4] = gsym.info;
                    out[ds + 5] = gsym.visibility;
                }
                // Placeholder; the real section index is patched in once
                // section-header numbering is fixed (`def_shndx` below).
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
                // Import.  Its binding describes this output's references,
                // not the providing library's definition (see
                // `import_has_strong_ref`); its type is the one the
                // references carry -- NOTYPE unless they said STT_TLS,
                // which ld.so does need, as GNU ld does.  A FUNC/OBJECT
                // copied from the provider would claim knowledge the
                // referencing objects never had (and for an unresolved
                // weak reference such as `_ITM_registerTMCloneTable` there
                // is no provider to copy from at all).
                let bind = if gsym.is_dynamic {
                    match import_has_strong_ref.get(name.as_str()) {
                        Some(false) => STB_WEAK,
                        _ => STB_GLOBAL,
                    }
                } else {
                    gsym.info >> 4
                };
                let stype = import_ref_type
                    .get(name.as_str())
                    .copied()
                    .unwrap_or(STT_NOTYPE);
                let st_info = (bind << 4) | stype;
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
                out[ds + 4] = (STB_GLOBAL << 4)
                    | import_ref_type
                        .get(name.as_str())
                        .copied()
                        .unwrap_or(STT_NOTYPE);
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
    if !verneed_data.is_empty() {
        write_bytes(&mut out, verneed_offset as usize, &verneed_data);
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
        // IPLT entries: the slot is filled eagerly by IRELATIVE, so there
        // is no lazy path to fall back to -- trap instead of the push/jmp.
        for j in 0..n_iplt {
            let i = n_plt + j;
            let ep = po + 16 + i * 16;
            let pea = plt_addr + 16 + i as u64 * 16;
            let gea = got_plt_addr + 24 + i as u64 * 8;
            out[ep] = 0xff;
            out[ep + 1] = 0x25; // jmp [GOT.PLT slot]
            w32(&mut out, ep + 2, (gea as i64 - (pea + 6) as i64) as u32);
            out[ep + 6..ep + 16].fill(0xcc);
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
        // IPLT slots: the resolver until ld.so applies the IRELATIVE.
        for (j, &resolver) in iplt_resolvers.iter().enumerate() {
            w64(&mut out, gp + 24 + (n_plt + j) * 8, resolver);
        }
    }

    // .rela.plt - JMPREL relocations for GOT.PLT entries
    if rela_plt_size > 0 {
        let mut rp = rela_plt_offset as usize;
        let gpb = got_plt_addr + 24; // base of per-symbol GOT.PLT slots
        for (i, name) in plt_names.iter().enumerate() {
            let gea = gpb + i as u64 * 8;
            // Find symbol index in dynsym
            let si = dyn_sym_index.get(name.as_str()).copied().ok_or_else(|| {
                format!("internal error: PLT entry for '{name}', which is not in .dynsym")
            })?;
            w64(&mut out, rp, gea); // r_offset = GOT.PLT slot address
            w64(&mut out, rp + 8, (si << 32) | R_X86_64_JUMP_SLOT as u64);
            w64(&mut out, rp + 16, 0); // r_addend = 0
            rp += 24;
        }
        for (j, &resolver) in iplt_resolvers.iter().enumerate() {
            w64(&mut out, rp, gpb + (n_plt + j) as u64 * 8);
            w64(&mut out, rp + 8, R_X86_64_IRELATIVE as u64);
            w64(&mut out, rp + 16, resolver); // ld.so adds the load bias
            rp += 24;
        }
    }

    // Build GOT entries map.  Slot CONTENTS and their dynamic relocations
    // are decided once, per slot, in the pass below the TLS setup.
    let mut got_sym_addrs: FxHashMap<String, u64> = FxHashMap::default();
    for (i, name) in got_needed.iter().enumerate() {
        got_sym_addrs.insert(name.clone(), got_addr + i as u64 * 8);
    }

    // Apply relocations and collect dynamic relocation entries
    let globals_snap: FxHashMap<String, GlobalSymbol> = globals.clone();
    let mut rela_dyn_entries: Vec<(u64, u64)> = Vec::new(); // (offset, value) for RELATIVE relocs
    let mut glob_dat_entries: Vec<(u64, String)> = Vec::new(); // (offset, sym_name) for GLOB_DAT relocs
    // (offset, sym_name or "" for symbol 0, addend) for R_X86_64_TPOFF64:
    // symbolic for a preemptible/imported TLS symbol, symbol 0 plus the
    // offset inside this module's TLS block otherwise -- the thread-pointer
    // offset of a shared library's block is only known to ld.so.
    let mut tpoff64_entries: Vec<(u64, String, u64)> = Vec::new();
    let mut abs64_entries: Vec<(u64, String, i64)> = Vec::new(); // (offset, sym_name, addend) for R_X86_64_64 relocs
    // (offset, type, sym_name or "" for symbol 0, addend): R_X86_64_PC32 /
    // R_X86_64_PC64 left to ld.so -- see `pc_rel_to_loader` below.
    let mut pcrel_entries: Vec<(u64, u32, String, i64)> = Vec::new();
    // TLS module-id relocations: (got_slot_vaddr, sym_name_or_empty).
    // DTPMOD64 always needed (module id known only at load time); symbol 0
    // means "this module".  DTPOFF64 only for preemptible/imported symbols.
    let mut dtpmod64_entries: Vec<(u64, String)> = Vec::new();
    let mut dtpoff64_entries: Vec<(u64, String)> = Vec::new();
    // (descriptor vaddr, sym_name or "", addend) for R_X86_64_TLSDESC.
    let mut tlsdesc_entries: Vec<(u64, String, u64)> = Vec::new();

    // A TLS symbol whose offset in THIS module's TLS block is a link-time
    // constant: defined here and not preemptible.
    let static_tls_offset = |name: &str| -> Option<u64> {
        let g = globals_snap.get(name)?;
        (g.defined_in.is_some()
            && !g.is_dynamic
            && g.section_idx != SHN_UNDEF
            && !preemptible_def(name, g))
        .then(|| g.value.wrapping_sub(tls_addr))
    };
    let local_sym_addr = |(obj_idx, si): (usize, usize)| -> u64 {
        if let Some(&iplt) = local_ifunc_addr.get(&(obj_idx, si)) {
            return iplt;
        }
        resolve_sym(
            obj_idx,
            &objects[obj_idx].symbols[si],
            &globals_snap,
            section_map,
            output_sections,
            plt_addr,
        )
    };

    // TLS GD pairs (DTPMOD64 at slot, DTPOFF64 at slot+8).  A symbol defined
    // here and not preemptible has a static DTPOFF and needs only the module
    // id; anything else is resolved by ld.so against the DEFINING module --
    // DTPMOD64 must then name the symbol, not symbol 0 ("this module"), or
    // the access lands in this library's block at the other module's offset.
    let mut tlsgd_slot_addr: FxHashMap<String, u64> = FxHashMap::default();
    for (i, name) in tlsgd_names.iter().enumerate() {
        let slot = got_addr + tlsgd_got_base + i as u64 * 16;
        tlsgd_slot_addr.insert(name.clone(), slot);
        match static_tls_offset(name) {
            Some(off) => {
                dtpmod64_entries.push((slot, String::new()));
                w64(
                    &mut out,
                    (got_offset + tlsgd_got_base + i as u64 * 16 + 8) as usize,
                    off,
                );
            }
            None => {
                dtpmod64_entries.push((slot, name.clone()));
                dtpoff64_entries.push((slot + 8, name.clone()));
            }
        }
    }
    for (i, &key) in local_gd.keys().iter().enumerate() {
        let off = local_gd_base + i as u64 * 16;
        dtpmod64_entries.push((got_addr + off, String::new()));
        let dtpoff = local_sym_addr(key).wrapping_sub(tls_addr);
        w64(&mut out, (got_offset + off + 8) as usize, dtpoff);
    }
    // TLS descriptors: one R_X86_64_TLSDESC per 16-byte pair, resolved
    // eagerly from .rela.dyn (glibc handles it there; no DT_TLSDESC_* lazy
    // trampoline is needed).
    let mut tlsdesc_slot_addr: FxHashMap<String, u64> = FxHashMap::default();
    for (i, name) in tlsdesc_names.iter().enumerate() {
        let slot = got_addr + tlsdesc_base + i as u64 * 16;
        tlsdesc_slot_addr.insert(name.clone(), slot);
        match static_tls_offset(name) {
            Some(off) => tlsdesc_entries.push((slot, String::new(), off)),
            None => tlsdesc_entries.push((slot, name.clone(), 0)),
        }
    }
    for (i, &key) in local_desc.keys().iter().enumerate() {
        let slot = got_addr + local_desc_base + i as u64 * 16;
        let off = local_sym_addr(key).wrapping_sub(tls_addr);
        tlsdesc_entries.push((slot, String::new(), off));
    }
    let tlsld_slot = if needs_tlsld_slot {
        let slot = got_addr + tlsld_got_off;
        dtpmod64_entries.push((slot, String::new()));
        // slot+8 stays 0 (offsets computed via DTPOFF32 addends)
        Some(slot)
    } else {
        None
    };
    // Local IE slots: TPOFF64 against symbol 0, addend = block offset.
    for (i, &key) in local_ie.keys().iter().enumerate() {
        let off = local_sym_addr(key).wrapping_sub(tls_addr);
        tpoff64_entries.push((got_addr + local_ie_base + i as u64 * 8, String::new(), off));
    }
    // Local GOT slots: the symbol's address.  An absolute symbol's value is
    // stored as-is; RELATIVE would add the load bias to it.
    for (i, &(obj_idx, si)) in local_got.keys().iter().enumerate() {
        let off = local_got_base + i as u64 * 8;
        let v = local_sym_addr((obj_idx, si));
        if objects[obj_idx].symbols[si].shndx == SHN_ABS {
            w64(&mut out, (got_offset + off) as usize, v);
        } else {
            rela_dyn_entries.push((got_addr + off, v));
        }
    }

    // Global GOT slots.  Preemptible definitions get GLOB_DAT so that ld.so
    // can bind them elsewhere -- above all to an executable's COPY of a data
    // object, which RELATIVE would split in two -- non-preemptible ones a
    // RELATIVE (or their value, if absolute); IE slots get TPOFF64.
    for (i, name) in got_needed.iter().enumerate() {
        let gea = got_addr + i as u64 * 8;
        let slot_off = (got_offset + i as u64 * 8) as usize;
        let is_tls = tls_got_names.contains(name);
        let g = globals_snap.get(name);
        let local_def =
            g.filter(|g| g.defined_in.is_some() && !g.is_dynamic && g.section_idx != SHN_UNDEF);
        if is_tls {
            match static_tls_offset(name) {
                Some(off) => tpoff64_entries.push((gea, String::new(), off)),
                None => tpoff64_entries.push((gea, name.clone(), 0)),
            }
            continue;
        }
        match local_def {
            Some(g) if preemptible_def(name, g) => glob_dat_entries.push((gea, name.clone())),
            Some(g) if g.absolute => w64(&mut out, slot_off, g.value),
            Some(g) => {
                w64(&mut out, slot_off, g.value);
                rela_dyn_entries.push((gea, g.value));
            }
            // A hidden undefined (weak) symbol is 0 in this component: the
            // slot keeps its zero and has no relocation.
            None if g.is_some_and(|g| !imports_undef(g)) => {}
            None => glob_dat_entries.push((gea, name.clone())),
        }
    }

    // Whether ld.so may bind `sym` outside this library: an import, or a
    // preemptible definition.  The same predicate that makes an
    // R_X86_64_64 symbolic.
    let binds_outside = |sym: &Symbol, global: Option<&GlobalSymbol>| -> bool {
        sym.sym_type() != STT_SECTION
            && match global {
                Some(g) if g.defined_in.is_some() && !g.is_dynamic => preemptible_def(&sym.name, g),
                Some(g) => imports_undef(g),
                None => !sym.name.is_empty() && !sym.is_local(),
            }
    };
    // A truly absolute target (`--defsym X=0x1000`, an SHN_ABS input
    // symbol; never a linker-provided anchor, which is section-relative).
    let abs_target = |sym: &Symbol, global: Option<&GlobalSymbol>| -> bool {
        match global {
            Some(g) => g.absolute,
            None => sym.shndx == SHN_ABS && sym.sym_type() != STT_SECTION,
        }
    };
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
                let s = resolve_sym(
                    obj_idx,
                    sym,
                    &globals_snap,
                    section_map,
                    output_sections,
                    plt_addr,
                );
                // A bound local IFUNC's address is its IPLT entry.
                let s = if sym.is_local() {
                    local_ifunc_addr.get(&(obj_idx, si)).copied().unwrap_or(s)
                } else {
                    s
                };
                let global = (!sym.name.is_empty() && !sym.is_local())
                    .then(|| globals_snap.get(sym.name.as_str()))
                    .flatten();
                match rela.rela_type {
                    R_X86_64_64 => {
                        let val = (s as i64 + a) as u64;
                        w64(&mut out, fp, val);
                        // Determine what kind of dynamic relocation to emit.
                        // Named global/weak symbols need R_X86_64_64 dynamic relocs
                        // (with symbol index) to support symbol interposition.
                        // Section symbols and local symbols use R_X86_64_RELATIVE.
                        // Symbolic iff ld.so may bind the symbol outside
                        // this library: an import, or a preemptible
                        // definition.  Everything else is RELATIVE (or
                        // nothing, for a hidden undefined weak symbol, 0).
                        let symbolic = binds_outside(sym, global);
                        let absolute = !symbolic
                            && match global {
                                Some(g) => g.absolute,
                                None => sym.shndx == SHN_ABS,
                            };
                        if symbolic {
                            abs64_entries.push((p, sym.name.to_string(), a));
                        } else if s != 0 && !absolute {
                            rela_dyn_entries.push((p, val));
                        }
                    }
                    R_X86_64_PC32 | R_X86_64_PLT32 => {
                        // A PC-relative field has no link-time value when
                        // its target is not fixed relative to this image:
                        // an absolute symbol (the image moves, the target
                        // does not), or -- for anything but a branch that
                        // goes through the PLT -- a symbol ld.so may bind
                        // elsewhere.  A non-branch PC32 takes the symbol's
                        // ADDRESS (`lea x(%rip)`, `.long x - .`): the PLT
                        // entry is not that address, and a preemptible
                        // definition's own copy is not the one an
                        // executable's COPY relocation made canonical.
                        let via_plt = global.is_some_and(|g| g.plt_idx.is_some())
                            && (rela.rela_type == R_X86_64_PLT32
                                || super::plt_got::is_branch_rel32(
                                    in_sec.flags,
                                    objects[obj_idx].section_data[sec_idx].as_slice(),
                                    rela.offset,
                                ));
                        if in_sec.flags & SHF_ALLOC != 0
                            && !via_plt
                            && (abs_target(sym, global) || binds_outside(sym, global))
                        {
                            pc_rel_to_loader(
                                &mut pcrel_entries,
                                in_sec.flags,
                                rela.rela_type,
                                p,
                                sym,
                                binds_outside(sym, global),
                                s,
                                a,
                                obj_name,
                            )?;
                            continue;
                        }
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
                        w32_checked(
                            &mut out,
                            fp,
                            t as i64 + a - p as i64,
                            rela.rela_type,
                            &sym.name,
                            &objects[obj_idx].source_name,
                        )?;
                    }
                    // Absolute fields narrower than a pointer cannot hold a
                    // load-time address, so in loaded storage they are only
                    // valid against an absolute symbol that ld.so cannot
                    // rebind (GNU ld: "recompile with -fPIC"; it never
                    // turns them into dynamic relocations either).  Writing
                    // the link-time value -- this emitter's old behaviour
                    // -- produced a library that was wrong at every load
                    // address but 0.  A non-preemptible absolute value is
                    // load-independent and stays a link-time constant (as
                    // in lld; GNU ld refuses even that, a conservatism with
                    // no correctness basis).  Non-alloc sections (DWARF)
                    // are never loaded and keep link-time values.
                    R_X86_64_32 | R_X86_64_32S | R_X86_64_16 | R_X86_64_8
                        if in_sec.flags & SHF_ALLOC != 0
                            && (!abs_target(sym, global) || binds_outside(sym, global)) =>
                    {
                        return Err(non_pic_reloc_error(obj_name, rela.rela_type, &sym.name));
                    }
                    R_X86_64_32 => {
                        w32_checked(
                            &mut out,
                            fp,
                            s as i64 + a,
                            rela.rela_type,
                            &sym.name,
                            &objects[obj_idx].source_name,
                        )?;
                    }
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
                        w32_checked(
                            &mut out,
                            fp,
                            s as i64 + a,
                            rela.rela_type,
                            &sym.name,
                            &objects[obj_idx].source_name,
                        )?;
                    }
                    t if is_gotpcrel_family(t) => {
                        // Same predicates, same ORIGINAL bytes as the slot
                        // scan: a reference is either rewritten to address
                        // its target directly or has a slot.  The PIC forms
                        // only (`lea`, `addr32 call`, `jmp`).
                        let target = match global {
                            Some(g) => got_target(&sym.name, g),
                            None if sym.is_local() => Some(local_got_target(sym)),
                            None => None,
                        };
                        let relax = target.and_then(|target| {
                            gotpcrelx_relaxation(
                                rela.rela_type,
                                a,
                                objects[obj_idx].section_data[sec_idx].as_slice(),
                                rela.offset as usize,
                                true,
                                target,
                            )
                        });
                        if let Some(kind) = relax {
                            let (pos, v) =
                                rewrite_got_relax(&mut out, fp, kind, rela.rela_type, s, p);
                            w32_checked(&mut out, pos, v, rela.rela_type, &sym.name, obj_name)?;
                            continue;
                        }
                        let gea = if sym.is_local() {
                            local_got
                                .get((obj_idx, si))
                                .map(|i| got_addr + local_got_base + i as u64 * 8)
                        } else {
                            got_sym_addrs.get(sym.name.as_str()).copied()
                        };
                        let Some(gea) = gea else {
                            return Err(format!(
                                "{obj_name}: internal error: no GOT slot for {} against '{}'",
                                reloc_field::name(rela.rela_type).unwrap_or("relocation"),
                                sym.name
                            ));
                        };
                        w32_checked(
                            &mut out,
                            fp,
                            gea as i64 + a - p as i64,
                            rela.rela_type,
                            &sym.name,
                            obj_name,
                        )?;
                    }
                    R_X86_64_PC64 => {
                        if in_sec.flags & SHF_ALLOC != 0
                            && (abs_target(sym, global) || binds_outside(sym, global))
                        {
                            pc_rel_to_loader(
                                &mut pcrel_entries,
                                in_sec.flags,
                                rela.rela_type,
                                p,
                                sym,
                                binds_outside(sym, global),
                                s,
                                a,
                                obj_name,
                            )?;
                            continue;
                        }
                        w64(&mut out, fp, (s as i64 + a - p as i64) as u64);
                    }
                    // ── Medium/large code model: offsets from the GOT origin
                    // (`_GLOBAL_OFFSET_TABLE_` = `got_addr` in this emitter).
                    R_X86_64_GOTPC32 => {
                        w32_checked(
                            &mut out,
                            fp,
                            got_addr as i64 + a - p as i64,
                            rela.rela_type,
                            &sym.name,
                            obj_name,
                        )?;
                    }
                    R_X86_64_GOTPC64 => {
                        w64(&mut out, fp, (got_addr as i64 + a - p as i64) as u64);
                    }
                    R_X86_64_GOTOFF64 => {
                        // S + A - GOT only means something when S is bound
                        // in this module; GNU ld refuses it for an undefined
                        // symbol for the same reason.
                        if global.is_some_and(|g| imports_undef(g) && g.defined_in.is_none()) {
                            return Err(format!(
                                "{obj_name}: relocation R_X86_64_GOTOFF64 against undefined \
                                 symbol '{}' can not be used when making a shared object",
                                sym.name
                            ));
                        }
                        w64(&mut out, fp, (s as i64 + a - got_addr as i64) as u64);
                    }
                    R_X86_64_PLTOFF64 => {
                        // L + A - GOT: the PLT entry when the call must go
                        // through ld.so (import / preemptible definition),
                        // the definition itself otherwise.
                        let l = global
                            .and_then(|g| g.plt_idx)
                            .map_or(s, |pi| plt_addr + 16 + pi as u64 * 16);
                        w64(&mut out, fp, (l as i64 + a - got_addr as i64) as u64);
                    }
                    t if is_got64_family(t) => {
                        let gea = if sym.is_local() {
                            local_got
                                .get((obj_idx, si))
                                .map(|i| got_addr + local_got_base + i as u64 * 8)
                        } else {
                            got_sym_addrs.get(sym.name.as_str()).copied()
                        };
                        let Some(gea) = gea else {
                            return Err(format!(
                                "{obj_name}: internal error: no GOT slot for {} against '{}'",
                                reloc_field::name(rela.rela_type).unwrap_or("relocation"),
                                sym.name
                            ));
                        };
                        let v = if t == R_X86_64_GOTPCREL64 {
                            gea as i64 + a - p as i64
                        } else {
                            gea as i64 - got_addr as i64 + a
                        };
                        w64(&mut out, fp, v as u64);
                    }
                    t if is_gottpoff_family(t) => {
                        // Initial-Exec: the instruction reads the symbol's
                        // thread-pointer offset from its GOT slot, which
                        // ld.so fills (R_X86_64_TPOFF64).  Never relaxed to
                        // Local-Exec here: a shared library's TLS block
                        // offset is unknown until load time.
                        let gea = if sym.is_local() {
                            local_ie
                                .get((obj_idx, si))
                                .map(|i| got_addr + local_ie_base + i as u64 * 8)
                        } else {
                            got_sym_addrs.get(sym.name.as_str()).copied()
                        };
                        let Some(gea) = gea else {
                            return Err(format!(
                                "{obj_name}: internal error: no GOT slot for {} against '{}'",
                                reloc_field::name(rela.rela_type).unwrap_or("relocation"),
                                sym.name
                            ));
                        };
                        w32_checked(
                            &mut out,
                            fp,
                            gea as i64 + a - p as i64,
                            rela.rela_type,
                            &sym.name,
                            obj_name,
                        )?;
                    }
                    R_X86_64_TPOFF32 => {
                        // Local-Exec addresses the variable at a fixed
                        // offset from the thread pointer, which only the
                        // executable's own TLS block has.  GNU ld rejects
                        // this too.
                        return Err(format!(
                            "{obj_name}: relocation R_X86_64_TPOFF32 against '{}' can not be \
                             used when making a shared object; recompile with -fPIC",
                            sym.name
                        ));
                    }
                    R_X86_64_TLSGD => {
                        // Point the lea at the (DTPMOD64, DTPOFF64) GOT pair.
                        let slot = if sym.is_local() {
                            local_gd
                                .get((obj_idx, si))
                                .map(|i| got_addr + local_gd_base + i as u64 * 16)
                        } else {
                            tlsgd_slot_addr.get(sym.name.as_str()).copied()
                        };
                        let Some(slot) = slot else {
                            return Err(format!(
                                "{obj_name}: internal error: no TLSGD GOT pair for '{}'",
                                sym.name
                            ));
                        };
                        w32_checked(
                            &mut out,
                            fp,
                            slot as i64 + a - p as i64,
                            rela.rela_type,
                            &sym.name,
                            obj_name,
                        )?;
                    }
                    t if is_tlsdesc_gotpc(t) => {
                        // `lea sym@tlsdesc(%rip), %rax` -> the descriptor.
                        let slot = if sym.is_local() {
                            local_desc
                                .get((obj_idx, si))
                                .map(|i| got_addr + local_desc_base + i as u64 * 16)
                        } else {
                            tlsdesc_slot_addr.get(sym.name.as_str()).copied()
                        };
                        let Some(slot) = slot else {
                            return Err(format!(
                                "{obj_name}: internal error: no TLS descriptor for '{}'",
                                sym.name
                            ));
                        };
                        w32_checked(
                            &mut out,
                            fp,
                            slot as i64 + a - p as i64,
                            rela.rela_type,
                            &sym.name,
                            obj_name,
                        )?;
                    }
                    // `call *(%rax)` through the descriptor stays as is.
                    R_X86_64_TLSDESC_CALL => {}
                    R_X86_64_TLSLD => {
                        if let Some(slot) = tlsld_slot {
                            w32_checked(
                                &mut out,
                                fp,
                                slot as i64 + a - p as i64,
                                rela.rela_type,
                                &sym.name,
                                &objects[obj_idx].source_name,
                            )?;
                        }
                    }
                    R_X86_64_DTPOFF32 => {
                        // Offset of the symbol within this module's TLS block.
                        let dtpoff = s as i64 - tls_addr as i64;
                        w32_checked(
                            &mut out,
                            fp,
                            dtpoff + a,
                            rela.rela_type,
                            &sym.name,
                            &objects[obj_idx].source_name,
                        )?;
                    }
                    R_X86_64_DTPOFF64 => {
                        let dtpoff = s as i64 - tls_addr as i64;
                        w64(&mut out, fp, (dtpoff + a) as u64);
                    }
                    R_X86_64_SIZE32 => {
                        let size = global.map_or(sym.size, |g| g.size);
                        w32_checked(
                            &mut out,
                            fp,
                            size as i64 + a,
                            rela.rela_type,
                            &sym.name,
                            obj_name,
                        )?;
                    }
                    R_X86_64_SIZE64 => {
                        let size = global.map_or(sym.size, |g| g.size);
                        w64(&mut out, fp, (size as i64 + a) as u64);
                    }
                    R_X86_64_NONE | R_X86_64_GNU_VTINHERIT | R_X86_64_GNU_VTENTRY => {}
                    other => {
                        // Leaving the field unrelocated produced a library
                        // that loads and then misbehaves; refuse instead.
                        return Err(format!(
                            "{obj_name}: unsupported relocation {} ({other}) against '{}' in \
                             a shared library",
                            reloc_field::name(other).unwrap_or("type"),
                            sym.name
                        ));
                    }
                }
            }
        }
    }

    // Build .eh_frame_hdr from the RELOCATED .eh_frame (initial_location
    // fields are only meaningful after the PC32 relocations were applied).
    if eh_frame_hdr_size > 0
        && let Some(ef) = output_sections
            .iter()
            .find(|s| s.name == ".eh_frame" && s.mem_size > 0)
    {
        let ef_start = ef.file_offset as usize;
        let ef_end = ef_start + ef.mem_size as usize;
        if ef_end <= out.len() {
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

    // Write .rela.dyn entries
    // Symbol index of a dynamic relocation: "" is symbol 0; any other name
    // must be in `.dynsym`.  A lookup miss used to fall back to index 0 --
    // the null symbol -- silently turning an import into a module-relative
    // value; it is an internal inconsistency and is reported as one.
    let dyn_sym_ref = |name: &str| -> Result<u64, String> {
        if name.is_empty() {
            return Ok(0);
        }
        dyn_sym_index.get(name).copied().ok_or_else(|| {
            format!("internal error: dynamic relocation against '{name}', which is not in .dynsym")
        })
    };
    let relative_count = rela_dyn_entries.len();
    let total_rela_count = relative_count
        + glob_dat_entries.len()
        + tpoff64_entries.len()
        + tlsdesc_entries.len()
        + abs64_entries.len()
        + pcrel_entries.len()
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
        let si = dyn_sym_ref(sym_name)?;
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
    // Symbol 0 + addend for a TLS symbol bound inside this library.
    for (rel_offset, sym_name, addend) in &tpoff64_entries {
        let si = dyn_sym_ref(sym_name)?;
        if rd + 24 <= out.len() {
            w64(&mut out, rd, *rel_offset); // r_offset = GOT entry address
            w64(&mut out, rd + 8, (si << 32) | R_X86_64_TPOFF64 as u64);
            w64(&mut out, rd + 16, *addend);
            rd += 24;
        }
    }
    // TLS descriptors, same symbol-0 convention.
    for (rel_offset, sym_name, addend) in &tlsdesc_entries {
        let si = dyn_sym_ref(sym_name)?;
        if rd + 24 <= out.len() {
            w64(&mut out, rd, *rel_offset);
            w64(&mut out, rd + 8, (si << 32) | R_X86_64_TLSDESC as u64);
            w64(&mut out, rd + 16, *addend);
            rd += 24;
        }
    }
    // TLS module/offset relocations for General/Local-Dynamic GOT pairs.
    // DTPMOD64 with sym 0 = "this module"; ld.so writes the module id.
    for (rel_offset, sym_name) in &dtpmod64_entries {
        let si = dyn_sym_ref(sym_name)?;
        if rd + 24 <= out.len() {
            w64(&mut out, rd, *rel_offset);
            w64(&mut out, rd + 8, (si << 32) | R_X86_64_DTPMOD64 as u64);
            w64(&mut out, rd + 16, 0);
            rd += 24;
        }
    }
    for (rel_offset, sym_name) in &dtpoff64_entries {
        let si = dyn_sym_ref(sym_name)?;
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
        let si = dyn_sym_ref(sym_name)?;
        if rd + 24 <= out.len() {
            w64(&mut out, rd, *rel_offset); // r_offset
            w64(&mut out, rd + 8, (si << 32) | R_X86_64_64 as u64);
            w64(&mut out, rd + 16, *addend as u64); // r_addend
            rd += 24;
        }
    }
    // PC-relative fields ld.so computes (`S + A - P`, overflow-checked by
    // glibc for PC32).
    for (rel_offset, rtype, sym_name, addend) in &pcrel_entries {
        let si = dyn_sym_ref(sym_name)?;
        if rd + 24 <= out.len() {
            w64(&mut out, rd, *rel_offset);
            w64(&mut out, rd + 8, (si << 32) | *rtype as u64);
            w64(&mut out, rd + 16, *addend as u64);
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
    // DT_INIT/DT_FINI: the .init/.fini section addresses, right after SONAME
    // as in bfd.  The lookups cannot miss (`has_init`/`has_fini` above test
    // the same predicate on the same section list).
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
        (DT_RELA, rela_dyn_addr),
        (DT_RELASZ, rela_dyn_size),
        (DT_RELAENT, 24),
        // DT_RELACOUNT stays unconditional here (unlike the exec writer):
        // the RELATIVE entries are only discovered during relocation
        // processing, long after dyn_count must be final, so no exact
        // early predicate exists; ld.so treats a zero count exactly like
        // an absent tag, and every real-world .so has relatives anyway.
        (DT_RELACOUNT, relative_count as u64),
        // DT_TEXTREL not needed since we use PIC
    ] {
        w64(&mut out, dd, tag as u64);
        w64(&mut out, dd + 8, val);
        dd += 16;
    }
    // Hash tables, GNU first: a loader that understands DT_GNU_HASH prefers it
    // and ignores DT_HASH, which is what `--hash-style=both` is for.
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
    if dt_symbolic {
        // DT_SYMBOLIC (legacy) + DT_FLAGS:DF_SYMBOLIC (modern): search the
        // library itself before the global scope at run time.  Only for
        // `-Bsymbolic`: under `-Bsymbolic-functions` data must still be
        // looked up globally.
        w64(&mut out, dd, 16u64);
        w64(&mut out, dd + 8, 0);
        dd += 16; // DT_SYMBOLIC
    }
    if dt_flags != 0 {
        w64(&mut out, dd, DT_FLAGS as u64);
        w64(&mut out, dd + 8, dt_flags);
        dd += 16;
    }
    if dt_flags_1 != 0 {
        w64(&mut out, dd, 0x6fff_fffb); // DT_FLAGS_1
        w64(&mut out, dd + 8, dt_flags_1);
        dd += 16;
    }
    if verdef_count > 0 || verneed_count > 0 {
        w64(&mut out, dd, DT_VERSYM as u64);
        w64(&mut out, dd + 8, versym_addr);
        dd += 16;
    }
    if verdef_count > 0 {
        w64(&mut out, dd, DT_VERDEF as u64);
        w64(&mut out, dd + 8, verdef_addr);
        dd += 16;
        w64(&mut out, dd, DT_VERDEFNUM as u64);
        w64(&mut out, dd + 8, verdef_count);
        dd += 16;
    }
    if verneed_count > 0 {
        w64(&mut out, dd, DT_VERNEED as u64);
        w64(&mut out, dd + 8, verneed_addr);
        dd += 16;
        w64(&mut out, dd, DT_VERNEEDNUM as u64);
        w64(&mut out, dd + 8, verneed_count);
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
    if rela_plt_size > 0 {
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
    // Merged `.comment` (compiler version strings, deduplicated).  Non-alloc
    // metadata: section header plus file bytes, no address, no segment.
    let comment_data = linker_common::merge_comment_sections(objects);

    // Build .shstrtab string table
    let mut shstrtab = vec![0u8]; // null byte at offset 0
    let mut shstr_offsets: FxHashMap<String, u32> = FxHashMap::default();
    let known_names = [
        ".gnu.hash",
        ".hash",
        ".dynsym",
        ".dynstr",
        ".gnu.version",
        ".gnu.version_d",
        ".gnu.version_r",
        ".rela.dyn",
        ".rela.plt",
        ".plt",
        ".eh_frame_hdr",
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

    // Helper: write a 64-byte ELF64 section header
    // Use shared write_elf64_shdr from linker_common (aliased locally for brevity)
    let write_shdr_so = linker_common::write_elf64_shdr;

    // Pre-count section indices for cross-references
    // Numbered, not hardcoded: `--hash-style=sysv` swaps `.gnu.hash` for
    // `.hash`, and with both there are two hash headers before `.dynsym`.
    let dynsym_shidx: u32 = 1 + want_gnu_hash as u32 + want_sysv_hash as u32;
    // Derived, not hardcoded: `.dynstr` always follows `.dynsym`, whose own
    // index depends on how many hash tables precede it.
    let dynstr_shidx: u32 = dynsym_shidx + 1;

    // Map merged output sections to their final section-header indices.
    let mut out_sec_to_hdr: FxHashMap<usize, u16> = FxHashMap::default();
    // Must agree with `sh_count` above: NULL + .dynsym + .dynstr, plus one
    // header per hash table emitted.  This was hardcoded to 4 (NULL +
    // .gnu.hash + .dynsym + .dynstr), so `--hash-style=both` put `.symtab`'s
    // sh_link on `.symtab` itself instead of `.strtab`.
    let mut next_hdr = 3usize + want_gnu_hash as usize + want_sysv_hash as usize;
    if versym_size > 0 {
        next_hdr += 1;
    }
    if verdef_size > 0 {
        next_hdr += 1;
    }
    if verneed_size > 0 {
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
    if eh_frame_hdr_size > 0 {
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
    // .comment sits before .symtab, as in bfd.
    if !comment_data.is_empty() {
        next_hdr += 1;
    }
    let symtab_shidx = next_hdr as u16;
    let strtab_shidx = symtab_shidx + 1;

    // Section-header index of a global definition, for `.dynsym` and
    // `.symtab`.  Consumers DO read it: GNU ld decides where an executable's
    // COPY of a data object goes from the flags of the section the DSO says
    // it lives in -- a read-only one sends it to `.data.rel.ro`, which is
    // write-protected after relocation, so the first store to the variable
    // faults.  (Every definition used to claim index 1, `.gnu.hash`.)
    // COMMON symbols (section_idx 0xffff after allocation) live in `.bss`;
    // a definition whose input section has no header of its own (a
    // `--defsym` alias, a linker-provided symbol) takes the section whose
    // address range holds it.
    let bss_out = output_sections.iter().position(|s| s.name == ".bss");
    let def_shndx = |name: &str, g: &GlobalSymbol| -> u16 {
        if g.absolute {
            return SHN_ABS;
        }
        let mapped = if g.section_idx == 0xffff {
            bss_out
        } else {
            g.defined_in
                .and_then(|o| section_map.get(&(o, g.section_idx as usize)))
                .map(|&(oi, _)| oi)
        };
        if let Some(h) = mapped.and_then(|oi| out_sec_to_hdr.get(&oi)) {
            return *h;
        }
        // `__stop_SEC` is one past SEC's end, which is usually also the
        // start of the next section; it belongs to SEC (as in GNU ld), so
        // name-match the encapsulation anchors before falling back to
        // address containment.
        if linker_provided.contains(name)
            && let Some(sec_name) = name
                .strip_prefix("__start_")
                .or_else(|| name.strip_prefix("__stop_"))
            && let Some((_, &h)) = out_sec_to_hdr
                .iter()
                .find(|&(&oi, _)| output_sections[oi].name == sec_name)
        {
            return h;
        }
        out_sec_to_hdr
            .iter()
            .filter(|&(&oi, _)| {
                let sec = &output_sections[oi];
                sec.addr <= g.value && g.value <= sec.addr + sec.mem_size
            })
            .max_by_key(|&(&oi, _)| output_sections[oi].addr)
            .map(|(_, &h)| h)
            .unwrap_or(dynsym_shidx as u16)
    };
    {
        let mut ds = dynsym_offset as usize + 24;
        for name in &dyn_sym_names {
            if let Some(g) = globals.get(name)
                && g.defined_in.is_some()
                && !g.is_dynamic
                && g.section_idx != SHN_UNDEF
            {
                w16(&mut out, ds + 6, def_shndx(name, g));
                if linker_provided.contains(name.as_str()) {
                    out[ds + 5] = STV_PROTECTED;
                }
            }
            ds += 24;
        }
    }

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
        let mut value = output_sections[oi].addr + sec_off + sym.value;
        if sym.sym_type() == STT_TLS {
            value = value.wrapping_sub(tls_addr);
        }
        entry[8..16].copy_from_slice(&value.to_le_bytes());
        entry[16..24].copy_from_slice(&sym.size.to_le_bytes());
        symtab_entries.push(entry);
    }
    let mut global_names: Vec<(&String, &GlobalSymbol)> = globals
        .iter()
        .filter(|(_, sym)| {
            sym.defined_in.is_some()
                && !sym.is_dynamic
                // Same rule as `emit_exec`: a LOCAL-binding entry that is
                // section-defined and laid out is already emitted by the
                // locals loop above (synthetic `<string-merge>` pool symbols
                // live in both the object symbol lists and the resolved
                // map). Emitting it here too duplicates the symbol and breaks
                // the locals-before-globals order.
                && !matches!(sym.defined_in,
                    Some(obj_idx)
                        if (sym.info >> 4) == STB_LOCAL
                            && sym.section_idx != SHN_ABS
                            && sym.section_idx != SHN_COMMON
                            && section_map.contains_key(&(obj_idx, sym.section_idx as usize)))
        })
        .collect();
    global_names.sort_by(|a, b| a.0.cmp(b.0));
    for (name, sym) in global_names {
        let name_off = push_strtab_name(&mut symtab_names, name.as_bytes());
        let shndx = def_shndx(name, sym);
        // GNU ld demotes HIDDEN/INTERNAL definitions to STB_LOCAL in the
        // output's `.symtab` (keeping st_other); the stable partition sort
        // below moves them in front of the globals.
        let info = if linker_common::is_dynamic_visibility(sym.visibility) {
            sym.info
        } else {
            (STB_LOCAL << 4) | (sym.info & 0xf)
        };
        let mut entry = [0u8; 24];
        entry[0..4].copy_from_slice(&name_off.to_le_bytes());
        entry[4] = info;
        entry[5] = sym.visibility;
        entry[6..8].copy_from_slice(&shndx.to_le_bytes());
        // gABI: a TLS symbol's st_value in an executable or shared object
        // is its offset in the TLS template, not a virtual address.
        let value = if (sym.info & 0xf) == STT_TLS {
            sym.value.wrapping_sub(tls_addr)
        } else {
            sym.value
        };
        entry[8..16].copy_from_slice(&value.to_le_bytes());
        entry[16..24].copy_from_slice(&sym.size.to_le_bytes());
        symtab_entries.push(entry);
    }

    // ELF mandates every STB_LOCAL entry before the first global, and
    // `sh_info` must name the first global index.  Snapshotting
    // `symtab_entries.len()` between the loops is only correct while the
    // globals loop never appends a STB_LOCAL entry — and it can (see the
    // filter above).  Enforce the partition structurally instead: a STABLE
    // sort keeps the NULL at index 0 and preserves each loop's order
    // within its run, then `first_global` is derived from the finished
    // table.  Same treatment as `emit_exec`.
    symtab_entries.sort_by_key(|e| (e[4] >> 4) != STB_LOCAL);
    let first_global = symtab_entries
        .iter()
        .filter(|e| (e[4] >> 4) == STB_LOCAL)
        .count() as u32;

    // Count total sections to determine .shstrtab index
    // NULL + .dynsym + .dynstr, plus one header per hash table emitted.
    let mut sh_count: u16 = 3 + want_gnu_hash as u16 + want_sysv_hash as u16;
    if versym_size > 0 {
        sh_count += 1;
    }
    if verdef_size > 0 {
        sh_count += 1;
    }
    if verneed_size > 0 {
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
    if eh_frame_hdr_size > 0 {
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
    if !comment_data.is_empty() {
        sh_count += 1;
    } // .comment
    sh_count += 2; // .symtab + .strtab
    debug_assert_eq!(symtab_shidx, sh_count - 2);
    debug_assert_eq!(strtab_shidx, sh_count - 1);
    let shstrtab_shidx = sh_count; // .shstrtab is the last section
    sh_count += 1;

    // .comment data first (alignment 1, no padding needed), then the
    // 8-aligned .symtab + .strtab data.
    let comment_data_offset = out.len() as u64;
    out.extend_from_slice(&comment_data);
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
    if want_gnu_hash {
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
    }
    // .hash (SysV).  sh_info is unspecified for SHT_HASH by the ELF spec; GNU ld
    // and every loader in the wild use 0, and readelf warns on anything else.
    // (The "first global symbol index" reading of that field is SHT_DYNSYM's.)
    if let Some(sh) = &sysv_hash {
        write_shdr_so(
            &mut out,
            get_shname(".hash"),
            SHT_HASH,
            SHF_ALLOC,
            sysv_hash_addr,
            sysv_hash_offset,
            sh.size(),
            dynsym_shidx,
            0,
            4,
            0,
        );
    }
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
    if verneed_size > 0 {
        write_shdr_so(
            &mut out,
            get_shname(".gnu.version_r"),
            SHT_GNU_VERNEED,
            SHF_ALLOC,
            verneed_addr,
            verneed_offset,
            verneed_size,
            dynstr_shidx,
            verneed_count as u32,
            4,
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
    if eh_frame_hdr_size > 0 {
        write_shdr_so(
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
    // .comment (merged compiler version strings; non-alloc, addr 0).
    if !comment_data.is_empty() {
        write_shdr_so(
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
