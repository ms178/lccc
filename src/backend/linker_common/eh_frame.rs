//! .eh_frame_hdr builder for stack unwinding.
//!
//! Builds the .eh_frame_hdr section pointed to by PT_GNU_EH_FRAME. Contains a
//! binary search table mapping PC addresses to their FDE entries in .eh_frame.
//!
//! Format:
//!   u8  version          = 1
//!   u8  eh_frame_ptr_enc = DW_EH_PE_pcrel | DW_EH_PE_sdata4 (0x1b)
//!   u8  fde_count_enc    = DW_EH_PE_udata4 (0x03)
//!   u8  table_enc        = DW_EH_PE_datarel | DW_EH_PE_sdata4 (0x3b)
//!   i32 eh_frame_ptr     (PC-relative offset to .eh_frame start)
//!   u32 fde_count        (number of FDEs in the table)
//!   For each FDE:
//!     i32 initial_location (relative to eh_frame_hdr start)
//!     i32 fde_address      (relative to eh_frame_hdr start)

/// Bounds of one non-zero CIE/FDE record.  The length excludes its own
/// 4/12-byte header and must include the 4/8-byte CIE pointer.  Check it
/// against the *remaining input* before advancing any of the three walkers:
/// a wrapping extended length used to invent FDEs and even move backwards.
fn record_bounds(data: &[u8], pos: usize, length: u32) -> Option<(usize, usize, usize)> {
    if data.len().checked_sub(pos)? < 4 {
        return None;
    }
    let (body, id_size, size) = if length == u32::MAX {
        let body = pos.checked_add(12)?;
        if body > data.len() {
            return None;
        }
        (body, 8, read_u64_le(data, pos + 4))
    } else {
        (pos.checked_add(4)?, 4, u64::from(length))
    };
    let size = usize::try_from(size).ok()?;
    let end = body.checked_add(size)?;
    (size >= id_size && end <= data.len()).then_some((body, end, id_size))
}

/// Count the number of FDE entries in an .eh_frame section by scanning structure.
/// This only reads length and CIE_id fields, so it works on unrelocated data.
/// Used during layout to reserve space for .eh_frame_hdr (12 + 8 * count bytes).
pub fn count_eh_frame_fdes(data: &[u8]) -> usize {
    let mut count = 0;
    let mut pos = 0;
    while pos + 4 <= data.len() {
        let length = read_u32_le(data, pos);
        if length == 0 {
            pos += 4; // terminator between concatenated input sections
            continue;
        }
        let Some((entry_data_start, entry_end, id_size)) = record_bounds(data, pos, length) else {
            break;
        };
        let cie_id = if id_size == 8 {
            read_u64_le(data, entry_data_start)
        } else {
            u64::from(read_u32_le(data, entry_data_start))
        };
        if cie_id != 0 {
            count += 1;
        }
        pos = entry_end;
    }
    count
}

/// Build .eh_frame_hdr data from the merged .eh_frame section.
///
/// `eh_frame_data`: the merged .eh_frame section bytes
/// `eh_frame_vaddr`: virtual address where .eh_frame is loaded
/// `eh_frame_hdr_vaddr`: virtual address where .eh_frame_hdr will be loaded
/// `is_64bit`: true for 64-bit ELF, false for 32-bit
///
/// Returns the .eh_frame_hdr section data, or empty vec if parsing fails.
pub fn build_eh_frame_hdr(
    eh_frame_data: &[u8],
    eh_frame_vaddr: u64,
    eh_frame_hdr_vaddr: u64,
    is_64bit: bool,
) -> Vec<u8> {
    // Parse .eh_frame to find all FDEs and their initial_location values
    let fdes = parse_eh_frame_fdes(eh_frame_data, eh_frame_vaddr, is_64bit);

    // Header: 4 bytes + eh_frame_ptr (4 bytes) + fde_count (4 bytes).
    // Every table displacement is explicitly sdata4/udata4.  Never truncate
    // an address or count into those fields: a wrapped entry makes the
    // unwinder's binary search return an unrelated FDE.  The caller treats an
    // empty result as "no usable header" and can fall back to linear scans.
    let header_size = 4usize + 4 + 4;
    let table_entry_size = 8usize; // two i32s per entry
    let Some(table_size) = fdes.len().checked_mul(table_entry_size) else {
        return Vec::new();
    };
    let Some(total_size) = header_size.checked_add(table_size) else {
        return Vec::new();
    };
    let eh_frame_ptr = i128::from(eh_frame_vaddr) - (i128::from(eh_frame_hdr_vaddr) + 4);
    let Ok(eh_frame_ptr) = i32::try_from(eh_frame_ptr) else {
        return Vec::new();
    };
    let Ok(fde_count) = u32::try_from(fdes.len()) else {
        return Vec::new();
    };
    let mut data = vec![0u8; total_size];

    // Version
    data[0] = 1;
    // eh_frame_ptr encoding: DW_EH_PE_pcrel | DW_EH_PE_sdata4
    data[1] = 0x1b;
    // fde_count encoding: DW_EH_PE_udata4
    data[2] = 0x03;
    // table encoding: DW_EH_PE_datarel | DW_EH_PE_sdata4
    data[3] = 0x3b;

    // eh_frame_ptr: PC-relative offset from &data[4] to .eh_frame.
    write_i32_le(&mut data, 4, eh_frame_ptr);
    // fde_count
    write_u32_le(&mut data, 8, fde_count);

    // Table entries: sorted by initial_location.  Each entry is
    // (initial_location - eh_frame_hdr_vaddr, fde_address - eh_frame_hdr_vaddr).
    for (i, fde) in fdes.iter().enumerate() {
        let off = header_size + i * table_entry_size;
        let loc_rel = i128::from(fde.initial_location) - i128::from(eh_frame_hdr_vaddr);
        let fde_rel = i128::from(fde.fde_vaddr) - i128::from(eh_frame_hdr_vaddr);
        let Ok(loc_rel) = i32::try_from(loc_rel) else {
            return Vec::new();
        };
        let Ok(fde_rel) = i32::try_from(fde_rel) else {
            return Vec::new();
        };
        write_i32_le(&mut data, off, loc_rel);
        write_i32_le(&mut data, off + 4, fde_rel);
    }

    data
}

/// An FDE entry parsed from .eh_frame
struct EhFrameFde {
    initial_location: u64,
    fde_vaddr: u64,
}

/// Parse .eh_frame section to extract FDE entries.
///
/// Returns a sorted list of FDEs by initial_location.
fn parse_eh_frame_fdes(data: &[u8], base_vaddr: u64, is_64bit: bool) -> Vec<EhFrameFde> {
    let mut fdes = Vec::new();
    let mut pos = 0;

    // Memoise the CIE -> FDE-encoding lookup.
    //
    // `parse_cie_fde_encoding` is a pure function of `(data, cie_pos)`, but it
    // was called once per *FDE*, and real programs share a handful of CIEs
    // across all of them: a 20 000-function link re-parsed the same CIE 20 000
    // times, which made `build_eh_frame_hdr` 5.4% of the whole link. A tiny
    // linear-scan cache keyed on `cie_pos` removes that work. It stays a Vec
    // rather than a HashMap because the distinct-CIE count is in the single
    // digits for essentially every real input, where linear scan over a
    // contiguous array beats hashing.
    let mut cie_cache: Vec<(usize, Option<u8>)> = Vec::new();

    while pos + 4 <= data.len() {
        let length = read_u32_le(data, pos);
        if length == 0 {
            pos += 4;
            continue;
        }
        let entry_start = pos;
        let Some((entry_data_start, entry_end, cie_id_field_size)) =
            record_bounds(data, pos, length)
        else {
            break;
        };
        // CIE id is zero; an FDE instead points backwards to its CIE.
        let cie_id = if cie_id_field_size == 8 {
            read_u64_le(data, entry_data_start)
        } else {
            u64::from(read_u32_le(data, entry_data_start))
        };
        if cie_id != 0 {
            // This is an FDE
            // The CIE_pointer is relative: entry_data_start - cie_id points to the CIE
            let Some(cie_pos) = usize::try_from(cie_id)
                .ok()
                .and_then(|back| entry_data_start.checked_sub(back))
            else {
                pos = entry_end;
                continue;
            };

            // Parse the CIE to get the FDE encoding (memoised; see cie_cache)
            let encoding = match cie_cache.iter().find(|&&(p, _)| p == cie_pos) {
                Some(&(_, enc)) => enc,
                None => {
                    let enc = parse_cie_fde_encoding(data, cie_pos, is_64bit);
                    cie_cache.push((cie_pos, enc));
                    enc
                }
            };
            let Some(fde_encoding) = encoding else {
                pos = entry_end;
                continue; // invalid CIE: never guess an FDE encoding
            };

            // After CIE_pointer comes: initial_location, address_range, ...
            let iloc_offset = entry_data_start + cie_id_field_size;
            if entry_end - iloc_offset < 4 {
                pos = entry_end;
                continue;
            }

            let Some(fde_vaddr) = base_vaddr.checked_add(entry_start as u64) else {
                pos = entry_end;
                continue;
            };

            // Decode initial_location based on the CIE's FDE encoding
            let initial_location = decode_eh_pointer(
                &data[..entry_end],
                iloc_offset,
                fde_encoding,
                base_vaddr + iloc_offset as u64,
                is_64bit,
            );

            if let Some(iloc) = initial_location {
                fdes.push(EhFrameFde {
                    initial_location: iloc,
                    fde_vaddr,
                });
            }
        }

        pos = entry_end;
    }

    // Sort by initial_location for binary search
    fdes.sort_by_key(|f| f.initial_location);
    fdes
}

/// Parse a CIE to extract the FDE pointer encoding (R augmentation).
///
/// `Some(0)` means a valid absolute encoding; `None` means malformed input.
fn parse_cie_fde_encoding(data: &[u8], cie_pos: usize, is_64bit: bool) -> Option<u8> {
    if data
        .len()
        .checked_sub(cie_pos)
        .is_none_or(|remaining| remaining < 4)
    {
        return None;
    }
    let Some((start, end, id_size)) = record_bounds(data, cie_pos, read_u32_le(data, cie_pos))
    else {
        return None;
    };
    let id = if id_size == 8 {
        read_u64_le(data, start)
    } else {
        u64::from(read_u32_le(data, start))
    };
    if id != 0 {
        return None;
    }
    let Some(&version) = data.get(start + id_size).filter(|_| start + id_size < end) else {
        return None;
    };
    // `.eh_frame` CIEs in the supported DWARF32/DWARF64 formats use the
    // v1/v3/v4 layouts below.  Treat unknown versions as malformed rather
    // than interpreting a v5 CIE with the v3 layout and returning a plausible
    // but wrong FDE encoding.
    if !matches!(version, 1..=4) {
        return None;
    }
    let aug_start = start + id_size + 1;
    let Some(aug_len) = data[aug_start..end].iter().position(|&b| b == 0) else {
        return None;
    };
    let aug = &data[aug_start..aug_start + aug_len];
    let mut cur = aug_start + aug_len + 1;
    let cie = &data[..end];

    // DWARF v4 inserts address_size and segment_selector_size between the
    // augmentation string and the alignment factors.  Omitting these two
    // bytes (the old v1/v3-only parser did) shifts every subsequent read: a
    // valid v4 zR CIE is then either rejected or, worse, assigned an
    // unrelated FDE encoding.  We do not support segmented EH pointers, and
    // the CIE address size must agree with the output ELF class used by the
    // decoder, so fail closed on either case.
    if version == 4 {
        let Some(&address_size) = cie.get(cur) else {
            return None;
        };
        let Some(&segment_selector_size) = cie.get(cur + 1) else {
            return None;
        };
        let expected_address_size = if is_64bit { 8 } else { 4 };
        if address_size != expected_address_size || segment_selector_size != 0 {
            return None;
        }
        cur += 2;
    }

    // The return-address column is a byte in CIE v1, ULEB in v3/v4.
    let Some((_, n)) = read_uleb128(cie, cur) else {
        return None;
    };
    cur += n; // code alignment factor
    let Some((_, n)) = read_sleb128(cie, cur) else {
        return None;
    };
    cur += n; // data alignment factor
    if version == 1 {
        if cur >= end {
            return None;
        }
        cur += 1;
    } else {
        let Some((_, n)) = read_uleb128(cie, cur) else {
            return None;
        };
        cur += n;
    }
    if aug.first() != Some(&b'z') {
        return Some(0x00); // default absolute-pointer encoding
    }
    let Some((len, n)) = read_uleb128(cie, cur) else {
        return None;
    };
    cur += n;
    let Some(aug_end) = usize::try_from(len)
        .ok()
        .and_then(|len| cur.checked_add(len))
        .filter(|&e| e <= end)
    else {
        return None;
    };
    for &ch in &aug[1..] {
        match ch {
            b'R' => {
                return data.get(cur).filter(|_| cur < aug_end).copied();
            }
            b'L' => cur += 1,
            b'P' => {
                let Some(&enc) = data.get(cur).filter(|_| cur < aug_end) else {
                    return None;
                };
                cur += 1;
                let size = eh_pointer_size(enc, is_64bit);
                if size == 0 {
                    return None; // variable-length personality: cannot locate R
                }
                cur += size;
            }
            b'S' | b'B' => {} // no augmentation data
            _ => return None,
        }
        if cur > aug_end {
            return None;
        }
    }
    Some(0x00)
}

/// Decode an eh_frame pointer value based on its encoding.
fn decode_eh_pointer(
    data: &[u8],
    offset: usize,
    encoding: u8,
    pc: u64,
    is_64bit: bool,
) -> Option<u64> {
    if encoding == 0xFF {
        return None;
    } // DW_EH_PE_omit

    let base_enc = encoding & 0x0F;
    let rel = encoding & 0x70;

    // Indirect encodings require dereferencing a relocated address in the
    // output image.  This parser only has the section bytes, not the final
    // address space, so accepting one would manufacture the slot address as
    // the function PC.  Fail closed instead.
    if encoding & 0x80 != 0 {
        return None;
    }

    // Keep the raw value unsigned plus an explicit signedness bit.  The old
    // implementation forced udata4/udata8 through `i64`; a perfectly valid
    // 32-bit absolute address >= 0x8000_0000 was then sign-extended, and a
    // 64-bit udata8 with bit 63 set could overflow during a PC-relative add.
    // Signed encodings are sign-extended only when the relative base is
    // applied; unsigned encodings remain unsigned all the way through.
    let (raw_val, signed) = match base_enc {
        0x00 => {
            // DW_EH_PE_absptr
            if is_64bit {
                if data.len().checked_sub(offset).is_none_or(|n| n < 8) {
                    return None;
                }
                (read_u64_le(data, offset), false)
            } else {
                if data.len().checked_sub(offset).is_none_or(|n| n < 4) {
                    return None;
                }
                (u64::from(read_u32_le(data, offset)), false)
            }
        }
        0x01 => {
            // DW_EH_PE_uleb128
            let (v, _) = read_uleb128(data, offset)?;
            (v, false)
        }
        0x02 => {
            // DW_EH_PE_udata2
            if data.len().checked_sub(offset).is_none_or(|n| n < 2) {
                return None;
            }
            (
                u64::from(u16::from_le_bytes([data[offset], data[offset + 1]])),
                false,
            )
        }
        0x03 => {
            // DW_EH_PE_udata4
            if data.len().checked_sub(offset).is_none_or(|n| n < 4) {
                return None;
            }
            (u64::from(read_u32_le(data, offset)), false)
        }
        0x04 => {
            // DW_EH_PE_udata8
            if data.len().checked_sub(offset).is_none_or(|n| n < 8) {
                return None;
            }
            (read_u64_le(data, offset), false)
        }
        0x09 => {
            // DW_EH_PE_sleb128
            let (v, _) = read_sleb128(data, offset)?;
            (v as u64, true)
        }
        0x0A => {
            // DW_EH_PE_sdata2
            if data.len().checked_sub(offset).is_none_or(|n| n < 2) {
                return None;
            }
            (
                i16::from_le_bytes([data[offset], data[offset + 1]]) as i64 as u64,
                true,
            )
        }
        0x0B => {
            // DW_EH_PE_sdata4
            if data.len().checked_sub(offset).is_none_or(|n| n < 4) {
                return None;
            }
            (read_i32_le(data, offset) as i64 as u64, true)
        }
        0x0C => {
            // DW_EH_PE_sdata8
            if data.len().checked_sub(offset).is_none_or(|n| n < 8) {
                return None;
            }
            (read_u64_le(data, offset) as i64 as u64, true)
        }
        _ => return None,
    };

    match rel {
        0x00 => Some(raw_val), // DW_EH_PE_absptr
        0x10 => {
            // DW_EH_PE_pcrel.  Use checked arithmetic: malformed relocation
            // data must be rejected, never wrap to an unrelated FDE.
            if signed {
                pc.checked_add_signed(raw_val as i64)
            } else {
                pc.checked_add(raw_val)
            }
        }
        // The caller does not provide text/data bases, and treating either
        // encoding as absolute silently produces a wrong search table.
        0x20 | 0x30 => None, // DW_EH_PE_textrel/datarel
        _ => None,
    }
}

/// Return the byte size of an encoded pointer.
fn eh_pointer_size(encoding: u8, is_64bit: bool) -> usize {
    match encoding & 0x0F {
        0x00 => {
            if is_64bit {
                8
            } else {
                4
            }
        } // absptr
        0x02 | 0x0A => 2, // udata2/sdata2
        0x03 | 0x0B => 4, // udata4/sdata4
        0x04 | 0x0C => 8, // udata8/sdata8
        _ => 0,
    }
}

// ── Local binary helpers (avoid depending on elf::io to keep this self-contained) ──

// Little-endian reads reuse the shared helpers in `backend::elf::io` rather
// than re-implementing them here. The previous private copies indexed one byte
// at a time (eight bounds checks per u64) and were not even `#[inline]`;
// `read_u32_le` alone was 2.80% of a 20 000-symbol link.
use crate::backend::elf::{
    read_i32 as read_i32_le, read_u32 as read_u32_le, read_u64 as read_u64_le,
};

fn write_i32_le(data: &mut [u8], off: usize, val: i32) {
    let b = val.to_le_bytes();
    data[off..off + 4].copy_from_slice(&b);
}

fn write_u32_le(data: &mut [u8], off: usize, val: u32) {
    let b = val.to_le_bytes();
    data[off..off + 4].copy_from_slice(&b);
}

/// Bounded LEB readers: malformed inputs must not truncate modulo 64 bits or
/// borrow bytes from the next CIE/FDE.  A 64-bit number needs at most 10 bytes.
fn read_uleb128(data: &[u8], off: usize) -> Option<(u64, usize)> {
    let mut result = 0u128;
    for (i, &byte) in data.get(off..)?.iter().take(10).enumerate() {
        result |= u128::from(byte & 0x7f) << (i * 7);
        if byte & 0x80 == 0 {
            return Some((u64::try_from(result).ok()?, i + 1));
        }
    }
    None
}

fn read_sleb128(data: &[u8], off: usize) -> Option<(i64, usize)> {
    let mut result = 0i128;
    for (i, &byte) in data.get(off..)?.iter().take(10).enumerate() {
        let shift = (i + 1) * 7;
        result |= i128::from(byte & 0x7f) << (i * 7);
        if byte & 0x80 == 0 {
            if byte & 0x40 != 0 {
                result |= -1i128 << shift;
            }
            return Some((i64::try_from(result).ok()?, i + 1));
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Unrelocated .eh_frame record scanning and --gc-sections interaction
// ---------------------------------------------------------------------------

/// One CIE or FDE record inside an **unrelocated** input `.eh_frame` section.
///
/// [`parse_eh_frame_fdes`] decodes *values* and therefore only makes sense on
/// already-relocated bytes.  Garbage collection runs long before that, on the
/// raw input, where the `initial_location` field is still zero and only the
/// relocation entry says which function the FDE describes.  This struct is the
/// structural view needed at that stage: byte ranges plus the offset the
/// linker will later patch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EhFrameRecord {
    /// Byte offset of the record's length field.
    pub start: usize,
    /// One past the last byte of the record.
    pub end: usize,
    /// True for an FDE, false for a CIE.
    pub is_fde: bool,
    /// For an FDE: byte offset of the `initial_location` field, i.e. the slot
    /// a relocation patches with the address of the described function.
    pub iloc_offset: Option<usize>,
    /// Byte offset of the `CIE_id`/`CIE_pointer` field, which is 4 bytes for
    /// the 32-bit DWARF format and 8 for the 64-bit one.
    pub id_offset: usize,
    /// Width in bytes of that field.
    pub id_size: usize,
}

/// Walk an unrelocated `.eh_frame` section and return every CIE/FDE record.
///
/// Only length and `CIE_id` fields are read, so this is valid on raw input
/// data.  Truncated or malformed input terminates the scan instead of
/// panicking -- `.eh_frame` comes from arbitrary object files.
pub fn scan_eh_frame_records(data: &[u8]) -> Vec<EhFrameRecord> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos + 4 <= data.len() {
        let length = read_u32_le(data, pos);
        if length == 0 {
            pos += 4;
            continue;
        }
        let Some((data_start, end, id_size)) = record_bounds(data, pos, length) else {
            break;
        };
        let cie_id = if id_size == 8 {
            read_u64_le(data, data_start)
        } else {
            read_u32_le(data, data_start) as u64
        };
        let is_fde = cie_id != 0;
        out.push(EhFrameRecord {
            start: pos,
            end,
            is_fde,
            iloc_offset: if is_fde {
                Some(data_start + id_size)
            } else {
                None
            },
            id_offset: data_start,
            id_size,
        });
        pos = end;
    }
    out
}

/// An `.eh_frame` input section with some records removed; see
/// [`compact_eh_frame`].
pub struct EhFrameCompaction {
    /// The surviving records, packed.  No zero terminator is added: the
    /// compacted section is one of many concatenated into the output, and a
    /// terminator in the middle of `.eh_frame` ends it for every consumer
    /// that walks the section (libgcc's `__register_frame_info` path, gdb).
    /// The link's single terminator comes from `crtend.o`, as with GNU ld.
    /// (Zero-length records of the input are not records and do not
    /// survive either.)
    pub data: Vec<u8>,
    records: Vec<EhFrameRecord>,
    /// New offset of each record (`usize::MAX` when pruned).
    new_start: Vec<usize>,
}

impl EhFrameCompaction {
    /// Where a byte (a relocation offset) of the original section lives in
    /// the compacted one: `None` inside a pruned record. Offsets outside
    /// every record (malformed input) keep their position rather than being
    /// silently dropped.
    pub fn map_offset(&self, off: usize) -> Option<usize> {
        // Records are sorted and disjoint: binary search, since this runs
        // once per relocation of sections with thousands of FDEs.
        let i = self.records.partition_point(|r| r.start <= off);
        match i.checked_sub(1).filter(|&i| off < self.records[i].end) {
            Some(i) if self.new_start[i] == usize::MAX => None,
            Some(i) => Some(off - self.records[i].start + self.new_start[i]),
            None => Some(off),
        }
    }
}

/// Remove the records flagged in `prune` (FDEs only; a CIE is never pruned)
/// from an unrelocated `.eh_frame` section and pack the survivors.
///
/// Every surviving FDE's `CIE_pointer` is relative to its own position, so
/// it is re-encoded for the new layout.  The CIE is found by decoding the
/// pointer, never by assuming it is the nearest preceding CIE: GNU as emits
/// a CIE before the first FDE that needs it and later FDEs point back past
/// intervening CIEs (`zR` function, `zPLR` function with a cleanup, `zR`
/// function), and the integrated assembler places all CIEs first.  Re-aiming
/// such an FDE at the nearest CIE hands the unwinder the wrong augmentation
/// -- a phantom personality routine and LSDA for a plain C frame.
pub fn compact_eh_frame(
    data: &[u8],
    records: &[EhFrameRecord],
    prune: &[bool],
) -> EhFrameCompaction {
    let fde_only: Vec<bool> = records
        .iter()
        .zip(prune)
        .map(|(r, &p)| p && r.is_fde)
        .collect();
    compact_records(data, records, &fde_only)
}

/// Index in `records` (from [`scan_eh_frame_records`] over `data`) of the
/// CIE that FDE `records[fde]` uses, if its `CIE_pointer` leads to one.
pub(crate) fn fde_cie_index(data: &[u8], records: &[EhFrameRecord], fde: usize) -> Option<usize> {
    let rec = records.get(fde).filter(|r| r.is_fde)?;
    let id = if rec.id_size == 8 {
        read_u64_le(data, rec.id_offset)
    } else {
        u64::from(read_u32_le(data, rec.id_offset))
    };
    cie_at(records, (rec.id_offset as u64).wrapping_sub(id) as usize)
}

/// Index of the CIE record starting at `pos` (records are sorted by start).
fn cie_at(records: &[EhFrameRecord], pos: usize) -> Option<usize> {
    records
        .binary_search_by_key(&pos, |r| r.start)
        .ok()
        .filter(|&i| !records[i].is_fde)
}

/// [`compact_eh_frame`] for any records, CIEs included.  An FDE whose CIE
/// is dropped keeps its old `CIE_pointer` bytes: the caller re-aims it (see
/// [`pack_eh_frame_sections`]).
fn compact_records(data: &[u8], records: &[EhFrameRecord], drop: &[bool]) -> EhFrameCompaction {
    let read_id = |rec: &EhFrameRecord| -> u64 {
        if rec.id_size == 8 {
            read_u64_le(data, rec.id_offset)
        } else {
            read_u32_le(data, rec.id_offset) as u64
        }
    };
    let mut new_start = vec![usize::MAX; records.len()];
    let mut off = 0usize;
    for (i, rec) in records.iter().enumerate() {
        if drop[i] {
            continue;
        }
        new_start[i] = off;
        off += rec.end - rec.start;
    }
    let mut out = vec![0u8; off];
    for (i, rec) in records.iter().enumerate() {
        let dst = new_start[i];
        if dst == usize::MAX {
            continue;
        }
        out[dst..dst + (rec.end - rec.start)].copy_from_slice(&data[rec.start..rec.end]);
        if !rec.is_fde {
            continue;
        }
        // CIE_pointer = offset_of(CIE_pointer field) - offset_of(CIE).
        let cie_pos = (rec.id_offset as u64).wrapping_sub(read_id(rec)) as usize;
        let Some(cie) = cie_at(records, cie_pos) else {
            continue; // dangling pointer in the input: leave it as it was
        };
        if new_start[cie] == usize::MAX {
            continue; // the CIE was merged away; the caller re-aims this FDE
        }
        let field_pos = dst + (rec.id_offset - rec.start);
        if new_start[cie] >= field_pos {
            // Malformed input: the FDE precedes (or overlaps) the CIE its
            // pointer names, so the re-encoded `CIE_pointer` would be zero
            // or negative. `field_pos - new_start[cie]` underflows: a
            // panic in debug, a wild ~4 GiB-back pointer in release (and a
            // zero would even reclassify the FDE as a CIE). Well-formed
            // input always points strictly backwards, so skipping the
            // rewrite preserves every valid layout and leaves the garbage
            // bytes untouched instead of crashing the link.
            continue;
        }
        let value = field_pos - new_start[cie];
        if rec.id_size == 8 {
            out[field_pos..field_pos + 8].copy_from_slice(&(value as u64).to_le_bytes());
        } else {
            out[field_pos..field_pos + 4].copy_from_slice(&(value as u32).to_le_bytes());
        }
    }
    EhFrameCompaction {
        data: out,
        records: records.to_vec(),
        new_start,
    }
}

/// Drop the FDEs that describe garbage-collected functions.
///
/// `--gc-sections` works at *input section* granularity, but a translation
/// unit emits **one** `.eh_frame` section holding an FDE for every function it
/// defines.  With `-ffunction-sections` the functions land in their own
/// sections and are collected individually, so the FDE set and the live code
/// set diverge inside a single input section.  Leaving the stale FDEs in place
/// is not merely wasteful: after the dead sections are compacted away the
/// addresses they describe are recycled by live code, so the `.eh_frame_hdr`
/// binary-search table can hand the unwinder an FDE whose CFI belongs to a
/// different function.
///
/// This is why the FDE set must be pruned rather than the section being kept
/// or dropped wholesale.  bfd, lld and mold all do the equivalent.
///
/// # Why compaction and not zeroing
///
/// A zero-length record is the DWARF end-of-`.eh_frame` terminator.  Blanking
/// a dead FDE in place therefore *truncates* the section for every consumer
/// that walks it directly (gdb, `readelf --debug-dump=frames`, libgcc's
/// `__register_frame` path): only the FDEs before the first pruned one remain
/// visible.  The records are instead removed and the survivors compacted,
/// which also reclaims the bytes.  That requires two fixups, both handled
/// here:
///
/// * every surviving FDE's `CIE_pointer` is *relative to its own position*, so
///   it is rewritten from the new offsets; and
/// * relocations are shifted by the same delta, and those inside a pruned
///   record are dropped.
///
/// Returns the number of FDEs dropped.  FDEs whose `initial_location` has no
/// relocation (already-resolved inputs, e.g. the product of an earlier
/// `ld -r`) are conservatively kept.
pub fn prune_dead_fdes(
    objects: &mut [crate::backend::linker_common::Elf64Object],
    dead: &crate::common::fx_hash::FxHashSet<(usize, usize)>,
) -> usize {
    if dead.is_empty() {
        return 0;
    }
    use crate::backend::elf::{SHN_ABS, SHN_COMMON, SHN_UNDEF};
    let mut dropped = 0usize;
    for (obj_idx, obj) in objects.iter_mut().enumerate() {
        let crate::backend::linker_common::Elf64Object {
            sections,
            symbols,
            section_data,
            relocations,
            ..
        } = obj;
        for sec_idx in 0..sections.len() {
            let sec = &sections[sec_idx];
            if !sec.name.starts_with(".eh_frame") || sec.name.ends_with("_hdr") {
                continue;
            }
            let Some(data) = section_data.get(sec_idx) else {
                continue;
            };
            let records = scan_eh_frame_records(data);
            if records.is_empty() {
                continue;
            }
            let relocs = relocations.get(sec_idx).map(Vec::as_slice).unwrap_or(&[]);

            // Which FDEs describe a collected function?  (The first
            // relocation at each offset, as a linear `find` would pick, but
            // without its records x relocations cost on big C++ units.)
            let mut reloc_at: crate::common::fx_hash::FxHashMap<usize, usize> =
                crate::common::fx_hash::FxHashMap::default();
            for (ri, r) in relocs.iter().enumerate() {
                reloc_at.entry(r.offset as usize).or_insert(ri);
            }
            let mut prune: Vec<bool> = vec![false; records.len()];
            for (i, rec) in records.iter().enumerate() {
                let Some(iloc) = rec.iloc_offset else {
                    continue;
                };
                let Some(rela) = reloc_at.get(&iloc).map(|&ri| &relocs[ri]) else {
                    continue; // no relocation: cannot prove the target is dead
                };
                let Some(sym) = symbols.get(rela.sym_idx as usize) else {
                    continue;
                };
                if sym.shndx == SHN_UNDEF || sym.shndx == SHN_ABS || sym.shndx == SHN_COMMON {
                    continue; // not defined in a collectable input section
                }
                if dead.contains(&(obj_idx, sym.shndx as usize)) {
                    prune[i] = true;
                }
            }
            if !prune.iter().any(|&p| p) {
                continue;
            }

            let compacted = compact_eh_frame(data, &records, &prune);
            dropped += prune.iter().filter(|&&p| p).count();
            let mut new_relocs: Vec<_> = relocs
                .iter()
                .filter_map(|r| {
                    let offset = compacted.map_offset(r.offset as usize)?;
                    let mut r2 = r.clone();
                    r2.offset = offset as u64;
                    Some(r2)
                })
                .collect();
            new_relocs.sort_by_key(|r| r.offset);
            let out = compacted.data;

            // `SectionData` is immutable by design (it usually aliases the
            // mmap of the input), so the compacted section is materialised as
            // an owned buffer and the header size follows it.
            sections[sec_idx].size = out.len() as u64;
            section_data[sec_idx] = crate::backend::linker_common::SectionData::owned(out);
            if sec_idx < relocations.len() {
                relocations[sec_idx] = new_relocs;
            }
        }
    }
    dropped
}

/// An FDE whose CIE was merged into an identical one in an earlier (or the
/// same) `.eh_frame` input section; its `CIE_pointer` is written once the
/// output offsets of both sections are known ([`apply_cie_redirects`]).
#[derive(Debug, Clone, Copy)]
struct CieRedirect {
    /// The FDE's section and the offset of its `CIE_pointer` field there.
    obj: usize,
    sec: usize,
    field: usize,
    width: usize,
    /// The surviving CIE: section and offset of its length field.
    target: (usize, usize, usize),
}

/// Pending `CIE_pointer` rewrites from [`pack_eh_frame_sections`].
#[derive(Debug, Default)]
pub struct EhFramePacking {
    redirects: Vec<CieRedirect>,
    /// CIE records removed.
    pub merged: usize,
    /// Sections whose last record was lengthened to close the alignment gap.
    pub padded: usize,
}

/// Lengthen the record at `start` -- the last one of `buf` -- with
/// `DW_CFA_nop`s (zero bytes) until `buf.len()` is a multiple of `unit`.
/// Returns whether anything changed.
fn pad_last_record(buf: &mut Vec<u8>, start: usize, id_size: usize, unit: usize) -> bool {
    let pad = buf.len().next_multiple_of(unit) - buf.len();
    if pad == 0 {
        return false;
    }
    if id_size == 8 {
        let len = read_u64_le(buf, start + 4) + pad as u64;
        buf[start + 4..start + 12].copy_from_slice(&len.to_le_bytes());
    } else {
        let len = read_u32_le(buf, start) as usize + pad;
        // 0xffff_fff0.. are reserved (0xffff_ffff escapes to the 64-bit
        // format): never produce one; the gap then stays, as before.
        let Some(len) = u32::try_from(len).ok().filter(|&l| l < 0xffff_fff0) else {
            return false;
        };
        buf[start..start + 4].copy_from_slice(&len.to_le_bytes());
    }
    buf.resize(buf.len() + pad, 0);
    true
}

/// Identity of a relocation target inside a CIE (the personality pointer):
/// a global by name, anything local only within its own object.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum CieRelocTarget {
    Global(String),
    Local(usize, u32),
}

/// A CIE's identity: its bytes plus the relocations applied to them.
type CieKey = (Vec<u8>, Vec<(usize, u32, i64, CieRelocTarget)>);

/// The records of an unrelocated `.eh_frame` section, or `None` when they
/// do not tile it exactly (zero-length terminators aside).  Such a section
/// is opaque: nothing in it is merged, and none of its CIEs is offered to
/// later sections.
fn tiled_records(data: &[u8]) -> Option<Vec<EhFrameRecord>> {
    let records = scan_eh_frame_records(data);
    let mut pos = 0usize;
    let mut next = records.iter().peekable();
    while pos < data.len() {
        match next.peek() {
            Some(r) if r.start == pos => {
                pos = r.end;
                next.next();
            }
            _ if pos + 4 <= data.len() && read_u32_le(data, pos) == 0 => pos += 4,
            _ => return None,
        }
    }
    Some(records)
}

/// Prepare the link's `.eh_frame` input sections for concatenation:
/// merge identical CIEs across them and close the alignment gaps between
/// them.
///
/// Gaps: an input section is typically 8-aligned while its records only
/// sum to a multiple of 4 (crt1.o's is 0x2c bytes).  The zero bytes the
/// section merge would insert before the next input read as a zero-length
/// record, i.e. a terminator, in the middle of the output: libgcc's linear
/// FDE walk (`__register_frame_info`, the no-`.eh_frame_hdr` path) and
/// other consumers stop there and lose every later FDE.  So, as GNU ld
/// does, the last record of each section is lengthened with `DW_CFA_nop`
/// padding to a multiple of the strictest `.eh_frame` input alignment.
/// Sections that end in a terminator of their own (crtend.o's) or that
/// cannot be parsed are left alone.
///
/// CIEs:
/// Every translation unit carries its own CIE (usually the same `zR` one,
/// plus a `zPLR` one in C++), so a plain concatenation repeats a handful of
/// 24-to-32-byte records once per object -- GNU ld and lld keep one of
/// each.  The first occurrence in link order survives; later identical ones
/// are removed from their sections (compacted, relocations remapped, as for
/// pruned FDEs) and the FDEs that used them are re-aimed at the survivor.
/// Two CIEs are identical when their bytes AND the relocations inside them
/// (the personality routine) agree; a relocation against a local symbol
/// only matches within its own object.
///
/// The re-aimed `CIE_pointer`s cross input sections, so they are written
/// by [`apply_cie_redirects`] once the section merge has placed every
/// section.  That requires the merge to lay `.eh_frame` inputs out in the
/// order they are visited here (object, then section index) -- true of
/// `merge_sections_elf64*`, not of a linker script, whose path must not
/// call this.  Sections in `dead` are skipped.
pub fn pack_eh_frame_sections(
    objects: &mut [crate::backend::linker_common::Elf64Object],
    dead: &crate::common::fx_hash::FxHashSet<(usize, usize)>,
) -> EhFramePacking {
    use crate::backend::elf::{SHF_ALLOC, STB_LOCAL};
    let mut result = EhFramePacking::default();
    let mut canon: crate::common::fx_hash::FxHashMap<CieKey, (usize, usize, usize)> =
        crate::common::fx_hash::FxHashMap::default();
    let is_candidate = |obj_idx: usize,
                        sec_idx: usize,
                        sec: &crate::backend::linker_common::Elf64Section| {
        sec.name == ".eh_frame" && sec.flags & SHF_ALLOC != 0 && !dead.contains(&(obj_idx, sec_idx))
    };
    // The padding unit: the strictest input alignment, so every padded
    // section ends where the next one may start.  (Clamped: .eh_frame
    // fields are at most 8 bytes wide, and an absurd sh_addralign must not
    // turn into kilobytes of DW_CFA_nop.)
    let unit = objects
        .iter()
        .enumerate()
        .flat_map(|(o, obj)| {
            obj.sections
                .iter()
                .enumerate()
                .filter(move |&(s, sec)| is_candidate(o, s, sec))
                .map(|(_, sec)| sec.addralign)
        })
        .max()
        .unwrap_or(1)
        .clamp(1, 8) as usize;
    for (obj_idx, obj) in objects.iter_mut().enumerate() {
        let crate::backend::linker_common::Elf64Object {
            sections,
            symbols,
            section_data,
            relocations,
            ..
        } = obj;
        for sec_idx in 0..sections.len() {
            if !is_candidate(obj_idx, sec_idx, &sections[sec_idx]) {
                continue;
            }
            let Some(data) = section_data.get(sec_idx) else {
                continue;
            };
            let Some(records) = tiled_records(data) else {
                continue;
            };
            let relocs = relocations.get(sec_idx).map(Vec::as_slice).unwrap_or(&[]);
            let key_of = |rec: &EhFrameRecord| -> CieKey {
                let mut rs: Vec<(usize, u32, i64, CieRelocTarget)> = relocs
                    .iter()
                    .filter(|r| (r.offset as usize) >= rec.start && (r.offset as usize) < rec.end)
                    .map(|r| {
                        let target = match symbols.get(r.sym_idx as usize) {
                            Some(sym) if sym.binding() != STB_LOCAL && !sym.name.is_empty() => {
                                CieRelocTarget::Global(sym.name.to_string())
                            }
                            _ => CieRelocTarget::Local(obj_idx, r.sym_idx),
                        };
                        (r.offset as usize - rec.start, r.rela_type, r.addend, target)
                    })
                    .collect();
                rs.sort_by_key(|r| r.0);
                (data[rec.start..rec.end].to_vec(), rs)
            };
            // Which CIEs already exist?  (Keys of this section's own CIEs are
            // registered after its compaction, at their final offsets; an
            // identical CIE later in the SAME section merges into the first.)
            let mut drop = vec![false; records.len()];
            let mut target_of: Vec<Option<(usize, usize, usize)>> = vec![None; records.len()];
            let mut local_first: crate::common::fx_hash::FxHashMap<CieKey, usize> =
                crate::common::fx_hash::FxHashMap::default();
            let mut keys: Vec<Option<CieKey>> = vec![None; records.len()];
            for (i, rec) in records.iter().enumerate() {
                if rec.is_fde {
                    continue;
                }
                let key = key_of(rec);
                if let Some(&t) = canon.get(&key) {
                    drop[i] = true;
                    target_of[i] = Some(t);
                } else if let Some(&first) = local_first.get(&key) {
                    drop[i] = true;
                    target_of[i] = Some((obj_idx, sec_idx, first)); // record index, fixed below
                } else {
                    local_first.insert(key.clone(), i);
                    keys[i] = Some(key);
                }
            }
            if !drop.iter().any(|&d| d) {
                for (i, key) in keys.into_iter().enumerate() {
                    if let Some(key) = key {
                        canon.insert(key, (obj_idx, sec_idx, records[i].start));
                    }
                }
                // Untouched but for the tail padding, if the section ends
                // with a record (not with a terminator of its own).
                if let Some(last) = records.last().filter(|r| r.end == data.len()) {
                    let mut buf = data.to_vec();
                    if pad_last_record(&mut buf, last.start, last.id_size, unit) {
                        result.padded += 1;
                        sections[sec_idx].size = buf.len() as u64;
                        section_data[sec_idx] =
                            crate::backend::linker_common::SectionData::owned(buf);
                    }
                }
                continue;
            }
            let mut compacted = compact_records(data, &records, &drop);
            // Same-section targets were record indices; make them offsets.
            for t in target_of.iter_mut().flatten() {
                if (t.0, t.1) == (obj_idx, sec_idx) {
                    t.2 = compacted.new_start[t.2];
                }
            }
            for (i, key) in keys.into_iter().enumerate() {
                if let Some(key) = key {
                    canon.insert(key, (obj_idx, sec_idx, compacted.new_start[i]));
                }
            }
            // FDEs of a merged CIE.
            for (i, rec) in records.iter().enumerate() {
                if !rec.is_fde || compacted.new_start[i] == usize::MAX {
                    continue;
                }
                let id = if rec.id_size == 8 {
                    read_u64_le(data, rec.id_offset)
                } else {
                    u64::from(read_u32_le(data, rec.id_offset))
                };
                let cie_pos = (rec.id_offset as u64).wrapping_sub(id) as usize;
                let Some(cie) = cie_at(&records, cie_pos) else {
                    continue;
                };
                if let Some(target) = target_of[cie] {
                    result.redirects.push(CieRedirect {
                        obj: obj_idx,
                        sec: sec_idx,
                        field: compacted.new_start[i] + (rec.id_offset - rec.start),
                        width: rec.id_size,
                        target,
                    });
                }
            }
            result.merged += drop.iter().filter(|&&d| d).count();
            let mut buf = std::mem::take(&mut compacted.data);
            // Compaction keeps records only, so the last survivor ends the
            // section.
            if let Some(last) = (0..records.len())
                .rev()
                .find(|&i| compacted.new_start[i] != usize::MAX)
                && pad_last_record(
                    &mut buf,
                    compacted.new_start[last],
                    records[last].id_size,
                    unit,
                )
            {
                result.padded += 1;
            }
            let mut new_relocs: Vec<_> = relocs
                .iter()
                .filter_map(|r| {
                    let offset = compacted.map_offset(r.offset as usize)?;
                    let mut r2 = r.clone();
                    r2.offset = offset as u64;
                    Some(r2)
                })
                .collect();
            new_relocs.sort_by_key(|r| r.offset);
            sections[sec_idx].size = buf.len() as u64;
            section_data[sec_idx] = crate::backend::linker_common::SectionData::owned(buf);
            if sec_idx < relocations.len() {
                relocations[sec_idx] = new_relocs;
            }
        }
    }
    result
}

/// Write the `CIE_pointer`s [`pack_eh_frame_sections`] left pending, from the
/// sections' places in the merged output (`section_map`: input section to
/// output section and offset).  A pointer must lead backwards within one
/// output section; anything else means the merge did not preserve input
/// order, which is an internal error rather than a reason to emit a broken
/// unwind table.
pub fn apply_cie_redirects(
    objects: &mut [crate::backend::linker_common::Elf64Object],
    section_map: &crate::common::fx_hash::FxHashMap<(usize, usize), (usize, u64)>,
    dedup: &EhFramePacking,
) -> Result<(), String> {
    let mut patched: crate::common::fx_hash::FxHashMap<(usize, usize), Vec<u8>> =
        crate::common::fx_hash::FxHashMap::default();
    for r in &dedup.redirects {
        let placement = |obj: usize, sec: usize| section_map.get(&(obj, sec)).copied();
        let (Some((fde_out, fde_base)), Some((cie_out, cie_base))) =
            (placement(r.obj, r.sec), placement(r.target.0, r.target.1))
        else {
            return Err("internal error: merged .eh_frame CIE of an unplaced section".into());
        };
        let field = fde_base + r.field as u64;
        let cie = cie_base + r.target.2 as u64;
        if fde_out != cie_out || cie >= field {
            return Err(format!(
                "internal error: merged .eh_frame CIE at output offset {cie:#x} does not precede \
                 the FDE field at {field:#x}"
            ));
        }
        let value = field - cie;
        let buf = patched
            .entry((r.obj, r.sec))
            .or_insert_with(|| objects[r.obj].section_data[r.sec].to_vec());
        if r.width == 8 {
            buf[r.field..r.field + 8].copy_from_slice(&value.to_le_bytes());
        } else {
            let v = u32::try_from(value)
                .map_err(|_| format!("merged .eh_frame CIE pointer {value:#x} overflows"))?;
            buf[r.field..r.field + 4].copy_from_slice(&v.to_le_bytes());
        }
    }
    for ((obj, sec), buf) in patched {
        objects[obj].section_data[sec] = crate::backend::linker_common::SectionData::owned(buf);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal but *valid* .eh_frame: one CIE with the given FDE
    /// pointer encoding, followed by `n` FDEs that all point back to it.
    ///
    /// The CIE augmentation is `zR`, i.e. it carries an augmentation-data
    /// block whose single byte is the FDE encoding -- this is what
    /// `parse_cie_fde_encoding` has to recover, and it is the shape gcc and
    /// clang actually emit.
    fn synth_eh_frame(n: u32, fde_enc: u8) -> Vec<u8> {
        let mut out = Vec::new();

        // ---- CIE ----
        let mut cie = Vec::new();
        cie.extend_from_slice(&0u32.to_le_bytes()); // CIE_id == 0
        cie.push(1); // version
        cie.extend_from_slice(b"zR\0"); // augmentation
        cie.push(1); // code alignment (uleb)
        cie.push(0x78); // data alignment (sleb -8)
        cie.push(16); // return address register
        cie.push(1); // augmentation data length
        cie.push(fde_enc); // FDE pointer encoding
        while (cie.len() + 4) % 8 != 0 {
            cie.push(0); // DW_CFA_nop padding to 8-byte alignment
        }
        out.extend_from_slice(&(cie.len() as u32).to_le_bytes());
        out.extend_from_slice(&cie);

        // ---- FDEs ----
        for i in 0..n {
            let fde_start = out.len();
            let cie_ptr = (fde_start + 4) as u32; // distance back to CIE at 0
            let mut fde = Vec::new();
            fde.extend_from_slice(&cie_ptr.to_le_bytes());
            // initial_location, sdata4 pcrel: descending so we can also prove
            // the header table gets sorted.
            let loc = -((i as i32 + 1) * 0x100);
            fde.extend_from_slice(&loc.to_le_bytes());
            fde.extend_from_slice(&0x10u32.to_le_bytes()); // address_range
            fde.push(0); // augmentation data length
            while (fde.len() + 4) % 8 != 0 {
                fde.push(0);
            }
            out.extend_from_slice(&(fde.len() as u32).to_le_bytes());
            out.extend_from_slice(&fde);
        }
        out
    }

    const PCREL_SDATA4: u8 = 0x1b;
    const ABS_UDATA8: u8 = 0x04;

    /// Two CIEs with *different* FDE encodings, each followed by its own FDEs.
    /// This is what a real link looks like after merging .eh_frame from several
    /// objects (crt files and C++ code disagree on encodings), and it is the
    /// only shape that can distinguish a correctly-keyed CIE cache from one
    /// that just returns whatever it cached first.
    fn synth_two_cie_eh_frame() -> (Vec<u8>, usize, usize) {
        let a = synth_eh_frame(3, PCREL_SDATA4);
        let b = synth_eh_frame(2, ABS_UDATA8);
        let _split = a.len();
        let mut out = a;
        // Re-point the second block's FDEs at its own CIE, which now starts at
        // `split` rather than 0.
        let mut fixed = b.clone();
        let mut pos = 0usize;
        while pos + 4 <= fixed.len() {
            let len =
                u32::from_le_bytes([fixed[pos], fixed[pos + 1], fixed[pos + 2], fixed[pos + 3]])
                    as usize;
            if len == 0 || pos + 4 + len > fixed.len() {
                break;
            }
            let id_off = pos + 4;
            let id = u32::from_le_bytes([
                fixed[id_off],
                fixed[id_off + 1],
                fixed[id_off + 2],
                fixed[id_off + 3],
            ]);
            if id != 0 {
                // CIE_pointer is (position of this field) - (CIE position);
                // both shift by `split`, so the value is unchanged. Nothing to
                // do -- but assert the invariant we are relying on.
                debug_assert!(id as usize <= id_off);
            }
            pos += 4 + len;
        }
        out.append(&mut fixed);
        (out, 3, 2)
    }

    /// Independently decode every FDE's initial_location by walking the
    /// section and, for each FDE, parsing *its own* CIE from scratch -- no
    /// caching. This is the reference the cached implementation must match.
    fn expected_locations(data: &[u8], base_vaddr: u64) -> Vec<u64> {
        let mut out = Vec::new();
        let mut pos = 0usize;
        while pos + 4 <= data.len() {
            let len = u32::from_le_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]])
                as usize;
            if len == 0 {
                pos += 4;
                continue;
            }
            if pos + 4 + len > data.len() {
                break;
            }
            let id_off = pos + 4;
            let id = u32::from_le_bytes([
                data[id_off],
                data[id_off + 1],
                data[id_off + 2],
                data[id_off + 3],
            ]);
            if id != 0 {
                let cie_pos = id_off - id as usize;
                let enc = parse_cie_fde_encoding(data, cie_pos, true)
                    .expect("reference CIE must be valid");
                let iloc_off = id_off + 4;
                if let Some(v) =
                    decode_eh_pointer(data, iloc_off, enc, base_vaddr + iloc_off as u64, true)
                {
                    out.push(v);
                }
            }
            pos += 4 + len;
        }
        out
    }

    /// A CIE cache keyed incorrectly (or not keyed at all) silently applies one
    /// CIE's FDE encoding to another CIE's FDEs, which decodes garbage
    /// addresses into the .eh_frame_hdr search table -- the unwinder then jumps
    /// to a wrong or invalid FDE. Mutation-checked: replacing the `cie_pos`
    /// lookup with "return the first cached entry" must fail this test.
    /// A CIE cache keyed incorrectly (or not keyed at all) silently applies one
    /// CIE's FDE encoding to another CIE's FDEs, which decodes garbage
    /// addresses into the .eh_frame_hdr search table -- the unwinder then jumps
    /// to a wrong or invalid FDE. Mutation-checked: replacing the `cie_pos`
    /// lookup with "return the first cached entry" must fail this test.
    #[test]
    fn distinct_cies_keep_distinct_encodings() {
        let (data, n_a, n_b) = synth_two_cie_eh_frame();
        let fdes = parse_eh_frame_fdes(&data, 0x400000, true);
        assert_eq!(fdes.len(), n_a + n_b, "expected FDEs from both CIE groups");

        // Pin the exact decoded values, not merely that they differ.
        //
        // The two groups use different encodings (pcrel sdata4 vs absolute
        // udata8). Asserting only "all locations are distinct" is too weak: a
        // cache that returns the wrong CIE's encoding still yields distinct
        // (but wrong) numbers, so such a test passes under mutation. Compare
        // against the values produced by decoding each group with *its own*
        // CIE, which is what the reference unwinder does.
        let got: Vec<u64> = fdes.iter().map(|f| f.initial_location).collect();

        // Group A: pcrel sdata4, so location = pc_of_field + signed_offset.
        // Group B: absolute udata8, read straight out of the section.
        // Recompute both independently of the parser under test.
        // `parse_eh_frame_fdes` returns the table already sorted by
        // initial_location (the header needs that for binary search), so sort
        // the independent reference before comparing.
        let mut expect = expected_locations(&data, 0x400000);
        expect.sort_unstable();
        assert_eq!(
            got, expect,
            "decoded FDE locations differ from an independent decode; a \
                    CIE's encoding was applied to another CIE's FDEs"
        );
    }

    /// `count_eh_frame_fdes` is used to size the .eh_frame_hdr *before* the
    /// FDEs are parsed. If it ever disagrees with `parse_eh_frame_fdes`, the
    /// header is either truncated (the unwinder reads past the section and
    /// crashes) or over-sized. Pin them together across a range of counts.
    #[test]
    fn count_matches_parse_for_many_fde_counts() {
        for n in [0u32, 1, 2, 7, 64, 300] {
            let data = synth_eh_frame(n, PCREL_SDATA4);
            assert_eq!(
                count_eh_frame_fdes(&data),
                n as usize,
                "count_eh_frame_fdes disagrees at n={n}"
            );
            let fdes = parse_eh_frame_fdes(&data, 0x400000, true);
            assert_eq!(
                fdes.len(),
                n as usize,
                "parse_eh_frame_fdes disagrees at n={n}"
            );
        }
    }

    /// The CIE-encoding memoisation must be transparent: caching the encoding
    /// per `cie_pos` may not change which FDEs are found or where they point.
    /// Many FDEs share one CIE here, which is exactly the case the cache
    /// optimises (it took `build_eh_frame_hdr` from 5.4% of a link to noise).
    #[test]
    fn shared_cie_memoisation_is_transparent() {
        let data = synth_eh_frame(200, PCREL_SDATA4);
        let fdes = parse_eh_frame_fdes(&data, 0x400000, true);
        assert_eq!(fdes.len(), 200);
        // Every FDE must have decoded a distinct, correctly-decoded location.
        let mut seen = std::collections::HashSet::new();
        for f in &fdes {
            assert!(
                seen.insert(f.initial_location),
                "duplicate initial_location 0x{:x}: the cache returned a \
                     stale encoding for some FDE",
                f.initial_location
            );
        }
    }

    /// The header's binary-search table is only usable if it is sorted by
    /// initial_location -- the unwinder does a binary search over it. The
    /// synthetic input above is deliberately emitted in *descending* order.
    #[test]
    fn header_table_is_sorted_by_initial_location() {
        let data = synth_eh_frame(16, PCREL_SDATA4);
        let hdr = build_eh_frame_hdr(&data, 0x400000, 0x3f0000, true);
        assert!(!hdr.is_empty());
        let count = i32::from_le_bytes([hdr[8], hdr[9], hdr[10], hdr[11]]);
        assert_eq!(count, 16, "fde_count in header");
        // Header must be exactly 12 + 8*count bytes; a short section makes the
        // unwinder read past the end (observed as a SIGSEGV in __gxx_personality_v0).
        assert_eq!(hdr.len(), 12 + 8 * 16, "header size must match fde_count");
        let mut prev = i32::MIN;
        for i in 0..count as usize {
            let off = 12 + i * 8;
            let loc = i32::from_le_bytes([hdr[off], hdr[off + 1], hdr[off + 2], hdr[off + 3]]);
            assert!(loc >= prev, "table not sorted at entry {i}: {loc} < {prev}");
            prev = loc;
        }
    }

    /// `scan_eh_frame_records` must classify CIE vs FDE and report the byte
    /// offset of every FDE's `initial_location` slot -- that offset is the
    /// only thing tying an unrelocated FDE to the function it describes.
    #[test]
    fn scan_classifies_records_and_finds_initial_location() {
        let data = synth_eh_frame(4, PCREL_SDATA4);
        let recs = scan_eh_frame_records(&data);
        assert_eq!(recs.len(), 5, "1 CIE + 4 FDEs");
        assert!(
            !recs[0].is_fde && recs[0].iloc_offset.is_none(),
            "first is the CIE"
        );
        assert_eq!(recs[0].start, 0);
        for r in &recs[1..] {
            assert!(r.is_fde, "rest are FDEs");
            let iloc = r.iloc_offset.expect("FDE has initial_location");
            assert!(iloc > r.start && iloc + 4 <= r.end, "iloc inside record");
            assert!(r.end <= data.len());
        }
        // Records must tile the section without gaps or overlaps.
        for w in recs.windows(2) {
            assert_eq!(w[0].end, w[1].start, "records are contiguous");
        }
        assert_eq!(recs.last().unwrap().end, data.len());
    }

    /// FDEs may point back past an intervening CIE (GNU as: `zR` function,
    /// `zPLR` function with a cleanup, `zR` function; lccc's assembler puts
    /// every CIE first).  Compaction must re-aim each FDE at the CIE its
    /// pointer names, not at the nearest preceding one.
    #[test]
    fn compact_eh_frame_keeps_each_fde_on_its_own_cie() {
        fn cie(aug: &[u8]) -> Vec<u8> {
            let mut body = vec![0, 0, 0, 0, 1];
            body.extend_from_slice(aug);
            body.extend_from_slice(&[0, 1, 0x78, 16, 1, 0x1b]);
            while (body.len() + 4) % 8 != 0 {
                body.push(0);
            }
            let mut r = (body.len() as u32).to_le_bytes().to_vec();
            r.extend(body);
            r
        }
        fn fde(at: usize, cie_at: usize, tag: u8) -> Vec<u8> {
            let ptr = (at + 4 - cie_at) as u32;
            let mut body = ptr.to_le_bytes().to_vec();
            body.extend_from_slice(&[0, 0, 0, 0, tag, 0, 0, 0, 0, 0, 0, 0]);
            let mut r = (body.len() as u32).to_le_bytes().to_vec();
            r.extend(body);
            r
        }
        let mut data = cie(b"zR");
        let cie_a = 0;
        let fde1 = data.len();
        data.extend(fde(fde1, cie_a, 1));
        let cie_b = data.len();
        data.extend(cie(b"zPLR"));
        let fde2 = data.len();
        data.extend(fde(fde2, cie_b, 2));
        let fde3 = data.len();
        data.extend(fde(fde3, cie_a, 3));

        let recs = scan_eh_frame_records(&data);
        assert_eq!(recs.len(), 5);
        let prune = [false, true, false, false, false];
        let c = compact_eh_frame(&data, &recs, &prune);
        let out = &c.data;
        let recs2 = scan_eh_frame_records(out);
        assert_eq!(recs2.len(), 4, "FDE 1 is gone");
        let cie_of = |r: &EhFrameRecord| {
            let v = u32::from_le_bytes(out[r.id_offset..r.id_offset + 4].try_into().unwrap());
            r.id_offset - v as usize
        };
        let (a, b) = (recs2[0].start, recs2[1].start);
        assert_eq!(cie_of(&recs2[2]), b, "FDE 2 -> zPLR CIE");
        assert_eq!(cie_of(&recs2[3]), a, "FDE 3 -> zR CIE, not the nearer zPLR");
        // Relocation offsets move with their records; offsets in the pruned
        // record are dropped.
        assert_eq!(c.map_offset(fde1 + 8), None);
        assert_eq!(c.map_offset(fde3 + 8), Some(recs2[3].start + 8));
        assert_eq!(c.map_offset(cie_b + 3), Some(b + 3));
    }

    /// The core `--gc-sections` invariant: an FDE for a collected function is
    /// dropped, its relocation with it, and every surviving FDE is untouched.
    #[test]
    fn prune_dead_fdes_drops_only_the_collected_fde() {
        use crate::backend::linker_common::{Elf64Object, Elf64Rela, Elf64Section, SectionData};
        use crate::common::fx_hash::FxHashSet;

        let eh = synth_eh_frame(3, PCREL_SDATA4);
        let recs = scan_eh_frame_records(&eh);
        // Sections of the synthetic object: 0 NULL, 1..=3 code, 4 .eh_frame.
        let mut sections = vec![Elf64Section {
            name_idx: 0,
            name: String::new(),
            sh_type: 0,
            flags: 0,
            addr: 0,
            offset: 0,
            size: 0,
            link: 0,
            info: 0,
            addralign: 0,
            entsize: 0,
        }];
        for i in 1..=4u32 {
            sections.push(Elf64Section {
                name_idx: 0,
                name: if i == 4 {
                    ".eh_frame".into()
                } else {
                    format!(".text.f{i}")
                },
                sh_type: if i == 4 { 1 } else { 1 }, // SHT_PROGBITS
                flags: if i == 4 { 0x2 } else { 0x6 },
                addr: 0,
                offset: 0,
                size: eh.len() as u64,
                link: 0,
                info: 0,
                addralign: 8,
                entsize: 0,
            });
        }
        // One relocation per FDE, aimed at its own code section, plus a
        // relocation on the CIE that must never be dropped.
        let relocs: Vec<Elf64Rela> = recs
            .iter()
            .enumerate()
            .map(|(i, r)| Elf64Rela {
                offset: r.iloc_offset.unwrap_or(r.start + 4) as u64,
                sym_idx: i as u32, // sym i has shndx i
                rela_type: 2,
                addend: 0,
            })
            .collect();
        let mut symbols = Vec::new();
        for i in 0..5u16 {
            symbols.push(crate::backend::linker_common::Elf64Symbol {
                name_idx: 0,
                name: format!("s{i}").into(),
                info: 0x10,
                other: 0,
                shndx: i,
                value: 0,
                size: 0,
            });
        }
        let before = count_eh_frame_fdes(&eh);
        assert_eq!(before, 3);
        let mut objects = vec![Elf64Object {
            sections,
            symbols,
            section_data: vec![
                SectionData::empty(),
                SectionData::owned(vec![0u8; 16]),
                SectionData::owned(vec![0u8; 16]),
                SectionData::owned(vec![0u8; 16]),
                SectionData::owned(eh.clone()),
            ],
            relocations: vec![Vec::new(), Vec::new(), Vec::new(), Vec::new(), relocs],
            source_name: "<test>".into(),
        }];

        // Collect `.text.f2` (section 2 -> FDE index 1).
        let mut dead: FxHashSet<(usize, usize)> = FxHashSet::default();
        dead.insert((0, 2));

        let dropped = prune_dead_fdes(&mut objects, &dead);
        assert_eq!(dropped, 1, "exactly one FDE pruned");

        let after = count_eh_frame_fdes(&objects[0].section_data[4]);
        assert_eq!(after, 2, "two FDEs remain");
        assert!(
            objects[0].section_data[4].len() < eh.len(),
            "pruning compacts: the section shrinks"
        );
        assert_eq!(
            objects[0].sections[4].size as usize,
            objects[0].section_data[4].len(),
            "header size must follow the compacted bytes"
        );
        // Every surviving FDE must still resolve its CIE, i.e. the relative
        // CIE_pointer was rewritten rather than left pointing at the old
        // offset.  `parse_eh_frame_fdes` returns an FDE only when its CIE
        // decodes, so a full count proves the fixup.
        assert_eq!(
            parse_eh_frame_fdes(&objects[0].section_data[4], 0x40_0000, true).len(),
            2,
            "both surviving FDEs still decode against their CIE"
        );
        // A compacted section must remain a single contiguous record stream
        // with no zero-length terminator before the end.
        let recs_after = scan_eh_frame_records(&objects[0].section_data[4]);
        assert_eq!(recs_after.len(), 3, "CIE + 2 FDEs, no gaps");
        for w in recs_after.windows(2) {
            assert_eq!(w[0].end, w[1].start);
        }
        // 4 records (1 CIE + 3 FDEs) -> 4 relocations, minus the one inside
        // the pruned FDE.  The CIE's own relocation must survive.
        assert_eq!(
            objects[0].relocations[4].len(),
            3,
            "only the pruned FDE's relocation is removed"
        );
        // The surviving FDEs must still decode, and the pruned one must not
        // appear in the header table.
        let hdr = build_eh_frame_hdr(&objects[0].section_data[4], 0x40_0000, 0x3f_0000, true);
        assert_eq!(
            i32::from_le_bytes([hdr[8], hdr[9], hdr[10], hdr[11]]),
            2,
            "header table lists only surviving FDEs"
        );
    }

    /// An FDE with no relocation on `initial_location` (the shape an earlier
    /// `ld -r` leaves behind) cannot be proven dead, so it must survive.
    #[test]
    fn prune_dead_fdes_keeps_fdes_without_a_relocation() {
        use crate::backend::linker_common::{Elf64Object, Elf64Section, SectionData};
        use crate::common::fx_hash::FxHashSet;

        let eh = synth_eh_frame(2, PCREL_SDATA4);
        let mut dead: FxHashSet<(usize, usize)> = FxHashSet::default();
        dead.insert((0, 1));
        let mut objects = vec![Elf64Object {
            sections: vec![
                Elf64Section {
                    name_idx: 0,
                    name: ".eh_frame".into(),
                    sh_type: 1,
                    flags: 0x2,
                    addr: 0,
                    offset: 0,
                    size: eh.len() as u64,
                    link: 0,
                    info: 0,
                    addralign: 8,
                    entsize: 0,
                };
                1
            ],
            symbols: Vec::new(),
            section_data: vec![SectionData::owned(eh.clone())],
            relocations: vec![Vec::new()],
            source_name: "<test>".into(),
        }];
        assert_eq!(prune_dead_fdes(&mut objects, &dead), 0);
        assert_eq!(count_eh_frame_fdes(&objects[0].section_data[0]), 2);
    }

    /// A damaged CIE pointer must not turn a PC-relative FDE into an
    /// apparently valid *absolute* PC and poison the header's search table.
    #[test]
    fn invalid_cie_pointer_cannot_fabricate_a_header_entry() {
        let mut data = synth_eh_frame(1, PCREL_SDATA4);
        let id_off = scan_eh_frame_records(&data)[1].id_offset;
        data[id_off..id_off + 4].copy_from_slice(&1u32.to_le_bytes());
        assert!(parse_eh_frame_fdes(&data, 0x400000, true).is_empty());
        let hdr = build_eh_frame_hdr(&data, 0x400000, 0x3f0000, true);
        assert_eq!(&hdr[8..12], &0u32.to_le_bytes());
    }

    /// Extended lengths must be checked *before* converting/adding them.
    /// On 64-bit release builds this length wraps the end offset to 11:
    /// an FDE is counted even though its record is outside the input.
    #[test]
    fn oversized_extended_record_is_not_an_fde() {
        let mut data = vec![0u8; 32];
        data[..4].copy_from_slice(&u32::MAX.to_le_bytes());
        data[4..12].copy_from_slice(&u64::MAX.to_le_bytes());
        data[12..20].copy_from_slice(&1u64.to_le_bytes());
        assert_eq!(count_eh_frame_fdes(&data), 0);
        assert!(scan_eh_frame_records(&data).is_empty());
        assert!(parse_eh_frame_fdes(&data, 0x400000, true).is_empty());
    }

    /// Long or truncated LEBs from an object must not panic the linker or
    /// be interpreted as a truncated, apparently valid pointer/length.
    #[test]
    fn malformed_leb_does_not_panic() {
        for data in [vec![0x80; 11], vec![0xff; 10], vec![0x80; 1]] {
            assert_eq!(read_uleb128(&data, 0), None);
            assert_eq!(read_sleb128(&data, 0), None);
        }
        let mut unsigned_max = vec![0xff; 9];
        unsigned_max.push(1);
        assert_eq!(read_uleb128(&unsigned_max, 0), Some((u64::MAX, 10)));
        let mut signed_min = vec![0x80; 9];
        signed_min.push(0x7f);
        assert_eq!(read_sleb128(&signed_min, 0), Some((i64::MIN, 10)));
        assert_eq!(read_uleb128(&[0xff; 9], 0), None);
        assert_eq!(read_sleb128(&[0xff; 9], 0), None);
    }

    /// Build a DWARF v4 CIE/FDE pair.  Version 4 has two fields that v1/v3
    /// do not: address_size and segment_selector_size.  The parser must skip
    /// them before reading the alignment factors and the `zR` augmentation
    /// payload.
    fn synth_dwarf4_eh_frame(fde_enc: u8) -> Vec<u8> {
        let mut data = Vec::new();
        let mut cie = Vec::new();
        cie.extend_from_slice(&0u32.to_le_bytes()); // CIE id
        cie.push(4); // DWARF v4
        cie.extend_from_slice(b"zR\0");
        cie.push(8); // address_size for an ELF64 output
        cie.push(0); // segment_selector_size
        cie.push(1); // code alignment factor
        cie.push(0x78); // data alignment factor (-8)
        cie.push(16); // return-address register (ULEB128)
        cie.push(1); // augmentation data length
        cie.push(fde_enc);
        while (cie.len() + 4) % 8 != 0 {
            cie.push(0); // DW_CFA_nop padding
        }
        data.extend_from_slice(&(cie.len() as u32).to_le_bytes());
        data.extend_from_slice(&cie);

        let fde_start = data.len();
        let mut fde = Vec::new();
        fde.extend_from_slice(&((fde_start + 4) as u32).to_le_bytes());
        fde.extend_from_slice(&0x100i32.to_le_bytes()); // pcrel initial_location
        fde.extend_from_slice(&0x20u32.to_le_bytes()); // address_range
        fde.push(0); // z augmentation data length
        while (fde.len() + 4) % 8 != 0 {
            fde.push(0);
        }
        data.extend_from_slice(&(fde.len() as u32).to_le_bytes());
        data.extend_from_slice(&fde);
        data
    }

    #[test]
    fn pointer_decoding_preserves_unsigned_addresses_and_rejects_unsupported_bases() {
        // A 32-bit absolute pointer with bit 31 set is still an address, not
        // a negative signed displacement.
        assert_eq!(
            decode_eh_pointer(&0x8000_0000u32.to_le_bytes(), 0, 0x00, 0, false),
            Some(0x8000_0000)
        );
        // Likewise, udata8 is allowed to use the high half of the address
        // space; it must not pass through i64 and wrap.
        assert_eq!(
            decode_eh_pointer(&u64::MAX.to_le_bytes(), 0, 0x04, 0, true),
            Some(u64::MAX)
        );
        // Signed PC-relative offsets are applied without signed-overflow
        // traps, even when the PC is in the high canonical address range.
        assert_eq!(
            decode_eh_pointer(
                &(-0x100i32).to_le_bytes(),
                0,
                PCREL_SDATA4,
                0xffff_ffff_ffff_1000,
                true
            ),
            Some(0xffff_ffff_ffff_0f00)
        );
        // No text/data base is available to this section-only decoder.
        assert_eq!(
            decode_eh_pointer(&0u32.to_le_bytes(), 0, 0x3b, 0x400000, true),
            None
        );
        // Indirect encodings need a relocated image dereference and therefore
        // cannot be decoded from the section byte slice alone.
        assert_eq!(
            decode_eh_pointer(&0u32.to_le_bytes(), 0, 0x9b, 0x400000, true),
            None
        );

        // The fixed-width header fields must reject an unrepresentable
        // displacement rather than truncating it modulo 2^32.
        let data = synth_eh_frame(1, PCREL_SDATA4);
        assert!(build_eh_frame_hdr(&data, 0, 0x1_0000_0000, true).is_empty());
    }

    #[test]
    fn dwarf4_cie_skips_address_and_segment_sizes() {
        let data = synth_dwarf4_eh_frame(PCREL_SDATA4);
        assert_eq!(count_eh_frame_fdes(&data), 1);
        let fdes = parse_eh_frame_fdes(&data, 0x400000, true);
        assert_eq!(fdes.len(), 1);
        let fde_start = scan_eh_frame_records(&data)[1].start;
        let iloc = 0x400000 + (fde_start + 8) as u64 + 0x100;
        assert_eq!(fdes[0].initial_location, iloc);

        // A truncated v4 prefix and an unsupported DWARF version must fail
        // closed instead of borrowing the following record's bytes.
        let mut truncated = data.clone();
        let cie_len = u32::from_le_bytes(truncated[..4].try_into().unwrap()) as usize;
        truncated.truncate(4 + cie_len - 1);
        assert!(parse_eh_frame_fdes(&truncated, 0x400000, true).is_empty());
        let mut unknown = data;
        unknown[8] = 5; // version byte: length(4) + CIE id(4)
        assert!(parse_eh_frame_fdes(&unknown, 0x400000, true).is_empty());
    }

    #[test]
    fn extended_cie_fde_uses_its_own_encoding() {
        // DWARF64 .eh_frame, CIE v1 with zR augmentation and pcrel/sdata4.
        // Both structural scan and relocated decode must agree on the FDE.
        let mut data = Vec::new();
        let cie = [
            0u8, 0, 0, 0, 0, 0, 0, 0, // 8-byte CIE id
            1, b'z', b'R', 0, 1, 0x78, 16, 1, 0x1b,
        ];
        data.extend(u32::MAX.to_le_bytes());
        data.extend((cie.len() as u64).to_le_bytes());
        data.extend(cie);
        let fde_start = data.len();
        data.extend(u32::MAX.to_le_bytes());
        data.extend(16u64.to_le_bytes()); // 8-byte CIE pointer + 4 + 4
        data.extend(((fde_start + 12) as u64).to_le_bytes());
        data.extend(0x100i32.to_le_bytes()); // PC-relative initial location
        data.extend(4u32.to_le_bytes());
        assert_eq!(count_eh_frame_fdes(&data), 1);
        let records = scan_eh_frame_records(&data);
        assert_eq!(records.len(), 2);
        assert_eq!(records[1].iloc_offset, Some(fde_start + 20));
        for is_64bit in [true, false] {
            let fdes = parse_eh_frame_fdes(&data, 0x400000, is_64bit);
            assert_eq!(fdes.len(), 1);
            assert_eq!(
                fdes[0].initial_location,
                0x400000 + fde_start as u64 + 20 + 0x100
            );
        }
        // Truncate inside the FDE's location: it must not borrow bytes from
        // a later record even if the input slice contains them.
        let short = &data[..fde_start + 12];
        assert_eq!(count_eh_frame_fdes(short), 0);
        assert!(parse_eh_frame_fdes(short, 0x400000, true).is_empty());
    }

    /// Deterministic length/pointer fuzzing of the three independent EH
    /// walkers.  An invalid record cannot cause a backwards scan, fabricate
    /// an FDE, index another record, or trap on an oversized LEB.
    #[test]
    fn arbitrary_records_do_not_escape_their_bounds() {
        let mut seed = 0x9e3779b97f4a7c15u64;
        for case in 0..2048 {
            let n = case % 113;
            let mut data = vec![0; n];
            for byte in &mut data {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                *byte = seed as u8;
            }
            if case % 4 == 0 && n >= 12 {
                data[..4].copy_from_slice(&u32::MAX.to_le_bytes());
                data[4..12].copy_from_slice(&seed.to_le_bytes());
            }
            let records = scan_eh_frame_records(&data);
            assert_eq!(
                count_eh_frame_fdes(&data),
                records.iter().filter(|r| r.is_fde).count(),
                "case {case}"
            );
            let mut prev_end = 0;
            for r in records {
                assert!(r.start >= prev_end && r.end > r.start && r.end <= n);
                assert!(r.id_offset + r.id_size <= r.end);
                prev_end = r.end;
            }
            for is_64bit in [true, false] {
                let _ = build_eh_frame_hdr(&data, 0x400000, 0x3f0000, is_64bit);
            }
        }
    }

    /// Malformed input must terminate, not hang or panic. The zero-length and
    /// truncated cases are what a fuzzer hits first.
    #[test]
    fn malformed_input_terminates_without_panic() {
        for data in [
            vec![],
            vec![0u8; 3],                 // shorter than a length field
            vec![0xff, 0xff, 0xff, 0xff], // extended length, truncated
            vec![0x10, 0, 0, 0],          // length past end of buffer
            vec![0u8; 64],                // all zero terminators
        ] {
            let _ = count_eh_frame_fdes(&data);
            let _ = parse_eh_frame_fdes(&data, 0x400000, true);
            let _ = build_eh_frame_hdr(&data, 0x400000, 0x3f0000, true);
        }
    }

    /// A std-format record shorter than its own `CIE_id` field (length 1..3)
    /// is malformed: the id read would land in the *next* record's bytes
    /// and compaction's FDE rewrite would clobber them. The scan must stop
    /// instead of emitting a record it does not own.
    #[test]
    fn scan_rejects_std_record_shorter_than_cie_id() {
        for len in [1u32, 2, 3] {
            let mut data = len.to_le_bytes().to_vec();
            data.extend_from_slice(&[0xAA; 16]);
            let recs = scan_eh_frame_records(&data);
            assert!(
                recs.is_empty(),
                "length-{len} record must terminate the scan"
            );
            assert_eq!(count_eh_frame_fdes(&data), 0, "length-{len} count");
        }
    }

    /// An extended-format record narrower than its 8-byte `CIE_id` reads
    /// and rewrites past its own end. Same law as the std form.
    #[test]
    fn scan_rejects_extended_record_shorter_than_cie_id() {
        for len in [0u64, 1, 4, 7] {
            let mut data = 0xFFFF_FFFFu32.to_le_bytes().to_vec();
            data.extend_from_slice(&len.to_le_bytes());
            data.extend_from_slice(&[0xBB; 16]);
            let recs = scan_eh_frame_records(&data);
            assert!(recs.is_empty(), "extended length-{len} must stop the scan");
            assert_eq!(count_eh_frame_fdes(&data), 0);
        }
    }

    /// A hostile `0xFFFFFFFFFFFFFFFF` extended length must not panic the
    /// linker (debug addition overflow) nor wrap into an out-of-bounds
    /// read (release). `.eh_frame` comes from arbitrary object files.
    #[test]
    fn scan_rejects_huge_extended_length_without_panic() {
        let mut data = 0xFFFF_FFFFu32.to_le_bytes().to_vec();
        data.extend_from_slice(&u64::MAX.to_le_bytes());
        data.extend_from_slice(&[0xCC; 32]);
        assert!(scan_eh_frame_records(&data).is_empty());
        assert_eq!(count_eh_frame_fdes(&data), 0);
        // `build_eh_frame_hdr` walks the same bytes during layout: a
        // header with an empty table, not a panic.
        let hdr = build_eh_frame_hdr(&data, 0x400000, 0x3f0000, true);
        assert_eq!(hdr.len(), 12);
        assert_eq!(i32::from_le_bytes([hdr[8], hdr[9], hdr[10], hdr[11]]), 0);
    }

    /// Truncated input (length past the section end) terminates every walk.
    #[test]
    fn scan_rejects_truncated_record() {
        let mut data = 64u32.to_le_bytes().to_vec();
        data.extend_from_slice(&[0xDD; 8]);
        assert!(scan_eh_frame_records(&data).is_empty());
        assert_eq!(count_eh_frame_fdes(&data), 0);
    }

    /// An FDE whose `CIE_pointer` aims *forward* at a later CIE is
    /// malformed (DWARF pointers always run backwards), but it must not
    /// crash compaction: `field_pos - new_start[cie]` underflows. The
    /// rewrite is skipped and the copied bytes are left untouched.
    ///
    /// The pointer needs its 8-byte form to reach the subtraction: a
    /// 32-bit forward pointer wraps `cie_pos` into unmapped `u64` space
    /// and takes the dangling-pointer exit first. A pruned FDE between
    /// the two records makes the test non-vacuous in release builds too
    /// (without the guard the rebased pointer differs from the input;
    /// with it the input bytes survive verbatim).
    #[test]
    fn compact_skips_fde_aimed_at_later_cie() {
        // Extended FDE at 0: body 24 bytes, 8-byte CIE_pointer forward
        // past the dead FDE to the CIE at 52.
        let ptr = 12u64.wrapping_sub(52);
        let mut data = 0xFFFF_FFFFu32.to_le_bytes().to_vec();
        data.extend_from_slice(&24u64.to_le_bytes());
        data.extend_from_slice(&ptr.to_le_bytes());
        data.extend_from_slice(&[0x11; 16]);
        // Dead std FDE at 36 (pruned): length 12, dangling id.
        assert_eq!(data.len(), 36);
        data.extend_from_slice(&12u32.to_le_bytes());
        data.extend_from_slice(&1u32.to_le_bytes());
        data.extend_from_slice(&[0x33; 8]);
        // CIE at 52: length 8, CIE_id 0.
        assert_eq!(data.len(), 52);
        data.extend_from_slice(&8u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&[0x22; 4]);

        let recs = scan_eh_frame_records(&data);
        assert_eq!(recs.len(), 3);
        assert!(recs[0].is_fde && recs[1].is_fde && !recs[2].is_fde);
        let prune = [false, true, false];
        let c = compact_eh_frame(&data, &recs, &prune);
        // The forward pointer survives byte-identical (no wild rewrite),
        // and the surviving records compact around the pruned one.
        assert_eq!(&c.data[..36], &data[..36]);
        assert_eq!(
            u64::from_le_bytes(c.data[12..20].try_into().unwrap()),
            ptr,
            "forward CIE_pointer left untouched"
        );
        assert_eq!(&c.data[36..48], &data[52..64]);
    }

    /// An object whose `.eh_frame` (section 1) is `data`, with one PC32
    /// relocation per FDE `initial_location` against `.text` (section 2)
    /// and, in `cie_relocs`, relocations inside the CIE against the named
    /// global (a personality pointer).
    fn eh_object(
        data: Vec<u8>,
        cie_reloc_target: Option<&str>,
    ) -> crate::backend::linker_common::Elf64Object {
        use crate::backend::linker_common::{Elf64Rela, Elf64Section, Elf64Symbol, SectionData};
        let sec = |name: &str, size: u64| Elf64Section {
            name_idx: 0,
            name: name.to_string(),
            sh_type: 1,
            flags: crate::backend::elf::SHF_ALLOC,
            addr: 0,
            offset: 0,
            size,
            link: 0,
            info: 0,
            addralign: 8,
            entsize: 0,
        };
        let mut relocs: Vec<Elf64Rela> = scan_eh_frame_records(&data)
            .iter()
            .filter_map(|r| r.iloc_offset)
            .map(|off| Elf64Rela {
                offset: off as u64,
                sym_idx: 1,
                rela_type: 2,
                addend: 0,
            })
            .collect();
        if cie_reloc_target.is_some() {
            relocs.push(Elf64Rela {
                offset: 12,
                sym_idx: 2,
                rela_type: 2,
                addend: 0,
            });
            relocs.sort_by_key(|r| r.offset);
        }
        let sym = |name: &str, info: u8, shndx: u16| Elf64Symbol {
            name_idx: 0,
            name: name.into(),
            info,
            other: 0,
            shndx,
            value: 0,
            size: 0,
        };
        crate::backend::linker_common::Elf64Object {
            sections: vec![
                sec("", 0),
                sec(".eh_frame", data.len() as u64),
                sec(".text", 64),
            ],
            symbols: vec![
                sym("", 0, 0),
                sym("", crate::backend::elf::STT_SECTION, 2),
                sym(cie_reloc_target.unwrap_or("unused"), 0x10, 0),
            ],
            section_data: vec![
                SectionData::owned(Vec::new()),
                SectionData::owned(data),
                SectionData::owned(vec![0; 64]),
            ],
            relocations: vec![Vec::new(), relocs, Vec::new()],
            source_name: "t.o".into(),
        }
    }

    /// Lay `.eh_frame` of every object out back to back (as the merge
    /// does), apply the redirects and return the concatenation.
    fn merged_eh_frame(
        objs: &mut [crate::backend::linker_common::Elf64Object],
        d: &EhFramePacking,
    ) -> Vec<u8> {
        let mut map = crate::common::fx_hash::FxHashMap::default();
        let mut off = 0u64;
        for (i, o) in objs.iter().enumerate() {
            map.insert((i, 1), (0usize, off));
            off += o.sections[1].size;
        }
        apply_cie_redirects(objs, &map, d).expect("redirects apply");
        objs.iter()
            .flat_map(|o| o.section_data[1].to_vec())
            .collect()
    }

    /// Every FDE's `CIE_pointer` must land on a CIE of the merged data.
    fn fde_cies(data: &[u8]) -> Vec<usize> {
        let recs = scan_eh_frame_records(data);
        recs.iter()
            .filter(|r| r.is_fde)
            .map(|r| {
                let cie = r.id_offset - read_u32_le(data, r.id_offset) as usize;
                assert!(
                    cie_at(&recs, cie).is_some(),
                    "FDE at {} aims at {cie}, not a CIE",
                    r.start
                );
                cie
            })
            .collect()
    }

    #[test]
    fn identical_cies_across_objects_merge() {
        let unit = synth_eh_frame(2, PCREL_SDATA4);
        let cie_len = scan_eh_frame_records(&unit)[0].end;
        let mut objs = vec![
            eh_object(unit.clone(), None),
            eh_object(unit.clone(), None),
            eh_object(unit.clone(), None),
        ];
        let d = pack_eh_frame_sections(&mut objs, &Default::default());
        assert_eq!(d.merged, 2);
        assert_eq!(objs[1].sections[1].size as usize, unit.len() - cie_len);
        // The FDE relocations moved with their records.
        let first_fde_iloc = scan_eh_frame_records(&unit)[1].iloc_offset.unwrap();
        assert_eq!(
            objs[1].relocations[1][0].offset as usize,
            first_fde_iloc - cie_len
        );
        let merged = merged_eh_frame(&mut objs, &d);
        assert_eq!(merged.len(), 3 * unit.len() - 2 * cie_len);
        let cies = fde_cies(&merged);
        assert_eq!(cies, vec![0; 6], "all six FDEs share the first CIE");
        assert_eq!(
            scan_eh_frame_records(&merged)
                .iter()
                .filter(|r| !r.is_fde)
                .count(),
            1
        );
    }

    #[test]
    fn cies_with_different_encodings_or_personalities_stay() {
        let a = synth_eh_frame(1, PCREL_SDATA4);
        let b = synth_eh_frame(1, ABS_UDATA8);
        let mut objs = vec![
            eh_object(a.clone(), None),
            eh_object(b.clone(), None),
            eh_object(a.clone(), Some("__gxx_personality_v0")),
            eh_object(a.clone(), Some("__gcc_personality_v0")),
            eh_object(a.clone(), Some("__gxx_personality_v0")),
        ];
        let d = pack_eh_frame_sections(&mut objs, &Default::default());
        // Only the last merges: same bytes, same personality as the third.
        assert_eq!(d.merged, 1);
        let merged = merged_eh_frame(&mut objs, &d);
        let cies = fde_cies(&merged);
        let starts: Vec<usize> = {
            let mut o = 0;
            objs.iter()
                .map(|x| {
                    let s = o;
                    o += x.sections[1].size as usize;
                    s
                })
                .collect()
        };
        assert_eq!(
            cies,
            vec![starts[0], starts[1], starts[2], starts[3], starts[2]]
        );
    }

    #[test]
    fn duplicate_cie_within_one_section_merges() {
        let unit = synth_eh_frame(1, PCREL_SDATA4);
        let mut twice = unit.clone();
        twice.extend_from_slice(&unit); // [CIE FDE CIE FDE], the second FDE aims at the second CIE
        let mut objs = vec![eh_object(twice.clone(), None)];
        let d = pack_eh_frame_sections(&mut objs, &Default::default());
        assert_eq!(d.merged, 1);
        let merged = merged_eh_frame(&mut objs, &d);
        assert_eq!(fde_cies(&merged), vec![0, 0]);
    }

    #[test]
    fn opaque_and_dead_sections_are_left_alone() {
        let unit = synth_eh_frame(1, PCREL_SDATA4);
        let mut junk = unit.clone();
        junk.extend_from_slice(&[1, 2, 3]); // does not tile
        let mut objs = vec![
            eh_object(unit.clone(), None),
            eh_object(junk.clone(), None),
            eh_object(unit.clone(), None),
        ];
        let mut dead = crate::common::fx_hash::FxHashSet::default();
        dead.insert((2usize, 1usize));
        let d = pack_eh_frame_sections(&mut objs, &dead);
        assert_eq!(d.merged, 0);
        assert_eq!(&*objs[1].section_data[1], &junk[..]);
        assert_eq!(&*objs[2].section_data[1], &unit[..]);
    }

    #[test]
    fn sections_are_padded_to_the_strictest_alignment() {
        // crt1.o's shape: CIE (0x18) + FDE (0x14) = 0x2c bytes, 8-aligned.
        let full = synth_eh_frame(1, PCREL_SDATA4);
        let recs = scan_eh_frame_records(&full);
        let mut odd = full[..recs[1].end].to_vec();
        let fde_len = odd.len() - recs[1].start - 4;
        if odd.len() % 8 == 0 {
            // Make it 4 mod 8: drop 4 bytes of the FDE's (nop) instructions.
            odd.truncate(odd.len() - 4);
            let l = (fde_len - 4) as u32;
            odd[recs[1].start..recs[1].start + 4].copy_from_slice(&l.to_le_bytes());
        }
        assert_eq!(odd.len() % 8, 4);
        let mut term_only = eh_object(vec![0; 4], None);
        term_only.sections[1].addralign = 4;
        let mut objs = vec![
            eh_object(odd.clone(), None),
            eh_object(full.clone(), None),
            term_only,
        ];
        let d = pack_eh_frame_sections(&mut objs, &Default::default());
        assert_eq!(d.padded, 1);
        assert_eq!(objs[0].sections[1].size as usize, odd.len() + 4);
        assert_eq!(
            objs[2].sections[1].size, 4,
            "a bare terminator is not padded"
        );
        let merged = merged_eh_frame(&mut objs, &d);
        // Back to back, the only zero-length record is the final one.
        let recs = scan_eh_frame_records(&merged);
        assert_eq!(recs.last().unwrap().end, merged.len() - 4);
        assert!(
            recs.windows(2).all(|w| w[0].end == w[1].start),
            "no gap between records"
        );
        assert_eq!(
            fde_cies(&merged),
            vec![0, 0],
            "the second CIE merged, the padded FDE intact"
        );
    }

    #[test]
    fn compaction_adds_no_terminator() {
        let data = synth_eh_frame(3, PCREL_SDATA4);
        let recs = scan_eh_frame_records(&data);
        let prune: Vec<bool> = recs.iter().enumerate().map(|(i, _)| i == 2).collect();
        let c = compact_eh_frame(&data, &recs, &prune);
        assert_eq!(c.data.len(), data.len() - (recs[2].end - recs[2].start));
        assert_eq!(scan_eh_frame_records(&c.data).len(), 3);
    }
}
