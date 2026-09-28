//! i386 relocation application for the i686 linker.
//!
//! Applies relocations from input objects to merged output sections.
//! All i386 relocation types are handled here, separated from the main
//! linking logic to keep the code manageable.
//!
//! The relocation types supported include absolute (R_386_32), PC-relative
//! (R_386_PC32, R_386_PLT32), GOT-related (R_386_GOT32, R_386_GOT32X,
//! R_386_GOTPC, R_386_GOTOFF), and TLS relocations.  Dynamic-model TLS
//! accesses are transitioned to initial/local exec (`tls.rs`).

use crate::common::fx_hash::FxHashMap;

use super::types::*;

/// Context for relocation application, containing all addresses needed
/// to resolve relocations.
pub(super) struct RelocContext<'a> {
    pub global_symbols: &'a FxHashMap<String, LinkerSymbol>,
    pub output_sections: &'a mut Vec<OutputSection>,
    pub section_map: &'a SectionMap,
    pub got_base: u32,
    pub got_vaddr: u32,
    pub gotplt_vaddr: u32,
    pub got_reserved: usize,
    pub gotplt_reserved: u32,
    #[cfg_attr(not(feature = "gcc_linker"), expect(dead_code))]
    // Set by linker layout; available for future PLT-relative relocations
    pub plt_vaddr: u32,
    #[cfg_attr(not(feature = "gcc_linker"), expect(dead_code))]
    // Set by linker layout; available for future PLT-relative relocations
    pub plt_header_size: u32,
    #[cfg_attr(not(feature = "gcc_linker"), expect(dead_code))]
    // Set by linker layout; available for future PLT-relative relocations
    pub plt_entry_size: u32,
    pub num_plt: usize,
    pub tls_addr: u32,
    pub tls_mem_size: u32,
    pub has_tls: bool,
    /// Absolute addresses of the `___tls_get_addr` call fields of
    /// transitioned GD/LDM sequences: their relocations are dropped.
    pub tls_relaxed_call_slots: crate::common::fx_hash::FxHashSet<u32>,
}

/// Apply all relocations from input objects to the output sections.
pub(super) fn apply_relocations(
    inputs: &[InputObject],
    ctx: &mut RelocContext,
) -> Result<(), String> {
    for (obj_idx, obj) in inputs.iter().enumerate() {
        for sec in &obj.sections {
            if sec.relocations.is_empty() {
                continue;
            }

            let _out_name = match output_section_name(&sec.name, sec.flags, sec.sh_type) {
                Some(n) => n,
                None => continue,
            };
            let (out_sec_idx, sec_base_offset) =
                match ctx.section_map.get(&(obj_idx, sec.input_index)) {
                    Some(&v) => v,
                    None => continue,
                };

            // Register the section's vanishing `___tls_get_addr` calls
            // before any relocation is applied (the call's relocation may
            // precede the `@tlsgd` one in the table).
            let sec_addr = ctx.output_sections[out_sec_idx].addr + sec_base_offset;
            for field in super::tls::transitioned_call_fields(&sec.data, &sec.relocations) {
                ctx.tls_relaxed_call_slots.insert(sec_addr + field);
            }
            for &(rel_offset, rel_type, sym_idx, addend) in &sec.relocations {
                apply_one_reloc(
                    obj_idx,
                    obj,
                    sec,
                    out_sec_idx,
                    sec_base_offset,
                    rel_offset,
                    rel_type,
                    sym_idx,
                    addend,
                    ctx,
                )?;
            }
        }
    }
    Ok(())
}

/// Apply a single relocation.
fn apply_one_reloc(
    obj_idx: usize,
    obj: &InputObject,
    _sec: &InputSection,
    out_sec_idx: usize,
    sec_base_offset: u32,
    rel_offset: u32,
    rel_type: u32,
    sym_idx: u32,
    addend: i32,
    ctx: &mut RelocContext,
) -> Result<(), String> {
    let patch_offset = sec_base_offset + rel_offset;
    let patch_addr = ctx.output_sections[out_sec_idx].addr + patch_offset;

    let sym = if (sym_idx as usize) < obj.symbols.len() {
        &obj.symbols[sym_idx as usize]
    } else {
        return Err(format!("invalid symbol index {} in reloc", sym_idx));
    };

    let sym_addr = resolve_sym_addr(obj_idx, sym, ctx);

    // Check if this symbol goes through PLT
    let is_dyn = !sym.name.is_empty()
        && ctx
            .global_symbols
            .get(sym.name.as_str())
            .map(|gs| gs.is_dynamic && gs.needs_plt)
            .unwrap_or(false);

    let mut relax_got32x = false;

    let value: u32 = match rel_type {
        R_386_NONE => return Ok(()),
        R_386_32 => (sym_addr as i32 + addend) as u32,
        R_386_PC32 | R_386_PLT32 => {
            if ctx.tls_relaxed_call_slots.contains(&patch_addr) {
                // The `call ___tls_get_addr` of a transitioned GD/LDM
                // sequence (see `apply_tls_transition`) is gone.
                return Ok(());
            }
            let s = if is_dyn {
                ctx.global_symbols
                    .get(sym.name.as_str())
                    .map(|gs| gs.address)
                    .unwrap_or(0)
            } else {
                sym_addr
            };
            (s as i32 + addend - patch_addr as i32) as u32
        }
        R_386_GOTPC => (ctx.got_base as i32 + addend - patch_addr as i32) as u32,
        R_386_GOTOFF => (sym_addr as i32 + addend - ctx.got_base as i32) as u32,
        R_386_GOT32 | R_386_GOT32X => {
            if ctx.tls_relaxed_call_slots.contains(&patch_addr) {
                // `call *___tls_get_addr@GOT(%reg)` of a transitioned
                // GD/LDM sequence: the call no longer exists.
                return Ok(());
            }
            resolve_got_reloc(sym, sym_addr, addend, rel_type, ctx, &mut relax_got32x)
        }
        R_386_TLS_TPOFF | R_386_TLS_LE => {
            // Negative offset from TP
            let tpoff = sym_addr as i32 - ctx.tls_addr as i32 - ctx.tls_mem_size as i32;
            (tpoff + addend) as u32
        }
        R_386_TLS_LE_32 | R_386_TLS_TPOFF32 => {
            // ccc emits `add` with TLS_TPOFF32, so compute negative offset
            // (same as TLS_TPOFF/TLS_LE) to match the `add` instruction.
            let tpoff = sym_addr as i32 - ctx.tls_addr as i32 - ctx.tls_mem_size as i32;
            (tpoff + addend) as u32
        }
        R_386_TLS_GD | R_386_TLS_LDM | R_386_TLS_LDO_32 | R_386_TLS_IE | R_386_TLS_GOTIE
        | R_386_TLS_GOTDESC | R_386_TLS_DESC_CALL => {
            apply_tls_transition(
                obj,
                sym,
                sym_addr,
                addend,
                rel_type,
                out_sec_idx,
                patch_offset,
                ctx,
            )?;
            return Ok(());
        }
        R_386_TLS_DTPMOD32 => 1u32,
        R_386_TLS_DTPOFF32 => {
            if ctx.has_tls {
                (sym_addr as i32 - ctx.tls_addr as i32 + addend) as u32
            } else {
                addend as u32
            }
        }
        other => {
            return Err(format!(
                "unsupported i686 relocation type {} at {}:0x{:x}",
                other, obj.filename, rel_offset
            ));
        }
    };

    // Patch the output section data
    let out_sec = &mut ctx.output_sections[out_sec_idx];
    let off = patch_offset as usize;
    if off + 4 <= out_sec.data.len() {
        // For GOT32X relaxation, rewrite mov (0x8b) → lea (0x8d)
        if relax_got32x && off >= 2 && out_sec.data[off - 2] == 0x8b {
            out_sec.data[off - 2] = 0x8d;
        }
        out_sec.data[off..off + 4].copy_from_slice(&value.to_le_bytes());
    }

    Ok(())
}

/// Link one dynamic-model TLS relocation of an executable by transitioning
/// the access (see `tls.rs` for the model table and encodings).  Writes the
/// complete rewritten sequence, value included.
#[allow(clippy::too_many_arguments)]
fn apply_tls_transition(
    obj: &InputObject,
    sym: &InputSymbol,
    sym_addr: u32,
    addend: i32,
    rel_type: u32,
    out_sec_idx: usize,
    patch_offset: u32,
    ctx: &mut RelocContext,
) -> Result<(), String> {
    use super::tls;
    let off = patch_offset as usize;
    let sec_addr = ctx.output_sections[out_sec_idx].addr;
    let global = if sym.binding != STB_LOCAL && !sym.name.is_empty() && sym.sym_type != STT_SECTION
    {
        ctx.global_symbols.get(sym.name.as_str())
    } else {
        None
    };
    let dynamic = global.is_some_and(|g| g.is_dynamic);
    let what = || {
        format!(
            "`{}' in {} (offset 0x{:x})",
            sym.name, obj.filename, patch_offset
        )
    };
    let fail = |model: &str| format!("TLS transition of {model} access to {} failed", what());
    // Main-image offset from the thread pointer (variant II: negative).
    // An undefined weak TLS reference (glibc's static `_nl_current_*`
    // categories) resolves as S = 0, as in GNU ld; strong undefined
    // references were already reported by the undefined-symbol check.
    let undef_weak = global.is_some_and(|g| !g.is_defined && !g.is_dynamic);
    let ntpoff = || -> Result<i32, String> {
        if !ctx.has_tls && undef_weak {
            return Ok(addend);
        }
        if !ctx.has_tls {
            return Err(format!(
                "TLS relocation against {} but the output has no TLS segment",
                what()
            ));
        }
        Ok((sym_addr as i32)
            .wrapping_sub(ctx.tls_addr as i32)
            .wrapping_sub(ctx.tls_mem_size as i32)
            .wrapping_add(addend))
    };
    // A DSO variable's TLS_TPOFF slot (allocated by mark_plt_got_needs).
    let slot_addr =
        |ctx: &RelocContext| -> Result<u32, String> {
            match global {
                Some(gs) if gs.needs_got => Ok(ctx.got_vaddr
                    + (ctx.got_reserved as u32 + (gs.got_index - ctx.num_plt) as u32) * 4),
                _ => Err(format!("internal error: no TLS GOT slot for {}", what())),
            }
        };
    let data = &mut ctx.output_sections[out_sec_idx].data;
    match rel_type {
        R_386_TLS_GD | R_386_TLS_LDM => {
            let model = if rel_type == R_386_TLS_GD {
                "general-dynamic"
            } else {
                "local-dynamic"
            };
            let Some(seq) = tls::gd_sequence(data, off) else {
                return Err(format!(
                    "{}: not a psABI `leal x@tls{}(%reg), %eax; call ___tls_get_addr` sequence",
                    fail(model),
                    if rel_type == R_386_TLS_GD {
                        "gd"
                    } else {
                        "ldm"
                    }
                ));
            };
            if rel_type == R_386_TLS_LDM {
                tls::ldm_to_le(data, &seq);
            } else if dynamic {
                let gotoff = slot_addr(ctx)?.wrapping_sub(ctx.got_base) as i32;
                let data = &mut ctx.output_sections[out_sec_idx].data;
                if !tls::gd_to_ie(data, &seq, gotoff) {
                    return Err(format!(
                        "{}: the initial-exec form needs the 12-byte sequence with a non-%eax GOT register (psABI: `leal x@tlsgd(,%ebx,1)`, or a `nop` after the direct call)",
                        fail(model)
                    ));
                }
            } else {
                let v = ntpoff()?;
                tls::gd_to_le(&mut ctx.output_sections[out_sec_idx].data, &seq, v);
            }
            // `apply_relocations` registered this call before any
            // relocation of the section was applied.
            debug_assert!(
                ctx.tls_relaxed_call_slots
                    .contains(&(sec_addr + seq.call_field as u32))
            );
        }
        R_386_TLS_LDO_32 => {
            // After LD→LE `%eax` holds the thread pointer itself.
            let v = ntpoff()?;
            data[off..off + 4].copy_from_slice(&(v as u32).to_le_bytes());
        }
        R_386_TLS_IE => {
            if dynamic {
                let v = slot_addr(ctx)?.wrapping_add(addend as u32);
                ctx.output_sections[out_sec_idx].data[off..off + 4]
                    .copy_from_slice(&v.to_le_bytes());
            } else {
                let v = ntpoff()?;
                tls::ie_to_le(&mut ctx.output_sections[out_sec_idx].data, off, v)
                    .map_err(|e| format!("{}: {e}", fail("initial-exec")))?;
            }
        }
        R_386_TLS_GOTIE => {
            if dynamic {
                let v = slot_addr(ctx)?
                    .wrapping_add(addend as u32)
                    .wrapping_sub(ctx.got_base);
                ctx.output_sections[out_sec_idx].data[off..off + 4]
                    .copy_from_slice(&v.to_le_bytes());
            } else {
                let v = ntpoff()?;
                tls::gotie_to_le(&mut ctx.output_sections[out_sec_idx].data, off, v)
                    .map_err(|e| format!("{}: {e}", fail("initial-exec")))?;
            }
        }
        R_386_TLS_GOTDESC => {
            if dynamic {
                let gotoff = slot_addr(ctx)?.wrapping_sub(ctx.got_base) as i32;
                tls::gotdesc_to_ie(&mut ctx.output_sections[out_sec_idx].data, off, gotoff)
            } else {
                let v = ntpoff()?;
                tls::gotdesc_to_le(&mut ctx.output_sections[out_sec_idx].data, off, v)
            }
            .map_err(|e| format!("{}: {e}", fail("TLS-descriptor")))?;
        }
        R_386_TLS_DESC_CALL => {
            tls::desc_call_to_nop(data, off)
                .map_err(|e| format!("{}: {e}", fail("TLS-descriptor")))?;
        }
        _ => unreachable!("not a transitioned TLS relocation"),
    }
    Ok(())
}

/// Resolve a symbol's address, handling local, section, and global symbols.
fn resolve_sym_addr(obj_idx: usize, sym: &InputSymbol, ctx: &RelocContext) -> u32 {
    if sym.sym_type == STT_SECTION {
        if sym.section_index != SHN_UNDEF && sym.section_index != SHN_ABS {
            match ctx.section_map.get(&(obj_idx, sym.section_index as usize)) {
                Some(&(sec_out_idx, sec_out_offset)) => {
                    ctx.output_sections[sec_out_idx].addr + sec_out_offset
                }
                None => 0,
            }
        } else {
            0
        }
    } else if sym.name.is_empty() {
        0
    } else if sym.binding == STB_LOCAL {
        // Local symbols resolve per-object via section_map to avoid
        // collisions between identically-named locals (e.g. .LC0).
        resolve_via_section_map(obj_idx, sym, ctx)
    } else {
        match ctx.global_symbols.get(sym.name.as_str()) {
            Some(gs) => gs.address,
            None => resolve_via_section_map(obj_idx, sym, ctx),
        }
    }
}

/// Resolve a symbol address through the section map + symbol value.
fn resolve_via_section_map(obj_idx: usize, sym: &InputSymbol, ctx: &RelocContext) -> u32 {
    if sym.section_index != SHN_UNDEF && sym.section_index != SHN_ABS {
        match ctx.section_map.get(&(obj_idx, sym.section_index as usize)) {
            Some(&(sec_out_idx, sec_out_offset)) => {
                ctx.output_sections[sec_out_idx].addr + sec_out_offset + sym.value
            }
            None => sym.value,
        }
    } else if sym.section_index == SHN_ABS {
        sym.value
    } else {
        0
    }
}

/// Resolve R_386_GOT32 or R_386_GOT32X relocations.
pub(super) fn resolve_got_reloc(
    sym: &InputSymbol,
    sym_addr: u32,
    addend: i32,
    rel_type: u32,
    ctx: &RelocContext,
    relax_got32x: &mut bool,
) -> u32 {
    if let Some(gs) = ctx.global_symbols.get(sym.name.as_str()) {
        if gs.needs_got {
            // Every GOT-relative reference to a named symbol gave it a slot
            // of its own (`mark_plt_got_needs`); for a shared-library symbol
            // that slot is filled by GLOB_DAT -- never the lazy `.got.plt`
            // slot of its PLT entry, which holds `PLT+6` until the first
            // call, so an address loaded from it would change over time.
            let got_entry_addr =
                ctx.got_vaddr + (ctx.got_reserved as u32 + (gs.got_index - ctx.num_plt) as u32) * 4;
            (got_entry_addr as i32 + addend - ctx.got_base as i32) as u32
        } else if gs.is_dynamic {
            // Unreachable while the invariant above holds; the PLT's slot is
            // the only one such a symbol could have.
            let got_entry_addr = ctx.gotplt_vaddr + (ctx.gotplt_reserved + gs.plt_index as u32) * 4;
            (got_entry_addr as i32 + addend - ctx.got_base as i32) as u32
        } else if rel_type == R_386_GOT32X {
            *relax_got32x = true;
            (sym_addr as i32 + addend - ctx.got_base as i32) as u32
        } else {
            (sym_addr as i32 + addend - ctx.got_base as i32) as u32
        }
    } else if rel_type == R_386_GOT32X {
        *relax_got32x = true;
        (sym_addr as i32 + addend - ctx.got_base as i32) as u32
    } else {
        (sym_addr as i32 + addend - ctx.got_base as i32) as u32
    }
}
