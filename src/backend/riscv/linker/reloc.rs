//! Phase 6: Relocation application.
//!
//! Applies RISC-V relocations to merged section data. Handles all relocation
//! types implemented from the RISC-V ELF psABI, including PC-relative, GOT, TLS, and
//! compressed instruction relocations.
//!
//! Shared between executable and shared library linking, with context-specific
//! behavior controlled by `RelocContext`.

use super::elf_read::*;
use super::got::{GotKind, GotLayout};
use super::relocations::{
    GlobalSym, MergedSection, R_RISCV_32, R_RISCV_32_PCREL, R_RISCV_64, R_RISCV_ADD8,
    R_RISCV_ADD16, R_RISCV_ADD32, R_RISCV_ADD64, R_RISCV_ALIGN, R_RISCV_BRANCH, R_RISCV_CALL_PLT,
    R_RISCV_GOT_HI20, R_RISCV_HI20, R_RISCV_JAL, R_RISCV_LO12_I, R_RISCV_LO12_S,
    R_RISCV_PCREL_HI20, R_RISCV_PCREL_LO12_I, R_RISCV_PCREL_LO12_S, R_RISCV_RELAX,
    R_RISCV_RVC_BRANCH, R_RISCV_RVC_JUMP, R_RISCV_SET_ULEB128, R_RISCV_SET6, R_RISCV_SET8,
    R_RISCV_SET16, R_RISCV_SET32, R_RISCV_SUB_ULEB128, R_RISCV_SUB6, R_RISCV_SUB8, R_RISCV_SUB16,
    R_RISCV_SUB32, R_RISCV_SUB64, R_RISCV_TLS_GD_HI20, R_RISCV_TLS_GOT_HI20, R_RISCV_TPREL_ADD,
    R_RISCV_TPREL_HI20, R_RISCV_TPREL_LO12_I, R_RISCV_TPREL_LO12_S, got_sym_key, patch_b_type,
    patch_cb_type, patch_cj_type, patch_i_type, patch_j_type, patch_s_type, patch_u_type,
    resolve_symbol_value,
};
use crate::common::fx_hash::{FxHashMap, FxHashSet};

/// Context for relocation application, providing all the resolved addresses
/// and symbol tables needed to patch instructions.
pub struct RelocContext<'a> {
    pub sec_mapping: &'a FxHashMap<(usize, usize), (usize, u64)>,
    pub section_vaddrs: &'a [u64],
    pub local_sym_vaddrs: &'a [Vec<u64>],
    pub global_syms: &'a FxHashMap<String, GlobalSym>,
    pub got_vaddr: u64,
    pub got: &'a GotLayout,
    pub got_plt_vaddr: u64,
    pub tls_vaddr: u64,
    /// For executable linking: GD->LE TLS relaxation info (auipc_vaddr -> (sym_value, addend))
    pub gd_tls_relax_info: &'a FxHashMap<u64, (u64, i64)>,
    /// For executable linking: vaddrs of __tls_get_addr calls to NOP out
    pub gd_tls_call_nop: &'a FxHashSet<u64>,
    /// Whether to collect R_RISCV_RELATIVE dynamic relocations (shared lib mode)
    pub collect_relatives: bool,
    /// For shared library linking: PLT stub addresses for undefined symbols
    pub plt_sym_addrs: &'a FxHashMap<String, u64>,
}

/// Result of applying relocations to one object.
pub struct RelocResult {
    /// Dynamic RELATIVE relocations (offset, addend) collected during shared lib linking
    pub relative_entries: Vec<(u64, u64)>,
    /// Symbol-based R_RISCV_64 dynamic relocations: place, name, signed addend.
    /// A default-visible global can be preempted even when defined in this DSO.
    pub symbol_entries: Vec<(u64, String, i64)>,
}

/// Minimum byte extent touched by each implemented relocation. Validate against
/// the INPUT section, not the merged output: a truncated input must not patch
/// a following object's bytes. Unknown types are rejected at application time,
/// so relocations in discarded/non-alloc sections do not require implementation.
fn relocation_extent(kind: u32) -> Option<u64> {
    match kind {
        0 | R_RISCV_RELAX | R_RISCV_ALIGN | R_RISCV_TPREL_ADD => Some(0),
        R_RISCV_ADD8 | R_RISCV_SUB8 | R_RISCV_SET6 | R_RISCV_SUB6 | R_RISCV_SET8
        | R_RISCV_SET_ULEB128 | R_RISCV_SUB_ULEB128 => Some(1),
        R_RISCV_ADD16 | R_RISCV_SUB16 | R_RISCV_SET16 | R_RISCV_RVC_BRANCH | R_RISCV_RVC_JUMP => {
            Some(2)
        }
        R_RISCV_64 | R_RISCV_ADD64 | R_RISCV_SUB64 | R_RISCV_CALL_PLT => Some(8),
        R_RISCV_32 | R_RISCV_32_PCREL | R_RISCV_ADD32 | R_RISCV_SUB32 | R_RISCV_SET32
        | R_RISCV_PCREL_HI20 | R_RISCV_PCREL_LO12_I | R_RISCV_PCREL_LO12_S | R_RISCV_GOT_HI20
        | R_RISCV_BRANCH | R_RISCV_JAL | R_RISCV_HI20 | R_RISCV_LO12_I | R_RISCV_LO12_S
        | R_RISCV_TPREL_HI20 | R_RISCV_TPREL_LO12_I | R_RISCV_TPREL_LO12_S
        | R_RISCV_TLS_GD_HI20 | R_RISCV_TLS_GOT_HI20 => Some(4),
        _ => None,
    }
}

/// Validate before GOT/TLS scans index symbol tables or any relocation mutates
/// merged data. All loaded archive members go through the same entry point.
/// This is structural validation, not permission to silently consume a type.
pub(super) fn validate_input_relocations(input_objs: &[(String, ElfObject)]) -> Result<(), String> {
    for (name, obj) in input_objs {
        for (sec_idx, relocs) in obj.relocations.iter().enumerate() {
            for (ri, reloc) in relocs.iter().enumerate() {
                if reloc.rela_type == 0 {
                    // R_RISCV_NONE has no operation, symbol or storage field.
                    continue;
                }
                if reloc.sym_idx as usize >= obj.symbols.len() {
                    return Err(format!(
                        "invalid RISC-V relocation symbol index {} in '{}' section {}",
                        reloc.sym_idx, name, sec_idx
                    ));
                }
                let size = obj.sections.get(sec_idx).map(|s| s.size).ok_or_else(|| {
                    format!(
                        "invalid RISC-V relocation section {} in '{}'",
                        sec_idx, name
                    )
                })?;
                let width = relocation_extent(reloc.rela_type).unwrap_or(0);
                if reloc.offset > size || width > size - reloc.offset {
                    return Err(format!(
                        "RISC-V relocation type {} at {:#x} exceeds input section {} (size {:#x}) in '{}'",
                        reloc.rela_type, reloc.offset, sec_idx, size, name
                    ));
                }
                if reloc.rela_type == R_RISCV_SET_ULEB128 {
                    let paired = relocs.get(ri + 1).is_some_and(|sub| {
                        sub.rela_type == R_RISCV_SUB_ULEB128 && sub.offset == reloc.offset
                    });
                    if !paired {
                        return Err(format!(
                            "unpaired RISC-V SET_ULEB128 in '{}' section {}",
                            name, sec_idx
                        ));
                    }
                    let off = usize::try_from(reloc.offset).map_err(|_| {
                        format!("RISC-V ULEB128 offset exceeds host size in '{}'", name)
                    })?;
                    let field =
                        obj.section_data[sec_idx]
                            .as_slice()
                            .get(off..)
                            .ok_or_else(|| {
                                format!("missing RISC-V ULEB128 input field in '{}'", name)
                            })?;
                    super::uleb128::field_len(field).map_err(|error| {
                        format!(
                            "{} in '{}' section {} at {:#x}",
                            error, name, sec_idx, reloc.offset
                        )
                    })?;
                } else if reloc.rela_type == R_RISCV_SUB_ULEB128 {
                    let paired = ri
                        .checked_sub(1)
                        .and_then(|i| relocs.get(i))
                        .is_some_and(|set| {
                            set.rela_type == R_RISCV_SET_ULEB128 && set.offset == reloc.offset
                        });
                    if !paired {
                        return Err(format!(
                            "unpaired RISC-V SUB_ULEB128 in '{}' section {}",
                            name, sec_idx
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

/// Apply all relocations for all input objects to the merged section data.
pub fn apply_relocations(
    input_objs: &[(String, super::elf_read::ElfObject)],
    merged_sections: &mut [MergedSection],
    ctx: &RelocContext,
) -> Result<RelocResult, String> {
    let mut result = RelocResult {
        relative_entries: Vec::new(),
        symbol_entries: Vec::new(),
    };

    for (obj_idx, (obj_name, obj)) in input_objs.iter().enumerate() {
        for (sec_idx, relocs) in obj.relocations.iter().enumerate() {
            if relocs.is_empty() {
                continue;
            }
            let (merged_idx, sec_offset) = match ctx.sec_mapping.get(&(obj_idx, sec_idx)) {
                Some(&v) => v,
                None => continue,
            };

            let ms_vaddr = ctx.section_vaddrs[merged_idx];

            let mut paired_sub = None;
            for (ri, reloc) in relocs.iter().enumerate() {
                if paired_sub == Some(ri) || reloc.rela_type == 0 {
                    continue; // R_RISCV_NONE, even without a symbol table.
                }
                let offset = sec_offset.checked_add(reloc.offset).ok_or_else(|| {
                    format!("RISC-V relocation offset overflow in '{}'", obj_name)
                })?;
                let p = ms_vaddr.checked_add(offset).ok_or_else(|| {
                    format!("RISC-V relocation address overflow in '{}'", obj_name)
                })?;

                let sym_idx = reloc.sym_idx as usize;
                let sym = obj.symbols.get(sym_idx).ok_or_else(|| {
                    format!(
                        "invalid RISC-V relocation symbol index {} in '{}'",
                        sym_idx, obj_name
                    )
                })?;

                // Resolve symbol value
                let s = resolve_sym_value(sym, sym_idx, obj_idx, obj, ctx);

                let a = reloc.addend;
                let data = &mut merged_sections[merged_idx].data;
                let off = usize::try_from(offset).map_err(|_| {
                    format!(
                        "RISC-V relocation offset does not fit host size in '{}'",
                        obj_name
                    )
                })?;

                if reloc.rela_type == R_RISCV_SET_ULEB128 {
                    let sub = relocs
                        .get(ri + 1)
                        .filter(|sub| {
                            sub.rela_type == R_RISCV_SUB_ULEB128 && sub.offset == reloc.offset
                        })
                        .ok_or_else(|| format!("unpaired RISC-V SET_ULEB128 in '{}'", obj_name))?;
                    let sub_sym_idx = sub.sym_idx as usize;
                    let sub_sym = obj.symbols.get(sub_sym_idx).ok_or_else(|| {
                        format!(
                            "invalid RISC-V relocation symbol index {} in '{}'",
                            sub_sym_idx, obj_name
                        )
                    })?;
                    let sub_value = resolve_sym_value(sub_sym, sub_sym_idx, obj_idx, obj, ctx);
                    let original_off = usize::try_from(reloc.offset).map_err(|_| {
                        format!("RISC-V ULEB128 offset exceeds host size in '{}'", obj_name)
                    })?;
                    let original = obj.section_data[sec_idx]
                        .as_slice()
                        .get(original_off..)
                        .ok_or_else(|| {
                            format!("missing RISC-V ULEB128 input field in '{}'", obj_name)
                        })?;
                    let reserved = super::uleb128::field_len(original)?;
                    let end = off.checked_add(reserved).ok_or_else(|| {
                        format!("RISC-V ULEB128 input extent overflow in '{}'", obj_name)
                    })?;
                    let field = data.get_mut(off..end).ok_or_else(|| {
                        format!(
                            "RISC-V ULEB128 input contribution exceeds output in '{}'",
                            obj_name
                        )
                    })?;
                    super::uleb128::apply_pair(field, s, a, sub_value, sub.addend).map_err(
                        |error| {
                            format!(
                                "{} in '{}' section {} at {:#x}",
                                error, obj_name, sec_idx, reloc.offset
                            )
                        },
                    )?;
                    paired_sub = Some(ri + 1);
                    continue;
                }

                apply_one_reloc(
                    reloc.rela_type,
                    data,
                    off,
                    s,
                    a,
                    p,
                    sym,
                    sym_idx,
                    obj_idx,
                    obj,
                    sec_idx,
                    sec_offset,
                    obj_name,
                    ctx,
                    &mut result,
                )?;
            }
        }
    }

    Ok(result)
}

/// Resolve the value of a symbol for relocation purposes.
fn resolve_sym_value(
    sym: &super::elf_read::Symbol,
    sym_idx: usize,
    obj_idx: usize,
    obj: &super::elf_read::ElfObject,
    ctx: &RelocContext,
) -> u64 {
    if sym.sym_type() == STT_SECTION {
        if (sym.shndx as usize) < obj.sections.len() {
            if let Some(&(mi, mo)) = ctx.sec_mapping.get(&(obj_idx, sym.shndx as usize)) {
                return ctx.section_vaddrs[mi] + mo;
            }
        }
        0
    } else if !sym.name.is_empty() && sym.binding() != STB_LOCAL {
        if let Some(gs) = ctx.global_syms.get(sym.name.as_str()) {
            if gs.needs_plt || gs.defined {
                gs.value
            } else {
                0
            }
        } else {
            0
        }
    } else {
        ctx.local_sym_vaddrs
            .get(obj_idx)
            .and_then(|v| v.get(sym_idx))
            .copied()
            .unwrap_or(0)
    }
}

/// The address of the GOT slot of `kind` a relocation against `sym`
/// addresses. An executable reaches an imported function's address through
/// its `.got.plt` slot when it has no `.got` slot of its own.
fn lookup_got_entry(
    sym: &super::elf_read::Symbol,
    obj_idx: usize,
    addend: i64,
    kind: GotKind,
    ctx: &RelocContext,
) -> Result<u64, String> {
    let (key, _) = got_sym_key(obj_idx, sym, addend);
    if let Some(off) = ctx.got.offset(&key, kind) {
        return Ok(ctx.got_vaddr + off);
    }
    if kind == GotKind::Addr && !ctx.collect_relatives {
        if let Some(gs) = ctx
            .global_syms
            .get(sym.name.as_str())
            .filter(|g| g.needs_plt)
        {
            return Ok(ctx.got_plt_vaddr + (2 + gs.plt_idx) as u64 * 8);
        }
    }
    Err(format!("no RISC-V {kind:?} GOT slot for '{}'", sym.name))
}

/// Resolve a 64-bit address and record the fixup the loader still owes it.
/// Executable linking remains an allocation/hash-lookup-free arithmetic path.
fn relocate_address64(
    symbol_value: u64,
    addend: i64,
    place: u64,
    sym: &Symbol,
    sym_idx: usize,
    ctx: &RelocContext,
    result: &mut RelocResult,
) -> u64 {
    // ELF address arithmetic is modulo 2^64, including negative RELA addends.
    let value = symbol_value.wrapping_add(addend as u64);
    if !ctx.collect_relatives {
        return value;
    }
    match shared_address_kind(sym, sym_idx, ctx) {
        SharedAddressKind::Symbol => {
            result
                .symbol_entries
                .push((place, sym.name.to_string(), addend));
            return 0;
        }
        SharedAddressKind::Relative => result.relative_entries.push((place, value)),
        SharedAddressKind::Absolute => {}
    }
    value
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SharedAddressKind {
    Absolute,
    Relative,
    Symbol,
}

/// Classify after cross-object binding/visibility resolution. ABS definitions
/// are still preemptible when default-visible; hidden undefined weak is zero.
fn shared_address_kind(sym: &Symbol, sym_idx: usize, ctx: &RelocContext) -> SharedAddressKind {
    let global = if sym.binding() != STB_LOCAL && !sym.name.is_empty() {
        ctx.global_syms.get(sym.name.as_str())
    } else {
        None
    };
    if global.is_some_and(|g| g.visibility == STV_DEFAULT) {
        return SharedAddressKind::Symbol;
    }
    let absolute = global.map_or(sym.shndx == SHN_ABS || sym_idx == 0, |g| {
        (g.defined && g.section_idx.is_none())
            || (!g.defined && g.binding == STB_WEAK && g.visibility != STV_DEFAULT)
    });
    if absolute {
        SharedAddressKind::Absolute
    } else {
        SharedAddressKind::Relative
    }
}

/// Classify a 32-bit address independently of its output storage extent.
fn relocate_address32(
    s: u64,
    a: i64,
    sym: &super::elf_read::Symbol,
    sym_idx: usize,
    obj_name: &str,
    ctx: &RelocContext,
) -> Result<u32, String> {
    // RV64 shared addresses cannot be frozen to link-time S+A. A
    // RELATIVE fixup would also overwrite eight bytes, not this word.
    // Match LLD's rejection unless resolution is genuinely absolute;
    // default-visible ABS globals are not exempt from preemption.
    if ctx.collect_relatives
        && shared_address_kind(sym, sym_idx, ctx) != SharedAddressKind::Absolute
    {
        return Err(format!(
            "R_RISCV_32 requires a non-preemptible absolute symbol in shared output: '{}' in '{}'",
            sym.name, obj_name
        ));
    }
    Ok(s.wrapping_add(a as u64) as u32)
}

/// Apply a single relocation to the merged section data.
fn apply_one_reloc(
    rela_type: u32,
    data: &mut [u8],
    off: usize,
    s: u64,
    a: i64,
    p: u64,
    sym: &super::elf_read::Symbol,
    sym_idx: usize,
    obj_idx: usize,
    obj: &super::elf_read::ElfObject,
    sec_idx: usize,
    sec_offset: u64,
    obj_name: &str,
    ctx: &RelocContext,
    result: &mut RelocResult,
) -> Result<(), String> {
    match rela_type {
        R_RISCV_RELAX | R_RISCV_ALIGN => { /* hints, skip */ }

        R_RISCV_64 => {
            let field = off
                .checked_add(8)
                .and_then(|end| data.get_mut(off..end))
                .ok_or_else(|| format!("R_RISCV_64 field exceeds output in '{}'", obj_name))?;
            let value = relocate_address64(s, a, p, sym, sym_idx, ctx, result);
            field.copy_from_slice(&value.to_le_bytes());
        }
        R_RISCV_32 => {
            let value = relocate_address32(s, a, sym, sym_idx, obj_name, ctx)?;
            let field = off
                .checked_add(4)
                .and_then(|end| data.get_mut(off..end))
                .ok_or_else(|| format!("R_RISCV_32 field exceeds output in '{}'", obj_name))?;
            field.copy_from_slice(&value.to_le_bytes());
        }
        R_RISCV_PCREL_HI20 => {
            let offset_val = super::displacement::checked_auipc_delta(s, a, p)?;
            patch_u_type(data, off, offset_val as u32);
        }
        R_RISCV_PCREL_LO12_I => {
            let auipc_addr = s as i64 + a;
            if let Some(&(sym_val, gd_addend)) = ctx.gd_tls_relax_info.get(&(auipc_addr as u64)) {
                let tprel = (sym_val as i64 + gd_addend - ctx.tls_vaddr as i64) as u32;
                patch_i_type(data, off, tprel & 0xFFF);
            } else {
                let hi_val = find_hi20_value_for_reloc(
                    obj,
                    obj_idx,
                    sec_idx,
                    ctx,
                    auipc_addr as u64,
                    sec_offset,
                );
                patch_i_type(data, off, hi_val as u32);
            }
        }
        R_RISCV_PCREL_LO12_S => {
            let auipc_addr = s as i64 + a;
            if let Some(&(sym_val, gd_addend)) = ctx.gd_tls_relax_info.get(&(auipc_addr as u64)) {
                let tprel = (sym_val as i64 + gd_addend - ctx.tls_vaddr as i64) as u32;
                patch_s_type(data, off, tprel & 0xFFF);
            } else {
                let hi_val = find_hi20_value_for_reloc(
                    obj,
                    obj_idx,
                    sec_idx,
                    ctx,
                    auipc_addr as u64,
                    sec_offset,
                );
                patch_s_type(data, off, hi_val as u32);
            }
        }
        R_RISCV_GOT_HI20 => {
            let got_entry_vaddr = lookup_got_entry(sym, obj_idx, a, GotKind::Addr, ctx)?;
            let offset_val = super::displacement::checked_auipc_delta(got_entry_vaddr, a, p)?;
            patch_u_type(data, off, offset_val as u32);
        }
        R_RISCV_CALL_PLT => {
            if ctx.gd_tls_call_nop.contains(&p) {
                // GD->LE relaxation: replace call with add a0, a0, tp + nop
                if off + 8 <= data.len() {
                    let add_insn: u32 = 0x00450533; // add a0, a0, tp
                    data[off..off + 4].copy_from_slice(&add_insn.to_le_bytes());
                    let nop: u32 = 0x00000013;
                    data[off + 4..off + 8].copy_from_slice(&nop.to_le_bytes());
                }
            } else {
                let target = if !sym.name.is_empty() && sym.binding() != STB_LOCAL {
                    // For undefined symbols in shared libs, redirect through PLT stub
                    if let Some(&plt_addr) = ctx.plt_sym_addrs.get(sym.name.as_str()) {
                        plt_addr as i64
                    } else {
                        ctx.global_syms
                            .get(sym.name.as_str())
                            .map(|gs| gs.value as i64)
                            .unwrap_or(s as i64)
                    }
                } else {
                    s as i64
                };
                let offset_val = super::displacement::checked_auipc_delta(target as u64, a, p)?;
                let hi = ((offset_val + 0x800) >> 12) & 0xFFFFF;
                let lo = offset_val & 0xFFF;
                if off + 8 <= data.len() {
                    let auipc = u32::from_le_bytes(data[off..off + 4].try_into().unwrap());
                    let auipc = (auipc & 0xFFF) | ((hi as u32) << 12);
                    data[off..off + 4].copy_from_slice(&auipc.to_le_bytes());
                    let jalr = u32::from_le_bytes(data[off + 4..off + 8].try_into().unwrap());
                    let jalr = (jalr & 0x000FFFFF) | ((lo as u32) << 20);
                    data[off + 4..off + 8].copy_from_slice(&jalr.to_le_bytes());
                }
            }
        }
        R_RISCV_BRANCH => {
            let offset_val =
                super::displacement::checked_delta(super::displacement::Encoding::Branch, s, a, p)?;
            patch_b_type(data, off, offset_val);
        }
        R_RISCV_JAL => {
            let offset_val =
                super::displacement::checked_delta(super::displacement::Encoding::Jal, s, a, p)?;
            patch_j_type(data, off, offset_val);
        }
        R_RISCV_RVC_BRANCH => {
            let offset_val = super::displacement::checked_delta(
                super::displacement::Encoding::RvcBranch,
                s,
                a,
                p,
            )?;
            patch_cb_type(data, off, offset_val);
        }
        R_RISCV_RVC_JUMP => {
            let offset_val = super::displacement::checked_delta(
                super::displacement::Encoding::RvcJump,
                s,
                a,
                p,
            )?;
            patch_cj_type(data, off, offset_val);
        }
        R_RISCV_HI20 => {
            let val = (s as i64 + a) as u32;
            let hi = (val.wrapping_add(0x800)) & 0xFFFFF000;
            if off + 4 <= data.len() {
                let insn = u32::from_le_bytes(data[off..off + 4].try_into().unwrap());
                let insn = (insn & 0xFFF) | hi;
                data[off..off + 4].copy_from_slice(&insn.to_le_bytes());
            }
        }
        R_RISCV_LO12_I => {
            let val = (s as i64 + a) as u32;
            patch_i_type(data, off, val & 0xFFF);
        }
        R_RISCV_LO12_S => {
            let val = (s as i64 + a) as u32;
            patch_s_type(data, off, val & 0xFFF);
        }
        R_RISCV_TPREL_HI20 => {
            let val = (s as i64 + a - ctx.tls_vaddr as i64) as u32;
            let hi = (val.wrapping_add(0x800)) & 0xFFFFF000;
            if off + 4 <= data.len() {
                let insn = u32::from_le_bytes(data[off..off + 4].try_into().unwrap());
                let insn = (insn & 0xFFF) | hi;
                data[off..off + 4].copy_from_slice(&insn.to_le_bytes());
            }
        }
        R_RISCV_TPREL_LO12_I => {
            let val = (s as i64 + a - ctx.tls_vaddr as i64) as u32;
            patch_i_type(data, off, val & 0xFFF);
        }
        R_RISCV_TPREL_LO12_S => {
            let val = (s as i64 + a - ctx.tls_vaddr as i64) as u32;
            patch_s_type(data, off, val & 0xFFF);
        }
        R_RISCV_TPREL_ADD => { /* hint */ }

        R_RISCV_ADD8 => {
            if off < data.len() {
                data[off] = data[off].wrapping_add((s as i64 + a) as u8);
            }
        }
        R_RISCV_ADD16 => {
            if off + 2 <= data.len() {
                let cur = u16::from_le_bytes(data[off..off + 2].try_into().unwrap());
                let val = cur.wrapping_add((s as i64 + a) as u16);
                data[off..off + 2].copy_from_slice(&val.to_le_bytes());
            }
        }
        R_RISCV_ADD32 => {
            if off + 4 <= data.len() {
                let cur = u32::from_le_bytes(data[off..off + 4].try_into().unwrap());
                let val = cur.wrapping_add((s as i64 + a) as u32);
                data[off..off + 4].copy_from_slice(&val.to_le_bytes());
            }
        }
        R_RISCV_ADD64 => {
            if off + 8 <= data.len() {
                let cur = u64::from_le_bytes(data[off..off + 8].try_into().unwrap());
                let val = cur.wrapping_add((s as i64 + a) as u64);
                data[off..off + 8].copy_from_slice(&val.to_le_bytes());
            }
        }
        R_RISCV_SUB8 => {
            if off < data.len() {
                data[off] = data[off].wrapping_sub((s as i64 + a) as u8);
            }
        }
        R_RISCV_SUB16 => {
            if off + 2 <= data.len() {
                let cur = u16::from_le_bytes(data[off..off + 2].try_into().unwrap());
                let val = cur.wrapping_sub((s as i64 + a) as u16);
                data[off..off + 2].copy_from_slice(&val.to_le_bytes());
            }
        }
        R_RISCV_SUB32 => {
            if off + 4 <= data.len() {
                let cur = u32::from_le_bytes(data[off..off + 4].try_into().unwrap());
                let val = cur.wrapping_sub((s as i64 + a) as u32);
                data[off..off + 4].copy_from_slice(&val.to_le_bytes());
            }
        }
        R_RISCV_SUB64 => {
            if off + 8 <= data.len() {
                let cur = u64::from_le_bytes(data[off..off + 8].try_into().unwrap());
                let val = cur.wrapping_sub((s as i64 + a) as u64);
                data[off..off + 8].copy_from_slice(&val.to_le_bytes());
            }
        }
        R_RISCV_SET6 => {
            if off < data.len() {
                data[off] = (data[off] & 0xC0) | (((s as i64 + a) as u8) & 0x3F);
            }
        }
        R_RISCV_SUB6 => {
            if off < data.len() {
                let cur = data[off] & 0x3F;
                let val = cur.wrapping_sub((s as i64 + a) as u8) & 0x3F;
                data[off] = (data[off] & 0xC0) | val;
            }
        }
        R_RISCV_SET8 => {
            if off < data.len() {
                data[off] = (s as i64 + a) as u8;
            }
        }
        R_RISCV_SET16 => {
            if off + 2 <= data.len() {
                let val = (s as i64 + a) as u16;
                data[off..off + 2].copy_from_slice(&val.to_le_bytes());
            }
        }
        R_RISCV_SET32 => {
            if off + 4 <= data.len() {
                let val = (s as i64 + a) as u32;
                data[off..off + 4].copy_from_slice(&val.to_le_bytes());
            }
        }
        R_RISCV_32_PCREL => {
            if off + 4 <= data.len() {
                let val = ((s as i64 + a) - p as i64) as u32;
                data[off..off + 4].copy_from_slice(&val.to_le_bytes());
            }
        }
        R_RISCV_SET_ULEB128 | R_RISCV_SUB_ULEB128 => {
            return Err(format!(
                "unpaired RISC-V ULEB128 relocation in '{}'",
                obj_name
            ));
        }
        R_RISCV_TLS_GD_HI20 | R_RISCV_TLS_GOT_HI20 => {
            apply_tls_hi20(rela_type, data, off, s, a, p, sym, obj_idx, ctx)?;
        }
        other => {
            // Collecting RELATIVE entries is not a license to discard other
            // relocation kinds. No dynamic relocation is emitted by this arm.
            return Err(format!(
                "unsupported RISC-V relocation type {} for symbol '{}' in '{}'",
                other, sym.name, obj_name
            ));
        }
    }

    Ok(())
}

/// Patch the `auipc` of a TLS access: either the `hi20` of a real
/// `tls_index`/`tls_offset` GOT slot, or -- for a `GD` access to a variable
/// the executable itself defines -- the relaxed `lui` form carrying the TP
/// offset. A shared object has no static TP offset to bake in, so a `GD`
/// access without a `tls_index` slot is refused rather than mis-encoded.
fn apply_tls_hi20(
    rela_type: u32,
    data: &mut [u8],
    off: usize,
    s: u64,
    a: i64,
    p: u64,
    sym: &super::elf_read::Symbol,
    obj_idx: usize,
    ctx: &RelocContext,
) -> Result<(), String> {
    let kind = if rela_type == R_RISCV_TLS_GD_HI20 {
        GotKind::TlsGd
    } else {
        GotKind::TlsIe
    };
    if ctx
        .got
        .offset(&got_sym_key(obj_idx, sym, a).0, kind)
        .is_some()
    {
        let slot = lookup_got_entry(sym, obj_idx, a, kind, ctx)?;
        let delta = super::displacement::checked_auipc_delta(slot, a, p)?;
        patch_u_type(data, off, delta as u32);
        return Ok(());
    }
    if kind == GotKind::TlsGd && !ctx.collect_relatives {
        // GD->LE relaxation of a variable the executable defines:
        // auipc -> lui with the TP offset (`collect_gd_tls_relax_info`).
        let tprel = (s as i64 + a - ctx.tls_vaddr as i64) as u32;
        let hi = tprel.wrapping_add(0x800) & 0xFFFFF000;
        if off + 4 <= data.len() {
            let lui_insn: u32 = 0x00000537 | hi; // lui a0, hi
            data[off..off + 4].copy_from_slice(&lui_insn.to_le_bytes());
        }
        return Ok(());
    }
    Err(format!("no RISC-V {kind:?} GOT slot for '{}'", sym.name))
}

/// Find the hi20 value for a PCREL_LO12 relocation, dispatching to the
/// appropriate variant based on whether we're building an executable or shared lib.
fn find_hi20_value_for_reloc(
    obj: &super::elf_read::ElfObject,
    obj_idx: usize,
    sec_idx: usize,
    ctx: &RelocContext,
    auipc_addr: u64,
    sec_offset: u64,
) -> i64 {
    if ctx.collect_relatives {
        super::relocations::find_hi20_value_shared(
            obj,
            obj_idx,
            sec_idx,
            ctx.sec_mapping,
            ctx.section_vaddrs,
            ctx.local_sym_vaddrs,
            ctx.global_syms,
            auipc_addr,
            sec_offset,
            ctx.got_vaddr,
            ctx.got,
        )
    } else {
        super::relocations::find_hi20_value(
            obj,
            obj_idx,
            sec_idx,
            ctx.sec_mapping,
            ctx.section_vaddrs,
            ctx.local_sym_vaddrs,
            ctx.global_syms,
            auipc_addr,
            sec_offset,
            ctx.got_vaddr,
            ctx.got,
            ctx.got_plt_vaddr,
            ctx.gd_tls_relax_info,
            ctx.tls_vaddr,
        )
    }
}

/// Collect GD TLS relaxation info for executables: every general-dynamic
/// access to a variable the executable defines (no `tls_index` slot).
///
/// Pre-scans relocations to find R_RISCV_TLS_GD_HI20 entries and their
/// associated __tls_get_addr CALL_PLT entries, so they can be relaxed
/// to Local-Exec (LUI + ADD) during relocation application.
pub fn collect_gd_tls_relax_info(
    input_objs: &[(String, super::elf_read::ElfObject)],
    sec_mapping: &FxHashMap<(usize, usize), (usize, u64)>,
    section_vaddrs: &[u64],
    local_sym_vaddrs: &[Vec<u64>],
    global_syms: &FxHashMap<String, GlobalSym>,
    got: &GotLayout,
    gd_tls_relax_info: &mut FxHashMap<u64, (u64, i64)>,
    gd_tls_call_nop: &mut FxHashSet<u64>,
) {
    for (obj_idx, (_, obj)) in input_objs.iter().enumerate() {
        for (sec_idx, relocs) in obj.relocations.iter().enumerate() {
            if relocs.is_empty() {
                continue;
            }
            let (merged_idx, sec_offset) = match sec_mapping.get(&(obj_idx, sec_idx)) {
                Some(&v) => v,
                None => continue,
            };
            let ms_vaddr = section_vaddrs[merged_idx];

            for (ri, reloc) in relocs.iter().enumerate() {
                if reloc.rela_type == R_RISCV_TLS_GD_HI20 {
                    let sym = &obj.symbols[reloc.sym_idx as usize];
                    let key = got_sym_key(obj_idx, sym, reloc.addend).0;
                    if got.offset(&key, GotKind::TlsGd).is_some() {
                        continue; // an import keeps its `tls_index`
                    }
                    let offset = sec_offset + reloc.offset;
                    let auipc_vaddr = ms_vaddr + offset;
                    let sym_val = resolve_symbol_value(
                        sym,
                        reloc.sym_idx as usize,
                        obj,
                        obj_idx,
                        sec_mapping,
                        section_vaddrs,
                        local_sym_vaddrs,
                        global_syms,
                    );
                    gd_tls_relax_info.insert(auipc_vaddr, (sym_val, reloc.addend));

                    for j in (ri + 1)..relocs.len().min(ri + 8) {
                        let call_reloc = &relocs[j];
                        if call_reloc.rela_type == R_RISCV_CALL_PLT {
                            let call_sym = &obj.symbols[call_reloc.sym_idx as usize];
                            if call_sym.name == "__tls_get_addr" {
                                let call_offset = sec_offset + call_reloc.offset;
                                let call_vaddr = ms_vaddr + call_offset;
                                gd_tls_call_nop.insert(call_vaddr);
                                break;
                            }
                        }
                    }
                }
            }
        }
    }
}
