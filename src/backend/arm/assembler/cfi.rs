//! AArch64 side of the `.cfi_*` lowering into `.eh_frame`
//! ([`crate::backend::cfi_lower`] holds the target-neutral part).
//!
//! Frame conventions as GNU as encodes them for aarch64-linux: CIE version
//! 1, augmentation `zR`, code alignment factor 4, data alignment factor -8,
//! return-address column 30 (`x30`/`lr`), initial instruction
//! `DW_CFA_def_cfa sp, 0` (the return address is in `x30` at entry). The
//! FDE initial location is `.long start - .`, which the writer turns into
//! `R_AARCH64_PREL32` like GNU as; the address range and the location
//! advances are assembly-time constants (no linker relaxation on AArch64).

use super::encoder::parse_reg_num;
use super::parser::{AsmDirective, AsmStatement, SectionDirective, parse_asm};
use crate::backend::cfi_lower::{CfiItemKind, CfiStream, CfiTarget, SectionKey};

/// DWARF register number of an AArch64 register name (AAPCS64 DWARF:
/// x0-x30 = 0-30, sp = 31, v0-v31 in any width = 64-95).
fn dwarf_reg(name: &str) -> Option<u64> {
    let lower = name.to_ascii_lowercase();
    match lower.as_str() {
        "sp" | "wsp" => return Some(31),
        "xzr" | "wzr" => return None,
        _ => {}
    }
    let num = u64::from(parse_reg_num(&lower)?);
    match lower.as_bytes()[0] {
        b'x' | b'w' | b'l' => (num <= 30).then_some(num),
        _ => Some(64 + num),
    }
}

/// The AArch64 frame conventions.
pub(crate) fn target() -> CfiTarget {
    CfiTarget {
        ptr_size: 8,
        code_align: 4,
        data_align: -8,
        default_ra_column: 30,
        sp_column: 31,
        entry_cfa_offset: 0,
        ra_saved_at_entry: false,
        cie_version: None,
        pcrel_item: false,
        dwarf_reg,
    }
}

fn section_key(dir: &SectionDirective) -> SectionKey {
    (dir.name.clone(), None)
}

impl CfiStream for AsmStatement {
    fn cfi_kind(&self) -> CfiItemKind {
        let AsmStatement::Directive(d) = self else {
            return match self {
                AsmStatement::Label(_) | AsmStatement::Empty => CfiItemKind::NoBytes,
                _ => CfiItemKind::Bytes,
            };
        };
        match d {
            AsmDirective::Section(dir) => CfiItemKind::Section(section_key(dir)),
            AsmDirective::PushSection(dir) => CfiItemKind::PushSection(section_key(dir)),
            AsmDirective::PopSection => CfiItemKind::PopSection,
            AsmDirective::Previous => CfiItemKind::Previous,
            AsmDirective::Cfi(name, args) => CfiItemKind::Cfi(name.clone(), args.clone()),
            // Symbol attributes and ignored directives place no bytes.
            // (`Ltorg` may flush a literal pool, so it counts as bytes.)
            AsmDirective::Global(_)
            | AsmDirective::Weak(_)
            | AsmDirective::Hidden(_)
            | AsmDirective::Protected(_)
            | AsmDirective::Internal(_)
            | AsmDirective::SymbolType(..)
            | AsmDirective::Size(..)
            | AsmDirective::Local(_)
            | AsmDirective::Set(..)
            | AsmDirective::Ignored => CfiItemKind::NoBytes,
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
        AsmStatement::Directive(AsmDirective::CfaAdvance { from, to })
    }
    fn pcrel32(sym: String) -> Result<Self, String> {
        Err(format!(
            "internal .eh_frame lowering: AArch64 references `{}` as `.long {} - .`",
            sym, sym
        ))
    }
}
