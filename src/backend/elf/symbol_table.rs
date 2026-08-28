//! Shared symbol table builder for all backend ELF writers.
//!
//! All four backend assemblers (x86-64, i686, ARM, RISC-V) use this shared
//! `build_elf_symbol_table` function to construct their symbol tables from
//! labels, aliases, and relocation references. This eliminates duplicated
//! symbol table construction logic across the backends.
//!
//! The only architecture-specific difference is that RISC-V needs to include
//! referenced local labels (for pcrel_hi synthetic labels) in the symbol table.

use super::constants::*;
use super::object_writer::ObjSection;
use crate::common::fx_hash::{FxHashMap, FxHashSet};

/// A symbol in a relocatable object file.
pub struct ObjSymbol {
    pub name: String,
    pub value: u64,
    pub size: u64,
    pub binding: u8,
    pub sym_type: u8,
    pub visibility: u8,
    /// Section name, or "*COM*" for COMMON, "*UND*" or empty for undefined.
    pub section_name: String,
}

/// Parameters for the shared `build_elf_symbol_table` function.
/// Collects the state needed to build the symbol table without requiring
/// a specific ElfWriter struct type.
pub struct SymbolTableInput<'a> {
    pub labels: &'a FxHashMap<String, (String, u64)>,
    pub global_symbols: &'a FxHashMap<String, bool>,
    pub weak_symbols: &'a FxHashMap<String, bool>,
    pub symbol_types: &'a FxHashMap<String, u8>,
    pub symbol_sizes: &'a FxHashMap<String, u64>,
    pub symbol_visibility: &'a FxHashMap<String, u8>,
    pub aliases: &'a FxHashMap<String, String>,
    pub sections: &'a FxHashMap<String, ObjSection>,
    /// If true, include .L* local labels that are referenced by relocations
    /// in the symbol table (needed by RISC-V for pcrel_hi/pcrel_lo pairs).
    pub include_referenced_locals: bool,
}

/// Build a symbol table from labels, aliases, and relocation references.
///
/// Returns a list of `ObjSymbol` entries ready for `write_relocatable_object`.
/// Handles:
/// - Defined labels (global, weak, local)
/// - .set/.equ aliases with chain resolution
/// - Undefined symbols (referenced in relocations but not defined)
/// - Optionally, referenced local labels (.L*) for RISC-V pcrel support
pub fn build_elf_symbol_table(input: &SymbolTableInput) -> Vec<ObjSymbol> {
    let mut symbols: Vec<ObjSymbol> = Vec::new();

    // Collect referenced local labels if needed (RISC-V pcrel_hi)
    let mut referenced_local_labels: FxHashSet<String> = FxHashSet::default();
    if input.include_referenced_locals {
        for sec in input.sections.values() {
            for reloc in &sec.relocs {
                if reloc.symbol_name.starts_with(".L") || reloc.symbol_name.starts_with(".l") {
                    referenced_local_labels.insert(reloc.symbol_name.clone());
                }
            }
        }
    }

    // Add defined labels as symbols
    for (name, (section, offset)) in input.labels {
        let is_local_label = name.starts_with(".L") || name.starts_with(".l");
        if is_local_label && !referenced_local_labels.contains(name) {
            continue;
        }

        let binding = if input.weak_symbols.contains_key(name) {
            STB_WEAK
        } else if input.global_symbols.contains_key(name) {
            STB_GLOBAL
        } else {
            STB_LOCAL
        };

        symbols.push(ObjSymbol {
            name: name.clone(),
            value: *offset,
            size: input.symbol_sizes.get(name).copied().unwrap_or(0),
            binding,
            sym_type: input.symbol_types.get(name).copied().unwrap_or(STT_NOTYPE),
            visibility: input
                .symbol_visibility
                .get(name)
                .copied()
                .unwrap_or(STV_DEFAULT),
            section_name: section.clone(),
        });
    }

    // Add alias symbols from .set/.equ directives
    let defined_names: FxHashMap<String, usize> = symbols
        .iter()
        .enumerate()
        .map(|(i, s)| (s.name.clone(), i))
        .collect();

    for (alias, target) in input.aliases {
        // Resolve through alias chains
        let mut resolved = target.as_str();
        let mut seen = FxHashSet::default();
        seen.insert(target.as_str());
        while let Some(next) = input.aliases.get(resolved) {
            if !seen.insert(next.as_str()) {
                break;
            }
            resolved = next.as_str();
        }

        let alias_binding = if input.weak_symbols.contains_key(alias) {
            Some(STB_WEAK)
        } else if input.global_symbols.contains_key(alias) {
            Some(STB_GLOBAL)
        } else {
            None
        };
        let alias_type = input.symbol_types.get(alias).copied();
        let alias_vis = input.symbol_visibility.get(alias).copied();

        if let Some(&idx) = defined_names.get(resolved) {
            let target_sym = &symbols[idx];
            symbols.push(ObjSymbol {
                name: alias.clone(),
                value: target_sym.value,
                size: target_sym.size,
                binding: alias_binding.unwrap_or(target_sym.binding),
                sym_type: alias_type.unwrap_or(target_sym.sym_type),
                // Visibility comes only from an explicit .hidden/.protected
                // directive on the alias itself (GNU as semantics). Copying the
                // target's visibility would make strong_aliases of hidden
                // objects hidden too — e.g. glibc rtld.c's
                // `strong_alias (__pointer_chk_guard_local, __pointer_chk_guard)`
                // would produce a hidden __pointer_chk_guard that the ld.map
                // version script cannot export (GLIBC_PRIVATE), leaving
                // libc.so with an undefined __pointer_chk_guard.
                visibility: alias_vis.unwrap_or(STV_DEFAULT),
                section_name: target_sym.section_name.clone(),
            });
        } else if let Some((section, offset)) = input.labels.get(resolved) {
            symbols.push(ObjSymbol {
                name: alias.clone(),
                value: *offset,
                size: 0,
                binding: alias_binding.unwrap_or(STB_LOCAL),
                sym_type: alias_type.unwrap_or(STT_NOTYPE),
                visibility: alias_vis.unwrap_or(STV_DEFAULT),
                section_name: section.clone(),
            });
        } else if let Ok(abs_val) = resolved.parse::<i64>() {
            // `.set sym, <integer>` defines an ABSOLUTE symbol (GNU as
            // semantics). glibc localeinfo.h `_NL_CURRENT_DEFINE` emits
            // `.set _nl_current_LC_CTYPE_used, 2` in inline asm; without the
            // absolute symbol the static libc link fails with undefined
            // references from setlocale.o.
            symbols.push(ObjSymbol {
                name: alias.clone(),
                value: abs_val as u64,
                size: 0,
                // `.set` alone does not export anything: the symbol is LOCAL
                // unless a separate `.globl`/`.weak` says otherwise, which is
                // what `alias_binding` carries.  Defaulting to STB_GLOBAL here
                // (while the label branch above defaults to STB_LOCAL) leaked
                // every assembler-internal constant into the object's global
                // namespace, where it could collide at link time.
                binding: alias_binding.unwrap_or(STB_LOCAL),
                sym_type: alias_type.unwrap_or(STT_NOTYPE),
                visibility: alias_vis.unwrap_or(STV_DEFAULT),
                section_name: "*ABS*".to_string(),
            });
        }
    }

    // Add undefined symbols (referenced in relocations but not defined)
    let mut referenced: FxHashSet<String> = FxHashSet::default();
    // Symbols referenced by TLS relocations must be STT_TLS in the symbol
    // table; otherwise the linker rejects the object with
    // "TLS definition ... mismatches non-TLS reference" (glibc's errno.os /
    // __libc_errno via @GOTTPOFF hits this).
    let mut tls_referenced: FxHashSet<String> = FxHashSet::default();
    for sec in input.sections.values() {
        for reloc in &sec.relocs {
            if reloc.symbol_name.is_empty() {
                continue;
            }
            if reloc.symbol_name.starts_with(".L") || reloc.symbol_name.starts_with(".l") {
                continue;
            }
            referenced.insert(reloc.symbol_name.clone());
            if is_tls_reloc(reloc.reloc_type) {
                tls_referenced.insert(reloc.symbol_name.clone());
            }
        }
    }

    let defined: FxHashSet<String> = symbols.iter().map(|s| s.name.clone()).collect();

    for name in &referenced {
        if input.sections.contains_key(name) {
            continue; // Skip section names
        }
        if !defined.contains(name) {
            let binding = if input.weak_symbols.contains_key(name) {
                STB_WEAK
            } else {
                STB_GLOBAL
            };
            symbols.push(ObjSymbol {
                name: name.clone(),
                value: 0,
                size: 0,
                binding,
                sym_type: input.symbol_types.get(name).copied().unwrap_or_else(|| {
                    if tls_referenced.contains(name) {
                        STT_TLS
                    } else {
                        STT_NOTYPE
                    }
                }),
                visibility: input
                    .symbol_visibility
                    .get(name)
                    .copied()
                    .unwrap_or(STV_DEFAULT),
                section_name: "*UND*".to_string(),
            });
        }
    }

    symbols
}

/// True if the relocation type is a TLS relocation (x86-64 and i386).
/// Undefined symbols referenced by these must be emitted as STT_TLS so the
/// static linker can match them against TLS definitions.
///
/// The relocation-type numbers COLLIDE across the two architectures, so the
/// classifier must be target-aware. The old naive union included 42, which is
/// `R_X86_64_REX_GOTPCRELX` (a relaxable GOT reference) — NOT a TLS
/// relocation on either architecture. Every `@GOTPCRELX`-referenced symbol
/// was therefore emitted as STT_TLS, and the linker laid it out as a
/// thread-local symbol whose address resolved to the WRONG symbol (gzip:
/// `prev` resolved to `&rsync`, so `head = prev + WSIZE` wrote hash entries
/// into the globals region and corrupted `strstart` — SIGSEGV in deflate
/// under PGO builds, and only there because block alignment changed the
/// GOT-layout path).
fn is_tls_reloc(rtype: u32) -> bool {
    // Relocation numbers are per-ISA namespaces: key off the ELF machine, NOT
    // the pointer size. The old `target_is_32bit()` split classified RISC-V
    // objects with the x86-64 table, where 16..=23 covers R_RISCV_BRANCH(16),
    // R_RISCV_JAL(17), R_RISCV_CALL(18), R_RISCV_CALL_PLT(19) and
    // R_RISCV_PCREL_HI20(23) — every external call symbol was stamped STT_TLS
    // and cross-GCC links rejected the object ("TLS reference mismatches
    // non-TLS definition"). Tables below list exactly the TLS-form reloc
    // numbers of each machine; anything else is not TLS.
    match crate::common::types::target_elf_machine() {
        crate::backend::elf::EM_386 => {
            // i386 R_386_TLS_*: TPOFF=14, IE=15, GOTIE=16, LE=17, GD=18, LDM=19,
            // GD_32..TPOFF32 = 24..=37, GOTDESC=39, DESC_CALL=40, DESC=41.
            matches!(rtype, 14..=19 | 24..=37 | 39..=41)
        }
        crate::backend::elf::EM_X86_64 => {
            // x86-64 TLS relocations: DTPMOD64=16, DTPOFF64=17, TPOFF64=18,
            // TLSGD=19, TLSLD=20, DTPOFF32=21, GOTTPOFF=22, TPOFF32=23,
            // GOTPC32_TLSDESC=29, TLSDESC_CALL=30, TLSDESC=31.
            // 41 (GOTPCRELX) and 42 (REX_GOTPCRELX) are relaxable GOT relocations,
            // NOT TLS — deliberately excluded.
            matches!(rtype, 16..=23 | 29..=31)
        }
        crate::backend::elf::EM_AARCH64 => {
            // AArch64 TLS: TLSLE_ADD_TPREL_HI12=549, ADD_TPREL_LO12=550,
            // LO12_NC=551 (numbers verified against the installed
            // aarch64 binutils by decoding emitted TLS objects).
            matches!(rtype, 549..=551)
        }
        crate::backend::elf::EM_RISCV => {
            // RISC-V TLS: TLS_GOT_HI20=21, TLS_GD_HI20=22, TPREL_HI20=29,
            // TPREL_LO12_I=30, TPREL_LO12_S=31, TPREL_ADD=32.
            matches!(rtype, 21..=22 | 29..=32)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression: is_tls_reloc must key off the ELF machine, not the pointer
    /// size. The old target_is_32bit() split classified RISC-V objects with the
    /// x86-64 table — R_RISCV_CALL_PLT(19) fell inside x86-64's TLSGD=19..=
    /// TPOFF32=23 range and every external function symbol was stamped STT_TLS
    /// ("TLS reference mismatches non-TLS definition" at cross-GCC link).
    #[test]
    fn tls_classification_is_per_machine() {
        use crate::backend::elf::{EM_386, EM_AARCH64, EM_RISCV, EM_X86_64};
        use crate::common::types::{set_target_elf_machine, target_elf_machine};

        // RISC-V: calls and PC-relative data references are NOT TLS.
        set_target_elf_machine(EM_RISCV);
        assert_eq!(target_elf_machine(), EM_RISCV);
        for r in [16u32 /*BRANCH*/, 17 /*JAL*/, 18 /*CALL*/, 19 /*CALL_PLT*/, 23 /*PCREL_HI20*/, 24 /*PCREL_LO12_I*/, 51 /*RELAX*/] {
            assert!(!is_tls_reloc(r), "RISC-V r{r} must not be TLS");
        }
        for r in [21u32, 22, 29, 30, 31, 32] {
            assert!(is_tls_reloc(r), "RISC-V r{r} must be TLS");
        }

        // AArch64: call/jump/GOT relocations are NOT TLS; local-exec TPREL is.
        set_target_elf_machine(EM_AARCH64);
        for r in [257u32, 275, 282 /*JUMP26*/, 283 /*CALL26*/] {
            assert!(!is_tls_reloc(r), "AArch64 r{r} must not be TLS");
        }
        for r in [549u32, 550, 551] {
            assert!(is_tls_reloc(r), "AArch64 r{r} must be TLS");
        }

        // x86-64: TLSGD=19 is TLS while GOTPCRELX=41 / REX_GOTPCRELX=42 are not.
        set_target_elf_machine(EM_X86_64);
        assert!(is_tls_reloc(19));
        assert!(is_tls_reloc(16));
        assert!(!is_tls_reloc(41));
        assert!(!is_tls_reloc(42));

        // i386 keeps its proven table.
        set_target_elf_machine(EM_386);
        assert!(is_tls_reloc(14));
        assert!(is_tls_reloc(37));
        assert!(!is_tls_reloc(20));
        assert!(!is_tls_reloc(38));
    }
}
