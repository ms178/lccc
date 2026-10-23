//! RISC-V side of the `.cfi_*` lowering into `.eh_frame`
//! ([`crate::backend::cfi_lower`] holds the target-neutral part).
//!
//! Frame conventions as GNU as 2.47 encodes them (measured): CIE version 3,
//! augmentation `zR`, code alignment factor 1, data alignment factor -4,
//! return-address column 1 (`ra`), initial instruction `DW_CFA_def_cfa sp,
//! 0` (the return address is in a register at entry, not on the stack).
//!
//! Relocations follow GNU as as well: the FDE initial location is an
//! `R_RISCV_32_PCREL` ([`Directive::Pcrel32`]), the FDE address range an
//! ADD32/SUB32 pair (an ordinary `.long end - start` over code labels).
//! A location advance ([`Directive::CfaAdvance`]) is relocated with a
//! SET6/SUB6, SET8/SUB8, SET16/SUB16 or SET32/SUB32 pair whenever the object
//! is assembled with relaxation, because linker relaxation can shrink the
//! code it spans. GNU as relocates only the advances that span a frag
//! boundary -- it starts a new frag after every instruction the linker may
//! shrink or delete ("necessary to get correct EH info", tc-riscv.c) and at
//! every alignment -- and folds the rest; relocating every advance is the
//! conservative superset of that rule, exact for any input and needing no
//! knowledge of which instructions the linker may relax.

use super::encoder::{freg_num, reg_num, vreg_num};
use super::parser::{AsmStatement, Directive, SectionInfo, parse_asm};
use crate::backend::cfi_lower::{CfiItemKind, CfiStream, CfiTarget, SectionKey};

/// DWARF register number of a RISC-V register name (psABI "DWARF Register
/// Numbers": x0-x31 = 0-31, f0-f31 = 32-63, v0-v31 = 96-127).
fn dwarf_reg(name: &str) -> Option<u64> {
    reg_num(name)
        .map(u64::from)
        .or_else(|| freg_num(name).map(|r| 32 + u64::from(r)))
        .or_else(|| vreg_num(name).map(|r| 96 + u64::from(r)))
}

/// The RISC-V frame conventions; `ptr_size` is 8 for RV64, 4 for RV32.
pub(crate) fn target(ptr_size: i64) -> CfiTarget {
    CfiTarget {
        ptr_size,
        code_align: 1,
        data_align: -4,
        default_ra_column: 1,
        sp_column: 2,
        entry_cfa_offset: 0,
        ra_saved_at_entry: false,
        cie_version: Some(3),
        pcrel_item: true,
        dwarf_reg,
    }
}

fn section_key(info: &SectionInfo) -> SectionKey {
    (info.name.clone(), None)
}

impl CfiStream for AsmStatement {
    fn cfi_kind(&self) -> CfiItemKind {
        let AsmStatement::Directive(d) = self else {
            return match self {
                AsmStatement::Label(_) | AsmStatement::Empty => CfiItemKind::NoBytes,
                _ => CfiItemKind::Bytes,
            };
        };
        let std_section = |name: &str| CfiItemKind::Section((name.to_string(), None));
        match d {
            Directive::Section(info) => CfiItemKind::Section(section_key(info)),
            Directive::Text => std_section(".text"),
            Directive::Data => std_section(".data"),
            Directive::Bss => std_section(".bss"),
            Directive::Rodata => std_section(".rodata"),
            Directive::PushSection(info) => CfiItemKind::PushSection(section_key(info)),
            Directive::PopSection => CfiItemKind::PopSection,
            Directive::Previous => CfiItemKind::Previous,
            Directive::Cfi(name, args) => CfiItemKind::Cfi(name.clone(), args.clone()),
            // Symbol attributes, assembler state and ignored directives
            // place no bytes.
            Directive::Globl(_)
            | Directive::Weak(_)
            | Directive::SymVisibility(..)
            | Directive::Type(..)
            | Directive::Size(..)
            | Directive::Local(_)
            | Directive::Set(..)
            | Directive::ArchOption(_)
            | Directive::Attribute(_)
            | Directive::Ignored => CfiItemKind::NoBytes,
            _ => CfiItemKind::Bytes,
        }
    }
    fn as_label(&self) -> Option<&str> {
        match self {
            AsmStatement::Label(l) => Some(l),
            _ => None,
        }
    }
    fn label(name: String) -> Self {
        AsmStatement::Label(name)
    }
    fn parse(text: &str) -> Result<Vec<Self>, String> {
        parse_asm(text)
    }
    fn advance(from: String, to: String) -> Self {
        AsmStatement::Directive(Directive::CfaAdvance { from, to })
    }
    fn pcrel32(sym: String) -> Result<Self, String> {
        Ok(AsmStatement::Directive(Directive::Pcrel32(sym)))
    }
}

/// Bytes of the smallest `DW_CFA_advance_loc*` form holding `delta`.
pub(crate) fn advance_width(delta: u64) -> usize {
    match delta {
        0..=0x3f => 1,
        0x40..=0xff => 2,
        0x100..=0xffff => 3,
        _ => 5,
    }
}

/// A location advance by `delta` code units in the form `width` bytes long
/// (1: `DW_CFA_advance_loc`, 2: `_loc1`, 3: `_loc2`, 5: `_loc4`; `width` is
/// at least [`advance_width`]): the bytes, the offset of the relocated field
/// within them and its (R_RISCV_SETn, R_RISCV_SUBn) relocation pair.
pub(crate) fn encode_advance(delta: u64, width: usize) -> Result<(Vec<u8>, u64, u32, u32), String> {
    const SUB6: u32 = 52;
    const SET6: u32 = 53;
    const SET8: u32 = 54;
    const SET16: u32 = 55;
    const SET32: u32 = 56;
    const SUB8: u32 = 37;
    const SUB16: u32 = 38;
    const SUB32: u32 = 39;
    if delta > u64::from(u32::MAX) {
        return Err(format!(
            "CFA advance of {delta:#x} bytes exceeds DW_CFA_advance_loc4"
        ));
    }
    Ok(match width.max(advance_width(delta)) {
        // DW_CFA_advance_loc: the delta is the low 6 bits of the opcode.
        1 => (vec![0x40 | delta as u8], 0, SET6, SUB6),
        2 => (vec![0x02, delta as u8], 1, SET8, SUB8),
        3 => {
            let mut v = vec![0x03];
            v.extend_from_slice(&(delta as u16).to_le_bytes());
            (v, 1, SET16, SUB16)
        }
        _ => {
            let mut v = vec![0x04];
            v.extend_from_slice(&(delta as u32).to_le_bytes());
            (v, 1, SET32, SUB32)
        }
    })
}
