//! AArch64 relocation application.
//!
//! Applies ELF relocations to the output buffer after section layout
//! has been determined.

use super::elf::*;
use super::types::GlobalSymbol;
use crate::backend::linker_common::OutputSection;
use crate::common::fx_hash::FxHashMap;

// TLS relocation types (AArch64 Local Exec model for static linking)
const R_AARCH64_TLSLE_ADD_TPREL_HI12: u32 = 549;
const R_AARCH64_TLSLE_ADD_TPREL_LO12: u32 = 550;
const R_AARCH64_TLSLE_ADD_TPREL_LO12_NC: u32 = 551;
// Numbers from <elf.h>, which assigns this family in order of DESCENDING
// significance (G2 first). They were defined here in ascending name order, which
// rotates all five: a real R_AARCH64_TLSLE_MOVW_TPREL_G0 (547) matched the arm
// for G1_NC and was encoded with `tp >> 16` instead of `tp & 0xffff`, and a real
// G0_NC (548) matched the G2 arm and got `tp >> 32`. Four of the five types were
// therefore mis-encoded -- only G1 was unaffected, and only because the arm it
// landed in happens to use the same shift. The `_NC` suffix made it silent: the
// ABI exempts those forms from range checking, so nothing complained about a
// wrong TLS offset.
const R_AARCH64_TLSLE_MOVW_TPREL_G2: u32 = 544;
const R_AARCH64_TLSLE_MOVW_TPREL_G1: u32 = 545;
const R_AARCH64_TLSLE_MOVW_TPREL_G1_NC: u32 = 546;
const R_AARCH64_TLSLE_MOVW_TPREL_G0: u32 = 547;
const R_AARCH64_TLSLE_MOVW_TPREL_G0_NC: u32 = 548;

// TLS descriptor: relaxed in executables
pub const R_AARCH64_TLSDESC_ADR_PAGE21: u32 = 562;
pub const R_AARCH64_TLSDESC_LD64_LO12: u32 = 563;
pub const R_AARCH64_TLSDESC_ADD_LO12: u32 = 564;
pub const R_AARCH64_TLSDESC_CALL: u32 = 569;

// GD (General Dynamic): relaxed in executables
pub const R_AARCH64_TLSGD_ADR_PAGE21: u32 = 513;
pub const R_AARCH64_TLSGD_ADD_LO12_NC: u32 = 514;

// The instructions TLS relaxation writes.
const NOP: u32 = 0xd503_201f;
const MOVZ_X_LSL16: u32 = 0xd2a0_0000;
const MOVK_X: u32 = 0xf280_0000;
const LDR_X_UIMM: u32 = 0xf940_0000;
const MRS_X1_TPIDR_EL0: u32 = 0xd53b_d041;
const ADD_X0_X1_X0: u32 = 0x8b00_0020;

/// The `bl __tls_get_addr` of a general-dynamic sequence: the CALL26 right
/// after its TLSGD_ADD_LO12_NC, which that relocation rewrote.
pub fn is_relaxed_tls_call(relas: &[Rela], i: usize, sym_name: &str) -> bool {
    i > 0
        && sym_name == "__tls_get_addr"
        && relas[i].rela_type == R_AARCH64_CALL26
        && relas[i - 1].rela_type == R_AARCH64_TLSGD_ADD_LO12_NC
        && relas[i - 1].offset + 4 == relas[i].offset
}

/// An instruction-rewrite site must lie inside the output.
fn check_insn_site(out: &[u8], fp: usize, len: usize) -> Result<(), String> {
    if fp + len > out.len() {
        return Err(format!("AArch64 TLS rewrite site {fp:#x} exceeds output"));
    }
    Ok(())
}

// IE (Initial Exec) TLS
pub const R_AARCH64_TLSIE_ADR_GOTTPREL_PAGE21: u32 = 541;
pub const R_AARCH64_TLSIE_LD64_GOTTPREL_LO12_NC: u32 = 542;

/// TLS layout info needed for relocation processing
pub struct TlsInfo {
    pub tls_addr: u64,
    pub tls_size: u64,
}

/// GOT (Global Offset Table) for static linking.
/// Maps symbol names to GOT entry indices for R_AARCH64_ADR_GOT_PAGE
/// and R_AARCH64_LD64_GOT_LO12_NC relocations.
pub struct GotInfo {
    pub got_addr: u64,
    /// (symbol_key, resolved_address) -- symbol_key is "name" or "sec:obj:sec"
    pub entries: FxHashMap<String, usize>,
}

impl GotInfo {
    /// Get the address of a GOT entry for a given symbol key.
    pub fn entry_addr(&self, key: &str) -> Option<u64> {
        self.entries
            .get(key)
            .map(|&idx| self.got_addr + (idx as u64) * 8)
    }
}

/// Resolve a symbol's final address given the global symbol table and section map.
///
/// Linker-defined symbols (__bss_start, _edata, _end, __end, etc.) are resolved
/// through the globals table where they are registered by get_standard_linker_symbols().
pub fn resolve_sym(
    obj_idx: usize,
    sym: &Symbol,
    globals: &FxHashMap<String, GlobalSymbol>,
    section_map: &FxHashMap<(usize, usize), (usize, u64)>,
    output_sections: &[OutputSection],
) -> u64 {
    if sym.sym_type() == STT_SECTION {
        let si = sym.shndx as usize;
        return section_map
            .get(&(obj_idx, si))
            .map(|&(oi, so)| output_sections[oi].addr + so)
            .unwrap_or(0);
    }
    if !sym.name.is_empty() && !sym.is_local() {
        // All linker-defined symbols (including __bss_start, _edata, _end, __end)
        // are registered in the globals table with defined_in = Some(usize::MAX),
        // so they are resolved through the standard globals lookup below.
        // Local (STB_LOCAL) symbols must NOT be resolved via globals, since a
        // local symbol named e.g. "write" must not be confused with libc's write().
        if let Some(g) = globals.get(sym.name.as_str()) {
            if g.defined_in.is_some() {
                return g.value;
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

/// Whether `sym` names something an `ADR`/`ADRP` can encode.
///
/// Those forms write `S + A - P` into the instruction, so `S` must be an
/// address the loader maps. A symbol defined in a section that survives only
/// as file content -- `.debug_*` or any other input section without
/// `SHF_ALLOC` -- has no runtime address: such sections are laid out after the
/// image near file offset 0, so the encoded displacement would land on the
/// load bias instead of the symbol. A discarded input section is equally
/// addressless. Checking the section is what makes this sound: `S == 0` alone
/// proves nothing, because zero is also a legitimate mapped address, and
/// UNDEF/ABS symbols resolve to zero without owning a section at all.
///
/// Symbols that do not resolve through an input section keep their existing
/// treatment and count as addressable here: undefined and weak references,
/// `SHN_ABS` values (an address by definition), symbols imported from a shared
/// library, and linker-defined symbols (which live in allocated output
/// sections). The callers own those rules.
pub(super) fn adr_target_is_addressable(
    obj_idx: usize,
    sym: &Symbol,
    globals: &FxHashMap<String, GlobalSymbol>,
    section_map: &FxHashMap<(usize, usize), (usize, u64)>,
    output_sections: &[OutputSection],
) -> bool {
    let via_input_section = sym.is_local() || sym.name.is_empty();
    let (def_obj, shndx) = if via_input_section {
        (obj_idx, sym.shndx)
    } else {
        match globals.get(sym.name.as_str()) {
            Some(g) => match g.defined_in {
                // Linker-defined symbols are allocated by construction.
                Some(usize::MAX) => return true,
                // Defined in this link: that object owns the section, and this
                // object only carries the undefined reference.
                Some(def_obj) => (def_obj, g.section_idx),
                None => return true,
            },
            None => return true,
        }
    };
    match shndx {
        // A global reference that stayed undefined is resolved by the callers
        // (weak -> 0, DSO import -> PLT/GOT, preemptible -> refused). A local
        // one has no definition anywhere, so there is nothing to address.
        SHN_UNDEF => return !via_input_section,
        // Both name a value rather than a location, and the merged global
        // holds the real address.
        SHN_ABS | SHN_COMMON => return true,
        // Any other reserved index is not an input section at all, so there
        // is no mapping to prove -- `resolve_sym` falls back to the raw
        // `st_value`, which is not an address.
        _ if shndx >= SHN_LORESERVE => return false,
        _ => {}
    }
    section_map
        .get(&(def_obj, shndx as usize))
        .is_some_and(|&(out_idx, _)| {
            output_sections
                .get(out_idx)
                .is_some_and(|section| section.flags & SHF_ALLOC != 0)
        })
}

/// The refusal every AArch64 writer raises for an `ADR`/`ADRP` whose target
/// has no load address. One spelling, so the diagnostic cannot drift between
/// the static, dynamic and shared writers.
pub(super) fn adr_section_error(sym: &str) -> String {
    format!(
        "ADR/ADRP relocation requires a symbol in a surviving section: '{sym}' \
         (undefined, common, or its input section was discarded or is not loaded: a \
         non-allocated section is written to the file but never mapped, so the encoded \
         displacement would address a file offset or the load bias instead of the symbol)"
    )
}

/// A shared ADR can cancel load bias only against a local allocated section.
/// Do not infer this property from an address: zero is a valid mapped address,
/// whereas UNDEF/ABS and discarded sections can also resolve numerically to zero.
/// This deliberately does not change resolution of unrelated relocation kinds.
pub(super) fn resolve_shared_adr_target(
    obj_idx: usize,
    sym: &Symbol,
    section_map: &FxHashMap<(usize, usize), (usize, u64)>,
    output_sections: &[OutputSection],
) -> Option<u64> {
    // 0xff00 begins the ELF reserved section-index range, not an input section.
    if !sym.is_local() || sym.shndx == SHN_UNDEF || sym.shndx >= 0xff00 {
        return None;
    }
    let &(out_idx, offset) = section_map.get(&(obj_idx, sym.shndx as usize))?;
    let section = output_sections.get(out_idx)?;
    if section.flags & SHF_ALLOC == 0 || section.flags & SHF_TLS != 0 {
        return None;
    }
    section.addr.checked_add(offset)?.checked_add(sym.value)
}

/// Build a GOT key for a symbol reference in a relocation.
/// Local symbols must be scoped to their object to avoid collisions
/// (e.g., `.LANCHOR3` in different objects referring to different TLS vars).
pub fn got_key(obj_idx: usize, sym: &Symbol) -> String {
    if !sym.name.is_empty() && !sym.is_local() {
        sym.name.to_string()
    } else if !sym.name.is_empty() {
        format!("{}@{}", sym.name, obj_idx)
    } else if sym.sym_type() == STT_SECTION {
        format!("__sec_{}_{}", obj_idx, sym.shndx)
    } else {
        format!("__anon_{}_{}", obj_idx, sym.shndx)
    }
}

/// Scan all relocations to find symbols that need GOT entries.
/// This includes both regular GOT references and TLS IE references
/// (which use GOT entries containing the TP offset).
pub fn collect_got_symbols(objects: &[ElfObject]) -> Vec<(String, GotEntryKind)> {
    let mut got_syms: Vec<(String, GotEntryKind)> = Vec::new();
    for obj_idx in 0..objects.len() {
        for sec_idx in 0..objects[obj_idx].sections.len() {
            for rela in &objects[obj_idx].relocations[sec_idx] {
                let kind = match rela.rela_type {
                    R_AARCH64_ADR_GOT_PAGE | R_AARCH64_LD64_GOT_LO12_NC => GotEntryKind::Regular,
                    R_AARCH64_TLSIE_ADR_GOTTPREL_PAGE21 | R_AARCH64_TLSIE_LD64_GOTTPREL_LO12_NC => {
                        GotEntryKind::TlsIE
                    }
                    _ => continue,
                };
                let si = rela.sym_idx as usize;
                if si < objects[obj_idx].symbols.len() {
                    let key = got_key(obj_idx, &objects[obj_idx].symbols[si]);
                    if !got_syms.iter().any(|(k, _)| k == &key) {
                        got_syms.push((key, kind));
                    }
                }
            }
        }
    }
    got_syms
}

/// What kind of value a GOT entry holds.
#[derive(Clone, Copy, PartialEq)]
pub enum GotEntryKind {
    /// Regular GOT entry: holds the absolute address of the symbol.
    Regular,
    /// TLS IE GOT entry: holds the TP-relative offset of the TLS variable.
    TlsIE,
}

/// Apply all relocations to the output buffer.
pub fn apply_relocations(
    objects: &[ElfObject],
    globals: &FxHashMap<String, GlobalSymbol>,
    output_sections: &[OutputSection],
    section_map: &FxHashMap<(usize, usize), (usize, u64)>,
    out: &mut [u8],
    tls_info: &TlsInfo,
    got_info: &GotInfo,
) -> Result<(), String> {
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
            // Unloaded (debug) sections resolve statically in `debug.rs`.
            if output_sections[out_idx].flags & SHF_ALLOC == 0 {
                continue;
            }
            let sa = output_sections[out_idx].addr;
            let sfo = output_sections[out_idx].file_offset;

            for (ri, rela) in relas.iter().enumerate() {
                if rela.rela_type == R_AARCH64_NONE {
                    continue;
                }
                validate_address_site(
                    rela.rela_type,
                    rela.offset,
                    rela.sym_idx,
                    objects[obj_idx].section_data[sec_idx].len(),
                    objects[obj_idx].symbols.len(),
                )?;
                let si = rela.sym_idx as usize;
                if si >= objects[obj_idx].symbols.len() {
                    continue;
                }
                let sym = &objects[obj_idx].symbols[si];
                if is_relaxed_tls_call(relas, ri, &sym.name) {
                    continue;
                }
                let p = sa + sec_off + rela.offset;
                let fp = (sfo + sec_off + rela.offset) as usize;
                let a = rela.addend;
                // ADR/ADRP write `S + A - P` into the instruction, so the
                // target has to own a load address. `apply_one_reloc` only
                // sees the resolved number, where an addressless symbol is
                // indistinguishable from a legitimate one (zero is mappable),
                // so the section proof happens here while the symbol is still
                // in hand.
                if matches!(
                    rela.rela_type,
                    R_AARCH64_ADR_PREL_LO21 | R_AARCH64_ADR_PREL_PG_HI21
                ) && !adr_target_is_addressable(
                    obj_idx,
                    sym,
                    globals,
                    section_map,
                    output_sections,
                ) {
                    return Err(adr_section_error(&sym.name));
                }
                let s = resolve_sym(obj_idx, sym, globals, section_map, output_sections);
                let gkey = got_key(obj_idx, sym);

                apply_one_reloc(
                    out,
                    fp,
                    rela.rela_type,
                    s,
                    a,
                    p,
                    &sym.name,
                    &objects[obj_idx].source_name,
                    tls_info,
                    got_info,
                    &gkey,
                )?;
            }
        }
    }
    Ok(())
}

/// Compute TP offset for AArch64. On AArch64, the TLS block starts at TP + 16
/// (TP points to the DTV, TLS block is right after).
/// For Local Exec: tp_offset = sym_addr - tls_start_addr + 16
/// Whether `LINKER_DEBUG_TLS` is set, read once.
///
/// This was `std::env::var("LINKER_DEBUG_TLS").is_ok()` inline in the relocation
/// arms, i.e. a lock, a scan of the environment block and a `String` allocation
/// for *every TLS relocation in the link* -- paid whether or not anybody is
/// debugging, on the path that runs millions of times for a kernel-sized link.
/// The variable cannot change during a link, so `OnceLock` turns it into one
/// atomic load after the first call.
pub(super) fn tls_debug() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("LINKER_DEBUG_TLS").is_ok())
}

fn tprel(s: u64, a: i64, tls_info: &TlsInfo) -> i64 {
    // AArch64 uses variant 1 TLS: TP points to start of TCB (16 bytes),
    // followed by TLS block. So offset from TP = (S+A - tls_base) + 16
    if tls_info.tls_addr == 0 {
        return (s as i64).wrapping_add(a);
    }
    let sa = (s as i64).wrapping_add(a) as u64;
    (sa as i64) - (tls_info.tls_addr as i64) + 16
}

/// Apply a single AArch64 relocation.
pub fn apply_one_reloc(
    out: &mut [u8],
    fp: usize,
    rtype: u32,
    s: u64,
    a: i64,
    p: u64,
    sym_name: &str,
    source: &str,
    tls_info: &TlsInfo,
    got_info: &GotInfo,
    got_key: &str,
) -> Result<(), String> {
    match rtype {
        R_AARCH64_NONE => {}

        // ── Absolute relocations ──
        R_AARCH64_ABS64 => {
            let val = (s as i64).wrapping_add(a) as u64;
            w64(out, fp, val);
        }
        R_AARCH64_ABS32 | R_AARCH64_ABS16 => {
            write_checked_data(out, fp, rtype, (s as i64).wrapping_add(a), sym_name)?;
        }

        // ── PC-relative relocations ──
        R_AARCH64_PREL64 => {
            let val = (s as i64).wrapping_add(a).wrapping_sub(p as i64);
            w64(out, fp, val as u64);
        }
        R_AARCH64_PREL32 | R_AARCH64_PREL16 => {
            let val = (s as i64).wrapping_add(a).wrapping_sub(p as i64);
            write_checked_data(out, fp, rtype, val, sym_name)?;
        }

        // ── ADRP: Page-relative high 21 bits ──
        R_AARCH64_ADR_PREL_PG_HI21 => {
            let sa = (s as i64).wrapping_add(a) as u64;
            let page_s = sa & !0xFFF;
            let page_p = p & !0xFFF;
            let offset = (page_s as i64).wrapping_sub(page_p as i64);
            let imm = offset >> 12;
            encode_adrp(out, fp, imm)?;
        }

        // ── ADR: PC-relative low 21 bits ──
        R_AARCH64_ADR_PREL_LO21 => {
            let offset = (s as i64).wrapping_add(a).wrapping_sub(p as i64);
            encode_adr(out, fp, offset)?;
        }

        // ── ADD: absolute low 12 bits (no carry check) ──
        R_AARCH64_ADD_ABS_LO12_NC => {
            let sa = (s as i64).wrapping_add(a) as u64;
            encode_add_imm12(out, fp, (sa & 0xFFF) as u32);
        }

        // ── Load/store low 12 bits for different sizes ──
        R_AARCH64_LDST8_ABS_LO12_NC => {
            let sa = (s as i64).wrapping_add(a) as u64;
            encode_ldst_imm12(out, fp, (sa & 0xFFF) as u32, 0);
        }
        R_AARCH64_LDST16_ABS_LO12_NC => {
            let sa = (s as i64).wrapping_add(a) as u64;
            encode_ldst_imm12(out, fp, (sa & 0xFFF) as u32, 1);
        }
        R_AARCH64_LDST32_ABS_LO12_NC => {
            let sa = (s as i64).wrapping_add(a) as u64;
            encode_ldst_imm12(out, fp, (sa & 0xFFF) as u32, 2);
        }
        R_AARCH64_LDST64_ABS_LO12_NC => {
            let sa = (s as i64).wrapping_add(a) as u64;
            encode_ldst_imm12(out, fp, (sa & 0xFFF) as u32, 3);
        }
        R_AARCH64_LDST128_ABS_LO12_NC => {
            let sa = (s as i64).wrapping_add(a) as u64;
            encode_ldst_imm12(out, fp, (sa & 0xFFF) as u32, 4);
        }

        // ── Branch instructions ──
        R_AARCH64_CALL26 | R_AARCH64_JUMP26 => {
            let sa = (s as i64).wrapping_add(a) as u64;
            if sa == 0 {
                // Undefined weak symbol resolved to 0: the call becomes a NOP
                // (bfd's and lld's rule for a branch to an undefined weak).
                if fp + 4 > out.len() {
                    return Err(site_past_end(rtype, fp, out.len()));
                }
                w32(out, fp, 0xd503201f);
            } else {
                encode_branch(out, fp, rtype, (sa as i64).wrapping_sub(p as i64), sym_name)?;
            }
        }

        // ── B.cond/CBZ/CBNZ, LDR (literal), TBZ/TBNZ ──
        R_AARCH64_CONDBR19 | R_AARCH64_LD_PREL_LO19 | R_AARCH64_TSTBR14 => {
            let disp = (s as i64).wrapping_add(a).wrapping_sub(p as i64);
            encode_branch(out, fp, rtype, disp, sym_name)?;
        }

        // ── MOVW relocations (group relocations, see encode_movw_group) ──
        R_AARCH64_MOVW_UABS_G0
        | R_AARCH64_MOVW_UABS_G0_NC
        | R_AARCH64_MOVW_UABS_G1
        | R_AARCH64_MOVW_UABS_G1_NC
        | R_AARCH64_MOVW_UABS_G2
        | R_AARCH64_MOVW_UABS_G2_NC
        | R_AARCH64_MOVW_UABS_G3
        | R_AARCH64_MOVW_SABS_G0
        | R_AARCH64_MOVW_SABS_G1
        | R_AARCH64_MOVW_SABS_G2 => {
            encode_movw_group(out, fp, rtype, (s as i64).wrapping_add(a), sym_name)?;
        }

        // ── GOT relocations ──
        // Even in static linking, we use a real GOT since the instruction
        // is an LDR (load from memory), not an ADD.
        R_AARCH64_ADR_GOT_PAGE => {
            if let Some(got_entry_addr) = got_info.entry_addr(got_key) {
                let page_g = got_entry_addr & !0xFFF;
                let page_p = p & !0xFFF;
                let imm = (page_g as i64).wrapping_sub(page_p as i64) >> 12;
                encode_adrp(out, fp, imm)?;
            } else {
                // Fallback: treat like ADR_PREL_PG_HI21
                let sa = (s as i64).wrapping_add(a) as u64;
                let page_s = sa & !0xFFF;
                let page_p = p & !0xFFF;
                let imm = (page_s as i64).wrapping_sub(page_p as i64) >> 12;
                encode_adrp(out, fp, imm)?;
            }
        }
        R_AARCH64_LD64_GOT_LO12_NC => {
            if let Some(got_entry_addr) = got_info.entry_addr(got_key) {
                encode_ldst_imm12(out, fp, (got_entry_addr & 0xFFF) as u32, 3);
            } else {
                let sa = (s as i64).wrapping_add(a) as u64;
                encode_ldst_imm12(out, fp, (sa & 0xFFF) as u32, 3);
            }
        }

        // ── TLS Local Exec (LE) relocations ──
        // These are used when the TLS variable is in the executable itself.
        // On AArch64 variant 1, tp offset = sym_offset_in_tls + 16 (TCB size)
        R_AARCH64_TLSLE_ADD_TPREL_HI12 => {
            let tp = tprel(s, a, tls_info);
            if tls_debug() {
                eprintln!(
                    "  TLSLE_HI12: sym='{}' s=0x{:x} a={} tls_addr=0x{:x} tls_size=0x{:x} -> tp=0x{:x}",
                    sym_name, s, a, tls_info.tls_addr, tls_info.tls_size, tp as u64
                );
            }
            let imm12 = ((tp as u64 >> 12) & 0xFFF) as u32;
            encode_add_imm12(out, fp, imm12);
        }
        R_AARCH64_TLSLE_ADD_TPREL_LO12 | R_AARCH64_TLSLE_ADD_TPREL_LO12_NC => {
            let tp = tprel(s, a, tls_info);
            if tls_debug() {
                eprintln!(
                    "  TLSLE_LO12: sym='{}' s=0x{:x} a={} tls_addr=0x{:x} -> tp=0x{:x}",
                    sym_name, s, a, tls_info.tls_addr, tp as u64
                );
            }
            let imm12 = (tp as u64 & 0xFFF) as u32;
            encode_add_imm12(out, fp, imm12);
        }
        R_AARCH64_TLSLE_MOVW_TPREL_G0 | R_AARCH64_TLSLE_MOVW_TPREL_G0_NC => {
            let tp = tprel(s, a, tls_info);
            encode_movw(out, fp, (tp as u64 & 0xffff) as u32);
        }
        R_AARCH64_TLSLE_MOVW_TPREL_G1 | R_AARCH64_TLSLE_MOVW_TPREL_G1_NC => {
            let tp = tprel(s, a, tls_info);
            encode_movw(out, fp, ((tp as u64 >> 16) & 0xffff) as u32);
        }
        R_AARCH64_TLSLE_MOVW_TPREL_G2 => {
            let tp = tprel(s, a, tls_info);
            encode_movw(out, fp, ((tp as u64 >> 32) & 0xffff) as u32);
        }

        // ── TLS IE (Initial Exec) via GOT for static linking ──
        // Instead of relaxing to MOVZ/MOVK (which can break if registers
        // differ between ADRP and LDR), we use real GOT entries that hold
        // the pre-computed TP offset. The instructions remain ADRP+LDR
        // pointing at our GOT entry.
        R_AARCH64_TLSIE_ADR_GOTTPREL_PAGE21 => {
            if let Some(got_entry_addr) = got_info.entry_addr(got_key) {
                let page_g = got_entry_addr & !0xFFF;
                let page_p = p & !0xFFF;
                let imm = (page_g as i64).wrapping_sub(page_p as i64) >> 12;
                encode_adrp(out, fp, imm)?;
            } else {
                // Fallback: relax to MOVZ
                let tp = tprel(s, a, tls_info);
                if fp + 4 > out.len() {
                    return Ok(());
                }
                let insn = read_u32(out, fp);
                let rd = insn & 0x1f;
                let imm16 = ((tp as u64 >> 16) & 0xffff) as u32;
                let new_insn = 0xd2a00000 | (imm16 << 5) | rd;
                w32(out, fp, new_insn);
            }
        }
        R_AARCH64_TLSIE_LD64_GOTTPREL_LO12_NC => {
            if let Some(got_entry_addr) = got_info.entry_addr(got_key) {
                encode_ldst_imm12(out, fp, (got_entry_addr & 0xFFF) as u32, 3);
            } else {
                // Fallback: relax to MOVK
                let tp = tprel(s, a, tls_info);
                if fp + 4 > out.len() {
                    return Ok(());
                }
                let insn = read_u32(out, fp);
                let rd = insn & 0x1f;
                let imm16 = (tp as u64 & 0xffff) as u32;
                let new_insn = 0xf2800000 | (imm16 << 5) | rd;
                w32(out, fp, new_insn);
            }
        }

        // ── TLS descriptor and general-dynamic access, in an executable ──
        // A variable the executable defines is relaxed to local-exec, one
        // imported from a shared library to initial-exec through its GOT
        // slot (TPREL64, `emit_dynamic`), as GNU ld does. The descriptor ABI
        // returns the TP offset in x0, the general-dynamic one the address.
        R_AARCH64_TLSDESC_ADR_PAGE21 | R_AARCH64_TLSGD_ADR_PAGE21 => {
            check_insn_site(out, fp, 4)?;
            if let Some(slot) = got_info.entry_addr(got_key) {
                let imm = ((slot & !0xFFF) as i64).wrapping_sub((p & !0xFFF) as i64) >> 12;
                encode_adrp(out, fp, imm)?;
            } else {
                let rd = read_u32(out, fp) & 0x1f;
                let tp = tprel(s, a, tls_info) as u64;
                w32(
                    out,
                    fp,
                    MOVZ_X_LSL16 | ((((tp >> 16) & 0xffff) as u32) << 5) | rd,
                );
            }
        }
        R_AARCH64_TLSDESC_LD64_LO12 | R_AARCH64_TLSGD_ADD_LO12_NC => {
            check_insn_site(out, fp, 4)?;
            let insn = read_u32(out, fp);
            // The result register: the descriptor load's base (x0 by the
            // ABI; its destination is the resolver, x1), the add's target.
            let xd = if rtype == R_AARCH64_TLSDESC_LD64_LO12 {
                (insn >> 5) & 0x1f
            } else {
                insn & 0x1f
            };
            if let Some(slot) = got_info.entry_addr(got_key) {
                // ldr xd, [xd, #:lo12:slot]
                let imm12 = ((slot & 0xFFF) >> 3) as u32;
                w32(out, fp, LDR_X_UIMM | (imm12 << 10) | (xd << 5) | xd);
            } else {
                let tp = tprel(s, a, tls_info) as u64;
                w32(out, fp, MOVK_X | (((tp & 0xffff) as u32) << 5) | xd);
            }
            if rtype == R_AARCH64_TLSGD_ADD_LO12_NC {
                // `bl __tls_get_addr; nop` -> `mrs x1, tpidr_el0;
                // add x0, x1, x0` (the call's CALL26 is skipped, see
                // `is_relaxed_tls_call`).
                check_insn_site(out, fp + 4, 8)?;
                if read_u32(out, fp + 4) & 0xfc00_0000 != 0x9400_0000
                    || read_u32(out, fp + 8) != NOP
                {
                    return Err(format!(
                        "AArch64 general-dynamic TLS access to '{sym_name}' in {source} is not \
                         `add; bl __tls_get_addr; nop`"
                    ));
                }
                w32(out, fp + 4, MRS_X1_TPIDR_EL0);
                w32(out, fp + 8, ADD_X0_X1_X0);
            }
        }
        R_AARCH64_TLSDESC_ADD_LO12 | R_AARCH64_TLSDESC_CALL => {
            check_insn_site(out, fp, 4)?;
            w32(out, fp, NOP);
        }

        other => {
            return Err(format!(
                "unsupported AArch64 relocation type {} for '{}' in {}",
                other, sym_name, source
            ));
        }
    }
    Ok(())
}

/// Check the *input* instruction range before forming output addresses. An
/// output-only check can overwrite a neighboring input section, and a huge
/// r_offset can overflow address arithmetic before reaching the field patcher.
pub(super) fn validate_address_site(
    kind: u32,
    offset: u64,
    symbol: u32,
    section_size: usize,
    symbol_count: usize,
) -> Result<(), String> {
    if !matches!(
        kind,
        R_AARCH64_ADR_PREL_LO21
            | R_AARCH64_ADR_PREL_PG_HI21
            | R_AARCH64_ADR_GOT_PAGE
            | R_AARCH64_TLSIE_ADR_GOTTPREL_PAGE21
            | R_AARCH64_TLSDESC_ADR_PAGE21
            | R_AARCH64_TLSGD_ADR_PAGE21
    ) {
        return Ok(());
    }
    if symbol as usize >= symbol_count {
        return Err(format!(
            "ADR/ADRP relocation has invalid symbol index {symbol}"
        ));
    }
    if offset
        .checked_add(4)
        .is_none_or(|end| end > section_size as u64)
    {
        return Err(format!(
            "ADR/ADRP relocation site {offset:#x} exceeds input section size {section_size:#x}"
        ));
    }
    Ok(())
}

// ── Instruction encoding helpers ───────────────────────────────────────

/// The ELF name of a relocation type these encoders check, for diagnostics.
fn reloc_name(rtype: u32) -> String {
    let name = match rtype {
        R_AARCH64_ABS32 => "R_AARCH64_ABS32",
        R_AARCH64_ABS16 => "R_AARCH64_ABS16",
        R_AARCH64_PREL32 => "R_AARCH64_PREL32",
        R_AARCH64_PREL16 => "R_AARCH64_PREL16",
        R_AARCH64_CALL26 => "R_AARCH64_CALL26",
        R_AARCH64_JUMP26 => "R_AARCH64_JUMP26",
        R_AARCH64_CONDBR19 => "R_AARCH64_CONDBR19",
        R_AARCH64_LD_PREL_LO19 => "R_AARCH64_LD_PREL_LO19",
        R_AARCH64_TSTBR14 => "R_AARCH64_TSTBR14",
        R_AARCH64_MOVW_UABS_G0 => "R_AARCH64_MOVW_UABS_G0",
        R_AARCH64_MOVW_UABS_G1 => "R_AARCH64_MOVW_UABS_G1",
        R_AARCH64_MOVW_UABS_G2 => "R_AARCH64_MOVW_UABS_G2",
        R_AARCH64_MOVW_SABS_G0 => "R_AARCH64_MOVW_SABS_G0",
        R_AARCH64_MOVW_SABS_G1 => "R_AARCH64_MOVW_SABS_G1",
        R_AARCH64_MOVW_SABS_G2 => "R_AARCH64_MOVW_SABS_G2",
        _ => return format!("relocation type {rtype}"),
    };
    name.to_string()
}

/// GNU ld's overflow diagnostic: the value does not fit the relocated field.
pub(super) fn truncated(rtype: u32, sym: &str) -> String {
    let sym = if sym.is_empty() {
        "a section symbol"
    } else {
        sym
    };
    format!(
        "relocation truncated to fit: {} against `{sym}'",
        reloc_name(rtype)
    )
}

fn site_past_end(rtype: u32, fp: usize, len: usize) -> String {
    format!(
        "{} site {fp:#x} exceeds output size {len:#x}",
        reloc_name(rtype)
    )
}

/// Encode the displacement of a B/BL (imm26 at bit 0), B.cond/CBZ/CBNZ/LDR
/// literal (imm19 at bit 5) or TBZ/TBNZ (imm14 at bit 5): word-aligned and
/// within the signed field, else `relocation truncated to fit` -- never the
/// masked low bits, which branch or load somewhere else.
pub(super) fn encode_branch(
    out: &mut [u8],
    fp: usize,
    rtype: u32,
    disp: i64,
    sym: &str,
) -> Result<(), String> {
    let (bits, lsb) = match rtype {
        R_AARCH64_CALL26 | R_AARCH64_JUMP26 => (26, 0),
        R_AARCH64_CONDBR19 | R_AARCH64_LD_PREL_LO19 => (19, 5),
        R_AARCH64_TSTBR14 => (14, 5),
        _ => return Err(format!("{} is not a branch relocation", reloc_name(rtype))),
    };
    // imm is a signed word count: bytes in [-2^(bits+1), 2^(bits+1)).
    let span = 1i64 << (bits + 1);
    if disp & 3 != 0 || !(-span..span).contains(&disp) {
        return Err(truncated(rtype, sym));
    }
    if fp.checked_add(4).is_none_or(|end| end > out.len()) {
        return Err(site_past_end(rtype, fp, out.len()));
    }
    let mask = ((1u32 << bits) - 1) << lsb;
    let insn = (read_u32(out, fp) & !mask) | ((((disp >> 2) as u32) << lsb) & mask);
    w32(out, fp, insn);
    Ok(())
}

/// Write an ABS16/ABS32/PREL16/PREL32 value with the AArch64 ELF overflow
/// check `-2^(n-1) <= X < 2^n` (either signedness fits).
pub(super) fn write_checked_data(
    out: &mut [u8],
    fp: usize,
    rtype: u32,
    val: i64,
    sym: &str,
) -> Result<(), String> {
    let n: u32 = if matches!(rtype, R_AARCH64_ABS32 | R_AARCH64_PREL32) {
        32
    } else {
        16
    };
    if !(-(1i64 << (n - 1))..(1i64 << n)).contains(&val) {
        return Err(truncated(rtype, sym));
    }
    if fp
        .checked_add(n as usize / 8)
        .is_none_or(|end| end > out.len())
    {
        return Err(site_past_end(rtype, fp, out.len()));
    }
    if n == 32 {
        w32(out, fp, val as u32);
    } else {
        w16(out, fp, val as u16);
    }
    Ok(())
}

/// MOVZ/MOVK/MOVN group relocations: the field is bits [16g, 16g+16) of X.
/// The checked forms require X to fit in 16(g+1) bits -- unsigned for
/// UABS_Gg, signed for SABS_Gg, whose instruction becomes MOVZ for X >= 0
/// and MOVN with field ~X for X < 0 (AArch64 ELF ABI; pre-fix the raw
/// two's-complement bits went into a MOVZ, so a negative X yielded a large
/// positive value). The _NC forms and G3 do not check.
pub(super) fn encode_movw_group(
    out: &mut [u8],
    fp: usize,
    rtype: u32,
    x: i64,
    sym: &str,
) -> Result<(), String> {
    let (group, check, signed) = match rtype {
        R_AARCH64_MOVW_UABS_G0 => (0, true, false),
        R_AARCH64_MOVW_UABS_G0_NC => (0, false, false),
        R_AARCH64_MOVW_UABS_G1 => (1, true, false),
        R_AARCH64_MOVW_UABS_G1_NC => (1, false, false),
        R_AARCH64_MOVW_UABS_G2 => (2, true, false),
        R_AARCH64_MOVW_UABS_G2_NC => (2, false, false),
        R_AARCH64_MOVW_UABS_G3 => (3, false, false),
        R_AARCH64_MOVW_SABS_G0 => (0, true, true),
        R_AARCH64_MOVW_SABS_G1 => (1, true, true),
        R_AARCH64_MOVW_SABS_G2 => (2, true, true),
        _ => return Err(format!("{} is not a MOVW relocation", reloc_name(rtype))),
    };
    let width = 16 * (group + 1);
    if check {
        let fits = if signed {
            (-(1i64 << width)..(1i64 << width)).contains(&x)
        } else {
            (x as u64) < (1u64 << width)
        };
        if !fits {
            return Err(truncated(rtype, sym));
        }
    }
    if fp.checked_add(4).is_none_or(|end| end > out.len()) {
        return Err(site_past_end(rtype, fp, out.len()));
    }
    let mut insn = read_u32(out, fp);
    let mut v = x as u64;
    if signed {
        // opc (bits 30:29): 10 = MOVZ, 00 = MOVN.
        if x < 0 {
            v = !v;
            insn &= !(1 << 30);
        } else {
            insn |= 1 << 30;
        }
    }
    let imm16 = ((v >> (16 * group)) & 0xffff) as u32;
    w32(out, fp, (insn & 0xffe0001f) | (imm16 << 5));
    Ok(())
}

pub(super) fn encode_adrp(out: &mut [u8], fp: usize, imm: i64) -> Result<(), String> {
    encode_adr(out, fp, imm)
}

pub(super) fn encode_adr(out: &mut [u8], fp: usize, offset: i64) -> Result<(), String> {
    let bits = crate::backend::arm::address::adr_imm_bits(offset)?;
    let end = fp
        .checked_add(4)
        .filter(|&end| end <= out.len())
        .ok_or_else(|| {
            format!(
                "ADR/ADRP relocation site {fp:#x} exceeds output size {:#x}",
                out.len()
            )
        })?;
    let insn = read_u32(out, fp);
    out[fp..end].copy_from_slice(&((insn & 0x9f00001f) | bits).to_le_bytes());
    Ok(())
}

pub(super) fn encode_add_imm12(out: &mut [u8], fp: usize, imm12: u32) {
    if fp + 4 > out.len() {
        return;
    }
    let mut insn = read_u32(out, fp);
    insn = (insn & 0xffc003ff) | ((imm12 & 0xfff) << 10);
    w32(out, fp, insn);
}

pub(super) fn encode_ldst_imm12(out: &mut [u8], fp: usize, lo12: u32, shift: u32) {
    if fp + 4 > out.len() {
        return;
    }
    let mut insn = read_u32(out, fp);
    let imm12 = (lo12 >> shift) & 0xfff;
    insn = (insn & 0xffc003ff) | (imm12 << 10);
    w32(out, fp, insn);
}

pub(super) fn encode_movw(out: &mut [u8], fp: usize, imm16: u32) {
    if fp + 4 > out.len() {
        return;
    }
    let mut insn = read_u32(out, fp);
    insn = (insn & 0xffe0001f) | ((imm16 & 0xffff) << 5);
    w32(out, fp, insn);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Apply one relocation to a `MOVZ x0, #imm` and return the immediate that
    /// landed in bits 20:5.
    ///
    /// `tls_addr == 0` makes [`tprel`] return `s + a` unchanged, so the test
    /// controls the TP offset exactly by passing it as `s`.
    fn movw_imm(rtype: u32, tp: u64) -> u32 {
        let mut out = 0xd280_0000u32.to_le_bytes().to_vec();
        let tls = TlsInfo {
            tls_addr: 0,
            tls_size: 0,
        };
        let got = GotInfo {
            got_addr: 0,
            entries: FxHashMap::default(),
        };
        apply_one_reloc(
            &mut out, 0, rtype, tp, 0, 0, "tlsvar", "t.o", &tls, &got, "",
        )
        .expect("a TLS local-exec MOVW relocation must apply");
        let insn = u32::from_le_bytes(out[..4].try_into().unwrap());
        (insn >> 5) & 0xffff
    }

    #[test]
    fn tls_movw_tprel_numbers_are_the_elf_h_numbers() {
        // <elf.h> assigns this family in order of descending significance, which
        // is the opposite of what reading the names suggests. Pinning the numbers
        // is the regression test for the rotation this file had: G0 was 544 and
        // G2 was 548, so four of the five types were encoded with the wrong
        // shift and the `_NC` forms suppressed the only diagnostic that could
        // have caught it.
        assert_eq!(R_AARCH64_TLSLE_MOVW_TPREL_G2, 544);
        assert_eq!(R_AARCH64_TLSLE_MOVW_TPREL_G1, 545);
        assert_eq!(R_AARCH64_TLSLE_MOVW_TPREL_G1_NC, 546);
        assert_eq!(R_AARCH64_TLSLE_MOVW_TPREL_G0, 547);
        assert_eq!(R_AARCH64_TLSLE_MOVW_TPREL_G0_NC, 548);
        // The invariant behind the numbers, stated so a future edit cannot
        // restore the rotation while keeping the test passing by renumbering:
        // significance descends as the number ascends.
        //
        // Compile-time: the constants are fixed at build time, so the check
        // belongs in a const block and a regression fails the build rather
        // than the test run.
        const {
            assert!(
                R_AARCH64_TLSLE_MOVW_TPREL_G2 < R_AARCH64_TLSLE_MOVW_TPREL_G1
                    && R_AARCH64_TLSLE_MOVW_TPREL_G1 < R_AARCH64_TLSLE_MOVW_TPREL_G0
            );
        }
        // A `_NC` form is the checked one plus 1, never minus 1.
        assert_eq!(
            R_AARCH64_TLSLE_MOVW_TPREL_G0_NC,
            R_AARCH64_TLSLE_MOVW_TPREL_G0 + 1
        );
        assert_eq!(
            R_AARCH64_TLSLE_MOVW_TPREL_G1_NC,
            R_AARCH64_TLSLE_MOVW_TPREL_G1 + 1
        );
    }

    #[test]
    fn each_tls_movw_type_selects_its_own_16_bit_slice() {
        // A different pattern in every slice, so a wrong shift is visible rather
        // than merely plausible.
        let tp: u64 = 0x0006_0005_0004_1234;
        assert_eq!(movw_imm(R_AARCH64_TLSLE_MOVW_TPREL_G0, tp), 0x1234);
        assert_eq!(movw_imm(R_AARCH64_TLSLE_MOVW_TPREL_G0_NC, tp), 0x1234);
        assert_eq!(movw_imm(R_AARCH64_TLSLE_MOVW_TPREL_G1, tp), 0x0004);
        assert_eq!(movw_imm(R_AARCH64_TLSLE_MOVW_TPREL_G1_NC, tp), 0x0004);
        assert_eq!(movw_imm(R_AARCH64_TLSLE_MOVW_TPREL_G2, tp), 0x0005);
    }

    #[test]
    fn the_rotation_this_replaced_is_excluded_by_number_not_by_name() {
        // 547 is G0. Under the old numbering 547 named G1_NC, whose arm applies
        // `tp >> 16` -- so this assertion fails if the numbers are ever rotated
        // back, even though the arm bodies (which are keyed by name) never move.
        let tp: u64 = 0x0000_0000_0007_0009;
        assert_eq!(movw_imm(547, tp), 0x0009, "547 must take bits 15:0");
        assert_ne!(
            movw_imm(547, tp),
            0x0007,
            "547 must not take bits 31:16, which is what the rotation produced"
        );
        // And the other end of the family: 544 is G2, not G0.
        let hi: u64 = 0x0000_00ab_0000_0000;
        assert_eq!(movw_imm(544, hi), 0x00ab, "544 must take bits 47:32");
    }
}

#[cfg(test)]
mod adr_checked_tests {
    use super::*;

    #[test]
    fn shared_adr_requires_section_provenance_not_nonzero_address() {
        let mut symbol = Symbol {
            name_idx: 0,
            name: "local".into(),
            info: STT_OBJECT,
            other: 0,
            shndx: 1,
            value: 0,
            size: 0,
        };
        let mut sections = vec![OutputSection {
            name: ".data".into(),
            sh_type: 1,
            flags: SHF_ALLOC,
            alignment: 1,
            inputs: vec![],
            data: vec![],
            addr: 0,
            file_offset: 0,
            mem_size: 8,
        }];
        let mut map = FxHashMap::default();
        map.insert((0, 1), (0, 0));
        assert_eq!(
            resolve_shared_adr_target(0, &symbol, &map, &sections),
            Some(0)
        );
        // Object identity, missing/discarded mappings, and invalid output indexes.
        assert_eq!(resolve_shared_adr_target(1, &symbol, &map, &sections), None);
        map.insert((0, 1), (1, 0));
        assert_eq!(resolve_shared_adr_target(0, &symbol, &map, &sections), None);
        map.insert((0, 1), (0, 0));
        for shndx in [SHN_UNDEF, SHN_ABS, SHN_COMMON, 0xff00, 2] {
            symbol.shndx = shndx;
            assert_eq!(resolve_shared_adr_target(0, &symbol, &map, &sections), None);
        }
        symbol.shndx = 1;
        for flags in [0, SHF_ALLOC | SHF_TLS] {
            sections[0].flags = flags;
            assert_eq!(resolve_shared_adr_target(0, &symbol, &map, &sections), None);
        }
        sections[0].flags = SHF_ALLOC;
        for binding in [STB_GLOBAL, STB_WEAK] {
            symbol.info = binding << 4 | STT_OBJECT;
            for visibility in 0..4 {
                symbol.other = visibility;
                assert_eq!(resolve_shared_adr_target(0, &symbol, &map, &sections), None);
            }
        }
        symbol.info = STT_SECTION;
        symbol.value = 4;
        assert_eq!(
            resolve_shared_adr_target(0, &symbol, &map, &sections),
            Some(4)
        );
        sections[0].addr = u64::MAX;
        assert_eq!(resolve_shared_adr_target(0, &symbol, &map, &sections), None);
    }

    /// The ADR/ADRP admission predicate is the only thing standing between a
    /// kept-but-unloaded section and an instruction that points at a file
    /// offset. Every branch is pinned here: a section that survives allocated
    /// is addressable, one that survives unallocated or was discarded is not,
    /// and the reserved indices keep their by-meaning treatment.
    #[test]
    fn adr_targets_need_an_allocated_section() {
        let mut symbol = Symbol {
            name_idx: 0,
            name: "".into(),
            info: 0, // STB_LOCAL | STT_NOTYPE
            other: 0,
            shndx: 1,
            value: 0,
            size: 0,
        };
        let section = |flags: u64| OutputSection {
            name: ".text".into(),
            sh_type: 1,
            flags,
            alignment: 1,
            inputs: vec![],
            data: vec![],
            addr: 0,
            file_offset: 0,
            mem_size: 8,
        };
        let sections = vec![section(SHF_ALLOC), section(0)];
        let mut map = FxHashMap::default();
        map.insert((0, 1), (0, 0));
        map.insert((1, 1), (0, 0));
        let globals: FxHashMap<String, GlobalSymbol> = FxHashMap::default();
        let addressable = |obj_idx: usize,
                           sym: &Symbol,
                           map: &FxHashMap<(usize, usize), (usize, u64)>,
                           sections: &[OutputSection]| {
            adr_target_is_addressable(obj_idx, sym, &globals, map, sections)
        };

        // Local in a loaded section: the ordinary case.
        assert!(addressable(0, &symbol, &map, &sections));
        // The same symbol in a section kept only as file content: no address.
        map.insert((0, 1), (1, 0));
        assert!(!addressable(0, &symbol, &map, &sections));
        // Discarded entirely (the pre-`keep .debug_*` behaviour).
        map.remove(&(0, 1));
        assert!(!addressable(0, &symbol, &map, &sections));
        map.insert((0, 1), (0, 0));

        // Reserved indices: ABS and COMMON name values, so they stay legal;
        // every other reserved index is not an input section at all.
        for (shndx, expected) in [
            (SHN_UNDEF, false), // a local reference with no definition
            (SHN_ABS, true),
            (SHN_COMMON, true),
            (SHN_LORESERVE, false),
            (0xfff1, true),  // SHN_ABS under a different spelling
            (0xffff, false), // SHN_XINDEX: no section to prove
        ] {
            symbol.shndx = shndx;
            assert_eq!(
                addressable(0, &symbol, &map, &sections),
                expected,
                "{shndx:#x}"
            );
        }
        symbol.shndx = 1;

        // A global defined in another object is judged by *that* object's
        // section, which is why the predicate cannot look at the reference's
        // own (undefined) st_shndx.
        let mut globals: FxHashMap<String, GlobalSymbol> = FxHashMap::default();
        let mut global = Symbol {
            name: "target".into(),
            info: (STB_GLOBAL << 4) | STT_OBJECT,
            shndx: SHN_UNDEF,
            ..symbol.clone()
        };
        globals.insert(
            "target".into(),
            GlobalSymbol {
                value: 0,
                size: 0,
                info: (STB_GLOBAL << 4) | STT_OBJECT,
                defined_in: Some(1),
                section_idx: 1,
                from_lib: None,
                plt_idx: None,
                got_idx: None,
                is_dynamic: false,
                copy_reloc: false,
                lib_sym_value: 0,
                lib_in_exec: false,
                visibility: 0,
            },
        );
        // Object 1's section 1 is the unallocated one via `map` above.
        let mut map2 = FxHashMap::default();
        map2.insert((1, 1), (1, 0));
        assert!(!adr_target_is_addressable(
            0, &global, &globals, &map2, &sections
        ));
        map2.insert((1, 1), (0, 0));
        assert!(adr_target_is_addressable(
            0, &global, &globals, &map2, &sections
        ));
        // Linker-defined symbols (defined_in == usize::MAX) are allocated by
        // construction and never consult the map.
        globals.get_mut("target").unwrap().defined_in = Some(usize::MAX);
        assert!(adr_target_is_addressable(
            0, &global, &globals, &map2, &sections
        ));
        // Still-undefined global: the callers' weak/preemptible rules decide.
        globals.get_mut("target").unwrap().defined_in = None;
        assert!(adr_target_is_addressable(
            0, &global, &globals, &map2, &sections
        ));
        // A named global with no table entry at all is treated the same way.
        global.name = "absent".into();
        assert!(adr_target_is_addressable(
            0, &global, &globals, &map2, &sections
        ));
    }

    #[test]
    fn input_site_validation_precedes_address_arithmetic() {
        for kind in [
            R_AARCH64_ADR_PREL_LO21,
            R_AARCH64_ADR_PREL_PG_HI21,
            R_AARCH64_ADR_GOT_PAGE,
            R_AARCH64_TLSIE_ADR_GOTTPREL_PAGE21,
            R_AARCH64_TLSDESC_ADR_PAGE21,
            R_AARCH64_TLSGD_ADR_PAGE21,
        ] {
            assert!(validate_address_site(kind, 4, 1, 8, 2).is_ok());
            for offset in [5, 8, u64::MAX] {
                assert!(validate_address_site(kind, offset, 1, 8, 2).is_err());
            }
            assert!(validate_address_site(kind, 0, 2, 8, 2).is_err());
        }
    }

    #[test]
    fn checked_address_patch_is_atomic_and_preserves_opcode_and_register() {
        for opcode in [0x10000000_u32, 0x90000000] {
            for rd in 0..32 {
                for value in [-1048576, -1, 0, 1, 1048575] {
                    let original = (opcode | rd).to_le_bytes();
                    let mut out = original;
                    encode_adr(&mut out, 0, value).unwrap();
                    assert_eq!(u32::from_le_bytes(out) & 0x9f00001f, opcode | rd);
                    for bad in [i64::MIN, -1048577, 1048576, i64::MAX] {
                        let before = out;
                        assert!(encode_adr(&mut out, 0, bad).is_err());
                        assert_eq!(out, before);
                    }
                    for site in [1, 4, usize::MAX] {
                        let before = out;
                        assert!(encode_adr(&mut out, site, 0).is_err());
                        assert_eq!(out, before);
                    }
                }
            }
        }
    }
}
