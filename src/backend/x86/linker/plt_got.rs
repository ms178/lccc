//! PLT/GOT construction and IFUNC collection for the x86-64 linker.
//!
//! Scans object file relocations to determine which symbols need PLT stubs
//! or GOT entries, and collects IFUNC symbols for IRELATIVE relocations.

use crate::common::fx_hash::FxHashMap;

use super::elf::*;
use super::types::{GlobalSymbol, LocalSlots};

/// Local (`STB_LOCAL`) IFUNC symbols, as `(object index, symbol index)`.
///
/// `collect_ifunc_symbols` can only see global IFUNCs, because it walks
/// `globals` -- and a `static int fn(void) __attribute__((ifunc("rsv")))` never
/// gets promoted there.  Such a symbol used to resolve through the ordinary
/// section path, which binds the call site straight to the *resolver*: the
/// caller receives the implementation's address as the call's "return value"
/// and prints something like `4199558` where `1` was meant.  Nothing diagnoses
/// it; the program simply computes the wrong answer.
///
/// Keyed by index rather than name because local symbols legitimately repeat
/// across objects and must not be merged.
pub(super) fn collect_local_ifuncs(
    objects: &[crate::backend::linker_common::Elf64Object],
    dead_sections: &crate::common::fx_hash::FxHashSet<(usize, usize)>,
) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for (obj_idx, obj) in objects.iter().enumerate() {
        for (sym_idx, sym) in obj.symbols.iter().enumerate() {
            if sym.sym_type() != STT_GNU_IFUNC
                || !sym.is_local()
                || sym.shndx == crate::backend::elf::SHN_UNDEF
                || sym.shndx == crate::backend::elf::SHN_ABS
            {
                continue;
            }
            // A resolver in a collected section cannot run; skip it rather than
            // emit an IRELATIVE pointing at unmapped memory.
            if dead_sections.contains(&(obj_idx, sym.shndx as usize)) {
                continue;
            }
            out.push((obj_idx, sym_idx));
        }
    }
    // Deterministic slot order: the IPLT, the IFUNC GOT and the IRELATIVE table
    // are all indexed by position, so it must not depend on iteration order.
    out.sort_unstable();
    out
}

pub(super) fn collect_ifunc_symbols(
    globals: &FxHashMap<String, GlobalSymbol>,
    _is_static: bool,
) -> Vec<String> {
    // IFUNCs need IRELATIVE handling in BOTH static and dynamic executables.
    // Static: .rela.iplt applied by glibc csu via __rela_iplt_start/end.
    // Dynamic: IRELATIVE entries appended to .rela.dyn, applied by ld.so.
    // Without this, calls bind directly to the RESOLVER, so callers receive
    // the implementation's address as the "return value" of the call.
    let mut ifunc_symbols: Vec<String> = globals
        .iter()
        .filter(|(_, g)| g.defined_in.is_some() && (g.info & 0xf) == STT_GNU_IFUNC)
        .map(|(n, _)| n.clone())
        .collect();
    ifunc_symbols.sort();
    ifunc_symbols
}

/// One absolute (`R_X86_64_64`) relocation against a dynamic data symbol.
///
/// These cannot be resolved at link time in an ET_EXEC image: the target lives
/// in a shared library whose load address is unknown until `ld.so` maps it.
/// GNU ld emits a dynamic `R_X86_64_64` for each one so the loader patches the
/// storage in place; see `AbsDynReloc` use in `emit_exec`.
#[derive(Clone, Debug)]
pub(super) struct AbsDynReloc {
    /// Symbol the relocation refers to (index into the dynamic symbol table is
    /// resolved later, once that table is laid out).
    pub name: String,
    /// Input object and section holding the storage to be patched.
    pub obj_idx: usize,
    pub sec_idx: usize,
    /// Offset of the storage within that input section.
    pub offset: u64,
    /// Addend to hand the loader (e.g. `+0x10` to skip a vtable's RTTI header).
    pub addend: i64,
}

/// One `R_X86_64_64` relocation that a PIE turns into `R_X86_64_RELATIVE`.
///
/// A position-independent executable is mapped at a base chosen by the kernel,
/// so every absolute address stored in a writable section has to be slid by
/// that base at load time.  `ld.so` does the sliding; our job is to hand it one
/// `RELATIVE` entry per such storage, carrying the link-time value as the
/// addend.
///
/// The entry stores *coordinates*, not a value: the value depends on final
/// addresses, which do not exist yet when this list is built.  `emit_exec`
/// resolves the coordinates exactly once, when it fills `.rela.dyn`.  Building
/// the list here -- in the same relocation walk that already decides PLT, GOT,
/// copy relocs and `AbsDynReloc` -- is what makes `DT_RELASZ` agree with the
/// bytes written: the count and the emission read the same `Vec`, so they
/// cannot drift apart.  A separate counting pass with its own predicate would
/// be free to disagree, and a wrong `DT_RELASZ` silently corrupts the image.
pub(super) struct PieRelative {
    pub obj_idx: usize,
    pub sec_idx: usize,
    pub offset: u64,
    pub addend: i64,
    /// `sym_idx` of the relocation's symbol, kept so the emitter resolves the
    /// target through the identical path the non-PIE applier uses (including
    /// string-merge remapping, COMDAT losers and ICF folds).
    pub sym_idx: usize,
}

/// Whether an `R_X86_64_64` against global `g`, stored in a section with
/// flags `sh_flags`, is left for ld.so to fill through a symbolic dynamic
/// `R_X86_64_64` (an `AbsDynReloc`) rather than resolved by this link.
///
/// * A shared-library DATA object that was not copy-relocated: its address
///   exists only once the library is mapped.
/// * A shared-library FUNCTION, in writable storage of a PIE: the loader has
///   to write the slot anyway (a PIE is relocated as a whole), so it may as
///   well write the function's real address -- which is what GNU ld and lld
///   do.  An indirect call through the pointer then goes straight to the
///   function instead of through our PLT, and no canonical PLT entry is
///   needed.  Read-only storage, or a non-PIE (whose image is not relocated
///   at all), instead gets the canonical PLT entry, see
///   [`GlobalSymbol::canonical_plt`].
///
/// Every input to the decision is final once `create_plt_got` has run
/// (`copy_reloc` is only ever set on `STT_OBJECT`s, and it is only consulted
/// for those), so the planner, the `RELATIVE` filter and the relocation
/// applier all agree by calling this one function.
pub(super) fn abs64_defers_to_loader(g: &GlobalSymbol, sh_flags: u64, is_pie: bool) -> bool {
    g.is_dynamic
        && !g.copy_reloc
        && ((g.info & 0xf) == STT_OBJECT || (is_pie && sh_flags & SHF_WRITE != 0))
}

/// True when the value the relocation applier stores for an `R_X86_64_64`
/// against `g` is an address inside *this* output, and therefore has to be
/// slid by the load base in a PIE (an `R_X86_64_RELATIVE`).
///
/// Mirrors `emit_exec`'s applier arm for `R_X86_64_64` case for case, because
/// a disagreement is a wrong image either way: a pointer that is never slid,
/// or a slide applied on top of an address the loader is about to overwrite.
///
/// Must be asked after `create_plt_got` has finished: a copy-relocated
/// symbol's storage is our own `.bss` copy, and that is only known once every
/// relocation has been seen.  (Asking during the scan, as this used to be,
/// made the answer depend on relocation order: `int *p = &lib_var;` lost its
/// slide whenever a later PC32 turned `lib_var` into a copy relocation.)
pub(super) fn abs64_value_is_local(g: &GlobalSymbol, sh_flags: u64, is_pie: bool) -> bool {
    if g.copy_reloc {
        return true;
    }
    if g.is_dynamic {
        // Either the loader writes the whole value, or we store our own
        // canonical PLT entry (a local address).
        return !abs64_defers_to_loader(g, sh_flags, is_pie);
    }
    // A definition we own: a normal local/global definition, or one of the
    // synthetic `__lccc.strmerge.N` pool symbols that string merging
    // substitutes for references into merged string sections (those carry the
    // per-string offset in the *addend* and look undefined -- `shndx == 0` --
    // but are entirely local).  An absolute definition's value is the same
    // at every load address.
    g.defined_in.is_some() && !g.absolute
}

/// Whether a PIE `R_X86_64_64` through `sym`, stored in a section with flags
/// `sh_flags`, becomes an `R_X86_64_RELATIVE`.  Everything that resolves to an
/// address inside this output needs one -- including references through
/// section symbols, which is how the compiler points an array of
/// `const char *` at merged string literals.  Those are the easy ones to
/// miss, and missing one leaves a pointer holding its link-time offset, which
/// faults on first use under ASLR.
pub(super) fn pie_needs_relative(
    sym: &crate::backend::linker_common::Elf64Symbol,
    globals: &FxHashMap<String, GlobalSymbol>,
    sh_flags: u64,
) -> bool {
    if !sym.name.is_empty() && !sym.is_local() {
        if let Some(g) = globals.get(sym.name.as_str()) {
            return abs64_value_is_local(g, sh_flags, true);
        }
        if sym.is_weak() {
            return false; // undefined weak resolves to 0; nothing to slide
        }
    }
    !sym.is_undefined() && sym.shndx != SHN_ABS
}

/// Whether a PIE GOT slot for `g` holds an address inside this output (and so
/// needs an `R_X86_64_RELATIVE`).  A shared-library symbol's slot is always
/// filled by the loader (`GLOB_DAT`, or `TPOFF64` for TLS) -- never with our
/// PLT entry, because ld.so resolves every *other* object's references to the
/// library's definition unless `.dynsym` says otherwise, and a PLT address in
/// the slot would then compare unequal to the library's own `&f`.  A TLS slot
/// holds a thread-pointer offset, which must not be slid either.
pub(super) fn got_slot_holds_local_address(g: &GlobalSymbol) -> bool {
    if (g.info & 0xf) == STT_TLS {
        return false;
    }
    if g.copy_reloc {
        return true;
    }
    !g.is_dynamic && g.defined_in.is_some() && !g.absolute
}

/// How an executable's GOT-indirect reference to `g` may address it
/// directly instead, or `None` if it must keep its slot (see
/// `elf::gotpcrelx_relaxation` for the instruction side).  The symbol must be
/// defined in the executable itself -- nothing can preempt it -- and must not
/// be an IFUNC (its canonical address is the IPLT stub the IFUNC GOT serves)
/// or a copy-relocated DSO object.  An absolute definition is a
/// [`GotTarget::Absolute`], which PIE may still encode as an immediate.
///
/// `create_plt_got` and the relocation pass both ask this exact question of
/// the same symbol table, so a reference whose slot was elided is always one
/// that gets rewritten.  That is why a linker-created absolute symbol (a
/// `--defsym` constant or expression) keeps its slot: an expression's value
/// is only computed after layout, so the planner would decide on a
/// placeholder and the applier on the real value.
pub(super) fn exec_got_target(g: &GlobalSymbol) -> Option<GotTarget> {
    if g.defined_in.is_none() || g.is_dynamic || g.copy_reloc || (g.info & 0xf) == STT_GNU_IFUNC {
        return None;
    }
    if !g.absolute {
        return Some(GotTarget::Image);
    }
    (g.defined_in != Some(usize::MAX)).then_some(GotTarget::Absolute(g.value))
}

/// Whether an executable's Initial-Exec reference to `g` may become
/// Local-Exec (`elf::gottpoff_ie_to_le` decides the instruction side): the
/// TLS symbol is defined in the executable, so its offset from the thread
/// pointer is a link-time constant.  A shared library's TLS keeps its slot
/// and `R_X86_64_TPOFF64`; so does an undefined weak one, whose slot stays
/// zero.  Like [`exec_got_target`], `create_plt_got` and the relocation pass
/// both ask this, so every reference left unrelaxed has a slot.  (Every
/// LOCAL TLS symbol qualifies.)
pub(super) fn exec_ie_target_local(g: &GlobalSymbol) -> bool {
    !g.is_dynamic && g.defined_in.is_some()
}

/// Whether a LOCAL symbol is thread-local: an `STT_TLS` symbol, or a
/// section symbol of a TLS section (an assembler may relocate against
/// either).  Its GOT slot holds a TP offset rather than an address.
pub(super) fn local_is_tls(
    obj: &crate::backend::linker_common::Elf64Object,
    sym: &crate::backend::linker_common::Elf64Symbol,
) -> bool {
    sym.sym_type() == STT_TLS
        || obj
            .sections
            .get(sym.shndx as usize)
            .is_some_and(|sec| sym.shndx != SHN_ABS && sec.flags & SHF_TLS != 0)
}

/// The relaxation target of a GOT-indirect reference through a LOCAL
/// symbol (never preemptible; a local IFUNC resolves to its IPLT stub).
/// Shared by the executable and shared-object planners and appliers.
pub(super) fn local_got_target(sym: &crate::backend::linker_common::Elf64Symbol) -> GotTarget {
    if sym.shndx == SHN_ABS {
        GotTarget::Absolute(sym.value)
    } else {
        GotTarget::Image
    }
}

/// Whether the relocated field at `offset` is the rel32 operand of a direct
/// branch (`call`/`jmp`/`jcc rel32`), i.e. a use that does not take the
/// target's address.  Only code can hold a branch; in any other section a
/// PC-relative field (`.long f - .`) is data that *does* take the address.
/// In code the byte before a RIP-relative disp32 is otherwise always a ModRM
/// byte of the form `00 reg 101` (0x05..0x3d), which can never be mistaken
/// for the `e8`/`e9` opcodes; a `0f 8x` pair is a two-byte `jcc`.
pub(super) fn is_branch_rel32(sh_flags: u64, data: &[u8], offset: u64) -> bool {
    if sh_flags & SHF_EXECINSTR == 0 {
        return false;
    }
    let off = offset as usize;
    if off == 0 || off > data.len() {
        return false;
    }
    let op = data[off - 1];
    op == 0xe8 || op == 0xe9 || (off >= 2 && data[off - 2] == 0x0f && (0x80..=0x8f).contains(&op))
}

pub(super) fn create_plt_got(
    objects: &[ElfObject],
    globals: &mut FxHashMap<String, GlobalSymbol>,
    is_pie: bool,
) -> (
    Vec<String>,
    Vec<(String, bool)>,
    Vec<AbsDynReloc>,
    Vec<PieRelative>,
    LocalSlots,
) {
    // Ordered vectors preserve deterministic layout; the shadow HashSets make
    // membership tests O(1). With tens of thousands of GOT symbols (e.g. a
    // kernel-sized link) the previous Vec::contains scans were O(n^2) and
    // dominated total link time.
    use crate::common::fx_hash::FxHashSet;
    let mut plt_names: Vec<String> = Vec::new();
    let mut plt_set: FxHashSet<String> = FxHashSet::default();
    let mut got_only_names: Vec<String> = Vec::new();
    let mut got_only_set: FxHashSet<String> = FxHashSet::default();
    let mut copy_reloc_names: Vec<String> = Vec::new();
    let mut copy_reloc_set: FxHashSet<String> = FxHashSet::default();
    let mut abs_dyn_relocs: Vec<AbsDynReloc> = Vec::new();
    let mut pie_relative: Vec<PieRelative> = Vec::new();
    let mut canonical_plt_set: FxHashSet<String> = FxHashSet::default();
    // GOT slots of LOCAL symbols (section symbols included): a GOT-indirect
    // reference to one that cannot be rewritten -- a `call *` behind a REX
    // prefix, an addend other than -4, a plain GOTPCREL, a large-model
    // `@GOT` -- still loads the slot's contents, so the slot must exist and
    // hold the symbol's address.
    let mut local_got = LocalSlots::default();

    for (obj_i, obj) in objects.iter().enumerate() {
        for sec_idx in 0..obj.sections.len() {
            let sh_flags = obj.sections[sec_idx].flags;
            for rela in &obj.relocations[sec_idx] {
                let si = rela.sym_idx as usize;
                if si >= obj.symbols.len() {
                    continue;
                }
                let sym = &obj.symbols[si];
                // PIE slide candidates must be collected *before* the
                // local-symbol skip below.  Locally-defined absolute references
                // are exactly the ones a slide applies to, and the ones most
                // often written through a section symbol -- `const char
                // *msgs[] = {...}` relocates against the merged
                // `.rodata.str1.1` section symbol, not against a named symbol.
                // Which candidates really become RELATIVE depends on the final
                // copy-relocation set, so `emit_exec` filters them with
                // `pie_needs_relative` once this scan is complete.  Non-alloc
                // storage (DWARF's DW_AT_low_pc and friends) is never loaded
                // and needs no slide.  Everything after this point only
                // concerns references that cross the dynamic boundary.
                if is_pie && rela.rela_type == R_X86_64_64 && sh_flags & SHF_ALLOC != 0 {
                    pie_relative.push(PieRelative {
                        obj_idx: obj_i,
                        sec_idx,
                        offset: rela.offset,
                        addend: rela.addend,
                        sym_idx: si,
                    });
                }
                if sym.is_local() {
                    let t = rela.rela_type;
                    // An Initial-Exec reference to a local TLS symbol the
                    // rewrite does not cover (a hand-encoded non-REX.W
                    // form, EVEX with an operand-size prefix, ...) loads its
                    // TP offset from a slot, filled at link time.
                    let needs_slot = is_got64_family(t)
                        || (is_gottpoff_family(t)
                            && gottpoff_ie_to_le(
                                t,
                                obj.section_data[sec_idx].as_slice(),
                                rela.offset as usize,
                            )
                            .is_none())
                        || (is_gotpcrel_family(t)
                            && gotpcrelx_relaxation(
                                t,
                                rela.addend,
                                obj.section_data[sec_idx].as_slice(),
                                rela.offset as usize,
                                is_pie,
                                local_got_target(sym),
                            )
                            .is_none());
                    if needs_slot {
                        local_got.insert((obj_i, si, got_slot_addr_addend(t, rela.addend)));
                    }
                    continue;
                }
                if sym.name.is_empty() {
                    continue;
                }
                let gsym_info = globals
                    .get(sym.name.as_str())
                    .map(|g| (g.is_dynamic, g.info & 0xf));

                match rela.rela_type {
                    R_X86_64_PLT32 | R_X86_64_PC32 if gsym_info.map(|g| g.0).unwrap_or(false) => {
                        let sym_type = gsym_info.map(|g| g.1).unwrap_or(0);
                        if sym_type == STT_OBJECT {
                            // Dynamic data symbol - needs copy relocation
                            if copy_reloc_set.insert(sym.name.to_string()) {
                                copy_reloc_names.push(sym.name.to_string());
                            }
                        } else {
                            // Dynamic function symbol - needs PLT.  Unless the
                            // field is the rel32 of a branch, the reference
                            // takes the function's address (`lea f(%rip)`, or a
                            // `.long f - .` in data), and that address is the
                            // PLT entry -- so it has to be made canonical.
                            if plt_set.insert(sym.name.to_string()) {
                                plt_names.push(sym.name.to_string());
                            }
                            if !is_branch_rel32(
                                sh_flags,
                                obj.section_data[sec_idx].as_slice(),
                                rela.offset,
                            ) {
                                canonical_plt_set.insert(sym.name.to_string());
                            }
                        }
                    }
                    R_X86_64_GOTPCREL
                    | R_X86_64_GOTPCRELX
                    | R_X86_64_REX_GOTPCRELX
                    | R_X86_64_CODE_4_GOTPCRELX
                    | R_X86_64_CODE_5_GOTPCRELX
                    | R_X86_64_CODE_6_GOTPCRELX => {
                        // A reference the relocation pass rewrites to address
                        // the symbol directly (`mov` -> `lea`, `call *` ->
                        // `addr32 call`, ...) needs no slot; the symbol gets
                        // one only if some OTHER reference to it cannot be
                        // relaxed.  GNU ld and lld elide these slots too --
                        // each one kept costs 8 bytes of .got, a 24-byte
                        // R_X86_64_RELATIVE in a PIE, and a dependent load
                        // on every execution of the instruction.
                        let relaxed = globals
                            .get(sym.name.as_str())
                            .and_then(exec_got_target)
                            .and_then(|target| {
                                gotpcrelx_relaxation(
                                    rela.rela_type,
                                    rela.addend,
                                    obj.section_data[sec_idx].as_slice(),
                                    rela.offset as usize,
                                    is_pie,
                                    target,
                                )
                            })
                            .is_some();
                        // Otherwise GOTPCREL needs a dedicated GOT entry, even if the
                        // symbol also has a PLT entry. The PLT's GOT.PLT slot uses
                        // JUMP_SLOT (lazy binding, initially PLT+6) which is wrong
                        // for address-of. For symbols with PLT, the GOT entry is
                        // statically filled with the PLT address (no GLOB_DAT);
                        // for other dynamic symbols, GLOB_DAT is used.
                        if !relaxed && got_only_set.insert(sym.name.to_string()) {
                            got_only_names.push(sym.name.to_string());
                        }
                    }
                    // Large code model: `movabs $sym@GOT` can never be
                    // relaxed, so the symbol needs a slot; `$f@PLTOFF` of a
                    // library function needs its PLT entry (a call target,
                    // never an address-of, so not canonical).
                    t if is_got64_family(t) => {
                        if got_only_set.insert(sym.name.to_string()) {
                            got_only_names.push(sym.name.to_string());
                        }
                    }
                    // `L` of `f@PLTOFF` is a function's PLT entry.  A library
                    // VARIABLE has none to name: a stub would make `v@PLTOFF`
                    // point at code, give the variable a JUMP_SLOT, and -- the
                    // PC32 arm below redirects to any PLT entry -- send every
                    // direct reference to it there too (measured: all of them
                    // read the stub's bytes; GNU ld 2.44 does exactly that).
                    // Its link-time address is its copy, so it is resolved as
                    // any direct reference to it is.
                    R_X86_64_PLTOFF64 if gsym_info.map(|g| g.0).unwrap_or(false) => {
                        if gsym_info.map(|g| g.1) == Some(STT_OBJECT) {
                            if copy_reloc_set.insert(sym.name.to_string()) {
                                copy_reloc_names.push(sym.name.to_string());
                            }
                        } else if plt_set.insert(sym.name.to_string()) {
                            plt_names.push(sym.name.to_string());
                        }
                    }
                    t if is_gottpoff_family(t) => {
                        // Rewritten to `mov/add $tpoff` when the target is
                        // this executable's own TLS and the instruction is
                        // one the rewrite covers; only the rest need a slot.
                        let relaxed = globals
                            .get(sym.name.as_str())
                            .is_some_and(exec_ie_target_local)
                            && gottpoff_ie_to_le(
                                t,
                                obj.section_data[sec_idx].as_slice(),
                                rela.offset as usize,
                            )
                            .is_some();
                        if !relaxed
                            && !plt_set.contains(sym.name.as_str())
                            && got_only_set.insert(sym.name.to_string())
                        {
                            got_only_names.push(sym.name.to_string());
                        }
                    }
                    R_X86_64_TLSGD => {
                        // GD against a dynamic TLS symbol relaxes to IE, which
                        // needs a GOT slot carrying an R_X86_64_TPOFF64 dynamic
                        // relocation. GD against local symbols relaxes to LE
                        // (no GOT entry needed).
                        if gsym_info.map(|g| g.0).unwrap_or(false)
                            && !plt_set.contains(sym.name.as_str())
                            && got_only_set.insert(sym.name.to_string())
                        {
                            got_only_names.push(sym.name.to_string());
                        }
                    }
                    _ if gsym_info.map(|g| g.0).unwrap_or(false) => {
                        let sym_type = gsym_info.map(|g| g.1).unwrap_or(0);
                        if rela.rela_type == R_X86_64_64 {
                            if sym_type != STT_OBJECT && !(is_pie && sh_flags & SHF_WRITE != 0) {
                                // Function pointer in storage the loader does
                                // not otherwise touch (a non-PIE, or read-only
                                // data): the address we store is our PLT entry,
                                // which is then the function's canonical address.
                                if plt_set.insert(sym.name.to_string()) {
                                    plt_names.push(sym.name.to_string());
                                }
                                canonical_plt_set.insert(sym.name.to_string());
                            } else if sym_type != STT_OBJECT {
                                // PIE, writable storage: let ld.so store the
                                // real address (see `abs64_defers_to_loader`).
                                abs_dyn_relocs.push(AbsDynReloc {
                                    name: sym.name.to_string(),
                                    obj_idx: obj_i,
                                    sec_idx,
                                    offset: rela.offset,
                                    addend: rela.addend,
                                });
                            } else if sh_flags & (SHF_ALLOC | SHF_WRITE) == SHF_ALLOC {
                                // The same reference from storage the loader
                                // may not write after mapping (`movabs $var`
                                // in -mcmodel=large -fno-pic text, a non-PIC
                                // `.rodata` pointer): a dynamic relocation
                                // there is a text relocation, which this
                                // linker does not emit (and the image has no
                                // DT_TEXTREL, so ld.so faulted writing it).
                                // Like a PC32 reference, it takes the address
                                // of the variable's copy-relocated home,
                                // which is a link-time constant.
                                if copy_reloc_set.insert(sym.name.to_string()) {
                                    copy_reloc_names.push(sym.name.to_string());
                                }
                            } else {
                                // Absolute 64-bit reference to a dynamic DATA
                                // symbol, e.g. the `_ZTVN10__cxxabiv1*` vtable
                                // pointer that every C++ typeinfo object starts
                                // with. The address is only known once ld.so has
                                // mapped libstdc++, so it must become a dynamic
                                // R_X86_64_64 -- a GOT slot is useless here
                                // because the storage being initialised is the
                                // typeinfo object itself, not a GOT entry.
                                //
                                // Getting this wrong is silent and lethal: the
                                // slot keeps just the addend (0x10), and the
                                // first C++ throw of a class type segfaults
                                // inside __gxx_personality_v0 while reading the
                                // LSDA type table.
                                abs_dyn_relocs.push(AbsDynReloc {
                                    name: sym.name.to_string(),
                                    obj_idx: obj_i,
                                    sec_idx,
                                    offset: rela.offset,
                                    addend: rela.addend,
                                });
                            }
                        } else if matches!(
                            rela.rela_type,
                            R_X86_64_32
                                | R_X86_64_32S
                                | R_X86_64_16
                                | R_X86_64_8
                                | R_X86_64_PC64
                                | R_X86_64_PC16
                                | R_X86_64_PC8
                        ) {
                            // Non-PIC address-of (`mov $f, %edi`, `.quad f - .`)
                            // resolved at link time: nothing in the image is
                            // left for the loader, so the address has to be
                            // one this link knows.  A function's is its
                            // canonical PLT entry; a data object's is its
                            // copy-relocated `.bss` home -- exactly what a
                            // PC32 reference to it gets.  (These used to fall
                            // into the GOT-only arm below, which gave the
                            // symbol a slot nothing read and resolved the field
                            // itself to 0.)
                            if sym_type == STT_OBJECT {
                                if copy_reloc_set.insert(sym.name.to_string()) {
                                    copy_reloc_names.push(sym.name.to_string());
                                }
                            } else {
                                if plt_set.insert(sym.name.to_string()) {
                                    plt_names.push(sym.name.to_string());
                                }
                                canonical_plt_set.insert(sym.name.to_string());
                            }
                        } else if !plt_set.contains(sym.name.as_str())
                            && got_only_set.insert(sym.name.to_string())
                        {
                            got_only_names.push(sym.name.to_string());
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // Mark copy relocation symbols and their aliases.
    // When a symbol like `environ` (WEAK) needs a COPY relocation, we must also
    // mark aliases like `__environ` (GLOBAL) at the same shared library address.
    // This ensures the dynamic linker redirects all references to our BSS copy.
    let mut copy_reloc_lib_addrs: Vec<(String, u64)> = Vec::new(); // (from_lib, lib_sym_value)
    for name in &copy_reloc_names {
        if let Some(gsym) = globals.get_mut(name) {
            gsym.copy_reloc = true;
            if let Some(ref lib) = gsym.from_lib {
                if (gsym.info & 0xf) == STT_OBJECT && gsym.lib_sym_value != 0 {
                    let key = (lib.clone(), gsym.lib_sym_value);
                    if !copy_reloc_lib_addrs.contains(&key) {
                        copy_reloc_lib_addrs.push(key);
                    }
                }
            }
        }
    }
    // Also mark aliases (other dynamic STT_OBJECT symbols at the same library address)
    if !copy_reloc_lib_addrs.is_empty() {
        let alias_names: Vec<String> = globals
            .iter()
            .filter(|(name, g)| {
                g.is_dynamic
                    && !g.copy_reloc
                    && (g.info & 0xf) == STT_OBJECT
                    && !copy_reloc_set.contains(*name)
                    && g.from_lib.is_some()
                    && g.lib_sym_value != 0
                    && copy_reloc_lib_addrs
                        .contains(&(g.from_lib.as_ref().unwrap().clone(), g.lib_sym_value))
            })
            .map(|(n, _)| n.clone())
            .collect();
        for name in alias_names {
            if let Some(gsym) = globals.get_mut(&name) {
                gsym.copy_reloc = true;
            }
        }
    }

    let mut got_entries: Vec<(String, bool)> = Vec::new();
    got_entries.push((String::new(), false)); // GOT[0]
    got_entries.push((String::new(), false)); // GOT[1]
    got_entries.push((String::new(), false)); // GOT[2]

    for (plt_idx, name) in plt_names.iter().enumerate() {
        let got_idx = got_entries.len();
        got_entries.push((name.clone(), true));
        if let Some(gsym) = globals.get_mut(name) {
            gsym.plt_idx = Some(plt_idx);
            gsym.got_idx = Some(got_idx);
            gsym.canonical_plt = canonical_plt_set.contains(name);
        }
    }

    for name in &got_only_names {
        let got_idx = got_entries.len();
        got_entries.push((name.clone(), false));
        if let Some(gsym) = globals.get_mut(name) {
            gsym.got_idx = Some(got_idx);
        }
    }

    if std::env::var("LCCC_DEBUG_GOT").is_ok() {
        for (i, (name, is_plt)) in got_entries.iter().enumerate() {
            eprintln!("[GOT] idx={} plt={} name={:?}", i, is_plt, name);
        }
    }
    if is_pie && std::env::var("LCCC_DEBUG_GOT").is_ok() {
        eprintln!("[GOT] pie_relative={} entries", pie_relative.len());
    }
    (
        plt_names,
        got_entries,
        abs_dyn_relocs,
        pie_relative,
        local_got,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dso_sym(stt: u8) -> GlobalSymbol {
        GlobalSymbol {
            value: 0,
            size: 0,
            info: (STB_GLOBAL << 4) | stt,
            defined_in: None,
            from_lib: Some("libc.so.6".into()),
            plt_idx: None,
            got_idx: None,
            section_idx: SHN_UNDEF,
            is_dynamic: true,
            copy_reloc: false,
            canonical_plt: false,
            visibility: 0,
            lib_sym_value: 0x1000,
            version: None,
            absolute: false,
        }
    }

    #[test]
    fn branch_rel32_detection() {
        let x = SHF_ALLOC | SHF_EXECINSTR;
        // call / jmp / jcc rel32
        assert!(is_branch_rel32(x, &[0xe8, 0, 0, 0, 0], 1));
        assert!(is_branch_rel32(x, &[0xe9, 0, 0, 0, 0], 1));
        assert!(is_branch_rel32(x, &[0x0f, 0x84, 0, 0, 0, 0], 2));
        // lea f(%rip), %rax: ModRM 0x05 precedes the field.
        assert!(!is_branch_rel32(x, &[0x48, 0x8d, 0x05, 0, 0, 0, 0], 3));
        // `.long f - .` after a data byte that happens to be 0xe8.
        assert!(!is_branch_rel32(SHF_ALLOC, &[0xe8, 0, 0, 0, 0], 1));
        // Field at the start of the section / beyond it.
        assert!(!is_branch_rel32(x, &[0, 0, 0, 0], 0));
        assert!(!is_branch_rel32(x, &[0xe8], 9));
    }

    #[test]
    fn abs64_placement() {
        let func = dso_sym(STT_FUNC);
        let data = dso_sym(STT_OBJECT);
        let (ro, rw) = (SHF_ALLOC, SHF_ALLOC | SHF_WRITE);
        // Data: always the loader's (unless copy-relocated).
        assert!(abs64_defers_to_loader(&data, ro, false));
        assert!(abs64_defers_to_loader(&data, rw, true));
        let mut copied = data.clone();
        copied.copy_reloc = true;
        assert!(!abs64_defers_to_loader(&copied, rw, true));
        assert!(abs64_value_is_local(&copied, rw, true));
        // Functions: the loader's only in writable PIE storage; otherwise our
        // canonical PLT entry, which a PIE must slide.
        assert!(abs64_defers_to_loader(&func, rw, true));
        assert!(!abs64_value_is_local(&func, rw, true));
        assert!(!abs64_defers_to_loader(&func, ro, true));
        assert!(abs64_value_is_local(&func, ro, true));
        assert!(!abs64_defers_to_loader(&func, rw, false));
    }

    #[test]
    fn got_slots_of_dso_and_tls_symbols_are_never_slid() {
        let mut func = dso_sym(STT_FUNC);
        func.plt_idx = Some(0);
        func.canonical_plt = true;
        assert!(!got_slot_holds_local_address(&func));
        let mut tls = dso_sym(STT_TLS);
        tls.is_dynamic = false;
        tls.defined_in = Some(0);
        assert!(!got_slot_holds_local_address(&tls));
        let mut local = dso_sym(STT_OBJECT);
        local.is_dynamic = false;
        local.defined_in = Some(0);
        assert!(got_slot_holds_local_address(&local));
    }
}
