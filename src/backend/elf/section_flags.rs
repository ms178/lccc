//! Section flags parsing for `.section` directives.
//!
//! Converts section name, flags string ("awx"), and type string ("@nobits")
//! into ELF `(sh_type, sh_flags)` tuples. Used by x86 and i686 ELF writers.

use super::constants::*;

/// Parse section name, flags string, and type into ELF section type and flags.
///
/// Returns `(sh_type, sh_flags)` based on well-known section names (`.text`,
/// `.data`, `.bss`, etc.) and optional explicit flags/type strings from the
/// `.section` directive.
pub fn parse_section_flags(
    name: &str,
    flags_str: Option<&str>,
    type_str: Option<&str>,
) -> (u32, u64) {
    let (default_type, default_flags) = match name {
        ".text" => (SHT_PROGBITS, SHF_ALLOC | SHF_EXECINSTR),
        ".data" => (SHT_PROGBITS, SHF_ALLOC | SHF_WRITE),
        ".bss" => (SHT_NOBITS, SHF_ALLOC | SHF_WRITE),
        ".rodata" => (SHT_PROGBITS, SHF_ALLOC),
        ".tdata" => (SHT_PROGBITS, SHF_ALLOC | SHF_WRITE | SHF_TLS),
        ".tbss" => (SHT_NOBITS, SHF_ALLOC | SHF_WRITE | SHF_TLS),
        ".init" => (SHT_PROGBITS, SHF_ALLOC | SHF_EXECINSTR),
        ".fini" => (SHT_PROGBITS, SHF_ALLOC | SHF_EXECINSTR),
        ".init_array" => (SHT_INIT_ARRAY, SHF_ALLOC | SHF_WRITE),
        ".fini_array" => (SHT_FINI_ARRAY, SHF_ALLOC | SHF_WRITE),
        n if n.starts_with(".text.") => (SHT_PROGBITS, SHF_ALLOC | SHF_EXECINSTR),
        n if n.starts_with(".data.") => (SHT_PROGBITS, SHF_ALLOC | SHF_WRITE),
        n if n.starts_with(".bss.") => (SHT_NOBITS, SHF_ALLOC | SHF_WRITE),
        n if n.starts_with(".rodata.") => (SHT_PROGBITS, SHF_ALLOC),
        n if n.starts_with(".note.") => (SHT_NOTE, 0),
        _ => (SHT_PROGBITS, 0),
    };

    if flags_str.is_none() && type_str.is_none() {
        return (default_type, default_flags);
    }

    let mut flags = 0u64;
    if let Some(f) = flags_str {
        for c in f.chars() {
            match c {
                'a' => flags |= SHF_ALLOC,
                'w' => flags |= SHF_WRITE,
                'x' => flags |= SHF_EXECINSTR,
                'M' => flags |= SHF_MERGE,
                'S' => flags |= SHF_STRINGS,
                'T' => flags |= SHF_TLS,
                'G' => flags |= SHF_GROUP,
                'o' => {} // SHF_LINK_ORDER - handle later
                _ => {}
            }
        }
    } else {
        flags = default_flags;
    }

    let section_type = if let Some(t) = type_str {
        match t {
            "@progbits" => SHT_PROGBITS,
            "@nobits" => SHT_NOBITS,
            "@note" => SHT_NOTE,
            "@init_array" => SHT_INIT_ARRAY,
            "@fini_array" => SHT_FINI_ARRAY,
            _ => default_type,
        }
    } else {
        default_type
    };

    (section_type, flags)
}

/// Fixed section type for the classic section names, mirroring GNU as.
///
/// GNU as assigns well-known section names a hard-coded type and ignores (with
/// a warning) any contradicting `@type` suffix: `.section .data,"aw",@nobits`
/// produces PROGBITS, because `.data` is always PROGBITS. Without this rule a
/// single stray `@nobits` declaration poisons every later `.section .data` in
/// the file — the kernel's compressed misc.c (`static int lines
/// __section(".data")` next to relocatable pointer data) lost its `.data`
/// contents exactly this way, and QEMU could not boot the resulting bzImage.
pub fn well_known_section_type(name: &str) -> Option<u32> {
    match name {
        ".bss" | ".tbss" | ".sbss" | ".lbss" => Some(SHT_NOBITS),
        ".init_array" => Some(SHT_INIT_ARRAY),
        ".fini_array" => Some(SHT_FINI_ARRAY),
        ".preinit_array" => Some(SHT_PREINIT_ARRAY),
        ".text" | ".data" | ".rodata" | ".tdata" | ".sdata" | ".init" | ".fini" | ".ctors"
        | ".dtors" | ".got" | ".got.plt" | ".plt" | ".comment" | ".note.GNU-stack"
        | ".eh_frame" | ".interp" | ".dynamic" | ".dynsym" | ".dynstr" | ".hash" | ".gnu.hash"
        | ".symtab" | ".strtab" | ".shstrtab" | ".jcr" => Some(SHT_PROGBITS),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn well_known_types_are_fixed() {
        assert_eq!(well_known_section_type(".data"), Some(SHT_PROGBITS));
        assert_eq!(well_known_section_type(".text"), Some(SHT_PROGBITS));
        assert_eq!(well_known_section_type(".rodata"), Some(SHT_PROGBITS));
        assert_eq!(well_known_section_type(".bss"), Some(SHT_NOBITS));
        assert_eq!(well_known_section_type(".tbss"), Some(SHT_NOBITS));
        assert_eq!(well_known_section_type(".init_array"), Some(SHT_INIT_ARRAY));
        // Custom sections are not fixed.
        assert_eq!(well_known_section_type(".lccc_pgo_cnts"), None);
        assert_eq!(well_known_section_type(".my_section"), None);
    }
}
