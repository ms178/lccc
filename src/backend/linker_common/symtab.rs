//! The static symbol table (`.symtab`) policy shared by the ELF64 linkers.
//!
//! `.symtab` is what `nm`, `objdump`, `perf` and gdb read a link's names from,
//! so its contents are a compatibility surface, not an implementation detail.
//! bfd copies every input symbol whose section survives into `.symtab` and
//! leaves out only what carries no user-visible meaning, and this module is
//! the one place in this tree that decides which is which. `arm/linker/debug.rs`
//! and `riscv/linker/debug.rs` each used to carry a copy of that decision; the
//! copies had already drifted (one kept an untyped local and the other did
//! not, and one approximated "placeholder" with a zero value), so the rules
//! live here now and both backends call them.
//!
//! Three rules, each with the bfd behaviour it reproduces:
//!
//! * A local symbol is kept when it names something a reader can find: a
//!   function, object, TLS object or untyped label. `STT_SECTION` and
//!   `STT_FILE` locals are structural and are not copied as symbols;
//!   assembler temporaries (`.L*`) and mapping symbols (`$x`, `$d`,
//!   `$xrv64...`) are architecture bookkeeping, not program names.
//! * A global is kept when this link defines it and does not import it. An
//!   import (dynamic, PLT, copy-relocated) has no address in this image;
//!   a `.symtab` entry for one would name a location the file does not own.
//! * A symbol this linker synthesized to satisfy an optional runtime hook
//!   (`_ITM_registerTMCloneTable`, ...) is a placeholder, not a definition:
//!   bfd leaves those weak and undefined, so they stay out of `.symtab`.
//!   The caller says so explicitly; the value is never inspected.
//!
//! Values follow the gABI: `st_value` is the address, except for `STT_TLS`
//! symbols in an executable or shared object, where it is the symbol's offset
//! in the TLS template ([`symtab_value`]).

use crate::backend::elf::{STT_FUNC, STT_NOTYPE, STT_OBJECT, STT_TLS};
use crate::backend::linker_common::shdr::{
    SectionLookup, ShdrSpec, SymSection, SymtabEntry, symtab_value, tls_base,
};

/// Whether a local symbol of an input object belongs in `.symtab`.
///
/// `sym_type` is the low nibble of `st_info`; `name` is the symbol's name.
pub fn include_local(name: &str, sym_type: u8) -> bool {
    matches!(sym_type, STT_NOTYPE | STT_OBJECT | STT_FUNC | STT_TLS)
        && !name.is_empty()
        && !name.starts_with(".L")
        && !name.starts_with('$')
}

/// Whether a global symbol belongs in `.symtab`.
///
/// `defined` is "this link defines it somewhere in the image" and `imported`
/// is "it is bound at run time from a shared object" (`needs_plt`,
/// `is_dynamic`, a copy relocation) or unresolved.
pub fn include_global(defined: bool, imported: bool) -> bool {
    defined && !imported
}

/// Builds a `.symtab` in one pass over a finished header table.
///
/// The section lookup, the TLS base and the per-section `SymSection` are
/// computed once: a link resolves one query per symbol, and the linear scan
/// this replaces recomputed the image end inside every one of them.
pub struct SymtabBuilder<'a> {
    lookup: SectionLookup<'a>,
    tls_base: Option<u64>,
    /// `SymSection` per header index, memoized: the name is formatted once
    /// per section rather than once per symbol.
    named: Vec<Option<SymSection>>,
    entries: Vec<SymtabEntry>,
}

impl<'a> SymtabBuilder<'a> {
    /// Index `headers`, which must be final (as `order_headers` returns them).
    pub fn new(headers: &'a [ShdrSpec]) -> Self {
        Self {
            tls_base: tls_base(headers),
            named: vec![None; headers.len()],
            lookup: SectionLookup::new(headers),
            entries: Vec::new(),
        }
    }

    /// Push one symbol. `info` is `st_info` (binding and type), `other` is
    /// `st_other` (visibility); a TLS type takes the template offset as its
    /// value, and the section is resolved by name for a `__start_`/`__stop_`
    /// delimiter and by address otherwise.
    pub fn push(&mut self, name: &str, info: u8, other: u8, value: u64, size: u64) {
        let tls = info & 0xf == STT_TLS;
        let section = self.section(name, value, tls);
        self.entries.push(SymtabEntry {
            name: name.to_string(),
            info,
            other,
            section,
            value: symtab_value(value, tls, self.tls_base),
            size,
        });
    }

    /// Push a symbol that is absolute: an `SHN_ABS` definition, or one whose
    /// section this link dropped.
    pub fn push_absolute(&mut self, name: &str, info: u8, other: u8, value: u64, size: u64) {
        let tls = info & 0xf == STT_TLS;
        self.entries.push(SymtabEntry {
            name: name.to_string(),
            info,
            other,
            section: SymSection::Abs,
            value: symtab_value(value, tls, self.tls_base),
            size,
        });
    }

    fn section(&mut self, name: &str, value: u64, tls: bool) -> SymSection {
        let Some(index) = self.lookup.header_of(name, value, tls) else {
            return SymSection::Abs;
        };
        self.named[index]
            .get_or_insert_with(|| SymSection::Named(self.lookup.headers()[index].name.clone()))
            .clone()
    }

    /// The entries, in the order pushed.
    pub fn finish(self) -> Vec<SymtabEntry> {
        self.entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::elf::{SHF_ALLOC, SHF_TLS, SHF_WRITE};

    fn header(name: &str, flags: u64, addr: u64, size: u64) -> ShdrSpec {
        ShdrSpec::new(name, 1, flags, addr, addr, size)
    }

    #[test]
    fn local_symbol_policy_matches_bfd() {
        // Program names a reader can find.
        assert!(include_local("main", STT_FUNC));
        assert!(include_local("counter", STT_OBJECT));
        assert!(include_local("tls_var", STT_TLS));
        // Untyped assembler labels are real symbols: bfd keeps them, and this
        // is exactly the case the two backend copies disagreed on.
        assert!(include_local("asm_label", STT_NOTYPE));
        // Structural and architectural bookkeeping.
        assert!(!include_local("", STT_FUNC));
        assert!(!include_local(".Ltmp0", STT_NOTYPE));
        assert!(!include_local("$x", STT_NOTYPE));
        assert!(!include_local("$xrv64i2p1", STT_NOTYPE));
        assert!(!include_local("file.c", 4)); // STT_FILE
        assert!(!include_local(".text", 3)); // STT_SECTION
    }

    #[test]
    fn global_symbol_policy() {
        assert!(include_global(true, false), "a definition is kept");
        assert!(!include_global(true, true), "an import has no address here");
        assert!(
            !include_global(false, false),
            "an undefined global is no entry"
        );
    }

    #[test]
    fn builder_resolves_sections_and_tls_offsets() {
        let headers = vec![
            header(".text", SHF_ALLOC, 0x1000, 0x100),
            header(".tdata", SHF_ALLOC | SHF_WRITE | SHF_TLS, 0x2000, 0x10),
            header(".data", SHF_ALLOC | SHF_WRITE, 0x3000, 0x20),
        ];
        let mut builder = SymtabBuilder::new(&headers);
        builder.push("main", 0x12, 0, 0x1010, 8);
        builder.push("tls_var", 0x16, 0, 0x2008, 4); // STB_GLOBAL | STT_TLS
        builder.push("counter", 0x11, 0, 0x3010, 4);
        builder.push_absolute("absolute_sym", 0x11, 0, 0x1234, 0);
        let entries = builder.finish();
        assert_eq!(entries[0].section, SymSection::Named(".text".into()));
        assert_eq!(entries[0].value, 0x1010);
        // gABI: an STT_TLS value is the template offset, not the address.
        assert_eq!(entries[1].section, SymSection::Named(".tdata".into()));
        assert_eq!(entries[1].value, 0x8);
        assert_eq!(entries[2].section, SymSection::Named(".data".into()));
        assert_eq!(entries[3].section, SymSection::Abs);
        assert_eq!(entries[3].value, 0x1234);
    }

    #[test]
    fn builder_names_start_stop_delimiters() {
        // `.empty` is a declared section of the table with no content, so no
        // address identifies it; the delimiter's name does.
        let headers = vec![
            header(".text", SHF_ALLOC, 0x1000, 0x100),
            header(".empty", SHF_ALLOC, 0x1100, 0),
        ];
        let mut builder = SymtabBuilder::new(&headers);
        builder.push("__start_empty", 0x11, 0, 0x1100, 0);
        let entries = builder.finish();
        assert_eq!(entries[0].section, SymSection::Named(".empty".into()));
    }

    #[test]
    fn an_address_outside_the_image_is_absolute() {
        let headers = vec![header(".text", SHF_ALLOC, 0x1000, 0x100)];
        let mut builder = SymtabBuilder::new(&headers);
        builder.push("outside", 0x11, 0, 0x9000, 0);
        let entries = builder.finish();
        assert_eq!(entries[0].section, SymSection::Abs);
    }
}
