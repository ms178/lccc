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

            // A section with no output-section mapping is skipped.  This was
            // written as `let _out_name = match .. { Some(n) => n, None =>
            // continue }`, which is a control-flow guard wearing a binding: the
            // binding was never read, so the only thing it expressed was the
            // `continue`.
            if output_section_name(&sec.name, sec.flags, sec.sh_type).is_none() {
                continue;
            }

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
    sec: &InputSection,
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

    let mut got32x_conv: Option<Got32xConversion> = None;

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
            let off = rel_offset as usize;
            let conversion = got32x_conversion(&sec.data, off);
            let baseless = got_field_baseless(&sec.data, off);
            let (value, conv) = resolve_got_reloc(
                obj, sym, sym_addr, addend, rel_type, conversion, baseless, patch_addr, ctx,
            )?;
            got32x_conv = conv;
            value
        }
        R_386_TLS_TPOFF | R_386_TLS_LE | R_386_TLS_LE_32 | R_386_TLS_TPOFF32
            if !sym.name.is_empty()
                && sym.binding != STB_LOCAL
                && ctx
                    .global_symbols
                    .get(sym.name.as_str())
                    .is_some_and(|gs| gs.is_dynamic) =>
        {
            // Local-Exec offsets exist only for the executable's own TLS
            // block (GNU ld: "unresolvable R_386_TLS_LE relocation").
            return Err(format!(
                "{}: unresolvable {} relocation against shared-library TLS symbol `{}': \
                 a Local-Exec access needs a definition in the executable \
                 (drop -ftls-model=local-exec)",
                obj.filename,
                if matches!(rel_type, R_386_TLS_LE | R_386_TLS_TPOFF) {
                    "R_386_TLS_LE"
                } else {
                    "R_386_TLS_LE_32"
                },
                sym.name
            ));
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
        // `got32x_conversion` decoded these bytes (see it for the forms);
        // lccc-ld executables are never PIE (options.rs), so the symbol's
        // address is a link-time constant.
        if let Some(conv) = got32x_conv {
            let reg = (out_sec.data[off - 1] >> 3) & 7;
            let (op, modrm) = match conv {
                Got32xConversion::MovImm => (0xc7, 0xc0 | reg),
                Got32xConversion::AluImm(alu) => (0x81, 0xc0 | (alu << 3) | reg),
                Got32xConversion::TestImm => (0xf7, 0xc0 | reg),
                // `68 imm32` behind a meaningless `%cs:` keeps the length.
                Got32xConversion::Push => (0x2e, 0x68),
                Got32xConversion::Call => (0x67, 0xe8),
                Got32xConversion::Jmp => {
                    out_sec.data[off - 2] = 0xe9;
                    out_sec.data[off - 1..off + 3].copy_from_slice(&value.to_le_bytes());
                    out_sec.data[off + 3] = 0x90;
                    return Ok(());
                }
            };
            out_sec.data[off - 2] = op;
            out_sec.data[off - 1] = modrm;
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

/// GNU ld's i386 GOT32X conversions for a symbol defined in a non-PIC
/// executable (`elf_i386_convert_load_reloc`), each the same length as the
/// GOT access it replaces and none of them loading from the GOT:
///   `movl x@GOT(%b), %r`   -> `movl $x, %r`      (C7 /0)
///   `OPl  x@GOT(%b), %r`   -> `OPl $x, %r`       (81 /op, the 8 ALU ops)
///   `testl x@GOT(%b), %r`  -> `testl $x, %r`     (F7 /0)
///   `pushl x@GOT(%b)`      -> `%cs: pushl $x`    (2E 68 imm32)
///   `call *x@GOT(%b)`      -> `addr32 call x`    (67 E8 rel32)
///   `jmp *x@GOT(%b)`       -> `jmp x; nop`       (E9 rel32 90)
/// with or without the base register (ModR/M mod=10 rm!=100, or the
/// base-less mod=00 rm=101), and only with a zero in-place addend: the
/// GOT32X addend offsets the GOT slot, not the symbol, so `x@GOT+4` has no
/// equivalent immediate (ld 2.47 refuses it too). Decoding backwards from
/// the field is unambiguous: in a SIB form the byte two before the field is
/// a ModR/M whose rm is 100, which no opcode below ends in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Got32xConversion {
    MovImm,
    AluImm(u8),
    TestImm,
    Push,
    Call,
    Jmp,
}

pub(super) fn got32x_conversion(data: &[u8], off: usize) -> Option<Got32xConversion> {
    let op = *data.get(off.checked_sub(2)?)?;
    let m = *data.get(off - 1)?;
    if !((m >> 6 == 2 && m & 7 != 4) || m & 0xc7 == 0x05) {
        return None;
    }
    if data.get(off..off + 4)? != [0; 4] {
        return None;
    }
    Some(match op {
        0x8b => Got32xConversion::MovImm,
        0x85 => Got32xConversion::TestImm,
        0x03 | 0x0b | 0x13 | 0x1b | 0x23 | 0x2b | 0x33 | 0x3b => Got32xConversion::AluImm(op >> 3),
        0xff if (m >> 3) & 7 == 2 => Got32xConversion::Call,
        0xff if (m >> 3) & 7 == 4 => Got32xConversion::Jmp,
        0xff if (m >> 3) & 7 == 6 => Got32xConversion::Push,
        _ => return None,
    })
}

/// A GOT reference without a base register (`movl x@GOT, %r`, GCC's
/// `-fno-pic -fno-plt` `call *f@GOT`): ModR/M mod=00 rm=101, GNU ld's test.
/// The i386 psABI value is then the slot's absolute address `G + A`, not the
/// GOT-relative `G + A - GOT` of the based form.
pub(super) fn got_field_baseless(data: &[u8], off: usize) -> bool {
    off >= 1 && data.get(off - 1).is_some_and(|&m| m & 0xc7 == 0x05)
}

/// Whether the target of a GOT reference may be addressed GOT-relatively
/// instead (the executable's own non-IFUNC definition; the executable is
/// never PIE, see options.rs, so even an SHN_ABS value is a link-time
/// constant). `gs` is the global entry of `sym`'s name, which for an
/// STB_LOCAL `sym` may belong to another object, so locals answer from
/// their own symbol.
pub(super) fn got_target_relaxable(sym: &InputSymbol, gs: Option<&LinkerSymbol>) -> bool {
    // ld.so may read `_DYNAMIC`'s link-time value from its GOT slot (ld
    // 2.47 never converts it either).
    if sym.sym_type == STT_GNU_IFUNC || sym.name == "_DYNAMIC" {
        return false;
    }
    if sym.binding == STB_LOCAL || sym.sym_type == STT_SECTION {
        return sym.section_index != SHN_UNDEF;
    }
    gs.is_some_and(|g| g.is_defined && !g.is_dynamic && g.sym_type != STT_GNU_IFUNC)
}

/// Resolve R_386_GOT32 or R_386_GOT32X: the GOT-relative offset of the
/// symbol's slot (the slot's absolute address for a base-less field), or --
/// for a relaxable `GOT32X` load site of a symbol defined here -- the
/// symbol's address, with `true` telling the caller to turn the load into
/// `movl $x, %r`. Slots are keyed by name in `global_symbols`,
/// where an STB_LOCAL symbol may find another object's entry (locals are
/// fallback definitions, see `symbols.rs`): a local owns the slot only when
/// the slot's link-time value is its own address.
pub(super) fn resolve_got_reloc(
    obj: &InputObject,
    sym: &InputSymbol,
    sym_addr: u32,
    addend: i32,
    rel_type: u32,
    conversion: Option<Got32xConversion>,
    baseless: bool,
    patch_addr: u32,
    ctx: &RelocContext,
) -> Result<(u32, Option<Got32xConversion>), String> {
    let gs = (!sym.name.is_empty())
        .then(|| ctx.global_symbols.get(sym.name.as_str()))
        .flatten();
    if let Some(conv) = conversion.filter(|_| rel_type == R_386_GOT32X) {
        if got_target_relaxable(sym, gs) {
            // The caller rewrites the instruction (see `Got32xConversion`):
            // an immediate takes the absolute value, a direct branch the
            // displacement from its own end -- the field stays at the
            // relocation offset for `addr32 call`, moves one byte back for
            // `jmp`.
            let s_a = sym_addr.wrapping_add(addend as u32);
            let value = match conv {
                Got32xConversion::Call => s_a.wrapping_sub(patch_addr.wrapping_add(4)),
                Got32xConversion::Jmp => s_a.wrapping_sub(patch_addr.wrapping_add(3)),
                _ => s_a,
            };
            return Ok((value, Some(conv)));
        }
    }
    if let Some(gs) = gs.filter(|gs| gs.needs_got) {
        let owned = sym.binding != STB_LOCAL || (!gs.is_dynamic && gs.address == sym_addr);
        if owned {
            // Every GOT-relative reference to a named symbol gave it a slot
            // of its own (`mark_plt_got_needs`); for a shared-library symbol
            // that slot is filled by GLOB_DAT -- never the lazy `.got.plt`
            // slot of its PLT entry, which holds `PLT+6` until the first
            // call, so an address loaded from it would change over time.
            let got_entry_addr =
                ctx.got_vaddr + (ctx.got_reserved as u32 + (gs.got_index - ctx.num_plt) as u32) * 4;
            let base = if baseless { 0 } else { ctx.got_base as i32 };
            return Ok(((got_entry_addr as i32 + addend - base) as u32, None));
        }
    }
    // Resolving to the symbol's GOT-relative offset without converting the
    // instruction would load from the symbol instead of its slot.
    Err(format!(
        "{}: {} against local symbol `{}' needs a GOT slot of its own, but \
         lccc-ld keys GOT slots by name and that name is bound to another \
         definition (address the local symbol with @GOTOFF)",
        obj.filename,
        if rel_type == R_386_GOT32X {
            "R_386_GOT32X"
        } else {
            "R_386_GOT32"
        },
        if sym.name.is_empty() {
            "<section>"
        } else {
            sym.name.as_str()
        }
    ))
}

#[cfg(test)]
mod got32x_tests {
    use super::{Got32xConversion as C, got_field_baseless, got32x_conversion};

    /// `bytes` followed by a zero disp32; the field starts after `bytes`.
    fn site(bytes: &[u8]) -> (Vec<u8>, usize) {
        let mut v = bytes.to_vec();
        let off = v.len();
        v.extend_from_slice(&[0; 4]);
        (v, off)
    }

    #[test]
    fn converts_exactly_ld_2_47_forms() {
        let cases: &[(&[u8], Option<C>)] = &[
            (&[0x8b, 0x83], Some(C::MovImm)),    // movl x@GOT(%ebx),%eax
            (&[0x8b, 0x05], Some(C::MovImm)),    // movl x@GOT,%eax
            (&[0x8b, 0x0d], Some(C::MovImm)),    // movl x@GOT,%ecx
            (&[0x85, 0x83], Some(C::TestImm)),   // testl %eax,x@GOT(%ebx)
            (&[0x03, 0x83], Some(C::AluImm(0))), // addl
            (&[0x0b, 0x93], Some(C::AluImm(1))), // orl ...,%edx
            (&[0x13, 0x83], Some(C::AluImm(2))), // adcl
            (&[0x3b, 0x83], Some(C::AluImm(7))), // cmpl
            (&[0xff, 0x93], Some(C::Call)),      // call *x@GOT(%ebx)
            (&[0xff, 0x15], Some(C::Call)),      // call *x@GOT
            (&[0xff, 0xa3], Some(C::Jmp)),       // jmp *x@GOT(%ebx)
            (&[0xff, 0x25], Some(C::Jmp)),       // jmp *x@GOT
            (&[0xff, 0xb3], Some(C::Push)),      // pushl x@GOT(%ebx)
            (&[0xff, 0x35], Some(C::Push)),      // pushl x@GOT
            (&[0x89, 0x83], None),               // store
            (&[0x01, 0x83], None),               // addl %eax,x@GOT(%ebx)
            (&[0x8d, 0x83], None),               // lea
            (&[0xff, 0x83], None),               // incl (FF /0)
            (&[0xff, 0x8b], None),               // decl (FF /1)
            (&[0x8b, 0xc3], None),               // register form
            (&[0x8b, 0x43], None),               // disp8 cannot hold it
            (&[0x8b, 0x84, 0xb3], None),         // SIB: off-2 is ModR/M
            (&[0x0f, 0xb6, 0x83], None),         // movzbl (0F map)
        ];
        for (bytes, want) in cases {
            let (v, off) = site(bytes);
            assert_eq!(got32x_conversion(&v, off), *want, "{bytes:02x?}");
        }
    }

    #[test]
    fn nonzero_addend_and_truncated_sites_are_not_converted() {
        // The GOT32X addend offsets the slot, not the symbol.
        let v = [0x8b, 0x83, 4, 0, 0, 0];
        assert_eq!(got32x_conversion(&v, 2), None);
        assert_eq!(got32x_conversion(&[0x8b, 0x83, 0, 0], 2), None);
        assert_eq!(got32x_conversion(&[0x83, 0, 0, 0, 0], 1), None);
    }

    #[test]
    fn baseless_is_modrm_mod00_rm101() {
        assert!(got_field_baseless(&[0xff, 0x15, 0, 0, 0, 0], 2));
        assert!(got_field_baseless(&[0x8b, 0x0d, 0, 0, 0, 0], 2));
        assert!(!got_field_baseless(&[0x8b, 0x83, 0, 0, 0, 0], 2));
        assert!(!got_field_baseless(&[0, 0, 0, 0], 0));
    }
}
