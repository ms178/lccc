//! Post-link undefined symbol checking.
//!
//! Validates that all required symbols have been resolved after linking,
//! filtering out dynamic, weak, and linker-defined symbols.

use crate::common::fx_hash::FxHashMap;

use super::symbols::{is_linker_defined_symbol, GlobalSymbolOps};
use crate::backend::elf::STB_WEAK;

/// Check for undefined symbols in the global symbol table and return an error
/// if any truly undefined symbols are found.
///
/// Filters out dynamic symbols, weak symbols, and linker-defined symbols
/// using the `GlobalSymbolOps` trait methods. `max_report` limits how many
/// symbols are shown in the error message (typically 20).
pub fn check_undefined_symbols_elf64<G: GlobalSymbolOps>(
    globals: &FxHashMap<String, G>,
    max_report: usize,
) -> Result<(), String> {
    check_undefined_symbols_elf64_verbose(globals, max_report, &[])
}

/// Like `check_undefined_symbols_elf64` but, when the input objects are
/// available, reports each undefined symbol GNU-ld style with the referencing
/// object file and the enclosing function:
///
/// ```text
/// undefined reference to `frob'
///   referenced from main.o (in function `do_work')
/// ```
pub fn check_undefined_symbols_elf64_verbose<G: GlobalSymbolOps>(
    globals: &FxHashMap<String, G>,
    max_report: usize,
    objects: &[super::types::Elf64Object],
) -> Result<(), String> {
    let mut truly_undefined: Vec<&String> = globals
        .iter()
        .filter(|(name, sym)| {
            !sym.is_defined()
                && !sym.is_dynamic()
                && (sym.info() >> 4) != STB_WEAK
                && !is_linker_defined_symbol(name)
        })
        .map(|(name, _)| name)
        .collect();
    if truly_undefined.is_empty() {
        return Ok(());
    }
    truly_undefined.sort();
    truly_undefined.truncate(max_report);

    if objects.is_empty() {
        return Err(format!(
            "undefined symbols: {}",
            truly_undefined
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    // Locate the first reference site of each undefined symbol: scan
    // relocations, then attribute the reloc offset to the enclosing FUNC
    // symbol of the section being relocated.
    use crate::backend::elf::{SHN_UNDEF, STT_FUNC};
    let mut msg = String::from("undefined symbols:\n");
    for name in &truly_undefined {
        let mut site: Option<(String, Option<String>)> = None; // (object, function)
        'scan: for obj in objects {
            for (sec_idx, relas) in obj.relocations.iter().enumerate() {
                for rela in relas {
                    let si = rela.sym_idx as usize;
                    if si >= obj.symbols.len() {
                        continue;
                    }
                    if obj.symbols[si].name != ***name {
                        continue;
                    }
                    if obj.symbols[si].shndx != SHN_UNDEF {
                        continue;
                    }
                    // Find the FUNC symbol covering rela.offset in sec_idx.
                    let func = obj
                        .symbols
                        .iter()
                        .filter(|s| {
                            s.sym_type() == STT_FUNC
                                && s.shndx as usize == sec_idx
                                && s.value <= rela.offset
                                && (s.size == 0 || rela.offset < s.value + s.size)
                                && !s.name.is_empty()
                        })
                        .max_by_key(|s| s.value)
                        .map(|s| s.name.clone());
                    site = Some((obj.source_name.clone(), func.map(|s| s.to_string())));
                    break 'scan;
                }
            }
        }
        match site {
            Some((obj, Some(func))) => {
                msg.push_str(&format!(
                    "  undefined reference to `{}'\n    referenced from {} (in function `{}')\n",
                    name, obj, func
                ));
            }
            Some((obj, None)) => {
                msg.push_str(&format!(
                    "  undefined reference to `{}'\n    referenced from {}\n",
                    name, obj
                ));
            }
            None => {
                msg.push_str(&format!("  undefined reference to `{}'\n", name));
            }
        }
    }
    Err(msg.trim_end().to_string())
}
