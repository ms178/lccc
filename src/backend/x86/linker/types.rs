//! x86-64 linker types and constants.
//!
//! Defines the `GlobalSymbol` type used by all linker phases, plus
//! architecture-specific constants (base address, page size, interpreter path).

use super::elf::{SHN_ABS, SHN_COMMON, SHN_UNDEF};
use crate::backend::linker_common::{self, Elf64Symbol, GlobalSymbolOps};

/// Base virtual address for the executable (standard non-PIE x86-64 address)
pub const BASE_ADDR: u64 = 0x400000;
/// Page size for alignment
pub const PAGE_SIZE: u64 = 0x1000;
/// Dynamic linker path
pub const INTERP: &[u8] = b"/lib64/ld-linux-x86-64.so.2\0";

/// A resolved global symbol.
///
/// This struct has x86-specific dynamic linking fields (plt_idx, got_idx,
/// copy_reloc, from_lib, version, lib_sym_value) in addition to the common
/// fields needed by the shared linker infrastructure.
#[derive(Clone)]
pub struct GlobalSymbol {
    pub value: u64,
    pub size: u64,
    pub info: u8,
    pub defined_in: Option<usize>,
    pub from_lib: Option<String>,
    pub plt_idx: Option<usize>,
    pub got_idx: Option<usize>,
    pub section_idx: u16,
    pub is_dynamic: bool,
    pub copy_reloc: bool,
    /// A shared-library function whose PLT entry serves as its address in
    /// this executable (a non-PIC address-of: `mov $f`, `R_X86_64_64` in
    /// read-only data, a non-branch `R_X86_64_PC32`).  Its `.dynsym` entry
    /// then carries the PLT entry's address as `st_value` -- the "canonical
    /// PLT" convention -- so ld.so resolves every other object's references
    /// to that same address and `&f` compares equal everywhere.
    pub canonical_plt: bool,
    /// Merged ELF visibility (`STV_*`) of every relocatable-object symbol
    /// table entry naming this symbol -- definition and references alike --
    /// combined by the gABI "most constraining wins" rule
    /// (`linker_common::merge_object_visibility`).  Shared-library entries
    /// never contribute: a DSO's visibility is its own business.  Anything
    /// other than `STV_DEFAULT` makes the symbol non-preemptible; `HIDDEN`
    /// and `INTERNAL` additionally keep it out of `.dynsym`.
    pub visibility: u8,
    pub lib_sym_value: u64,
    /// True when `lib_sym_value` falls inside a `PF_X` LOAD segment of the
    /// defining library (see `DynSymbol::in_exec_segment`). Only meaningful
    /// for dynamic symbols; lets the COPY-vs-PLT classifier tell untyped
    /// data (copy) from untyped code (PLT) when `info` is `STT_NOTYPE`.
    pub lib_in_exec: bool,
    pub version: Option<String>,
    /// The value is a link-time constant rather than an address in this
    /// output: an `SHN_ABS` definition in an object, or a `--defsym` whose
    /// expression GNU ld evaluates to an absolute value (a number, a
    /// difference of two addresses, ...).  It never moves with the load base,
    /// so a PIE or shared object must not add `R_X86_64_RELATIVE` for it,
    /// exports it as `SHN_ABS`, and may encode it as an immediate.
    ///
    /// Not derivable from `section_idx`: every symbol the linker creates --
    /// `__ehdr_start`, `_end`, `__start_SEC`, `--defsym x=_start+4` -- is
    /// stored with `section_idx == SHN_ABS` (it belongs to no input section)
    /// yet is an ADDRESS, which slides with the load base like any other.
    pub absolute: bool,
}

/// GOT slots (or slot pairs) of LOCAL symbols, keyed by (object index,
/// symbol index, ADDRESS ADDEND) and numbered in first-reference order, so
/// the layout is deterministic.  Section symbols (empty names) are keyed like
/// any other local: `mov .Lfoo@GOTPCREL(%rip)` usually reaches the linker as
/// one.
///
/// WHY THE KEY CARRIES AN ADDEND.  `R_X86_64_GOT64` is an ABSOLUTE 64-bit
/// reference: its slot holds `S + A` and the field holds that slot's plain
/// offset from `_GLOBAL_OFFSET_TABLE_`.  Every other GOT form's addend is a
/// displacement bias and its slot holds plain `S`.  The two spellings of a
/// local address differ between assemblers -- `$lvar@GOT` is
/// `R_X86_64_GOT64 lvar + 0` on binutils >= 2.44 but
/// `R_X86_64_GOT64 .data + 8` on <= 2.43, one address written two ways -- and
/// a section symbol is shared by every local in its section, so `.data + 0`,
/// `.data + 8` and `.data + 16` are three addresses behind ONE symbol.  Keyed
/// by symbol alone they collapse to one slot, and the field then points at
/// (slot + A): a second local reads eight bytes past a slot, and the first
/// reads the section base.  Measured against GNU ld 2.42 on a `.data` with
/// three locals at offsets 0, 16 and 32: lccc-ld and GNU ld BOTH answer
/// `1 0 0` instead of `1 1 1`.  The addend belongs to the slot's VALUE, not
/// to the field; [`super::elf::got_slot_addr_addend`] returns it for
/// `R_X86_64_GOT64` and 0 for every other GOT type, which keeps the slot
/// value and the field formula independent of each other.
#[derive(Default)]
pub struct LocalSlots {
    order: Vec<(usize, usize, i64)>,
    index: crate::common::fx_hash::FxHashMap<(usize, usize, i64), usize>,
}

impl LocalSlots {
    pub fn insert(&mut self, key: (usize, usize, i64)) {
        if let std::collections::hash_map::Entry::Vacant(e) = self.index.entry(key) {
            e.insert(self.order.len());
            self.order.push(key);
        }
    }
    pub fn len(&self) -> usize {
        self.order.len()
    }
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }
    pub fn get(&self, key: (usize, usize, i64)) -> Option<usize> {
        self.index.get(&key).copied()
    }
    /// Keys in slot order.
    pub fn keys(&self) -> &[(usize, usize, i64)] {
        &self.order
    }
}

impl GlobalSymbolOps for GlobalSymbol {
    fn is_defined(&self) -> bool {
        self.defined_in.is_some()
    }
    fn is_dynamic(&self) -> bool {
        self.is_dynamic
    }
    fn info(&self) -> u8 {
        self.info
    }
    fn section_idx(&self) -> u16 {
        self.section_idx
    }
    fn has_copy_reloc(&self) -> bool {
        self.copy_reloc
    }
    fn value(&self) -> u64 {
        self.value
    }
    fn size(&self) -> u64 {
        self.size
    }
    fn visibility(&self) -> u8 {
        self.visibility
    }
    fn set_visibility(&mut self, visibility: u8) {
        self.visibility = visibility;
    }
    fn new_defined(obj_idx: usize, sym: &Elf64Symbol) -> Self {
        GlobalSymbol {
            value: sym.value,
            size: sym.size,
            info: sym.info,
            defined_in: Some(obj_idx),
            from_lib: None,
            plt_idx: None,
            got_idx: None,
            section_idx: sym.shndx,
            is_dynamic: false,
            copy_reloc: false,
            canonical_plt: false,
            visibility: 0,
            lib_sym_value: 0,
            lib_in_exec: false,
            version: None,
            absolute: sym.shndx == SHN_ABS,
        }
    }
    fn new_common(obj_idx: usize, sym: &Elf64Symbol) -> Self {
        GlobalSymbol {
            value: sym.value,
            size: sym.size,
            info: sym.info,
            defined_in: Some(obj_idx),
            from_lib: None,
            plt_idx: None,
            got_idx: None,
            section_idx: SHN_COMMON,
            is_dynamic: false,
            copy_reloc: false,
            canonical_plt: false,
            visibility: 0,
            lib_sym_value: 0,
            lib_in_exec: false,
            version: None,
            absolute: false,
        }
    }
    fn new_undefined(sym: &Elf64Symbol) -> Self {
        GlobalSymbol {
            value: 0,
            size: 0,
            info: sym.info,
            defined_in: None,
            from_lib: None,
            plt_idx: None,
            got_idx: None,
            section_idx: SHN_UNDEF,
            is_dynamic: false,
            copy_reloc: false,
            canonical_plt: false,
            visibility: 0,
            lib_sym_value: 0,
            lib_in_exec: false,
            version: None,
            absolute: false,
        }
    }
    fn set_common_bss(&mut self, bss_offset: u64) {
        self.value = bss_offset;
        self.section_idx = 0xffff;
    }
    fn new_dynamic(dsym: &linker_common::DynSymbol, soname: &str) -> Self {
        GlobalSymbol {
            value: 0,
            size: dsym.size,
            info: dsym.info,
            defined_in: None,
            from_lib: Some(soname.to_string()),
            plt_idx: None,
            got_idx: None,
            section_idx: SHN_UNDEF,
            is_dynamic: true,
            copy_reloc: false,
            canonical_plt: false,
            visibility: 0,
            lib_sym_value: dsym.value,
            lib_in_exec: dsym.in_exec_segment,
            version: dsym.version.clone(),
            absolute: false,
        }
    }
}

/// For x86, a dynamic definition should be replaced by a static definition.
pub fn x86_should_replace_extra(existing: &GlobalSymbol) -> bool {
    existing.is_dynamic
}
