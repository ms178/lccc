//! ELF64 parsing for the AArch64 linker.
//!
//! This module re-exports the shared ELF64 types and parser from `linker_common`,
//! plus provides AArch64-specific relocation constants. The actual parsing logic
//! lives in the shared module to avoid duplication with x86 and RISC-V.

// Re-export shared ELF constants so existing callers (mod.rs, reloc.rs)
// continue to work via `use super::elf::*`.
pub use crate::backend::elf::{
    get_standard_linker_symbols, is_thin_archive, parse_linker_script_entries, read_u16, read_u32,
    w16, w32, w64, wphdr, write_bytes, LinkerScriptEntry, LinkerSymbolAddresses, DF_1_NOW,
    DF_BIND_NOW, DT_DEBUG, DT_FINI_ARRAY, DT_FINI_ARRAYSZ, DT_FLAGS, DT_FLAGS_1, DT_GNU_HASH,
    DT_INIT_ARRAY, DT_INIT_ARRAYSZ, DT_JMPREL, DT_NEEDED, DT_NULL, DT_PLTGOT, DT_PLTREL,
    DT_PLTRELSZ, DT_RELA, DT_RELACOUNT, DT_RELAENT, DT_RELASZ, DT_SONAME, DT_STRSZ, DT_STRTAB,
    DT_SYMENT, DT_SYMTAB, ELFCLASS64, ELFDATA2LSB, ELF_MAGIC, EM_AARCH64, ET_DYN, ET_EXEC, PF_R,
    PF_W, PF_X, PT_DYNAMIC, PT_GNU_EH_FRAME, PT_GNU_STACK, PT_INTERP, PT_LOAD, PT_PHDR, PT_TLS,
    SHF_ALLOC, SHF_EXECINSTR, SHF_TLS, SHF_WRITE, SHN_ABS, SHN_COMMON, SHN_UNDEF, SHT_NOBITS,
    STB_GLOBAL, STB_WEAK, STT_FUNC, STT_GNU_IFUNC, STT_OBJECT, STT_SECTION, STT_TLS,
};

use crate::backend::linker_common;

// ── AArch64 relocation types ───────────────────────────────────────────

pub const R_AARCH64_NONE: u32 = 0;
pub const R_AARCH64_ABS64: u32 = 257; // S + A
pub const R_AARCH64_ABS32: u32 = 258; // S + A (32-bit)
pub const R_AARCH64_ABS16: u32 = 259; // S + A (16-bit)
pub const R_AARCH64_PREL64: u32 = 260; // S + A - P
pub const R_AARCH64_PREL32: u32 = 261; // S + A - P
pub const R_AARCH64_PREL16: u32 = 262; // S + A - P
pub const R_AARCH64_ADR_PREL_PG_HI21: u32 = 275; // Page(S+A) - Page(P)
pub const R_AARCH64_ADR_PREL_LO21: u32 = 274; // S + A - P
pub const R_AARCH64_ADD_ABS_LO12_NC: u32 = 277; // (S + A) & 0xFFF
pub const R_AARCH64_LDST8_ABS_LO12_NC: u32 = 278;
pub const R_AARCH64_LDST16_ABS_LO12_NC: u32 = 284;
pub const R_AARCH64_LDST32_ABS_LO12_NC: u32 = 285;
pub const R_AARCH64_LDST64_ABS_LO12_NC: u32 = 286;
pub const R_AARCH64_LDST128_ABS_LO12_NC: u32 = 299;
pub const R_AARCH64_JUMP26: u32 = 282; // S + A - P (26-bit B)
pub const R_AARCH64_CALL26: u32 = 283; // S + A - P (26-bit BL)
                                       // MOVW (movz/movk) absolute halfword relocations, ABI numbers per
                                       // IHI0056B / llvm/include/llvm/BinaryFormat/ELFRelocs/AArch64.def:
                                       // G0=0x107 G0_NC=0x108 G1=0x109 G1_NC=0x10a G2=0x10b G2_NC=0x10c
                                       // G3=0x10d SABS_G0=0x10e SABS_G1=0x10f SABS_G2=0x110.
                                       // (There is deliberately no G3_NC and no SABS_G3 in the ABI. The
                                       // previous table here assumed a consecutive G0/G0_NC/G1_NC/G2_NC/G3
                                       // numbering that matched nothing — any object carrying real ABI MOVW
                                       // relocs would have been resolved with the wrong shift.)
pub const R_AARCH64_MOVW_UABS_G0: u32 = 263; // 0x107
pub const R_AARCH64_MOVW_UABS_G0_NC: u32 = 264; // 0x108
pub const R_AARCH64_MOVW_UABS_G1: u32 = 265; // 0x109
pub const R_AARCH64_MOVW_UABS_G1_NC: u32 = 266; // 0x10a
pub const R_AARCH64_MOVW_UABS_G2: u32 = 267; // 0x10b
pub const R_AARCH64_MOVW_UABS_G2_NC: u32 = 268; // 0x10c
pub const R_AARCH64_MOVW_UABS_G3: u32 = 269; // 0x10d
pub const R_AARCH64_MOVW_SABS_G0: u32 = 270; // 0x10e
pub const R_AARCH64_MOVW_SABS_G1: u32 = 271; // 0x10f
pub const R_AARCH64_MOVW_SABS_G2: u32 = 272; // 0x110
pub const R_AARCH64_ADR_GOT_PAGE: u32 = 311;
pub const R_AARCH64_LD64_GOT_LO12_NC: u32 = 312;
pub const R_AARCH64_CONDBR19: u32 = 280;
pub const R_AARCH64_TSTBR14: u32 = 279;
pub const R_AARCH64_LD_PREL_LO19: u32 = 273; // LDR literal: (S + A - P) >> 2, 19-bit

// ── Type aliases ─────────────────────────────────────────────────────────
// Re-export shared types under the names the ARM linker already uses.

pub type SectionHeader = linker_common::Elf64Section;
pub type Symbol = linker_common::Elf64Symbol;
pub type Rela = linker_common::Elf64Rela;
pub type ElfObject = linker_common::Elf64Object;

// ── Parsing functions ────────────────────────────────────────────────────
// Delegate to shared implementations.

pub fn parse_object(data: &[u8], source_name: &str) -> Result<ElfObject, String> {
    linker_common::parse_elf64_object(data, source_name, EM_AARCH64)
}
