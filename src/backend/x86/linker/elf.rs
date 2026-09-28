/// ELF file parsing for the x86-64 linker.
///
/// This module re-exports the shared ELF64 types and parser from `linker_common`,
/// plus provides x86-64-specific relocation constants. The actual parsing logic
/// lives in the shared module to avoid duplication with ARM and RISC-V.
// Re-export shared ELF constants for mod.rs and the emitter functions.
// Archive/linker-script functions are now called via linker_common.
pub use crate::backend::elf::{
    DT_FINI, DT_GNU_HASH, DT_HASH, DT_INIT, DT_JMPREL, DT_NEEDED, DT_NULL, DT_PLTGOT, DT_PLTREL,
    DT_PLTRELSZ, DT_RELA, DT_RELAENT, DT_RELASZ, DT_STRSZ, DT_STRTAB, DT_SYMENT, DT_SYMTAB,
    ELF_MAGIC, ELFCLASS64, ELFDATA2LSB, EM_X86_64, ET_DYN, ET_EXEC, LinkerScriptEntry,
    LinkerSymbolAddresses, PF_R, PF_W, PF_X, PT_DYNAMIC, PT_GNU_EH_FRAME, PT_GNU_PROPERTY,
    PT_GNU_RELRO, PT_GNU_STACK, PT_INTERP, PT_LOAD, PT_NOTE, PT_PHDR, PT_TLS, SHF_ALLOC,
    SHF_EXECINSTR, SHF_MERGE, SHF_STRINGS, SHF_TLS, SHF_WRITE, SHN_ABS, SHN_COMMON, SHN_UNDEF,
    SHT_DYNAMIC, SHT_DYNSYM, SHT_FINI_ARRAY, SHT_GNU_HASH, SHT_GNU_VERDEF, SHT_GNU_VERNEED,
    SHT_GNU_VERSYM, SHT_HASH, SHT_INIT_ARRAY, SHT_NOBITS, SHT_NOTE, SHT_PREINIT_ARRAY,
    SHT_PROGBITS, SHT_RELA, SHT_STRTAB, SHT_SYMTAB, STB_GLOBAL, STB_LOCAL, STB_WEAK, STT_FUNC,
    STT_GNU_IFUNC, STT_OBJECT, STT_SECTION, STT_TLS, get_standard_linker_symbols, is_thin_archive,
    parse_linker_script_entries, w16, w32, w64, wphdr, write_bytes,
};

use crate::backend::linker_common;

// x86-64 relocation types
pub const R_X86_64_NONE: u32 = 0;
pub const R_X86_64_64: u32 = 1;
pub const R_X86_64_PC32: u32 = 2;
pub const R_X86_64_GOT32: u32 = 3;
pub const R_X86_64_PLT32: u32 = 4;
pub const R_X86_64_GLOB_DAT: u32 = 6;
pub const R_X86_64_JUMP_SLOT: u32 = 7;
pub const R_X86_64_RELATIVE: u32 = 8;
pub const R_X86_64_GOTPCREL: u32 = 9;
pub const R_X86_64_32: u32 = 10;
pub const R_X86_64_32S: u32 = 11;
pub const R_X86_64_DTPMOD64: u32 = 16;
pub const R_X86_64_DTPOFF64: u32 = 17;
pub const R_X86_64_TLSGD: u32 = 19;
pub const R_X86_64_TLSLD: u32 = 20;
pub const R_X86_64_DTPOFF32: u32 = 21;
pub const R_X86_64_GOTPC32_TLSDESC: u32 = 34;
pub const R_X86_64_TLSDESC_CALL: u32 = 35;
pub const R_X86_64_TPOFF64: u32 = 18;
pub const R_X86_64_GOTTPOFF: u32 = 22;
pub const R_X86_64_TPOFF32: u32 = 23;
pub const R_X86_64_PC64: u32 = 24;
pub const R_X86_64_GOTPCRELX: u32 = 41;
pub const R_X86_64_REX_GOTPCRELX: u32 = 42;
pub const R_X86_64_CODE_4_GOTPCRELX: u32 = 43;
pub const R_X86_64_CODE_4_GOTTPOFF: u32 = 44;
pub const R_X86_64_CODE_4_GOTPC32_TLSDESC: u32 = 45;
pub const R_X86_64_CODE_6_GOTPCRELX: u32 = 49;
pub const R_X86_64_CODE_6_GOTTPOFF: u32 = 50;
pub const R_X86_64_CODE_6_GOTPC32_TLSDESC: u32 = 51;
pub const R_X86_64_TLSDESC: u32 = 36;
/// C++ `-fvtable-gc` bookkeeping (GNU): no field to patch.
pub const R_X86_64_GNU_VTINHERIT: u32 = 250;
pub const R_X86_64_GNU_VTENTRY: u32 = 251;
pub const R_X86_64_IRELATIVE: u32 = 37;

/// `lea sym@tlsdesc(%rip)` (classic / REX2 / APX EVEX): the address of the
/// symbol's 16-byte TLS descriptor in the GOT.
#[inline]
pub fn is_tlsdesc_gotpc(t: u32) -> bool {
    matches!(
        t,
        R_X86_64_GOTPC32_TLSDESC
            | R_X86_64_CODE_4_GOTPC32_TLSDESC
            | R_X86_64_CODE_6_GOTPC32_TLSDESC
    )
}

/// GOT-load family: fill from a GOT slot (and, for the `*X` types, may LEA-relax).
#[inline]
pub fn is_gotpcrel_family(t: u32) -> bool {
    matches!(
        t,
        R_X86_64_GOTPCREL
            | R_X86_64_GOTPCRELX
            | R_X86_64_REX_GOTPCRELX
            | R_X86_64_CODE_4_GOTPCRELX
            | R_X86_64_CODE_6_GOTPCRELX
    )
}

/// Relaxable GOTPCRELX variants (linker may rewrite `mov` → `lea`).
#[inline]
pub fn is_gotpcrelx_relaxable(t: u32) -> bool {
    matches!(
        t,
        R_X86_64_GOTPCRELX
            | R_X86_64_REX_GOTPCRELX
            | R_X86_64_CODE_4_GOTPCRELX
            | R_X86_64_CODE_6_GOTPCRELX
    )
}

/// What a GOT-indirect reference's target is, as far as relaxing the
/// reference is concerned.  Whether the target may be relaxed to at all
/// (defined here, not preemptible, not an IFUNC, not copy-relocated) is the
/// caller's decision; this says how its value relates to the load address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GotTarget {
    /// An address inside this output: moves with the load base of a PIE or
    /// shared object.  The RIP-relative forms always work; the immediate
    /// forms only without PIC.
    Image,
    /// A link-time constant (an `SHN_ABS` definition) with this value.  It
    /// never moves, so the immediate forms work even with PIC, provided the
    /// value fits the imm32.  The RIP-relative forms never: with PIC the
    /// result would slide (GNU ld 2.47 makes exactly that mistake for a
    /// `--defsym` constant), and without PIC whether an arbitrary constant
    /// is within +-2 GiB of the instruction is unknown until layout -- a
    /// reference the planner elided a slot for must never turn out
    /// unencodable.  A constant that does not fit keeps its slot.
    Absolute(u64),
}

/// Whether `v` is representable by the imm32 of every immediate form used
/// below: sign-extended (REX.W) and zero-extended (32-bit) alike.
#[inline]
fn fits_imm32(v: u64) -> bool {
    v <= i32::MAX as u64
}

/// How a GOT-indirect reference can address its target directly (x86-64
/// psABI, "Optimize GOTPCRELX Relocations"; the transformations GNU ld and
/// lld perform).  Decided from the relocation and the ORIGINAL section bytes
/// by [`gotpcrelx_relaxation`], which also validates every byte
/// [`rewrite_got_relax`] later rewrites, so the rewriter never has to
/// re-derive (or trust) the encoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GotRelax {
    /// `mov foo@GOTPCREL(%rip), %reg` -> `mov $foo, %reg` when `imm` and
    /// the final value fits the imm32, else `lea foo(%rip), %reg`.  (For an
    /// image address the value is only known after layout, so the choice is
    /// the rewriter's; `imm` says whether the immediate form is allowed.)
    Load { imm: bool },
    /// `call *foo@GOTPCREL(%rip)` -> `addr32 call foo` (same 6 bytes).
    Call,
    /// `jmp *foo@GOTPCREL(%rip)` -> `jmp foo; nop`.
    Jmp,
    /// `test %reg, foo@GOTPCREL(%rip)` -> `test $foo, %reg`.
    Test,
    /// `add/or/adc/sbb/and/sub/xor/cmp foo@GOTPCREL(%rip), %reg` ->
    /// `<op> $foo, %reg`.
    Binop,
}

/// The relaxation available for a GOT-indirect relocation whose 32-bit
/// field is at section offset `off` of `code` (the input section's bytes),
/// or `None` when the reference must go through a GOT slot.
///
/// Only the `*GOTPCRELX` types promise a relaxable instruction, and only
/// with addend -4: any other addend means the 32-bit field is not the last
/// thing in the instruction or the instruction reads part of the slot
/// (`movl foo@GOTPCREL+4(%rip), %eax` loads its HIGH half).  The ModRM byte
/// must be the RIP-relative form.  Each type fixes the prefix layout, and
/// the prefix is VERIFIED inside the section, never assumed:
///
/// * `GOTPCRELX`: no prefix (the relocation promises none) -- `mov`, and
///   `call`/`jmp`, which are rewritten into prefix-free encodings.
/// * `REX_GOTPCRELX`: a REX byte (0x40-0x4f) at `off - 3` -- `mov`, and
///   the immediate `test`/binop forms, which also need REX.W (their imm32 is
///   sign-extended to the 64-bit operation).  Not `call`/`jmp`: the rewrite
///   would leave the REX byte in front of the `addr32` prefix.
/// * `CODE_4_GOTPCRELX`: a REX2 prefix (`d5 xx`) at `off - 4` -- the
///   opcode-preserving `lea` only; the REX2 payload of the immediate forms
///   is laid out differently, and `call`/`jmp` cannot carry it.
///
/// A type whose prefix is absent (a misdescribed or truncated instruction,
/// e.g. `REX_GOTPCRELX` two bytes into its section) is never relaxed: its
/// GOT slot is always correct, and rewriting would touch bytes outside the
/// instruction.
pub fn gotpcrelx_relaxation(
    rela_type: u32,
    addend: i64,
    code: &[u8],
    off: usize,
    is_pic: bool,
    target: GotTarget,
) -> Option<GotRelax> {
    if addend != -4 || off < 2 || off.checked_add(4)? > code.len() {
        return None;
    }
    let (op, modrm) = (code[off - 2], code[off - 1]);
    if modrm & 0xc7 != 0x05 {
        return None;
    }
    // Which families a target admits: RIP-relative (`lea`, `call`, `jmp`)
    // and immediate (`mov $`, `test $`, `<op> $`).
    let (rip_ok, imm_ok) = match target {
        GotTarget::Image => (true, !is_pic),
        GotTarget::Absolute(v) => (false, fits_imm32(v)),
    };
    // An image address is only known after layout: the rewriter uses the
    // immediate when allowed and the address fits, else `lea` (always in
    // reach: the small code model keeps the image within 2 GiB).  An
    // absolute value is known now and fits, or the load is not relaxed.
    let load = || (rip_ok || imm_ok).then_some(GotRelax::Load { imm: imm_ok });
    match rela_type {
        R_X86_64_GOTPCRELX => match op {
            0x8b => load(),
            0xff if modrm == 0x15 && rip_ok => Some(GotRelax::Call),
            0xff if modrm == 0x25 && rip_ok => Some(GotRelax::Jmp),
            _ => None,
        },
        R_X86_64_REX_GOTPCRELX => {
            let rex = *code.get(off.checked_sub(3)?)?;
            if rex & 0xf0 != 0x40 {
                return None;
            }
            let rex_w = rex & 0x08 != 0;
            match op {
                0x8b => load(),
                0x85 if rex_w && imm_ok => Some(GotRelax::Test),
                0x03 | 0x0b | 0x13 | 0x1b | 0x23 | 0x2b | 0x33 | 0x3b if rex_w && imm_ok => {
                    Some(GotRelax::Binop)
                }
                _ => None,
            }
        }
        R_X86_64_CODE_4_GOTPCRELX => {
            if off < 4 || code[off - 4] != 0xd5 || op != 0x8b || !rip_ok {
                return None;
            }
            Some(GotRelax::Load { imm: false })
        }
        _ => None,
    }
}

/// REX byte after moving ModRM.reg into ModRM.rm: REX.R becomes REX.B.  The
/// old REX.B MUST be cleared, not kept: in the RIP-relative source form it
/// selects nothing (ModRM.rm = 101 with mod = 00 means RIP whatever REX.B
/// says), but in the register form it selects the destination -- keeping it
/// turned `49 8b 05` (load into %rax) into `49 c7 c0` (store into %r8).
/// W and X are kept (X is ignored by both forms).
#[inline]
fn rex_r_to_b(rex: u8) -> u8 {
    (rex & !0x05) | ((rex & 0x04) >> 2)
}

/// Rewrite the instruction whose 32-bit GOTPCRELX field (relocation type
/// `rela_type`) is at `fp` for a target whose final value is `s` (the
/// field's address is `p`); returns the position and value of the 32-bit
/// field to store, for the caller's range-checked write.  `kind` must come
/// from [`gotpcrelx_relaxation`] over the same instruction bytes: it has
/// verified the prefix this touches.
pub fn rewrite_got_relax(
    buf: &mut [u8],
    fp: usize,
    kind: GotRelax,
    rela_type: u32,
    s: u64,
    p: u64,
) -> (usize, i64) {
    // The GOTPCRELX addend is -4 (checked by `gotpcrelx_relaxation`).
    let pcrel = s as i64 - 4 - p as i64;
    let has_rex = rela_type == R_X86_64_REX_GOTPCRELX;
    debug_assert!(
        !has_rex || buf[fp - 3] & 0xf0 == 0x40,
        "unverified REX prefix"
    );
    match kind {
        // `mov $foo, %reg` (`c7 /0 imm32`, same 7/6 bytes): an immediate
        // move depends on nothing, where the RIP-relative `lea` still needs
        // an address generation.  The imm32 of the REX.W form is
        // sign-extended and that of the 32-bit form zero-extended;
        // `fits_imm32` holds for both.  ModRM.reg moves into ModRM.rm.
        GotRelax::Load { imm: true } if fits_imm32(s) => {
            let modrm = buf[fp - 1];
            buf[fp - 1] = 0xc0 | ((modrm & 0x38) >> 3);
            buf[fp - 2] = 0xc7;
            if has_rex {
                buf[fp - 3] = rex_r_to_b(buf[fp - 3]);
            }
            (fp, s as i64)
        }
        GotRelax::Load { .. } => {
            buf[fp - 2] = 0x8d;
            (fp, pcrel)
        }
        GotRelax::Call => {
            buf[fp - 2] = 0x67;
            buf[fp - 1] = 0xe8;
            (fp, pcrel)
        }
        GotRelax::Jmp => {
            // e9 <rel32> 90: the rel32 now starts one byte earlier and is
            // relative to the end of the 5-byte jmp.
            buf[fp - 2] = 0xe9;
            buf[fp + 3] = 0x90;
            (fp - 1, pcrel + 1)
        }
        GotRelax::Test => {
            let modrm = buf[fp - 1];
            buf[fp - 1] = 0xc0 | ((modrm & 0x38) >> 3);
            buf[fp - 2] = 0xf7;
            buf[fp - 3] = rex_r_to_b(buf[fp - 3]);
            (fp, s as i64)
        }
        GotRelax::Binop => {
            let (op, modrm) = (buf[fp - 2], buf[fp - 1]);
            buf[fp - 1] = 0xc0 | ((modrm & 0x38) >> 3) | (op & 0x38);
            buf[fp - 2] = 0x81;
            buf[fp - 3] = rex_r_to_b(buf[fp - 3]);
            (fp, s as i64)
        }
    }
}

/// Initial-Exec -> Local-Exec: the instruction an `R_X86_64_GOTTPOFF` at
/// section offset `off` of `code` belongs to, if it is one of the two the
/// psABI defines (`movq foo@gottpoff(%rip), %reg` / `addq ..., %reg`, REX.W
/// at `off - 3`, RIP-relative ModRM).  Verified inside the section, as for
/// [`gotpcrelx_relaxation`]; REX2/APX forms are not rewritten.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IeToLe {
    Mov,
    Add,
}

pub fn gottpoff_ie_to_le(rela_type: u32, code: &[u8], off: usize) -> Option<IeToLe> {
    if rela_type != R_X86_64_GOTTPOFF || off < 3 || off.checked_add(4)? > code.len() {
        return None;
    }
    let (rex, op, modrm) = (code[off - 3], code[off - 2], code[off - 1]);
    if rex & 0xf8 != 0x48 || modrm & 0xc7 != 0x05 {
        return None;
    }
    match op {
        0x8b => Some(IeToLe::Mov),
        0x03 => Some(IeToLe::Add),
        _ => None,
    }
}

/// Apply an [`IeToLe`] rewrite at field position `fp`: `REX' c7 /0` or
/// `REX' 81 /0` with the destination moved into ModRM.rm (REX.R -> REX.B,
/// see [`rex_r_to_b`]; without it `%r12` silently became `%rsp`).  The
/// caller stores the TP offset into the imm32 at `fp`.
pub fn rewrite_ie_to_le(buf: &mut [u8], fp: usize, kind: IeToLe) {
    let modrm = buf[fp - 1];
    buf[fp - 1] = 0xc0 | ((modrm & 0x38) >> 3);
    buf[fp - 2] = match kind {
        IeToLe::Mov => 0xc7,
        IeToLe::Add => 0x81,
    };
    buf[fp - 3] = rex_r_to_b(buf[fp - 3]);
}

/// TLS Initial-Exec through a GOT slot (classic / REX2 / APX EVEX).
#[inline]
pub fn is_gottpoff_family(t: u32) -> bool {
    matches!(
        t,
        R_X86_64_GOTTPOFF | R_X86_64_CODE_4_GOTTPOFF | R_X86_64_CODE_6_GOTTPOFF
    )
}
pub const R_X86_64_16: u32 = 12;
pub const R_X86_64_PC16: u32 = 13;
pub const R_X86_64_8: u32 = 14;
pub const R_X86_64_PC8: u32 = 15;
pub const R_X86_64_GOTOFF64: u32 = 25;
pub const R_X86_64_GOTPC32: u32 = 26;
pub const R_X86_64_SIZE32: u32 = 32;
pub const R_X86_64_SIZE64: u32 = 33;
// The 64-bit GOT family and RELATIVE64. Numbers verified against
// /usr/include/elf.h (GOT64 27, GOTPCREL64 28, GOTPC64 29, GOTPLT64 30,
// PLTOFF64 31, RELATIVE64 38) rather than recalled: a wrong constant here is
// indistinguishable from a wrong relocation at link time. They are in the field
// table so that `field()` answers truthfully for every x86-64 type the ABI
// defines, and so a diagnostic can name a type this backend encounters in a
// foreign object instead of printing "type 29".
pub const R_X86_64_GOT64: u32 = 27;
pub const R_X86_64_GOTPCREL64: u32 = 28;
pub const R_X86_64_GOTPC64: u32 = 29;
pub const R_X86_64_GOTPLT64: u32 = 30;
pub const R_X86_64_PLTOFF64: u32 = 31;
pub const R_X86_64_RELATIVE64: u32 = 38;

/// Large-code-model references that address a GOT slot by a 64-bit offset
/// (`movabs $sym@GOT, %rax` = G + A from `_GLOBAL_OFFSET_TABLE_`, or
/// GOTPCREL64 = G + GOT + A - P).  Unlike the 32-bit GOTPCREL family they
/// can never be relaxed -- the instruction is a `movabs` of an offset, not
/// a memory operand -- so every one of them needs a slot.  GOTPLT64 is
/// GOT64 whose slot may be the PLT's; one ordinary slot serves both.
pub fn is_got64_family(t: u32) -> bool {
    matches!(t, R_X86_64_GOT64 | R_X86_64_GOTPCREL64 | R_X86_64_GOTPLT64)
}

// DT_* constants now in shared module - re-export them
pub use crate::backend::elf::{
    DT_DEBUG, DT_FINI_ARRAY, DT_FINI_ARRAYSZ, DT_FLAGS, DT_FLAGS_1, DT_INIT_ARRAY, DT_INIT_ARRAYSZ,
    DT_PREINIT_ARRAY, DT_PREINIT_ARRAYSZ, DT_RELACOUNT, DT_RPATH, DT_RUNPATH, DT_SONAME, DT_VERDEF,
    DT_VERDEFNUM, DT_VERNEED, DT_VERNEEDNUM, DT_VERSYM,
};

// ── Type aliases ─────────────────────────────────────────────────────────
// Re-export shared types under the names the x86 linker already uses.

pub type SectionHeader = linker_common::Elf64Section;
pub type Symbol = linker_common::Elf64Symbol;
pub type Rela = linker_common::Elf64Rela;
pub type ElfObject = linker_common::Elf64Object;
pub type DynSymbol = linker_common::DynSymbol;

// ── Parsing functions ────────────────────────────────────────────────────
// Delegate to shared implementations.

pub fn parse_object(data: &[u8], source_name: &str) -> Result<ElfObject, String> {
    linker_common::parse_elf64_object(data, source_name, EM_X86_64)
}

/// Parse an object that lives inside an already-shared buffer, so section
/// contents become windows into it instead of copies. See `secdata.rs`.
pub fn parse_object_shared(
    buf: &std::sync::Arc<[u8]>,
    base: usize,
    size: usize,
    source_name: &str,
) -> Result<ElfObject, String> {
    linker_common::parse_elf64_object_at(buf, base, size, source_name, EM_X86_64)
}

pub fn parse_shared_library_symbols(data: &[u8], lib_name: &str) -> Result<Vec<DynSymbol>, String> {
    linker_common::parse_shared_library_symbols(data, lib_name)
}

pub fn parse_soname(data: &[u8]) -> Option<String> {
    linker_common::parse_soname(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Classify and rewrite `insn` (whose last 4 bytes are the GOTPCRELX
    /// field), placed after a one-byte stand-in for the previous instruction
    /// so an out-of-bounds read of "fp-3" on a non-REX form would be visible.
    fn relax(insn: &[u8], rela_type: u32, is_pic: bool, s: u64) -> Option<Vec<u8>> {
        relax_to(insn, rela_type, is_pic, GotTarget::Image, s)
    }

    fn relax_to(
        insn: &[u8],
        rela_type: u32,
        is_pic: bool,
        target: GotTarget,
        s: u64,
    ) -> Option<Vec<u8>> {
        let mut buf = vec![0xc3];
        buf.extend_from_slice(insn);
        let fp = buf.len() - 4;
        let kind = gotpcrelx_relaxation(rela_type, -4, &buf, fp, is_pic, target)?;
        let p = 0x1000 + fp as u64;
        let (pos, v) = rewrite_got_relax(&mut buf, fp, kind, rela_type, s, p);
        buf[pos..pos + 4].copy_from_slice(&(v as i32).to_le_bytes());
        assert_eq!(buf[0], 0xc3, "rewrite touched the previous instruction");
        Some(buf[1..].to_vec())
    }

    fn with_imm(prefix: &[u8], imm: i32) -> Vec<u8> {
        let mut v = prefix.to_vec();
        v.extend_from_slice(&imm.to_le_bytes());
        v
    }

    const Z: [u8; 4] = [0; 4];

    #[test]
    fn mov_becomes_mov_imm_without_pic() {
        // movq foo@GOTPCREL(%rip), %rax  ->  movq $foo, %rax
        let got = relax(
            &[0x48, 0x8b, 0x05, 0, 0, 0, 0],
            R_X86_64_REX_GOTPCRELX,
            false,
            0x404020,
        );
        assert_eq!(got.unwrap(), with_imm(&[0x48, 0xc7, 0xc0], 0x404020));
        // movq foo@GOTPCREL(%rip), %r9  ->  movq $foo, %r9 (REX.R -> REX.B)
        let got = relax(
            &[0x4c, 0x8b, 0x0d, 0, 0, 0, 0],
            R_X86_64_REX_GOTPCRELX,
            false,
            0x404020,
        );
        assert_eq!(got.unwrap(), with_imm(&[0x49, 0xc7, 0xc1], 0x404020));
        // movl foo@GOTPCREL(%rip), %r8d (REX without W)  ->  movl $foo, %r8d
        let got = relax(
            &[0x44, 0x8b, 0x05, 0, 0, 0, 0],
            R_X86_64_REX_GOTPCRELX,
            false,
            0x404020,
        );
        assert_eq!(got.unwrap(), with_imm(&[0x41, 0xc7, 0xc0], 0x404020));
        // movl foo@GOTPCREL(%rip), %edx (no REX; the byte before is not ours)
        let got = relax(
            &[0x8b, 0x15, 0, 0, 0, 0],
            R_X86_64_GOTPCRELX,
            false,
            0x404020,
        );
        assert_eq!(got.unwrap(), with_imm(&[0xc7, 0xc2], 0x404020));
    }

    #[test]
    fn mov_stays_lea_when_imm_cannot_hold_the_address() {
        let s = 0x8000_0000u64; // sign-extends wrongly in the REX.W imm32
        let got = relax(
            &[0x48, 0x8b, 0x05, 0, 0, 0, 0],
            R_X86_64_REX_GOTPCRELX,
            false,
            s,
        )
        .unwrap();
        let fp = 1 + 3;
        let pcrel = s as i64 - 4 - (0x1000 + fp as i64);
        assert_eq!(got, with_imm(&[0x48, 0x8d, 0x05], pcrel as i32));
    }

    #[test]
    fn mov_becomes_lea_with_pic_and_for_apx() {
        let got = relax(
            &[0x48, 0x8b, 0x05, 0, 0, 0, 0],
            R_X86_64_REX_GOTPCRELX,
            true,
            0x2000,
        )
        .unwrap();
        assert_eq!(&got[..3], &[0x48, 0x8d, 0x05]);
        assert_eq!(
            i32::from_le_bytes(got[3..7].try_into().unwrap()),
            0x2000 - 4 - 0x1004
        );
        // REX2 (d5 xx) mov: the opcode-preserving lea only.
        let got = relax(
            &[0xd5, 0x48, 0x8b, 0x05, 0, 0, 0, 0],
            R_X86_64_CODE_4_GOTPCRELX,
            false,
            0x2000,
        );
        assert_eq!(&got.unwrap()[..4], &[0xd5, 0x48, 0x8d, 0x05]);
    }

    #[test]
    fn branches_and_immediate_forms() {
        // call *foo@GOTPCREL(%rip) -> addr32 call foo
        let got = relax(&[0xff, 0x15, 0, 0, 0, 0], R_X86_64_GOTPCRELX, true, 0x2000).unwrap();
        assert_eq!(&got[..2], &[0x67, 0xe8]);
        // jmp *foo@GOTPCREL(%rip) -> jmp foo; nop (rel32 one byte earlier)
        let got = relax(&[0xff, 0x25, 0, 0, 0, 0], R_X86_64_GOTPCRELX, true, 0x2000).unwrap();
        assert_eq!(got[0], 0xe9);
        assert_eq!(got[5], 0x90);
        assert_eq!(
            i32::from_le_bytes(got[1..5].try_into().unwrap()),
            0x2000 - (0x1001 + 5)
        );
        // test %rcx, foo@GOTPCREL(%rip) -> test $foo, %rcx
        let got = relax(
            &[0x48, 0x85, 0x0d, 0, 0, 0, 0],
            R_X86_64_REX_GOTPCRELX,
            false,
            0x4000,
        );
        assert_eq!(got.unwrap(), with_imm(&[0x48, 0xf7, 0xc1], 0x4000));
        // cmp foo@GOTPCREL(%rip), %r9 -> cmp $foo, %r9
        let got = relax(
            &[0x4c, 0x3b, 0x0d, 0, 0, 0, 0],
            R_X86_64_REX_GOTPCRELX,
            false,
            0x4000,
        );
        assert_eq!(got.unwrap(), with_imm(&[0x49, 0x81, 0xf9], 0x4000));
        // add foo@GOTPCREL(%rip), %rax -> add $foo, %rax
        let got = relax(
            &[0x48, 0x03, 0x05, 0, 0, 0, 0],
            R_X86_64_REX_GOTPCRELX,
            false,
            0x4000,
        );
        assert_eq!(got.unwrap(), with_imm(&[0x48, 0x81, 0xc0], 0x4000));
    }

    #[test]
    fn unrelaxable_references() {
        let mov = [0x48, 0x8b, 0x05, 0, 0, 0, 0];
        let buf = [&[0xc3][..], &mov[..]].concat();
        // Plain GOTPCREL promises nothing about the instruction.
        assert_eq!(
            gotpcrelx_relaxation(R_X86_64_GOTPCREL, -4, &buf, 4, false, GotTarget::Image),
            None
        );
        // A non -4 addend reads part of the slot (e.g. its high half).
        assert_eq!(
            gotpcrelx_relaxation(R_X86_64_REX_GOTPCRELX, 0, &buf, 4, false, GotTarget::Image),
            None
        );
        // Immediate forms need a link-time-constant address and REX.W.
        let test = [&[0xc3][..], &[0x48, 0x85, 0x0d], &Z[..]].concat();
        assert_eq!(
            gotpcrelx_relaxation(R_X86_64_REX_GOTPCRELX, -4, &test, 4, true, GotTarget::Image),
            None
        );
        let test32 = [&[0xc3][..], &[0x44, 0x85, 0x0d], &Z[..]].concat();
        assert_eq!(
            gotpcrelx_relaxation(
                R_X86_64_REX_GOTPCRELX,
                -4,
                &test32,
                4,
                false,
                GotTarget::Image
            ),
            None
        );
        // Not a RIP-relative ModRM.
        let reg = [&[0xc3][..], &[0x48, 0x8b, 0x04], &Z[..]].concat();
        assert_eq!(
            gotpcrelx_relaxation(R_X86_64_REX_GOTPCRELX, -4, &reg, 4, false, GotTarget::Image),
            None
        );
        // Field running past the end of the section.
        assert_eq!(
            gotpcrelx_relaxation(
                R_X86_64_REX_GOTPCRELX,
                -4,
                &buf[..6],
                4,
                false,
                GotTarget::Image
            ),
            None
        );
    }

    /// REX.B selects nothing in the RIP-relative source form but the
    /// destination in the register form: it must be REPLACED by REX.R,
    /// never OR-ed with it.  Every R/B/W/X combination, exact bytes.
    #[test]
    fn rex_b_of_the_source_is_replaced_not_kept() {
        for rex in 0x40u8..=0x4f {
            let reg = 2u8; // ModRM.reg = %rdx / %r10
            let mov = [rex, 0x8b, 0x05 | (reg << 3), 0, 0, 0, 0];
            let got = relax(&mov, R_X86_64_REX_GOTPCRELX, false, 0x404020).unwrap();
            let want_rex = (rex & 0x0a) | 0x40 | ((rex & 0x04) >> 2);
            assert_eq!(
                got,
                with_imm(&[want_rex, 0xc7, 0xc0 | reg], 0x404020),
                "rex {rex:#x}"
            );
            // The destination register number is preserved exactly.
            let src_dst = ((rex & 0x04) << 1) | reg;
            let out_dst = ((got[0] & 0x01) << 3) | (got[2] & 7);
            assert_eq!(src_dst, out_dst, "rex {rex:#x}");
            if rex & 0x08 != 0 {
                for (op, new_op) in [(0x85u8, 0xf7u8), (0x3b, 0x81)] {
                    let insn = [rex, op, 0x05 | (reg << 3), 0, 0, 0, 0];
                    let got = relax(&insn, R_X86_64_REX_GOTPCRELX, false, 0x4000).unwrap();
                    assert_eq!(got[0], want_rex, "op {op:#x} rex {rex:#x}");
                    assert_eq!(got[1], new_op);
                    assert_eq!(((got[0] & 1) << 3) | (got[2] & 7), src_dst);
                }
            }
        }
        // The reviewer's case: 49 8b 05 loads %rax; it must stay %rax.
        let got = relax(
            &[0x49, 0x8b, 0x05, 0, 0, 0, 0],
            R_X86_64_REX_GOTPCRELX,
            false,
            0x10,
        );
        assert_eq!(got.unwrap(), with_imm(&[0x48, 0xc7, 0xc0], 0x10));
    }

    /// The prefix a relocation type promises is checked inside the section:
    /// `REX_GOTPCRELX` two bytes into its section has no REX byte to fix, a
    /// REX-typed field behind a non-REX byte is misdescribed, and REX2 must
    /// really be there.  All stay on their (always correct) GOT slot.
    #[test]
    fn misdescribed_or_truncated_prefixes_are_not_relaxed() {
        let at_start = [0x8b, 0x05, 0, 0, 0, 0];
        for t in [R_X86_64_REX_GOTPCRELX, R_X86_64_CODE_4_GOTPCRELX] {
            assert_eq!(
                gotpcrelx_relaxation(t, -4, &at_start, 2, false, GotTarget::Image),
                None
            );
        }
        // Well-formed without a prefix: GOTPCRELX at offset 2 is fine.
        assert_eq!(
            gotpcrelx_relaxation(
                R_X86_64_GOTPCRELX,
                -4,
                &at_start,
                2,
                false,
                GotTarget::Image
            ),
            Some(GotRelax::Load { imm: true })
        );
        let no_rex = [0x90, 0x8b, 0x05, 0, 0, 0, 0];
        assert_eq!(
            gotpcrelx_relaxation(
                R_X86_64_REX_GOTPCRELX,
                -4,
                &no_rex,
                3,
                false,
                GotTarget::Image
            ),
            None
        );
        let no_rex2 = [0x90, 0x48, 0x8b, 0x05, 0, 0, 0, 0];
        assert_eq!(
            gotpcrelx_relaxation(
                R_X86_64_CODE_4_GOTPCRELX,
                -4,
                &no_rex2,
                4,
                false,
                GotTarget::Image
            ),
            None
        );
        // REX2 call/jmp and REX call: no prefix-preserving rewrite exists.
        for insn in [&[0xd5, 0x00, 0xff, 0x15][..], &[0xd5, 0x00, 0xff, 0x25][..]] {
            let b = [insn, &Z[..]].concat();
            assert_eq!(
                gotpcrelx_relaxation(R_X86_64_CODE_4_GOTPCRELX, -4, &b, 4, true, GotTarget::Image),
                None
            );
        }
        let rex_call = [0x48, 0xff, 0x15, 0, 0, 0, 0];
        assert_eq!(
            gotpcrelx_relaxation(
                R_X86_64_REX_GOTPCRELX,
                -4,
                &rex_call,
                3,
                true,
                GotTarget::Image
            ),
            None
        );
    }

    /// An absolute target never moves: immediate forms are valid even with
    /// PIC when the value fits; RIP-relative ones are never chosen (they
    /// would slide with PIC, and may be out of reach without it).
    #[test]
    fn absolute_targets() {
        let mov = [0x48, 0x8b, 0x05, 0, 0, 0, 0];
        let abs = |v| GotTarget::Absolute(v);
        // PIE/.so: mov $imm for a small constant, a slot for a large one.
        let got = relax_to(&mov, R_X86_64_REX_GOTPCRELX, true, abs(0x1234), 0x1234);
        assert_eq!(got.unwrap(), with_imm(&[0x48, 0xc7, 0xc0], 0x1234));
        assert_eq!(
            relax_to(&mov, R_X86_64_REX_GOTPCRELX, true, abs(1 << 40), 1 << 40),
            None
        );
        // ... and no lea/call/jmp, whose RIP-relative result would slide.
        let call = [0xff, 0x15, 0, 0, 0, 0];
        assert_eq!(
            relax_to(&call, R_X86_64_GOTPCRELX, true, abs(0x1234), 0x1234),
            None
        );
        let rex2 = [0xd5, 0x48, 0x8b, 0x05, 0, 0, 0, 0];
        assert_eq!(
            relax_to(&rex2, R_X86_64_CODE_4_GOTPCRELX, true, abs(0x10), 0x10),
            None
        );
        // test/binop immediates are position-independent for a constant.
        let test = [0x48, 0x85, 0x05, 0, 0, 0, 0];
        let got = relax_to(&test, R_X86_64_REX_GOTPCRELX, true, abs(0x80), 0x80);
        assert_eq!(got.unwrap(), with_imm(&[0x48, 0xf7, 0xc0], 0x80));
        // Without PIC too: a large constant keeps its slot, and a small one
        // becomes an immediate, never a RIP-relative form.
        let big = 0x9000_0000u64;
        assert_eq!(
            relax_to(&mov, R_X86_64_REX_GOTPCRELX, false, abs(big), big),
            None
        );
        let got = relax_to(&mov, R_X86_64_REX_GOTPCRELX, false, abs(0x1234), 0x1234);
        assert_eq!(got.unwrap(), with_imm(&[0x48, 0xc7, 0xc0], 0x1234));
        assert_eq!(
            relax_to(&call, R_X86_64_GOTPCRELX, false, abs(0x1234), 0x1234),
            None
        );
    }

    #[test]
    fn ie_to_le_validates_and_moves_rex_r_to_b() {
        // movq foo@gottpoff(%rip), %r12  ->  movq $tpoff, %r12
        let mut b = vec![0xc3, 0x4c, 0x8b, 0x25, 0, 0, 0, 0];
        assert_eq!(
            gottpoff_ie_to_le(R_X86_64_GOTTPOFF, &b, 4),
            Some(IeToLe::Mov)
        );
        rewrite_ie_to_le(&mut b, 4, IeToLe::Mov);
        assert_eq!(&b[..4], &[0xc3, 0x49, 0xc7, 0xc4]);
        // addq foo@gottpoff(%rip), %rax with a stray REX.B
        let mut b = vec![0x49, 0x03, 0x05, 0, 0, 0, 0];
        assert_eq!(
            gottpoff_ie_to_le(R_X86_64_GOTTPOFF, &b, 3),
            Some(IeToLe::Add)
        );
        rewrite_ie_to_le(&mut b, 3, IeToLe::Add);
        assert_eq!(&b[..3], &[0x48, 0x81, 0xc0]);
        // At the section start, without REX.W, not RIP-relative, other op.
        assert_eq!(
            gottpoff_ie_to_le(R_X86_64_GOTTPOFF, &[0x8b, 0x05, 0, 0, 0, 0], 2),
            None
        );
        assert_eq!(
            gottpoff_ie_to_le(R_X86_64_GOTTPOFF, &[0x44, 0x8b, 0x05, 0, 0, 0, 0], 3),
            None
        );
        assert_eq!(
            gottpoff_ie_to_le(R_X86_64_GOTTPOFF, &[0x48, 0x8b, 0x04, 0, 0, 0, 0], 3),
            None
        );
        assert_eq!(
            gottpoff_ie_to_le(R_X86_64_GOTTPOFF, &[0x48, 0x2b, 0x05, 0, 0, 0, 0], 3),
            None
        );
        assert_eq!(
            gottpoff_ie_to_le(
                R_X86_64_CODE_4_GOTTPOFF,
                &[0xd5, 0x48, 0x8b, 0x05, 0, 0, 0, 0],
                4
            ),
            None
        );
    }
}
