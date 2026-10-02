//! Input file loading for the AArch64 linker.
//!
//! Handles loading of object files (.o), archives (.a), shared libraries (.so),
//! and linker scripts. Delegates to `linker_common` for ELF parsing.

use crate::backend::elf::MAX_LINKER_SCRIPT_DEPTH;
use crate::common::fx_hash::FxHashMap;
use std::path::Path;

use super::elf::*;
use super::types::{GlobalSymbol, arm_should_replace_extra};
use crate::backend::linker_common;

pub fn load_file(
    path: &str,
    objects: &mut linker_common::ObjectSet,
    globals: &mut FxHashMap<String, GlobalSymbol>,
    needed_sonames: &mut Vec<String>,
    lib_paths: &[String],
    is_static: bool,
) -> Result<(), String> {
    load_file_depth(
        path,
        objects,
        globals,
        needed_sonames,
        lib_paths,
        is_static,
        0,
    )
}

fn load_file_depth(
    path: &str,
    objects: &mut linker_common::ObjectSet,
    globals: &mut FxHashMap<String, GlobalSymbol>,
    needed_sonames: &mut Vec<String>,
    lib_paths: &[String],
    is_static: bool,
    depth: usize,
) -> Result<(), String> {
    if std::env::var("LINKER_DEBUG").is_ok() {
        eprintln!("load_file: {}", path);
    }
    let data = std::fs::read(path).map_err(|e| format!("failed to read '{}': {}", path, e))?;

    // Regular archive
    if data.len() >= 8 && &data[0..8] == b"!<arch>\n" {
        return linker_common::load_archive_elf64(
            &data,
            path,
            objects,
            globals,
            EM_AARCH64,
            arm_should_replace_extra,
            false,
        );
    }

    // Thin archive
    if is_thin_archive(&data) {
        return linker_common::load_thin_archive_elf64(
            &data,
            path,
            objects,
            globals,
            EM_AARCH64,
            arm_should_replace_extra,
            false,
        );
    }

    // Not ELF? Try linker script (handles both GROUP and INPUT directives).
    //
    // `parse_linker_script_inputs` keeps `AS_NEEDED ( ... )` contents: a
    // script that is nothing but `GROUP ( AS_NEEDED ( -lgcc_s ) )` has no
    // non-as-needed entry, and dropping those entries made such a script
    // parse to "no inputs" and be rejected as a bad file format.
    if data.len() >= 4 && data[0..4] != ELF_MAGIC {
        if let Ok(text) = std::str::from_utf8(&data) {
            if let Some(inputs) = parse_linker_script_inputs(text) {
                if depth >= MAX_LINKER_SCRIPT_DEPTH {
                    return Err(format!(
                        "{}: linker scripts nested more than {} deep",
                        path, MAX_LINKER_SCRIPT_DEPTH
                    ));
                }
                let script_dir = Path::new(path)
                    .parent()
                    .map(|p| p.to_string_lossy().into_owned());
                for input in &inputs {
                    let resolved = match &input.entry {
                        LinkerScriptEntry::Path(p) => {
                            linker_common::resolve_script_path(p, script_dir.as_deref(), lib_paths)
                        }
                        LinkerScriptEntry::Lib(name) => resolve_lib(name, lib_paths),
                    }
                    .ok_or_else(|| {
                        format!(
                            "cannot find {} (referenced from linker script {})",
                            linker_common::script_input_spelling(input),
                            path
                        )
                    })?;
                    // `input.as_needed` has no effect here by construction:
                    // this backend gives every loaded DSO a `DT_NEEDED` entry
                    // (`load_shared_library_elf64` carries no as-needed
                    // state), so an `AS_NEEDED` marker can only be accepted,
                    // never acted on.  Keeping the entry — rather than
                    // dropping it as the filtered accessor did — is what
                    // makes such a script loadable at all.
                    load_file_depth(
                        &resolved,
                        objects,
                        globals,
                        needed_sonames,
                        lib_paths,
                        is_static,
                        depth + 1,
                    )?;
                }
                return Ok(());
            }
        }
        return Err(format!("{}: not a valid ELF object or archive", path));
    }

    // Shared library?
    if data.len() >= 18 {
        let e_type = read_u16(&data, 16);
        if e_type == ET_DYN {
            if is_static {
                return Ok(()); // Skip .so in static linking
            }
            return linker_common::load_shared_library_elf64(
                path,
                globals,
                needed_sonames,
                lib_paths,
            );
        }
    }

    let obj = parse_object(&data, path)?;
    linker_common::register_symbols_elf64(objects, &obj, globals, arm_should_replace_extra)?;
    objects.push(obj);
    Ok(())
}

pub fn resolve_lib(name: &str, paths: &[String]) -> Option<String> {
    crate::backend::linker_common::resolve_lib(name, paths, true)
}

pub fn resolve_lib_prefer_shared(name: &str, paths: &[String]) -> Option<String> {
    // For dynamic linking, prefer .so over .a
    crate::backend::linker_common::resolve_lib(name, paths, false)
}

#[cfg(test)]
mod tests {
    use super::GlobalSymbol;
    use super::load_file;
    use crate::backend::linker_common::ObjectSet;
    use crate::common::fx_hash::FxHashMap;

    /// Load `contents` as a linker script named `file_name` and return the
    /// loader's diagnostic (these inputs are deliberately unresolvable, so the
    /// error message is the observable).
    fn load_script(file_name: &str, contents: &str, tag: &str) -> String {
        let dir =
            std::env::temp_dir().join(format!("lccc_arm_script_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join(file_name);
        std::fs::write(&script, contents).unwrap();
        let mut objects = ObjectSet::new();
        let mut globals: FxHashMap<String, GlobalSymbol> = FxHashMap::default();
        let mut needed_sonames: Vec<String> = Vec::new();
        let lib_paths = vec![dir.display().to_string()];
        let result = load_file(
            &script.display().to_string(),
            &mut objects,
            &mut globals,
            &mut needed_sonames,
            &lib_paths,
            false,
        );
        let _ = std::fs::remove_dir_all(&dir);
        result.unwrap_err()
    }

    /// The regression: a script whose every input sits inside `AS_NEEDED`
    /// parsed to "no inputs" through the filtered accessor and was then
    /// rejected as a bad file format instead of being read as a script.
    #[test]
    fn all_as_needed_script_is_read_as_a_script() {
        let err = load_script(
            "liball_asneeded.so",
            "/* GNU ld script */\nGROUP ( AS_NEEDED ( -lfoo ) )\n",
            "asan",
        );
        assert!(
            !err.contains("not a valid ELF object or archive"),
            "an all-AS_NEEDED script must not be reported as a bad file format: {err}"
        );
        assert!(
            err.contains("cannot find -lfoo") && err.contains("liball_asneeded.so"),
            "diagnostic must name the operand and the script: {err}"
        );
    }

    /// A cycle between scripts ends in the shared nesting diagnostic rather
    /// than exhausting the stack.
    #[test]
    fn self_referencing_script_is_bounded() {
        let err = load_script(
            "loop.so",
            "/* GNU ld script */\nINPUT ( loop.so )\n",
            "loop",
        );
        assert!(err.contains("nested more than"), "{err}");
    }
}
