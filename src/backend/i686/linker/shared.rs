//! Shared library (.so) emission for the i686 linker.
//!
//! # Binding model
//!
//! A reference inside the shared object either *binds locally* — it is
//! resolved at link time and at most needs the load bias added
//! (`R_386_RELATIVE`) — or stays *preemptible* and is bound by the dynamic
//! loader through a symbolic dynamic relocation, the GOT or the PLT.  A
//! definition binds locally when it is `STB_LOCAL`, has non-default
//! visibility (hidden, internal or protected, merged over every reference),
//! or `-Bsymbolic` (all definitions) / `-Bsymbolic-functions` (functions)
//! applies; undefined symbols and other default-visibility definitions are
//! preemptible, exactly as the gABI and GNU ld specify.  (The previous
//! emitter bound every definition locally, which silently broke symbol
//! interposition and `LD_PRELOAD`, and dropped references to undefined data
//! entirely.)
//!
//! Every relocation is classified once by [`plan`]; the sizing pass and the
//! application pass both run that one function, so the dynamic relocation
//! count reserved before layout and the table written after it agree by
//! construction (and are still checked).
//!
//! # TLS
//!
//! General-dynamic (`R_386_TLS_GD`), local-dynamic (`R_386_TLS_LDM` +
//! `R_386_TLS_LDO_32`), initial-exec (`R_386_TLS_IE`, `R_386_TLS_GOTIE`),
//! descriptors (`R_386_TLS_GOTDESC`/`R_386_TLS_DESC_CALL`) and local-exec in
//! a DSO (dynamic `R_386_TLS_TPOFF` at the site) are all linked with the
//! dynamic relocations glibc's `elf_machine_rel` expects; any static-TLS
//! model sets `DF_STATIC_TLS`.  Link-time offsets are relative to this
//! object's PT_TLS block — its placement in the thread's TLS area is only
//! known to the loader, which is why no DSO access may be resolved to a
//! final thread-pointer offset here.
//!
//! # Layout
//!
//! ```text
//! R    ELF header, phdrs, notes (+ build-id), .hash, .gnu.hash, .dynsym,
//!      .dynstr, .rel.dyn, .rel.plt
//! RX   .init .plt .text .fini <custom exec>
//! R    .rodata .eh_frame .eh_frame_hdr <custom ro>
//! RW   [RELRO: .tdata/.tbss .init_array .fini_array .data.rel.ro .dynamic
//!       .got (.got.plt with -z now)] .got.plt <custom rw> .data .bss
//! ```

use crate::common::fx_hash::{FxHashMap, FxHashSet};

use super::DynStrTab;
use super::dynamic::{DynamicDesc, image_function_addr, option_entries};
use super::emit::{
    PltAddressing, build_plt, check_sections_placed, inputs_want_exec_stack,
    layout_custom_sections, layout_section, layout_tls,
};
use super::gnu_hash::build_gnu_hash_32;
use super::options::{LinkOptions, Symbolic};
use super::shdr::{self, Shdr32, StdSection};
use super::types::*;
use crate::backend::linker_common;

/// Identity of a relocation target: named global, or an object-local symbol.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum SymKey {
    Global(String),
    Local(usize, usize),
}

/// A relocation target, classified.
#[derive(Clone, Debug)]
struct Target {
    key: SymKey,
    name: String,
    /// Resolved at link time (see the module documentation).
    local: bool,
    /// Defined in this image (a section it lives in was kept, or ABS).
    defined: bool,
    /// Absolute value: must not receive the load bias.
    abs: bool,
}

/// What a GOT entry holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum GotKind {
    /// The symbol's address (1 word).
    Addr,
    /// `tls_index` {module, offset} for `___tls_get_addr` (2 words).
    TlsGd,
    /// Thread-pointer offset (1 word).
    TlsIe,
    /// TLS descriptor {entry, argument} (2 words).
    TlsDesc,
}

impl GotKind {
    fn words(self) -> u32 {
        match self {
            GotKind::Addr | GotKind::TlsIe => 1,
            GotKind::TlsGd | GotKind::TlsDesc => 2,
        }
    }
}

/// How one input relocation is linked.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Plan {
    Nothing,
    /// `S + A`; `relative` adds an `R_386_RELATIVE` at the site.
    Abs32 {
        relative: bool,
    },
    /// A symbolic dynamic relocation of this type at the site; the site
    /// holds the implicit addend.
    DynSite(u32),
    /// `S + A - P`.
    Pc32,
    /// `L + A - P` through this symbol's PLT entry.
    Plt,
    /// `G + A - GOT` (slot offset from `_GLOBAL_OFFSET_TABLE_`).
    GotSlot(GotKind),
    /// `G + A`: the slot's absolute address (+ `R_386_RELATIVE` at the site).
    GotSlotAbs(GotKind),
    /// `mov x@GOT(%reg)` relaxed to `lea x@GOTOFF(%reg)`: `S + A - GOT`.
    GotRelaxLea,
    /// `S + A - GOT`.
    GotOff,
    /// `GOT + A - P`.
    GotPc,
    /// The module's local-dynamic `tls_index` pair.
    TlsLdm,
    /// `S + A - TLS block start`.
    TlsLdo,
    /// `Z + A`.
    Size32,
    /// 8/16-bit field: (width, pc-relative).
    Narrow(u32, bool),
    Error(String),
}

/// Whether a plan uses a static-TLS model.
fn is_static_tls(rel_type: u32) -> bool {
    matches!(
        rel_type,
        R_386_TLS_IE
            | R_386_TLS_GOTIE
            | R_386_TLS_LE
            | R_386_TLS_LE_32
            | R_386_TLS_TPOFF
            | R_386_TLS_TPOFF32
    )
}

fn reloc_name(t: u32) -> String {
    format!("R_386 type {t}")
}

/// Classify one relocation.  `data`/`off` are the input section bytes and
/// the field offset (for the instruction-dependent GOT32X forms).
fn plan(rel_type: u32, t: &Target, data: &[u8], off: usize) -> Plan {
    let not_in_dso = || {
        Plan::Error(format!(
            "relocation {} against `{}' can not be used when making a shared object; recompile with -fPIC",
            reloc_name(rel_type),
            t.name
        ))
    };
    match rel_type {
        R_386_NONE | R_386_TLS_DESC_CALL => Plan::Nothing,
        R_386_32 => {
            if t.local {
                Plan::Abs32 {
                    relative: t.defined && !t.abs,
                }
            } else {
                Plan::DynSite(R_386_32)
            }
        }
        R_386_PC32 => {
            if t.local {
                Plan::Pc32
            } else {
                Plan::DynSite(R_386_PC32)
            }
        }
        R_386_PLT32 => {
            if t.local {
                Plan::Pc32
            } else {
                Plan::Plt
            }
        }
        R_386_GOT32 | R_386_GOT32X => {
            // `8b modrm disp32` with modrm = mod:2 reg:3 rm:3.  mod=00,rm=101
            // is the base-less `disp32` form (non-PIC `movl x@GOT, %r`).
            let opcode = off.checked_sub(2).and_then(|i| data.get(i)).copied();
            let modrm = off.checked_sub(1).and_then(|i| data.get(i)).copied();
            let baseless = modrm.is_some_and(|m| m >> 6 == 0 && m & 7 == 5);
            if rel_type == R_386_GOT32X && baseless {
                Plan::GotSlotAbs(GotKind::Addr)
            } else if baseless {
                not_in_dso()
            } else if rel_type == R_386_GOT32X
                && t.local
                && t.defined
                && !t.abs
                && opcode == Some(0x8b)
                && modrm.is_some_and(|m| m >> 6 == 2 && m & 7 != 4)
            {
                Plan::GotRelaxLea
            } else {
                Plan::GotSlot(GotKind::Addr)
            }
        }
        R_386_GOTOFF => {
            if t.defined {
                Plan::GotOff
            } else {
                Plan::Error(format!(
                    "relocation R_386_GOTOFF against undefined symbol `{}' can not be used when making a shared object",
                    t.name
                ))
            }
        }
        R_386_GOTPC => Plan::GotPc,
        R_386_TLS_GD => Plan::GotSlot(GotKind::TlsGd),
        R_386_TLS_LDM => Plan::TlsLdm,
        R_386_TLS_LDO_32 => {
            if t.local && t.defined {
                Plan::TlsLdo
            } else {
                Plan::Error(format!(
                    "local-dynamic TLS offset (R_386_TLS_LDO_32) against `{}', which is not a TLS symbol of this object",
                    t.name
                ))
            }
        }
        R_386_TLS_IE => Plan::GotSlotAbs(GotKind::TlsIe),
        R_386_TLS_GOTIE => Plan::GotSlot(GotKind::TlsIe),
        // Local-exec in a DSO: the thread-pointer offset is only known at
        // load time.  (The i686 backend reads R_386_TLS_LE_32/TPOFF32 as
        // the negative offset, like R_386_TLS_LE; see reloc.rs.)
        R_386_TLS_LE | R_386_TLS_LE_32 | R_386_TLS_TPOFF | R_386_TLS_TPOFF32 => {
            Plan::DynSite(R_386_TLS_TPOFF)
        }
        R_386_TLS_GOTDESC => Plan::GotSlot(GotKind::TlsDesc),
        R_386_SIZE32 => Plan::Size32,
        R_386_16 | R_386_8 => {
            if t.local && (t.abs || !t.defined) {
                Plan::Narrow(if rel_type == R_386_16 { 16 } else { 8 }, false)
            } else {
                not_in_dso()
            }
        }
        R_386_PC16 | R_386_PC8 => {
            if t.local {
                Plan::Narrow(if rel_type == R_386_PC16 { 16 } else { 8 }, true)
            } else {
                not_in_dso()
            }
        }
        other => Plan::Error(format!(
            "unsupported relocation {} against `{}' in a shared object",
            reloc_name(other),
            t.name
        )),
    }
}

/// Names the emitter defines itself when no input does: the layout
/// anchors of `get_standard_linker_symbols` (`_GLOBAL_OFFSET_TABLE_`,
/// `_DYNAMIC`, `_end`, the init/fini array bounds, `__dso_handle`, ...)
/// and `__start_SEC` / `__stop_SEC` for an existing, C-identifier-named
/// output section (GNU ld).  Deliberately NOT the broader
/// `is_linker_defined_symbol` list: that also names runtime functions
/// (`___tls_get_addr`, `_Unwind_Resume`, `_ITM_*TMCloneTable`, ...) that
/// must be imported, and binding those locally to 0 breaks e.g. crtbeginS's
/// weak `_ITM_registerTMCloneTable` test.
pub(super) fn is_emitter_defined(name: &str, sections: &FxHashMap<String, usize>) -> bool {
    linker_common::defsym::is_linker_defined(name)
        || name
            .strip_prefix("__start_")
            .or_else(|| name.strip_prefix("__stop_"))
            .is_some_and(|sec| {
                linker_common::is_valid_c_identifier_for_section(sec) && sections.contains_key(sec)
            })
}

/// Whether a defined global binds locally (module documentation).
fn binds_locally(gs: &LinkerSymbol, symbolic: Symbolic) -> bool {
    gs.binding == STB_LOCAL || gs.visibility != STV_DEFAULT || symbolic.binds_locally(gs.sym_type)
}

fn binds_locally_opts(name: &str, gs: &LinkerSymbol, opts: &LinkOptions) -> bool {
    if gs.binding == STB_LOCAL || gs.visibility != STV_DEFAULT {
        return true;
    }
    if opts.is_pie {
        return true;
    }
    if opts.dynamic_list.matches(name) {
        return false;
    }
    opts.symbolic.binds_locally(gs.sym_type)
}

fn classify(
    obj_idx: usize,
    sym_idx: usize,
    sym: &InputSymbol,
    global_symbols: &FxHashMap<String, LinkerSymbol>,
    section_map: &SectionMap,
    sections: &FxHashMap<String, usize>,
    opts: &LinkOptions,
) -> Target {
    let section_kept = |shndx: u16| section_map.contains_key(&(obj_idx, shndx as usize));
    if sym.sym_type == STT_SECTION || sym.binding == STB_LOCAL || sym.name.is_empty() {
        let abs = sym.section_index == SHN_ABS;
        return Target {
            key: SymKey::Local(obj_idx, sym_idx),
            name: sym.name.clone(),
            local: true,
            defined: abs || section_kept(sym.section_index),
            abs,
        };
    }
    let key = SymKey::Global(sym.name.clone());
    match global_symbols.get(sym.name.as_str()) {
        Some(gs) if gs.is_defined => Target {
            key,
            name: sym.name.clone(),
            local: binds_locally_opts(&sym.name, gs, opts),
            defined: true,
            abs: gs.output_section == usize::MAX && !is_emitter_defined(&sym.name, sections),
        },
        // A weak undefined non-default-visibility reference resolves to 0
        // inside the object (it can never be bound elsewhere).
        Some(gs)
            if sym.binding == STB_WEAK
                && (gs.visibility != STV_DEFAULT || sym.visibility != STV_DEFAULT) =>
        {
            Target {
                key,
                name: sym.name.clone(),
                local: true,
                defined: false,
                abs: true,
            }
        }
        _ if is_emitter_defined(&sym.name, sections) => Target {
            key,
            name: sym.name.clone(),
            local: true,
            defined: true,
            abs: false,
        },
        _ => Target {
            key,
            name: sym.name.clone(),
            local: false,
            defined: false,
            abs: false,
        },
    }
}

/// Value of an input symbol after layout.
fn symbol_value(
    obj_idx: usize,
    sym: &InputSymbol,
    global_symbols: &FxHashMap<String, LinkerSymbol>,
    section_map: &SectionMap,
    output_sections: &[OutputSection],
) -> u32 {
    let in_section = |extra: u32| match section_map.get(&(obj_idx, sym.section_index as usize)) {
        Some(&(out_idx, out_off)) => output_sections[out_idx].addr + out_off + extra,
        None => extra,
    };
    if sym.sym_type == STT_SECTION {
        return match sym.section_index {
            SHN_UNDEF | SHN_ABS => 0,
            _ => in_section(0),
        };
    }
    if sym.binding == STB_LOCAL || sym.name.is_empty() {
        return match sym.section_index {
            SHN_UNDEF => 0,
            SHN_ABS => sym.value,
            _ => in_section(sym.value),
        };
    }
    match global_symbols.get(sym.name.as_str()) {
        Some(gs) => gs.address,
        None => 0,
    }
}

/// One dynamic relocation: (offset, type, dynsym index).
type DynRel = (u32, u32, u32);

fn write32(data: &mut [u8], off: usize, v: u32) -> Result<(), String> {
    match data.get_mut(off..off + 4) {
        Some(slot) => {
            slot.copy_from_slice(&v.to_le_bytes());
            Ok(())
        }
        None => Err(format!(
            "relocation field at 0x{off:x} is outside its section"
        )),
    }
}

fn build_shared_verneed_32(
    imports: &[String],
    import_set: &FxHashSet<String>,
    dynsym_names: &[String],
    needed_sonames: &[String],
    dynlib_syms: &super::input::DynlibSyms,
    dynstr: &mut DynStrTab,
) -> Result<(Vec<u8>, Vec<u8>, u32), String> {
    let verneed_plan = linker_common::symver::VerneedPlan::new(
        imports.iter().filter_map(|n| {
            let d = dynlib_syms.get(n.as_str())?;
            Some((d.lib.as_str(), d.version.as_deref()?))
        }),
        needed_sonames,
        2,
    )?;
    if verneed_plan.is_empty() {
        return Ok((Vec::new(), Vec::new(), 0));
    }
    for s in verneed_plan.strings() {
        dynstr.add(s);
    }
    let verneed_data = verneed_plan.encode(|s| dynstr.get_offset(s));
    let verneed_count = verneed_plan.file_count() as u32;
    let mut versym_data: Vec<u8> = Vec::with_capacity((dynsym_names.len() + 1) * 2);
    versym_data.extend_from_slice(&0u16.to_le_bytes());
    for name in dynsym_names {
        let idx = if import_set.contains(name) {
            match dynlib_syms.get(name.as_str()) {
                Some(d) => verneed_plan.index_of(&d.lib, d.version.as_deref()),
                None => linker_common::symver::VER_NDX_GLOBAL,
            }
        } else {
            linker_common::symver::VER_NDX_GLOBAL
        };
        versym_data.extend_from_slice(&idx.to_le_bytes());
    }
    Ok((versym_data, verneed_data, verneed_count))
}

/// Emit an ELF32 shared library or PIE executable (`ET_DYN`, default base 0).
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_shared_library_32(
    inputs: &[InputObject],
    global_symbols: &mut FxHashMap<String, LinkerSymbol>,
    output_sections: &mut Vec<OutputSection>,
    section_name_to_idx: &FxHashMap<String, usize>,
    section_map: &SectionMap,
    needed_sonames: &[String],
    dynlib_syms: &super::input::DynlibSyms,
    interp: Option<&[u8]>,
    dso_refs: &FxHashSet<String>,
    output_path: &str,
    opts: &LinkOptions,
    pending_defsyms: &super::link::PendingDefsyms,
) -> Result<(), String> {
    if !opts.is_pie && section_name_to_idx.contains_key(".preinit_array") {
        // The loader runs DT_PREINIT_ARRAY of the executable only (gABI).
        return Err("`.preinit_array' section is not allowed in a shared object".to_string());
    }
    let sections = section_name_to_idx;

    // ── Pass 1: classify every relocation ────────────────────────────────
    let mut plt_names: Vec<String> = Vec::new();
    let mut plt_set: FxHashSet<String> = FxHashSet::default();
    let mut got_slots: Vec<(SymKey, GotKind)> = Vec::new();
    let mut got_set: FxHashSet<(SymKey, GotKind)> = FxHashSet::default();
    let mut slot_target: FxHashMap<(SymKey, GotKind), Target> = FxHashMap::default();
    let mut needs_ldm = false;
    let mut num_relative = 0usize;
    let mut num_relr_relocs = 0usize;
    let mut num_symbolic = 0usize;
    // First dynamic relocation into a read-only section (GNU's warning
    // names it); `Some` means the image needs DT_TEXTREL.
    let mut textrel_site: Option<String> = None;
    let mut static_tls = false;
    let mut imports: Vec<String> = Vec::new();
    let mut import_set: FxHashSet<String> = FxHashSet::default();
    let mut errors: Vec<String> = Vec::new();

    for (obj_idx, obj) in inputs.iter().enumerate() {
        for sec in &obj.sections {
            let Some(&(out_idx, sec_base)) = section_map.get(&(obj_idx, sec.input_index)) else {
                continue;
            };
            // Debug sections are resolved statically (`debug.rs`): nothing
            // at run time reads them, so they need no GOT slot or dynamic
            // relocation.
            if sec.relocations.is_empty() || output_sections[out_idx].flags & SHF_ALLOC == 0 {
                continue;
            }
            let writable = sec.flags & SHF_WRITE != 0;
            for &(rel_offset, rel_type, sym_idx, _addend) in &sec.relocations {
                let Some(sym) = obj.symbols.get(sym_idx as usize) else {
                    continue;
                };
                let t = classify(
                    obj_idx,
                    sym_idx as usize,
                    sym,
                    global_symbols,
                    section_map,
                    sections,
                    opts,
                );
                if !t.local && !t.defined {
                    if let SymKey::Global(n) = &t.key {
                        if import_set.insert(n.clone()) {
                            imports.push(n.clone());
                        }
                    }
                }
                static_tls |= is_static_tls(rel_type);
                let mut site_dynrel = |relative: bool| {
                    if relative {
                        num_relative += 1;
                        if opts.pack_relative_relocs {
                            output_sections[out_idx].align = output_sections[out_idx].align.max(4);
                            if ((sec_base + rel_offset) & 1) == 0 {
                                num_relr_relocs += 1;
                            }
                        }
                    } else {
                        num_symbolic += 1;
                    }
                    if !writable && textrel_site.is_none() {
                        textrel_site = Some(linker_common::TextrelPolicy::site(
                            &obj.filename,
                            match &t.key {
                                SymKey::Global(n) => Some(n.as_str()),
                                _ => None,
                            },
                            &sec.name,
                        ));
                    }
                };
                if !t.local
                    && !t.defined
                    && sym.visibility != STV_DEFAULT
                    && sym.binding != STB_WEAK
                {
                    errors.push(format!(
                        "{}: hidden symbol `{}' isn't defined",
                        obj.filename, t.name
                    ));
                    continue;
                }
                match plan(rel_type, &t, &sec.data, rel_offset as usize) {
                    Plan::Error(e) => errors.push(format!("{}: {}", obj.filename, e)),
                    Plan::Abs32 { relative: true } => site_dynrel(true),
                    Plan::DynSite(_) => site_dynrel(false),
                    Plan::Plt => {
                        if let SymKey::Global(n) = &t.key {
                            if plt_set.insert(n.clone()) {
                                plt_names.push(n.clone());
                            }
                        }
                    }
                    p @ (Plan::GotSlot(_) | Plan::GotSlotAbs(_)) => {
                        let kind = match p {
                            Plan::GotSlotAbs(k) => {
                                site_dynrel(true);
                                k
                            }
                            Plan::GotSlot(k) => k,
                            _ => unreachable!(),
                        };
                        let k = (t.key.clone(), kind);
                        if got_set.insert(k.clone()) {
                            got_slots.push(k.clone());
                            slot_target.insert(k, t.clone());
                        }
                    }
                    Plan::TlsLdm => needs_ldm = true,
                    _ => {}
                }
            }
        }
    }
    if !errors.is_empty() {
        errors.sort();
        errors.dedup();
        return Err(errors.join("\n"));
    }
    // Every undefined global the inputs reference is recorded in .dynsym,
    // even when no dynamic relocation names it: consumers (an executable
    // deciding which of its definitions to export, `--no-undefined` checks
    // of a later link) read the object's undefined dynamic symbols.
    for obj in inputs {
        for sym in &obj.symbols {
            if sym.section_index == SHN_UNDEF
                && !sym.name.is_empty()
                && sym.binding != STB_LOCAL
                && !global_symbols
                    .get(sym.name.as_str())
                    .is_some_and(|g| g.is_defined)
                && !is_emitter_defined(&sym.name, sections)
                && import_set.insert(sym.name.clone())
            {
                imports.push(sym.name.clone());
            }
        }
    }
    imports.sort();
    plt_names.sort();
    let num_plt = plt_names.len();

    // GOT slot contents: dynamic relocations per slot.
    let slot_dynrels = |k: &(SymKey, GotKind)| -> (usize, usize) {
        let t = &slot_target[k];
        match k.1 {
            GotKind::Addr if t.local => (usize::from(t.defined && !t.abs), 0),
            GotKind::Addr => (0, 1),
            GotKind::TlsGd if t.local => (0, 1), // DTPMOD32 only
            GotKind::TlsGd => (0, 2),
            GotKind::TlsIe | GotKind::TlsDesc => (0, 1),
        }
    };
    for k in &got_slots {
        let (r, s) = slot_dynrels(k);
        num_relative += r;
        if opts.pack_relative_relocs {
            num_relr_relocs += r;
        }
        num_symbolic += s;
    }
    if needs_ldm {
        num_symbolic += 1;
    }
    let num_rel_relative = num_relative - num_relr_relocs;
    let num_rel_dyn = num_rel_relative + num_symbolic;

    // GOT word offsets (from the start of .got; word 0 is reserved).
    let mut got_word: FxHashMap<(SymKey, GotKind), u32> = FxHashMap::default();
    let mut next_word = 1u32;
    for k in &got_slots {
        got_word.insert(k.clone(), next_word);
        next_word += k.1.words();
    }
    let ldm_word = next_word;
    if needs_ldm {
        next_word += 2;
    }
    let got_size = next_word * 4;

    // ── .dynsym: imports (unhashed), then exported definitions (hashed) ──
    let mut exported: Vec<String> = global_symbols
        .iter()
        .filter(|(name, gs)| {
            gs.is_defined
                && !name.is_empty()
                && (gs.binding == STB_GLOBAL || gs.binding == STB_WEAK)
                && (gs.visibility == STV_DEFAULT || gs.visibility == STV_PROTECTED)
                && gs.sym_type != STT_SECTION
                && gs.sym_type != STT_FILE
                && name.as_str() != "_GLOBAL_OFFSET_TABLE_"
                && !import_set.contains(name.as_str())
                && (!opts.is_pie
                    || opts.export_dynamic
                    || opts.dynamic_list.matches(name)
                    || dso_refs.contains(name.as_str()))
        })
        .map(|(n, _)| n.clone())
        .collect();
    exported.sort();

    let mut dynsym_names: Vec<String> = Vec::with_capacity(imports.len() + exported.len());
    dynsym_names.extend(imports.iter().cloned());
    let gnu_hash_symoffset = dynsym_names.len() + 1;
    let (gnu_hash_data, sorted_indices) = build_gnu_hash_32(&exported, gnu_hash_symoffset as u32);
    if sorted_indices.is_empty() {
        dynsym_names.extend(exported.iter().cloned());
    } else {
        dynsym_names.extend(sorted_indices.iter().map(|&i| exported[i].clone()));
    }
    let dynsym_index: FxHashMap<String, u32> = dynsym_names
        .iter()
        .enumerate()
        .map(|(i, n)| (n.clone(), i as u32 + 1))
        .collect();

    let mut dynstr = DynStrTab::new();
    let _ = dynstr.add("");
    for lib in needed_sonames {
        dynstr.add(lib);
    }
    let option_tags = opts.extra_dynamic_tags(false, !opts.is_pie, false);
    for (_, v) in &option_tags {
        if let super::options::DynValue::Str(s) = v {
            dynstr.add(s);
        }
    }
    for name in &dynsym_names {
        dynstr.add(name);
    }
    let (versym_data, verneed_data, verneed_count) = build_shared_verneed_32(
        &imports,
        &import_set,
        &dynsym_names,
        needed_sonames,
        dynlib_syms,
        &mut dynstr,
    )?;
    let dynstr_data = dynstr.as_bytes().to_vec();

    let sysv_hash = opts.hash_style.wants_sysv().then(|| {
        let names: Vec<&str> = dynsym_names.iter().map(String::as_str).collect();
        linker_common::build_sysv_hash(&names)
    });
    let want_gnu_hash = opts.hash_style.wants_gnu();

    let image_kind = if opts.is_pie {
        "PIE object"
    } else {
        "shared object"
    };
    let textrel = match &textrel_site {
        Some(site) => {
            opts.textrel.apply(site, image_kind)?;
            true
        }
        None => false,
    };

    // ── Layout ────────────────────────────────────────────────────────────
    let page_size = opts.effective_page_size();
    let page_mask = page_size - 1;
    let base_addr: u32 = opts.image_base.unwrap_or(0);
    let ehdr_size: u32 = 52;
    let phdr_size: u32 = 32;
    let has_tls = output_sections
        .iter()
        .any(|s| s.flags & SHF_TLS != 0 && s.flags & SHF_ALLOC != 0);
    let note_sec_idx = section_name_to_idx.get(".note").copied();
    let input_note_size = note_sec_idx.map_or(0, |i| output_sections[i].data.len() as u32);
    let build_id_style = opts.effective_build_id_style();
    let build_id_size = build_id_style.note_size() as u32;
    let has_notes = input_note_size + build_id_size > 0;
    // The merged GNU property note, covered by PT_GNU_PROPERTY too.
    let property_note = super::sections::property_note_extent(inputs, section_map, note_sec_idx);
    let eh_frame_sec_idx = section_name_to_idx.get(".eh_frame").copied();
    let fde_count = eh_frame_sec_idx.map_or(0, |i| {
        linker_common::count_eh_frame_fdes(&output_sections[i].data)
    });
    let eh_frame_hdr_size = if fde_count > 0 {
        (12 + 8 * fde_count) as u32
    } else {
        0
    };

    // PHDR, [INTERP], LOAD x4, DYNAMIC, GNU_STACK
    // [+ TLS, NOTE, GNU_PROPERTY, GNU_EH_FRAME, GNU_RELRO]
    let num_phdrs: u32 = 7
        + u32::from(interp.is_some())
        + u32::from(has_tls)
        + u32::from(has_notes)
        + u32::from(property_note.is_some())
        + u32::from(eh_frame_hdr_size > 0)
        + u32::from(opts.relro);
    let phdrs_total_size = num_phdrs * phdr_size;

    let mut file_offset: u32 = ehdr_size;
    let mut vaddr: u32 = base_addr + ehdr_size;
    let phdr_offset = file_offset;
    let phdr_vaddr = vaddr;
    file_offset += phdrs_total_size;
    vaddr += phdrs_total_size;

    let interp_offset = file_offset;
    let interp_vaddr = vaddr;
    let interp_size = interp.map_or(0, |b| b.len() as u32);
    file_offset += interp_size;
    vaddr += interp_size;

    // Notes: merged input `.note.*`, then the build-id note; one PT_NOTE.
    let note_align = note_sec_idx.map_or(4, |i| output_sections[i].align.max(4));
    file_offset = align_up(file_offset, note_align);
    vaddr = align_up(vaddr, note_align);
    let note_offset = file_offset;
    let note_vaddr = vaddr;
    if let Some(idx) = note_sec_idx {
        output_sections[idx].file_offset = file_offset;
        output_sections[idx].addr = vaddr;
        file_offset += input_note_size;
        vaddr += input_note_size;
    }
    file_offset = align_up(file_offset, 4);
    vaddr = align_up(vaddr, 4);
    let build_id_offset = file_offset;
    file_offset += build_id_size;
    vaddr += build_id_size;
    let note_total_size = file_offset - note_offset;

    // .hash
    let hash_offset = file_offset;
    let hash_vaddr = vaddr;
    let hash_size = sysv_hash.as_ref().map_or(0, |h| h.size() as u32);
    file_offset += hash_size;
    vaddr += hash_size;

    // .gnu.hash
    let gnu_hash_offset = file_offset;
    let gnu_hash_vaddr = vaddr;
    let gnu_hash_size = if want_gnu_hash {
        gnu_hash_data.len() as u32
    } else {
        0
    };
    file_offset += gnu_hash_size;
    vaddr += gnu_hash_size;

    // .dynsym
    let dynsym_offset = file_offset;
    let dynsym_vaddr = vaddr;
    let dynsym_size = (dynsym_names.len() as u32 + 1) * 16;
    file_offset += dynsym_size;
    vaddr += dynsym_size;

    // .dynstr
    let dynstr_offset = file_offset;
    let dynstr_vaddr = vaddr;
    let dynstr_size = dynstr_data.len() as u32;
    file_offset += dynstr_size;
    vaddr += dynstr_size;

    // .gnu.version, .gnu.version_r
    let (versym_offset, versym_vaddr, versym_size) = if !versym_data.is_empty() {
        file_offset = align_up(file_offset, 2);
        vaddr = align_up(vaddr, 2);
        let (o, a, s) = (file_offset, vaddr, versym_data.len() as u32);
        file_offset += s;
        vaddr += s;
        (o, a, s)
    } else {
        (file_offset, vaddr, 0)
    };
    let (verneed_offset, verneed_vaddr, verneed_size) = if !verneed_data.is_empty() {
        file_offset = align_up(file_offset, 4);
        vaddr = align_up(vaddr, 4);
        let (o, a, s) = (file_offset, vaddr, verneed_data.len() as u32);
        file_offset += s;
        vaddr += s;
        (o, a, s)
    } else {
        (file_offset, vaddr, 0)
    };

    // .rel.dyn, .relr.dyn, .rel.plt
    file_offset = align_up(file_offset, 4);
    vaddr = align_up(vaddr, 4);
    let rel_dyn_offset = file_offset;
    let rel_dyn_vaddr = vaddr;
    let rel_dyn_size = num_rel_dyn as u32 * 8;
    file_offset += rel_dyn_size;
    vaddr += rel_dyn_size;
    let relr_dyn_offset = file_offset;
    let relr_dyn_vaddr = vaddr;
    let relr_reserved_size = num_relr_relocs as u32 * 4;
    file_offset += relr_reserved_size;
    vaddr += relr_reserved_size;
    let rel_plt_offset = file_offset;
    let rel_plt_vaddr = vaddr;
    let rel_plt_size = num_plt as u32 * 8;
    file_offset += rel_plt_size;
    vaddr += rel_plt_size;

    let ro_headers_end = file_offset;

    // ── RX: .init .plt .text .fini <custom exec> ──
    file_offset = align_up(file_offset, page_size);
    vaddr = align_up(vaddr, page_size);
    vaddr = (vaddr & !page_mask) | (file_offset & page_mask);
    if vaddr < base_addr + file_offset {
        vaddr += page_size;
    }
    let text_seg_file_start = file_offset;
    let text_seg_vaddr_start = vaddr;
    let _ = layout_section(
        ".init",
        section_name_to_idx,
        output_sections,
        &mut file_offset,
        &mut vaddr,
        4,
    );
    let plt_entry_size: u32 = 16;
    let plt_header_size: u32 = if num_plt > 0 { 16 } else { 0 };
    let plt_total_size = plt_header_size + num_plt as u32 * plt_entry_size;
    file_offset = align_up(file_offset, 16);
    vaddr = align_up(vaddr, 16);
    let plt_offset = file_offset;
    let plt_vaddr = vaddr;
    file_offset += plt_total_size;
    vaddr += plt_total_size;
    let _ = layout_section(
        ".text",
        section_name_to_idx,
        output_sections,
        &mut file_offset,
        &mut vaddr,
        16,
    );
    let _ = layout_section(
        ".fini",
        section_name_to_idx,
        output_sections,
        &mut file_offset,
        &mut vaddr,
        4,
    );
    layout_custom_sections(
        section_name_to_idx,
        output_sections,
        &mut file_offset,
        &mut vaddr,
        SHF_EXECINSTR,
    );
    let text_seg_file_end = file_offset;
    let text_seg_vaddr_end = vaddr;

    // ── R: .rodata .eh_frame .eh_frame_hdr <custom ro> ──
    file_offset = align_up(file_offset, page_size);
    vaddr = align_up(vaddr, page_size);
    vaddr = (vaddr & !page_mask) | (file_offset & page_mask);
    if vaddr <= text_seg_vaddr_end {
        vaddr = align_up(text_seg_vaddr_end, page_size) | (file_offset & page_mask);
    }
    let rodata_seg_file_start = file_offset;
    let rodata_seg_vaddr_start = vaddr;
    let _ = layout_section(
        ".rodata",
        section_name_to_idx,
        output_sections,
        &mut file_offset,
        &mut vaddr,
        16,
    );
    let _ = layout_section(
        ".eh_frame",
        section_name_to_idx,
        output_sections,
        &mut file_offset,
        &mut vaddr,
        4,
    );
    file_offset = align_up(file_offset, 4);
    vaddr = align_up(vaddr, 4);
    let eh_frame_hdr_offset = file_offset;
    let eh_frame_hdr_vaddr = vaddr;
    file_offset += eh_frame_hdr_size;
    vaddr += eh_frame_hdr_size;
    layout_custom_sections(
        section_name_to_idx,
        output_sections,
        &mut file_offset,
        &mut vaddr,
        0,
    );
    let rodata_seg_file_end = file_offset;
    let rodata_seg_vaddr_end = vaddr;

    // ── RW ──
    file_offset = align_up(file_offset, page_size);
    vaddr = align_up(vaddr, page_size);
    vaddr = (vaddr & !page_mask) | (file_offset & page_mask);
    if vaddr <= rodata_seg_vaddr_end {
        vaddr = align_up(rodata_seg_vaddr_end, page_size) | (file_offset & page_mask);
    }
    let data_seg_file_start = file_offset;
    let data_seg_vaddr_start = vaddr;

    let gotplt_reserved: u32 = 3;
    let gotplt_size = (gotplt_reserved + num_plt as u32) * 4;
    let sec_len = |name: &str| {
        section_name_to_idx
            .get(name)
            .map_or(0, |&i| output_sections[i].data.len() as u32)
    };

    let mut dyn_numeric_tags = opts.extra_dynamic_tags(textrel, !opts.is_pie, static_tls);
    dyn_numeric_tags.retain(|(_, v)| matches!(v, super::options::DynValue::Num(_)));
    let (option_strings, _) = option_entries(&option_tags, |s| dynstr.get_offset(s));
    let (_, option_numeric) = option_entries(&dyn_numeric_tags, |s| dynstr.get_offset(s));
    let mut dyn_desc = DynamicDesc {
        needed: needed_sonames
            .iter()
            .map(|l| dynstr.get_offset(l))
            .collect(),
        strings: option_strings,
        // Presence only; the addresses are filled with the array entries.
        init: image_function_addr(global_symbols, output_sections, "_init").map(|_| 0),
        fini: image_function_addr(global_symbols, output_sections, "_fini").map(|_| 0),
        preinit_array: (opts.is_pie && sec_len(".preinit_array") > 0).then_some((0, 0)),
        init_array: (sec_len(".init_array") > 0).then_some((0, 0)),
        fini_array: (sec_len(".fini_array") > 0).then_some((0, 0)),
        hash: sysv_hash.as_ref().map(|_| hash_vaddr),
        gnu_hash: want_gnu_hash.then_some(gnu_hash_vaddr),
        strtab: dynstr_vaddr,
        symtab: dynsym_vaddr,
        strsz: dynstr_size,
        debug: opts.is_pie,
        plt: (num_plt > 0).then_some((0, rel_plt_vaddr, rel_plt_size)),
        rel: (num_rel_dyn > 0).then_some((rel_dyn_vaddr, rel_dyn_size)),
        relr: (num_relr_relocs > 0).then_some((relr_dyn_vaddr, relr_reserved_size)),
        relcount: (num_rel_relative > 0).then_some(num_rel_relative as u32),
        verneed: (verneed_size > 0).then_some((versym_vaddr, verneed_vaddr, verneed_count)),
        numeric: option_numeric,
    };
    let dynamic_size = dyn_desc.byte_size();

    // RELRO: shift the segment start so the region ends on a page boundary
    // (the loader rounds the end down); see the executable emitter.
    if opts.relro {
        let mut len = 0u32;
        let mut max_align = 4u32;
        let mut add = |name: &str, len: &mut u32| {
            if let Some(&i) = section_name_to_idx.get(name) {
                let a = output_sections[i].align.max(4);
                max_align = max_align.max(a);
                if output_sections[i].sh_type != SHT_NOBITS {
                    *len = align_up(*len, a) + output_sections[i].data.len() as u32;
                }
            }
        };
        for name in [
            ".tdata",
            ".tbss",
            ".preinit_array",
            ".init_array",
            ".fini_array",
            ".data.rel.ro",
        ] {
            if name == ".preinit_array" && !opts.is_pie {
                continue;
            }
            add(name, &mut len);
        }
        len = align_up(len, 4) + dynamic_size + got_size;
        if opts.bind_now {
            len += gotplt_size;
        }
        let pad = (page_size - len % page_size) % page_size;
        let pad = pad - pad % max_align;
        file_offset += pad;
        vaddr += pad;
    }
    let relro_offset = file_offset;
    let relro_vaddr = vaddr;

    let (tls_addr, tls_file_offset, tls_file_size, tls_mem_size, tls_align) = layout_tls(
        section_name_to_idx,
        output_sections,
        &mut file_offset,
        &mut vaddr,
    );
    let (preinit_array_vaddr, preinit_array_size) = if opts.is_pie {
        layout_section(
            ".preinit_array",
            section_name_to_idx,
            output_sections,
            &mut file_offset,
            &mut vaddr,
            4,
        )
    } else {
        (0, 0)
    };
    let (init_array_vaddr, init_array_size) = layout_section(
        ".init_array",
        section_name_to_idx,
        output_sections,
        &mut file_offset,
        &mut vaddr,
        4,
    );
    let (fini_array_vaddr, fini_array_size) = layout_section(
        ".fini_array",
        section_name_to_idx,
        output_sections,
        &mut file_offset,
        &mut vaddr,
        4,
    );
    let _ = layout_section(
        ".data.rel.ro",
        section_name_to_idx,
        output_sections,
        &mut file_offset,
        &mut vaddr,
        4,
    );
    file_offset = align_up(file_offset, 4);
    vaddr = align_up(vaddr, 4);
    let dynamic_offset = file_offset;
    let dynamic_vaddr = vaddr;
    file_offset += dynamic_size;
    vaddr += dynamic_size;
    let got_offset = file_offset;
    let got_vaddr = vaddr;
    file_offset += got_size;
    vaddr += got_size;
    let mut relro_end = vaddr;
    let gotplt_offset = file_offset;
    let gotplt_vaddr = vaddr;
    file_offset += gotplt_size;
    vaddr += gotplt_size;
    if opts.bind_now {
        relro_end = vaddr;
    }
    // `_GLOBAL_OFFSET_TABLE_` = start of .got.plt (psABI; PLT and GOTOFF
    // are relative to it).
    let got_base = gotplt_vaddr;

    layout_custom_sections(
        section_name_to_idx,
        output_sections,
        &mut file_offset,
        &mut vaddr,
        SHF_WRITE,
    );
    let _ = layout_section(
        ".data",
        section_name_to_idx,
        output_sections,
        &mut file_offset,
        &mut vaddr,
        16,
    );
    let _ = layout_section(
        ".tm_clone_table",
        section_name_to_idx,
        output_sections,
        &mut file_offset,
        &mut vaddr,
        4,
    );
    let data_seg_file_end = file_offset;
    if let Some(&idx) = section_name_to_idx.get(".bss") {
        let a = output_sections[idx].align.max(4);
        vaddr = align_up(vaddr, a);
        output_sections[idx].addr = vaddr;
        output_sections[idx].file_offset = file_offset;
        vaddr += output_sections[idx].data.len() as u32;
    }
    let data_seg_vaddr_end = vaddr;

    // ── Symbol addresses ──────────────────────────────────────────────────
    let new_sym = |address: u32, binding: u8| LinkerSymbol {
        address,
        size: 0,
        sym_type: STT_NOTYPE,
        binding,
        visibility: STV_HIDDEN,
        is_defined: true,
        needs_plt: false,
        needs_got: false,
        output_section: usize::MAX,
        section_offset: 0,
        plt_index: 0,
        got_index: 0,
        is_dynamic: false,
        dynlib: String::new(),
        needs_copy: false,
        copy_addr: 0,
        version: None,
        lib_value: 0,
        lib_in_exec: false,
        canonical_plt: false,
    };
    let gs = global_symbols
        .entry("_GLOBAL_OFFSET_TABLE_".to_string())
        .or_insert_with(|| new_sym(got_base, STB_LOCAL));
    gs.address = got_base;
    gs.is_defined = true;
    for (i, name) in plt_names.iter().enumerate() {
        if let Some(gs) = global_symbols.get_mut(name) {
            gs.plt_index = i;
        }
    }
    let bss_addr = section_name_to_idx
        .get(".bss")
        .map_or(data_seg_vaddr_end, |&i| output_sections[i].addr);
    for (name, sym) in global_symbols.iter_mut() {
        if sym.output_section < output_sections.len() {
            sym.address = output_sections[sym.output_section].addr + sym.section_offset;
        }
        // An input definition wins over every linker-provided name (GNU
        // ld's PROVIDE semantics); only references are synthesised.
        if sym.is_defined || !is_emitter_defined(name, sections) {
            continue;
        }
        if let Some(sec) = name.strip_prefix("__start_") {
            if let Some(&i) = section_name_to_idx.get(sec) {
                *sym = new_sym(output_sections[i].addr, STB_GLOBAL);
            }
        } else if let Some(sec) = name.strip_prefix("__stop_") {
            if let Some(&i) = section_name_to_idx.get(sec) {
                let end = output_sections[i].addr + output_sections[i].data.len() as u32;
                *sym = new_sym(end, STB_GLOBAL);
            }
        }
    }
    super::link::tls_module_base_address(global_symbols, tls_addr, tls_mem_size, false);
    let linker_addrs = LinkerSymbolAddresses {
        base_addr: base_addr as u64,
        got_addr: got_base as u64,
        dynamic_addr: dynamic_vaddr as u64,
        bss_addr: bss_addr as u64,
        bss_size: (data_seg_vaddr_end - bss_addr) as u64,
        text_end: text_seg_vaddr_end as u64,
        data_start: data_seg_vaddr_start as u64,
        init_array_start: init_array_vaddr as u64,
        init_array_size: init_array_size as u64,
        fini_array_start: fini_array_vaddr as u64,
        fini_array_size: fini_array_size as u64,
        preinit_array_start: preinit_array_vaddr as u64,
        preinit_array_size: preinit_array_size as u64,
        rela_iplt_start: 0,
        rela_iplt_size: 0,
    };
    for sym in &get_standard_linker_symbols(&linker_addrs) {
        if sym.name.starts_with("__rela_iplt") {
            continue;
        }
        let binding = if sym.name == "_GLOBAL_OFFSET_TABLE_" {
            STB_LOCAL
        } else {
            STB_GLOBAL
        };
        let e = global_symbols
            .entry(sym.name.to_string())
            .or_insert_with(|| new_sym(sym.value as u32, binding));
        if !e.is_defined {
            *e = new_sym(sym.value as u32, binding);
        }
    }
    let place = |g: &LinkerSymbol| {
        if g.output_section < output_sections.len() {
            output_sections[g.output_section].addr + g.section_offset
        } else {
            g.address
        }
    };
    super::link::evaluate_pending_defsyms(global_symbols, pending_defsyms, &place)?;
    let global_symbols: &FxHashMap<String, LinkerSymbol> = global_symbols;
    let plt_addr = |name: &str| -> u32 {
        let i = global_symbols.get(name).map_or(0, |g| g.plt_index) as u32;
        plt_vaddr + plt_header_size + i * plt_entry_size
    };
    let dynsym_of = |t: &Target| -> u32 {
        match &t.key {
            SymKey::Global(n) if !t.local => dynsym_index.get(n).copied().unwrap_or(0),
            _ => 0,
        }
    };

    // ── Pass 2: apply relocations ─────────────────────────────────────────
    let mut relative: Vec<u32> = Vec::with_capacity(num_relative);
    let mut symbolic: Vec<DynRel> = Vec::with_capacity(num_symbolic);
    let mut slot_value: FxHashMap<(SymKey, GotKind), u32> = FxHashMap::default();
    for (obj_idx, obj) in inputs.iter().enumerate() {
        for sec in &obj.sections {
            let Some(&(out_idx, sec_base)) = section_map.get(&(obj_idx, sec.input_index)) else {
                continue;
            };
            if output_sections[out_idx].flags & SHF_ALLOC == 0 {
                continue;
            }
            for &(rel_offset, rel_type, sym_idx, addend) in &sec.relocations {
                let Some(sym) = obj.symbols.get(sym_idx as usize) else {
                    continue;
                };
                let t = classify(
                    obj_idx,
                    sym_idx as usize,
                    sym,
                    global_symbols,
                    section_map,
                    sections,
                    opts,
                );
                let s = symbol_value(obj_idx, sym, global_symbols, section_map, output_sections);
                if matches!(t.key, SymKey::Local(..)) || t.local {
                    slot_value
                        .entry((t.key.clone(), GotKind::Addr))
                        .or_insert(s);
                }
                let off = (sec_base + rel_offset) as usize;
                let p = output_sections[out_idx].addr + sec_base + rel_offset;
                let a = addend;
                let tls_off = s.wrapping_sub(tls_addr);
                let slot_addr =
                    |kind: GotKind| -> u32 { got_vaddr + got_word[&(t.key.clone(), kind)] * 4 };
                let data = &mut output_sections[out_idx].data;
                let value: Option<u32> = match plan(rel_type, &t, &sec.data, rel_offset as usize) {
                    Plan::Nothing => None,
                    Plan::Error(e) => return Err(format!("{}: {}", obj.filename, e)),
                    Plan::Abs32 { relative: rel } => {
                        if rel {
                            relative.push(p);
                        }
                        Some(s.wrapping_add(a as u32))
                    }
                    Plan::DynSite(r_type) => {
                        symbolic.push((p, r_type, dynsym_of(&t)));
                        let inplace = if r_type == R_386_TLS_TPOFF && t.local {
                            tls_off.wrapping_add(a as u32)
                        } else {
                            a as u32
                        };
                        Some(inplace)
                    }
                    Plan::Pc32 => Some(s.wrapping_add(a as u32).wrapping_sub(p)),
                    Plan::Plt => Some(plt_addr(&t.name).wrapping_add(a as u32).wrapping_sub(p)),
                    Plan::GotSlot(kind) => Some(
                        slot_addr(kind)
                            .wrapping_add(a as u32)
                            .wrapping_sub(got_base),
                    ),
                    Plan::GotSlotAbs(kind) => {
                        relative.push(p);
                        Some(slot_addr(kind).wrapping_add(a as u32))
                    }
                    Plan::GotRelaxLea => {
                        data[off - 2] = 0x8d;
                        Some(s.wrapping_add(a as u32).wrapping_sub(got_base))
                    }
                    Plan::GotOff => Some(s.wrapping_add(a as u32).wrapping_sub(got_base)),
                    Plan::GotPc => Some(got_base.wrapping_add(a as u32).wrapping_sub(p)),
                    Plan::TlsLdm => Some(
                        (got_vaddr + ldm_word * 4)
                            .wrapping_add(a as u32)
                            .wrapping_sub(got_base),
                    ),
                    Plan::TlsLdo => Some(tls_off.wrapping_add(a as u32)),
                    Plan::Size32 => {
                        let size = match &t.key {
                            SymKey::Global(n) => global_symbols.get(n).map_or(sym.size, |g| g.size),
                            SymKey::Local(..) => sym.size,
                        };
                        Some(size.wrapping_add(a as u32))
                    }
                    Plan::Narrow(width, pcrel) => {
                        let raw = if pcrel {
                            s.wrapping_add(a as u32).wrapping_sub(p)
                        } else {
                            s.wrapping_add(a as u32)
                        };
                        let v = i64::from(raw as i32);
                        let (lo, hi) = if width == 16 {
                            (-0x8000i64, 0xffffi64)
                        } else {
                            (-0x80i64, 0xffi64)
                        };
                        if v < lo || v > hi {
                            return Err(format!(
                                "{}: relocation truncated to fit: {} against `{}'",
                                obj.filename,
                                reloc_name(rel_type),
                                t.name
                            ));
                        }
                        let bytes = (v as u32).to_le_bytes();
                        let n = (width / 8) as usize;
                        match data.get_mut(off..off + n) {
                            Some(f) => f.copy_from_slice(&bytes[..n]),
                            None => {
                                return Err(format!(
                                    "{}: relocation outside section",
                                    obj.filename
                                ));
                            }
                        }
                        None
                    }
                };
                if let Some(v) = value {
                    write32(data, off, v).map_err(|e| format!("{}: {}", obj.filename, e))?;
                }
            }
        }
    }

    // ── GOT contents ─────────────────────────────────────────────────────
    let mut got_data = vec![0u8; got_size as usize];
    got_data[0..4].copy_from_slice(&dynamic_vaddr.to_le_bytes());
    for k in &got_slots {
        let t = &slot_target[k];
        let w = got_word[k];
        let slot = got_vaddr + w * 4;
        let s = match &k.0 {
            SymKey::Global(n) if t.defined => global_symbols.get(n).map_or(0, |g| g.address),
            _ => slot_value
                .get(&(k.0.clone(), GotKind::Addr))
                .copied()
                .unwrap_or(0),
        };
        let idx = dynsym_of(t);
        let put = |got: &mut Vec<u8>, word: u32, v: u32| {
            got[(word * 4) as usize..(word * 4 + 4) as usize].copy_from_slice(&v.to_le_bytes());
        };
        match k.1 {
            GotKind::Addr => {
                if t.local {
                    put(&mut got_data, w, s);
                    if t.defined && !t.abs {
                        relative.push(slot);
                    }
                } else {
                    symbolic.push((slot, R_386_GLOB_DAT, idx));
                }
            }
            GotKind::TlsGd => {
                symbolic.push((slot, R_386_TLS_DTPMOD32, idx));
                if t.local {
                    put(&mut got_data, w + 1, s.wrapping_sub(tls_addr));
                } else {
                    symbolic.push((slot + 4, R_386_TLS_DTPOFF32, idx));
                }
            }
            GotKind::TlsIe => {
                if t.local {
                    put(&mut got_data, w, s.wrapping_sub(tls_addr));
                }
                symbolic.push((slot, R_386_TLS_TPOFF, idx));
            }
            GotKind::TlsDesc => {
                if t.local {
                    put(&mut got_data, w + 1, s.wrapping_sub(tls_addr));
                }
                symbolic.push((slot, R_386_TLS_DESC, idx));
            }
        }
    }
    if needs_ldm {
        symbolic.push((got_vaddr + ldm_word * 4, R_386_TLS_DTPMOD32, 0));
    }
    if relative.len() != num_relative || symbolic.len() != num_symbolic {
        return Err(format!(
            "internal error: dynamic relocation count changed between sizing ({num_relative}+{num_symbolic}) and emission ({}+{})",
            relative.len(),
            symbolic.len()
        ));
    }
    // RELATIVE first (DT_RELCOUNT), each group in address order.
    relative.sort_unstable();
    symbolic.sort_unstable();
    let (relr_aligned, rel_leftovers) = if opts.pack_relative_relocs {
        linker_common::relr::partition_relative_32(&relative)
    } else {
        (Vec::new(), relative)
    };
    let relr_dyn_data = linker_common::relr::encode_relr_32_bytes(&relr_aligned);
    let relr_actual_size = relr_dyn_data.len() as u32;
    if rel_leftovers.len() != num_rel_relative || relr_actual_size > relr_reserved_size {
        return Err(format!(
            "internal error: .relr.dyn sizing mismatch (leftovers {} vs {}, relr {} vs {})",
            rel_leftovers.len(),
            num_rel_relative,
            relr_actual_size,
            relr_reserved_size
        ));
    }
    let mut rel_dyn_data: Vec<u8> = Vec::with_capacity(rel_dyn_size as usize);
    for &addr in &rel_leftovers {
        rel_dyn_data.extend_from_slice(&addr.to_le_bytes());
        rel_dyn_data.extend_from_slice(&R_386_RELATIVE.to_le_bytes());
    }
    for &(addr, ty, idx) in &symbolic {
        rel_dyn_data.extend_from_slice(&addr.to_le_bytes());
        rel_dyn_data.extend_from_slice(&((idx << 8) | ty).to_le_bytes());
    }

    // ── PLT, .got.plt, .rel.plt ──────────────────────────────────────────
    let plt_data = build_plt(
        num_plt,
        plt_vaddr,
        plt_header_size,
        plt_entry_size,
        gotplt_vaddr,
        gotplt_reserved,
        PltAddressing::EbxRelative {
            got_symbol: got_base,
        },
    );
    let mut gotplt_data: Vec<u8> = Vec::with_capacity(gotplt_size as usize);
    gotplt_data.extend_from_slice(&dynamic_vaddr.to_le_bytes());
    gotplt_data.extend_from_slice(&0u32.to_le_bytes());
    gotplt_data.extend_from_slice(&0u32.to_le_bytes());
    // Lazy slots point back at their PLT entry's `push`; the loader adds
    // the load bias to them (elf_machine_lazy_rel).
    for i in 0..num_plt as u32 {
        gotplt_data.extend_from_slice(
            &(plt_vaddr + plt_header_size + i * plt_entry_size + 6).to_le_bytes(),
        );
    }
    let mut rel_plt_data: Vec<u8> = Vec::with_capacity(rel_plt_size as usize);
    for (i, name) in plt_names.iter().enumerate() {
        let slot = gotplt_vaddr + (gotplt_reserved + i as u32) * 4;
        rel_plt_data.extend_from_slice(&slot.to_le_bytes());
        rel_plt_data.extend_from_slice(&((dynsym_index[name] << 8) | R_386_JMP_SLOT).to_le_bytes());
    }

    // ── .dynamic ─────────────────────────────────────────────────────────
    for (slot, name) in [(&mut dyn_desc.init, "_init"), (&mut dyn_desc.fini, "_fini")] {
        if let Some(v) = slot.as_mut() {
            *v = image_function_addr(global_symbols, output_sections, name).unwrap_or(*v);
        }
    }
    if let Some(v) = dyn_desc.preinit_array.as_mut() {
        *v = (preinit_array_vaddr, preinit_array_size);
    }
    if let Some(v) = dyn_desc.init_array.as_mut() {
        *v = (init_array_vaddr, init_array_size);
    }
    if let Some(v) = dyn_desc.fini_array.as_mut() {
        *v = (fini_array_vaddr, fini_array_size);
    }
    if let Some(v) = dyn_desc.plt.as_mut() {
        v.0 = gotplt_vaddr;
    }
    if let Some(v) = dyn_desc.relr.as_mut() {
        *v = (relr_dyn_vaddr, relr_actual_size);
    }
    let dynamic_data = dyn_desc.serialize(dynamic_size)?;

    // ── Section headers ──────────────────────────────────────────────────
    // A header for every allocated section, in address order.  The loader
    // reads only program headers, but a link editor reading this library
    // uses them twice: to find .dynsym/.dynstr/.dynamic at all (bfd refuses
    // a library with e_shoff == 0), and to learn from each exported
    // symbol's st_shndx what kind of storage it lives in.  The latter is
    // not cosmetic: when an executable copy-relocates a variable, GNU ld
    // puts the copy in `.data.rel.ro` -- write-protected after relocation
    // -- if the defining section is read-only or inside PT_GNU_RELRO.  With
    // every definition claiming index 1 (then `.dynamic`, which is RELRO)
    // the first store to such a variable faulted.
    let mut shdrs: Vec<Shdr32> = Shdr32::standard_all(&[
        (
            StdSection::Interp,
            true,
            interp_vaddr,
            interp_offset,
            interp_size,
        ),
        (
            StdSection::BuildId,
            true,
            note_vaddr + (build_id_offset - note_offset),
            build_id_offset,
            build_id_size,
        ),
        (
            StdSection::Hash,
            sysv_hash.is_some(),
            hash_vaddr,
            hash_offset,
            hash_size,
        ),
        (
            StdSection::GnuHash,
            want_gnu_hash,
            gnu_hash_vaddr,
            gnu_hash_offset,
            gnu_hash_size,
        ),
        (
            StdSection::Dynsym,
            true,
            dynsym_vaddr,
            dynsym_offset,
            dynsym_size,
        ),
        (
            StdSection::Dynstr,
            true,
            dynstr_vaddr,
            dynstr_offset,
            dynstr_size,
        ),
        (
            StdSection::Versym,
            true,
            versym_vaddr,
            versym_offset,
            versym_size,
        ),
        (
            StdSection::Verneed,
            true,
            verneed_vaddr,
            verneed_offset,
            verneed_size,
        ),
        (
            StdSection::RelDyn,
            true,
            rel_dyn_vaddr,
            rel_dyn_offset,
            rel_dyn_size,
        ),
        (
            StdSection::Relr,
            true,
            relr_dyn_vaddr,
            relr_dyn_offset,
            relr_actual_size,
        ),
        (
            StdSection::RelPlt,
            true,
            rel_plt_vaddr,
            rel_plt_offset,
            rel_plt_size,
        ),
        (StdSection::Plt, true, plt_vaddr, plt_offset, plt_total_size),
        (
            StdSection::EhFrameHdr,
            true,
            eh_frame_hdr_vaddr,
            eh_frame_hdr_offset,
            eh_frame_hdr_size,
        ),
        (
            StdSection::Dynamic,
            true,
            dynamic_vaddr,
            dynamic_offset,
            dynamic_size,
        ),
        (StdSection::Got, true, got_vaddr, got_offset, got_size),
        (
            StdSection::GotPlt,
            true,
            gotplt_vaddr,
            gotplt_offset,
            gotplt_size,
        ),
    ])
    .collect();
    // Debug sections follow the load image; nothing at run time maps them.
    super::debug::apply_debug_relocations(
        inputs,
        section_map,
        output_sections,
        global_symbols,
        tls_addr,
        got_base,
        !opts.is_pie,
    )?;
    let file_end = super::debug::place_debug_sections(output_sections, data_seg_file_end);
    shdrs.extend(Shdr32::of_outputs(output_sections));
    shdr::finish(&mut shdrs, verneed_count);
    let symbols = (!opts.strip_all).then(|| {
        super::symtab::build(
            inputs,
            section_map,
            output_sections,
            global_symbols,
            section_name_to_idx,
            &shdrs,
            tls_addr,
        )
    });
    let file_end = symbols
        .as_ref()
        .map_or(file_end, |t| t.place(&mut shdrs, file_end));
    let dynsym_hdr = shdr::synth_index(&shdrs, ".dynsym") as u16;
    let out_sec_hdr = shdr::output_index_map(&shdrs);
    let containing_hdr =
        |name: &str, addr: u32| shdr::containing_index(&shdrs, name, addr, dynsym_hdr);

    // ── .dynsym ──────────────────────────────────────────────────────────
    let mut dynsym_data: Vec<u8> = vec![0u8; 16];
    for name in &dynsym_names {
        let (value, size, info, other, shndx) = match global_symbols.get(name) {
            Some(gs) if gs.is_defined && !import_set.contains(name) => {
                let hdr = || {
                    out_sec_hdr
                        .get(&gs.output_section)
                        .copied()
                        .unwrap_or_else(|| containing_hdr(name, gs.address))
                };
                let (value, shndx) = if gs.sym_type == STT_TLS {
                    (gs.address.wrapping_sub(tls_addr), hdr())
                } else if gs.output_section == usize::MAX && !is_emitter_defined(name, sections) {
                    // Absolute (ABS input, --defsym constant): the loader
                    // must not add the load bias (glibc checks SHN_ABS).
                    (gs.address, SHN_ABS)
                } else {
                    (gs.address, hdr())
                };
                (
                    value,
                    gs.size,
                    (gs.binding << 4) | gs.sym_type,
                    gs.visibility,
                    shndx,
                )
            }
            Some(gs) => {
                let ty = if gs.sym_type == STT_NOTYPE && plt_set.contains(name) {
                    STT_FUNC
                } else {
                    gs.sym_type
                };
                (0, 0, (gs.binding << 4) | ty, 0, SHN_UNDEF)
            }
            None => (0, 0, STB_GLOBAL << 4, 0, SHN_UNDEF),
        };
        dynsym_data.extend_from_slice(&dynstr.get_offset(name).to_le_bytes());
        dynsym_data.extend_from_slice(&value.to_le_bytes());
        dynsym_data.extend_from_slice(&size.to_le_bytes());
        dynsym_data.push(info);
        dynsym_data.push(other);
        dynsym_data.extend_from_slice(&shndx.to_le_bytes());
    }

    // ── .eh_frame_hdr ────────────────────────────────────────────────────
    let eh_frame_hdr_data = match (eh_frame_sec_idx, eh_frame_hdr_size) {
        (Some(idx), sz) if sz > 0 => {
            let sec = &output_sections[idx];
            let mut hdr = vec![0u8; sz as usize];
            linker_common::write_eh_frame_hdr(
                &mut hdr,
                &sec.data,
                sec.addr as u64,
                eh_frame_hdr_vaddr as u64,
                false,
            );
            hdr
        }
        _ => Vec::new(),
    };

    // ── Write ────────────────────────────────────────────────────────────
    let header_table = shdr::serialize(&shdrs, file_end)?;
    let mut output = vec![0u8; header_table.end];

    output[0..4].copy_from_slice(&ELF_MAGIC);
    output[4] = ELFCLASS32;
    output[5] = ELFDATA2LSB;
    output[6] = EV_CURRENT;
    output[16..18].copy_from_slice(&ET_DYN.to_le_bytes());
    output[18..20].copy_from_slice(&EM_386.to_le_bytes());
    output[20..24].copy_from_slice(&1u32.to_le_bytes());
    // e_entry: `-e SYMBOL` names a routine a program loader may run; GNU ld
    // records it for shared objects too (0 when absent).
    // GNU ld: `-e` as for executables; without it `_start` when defined,
    // else 0 (no warning for shared objects).
    let entry = match opts.entry.as_deref() {
        Some(e) => super::emit::resolve_entry_point(Some(e), global_symbols, text_seg_vaddr_start),
        None => global_symbols
            .get("_start")
            .filter(|g| g.is_defined && !g.is_dynamic)
            .map_or(0, |g| g.address),
    };
    output[24..28].copy_from_slice(&entry.to_le_bytes());
    output[28..32].copy_from_slice(&ehdr_size.to_le_bytes());
    output[40..42].copy_from_slice(&(ehdr_size as u16).to_le_bytes());
    output[42..44].copy_from_slice(&32u16.to_le_bytes());
    output[44..46].copy_from_slice(&(num_phdrs as u16).to_le_bytes());
    header_table.write(&mut output);
    if let Some(t) = &symbols {
        t.write(&shdrs, &mut output);
    }

    let mut phdrs: Vec<[u32; 8]> = Vec::with_capacity(num_phdrs as usize);
    // [type, offset, vaddr, filesz, memsz, flags, align, -]
    phdrs.push([
        PT_PHDR,
        phdr_offset,
        phdr_vaddr,
        phdrs_total_size,
        phdrs_total_size,
        PF_R,
        4,
        0,
    ]);
    if interp_size > 0 {
        phdrs.push([
            PT_INTERP,
            interp_offset,
            interp_vaddr,
            interp_size,
            interp_size,
            PF_R,
            1,
            0,
        ]);
    }
    phdrs.push([
        PT_LOAD,
        0,
        base_addr,
        ro_headers_end,
        ro_headers_end,
        PF_R,
        page_size,
        0,
    ]);
    phdrs.push([
        PT_LOAD,
        text_seg_file_start,
        text_seg_vaddr_start,
        text_seg_file_end - text_seg_file_start,
        text_seg_vaddr_end - text_seg_vaddr_start,
        PF_R | PF_X,
        page_size,
        0,
    ]);
    phdrs.push([
        PT_LOAD,
        rodata_seg_file_start,
        rodata_seg_vaddr_start,
        rodata_seg_file_end - rodata_seg_file_start,
        rodata_seg_vaddr_end - rodata_seg_vaddr_start,
        PF_R,
        page_size,
        0,
    ]);
    phdrs.push([
        PT_LOAD,
        data_seg_file_start,
        data_seg_vaddr_start,
        data_seg_file_end - data_seg_file_start,
        data_seg_vaddr_end - data_seg_vaddr_start,
        PF_R | PF_W,
        page_size,
        0,
    ]);
    phdrs.push([
        PT_DYNAMIC,
        dynamic_offset,
        dynamic_vaddr,
        dynamic_size,
        dynamic_size,
        PF_R | PF_W,
        4,
        0,
    ]);
    let exec_stack = opts
        .exec_stack
        .unwrap_or_else(|| inputs_want_exec_stack(inputs));
    let stack_flags = PF_R | PF_W | if exec_stack { PF_X } else { 0 };
    phdrs.push([PT_GNU_STACK, 0, 0, 0, 0, stack_flags, 16, 0]);
    if has_tls {
        phdrs.push([
            PT_TLS,
            tls_file_offset,
            tls_addr,
            tls_file_size,
            tls_mem_size,
            PF_R,
            tls_align,
            0,
        ]);
    }
    if has_notes {
        phdrs.push([
            PT_NOTE,
            note_offset,
            note_vaddr,
            note_total_size,
            note_total_size,
            PF_R,
            note_align,
            0,
        ]);
    }
    if let Some((off, size)) = property_note {
        phdrs.push([
            PT_GNU_PROPERTY,
            note_offset + off,
            note_vaddr + off,
            size,
            size,
            PF_R,
            4,
            0,
        ]);
    }
    if eh_frame_hdr_size > 0 {
        phdrs.push([
            PT_GNU_EH_FRAME,
            eh_frame_hdr_offset,
            eh_frame_hdr_vaddr,
            eh_frame_hdr_size,
            eh_frame_hdr_size,
            PF_R,
            4,
            0,
        ]);
    }
    if opts.relro {
        let sz = relro_end - relro_vaddr;
        phdrs.push([PT_GNU_RELRO, relro_offset, relro_vaddr, sz, sz, PF_R, 1, 0]);
    }
    debug_assert_eq!(phdrs.len() as u32, num_phdrs);
    for (i, ph) in phdrs.iter().enumerate() {
        let o = (phdr_offset + i as u32 * phdr_size) as usize;
        let fields = [ph[0], ph[1], ph[2], ph[2], ph[3], ph[4], ph[5], ph[6]];
        for (j, v) in fields.iter().enumerate() {
            output[o + j * 4..o + j * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
    }

    let mut put = |off: u32, bytes: &[u8]| {
        if !bytes.is_empty() {
            output[off as usize..off as usize + bytes.len()].copy_from_slice(bytes);
        }
    };
    if let Some(interp_bytes) = interp {
        put(interp_offset, interp_bytes);
    }
    if let Some(h) = &sysv_hash {
        let mut buf = vec![0u8; hash_size as usize];
        linker_common::write_sysv_hash(&mut buf, 0, h);
        put(hash_offset, &buf);
    }
    if want_gnu_hash {
        put(gnu_hash_offset, &gnu_hash_data);
    }
    put(dynsym_offset, &dynsym_data);
    put(dynstr_offset, &dynstr_data);
    put(versym_offset, &versym_data);
    put(verneed_offset, &verneed_data);
    put(rel_dyn_offset, &rel_dyn_data);
    put(relr_dyn_offset, &relr_dyn_data);
    put(rel_plt_offset, &rel_plt_data);
    put(plt_offset, &plt_data);
    put(eh_frame_hdr_offset, &eh_frame_hdr_data);
    put(dynamic_offset, &dynamic_data);
    put(got_offset, &got_data);
    put(gotplt_offset, &gotplt_data);
    check_sections_placed(output_sections, "shared library")?;
    for sec in output_sections.iter() {
        if sec.sh_type != SHT_NOBITS {
            put(sec.file_offset, &sec.data);
        }
    }

    if build_id_size > 0 {
        linker_common::build_id::write_build_id_skeleton_styled(
            &mut output,
            build_id_offset as usize,
            &build_id_style,
        );
        linker_common::build_id::patch_build_id_styled(
            &mut output,
            build_id_offset as usize,
            &build_id_style,
        );
    }

    super::emit::write_link_map_32(opts, output_sections, global_symbols)?;

    std::fs::write(output_path, &output)
        .map_err(|e| format!("failed to write {}: {}", output_path, e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(output_path, std::fs::Permissions::from_mode(0o755));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(local: bool, defined: bool, abs: bool) -> Target {
        Target {
            key: SymKey::Global("x".into()),
            name: "x".into(),
            local,
            defined,
            abs,
        }
    }

    #[test]
    fn absolute_references_follow_binding() {
        assert_eq!(
            plan(R_386_32, &target(true, true, false), &[], 0),
            Plan::Abs32 { relative: true }
        );
        assert_eq!(
            plan(R_386_32, &target(true, true, true), &[], 0),
            Plan::Abs32 { relative: false }
        );
        assert_eq!(
            plan(R_386_32, &target(false, true, false), &[], 0),
            Plan::DynSite(R_386_32)
        );
        assert_eq!(
            plan(R_386_32, &target(false, false, false), &[], 0),
            Plan::DynSite(R_386_32)
        );
    }

    #[test]
    fn calls_to_preemptible_functions_use_the_plt() {
        assert_eq!(
            plan(R_386_PLT32, &target(false, true, false), &[], 0),
            Plan::Plt
        );
        assert_eq!(
            plan(R_386_PLT32, &target(true, true, false), &[], 0),
            Plan::Pc32
        );
        assert_eq!(
            plan(R_386_PC32, &target(false, false, false), &[], 0),
            Plan::DynSite(R_386_PC32)
        );
    }

    #[test]
    fn got32x_relaxes_only_local_movs_with_a_base_register() {
        // movl x@GOT(%ebx), %eax = 8b 83 <disp32>
        let mov_ebx = [0x8b, 0x83, 0, 0, 0, 0];
        assert_eq!(
            plan(R_386_GOT32X, &target(true, true, false), &mov_ebx, 2),
            Plan::GotRelaxLea
        );
        assert_eq!(
            plan(R_386_GOT32X, &target(false, true, false), &mov_ebx, 2),
            Plan::GotSlot(GotKind::Addr)
        );
        assert_eq!(
            plan(R_386_GOT32X, &target(true, true, true), &mov_ebx, 2),
            Plan::GotSlot(GotKind::Addr),
            "an absolute symbol keeps its slot (the value must not get GOT-relative)"
        );
        // call *x@GOT(%ebx) = ff 93 <disp32>: not a mov, keeps the slot.
        let call = [0xff, 0x93, 0, 0, 0, 0];
        assert_eq!(
            plan(R_386_GOT32X, &target(true, true, false), &call, 2),
            Plan::GotSlot(GotKind::Addr)
        );
        // movl x@GOT, %eax = 8b 05 <disp32>: base-less.
        let abs = [0x8b, 0x05, 0, 0, 0, 0];
        assert_eq!(
            plan(R_386_GOT32X, &target(true, true, false), &abs, 2),
            Plan::GotSlotAbs(GotKind::Addr)
        );
        assert!(matches!(
            plan(R_386_GOT32, &target(true, true, false), &abs, 2),
            Plan::Error(_)
        ));
    }

    #[test]
    fn tls_models_map_to_the_dso_forms() {
        let t = target(false, false, false);
        assert_eq!(
            plan(R_386_TLS_GD, &t, &[], 0),
            Plan::GotSlot(GotKind::TlsGd)
        );
        assert_eq!(
            plan(R_386_TLS_GOTIE, &t, &[], 0),
            Plan::GotSlot(GotKind::TlsIe)
        );
        assert_eq!(
            plan(R_386_TLS_IE, &t, &[], 0),
            Plan::GotSlotAbs(GotKind::TlsIe)
        );
        assert_eq!(
            plan(R_386_TLS_LE, &t, &[], 0),
            Plan::DynSite(R_386_TLS_TPOFF)
        );
        assert_eq!(
            plan(R_386_TLS_GOTDESC, &t, &[], 0),
            Plan::GotSlot(GotKind::TlsDesc)
        );
        assert_eq!(plan(R_386_TLS_DESC_CALL, &t, &[], 0), Plan::Nothing);
        assert!(matches!(plan(R_386_TLS_LDO_32, &t, &[], 0), Plan::Error(_)));
        assert_eq!(
            plan(R_386_TLS_LDO_32, &target(true, true, false), &[], 0),
            Plan::TlsLdo
        );
        assert!(is_static_tls(R_386_TLS_GOTIE) && !is_static_tls(R_386_TLS_GD));
    }

    #[test]
    fn narrow_absolute_fields_cannot_be_relocated_at_load_time() {
        assert!(matches!(
            plan(R_386_16, &target(true, true, false), &[], 0),
            Plan::Error(_)
        ));
        assert_eq!(
            plan(R_386_16, &target(true, true, true), &[], 0),
            Plan::Narrow(16, false)
        );
        assert_eq!(
            plan(R_386_PC8, &target(true, true, false), &[], 0),
            Plan::Narrow(8, true)
        );
        assert!(matches!(
            plan(R_386_PC8, &target(false, true, false), &[], 0),
            Plan::Error(_)
        ));
    }

    #[test]
    fn binding_honours_visibility_and_symbolic() {
        let mut gs = LinkerSymbol {
            address: 0,
            size: 0,
            sym_type: STT_OBJECT,
            binding: STB_GLOBAL,
            visibility: STV_DEFAULT,
            is_defined: true,
            needs_plt: false,
            needs_got: false,
            output_section: 0,
            section_offset: 0,
            plt_index: 0,
            got_index: 0,
            is_dynamic: false,
            dynlib: String::new(),
            needs_copy: false,
            copy_addr: 0,
            version: None,
            lib_value: 0,
            lib_in_exec: false,
            canonical_plt: false,
        };
        assert!(!binds_locally(&gs, Symbolic::None));
        assert!(!binds_locally(&gs, Symbolic::Functions));
        assert!(binds_locally(&gs, Symbolic::All));
        gs.sym_type = STT_FUNC;
        assert!(binds_locally(&gs, Symbolic::Functions));
        gs.sym_type = STT_OBJECT;
        gs.visibility = STV_HIDDEN;
        assert!(binds_locally(&gs, Symbolic::None));
        gs.visibility = STV_PROTECTED;
        assert!(binds_locally(&gs, Symbolic::None));
    }
}
