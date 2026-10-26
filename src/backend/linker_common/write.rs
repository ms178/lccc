//! ELF64 binary emission helpers.
//!
//! Common functions for writing ELF64 section headers, program headers,
//! and performing alignment/padding. Used by x86, RISC-V, and ARM linkers.

/// Write a 64-byte ELF64 section header to the buffer.
pub fn write_elf64_shdr(
    elf: &mut Vec<u8>,
    name: u32,
    sh_type: u32,
    flags: u64,
    addr: u64,
    offset: u64,
    size: u64,
    link: u32,
    info: u32,
    align: u64,
    entsize: u64,
) {
    elf.extend_from_slice(&name.to_le_bytes());
    elf.extend_from_slice(&sh_type.to_le_bytes());
    elf.extend_from_slice(&flags.to_le_bytes());
    elf.extend_from_slice(&addr.to_le_bytes());
    elf.extend_from_slice(&offset.to_le_bytes());
    elf.extend_from_slice(&size.to_le_bytes());
    elf.extend_from_slice(&link.to_le_bytes());
    elf.extend_from_slice(&info.to_le_bytes());
    elf.extend_from_slice(&align.to_le_bytes());
    elf.extend_from_slice(&entsize.to_le_bytes());
}

/// Write a 56-byte ELF64 program header by appending to the buffer.
pub fn write_elf64_phdr(
    elf: &mut Vec<u8>,
    p_type: u32,
    p_flags: u32,
    offset: u64,
    vaddr: u64,
    paddr: u64,
    filesz: u64,
    memsz: u64,
    p_align: u64,
) {
    elf.extend_from_slice(&p_type.to_le_bytes());
    elf.extend_from_slice(&p_flags.to_le_bytes());
    elf.extend_from_slice(&offset.to_le_bytes());
    elf.extend_from_slice(&vaddr.to_le_bytes());
    elf.extend_from_slice(&paddr.to_le_bytes());
    elf.extend_from_slice(&filesz.to_le_bytes());
    elf.extend_from_slice(&memsz.to_le_bytes());
    elf.extend_from_slice(&p_align.to_le_bytes());
}

/// Write a 56-byte ELF64 program header at a specific offset (for backpatching).
pub fn write_elf64_phdr_at(
    elf: &mut [u8],
    off: usize,
    p_type: u32,
    p_flags: u32,
    offset: u64,
    vaddr: u64,
    paddr: u64,
    filesz: u64,
    memsz: u64,
    p_align: u64,
) {
    elf[off..off + 4].copy_from_slice(&p_type.to_le_bytes());
    elf[off + 4..off + 8].copy_from_slice(&p_flags.to_le_bytes());
    elf[off + 8..off + 16].copy_from_slice(&offset.to_le_bytes());
    elf[off + 16..off + 24].copy_from_slice(&vaddr.to_le_bytes());
    elf[off + 24..off + 32].copy_from_slice(&paddr.to_le_bytes());
    elf[off + 32..off + 40].copy_from_slice(&filesz.to_le_bytes());
    elf[off + 40..off + 48].copy_from_slice(&memsz.to_le_bytes());
    elf[off + 48..off + 56].copy_from_slice(&p_align.to_le_bytes());
}

/// Align `val` up to the next multiple of `align` (power-of-two alignment).
pub fn align_up_64(val: u64, align: u64) -> u64 {
    if align <= 1 {
        val
    } else {
        (val + align - 1) & !(align - 1)
    }
}

/// Extend buffer with zero bytes to reach `target` length.
pub fn pad_to(buf: &mut Vec<u8>, target: usize) {
    if buf.len() < target {
        buf.resize(target, 0);
    }
}

/// Extract a flat raw binary (`--oformat=binary` / `objcopy -O binary`) from an
/// emitted ELF32 or ELF64 image.
///
/// Uses allocated non-`SHT_NOBITS` sections (`SHF_ALLOC`, `sh_size > 0`),
/// translating each section's VMA to its LMA via the covering `PT_LOAD`
/// segment (`p_paddr + (sh_addr - p_vaddr)`) when program headers exist.
pub fn extract_flat_binary_from_elf(elf: &[u8]) -> Result<Vec<u8>, String> {
    if elf.len() < 52 || &elf[0..4] != b"\x7fELF" {
        return Err("--oformat=binary: output is not a valid ELF image".into());
    }
    let is_64 = match elf[4] {
        1 => false,
        2 => true,
        c => return Err(format!("--oformat=binary: unsupported ELF class {c}")),
    };
    let r16 = |o: usize| -> Result<u16, String> {
        elf.get(o..o + 2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .ok_or_else(|| "--oformat=binary: truncated ELF header".into())
    };
    let r32 = |o: usize| -> Result<u32, String> {
        elf.get(o..o + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .ok_or_else(|| "--oformat=binary: truncated ELF header".into())
    };
    let r64 = |o: usize| -> Result<u64, String> {
        elf.get(o..o + 8)
            .map(|b| u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
            .ok_or_else(|| "--oformat=binary: truncated ELF header".into())
    };

    let (phoff, phentsize, phnum, shoff, shentsize, shnum) = if is_64 {
        (
            r64(32)? as usize,
            r16(54)? as usize,
            r16(56)? as usize,
            r64(40)? as usize,
            r16(58)? as usize,
            r16(60)? as usize,
        )
    } else {
        (
            r32(28)? as usize,
            r16(42)? as usize,
            r16(44)? as usize,
            r32(32)? as usize,
            r16(46)? as usize,
            r16(48)? as usize,
        )
    };

    // Collect PT_LOAD segments: (file_off, filesz, vaddr, paddr)
    let mut loads: Vec<(u64, u64, u64, u64)> = Vec::new();
    if phoff != 0 && phentsize >= if is_64 { 56 } else { 32 } {
        for i in 0..phnum {
            let p = phoff + i * phentsize;
            if r32(p)? != 1 {
                continue; // PT_LOAD == 1
            }
            let seg = if is_64 {
                (r64(p + 8)?, r64(p + 32)?, r64(p + 16)?, r64(p + 24)?)
            } else {
                (
                    r32(p + 4)? as u64,
                    r32(p + 16)? as u64,
                    r32(p + 8)? as u64,
                    r32(p + 12)? as u64,
                )
            };
            loads.push(seg);
        }
    }

    // Collect SHF_ALLOC non-NOBITS sections: (lma, file_off, size)
    let mut chunks: Vec<(u64, usize, usize)> = Vec::new();
    if shoff != 0 && shentsize >= if is_64 { 64 } else { 40 } {
        for i in 1..shnum {
            let s = shoff + i * shentsize;
            let sh_type = r32(s + 4)?;
            if sh_type == 0 || sh_type == 8 {
                continue; // SHT_NULL or SHT_NOBITS
            }
            let (flags, addr, off, size) = if is_64 {
                (r64(s + 8)?, r64(s + 16)?, r64(s + 24)?, r64(s + 32)?)
            } else {
                (
                    r32(s + 8)? as u64,
                    r32(s + 12)? as u64,
                    r32(s + 16)? as u64,
                    r32(s + 20)? as u64,
                )
            };
            if flags & 0x2 == 0 || size == 0 {
                continue; // not SHF_ALLOC or empty
            }
            let lma = loads
                .iter()
                .find(|&&(fo, fsz, _, _)| off >= fo && off.saturating_add(size) <= fo + fsz)
                .map_or(addr, |&(_, _, va, pa)| {
                    pa.wrapping_add(addr.wrapping_sub(va))
                });
            chunks.push((lma, off as usize, size as usize));
        }
    }

    if chunks.is_empty() {
        return Ok(Vec::new());
    }
    chunks.sort_by_key(|&(lma, _, _)| lma);
    let min_lma = chunks[0].0;
    let max_end = chunks
        .iter()
        .map(|&(lma, _, sz)| lma.saturating_add(sz as u64))
        .max()
        .unwrap_or(min_lma);
    let total_len = usize::try_from(max_end - min_lma)
        .map_err(|_| "--oformat=binary: image span overflows host memory".to_string())?;
    if total_len > 512 * 1024 * 1024 {
        return Err(format!(
            "--oformat=binary: image span ({total_len} bytes) exceeds 512 MiB safety limit"
        ));
    }
    let mut flat = vec![0u8; total_len];
    for (lma, foff, sz) in chunks {
        let src = elf
            .get(foff..foff + sz)
            .ok_or_else(|| "--oformat=binary: section extends past end of ELF file".to_string())?;
        let dst = (lma - min_lma) as usize;
        flat[dst..dst + sz].copy_from_slice(src);
    }
    Ok(flat)
}

/// Convert an already-written ELF output file in-place to a raw flat binary.
pub fn convert_elf_file_to_flat_binary(path: &str) -> Result<(), String> {
    let elf = std::fs::read(path).map_err(|e| format!("failed to read '{path}': {e}"))?;
    let flat = extract_flat_binary_from_elf(&elf)?;
    std::fs::write(path, &flat).map_err(|e| format!("failed to write '{path}': {e}"))
}

/// Format GNU ld's `--print-memory-usage` summary table for `(name, used, length)` tuples.
pub fn format_memory_usage_table(regions: &[(String, u64, u64)]) -> String {
    use std::fmt::Write as _;
    let fmt_size = |bytes: u64| -> String {
        const GB: u64 = 1024 * 1024 * 1024;
        const MB: u64 = 1024 * 1024;
        const KB: u64 = 1024;
        if bytes != 0 && bytes.is_multiple_of(GB) {
            format!("{:>7} GB", bytes / GB)
        } else if bytes != 0 && bytes.is_multiple_of(MB) {
            format!("{:>7} MB", bytes / MB)
        } else if bytes != 0 && bytes.is_multiple_of(KB) {
            format!("{:>7} KB", bytes / KB)
        } else {
            format!("{:>8} B", bytes)
        }
    };
    let mut out = String::from("Memory region         Used Size  Region Size  %age Used\n");
    for (name, used, length) in regions {
        let pct = if *length == 0 {
            0.0
        } else {
            (*used as f64 * 100.0) / (*length as f64)
        };
        let _ = writeln!(
            out,
            "{:<16}: {:>12} {:>12}    {:>6.2}%",
            name,
            fmt_size(*used),
            fmt_size(*length),
            pct
        );
    }
    out
}
