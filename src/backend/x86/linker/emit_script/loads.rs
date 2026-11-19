//! File layout for the script linker. A LOAD is an affine mapping, not a
//! collection of sections that happen to have page-congruent file offsets.

use super::{OutSec, PT_LOAD_, section_segment_permissions};
use crate::backend::linker_common::linker_script::PhdrDecl;

#[derive(Clone, Copy)]
pub(super) struct Load {
    pub offset: u64,
    pub vaddr: u64,
    pub paddr: u64,
    pub filesz: u64,
    pub memsz: u64,
    pub align: u64,
    pub flags: u32,
}

/// First file offset at or after `cursor` with the same page residue as `addr`.
pub(super) fn congruent(cursor: u64, addr: u64, align: u64) -> Result<u64, String> {
    debug_assert!(align.is_power_of_two());
    cursor
        .checked_add(addr.wrapping_sub(cursor) & (align - 1))
        .ok_or_else(|| "linker script: LOAD file offset overflows".to_owned())
}

fn section_end(addr: u64, os: &OutSec) -> Result<u64, String> {
    addr.checked_add(os.size)
        .ok_or_else(|| format!("linker script: section {} exceeds address space", os.name))
}

/// The implicit image historically maps the contiguous LMA image used by
/// kernel/overlay scripts. Keep that convention, but keep the same file/LMA
/// origin for both the header and the payload, even when NOBITS comes first.
pub(super) fn implicit(
    sections: &mut [OutSec],
    cursor: &mut u64,
    page: u64,
) -> Result<Load, String> {
    let mut start = None;
    let (mut hi, mut file_hi, mut flags, mut align) = (0, 0, 4, page);
    for os in sections.iter().filter(|os| os.is_alloc && os.size > 0) {
        start = Some(start.map_or(os.lma, |v: u64| v.min(os.lma)));
        let end = section_end(os.lma, os)?;
        hi = hi.max(end);
        if !os.nobits {
            file_hi = file_hi.max(end);
        }
        flags |= section_segment_permissions(os.flags);
        align = align.max(os.align);
    }
    let start = start.unwrap_or(0);
    let base = if file_hi == 0 {
        start & (align - 1)
    } else {
        congruent(*cursor, start, align)?
    };
    let filesz = file_hi.saturating_sub(start);
    if filesz != 0 {
        *cursor = base
            .checked_add(filesz)
            .ok_or("linker script: LOAD size overflows file")?;
    }
    for os in sections.iter_mut().filter(|os| os.is_alloc && os.size > 0) {
        os.file_offset = base
            .checked_add(os.lma - start)
            .ok_or("linker script: section file offset overflows")?;
    }
    Ok(Load {
        offset: base,
        vaddr: start,
        paddr: start,
        filesz,
        memsz: hi - start,
        align,
        flags,
    })
}

fn root(parents: &mut [usize], mut i: usize) -> usize {
    while parents[i] != i {
        parents[i] = parents[parents[i]];
        i = parents[i];
    }
    i
}

/// Allocate a file image for each declared LOAD. If more than one LOAD maps
/// the same file-backed section, those LOADs must use the same translation;
/// pack them as one component. No extra space is reserved for trailing
/// NOBITS or for a NOBITS-only segment. A gap *inside* the file image (even
/// if it contains NOBITS) stays zero-filled at its script-specified VMA.
pub(super) fn declared(
    sections: &mut [OutSec],
    decls: &[PhdrDecl],
    section_phdrs: &[Vec<String>],
    cursor: &mut u64,
    page: u64,
    ehdr_size: u64,
) -> Result<Vec<Option<Load>>, String> {
    let headers_end = *cursor;
    let mut parents: Vec<usize> = (0..decls.len()).collect();
    let mut members = vec![Vec::new(); decls.len()];
    // Keep output-section order for equal addresses. The first section at a
    // segment's lowest VMA supplies its LMA (different moduli are legal).
    let mut order: Vec<usize> = (0..sections.len()).collect();
    order.sort_by_key(|&i| sections[i].vaddr);
    for i in order {
        let os = &sections[i];
        if !os.is_alloc || os.size == 0 {
            continue;
        }
        let mut first = None;
        for (di, decl) in decls.iter().enumerate() {
            if decl.ptype != PT_LOAD_ || !section_phdrs[i].contains(&decl.name) {
                continue;
            }
            members[di].push(i);
            if !os.nobits {
                let first: usize = *first.get_or_insert(di);
                let a = root(&mut parents, first);
                let b = root(&mut parents, di);
                parents[a.max(b)] = a.min(b);
            }
        }
    }
    let mut component_start = vec![u64::MAX; decls.len()];
    let mut component_align = vec![page; decls.len()];
    let mut component_file_end = vec![0; decls.len()];
    let mut component_headers = vec![false; decls.len()];
    for (di, indices) in members.iter().enumerate() {
        let Some(&first) = indices.first() else {
            continue;
        };
        let r = root(&mut parents, di);
        component_start[r] = component_start[r].min(sections[first].vaddr);
        component_headers[r] |= decls[di].has_filehdr || decls[di].has_phdrs;
        for &i in indices {
            let os = &sections[i];
            component_align[r] = component_align[r].max(os.align);
            if !os.nobits {
                component_file_end[r] = component_file_end[r].max(section_end(os.vaddr, os)?);
            }
        }
    }
    let mut base_offsets = vec![0; decls.len()];
    for r in 0..decls.len() {
        let start = component_start[r];
        if start == u64::MAX {
            continue;
        }
        let align = component_align[r];
        let file_end = component_file_end[r];
        let base = if file_end != 0 || component_headers[r] {
            congruent(*cursor, start, align)?
        } else {
            start & (align - 1)
        };
        base_offsets[r] = base;
        if file_end != 0 {
            *cursor = base
                .checked_add(file_end - start)
                .ok_or("linker script: LOAD size overflows file")?;
        }
    }
    let mut loads = vec![None; decls.len()];
    for (di, indices) in members.into_iter().enumerate() {
        let Some(&first) = indices.first() else {
            continue;
        };
        let r = root(&mut parents, di);
        let start = sections[first].vaddr;
        let mut lma = sections[first].lma;
        let base = base_offsets[r];
        let mut offset = base
            .checked_add(start - component_start[r])
            .ok_or("linker script: LOAD file offset overflows")?;
        let mut vaddr = start;
        let (mut mem_end, mut file_end, mut flags) = (start, offset, 4);
        let has_data = indices.iter().any(|&i| !sections[i].nobits);
        for i in indices {
            let os = &mut sections[i];
            let end = section_end(os.vaddr, os)?;
            let file_at = base
                .checked_add(os.vaddr - component_start[r])
                .ok_or("linker script: section file offset overflows")?;
            if !os.nobits || os.file_offset == 0 {
                os.file_offset = file_at;
            }
            mem_end = mem_end.max(end);
            if !os.nobits {
                file_end = file_end.max(
                    file_at
                        .checked_add(os.size)
                        .ok_or("linker script: section size overflows file")?,
                );
            }
            flags |= section_segment_permissions(os.flags);
        }
        let decl = &decls[di];
        if decl.has_filehdr || decl.has_phdrs {
            let prefix_offset = if decl.has_filehdr { 0 } else { ehdr_size };
            let prefix = offset.checked_sub(prefix_offset).ok_or_else(|| {
                format!("linker script: no room for headers in LOAD '{}'", decl.name)
            })?;
            vaddr = vaddr.checked_sub(prefix).ok_or_else(|| {
                format!("linker script: no room for headers in LOAD '{}'", decl.name)
            })?;
            // Some scripts use zero as the physical origin of a high-VMA
            // image. AT() describes the section, so its LMA must account
            // for the prefix just as its VMA does.
            lma = lma.checked_sub(prefix).ok_or_else(|| {
                format!(
                    "linker script: no physical room for headers in LOAD '{}'",
                    decl.name
                )
            })?;
            offset = prefix_offset;
            if !has_data {
                file_end = headers_end;
            }
        }
        let filesz = file_end - offset;
        let memsz = mem_end - vaddr;
        if filesz > memsz {
            return Err(format!(
                "linker script: headers exceed the memory image of LOAD '{}'",
                decl.name
            ));
        }
        loads[di] = Some(Load {
            offset,
            vaddr,
            paddr: lma,
            filesz,
            memsz,
            align: component_align[r],
            flags: decl.flags.map_or(flags, |f| f as u32),
        });
    }
    Ok(loads)
}
