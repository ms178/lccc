//! `R_RISCV_ALIGN`: trim the worst-case NOP runs of relaxable objects.
//!
//! With linker relaxation enabled an assembler cannot know where an alignment
//! directive in code will land -- relaxing an earlier `call` shrinks it -- so
//! GNU as (`riscv_frag_align_code`) writes the worst case, `align -
//! insn_alignment` bytes of NOPs, and marks them with an `R_RISCV_ALIGN`
//! whose addend is that byte count.  The linker owns the rest: it derives the
//! alignment from the addend (the smallest power of two above it), keeps the
//! NOPs the final address needs and deletes the others.  This is not an
//! optimisation the linker may skip: left untrimmed, the code after
//! `.p2align 4` in a GAS object lands at `start + 14`, not on the boundary
//! (measured: `lui; .p2align 4; f2:` put `f2` at an address ≡ 2 mod 16).
//!
//! This module is the whole of relaxation under `--no-relax` (GNU ld runs
//! its ALIGN pass even then); a relaxing link handles ALIGN inside the full
//! relaxation instead (`relax.rs`).  With no other size change, the only
//! movement inside an input section comes from these deletions, and the
//! section itself is placed at a multiple of its `sh_addralign`.  Raising
//! that to the largest alignment the section's `R_RISCV_ALIGN`s request (GNU
//! as records it anyway) makes every directive's final residue a function of
//! its offset in the section alone, so the trimming is done once per object,
//! before layout, and everything downstream sees an ordinary object whose
//! code simply has fewer NOPs.
//!
//! What moves with the bytes (all through one monotone old-to-new offset map):
//!   * the section's bytes and size;
//!   * the offsets of the relocations applied to the section;
//!   * symbols defined in it (value; size by mapping both ends);
//!   * the addend of every relocation, in ANY section, against the SECTION
//!     symbol of it: the target `value + addend` is mapped, so `.text+0x40`
//!     keeps naming the same byte.  An addend against any other symbol is
//!     left alone, as GNU ld (`riscv_relax_delete_bytes`) leaves every
//!     addend: `.LVL5-1` (GCC's DWARF location-list idiom) means "one byte
//!     before `.LVL5`" wherever `.LVL5` lands, and mapping it instead would
//!     clamp a target inside a deleted run onto `.LVL5` itself.
//!
//! Anything PC-relative inside relaxable code already carries a relocation
//! (that is the relaxation ABI: assemblers emit branch/jal/`%pcrel` relocs
//! and ADD/SUB/SET pairs for label differences, also in `.eh_frame` and
//! DWARF), so no resolved displacement can straddle a deleted range.

use super::elf_read::ElfObject;
use crate::backend::linker_common::SectionData;

const R_RISCV_ALIGN: u32 = 43;
const STT_SECTION: u8 = 3;
const SHN_LORESERVE: u16 = 0xff00;

/// One deletion in a section: `len` bytes starting at original `start`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Cut {
    start: u64,
    len: u64,
}

/// Monotone map from an original section offset to the trimmed one.  An
/// offset inside a deleted range maps to the range's start.
fn map_offset(cuts: &[Cut], off: u64) -> u64 {
    let mut removed = 0;
    for c in cuts {
        if off >= c.start + c.len {
            removed += c.len;
        } else if off > c.start {
            removed += off - c.start;
            break;
        } else {
            break;
        }
    }
    off - removed
}

/// The NOPs `riscv_relax_align` (GNU ld) writes for `n` bytes: full `nop`s,
/// then one `c.nop` for a 2-byte remainder.  An odd count only arises after
/// odd-sized data in a code section; the stray byte goes first as 0x00, the
/// convention GNU as uses (`riscv_make_nops`), so the instructions that follow
/// stay 2-byte aligned.
fn write_nops(out: &mut [u8]) {
    let mut i = 0;
    if out.len() % 2 == 1 {
        out[0] = 0;
        i = 1;
    }
    while out.len() - i >= 4 {
        out[i..i + 4].copy_from_slice(&0x0000_0013u32.to_le_bytes());
        i += 4;
    }
    if out.len() - i == 2 {
        out[i..i + 2].copy_from_slice(&0x0001u16.to_le_bytes());
    }
}

/// Trim the `R_RISCV_ALIGN` NOP runs of `obj` in place (module docs).
pub fn relax_align(obj: &mut ElfObject) -> Result<(), String> {
    let nsec = obj.sections.len();
    let mut cuts_of: Vec<Vec<Cut>> = vec![Vec::new(); nsec];
    let mut any = false;
    for s in 0..nsec.min(obj.relocations.len()) {
        let mut aligns: Vec<(u64, u64)> = obj.relocations[s]
            .iter()
            .filter(|r| r.rela_type == R_RISCV_ALIGN)
            .map(|r| (r.offset, r.addend.max(0) as u64))
            .collect();
        if aligns.is_empty() {
            continue;
        }
        aligns.sort_unstable();
        let size = obj.section_data.get(s).map_or(0, |d| d.len() as u64);
        let mut data: Option<Vec<u8>> = None;
        let mut removed = 0u64;
        let mut max_align = 1u64;
        let mut cuts = Vec::new();
        for (pos, avail) in aligns {
            if avail == 0 {
                continue;
            }
            if pos + avail > size {
                return Err(format!(
                    "{}: R_RISCV_ALIGN at {}+{pos:#x} covers {avail} bytes past the end of the section",
                    obj.source_name, obj.sections[s].name
                ));
            }
            let align = (avail + 1).next_power_of_two();
            max_align = max_align.max(align);
            let at = pos - removed;
            let need = (align - at % align) % align;
            if need > avail {
                return Err(format!(
                    "{}({}+{pos:#x}): {need} bytes required for alignment to {align}-byte boundary, but only {avail} present",
                    obj.source_name, obj.sections[s].name
                ));
            }
            let bytes = data.get_or_insert_with(|| obj.section_data[s].to_owned_vec());
            write_nops(&mut bytes[pos as usize..(pos + need) as usize]);
            if avail > need {
                cuts.push(Cut {
                    start: pos + need,
                    len: avail - need,
                });
                removed += avail - need;
            }
        }
        obj.sections[s].addralign = obj.sections[s].addralign.max(max_align);
        obj.relocations[s].retain(|r| r.rela_type != R_RISCV_ALIGN);
        if let Some(bytes) = data {
            let mut out = Vec::with_capacity(bytes.len() - removed as usize);
            let mut from = 0usize;
            for c in &cuts {
                out.extend_from_slice(&bytes[from..c.start as usize]);
                from = (c.start + c.len) as usize;
            }
            out.extend_from_slice(&bytes[from..]);
            obj.sections[s].size = out.len() as u64;
            obj.section_data[s] = SectionData::owned(out);
        }
        if !cuts.is_empty() {
            any = true;
            cuts_of[s] = cuts;
        }
    }
    if !any {
        return Ok(());
    }

    // Addends first, against the ORIGINAL symbol values.
    let home = |shndx: u16| -> Option<&Vec<Cut>> {
        (shndx != 0 && shndx < SHN_LORESERVE)
            .then(|| cuts_of.get(shndx as usize))
            .flatten()
            .filter(|c| !c.is_empty())
    };
    for relocs in &mut obj.relocations {
        for r in relocs.iter_mut() {
            let Some(sym) = obj.symbols.get(r.sym_idx as usize) else {
                continue;
            };
            if sym.info & 0xf != STT_SECTION {
                continue;
            }
            let Some(cuts) = home(sym.shndx) else {
                continue;
            };
            let target = sym.value as i64 + r.addend;
            if target < 0 {
                continue;
            }
            let base = map_offset(cuts, sym.value) as i64;
            r.addend = map_offset(cuts, target as u64) as i64 - base;
        }
    }
    for (s, cuts) in cuts_of.iter().enumerate() {
        if cuts.is_empty() {
            continue;
        }
        for r in &mut obj.relocations[s] {
            r.offset = map_offset(cuts, r.offset);
        }
    }
    for sym in &mut obj.symbols {
        let Some(cuts) = home(sym.shndx) else {
            continue;
        };
        let start = map_offset(cuts, sym.value);
        if sym.size != 0 && sym.info & 0xf != STT_SECTION {
            sym.size = map_offset(cuts, sym.value + sym.size) - start;
        }
        sym.value = start;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::linker_common::{Elf64Rela, Elf64Section, Elf64Symbol};

    const NOP: [u8; 4] = 0x0000_0013u32.to_le_bytes();
    const LUI: [u8; 4] = 0x1234_5537u32.to_le_bytes();

    fn section(name: &str, flags: u64, size: u64) -> Elf64Section {
        Elf64Section {
            name_idx: 0,
            name: name.into(),
            sh_type: 1,
            flags,
            addr: 0,
            offset: 0,
            size,
            link: 0,
            info: 0,
            addralign: 4,
            entsize: 0,
        }
    }

    fn sym(name: &str, info: u8, shndx: u16, value: u64, size: u64) -> Elf64Symbol {
        Elf64Symbol {
            name_idx: 0,
            name: name.into(),
            info,
            other: 0,
            shndx,
            value,
            size,
        }
    }

    fn rela(offset: u64, sym_idx: u32, rela_type: u32, addend: i64) -> Elf64Rela {
        Elf64Rela {
            offset,
            sym_idx,
            rela_type,
            addend,
        }
    }

    /// GNU as for `f1: lui a0,0x12345; .p2align 4; f2: c.addi; c.jr ra`
    /// with a `.data` word holding `.text+18` (f2 by section symbol) and a
    /// branch reloc after the run.
    fn gas_object() -> ElfObject {
        let mut text = Vec::new();
        text.extend_from_slice(&LUI);
        text.extend_from_slice(&0x0001u16.to_le_bytes()); // 14 bytes of NOPs
        text.extend_from_slice(&NOP);
        text.extend_from_slice(&NOP);
        text.extend_from_slice(&NOP);
        text.extend_from_slice(&[0x09, 0x05, 0x82, 0x80]); // f2 at 18
        let null = section("", 0, 0);
        ElfObject {
            sections: vec![null, section(".text", 0x6, 22), section(".data", 0x3, 8)],
            symbols: vec![
                sym("", 0, 0, 0, 0),
                sym("", STT_SECTION, 1, 0, 0),
                sym("f1", 0x12, 1, 0, 22),
                sym("f2", 0x12, 1, 18, 4),
            ],
            section_data: vec![
                SectionData::empty(),
                SectionData::owned(text),
                SectionData::owned(vec![0; 8]),
            ],
            relocations: vec![
                vec![],
                vec![rela(4, 0, R_RISCV_ALIGN, 14), rela(20, 3, 16, 0)],
                vec![rela(0, 1, 2, 18)],
            ],
            source_name: "a.o".into(),
            e_flags: 0,
        }
    }

    #[test]
    fn trims_to_the_boundary_and_moves_everything_after() {
        let mut o = gas_object();
        relax_align(&mut o).unwrap();
        let text = o.section_data[1].as_slice();
        // 12 NOPs reach 16; the other 2 bytes go.
        assert_eq!(text.len(), 20);
        assert_eq!(o.sections[1].size, 20);
        assert_eq!(&text[4..16], [NOP, NOP, NOP].concat().as_slice());
        assert_eq!(&text[16..18], &[0x09, 0x05]);
        assert!(o.sections[1].addralign >= 16);
        assert_eq!(o.symbols[3].value, 16, "f2 on the boundary");
        assert_eq!(o.symbols[2].size, 20, "f1 spans the trimmed bytes");
        assert!(
            o.relocations[1]
                .iter()
                .all(|r| r.rela_type != R_RISCV_ALIGN)
        );
        assert_eq!(o.relocations[1][0].offset, 18, "branch reloc moved");
        assert_eq!(o.relocations[2][0].addend, 16, ".text+18 now .text+16");
    }

    #[test]
    fn nothing_is_cut_when_the_whole_run_is_needed() {
        // After a 2-byte instruction the worst case is exactly the need.
        let mut o = gas_object();
        let mut text = o.section_data[1].to_owned_vec();
        text.drain(0..2); // 2-byte first instruction: run at 2, f2 at 16
        o.section_data[1] = SectionData::owned(text);
        o.relocations[1] = vec![rela(2, 0, R_RISCV_ALIGN, 14)];
        o.symbols[3].value = 16;
        o.relocations[2] = vec![rela(0, 1, 2, 16)];
        relax_align(&mut o).unwrap();
        assert_eq!(o.section_data[1].len(), 20);
        assert_eq!(o.symbols[3].value, 16);
        assert_eq!(o.relocations[2][0].addend, 16);
    }

    #[test]
    fn insufficient_padding_is_an_error_as_in_gnu_ld() {
        // Padding only to the boundary (8 bytes at offset 8 for 16) leaves
        // nothing for a shift: at offset 4 the 16-byte boundary needs 12.
        let mut o = gas_object();
        o.relocations[1] = vec![rela(4, 0, R_RISCV_ALIGN, 8)];
        let e = relax_align(&mut o).unwrap_err();
        assert!(
            e.contains("12 bytes required for alignment to 16-byte boundary, but only 8 present"),
            "{e}"
        );
    }

    #[test]
    fn successive_runs_compound() {
        // Two `.p2align 3` runs (6 bytes each, rvc) after 4-byte insns.
        let mut text = Vec::new();
        text.extend_from_slice(&LUI); // 0
        text.extend_from_slice(&[0x01, 0x00, 0x13, 0, 0, 0]); // run 1 at 4
        text.extend_from_slice(&LUI); // 10
        text.extend_from_slice(&[0x01, 0x00, 0x13, 0, 0, 0]); // run 2 at 14
        text.extend_from_slice(&LUI); // 20
        let mut o = gas_object();
        o.sections[1].size = text.len() as u64;
        o.section_data[1] = SectionData::owned(text);
        o.symbols[3].value = 20;
        o.symbols[3].size = 4;
        o.relocations[1] = vec![rela(4, 0, R_RISCV_ALIGN, 6), rela(14, 0, R_RISCV_ALIGN, 6)];
        o.relocations[2] = vec![rela(0, 1, 2, 20)];
        relax_align(&mut o).unwrap();
        // run 1 keeps 4 (-> 8), insn at 8, run 2 at 12 keeps 4 (-> 16).
        assert_eq!(o.section_data[1].len(), 20);
        assert_eq!(o.symbols[3].value, 16);
        assert_eq!(o.relocations[2][0].addend, 16);
    }

    #[test]
    fn symbol_addends_are_kept_as_gnu_ld_keeps_them() {
        // `.8byte f2-1` (the `.LVL-1` idiom): f2 follows the deleted run
        // [16, 18), so its target byte 17 lies inside it.  The symbol moves
        // to 16 and the reference must still mean f2-1, i.e. byte 15.
        let mut o = gas_object();
        o.relocations[2] = vec![rela(0, 3, 2, -1)];
        relax_align(&mut o).unwrap();
        assert_eq!(o.symbols[3].value, 16);
        assert_eq!(o.relocations[2][0].addend, -1);
    }

    #[test]
    fn map_offset_is_monotone_and_clamps_inside_cuts() {
        let cuts = [Cut { start: 4, len: 2 }, Cut { start: 10, len: 3 }];
        let got: Vec<u64> = (0..16).map(|o| map_offset(&cuts, o)).collect();
        assert_eq!(got, [0, 1, 2, 3, 4, 4, 4, 5, 6, 7, 8, 8, 8, 8, 9, 10]);
    }
}
