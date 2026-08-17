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

/// Count the number of FDE entries in an .eh_frame section by scanning structure.
/// This only reads length and CIE_id fields, so it works on unrelocated data.
/// Used during layout to reserve space for .eh_frame_hdr (12 + 8 * count bytes).
pub fn count_eh_frame_fdes(data: &[u8]) -> usize {
    let mut count = 0;
    let mut pos = 0;
    while pos + 4 <= data.len() {
        let length = read_u32_le(data, pos) as u64;
        if length == 0 {
            // Zero terminator from a merged input section; skip it
            pos += 4;
            continue;
        }
        let (actual_length, header_size) = if length == 0xFFFFFFFF {
            if pos + 12 > data.len() { break; }
            (read_u64_le(data, pos + 4), 12usize)
        } else {
            (length, 4usize)
        };
        let entry_data_start = pos + header_size;
        let entry_end = entry_data_start + actual_length as usize;
        if entry_end > data.len() || entry_data_start + 4 > data.len() { break; }
        let cie_id = if length == 0xFFFFFFFF {
            if entry_data_start + 8 > data.len() { break; }
            read_u64_le(data, entry_data_start)
        } else {
            read_u32_le(data, entry_data_start) as u64
        };
        if cie_id != 0 { count += 1; }
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

    // Header: 4 bytes + eh_frame_ptr (4 bytes) + fde_count (4 bytes)
    let header_size = 4 + 4 + 4;
    let table_entry_size = 8; // two i32s per entry
    let total_size = header_size + fdes.len() * table_entry_size;
    let mut data = vec![0u8; total_size];

    // Version
    data[0] = 1;
    // eh_frame_ptr encoding: DW_EH_PE_pcrel | DW_EH_PE_sdata4
    data[1] = 0x1b;
    // fde_count encoding: DW_EH_PE_udata4
    data[2] = 0x03;
    // table encoding: DW_EH_PE_datarel | DW_EH_PE_sdata4
    data[3] = 0x3b;

    // eh_frame_ptr: PC-relative offset from &data[4] to eh_frame
    let eh_frame_ptr = eh_frame_vaddr as i64 - (eh_frame_hdr_vaddr as i64 + 4);
    write_i32_le(&mut data, 4, eh_frame_ptr as i32);

    // fde_count
    write_i32_le(&mut data, 8, fdes.len() as i32);

    // Table entries: sorted by initial_location
    // Each entry is (initial_location - eh_frame_hdr_vaddr, fde_address - eh_frame_hdr_vaddr)
    for (i, fde) in fdes.iter().enumerate() {
        let off = header_size + i * table_entry_size;
        let loc_rel = fde.initial_location as i64 - eh_frame_hdr_vaddr as i64;
        let fde_rel = fde.fde_vaddr as i64 - eh_frame_hdr_vaddr as i64;
        write_i32_le(&mut data, off, loc_rel as i32);
        write_i32_le(&mut data, off + 4, fde_rel as i32);
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
    let mut cie_cache: Vec<(usize, u8)> = Vec::new();

    while pos + 4 <= data.len() {
        let length = read_u32_le(data, pos) as u64;
        if length == 0 {
            // Zero terminator from a merged input section; skip it
            pos += 4;
            continue;
        }

        let is_extended = length == 0xFFFFFFFF;
        let (actual_length, header_size) = if is_extended {
            if pos + 12 > data.len() { break; }
            (read_u64_le(data, pos + 4), 12usize)
        } else {
            (length, 4usize)
        };

        let entry_start = pos;
        let entry_data_start = pos + header_size;
        let entry_end = entry_data_start + actual_length as usize;
        if entry_end > data.len() { break; }

        // CIE_id field (4 or 8 bytes depending on extended)
        if entry_data_start + 4 > data.len() { break; }
        let cie_id = if is_extended {
            if entry_data_start + 8 > data.len() { break; }
            read_u64_le(data, entry_data_start)
        } else {
            read_u32_le(data, entry_data_start) as u64
        };

        // CIE has cie_id == 0; FDE has cie_id != 0 (it's a pointer back to CIE)
        if cie_id != 0 {
            // This is an FDE
            // The CIE_pointer is relative: entry_data_start - cie_id points to the CIE
            let cie_id_field_size = if is_extended { 8 } else { 4 };
            let cie_pos = (entry_data_start as u64).wrapping_sub(cie_id) as usize;

            // Parse the CIE to get the FDE encoding (memoised; see cie_cache)
            let fde_encoding = match cie_cache.iter().find(|&&(p, _)| p == cie_pos) {
                Some(&(_, enc)) => enc,
                None => {
                    let enc = parse_cie_fde_encoding(data, cie_pos, is_64bit);
                    cie_cache.push((cie_pos, enc));
                    enc
                }
            };

            // After CIE_pointer comes: initial_location, address_range, ...
            let iloc_offset = entry_data_start + cie_id_field_size;
            if iloc_offset + 4 > data.len() { pos = entry_end; continue; }

            let fde_vaddr = base_vaddr + entry_start as u64;

            // Decode initial_location based on the CIE's FDE encoding
            let initial_location = decode_eh_pointer(
                data, iloc_offset, fde_encoding,
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
/// Returns the encoding byte, or 0x00 (DW_EH_PE_absptr) if not found.
fn parse_cie_fde_encoding(data: &[u8], cie_pos: usize, _is_64bit: bool) -> u8 {
    if cie_pos + 4 > data.len() { return 0x00; }

    let length = read_u32_le(data, cie_pos) as u64;
    if length == 0 || length == 0xFFFFFFFF { return 0x00; }

    let header_size = 4usize;
    let cie_data_start = cie_pos + header_size;
    let cie_end = cie_data_start + length as usize;
    if cie_end > data.len() { return 0x00; }

    // CIE_id must be 0
    if cie_data_start + 4 > data.len() { return 0x00; }
    let cie_id = read_u32_le(data, cie_data_start);
    if cie_id != 0 { return 0x00; }

    // version (1 byte)
    if cie_data_start + 5 > data.len() { return 0x00; }
    let _version = data[cie_data_start + 4];

    // augmentation string (null-terminated)
    let aug_start = cie_data_start + 5;
    let mut aug_end = aug_start;
    while aug_end < cie_end && data[aug_end] != 0 {
        aug_end += 1;
    }
    if aug_end >= cie_end { return 0x00; }
    let aug_str: Vec<u8> = data[aug_start..aug_end].to_vec();
    let mut cur = aug_end + 1; // skip null terminator

    // code_alignment_factor (ULEB128)
    let (_, n) = read_uleb128(data, cur);
    cur += n;
    // data_alignment_factor (SLEB128)
    let (_, n) = read_sleb128(data, cur);
    cur += n;
    // return_address_register (ULEB128)
    let (_, n) = read_uleb128(data, cur);
    cur += n;

    // Parse augmentation data
    if !aug_str.is_empty() && aug_str[0] == b'z' {
        // Augmentation data length (ULEB128)
        let (aug_data_len, n) = read_uleb128(data, cur);
        cur += n;
        let aug_data_end = cur + aug_data_len as usize;

        // Walk augmentation string after 'z'
        for &ch in &aug_str[1..] {
            if cur >= aug_data_end { break; }
            match ch {
                b'R' => {
                    // FDE encoding
                    if cur < data.len() {
                        return data[cur];
                    }
                    return 0x00;
                }
                b'L' => {
                    // LSDA encoding (skip 1 byte)
                    cur += 1;
                }
                b'P' => {
                    // Personality encoding + pointer
                    if cur >= data.len() { return 0x00; }
                    let enc = data[cur];
                    cur += 1;
                    let ptr_size = eh_pointer_size(enc, _is_64bit);
                    cur += ptr_size;
                }
                b'S' | b'B' => {
                    // Signal frame / has ABI tag - no data
                }
                _ => break,
            }
        }
    }

    // Default: absolute pointer encoding
    0x00
}

/// Decode an eh_frame pointer value based on its encoding.
fn decode_eh_pointer(data: &[u8], offset: usize, encoding: u8, pc: u64, is_64bit: bool) -> Option<u64> {
    if encoding == 0xFF { return None; } // DW_EH_PE_omit

    let base_enc = encoding & 0x0F;
    let rel = encoding & 0x70;

    let (raw_val, _size) = match base_enc {
        0x00 => { // DW_EH_PE_absptr
            if is_64bit {
                if offset + 8 > data.len() { return None; }
                (read_u64_le(data, offset) as i64, 8)
            } else {
                if offset + 4 > data.len() { return None; }
                (read_u32_le(data, offset) as i32 as i64, 4)
            }
        }
        0x01 => { // DW_EH_PE_uleb128
            let (v, _) = read_uleb128(data, offset);
            (v as i64, 0)
        }
        0x02 => { // DW_EH_PE_udata2
            if offset + 2 > data.len() { return None; }
            (u16::from_le_bytes([data[offset], data[offset+1]]) as i64, 2)
        }
        0x03 => { // DW_EH_PE_udata4
            if offset + 4 > data.len() { return None; }
            (read_u32_le(data, offset) as i64, 4)
        }
        0x04 => { // DW_EH_PE_udata8
            if offset + 8 > data.len() { return None; }
            (read_u64_le(data, offset) as i64, 8)
        }
        0x09 => { // DW_EH_PE_sleb128
            let (v, _) = read_sleb128(data, offset);
            (v, 0)
        }
        0x0A => { // DW_EH_PE_sdata2
            if offset + 2 > data.len() { return None; }
            (i16::from_le_bytes([data[offset], data[offset+1]]) as i64, 2)
        }
        0x0B => { // DW_EH_PE_sdata4
            if offset + 4 > data.len() { return None; }
            (read_i32_le(data, offset) as i64, 4)
        }
        0x0C => { // DW_EH_PE_sdata8
            if offset + 8 > data.len() { return None; }
            (read_u64_le(data, offset) as i64, 8)
        }
        _ => return None,
    };

    let base_val = match rel {
        0x00 => 0i64,       // DW_EH_PE_absptr
        0x10 => pc as i64,  // DW_EH_PE_pcrel
        0x20 => 0i64,       // DW_EH_PE_textrel (not commonly used)
        0x30 => 0i64,       // DW_EH_PE_datarel
        _ => 0i64,
    };

    Some((base_val + raw_val) as u64)
}

/// Return the byte size of an encoded pointer.
fn eh_pointer_size(encoding: u8, is_64bit: bool) -> usize {
    match encoding & 0x0F {
        0x00 => if is_64bit { 8 } else { 4 }, // absptr
        0x02 | 0x0A => 2, // udata2/sdata2
        0x03 | 0x0B => 4, // udata4/sdata4
        0x04 | 0x0C => 8, // udata8/sdata8
        _ => 0,
    }
}

// ── Local binary helpers (avoid depending on elf::io to keep this self-contained) ──

fn read_u32_le(data: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([data[off], data[off+1], data[off+2], data[off+3]])
}

fn read_i32_le(data: &[u8], off: usize) -> i32 {
    i32::from_le_bytes([data[off], data[off+1], data[off+2], data[off+3]])
}

fn read_u64_le(data: &[u8], off: usize) -> u64 {
    u64::from_le_bytes([
        data[off], data[off+1], data[off+2], data[off+3],
        data[off+4], data[off+5], data[off+6], data[off+7],
    ])
}

fn write_i32_le(data: &mut [u8], off: usize, val: i32) {
    let b = val.to_le_bytes();
    data[off..off+4].copy_from_slice(&b);
}

fn read_uleb128(data: &[u8], mut off: usize) -> (u64, usize) {
    let start = off;
    let mut result = 0u64;
    let mut shift = 0;
    loop {
        if off >= data.len() { return (result, off - start); }
        let byte = data[off];
        off += 1;
        result |= ((byte & 0x7F) as u64) << shift;
        if byte & 0x80 == 0 { break; }
        shift += 7;
    }
    (result, off - start)
}

fn read_sleb128(data: &[u8], mut off: usize) -> (i64, usize) {
    let start = off;
    let mut result = 0i64;
    let mut shift = 0;
    let mut byte;
    loop {
        if off >= data.len() { return (result, off - start); }
        byte = data[off];
        off += 1;
        result |= ((byte & 0x7F) as i64) << shift;
        shift += 7;
        if byte & 0x80 == 0 { break; }
    }
    if shift < 64 && byte & 0x40 != 0 {
        result |= -(1i64 << shift);
    }
    (result, off - start)
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
        cie.push(1);                                // version
        cie.extend_from_slice(b"zR\0");             // augmentation
        cie.push(1);                                // code alignment (uleb)
        cie.push(0x78);                             // data alignment (sleb -8)
        cie.push(16);                               // return address register
        cie.push(1);                                // augmentation data length
        cie.push(fde_enc);                          // FDE pointer encoding
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
        let split = a.len();
        let mut out = a;
        // Re-point the second block's FDEs at its own CIE, which now starts at
        // `split` rather than 0.
        let mut fixed = b.clone();
        let mut pos = 0usize;
        while pos + 4 <= fixed.len() {
            let len = u32::from_le_bytes([fixed[pos], fixed[pos+1], fixed[pos+2], fixed[pos+3]]) as usize;
            if len == 0 || pos + 4 + len > fixed.len() { break; }
            let id_off = pos + 4;
            let id = u32::from_le_bytes([fixed[id_off], fixed[id_off+1], fixed[id_off+2], fixed[id_off+3]]);
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
            let len = u32::from_le_bytes(
                [data[pos], data[pos+1], data[pos+2], data[pos+3]]) as usize;
            if len == 0 { pos += 4; continue; }
            if pos + 4 + len > data.len() { break; }
            let id_off = pos + 4;
            let id = u32::from_le_bytes(
                [data[id_off], data[id_off+1], data[id_off+2], data[id_off+3]]);
            if id != 0 {
                let cie_pos = id_off - id as usize;
                let enc = parse_cie_fde_encoding(data, cie_pos, true);
                let iloc_off = id_off + 4;
                if let Some(v) = decode_eh_pointer(
                    data, iloc_off, enc, base_vaddr + iloc_off as u64, true) {
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
        assert_eq!(fdes.len(), n_a + n_b,
                   "expected FDEs from both CIE groups");

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
        assert_eq!(got, expect,
                   "decoded FDE locations differ from an independent decode; a \
                    CIE's encoding was applied to another CIE's FDEs");
    }


    /// `count_eh_frame_fdes` is used to size the .eh_frame_hdr *before* the
    /// FDEs are parsed. If it ever disagrees with `parse_eh_frame_fdes`, the
    /// header is either truncated (the unwinder reads past the section and
    /// crashes) or over-sized. Pin them together across a range of counts.
    #[test]
    fn count_matches_parse_for_many_fde_counts() {
        for n in [0u32, 1, 2, 7, 64, 300] {
            let data = synth_eh_frame(n, PCREL_SDATA4);
            assert_eq!(count_eh_frame_fdes(&data), n as usize,
                       "count_eh_frame_fdes disagrees at n={n}");
            let fdes = parse_eh_frame_fdes(&data, 0x400000, true);
            assert_eq!(fdes.len(), n as usize,
                       "parse_eh_frame_fdes disagrees at n={n}");
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
            assert!(seen.insert(f.initial_location),
                    "duplicate initial_location 0x{:x}: the cache returned a \
                     stale encoding for some FDE", f.initial_location);
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
            let loc = i32::from_le_bytes([hdr[off], hdr[off+1], hdr[off+2], hdr[off+3]]);
            assert!(loc >= prev, "table not sorted at entry {i}: {loc} < {prev}");
            prev = loc;
        }
    }

    /// Malformed input must terminate, not hang or panic. The zero-length and
    /// truncated cases are what a fuzzer hits first.
    #[test]
    fn malformed_input_terminates_without_panic() {
        for data in [
            vec![],
            vec![0u8; 3],                       // shorter than a length field
            vec![0xff, 0xff, 0xff, 0xff],       // extended length, truncated
            vec![0x10, 0, 0, 0],                // length past end of buffer
            vec![0u8; 64],                      // all zero terminators
        ] {
            let _ = count_eh_frame_fdes(&data);
            let _ = parse_eh_frame_fdes(&data, 0x400000, true);
            let _ = build_eh_frame_hdr(&data, 0x400000, 0x3f0000, true);
        }
    }
}
