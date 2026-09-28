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

/// How a GOT-indirect reference can address its target directly (x86-64
/// psABI, "Optimize GOTPCRELX Relocations"; the transformations GNU ld and
/// lld perform).  Decided from the relocation and the ORIGINAL instruction
/// bytes by [`gotpcrelx_relaxation`]; applied by [`rewrite_got_relax`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GotRelax {
    /// `mov foo@GOTPCREL(%rip), %reg` -> `lea foo(%rip), %reg`.
    Lea,
    /// `call *foo@GOTPCREL(%rip)` -> `addr32 call foo` (same 6 bytes).
    Call,
    /// `jmp *foo@GOTPCREL(%rip)` -> `jmp foo; nop`.
    Jmp,
    /// `test %reg, foo@GOTPCREL(%rip)` -> `test $foo, %reg` (non-PIC only).
    Test,
    /// `add/or/adc/sbb/and/sub/xor/cmp foo@GOTPCREL(%rip), %reg` ->
    /// `<op> $foo, %reg` (non-PIC only).
    Binop,
}

/// The relaxation available for a GOT-indirect relocation at `off` in
/// `code`, or `None` when the reference must go through a GOT slot.
///
/// Only the `*GOTPCRELX` types promise a relaxable instruction, and only
/// with addend -4: any other addend means the 32-bit field is not the last
/// thing in the instruction or the instruction reads part of the slot
/// (`movl foo@GOTPCREL+4(%rip), %eax` loads its HIGH half), so the bytes
/// cannot be rewritten.  The ModRM byte must be the RIP-relative form.  The
/// immediate forms (test/binop) turn the operand into the target's absolute
/// address, which is only link-time constant without PIC, and need the
/// REX.W prefix `R_X86_64_REX_GOTPCRELX` guarantees: the imm32 of a 64-bit
/// op is sign-extended, so the value range is checked when it is written.
/// REX2/APX encodings (`CODE_4`) get the opcode-preserving forms only; the
/// REX2 payload layout of the immediate forms differs.  Whether the TARGET
/// qualifies (non-preemptible, not IFUNC, not absolute) is the caller's
/// decision.
pub fn gotpcrelx_relaxation(
    rela_type: u32,
    addend: i64,
    code: &[u8],
    off: usize,
    is_pic: bool,
) -> Option<GotRelax> {
    if addend != -4
        || !matches!(
            rela_type,
            R_X86_64_GOTPCRELX | R_X86_64_REX_GOTPCRELX | R_X86_64_CODE_4_GOTPCRELX
        )
        || off < 2
        || off.checked_add(4)? > code.len()
    {
        return None;
    }
    let (op, modrm) = (code[off - 2], code[off - 1]);
    if modrm & 0xc7 != 0x05 {
        return None;
    }
    match op {
        0x8b => Some(GotRelax::Lea),
        0xff if modrm == 0x15 => Some(GotRelax::Call),
        0xff if modrm == 0x25 => Some(GotRelax::Jmp),
        _ if is_pic
            || rela_type != R_X86_64_REX_GOTPCRELX
            || off < 3
            || code[off - 3] & 0xf8 != 0x48 =>
        {
            None
        }
        0x85 => Some(GotRelax::Test),
        0x03 | 0x0b | 0x13 | 0x1b | 0x23 | 0x2b | 0x33 | 0x3b => Some(GotRelax::Binop),
        _ => None,
    }
}

/// Rewrite the instruction whose 32-bit GOTPCRELX field (relocation type
/// `rela_type`) is at `fp` for a target at `s` (the field's address is `p`);
/// returns the position and value of the 32-bit field to store, for the
/// caller's range-checked write.  `is_pic` must be the value
/// `gotpcrelx_relaxation` was asked with.
pub fn rewrite_got_relax(
    buf: &mut [u8],
    fp: usize,
    kind: GotRelax,
    rela_type: u32,
    is_pic: bool,
    s: u64,
    p: u64,
) -> (usize, i64) {
    // The GOTPCRELX addend is -4 (checked by `gotpcrelx_relaxation`).
    let pcrel = s as i64 - 4 - p as i64;
    let rex_r_to_b = |rex: u8| (rex & !0x04) | ((rex & 0x04) >> 2);
    match kind {
        // Without PIC the address is a link-time constant, so the load
        // becomes `mov $foo, %reg` (`c7 /0 imm32`, same 7/6 bytes): an
        // immediate move depends on nothing, where the RIP-relative `lea`
        // still needs an address generation.  GNU ld makes the same choice.
        // The imm32 of the REX.W form is sign-extended and that of the 32-bit
        // form zero-extended; `s <= i32::MAX` fits both (and the signed field
        // check of the caller's write), anything larger keeps the `lea`,
        // which reaches any target of the small code model.  Only the
        // `REX_GOTPCRELX` type promises that fp-3 is this instruction's REX
        // prefix; there ModRM.reg moves into ModRM.rm, so REX.R must become
        // REX.B.  APX (`CODE_4`) forms keep the `lea`.
        GotRelax::Lea
            if !is_pic
                && s <= i32::MAX as u64
                && (rela_type == R_X86_64_GOTPCRELX
                    || (rela_type == R_X86_64_REX_GOTPCRELX && buf[fp - 3] & 0xf0 == 0x40)) =>
        {
            let modrm = buf[fp - 1];
            buf[fp - 1] = 0xc0 | ((modrm & 0x38) >> 3);
            buf[fp - 2] = 0xc7;
            if rela_type == R_X86_64_REX_GOTPCRELX {
                buf[fp - 3] = rex_r_to_b(buf[fp - 3]);
            }
            (fp, s as i64)
        }
        GotRelax::Lea => {
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

// DT_* constants now in shared module - re-export them
pub use crate::backend::elf::{
    DF_1_NOW, DF_1_PIE, DT_DEBUG, DT_FINI_ARRAY, DT_FINI_ARRAYSZ, DT_FLAGS, DT_FLAGS_1,
    DT_INIT_ARRAY, DT_INIT_ARRAYSZ, DT_PREINIT_ARRAY, DT_PREINIT_ARRAYSZ, DT_RELACOUNT, DT_RPATH,
    DT_RUNPATH, DT_SONAME, DT_VERDEF, DT_VERDEFNUM, DT_VERNEED, DT_VERNEEDNUM, DT_VERSYM,
};

pub const DF_BIND_NOW: i64 = 0x8;

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
        let mut buf = vec![0xc3];
        buf.extend_from_slice(insn);
        let fp = buf.len() - 4;
        let kind = gotpcrelx_relaxation(rela_type, -4, &buf, fp, is_pic)?;
        let p = 0x1000 + fp as u64;
        let (pos, v) = rewrite_got_relax(&mut buf, fp, kind, rela_type, is_pic, s, p);
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
            gotpcrelx_relaxation(R_X86_64_GOTPCREL, -4, &buf, 4, false),
            None
        );
        // A non -4 addend reads part of the slot (e.g. its high half).
        assert_eq!(
            gotpcrelx_relaxation(R_X86_64_REX_GOTPCRELX, 0, &buf, 4, false),
            None
        );
        // Immediate forms need a link-time-constant address and REX.W.
        let test = [&[0xc3][..], &[0x48, 0x85, 0x0d], &Z[..]].concat();
        assert_eq!(
            gotpcrelx_relaxation(R_X86_64_REX_GOTPCRELX, -4, &test, 4, true),
            None
        );
        let test32 = [&[0xc3][..], &[0x44, 0x85, 0x0d], &Z[..]].concat();
        assert_eq!(
            gotpcrelx_relaxation(R_X86_64_REX_GOTPCRELX, -4, &test32, 4, false),
            None
        );
        // Not a RIP-relative ModRM.
        let reg = [&[0xc3][..], &[0x48, 0x8b, 0x04], &Z[..]].concat();
        assert_eq!(
            gotpcrelx_relaxation(R_X86_64_REX_GOTPCRELX, -4, &reg, 4, false),
            None
        );
        // Field running past the end of the section.
        assert_eq!(
            gotpcrelx_relaxation(R_X86_64_REX_GOTPCRELX, -4, &buf[..6], 4, false),
            None
        );
    }
}
