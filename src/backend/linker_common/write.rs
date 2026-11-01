//! ELF64 binary emission helpers.
//!
//! Common functions for writing ELF64 section headers, program headers,
//! and performing alignment/padding. Used by x86, RISC-V, and ARM linkers.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

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

/// Diagnostic-only ELF inspection: does any `PT_LOAD` carry both `PF_W` and
/// `PF_X`?
///
/// The reader is streaming and uses O(1) memory. A kernel `vmlinux` can be
/// hundreds of MiB; slurping it just to look at a handful of program headers
/// would add avoidable page-cache pressure and I/O to every link. Only the
/// ELF header, the optional extended-numbering section header, and the program
/// header table are read. Every offset/size computation is checked, every read
/// is exact, and malformed or truncated output fails closed instead of being
/// silently treated as "no RWX segment".
///
/// LCCC emits little-endian ELF32/ELF64 only. Big-endian or unknown classes are
/// rejected rather than byte-swapped on a guess.
fn elf_has_rwx_load<R: Read + Seek>(input: &mut R) -> Result<bool, String> {
    let invalid = || "invalid ELF output while checking RWX segments".to_string();

    let file_len = input.seek(SeekFrom::End(0)).map_err(|_| invalid())?;
    input.seek(SeekFrom::Start(0)).map_err(|_| invalid())?;

    let mut header = [0u8; 64];
    input.read_exact(&mut header[..16]).map_err(|_| invalid())?;
    if &header[..4] != b"\x7fELF" || header[5] != 1 || header[6] != 1 {
        return Err(invalid());
    }
    let wide = match header[4] {
        1 => false,
        2 => true,
        _ => return Err(invalid()),
    };
    let header_len: u64 = if wide { 64 } else { 52 };
    input
        .read_exact(&mut header[16..header_len as usize])
        .map_err(|_| invalid())?;

    let u16_at = |off: usize| -> u16 { u16::from_le_bytes([header[off], header[off + 1]]) };
    let u32_at = |off: usize| -> u32 {
        u32::from_le_bytes([
            header[off],
            header[off + 1],
            header[off + 2],
            header[off + 3],
        ])
    };
    let offset_at = |off: usize| -> u64 {
        if wide {
            u64::from_le_bytes([
                header[off],
                header[off + 1],
                header[off + 2],
                header[off + 3],
                header[off + 4],
                header[off + 5],
                header[off + 6],
                header[off + 7],
            ])
        } else {
            u64::from(u32_at(off))
        }
    };

    // e_version and e_ehsize must describe the header we just parsed.
    if u32_at(20) != 1 || u64::from(u16_at(if wide { 52 } else { 40 })) != header_len {
        return Err(invalid());
    }

    let phoff = offset_at(if wide { 32 } else { 28 });
    let phentsize = u64::from(u16_at(if wide { 54 } else { 42 }));
    let mut phnum = u64::from(u16_at(if wide { 56 } else { 44 }));
    let min_phentsize: u64 = if wide { 56 } else { 32 };

    if phnum == 0 {
        return Ok(false);
    }
    if phoff == 0 || phentsize < min_phentsize {
        return Err(invalid());
    }

    // PN_XNUM: the real program-header count lives in section header 0's
    // sh_info. Validate that section header before trusting the count.
    if phnum == 0xffff {
        let shoff = offset_at(if wide { 40 } else { 32 });
        let shentsize = u64::from(u16_at(if wide { 58 } else { 46 }));
        let min_shentsize: u64 = if wide { 64 } else { 44 };
        let sh_info_off: u64 = if wide { 44 } else { 28 };
        if shoff == 0 || shentsize < min_shentsize {
            return Err(invalid());
        }
        let sh_end = shoff.checked_add(shentsize).ok_or_else(invalid)?;
        if sh_end > file_len {
            return Err(invalid());
        }
        let mut info = [0u8; 4];
        input
            .seek(SeekFrom::Start(shoff + sh_info_off))
            .map_err(|_| invalid())?;
        input.read_exact(&mut info).map_err(|_| invalid())?;
        phnum = u64::from(u32::from_le_bytes(info));
        if phnum == 0 {
            return Ok(false);
        }
    }

    let table_size = phnum.checked_mul(phentsize).ok_or_else(invalid)?;
    let table_end = phoff.checked_add(table_size).ok_or_else(invalid)?;
    if table_end > file_len {
        return Err(invalid());
    }

    let flags_off = if wide { 4usize } else { 24usize };
    let read_len = min_phentsize as usize;
    let mut phdr = [0u8; 56];
    let mut off = phoff;
    for _ in 0..phnum {
        input.seek(SeekFrom::Start(off)).map_err(|_| invalid())?;
        input
            .read_exact(&mut phdr[..read_len])
            .map_err(|_| invalid())?;
        let p_type = u32::from_le_bytes([phdr[0], phdr[1], phdr[2], phdr[3]]);
        let p_flags = u32::from_le_bytes([
            phdr[flags_off],
            phdr[flags_off + 1],
            phdr[flags_off + 2],
            phdr[flags_off + 3],
        ]);
        // PF_W | PF_X == 3. Other permission bits (and processor-specific
        // bits) do not affect this diagnostic.
        if p_type == 1 && p_flags & 3 == 3 {
            return Ok(true);
        }
        // table_end <= file_len bounds every stride; checked_add keeps the
        // invariant explicit and host-width independent.
        off = off.checked_add(phentsize).ok_or_else(invalid)?;
    }
    Ok(false)
}

/// GNU ld's default RWX LOAD-segment warning, checked BEFORE flat-binary
/// conversion so `--oformat=binary` cannot hide the diagnostic by destroying
/// the program headers. Like bfd, a fatal diagnostic retains the completed
/// ELF output; callers must not use that file after a failed link.
///
/// Suppression (`warn == false`) performs no I/O at all.
pub fn check_rwx_segments(path: impl AsRef<Path>, warn: bool, fatal: bool) -> Result<(), String> {
    if !warn {
        return Ok(());
    }
    let path = path.as_ref();
    let mut file =
        File::open(path).map_err(|e| format!("failed to read '{}': {e}", path.display()))?;
    if elf_has_rwx_load(&mut file)? {
        eprintln!(
            "lccc-ld: warning: {} has a LOAD segment with RWX permissions",
            path.display()
        );
        if fatal {
            return Err("warning treated as error: LOAD segment with RWX permissions".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod rwx_tests {
    use super::elf_has_rwx_load;
    use std::io::Cursor;

    fn image(wide: bool, flags: u32) -> Vec<u8> {
        let header_len = if wide { 64 } else { 52 };
        let phentsize = if wide { 56 } else { 32 };
        let mut elf = vec![0u8; header_len + phentsize];
        elf[..4].copy_from_slice(b"\x7fELF");
        elf[4] = if wide { 2 } else { 1 };
        elf[5] = 1; // ELFDATA2LSB
        elf[6] = 1; // EV_CURRENT
        elf[16..18].copy_from_slice(&2u16.to_le_bytes()); // ET_EXEC
        elf[20..24].copy_from_slice(&1u32.to_le_bytes()); // e_version
        if wide {
            elf[32..40].copy_from_slice(&(header_len as u64).to_le_bytes());
            elf[52..54].copy_from_slice(&(header_len as u16).to_le_bytes());
            elf[54..56].copy_from_slice(&(phentsize as u16).to_le_bytes());
            elf[56..58].copy_from_slice(&1u16.to_le_bytes());
        } else {
            elf[28..32].copy_from_slice(&(header_len as u32).to_le_bytes());
            elf[40..42].copy_from_slice(&(header_len as u16).to_le_bytes());
            elf[42..44].copy_from_slice(&(phentsize as u16).to_le_bytes());
            elf[44..46].copy_from_slice(&1u16.to_le_bytes());
        }
        let phoff = header_len;
        elf[phoff..phoff + 4].copy_from_slice(&1u32.to_le_bytes()); // PT_LOAD
        let f = phoff + if wide { 4 } else { 24 };
        elf[f..f + 4].copy_from_slice(&flags.to_le_bytes());
        elf
    }

    fn has(elf: &[u8]) -> Result<bool, String> {
        elf_has_rwx_load(&mut Cursor::new(elf))
    }

    #[test]
    fn classes_and_permissions() {
        for wide in [false, true] {
            for flags in 0..8 {
                assert_eq!(has(&image(wide, flags)).unwrap(), flags & 3 == 3);
            }
            // Only PT_LOAD is diagnosed; a writable+executable GNU_STACK
            // note is a different diagnostic (--warn-execstack).
            let mut elf = image(wide, 7);
            let phoff = if wide { 64 } else { 52 };
            elf[phoff..phoff + 4].copy_from_slice(&0x6474e551u32.to_le_bytes());
            assert!(!has(&elf).unwrap());
        }
    }

    #[test]
    fn truncation_fails_closed() {
        for wide in [false, true] {
            let elf = image(wide, 7);
            for len in 0..elf.len() {
                assert!(has(&elf[..len]).is_err(), "wide={wide} len={len}");
            }
        }
    }

    #[test]
    fn extended_phnum() {
        for wide in [false, true] {
            let header_len = if wide { 64 } else { 52 };
            let phentsize = if wide { 56 } else { 32 };
            let shentsize = if wide { 64 } else { 44 };
            let shoff = header_len + phentsize;
            let mut elf = image(wide, 7);
            elf.resize(shoff + shentsize, 0);
            if wide {
                elf[40..48].copy_from_slice(&(shoff as u64).to_le_bytes());
                elf[56..58].copy_from_slice(&0xffffu16.to_le_bytes());
                elf[58..60].copy_from_slice(&(shentsize as u16).to_le_bytes());
                elf[60..62].copy_from_slice(&1u16.to_le_bytes());
                elf[shoff + 44..shoff + 48].copy_from_slice(&1u32.to_le_bytes());
            } else {
                elf[32..36].copy_from_slice(&(shoff as u32).to_le_bytes());
                elf[44..46].copy_from_slice(&0xffffu16.to_le_bytes());
                elf[46..48].copy_from_slice(&(shentsize as u16).to_le_bytes());
                elf[48..50].copy_from_slice(&1u16.to_le_bytes());
                elf[shoff + 28..shoff + 32].copy_from_slice(&1u32.to_le_bytes());
            }
            assert!(has(&elf).unwrap());
            let info = shoff + if wide { 44 } else { 28 };
            elf[info..info + 4].fill(0);
            assert!(!has(&elf).unwrap());
            // A truncated section header must not be trusted.
            assert!(has(&elf[..shoff + shentsize - 1]).is_err());
        }
    }

    #[test]
    fn overflow_and_malformed_headers_fail_closed() {
        for wide in [false, true] {
            let mut elf = image(wide, 7);
            let phoff_field = if wide { 32 } else { 28 };
            let phnum_field = if wide { 56 } else { 44 };
            let phentsize_field = if wide { 54 } else { 42 };
            // phoff + phnum*phentsize overflows u64.
            if wide {
                elf[phoff_field..phoff_field + 8].copy_from_slice(&u64::MAX.to_le_bytes());
            } else {
                elf[phoff_field..phoff_field + 4].copy_from_slice(&u32::MAX.to_le_bytes());
            }
            elf[phnum_field..phnum_field + 2].copy_from_slice(&2u16.to_le_bytes());
            assert!(has(&elf).is_err());

            let mut elf = image(wide, 7);
            elf[phentsize_field..phentsize_field + 2].copy_from_slice(&u16::MAX.to_le_bytes());
            elf[phnum_field..phnum_field + 2].copy_from_slice(&0xfffeu16.to_le_bytes());
            assert!(has(&elf).is_err());

            let mut elf = image(wide, 7);
            elf[phentsize_field..phentsize_field + 2].copy_from_slice(&1u16.to_le_bytes());
            assert!(has(&elf).is_err());

            let mut elf = image(wide, 7);
            elf[4] = 3; // invalid EI_CLASS
            assert!(has(&elf).is_err());
            let mut elf = image(wide, 7);
            elf[5] = 2; // big-endian: not an LCCC output class
            assert!(has(&elf).is_err());
            let mut elf = image(wide, 7);
            elf[20..24].fill(0); // invalid e_version
            assert!(has(&elf).is_err());
            let mut elf = image(wide, 7);
            let ehsize = if wide { 52 } else { 40 };
            elf[ehsize..ehsize + 2].fill(0);
            assert!(has(&elf).is_err());
        }
    }

    #[test]
    fn multiple_headers_with_stride() {
        for wide in [false, true] {
            let header_len = if wide { 64 } else { 52 };
            let min = if wide { 56 } else { 32 };
            let stride = min + 8;
            let mut elf = vec![0u8; header_len + 2 * stride];
            elf[..4].copy_from_slice(b"\x7fELF");
            elf[4] = if wide { 2 } else { 1 };
            elf[5] = 1;
            elf[6] = 1;
            elf[20..24].copy_from_slice(&1u32.to_le_bytes());
            if wide {
                elf[32..40].copy_from_slice(&(header_len as u64).to_le_bytes());
                elf[52..54].copy_from_slice(&(header_len as u16).to_le_bytes());
                elf[54..56].copy_from_slice(&(stride as u16).to_le_bytes());
                elf[56..58].copy_from_slice(&2u16.to_le_bytes());
            } else {
                elf[28..32].copy_from_slice(&(header_len as u32).to_le_bytes());
                elf[40..42].copy_from_slice(&(header_len as u16).to_le_bytes());
                elf[42..44].copy_from_slice(&(stride as u16).to_le_bytes());
                elf[44..46].copy_from_slice(&2u16.to_le_bytes());
            }
            let flags_off = if wide { 4 } else { 24 };
            // First header is PT_NULL with RWX flags: must be ignored.
            let first = header_len;
            elf[first..first + 4].fill(0);
            elf[first + flags_off..first + flags_off + 4].copy_from_slice(&7u32.to_le_bytes());
            // Second header is PT_LOAD; only its flags decide the verdict.
            let second = header_len + stride;
            elf[second..second + 4].copy_from_slice(&1u32.to_le_bytes());
            elf[second + flags_off..second + flags_off + 4].copy_from_slice(&7u32.to_le_bytes());
            assert!(has(&elf).unwrap());
            elf[second + flags_off..second + flags_off + 4].copy_from_slice(&5u32.to_le_bytes());
            assert!(!has(&elf).unwrap());
        }
    }

    #[test]
    fn no_program_headers_is_not_rwx() {
        for wide in [false, true] {
            let header_len = if wide { 64 } else { 52 };
            let mut elf = image(wide, 7);
            elf.truncate(header_len);
            let phnum = if wide { 56 } else { 44 };
            elf[phnum..phnum + 2].fill(0);
            assert!(!has(&elf).unwrap());
        }
    }
}
