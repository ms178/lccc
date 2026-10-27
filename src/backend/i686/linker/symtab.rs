//! The `.symtab`/`.strtab` of an i686 executable or shared object.
//!
//! GNU ld writes a static symbol table unless told `-s`; `nm`, `perf`,
//! `addr2line -f` and gdb (for code without DWARF: crt files, static libc)
//! name addresses with it. Same content rules as the x86-64 writer: each
//! object's named locals in laid-out sections (object order, then address),
//! then the defined non-dynamic globals by name. Section and file symbols are
//! left out.

use super::shdr::{self, Shdr32};
use super::types::*;
use crate::common::fx_hash::FxHashMap;

/// A serialised symbol table: `Elf32_Sym` entries and their string table.
pub(super) struct SymbolTable {
    pub symtab: Vec<u8>,
    pub strtab: Vec<u8>,
    /// `sh_info`: one past the last local (the null symbol included).
    pub n_local: u32,
}

/// Build the table; `shdrs` must already be `shdr::finish`ed, since each
/// entry's `st_shndx` is a final header index. `tls_addr` is the start of
/// the TLS template: an `STT_TLS` value in a linked image is the offset
/// within it.
pub(super) fn build(
    inputs: &[InputObject],
    section_map: &SectionMap,
    output_sections: &[OutputSection],
    global_symbols: &FxHashMap<String, LinkerSymbol>,
    section_name_to_idx: &FxHashMap<String, usize>,
    shdrs: &[Shdr32],
    tls_addr: u32,
) -> SymbolTable {
    let out_hdr = shdr::output_index_map(shdrs);
    let mut symtab = vec![0u8; 16];
    let mut strtab = vec![0u8];
    let mut push = |name: &str, value: u32, size: u32, info: u8, other: u8, shndx: u16| {
        let name_off = strtab.len() as u32;
        strtab.extend_from_slice(name.as_bytes());
        strtab.push(0);
        symtab.extend_from_slice(&name_off.to_le_bytes());
        symtab.extend_from_slice(&value.to_le_bytes());
        symtab.extend_from_slice(&size.to_le_bytes());
        symtab.push(info);
        symtab.push(other);
        symtab.extend_from_slice(&shndx.to_le_bytes());
    };
    let tls_value = |ty: u8, addr: u32| {
        if ty == STT_TLS {
            addr.wrapping_sub(tls_addr)
        } else {
            addr
        }
    };

    let mut n_local = 1u32;
    for (obj_idx, obj) in inputs.iter().enumerate() {
        let mut locals: Vec<(u32, &InputSymbol, u16)> = obj
            .symbols
            .iter()
            .filter(|s| {
                s.binding == STB_LOCAL
                    && !s.name.is_empty()
                    && s.sym_type != STT_SECTION
                    && s.sym_type != STT_FILE
                    && !matches!(s.section_index, SHN_UNDEF | SHN_ABS | SHN_COMMON)
            })
            .filter_map(|s| {
                // Unmapped: a discarded COMDAT member, never laid out.
                let &(out, offset) = section_map.get(&(obj_idx, usize::from(s.section_index)))?;
                let &hdr = out_hdr.get(&out)?;
                Some((output_sections[out].addr + offset + s.value, s, hdr))
            })
            .collect();
        locals.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.name.cmp(&b.1.name)));
        for (addr, s, hdr) in locals {
            push(
                &s.name,
                tls_value(s.sym_type, addr),
                s.size,
                (STB_LOCAL << 4) | s.sym_type,
                s.visibility,
                hdr,
            );
            n_local += 1;
        }
    }

    let mut globals: Vec<(&str, &LinkerSymbol)> = global_symbols
        .iter()
        .filter(|(_, g)| {
            g.binding != STB_LOCAL && ((g.is_defined && !g.is_dynamic) || g.needs_copy)
        })
        .map(|(n, g)| (n.as_str(), g))
        .collect();
    globals.sort_unstable_by(|a, b| a.0.cmp(b.0));
    for (name, g) in globals {
        let (value, shndx) = if g.needs_copy {
            (
                g.copy_addr,
                shdr::containing_index(shdrs, name, g.copy_addr, SHN_ABS),
            )
        } else if let Some(&hdr) = out_hdr.get(&g.output_section) {
            (tls_value(g.sym_type, g.address), hdr)
        } else if g.output_section == usize::MAX
            && !super::shared::is_emitter_defined(name, section_name_to_idx)
        {
            (g.address, SHN_ABS)
        } else {
            (
                tls_value(g.sym_type, g.address),
                shdr::containing_index(shdrs, name, g.address, SHN_ABS),
            )
        };
        push(
            name,
            value,
            g.size,
            (g.binding << 4) | g.sym_type,
            g.visibility,
            shndx,
        );
    }
    SymbolTable {
        symtab,
        strtab,
        n_local,
    }
}

impl SymbolTable {
    /// Append the `.symtab` and `.strtab` headers (after every other header:
    /// non-allocated, they take no part in the address order) with file
    /// offsets from `file_end`; returns the new end of the file data.
    pub fn place(&self, shdrs: &mut Vec<Shdr32>, file_end: u32) -> u32 {
        let symtab_offset = align_up(file_end, 4);
        let strtab_offset = symtab_offset + self.symtab.len() as u32;
        // 1-based header indices: .symtab is shdrs.len() + 1, .strtab next.
        let strtab_index = shdrs.len() as u32 + 2;
        let header = |name: &str, ty, offset, size, link, info, align, entsize| Shdr32 {
            name: name.to_string(),
            out_idx: None,
            ty,
            flags: 0,
            addr: 0,
            offset,
            size,
            link,
            info,
            align,
            entsize,
        };
        shdrs.push(header(
            ".symtab",
            SHT_SYMTAB,
            symtab_offset,
            self.symtab.len() as u32,
            strtab_index,
            self.n_local,
            4,
            16,
        ));
        shdrs.push(header(
            ".strtab",
            SHT_STRTAB,
            strtab_offset,
            self.strtab.len() as u32,
            0,
            0,
            1,
            0,
        ));
        strtab_offset + self.strtab.len() as u32
    }

    /// Copy the tables to the offsets `place` gave them.
    pub fn write(&self, shdrs: &[Shdr32], output: &mut [u8]) {
        for (name, bytes) in [(".symtab", &self.symtab), (".strtab", &self.strtab)] {
            if let Some(h) = shdrs.iter().find(|h| h.out_idx.is_none() && h.name == name) {
                let at = h.offset as usize;
                output[at..at + bytes.len()].copy_from_slice(bytes);
            }
        }
    }
}
