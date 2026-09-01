//! i386 relocation application for the i686 linker.
//!
//! Applies relocations from input objects to merged output sections.
//! All i386 relocation types are handled here, separated from the main
//! linking logic to keep the code manageable.
//!
//! The relocation types supported include absolute (R_386_32), PC-relative
//! (R_386_PC32, R_386_PLT32), GOT-related (R_386_GOT32, R_386_GOT32X,
//! R_386_GOTPC, R_386_GOTOFF), and TLS relocations.

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
    #[allow(dead_code)] // Set by linker layout; available for future PLT-relative relocations
    pub plt_vaddr: u32,
    #[allow(dead_code)] // Set by linker layout; available for future PLT-relative relocations
    pub plt_header_size: u32,
    #[allow(dead_code)] // Set by linker layout; available for future PLT-relative relocations
    pub plt_entry_size: u32,
    pub num_plt: usize,
    pub tls_addr: u32,
    pub tls_mem_size: u32,
    pub has_tls: bool,
    /// Absolute addresses of PLT32 relocation slots suppressed by a
    /// preceding TLS GD→LE relaxation (the call they patched is now a NOP).
    pub tls_relaxed_call_slots: crate::common::fx_hash::FxHashSet<u32>,
}

/// Apply all relocations from input objects to the output sections.
/// Returns a list of text relocations (address, dynsym_index) for symbols using textrel.
pub(super) fn apply_relocations(
    inputs: &[InputObject],
    ctx: &mut RelocContext,
) -> Result<Vec<(u32, String)>, String> {
    let mut text_relocs: Vec<(u32, String)> = Vec::new();
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

            for &(rel_offset, rel_type, sym_idx, addend) in &sec.relocations {
                let tr = apply_one_reloc(
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
                if let Some(t) = tr {
                    text_relocs.push(t);
                }
            }
        }
    }
    Ok(text_relocs)
}

/// Apply a single relocation.
/// Returns Some((patch_addr, sym_name)) if a text relocation entry is needed.
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
) -> Result<Option<(u32, String)>, String> {
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
    let mut text_reloc: Option<(u32, String)> = None;

    let value: u32 = match rel_type {
        R_386_NONE => return Ok(None),
        R_386_32 => {
            // Check if this symbol uses text relocations (WEAK dynamic data)
            if !sym.name.is_empty() {
                if let Some(gs) = ctx.global_symbols.get(sym.name.as_str()) {
                    if gs.uses_textrel {
                        // Record a text relocation; write 0 for now (dynamic linker fills it)
                        text_reloc = Some((patch_addr, sym.name.clone()));
                        addend as u32
                    } else {
                        (sym_addr as i32 + addend) as u32
                    }
                } else {
                    (sym_addr as i32 + addend) as u32
                }
            } else {
                (sym_addr as i32 + addend) as u32
            }
        }
        R_386_PC32 | R_386_PLT32 => {
            if ctx.tls_relaxed_call_slots.contains(&patch_addr) {
                // The GD→LE relaxation NOPed out this `call ___tls_get_addr`;
                // leave the NOP encoding intact.
                return Ok(None);
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
        R_386_TLS_IE => resolve_tls_ie(sym, sym_addr, addend, ctx),
        R_386_TLS_GOTIE => resolve_tls_gotie(sym, sym_addr, addend, ctx),
        R_386_TLS_GD => {
            if ctx.has_tls && sym.sym_type == STT_TLS {
                let tpoff = sym_addr as i32 - ctx.tls_addr as i32 - ctx.tls_mem_size as i32
                    + addend;
                // GD→LE relaxation for executables (matches GNU ld's
                // elf32-i386 tls transform). The GD sequence is:
                //   lea  sym@tlsgd(%x), %y   ; 8d /r disp32   (6 bytes)
                //   call ___tls_get_addr@PLT ; e8 disp32      (5 bytes)
                // Rewrite it into a local-exec access:
                //   movl $sym@tpoff, %y      ; c7 /0 modrm imm32 (6 bytes)
                //   <5-byte NOP>             ; 0f 1f 44 00 00    (5 bytes)
                // and suppress the call's own PLT32 relocation.
                let out_sec = &mut ctx.output_sections[out_sec_idx];
                let off = patch_offset as usize;
                // Detect the lea that hosts @tlsgd. GNU as emits either the
                // 7-byte SIB form `8d 04 1d disp32` (lea (%ebx,%ebx),reg —
                // the -fpic default via get_pc_thunk) or the 6-byte form
                // `8d modrm disp32` (lea disp32(%ebx),reg, mod=10).
                let sib_form = off >= 3
                    && off + 11 <= out_sec.data.len()
                    && out_sec.data[off - 3] == 0x8d
                    // modrm: mod=00 (disp32), reg=dest, rm=100 (SIB follows)
                    && (out_sec.data[off - 2] & 0xC7) == 0x04;
                let plain_form = off >= 2
                    && off + 10 <= out_sec.data.len()
                    && out_sec.data[off - 2] == 0x8d
                    && (out_sec.data[off - 1] & 0xC0) == 0x80;
                let (lea_start, dest) = if sib_form {
                    (off - 3, ((out_sec.data[off - 2] >> 3) & 7) as usize)
                } else if plain_form {
                    (off - 2, ((out_sec.data[off - 1] >> 3) & 7) as usize)
                } else {
                    (0, 0)
                };
                if lea_start != 0 && dest == 0 {
                    // dest == %eax (libbid's get_pc_thunk.ax pattern):
                    //   movl %gs:0, %eax        ; 65 a1 00000000  (6 bytes)
                    //   addl $tpoff, %eax       ; 05 imm32        (5 bytes)
                    // total 11 bytes; the 12-byte SIB form gets a 1-byte NOP.
                    // (TPOFF here is `sym - tp`, so the address is tp + tpoff.)
                    // Locate the `call ___tls_get_addr` BEFORE overwriting:
                    // it is the byte right after the lea (off+4 in both
                    // encodings — the lea is either 6 or 7 bytes ending at
                    // off+4).
                    let call_at = if out_sec.data.get(off + 4) == Some(&0xe8) {
                        Some(off + 4)
                    } else {
                        None
                    };
                    out_sec.data[lea_start..lea_start + 6]
                        .copy_from_slice(&[0x65, 0xa1, 0x00, 0x00, 0x00, 0x00]);
                    // addl $tpoff, %eax: opcode 05 at +6, imm32 at +7..+11.
                    out_sec.data[lea_start + 6] = 0x05;
                    out_sec.data[lea_start + 7..lea_start + 11]
                        .copy_from_slice(&(tpoff as u32).to_le_bytes());
                    if sib_form {
                        out_sec.data[lea_start + 11] = 0x90; // 1-byte nop fill
                    }
                    if let Some(call_at) = call_at {
                        // Sequence above already covers the original call bytes; no NOP needed.

                        // Suppress the call's PLT32 relocation (disp field).
                        ctx.tls_relaxed_call_slots.insert(patch_addr + (call_at - off) as u32 + 1);
                        // The relaxed bytes at the reloc offset are the zero
                        // displacement of ; patching the
                        // returned value there would corrupt the sequence.
                        return Ok(None);
                    }
                }
                tpoff as u32
            } else {
                addend as u32
            }
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

    Ok(text_reloc)
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
        if gs.is_dynamic {
            let got_entry_addr = if gs.needs_plt {
                ctx.gotplt_vaddr + (ctx.gotplt_reserved + gs.plt_index as u32) * 4
            } else {
                ctx.got_vaddr + (ctx.got_reserved as u32 + (gs.got_index - ctx.num_plt) as u32) * 4
            };
            (got_entry_addr as i32 + addend - ctx.got_base as i32) as u32
        } else if gs.needs_got {
            let got_entry_addr =
                ctx.got_vaddr + (ctx.got_reserved as u32 + (gs.got_index - ctx.num_plt) as u32) * 4;
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

/// Resolve R_386_TLS_IE relocation.
pub(super) fn resolve_tls_ie(
    sym: &InputSymbol,
    sym_addr: u32,
    addend: i32,
    ctx: &RelocContext,
) -> u32 {
    if let Some(gs) = ctx.global_symbols.get(sym.name.as_str()) {
        if gs.needs_got {
            let got_entry_addr =
                ctx.got_vaddr + (ctx.got_reserved as u32 + (gs.got_index - ctx.num_plt) as u32) * 4;
            (got_entry_addr as i32 + addend) as u32
        } else {
            let tpoff = sym_addr as i32 - ctx.tls_addr as i32 - ctx.tls_mem_size as i32;
            (tpoff + addend) as u32
        }
    } else {
        addend as u32
    }
}

/// Resolve R_386_TLS_GOTIE relocation.
pub(super) fn resolve_tls_gotie(
    sym: &InputSymbol,
    sym_addr: u32,
    addend: i32,
    ctx: &RelocContext,
) -> u32 {
    if let Some(gs) = ctx.global_symbols.get(sym.name.as_str()) {
        if gs.needs_got {
            let got_entry_addr =
                ctx.got_vaddr + (ctx.got_reserved as u32 + (gs.got_index - ctx.num_plt) as u32) * 4;
            (got_entry_addr as i32 + addend - ctx.got_base as i32) as u32
        } else {
            let tpoff = sym_addr as i32 - ctx.tls_addr as i32 - ctx.tls_mem_size as i32;
            (tpoff + addend) as u32
        }
    } else {
        addend as u32
    }
}
