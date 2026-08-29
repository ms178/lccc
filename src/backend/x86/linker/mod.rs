//! Native x86-64 ELF linker.
//!
//! Links ELF relocatable object files (.o) and static archives (.a) into
//! a dynamically-linked ELF executable. Resolves undefined symbols against
//! shared libraries (e.g., libc.so.6) and generates PLT/GOT entries for
//! dynamic function calls.
//!
//! ## Module structure
//!
//! - `elf`: ELF64 constants, type aliases, parsing (delegates to shared linker_common)
//! - `types`: `GlobalSymbol` struct, `GlobalSymbolOps` impl, arch constants
//! - `input`: Input file loading (objects, archives, shared libs, linker scripts)
//! - `plt_got`: PLT/GOT entry construction and IFUNC collection
//! - `link`: Orchestration - `link_builtin` and `link_shared` entry points
//! - `emit_exec`: Executable emission (both static and dynamic)
//! - `emit_shared`: Shared library (.so) emission

#[allow(dead_code)]
pub mod elf;
mod emit_exec;
pub mod emit_rel;
pub mod emit_script;
mod emit_shared;
pub mod icf;
mod input;
mod layout_plan;
mod link;
mod parallel_reloc;
mod plt_got;
pub mod types;
pub use icf::parse_icf_mode;

#[cfg(not(feature = "gcc_linker"))]
pub use link::link_builtin;

/// Self-contained input loader for the standalone `lccc-ld` driver.
///
/// Loads a mix of .o files and .a archives with --start-group semantics over
/// the whole input list (iterated to a fixed point), honoring per-input
/// --whole-archive state. `undefined` is the `-u SYM` / `--undefined=SYM`
/// set: those names are entered as undefined globals *before* the first
/// archive scan so members that define them are pulled in (GNU ld/mold).
/// Script-driven kernel links (`-T compressed/vmlinux.lds -u efi_pe_entry`)
/// depend on this; dropping `-u` left the EFI mixed stub out of the
/// decompressor and header.S failed with "32-bit and 64-bit EFI entry
/// points do not match".
pub fn load_inputs_for_ld(
    inputs: &[(String, bool)],
    objects: &mut Vec<crate::backend::linker_common::Elf64Object>,
    undefined: &[String],
) -> Result<(), String> {
    let mut globals: crate::common::fx_hash::FxHashMap<String, types::GlobalSymbol> =
        crate::common::fx_hash::FxHashMap::default();
    for sym_name in undefined {
        if globals.contains_key(sym_name) {
            continue;
        }
        let fake = crate::backend::linker_common::Elf64Symbol {
            name_idx: 0,
            name: crate::backend::linker_common::SymStr::new(sym_name),
            info: 1 << 4, // STB_GLOBAL, STT_NOTYPE
            other: 0,
            shndx: 0,
            value: 0,
            size: 0,
        };
        globals.insert(
            sym_name.clone(),
            <types::GlobalSymbol as crate::backend::linker_common::GlobalSymbolOps>::new_undefined(
                &fake,
            ),
        );
    }
    let mut needed_sonames: Vec<String> = Vec::new();
    let lib_paths: Vec<String> = Vec::new();
    let mut fully_loaded: crate::common::fx_hash::FxHashSet<String> =
        crate::common::fx_hash::FxHashSet::default();
    let mut changed = true;
    while changed {
        changed = false;
        let before = objects.len();
        for (path, wa) in inputs {
            // Plain .o files and whole-archives are fully consumed on first
            // load; re-loading them would duplicate every symbol. Only
            // selective archives participate in group re-scanning.
            let is_selective_archive = !*wa
                && (path.ends_with(".a") || {
                    // sniff: archives start with "!<arch>\n" or "!<thin>\n"
                    std::fs::File::open(path)
                        .ok()
                        .and_then(|mut f| {
                            use std::io::Read;
                            let mut m = [0u8; 8];
                            f.read_exact(&mut m)
                                .ok()
                                .map(|_| &m == b"!<arch>\n" || &m == b"!<thin>\n")
                        })
                        .unwrap_or(false)
                });
            if !is_selective_archive && fully_loaded.contains(path) {
                continue;
            }
            input::load_file(
                path,
                objects,
                &mut globals,
                &mut needed_sonames,
                &lib_paths,
                *wa,
            )?;
            if !is_selective_archive {
                fully_loaded.insert(path.clone());
            }
        }
        if objects.len() != before {
            changed = true;
        }
    }
    Ok(())
}
#[cfg(not(feature = "gcc_linker"))]
pub use link::link_shared;
