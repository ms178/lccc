//! x86-64 / i386 side of the `.cfi_*` lowering into `.eh_frame`
//! ([`crate::backend::cfi_lower`] holds the target-neutral part): register
//! numbering, frame conventions (`call` pushes the return address, so the
//! CIE says `CFA = sp + slot`, return address at `CFA - slot`; CIE version
//! 1) and the item adapter. A location advance is an `AsmItem::CfaAdvance`
//! that the ELF writer sizes after branch relaxation, in its smallest form
//! (`DW_CFA_advance_loc`, `_loc1`, `_loc2`, `_loc4`) like GNU as.

use super::parser::{AsmItem, CfiDirective, SectionDirective, parse_asm};
use crate::backend::cfi_lower::{self, CfiItemKind, CfiStream, CfiTarget, SectionKey};

/// Frame conventions of the target the object is assembled for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CfiArch {
    X86_64,
    I386,
}

impl CfiArch {
    pub(crate) fn ptr_size(self) -> i64 {
        match self {
            CfiArch::X86_64 => 8,
            CfiArch::I386 => 4,
        }
    }
    /// Bytes one push/pop/return-address slot occupies (= pointer size).
    pub(crate) fn slot(self) -> i64 {
        self.ptr_size()
    }
    pub(crate) fn sp(self) -> &'static str {
        match self {
            CfiArch::X86_64 => "rsp",
            CfiArch::I386 => "esp",
        }
    }
    pub(crate) fn fp(self) -> &'static str {
        match self {
            CfiArch::X86_64 => "rbp",
            CfiArch::I386 => "ebp",
        }
    }
    /// Return-address column name, as `.cfi_undefined`/`.cfi_offset` take it.
    pub(crate) fn ra(self) -> &'static str {
        match self {
            CfiArch::X86_64 => "rip",
            CfiArch::I386 => "eip",
        }
    }
    /// AT&T operand-size suffix of a slot-sized push/pop/mov.
    pub(crate) fn suffix(self) -> char {
        match self {
            CfiArch::X86_64 => 'q',
            CfiArch::I386 => 'l',
        }
    }
    /// Callee-saved general registers of the psABI.
    pub(crate) fn is_callee_saved(self, r: &str) -> bool {
        match self {
            CfiArch::X86_64 => matches!(r, "rbx" | "rbp" | "r12" | "r13" | "r14" | "r15"),
            CfiArch::I386 => matches!(r, "ebx" | "esi" | "edi" | "ebp"),
        }
    }
    pub(crate) fn data_align(self) -> i64 {
        -self.ptr_size()
    }
    fn default_ra_column(self) -> u64 {
        match self {
            CfiArch::X86_64 => 16,
            CfiArch::I386 => 8,
        }
    }
    fn sp_column(self) -> u64 {
        match self {
            CfiArch::X86_64 => 7,
            CfiArch::I386 => 4,
        }
    }

    /// DWARF register number of an assembler register name (with or without
    /// `%`) or a plain register number, as GNU as accepts in CFI operands.
    pub(crate) fn dwarf_reg(self, name: &str) -> Result<u64, String> {
        let n = name.trim().trim_start_matches('%').to_ascii_lowercase();
        if let Ok(v) = n.parse::<u64>() {
            return Ok(v);
        }
        let num = |prefix: &str, base: u64, count: u64| -> Option<u64> {
            let idx: u64 = n.strip_prefix(prefix)?.parse().ok()?;
            (idx < count).then_some(base + idx)
        };
        let r = match self {
            CfiArch::X86_64 => match n.as_str() {
                "rax" | "eax" => Some(0),
                "rdx" | "edx" => Some(1),
                "rcx" | "ecx" => Some(2),
                "rbx" | "ebx" => Some(3),
                "rsi" | "esi" => Some(4),
                "rdi" | "edi" => Some(5),
                "rbp" | "ebp" => Some(6),
                "rsp" | "esp" => Some(7),
                "rip" => Some(16),
                "rflags" | "eflags" => Some(49),
                "es" => Some(50),
                "cs" => Some(51),
                "ss" => Some(52),
                "ds" => Some(53),
                "fs" => Some(54),
                "gs" => Some(55),
                "fs.base" => Some(58),
                "gs.base" => Some(59),
                _ => {
                    // r8..r15 (and their d/w/b forms), xmm/st/mm banks.
                    let gpr = n
                        .strip_prefix('r')
                        .map(|s| s.trim_end_matches(['d', 'w', 'b']))
                        .and_then(|s| s.parse::<u64>().ok())
                        .filter(|v| (8..=15).contains(v));
                    gpr.or_else(|| num("xmm", 17, 16))
                        .or_else(|| num("st", 33, 8))
                        .or_else(|| num("mm", 41, 8))
                }
            },
            CfiArch::I386 => match n.as_str() {
                "eax" => Some(0),
                "ecx" => Some(1),
                "edx" => Some(2),
                "ebx" => Some(3),
                "esp" => Some(4),
                "ebp" => Some(5),
                "esi" => Some(6),
                "edi" => Some(7),
                "eip" => Some(8),
                "eflags" => Some(9),
                _ => num("st", 11, 8)
                    .or_else(|| num("xmm", 21, 8))
                    .or_else(|| num("mm", 29, 8)),
            },
        };
        let r = r.or_else(|| {
            // `%st(3)` spelling.
            let inner = n.strip_prefix("st(")?.strip_suffix(')')?;
            let idx: u64 = inner.parse().ok()?;
            let base = if self == CfiArch::X86_64 { 33 } else { 11 };
            (idx < 8).then_some(base + idx)
        });
        r.ok_or_else(|| format!("bad register expression `{}` in CFI directive", name.trim()))
    }
}

impl CfiArch {
    /// The frame conventions GNU as encodes for this target.
    pub(crate) fn target(self) -> CfiTarget {
        CfiTarget {
            ptr_size: self.ptr_size(),
            code_align: 1,
            data_align: self.data_align(),
            default_ra_column: self.default_ra_column(),
            sp_column: self.sp_column(),
            entry_cfa_offset: self.slot(),
            ra_saved_at_entry: true,
            cie_version: None,
            pcrel_item: false,
            dwarf_reg: match self {
                CfiArch::X86_64 => |r| CfiArch::X86_64.dwarf_reg(r).ok(),
                CfiArch::I386 => |r| CfiArch::I386.dwarf_reg(r).ok(),
            },
        }
    }
}

fn section_key(dir: &SectionDirective) -> SectionKey {
    (dir.name.clone(), dir.comdat_group.clone())
}

impl CfiStream for AsmItem {
    fn cfi_kind(&self) -> CfiItemKind {
        match self {
            AsmItem::Section(dir) => CfiItemKind::Section(section_key(dir)),
            AsmItem::PushSection(dir) => CfiItemKind::PushSection(section_key(dir)),
            AsmItem::PopSection => CfiItemKind::PopSection,
            AsmItem::Previous => CfiItemKind::Previous,
            AsmItem::Cfi(dir) => {
                let (name, args): (&str, String) = match dir {
                    CfiDirective::StartProc => (".cfi_startproc", String::new()),
                    CfiDirective::EndProc => (".cfi_endproc", String::new()),
                    CfiDirective::DefCfaOffset(v) => (".cfi_def_cfa_offset", v.to_string()),
                    CfiDirective::DefCfaRegister(r) => (".cfi_def_cfa_register", r.clone()),
                    CfiDirective::Offset(r, v) => (".cfi_offset", format!("{}, {}", r, v)),
                    CfiDirective::Other(line) => {
                        let t = line.trim();
                        match t.find(char::is_whitespace) {
                            Some(i) => (&t[..i], t[i..].trim().to_string()),
                            None => (t, String::new()),
                        }
                    }
                };
                CfiItemKind::Cfi(name.to_string(), args)
            }
            // Items that never emit bytes: a CFI location label placed
            // before them still marks the same address as one after them.
            AsmItem::Label(_)
            | AsmItem::Global(_)
            | AsmItem::Weak(_)
            | AsmItem::Hidden(_)
            | AsmItem::Protected(_)
            | AsmItem::Internal(_)
            | AsmItem::SymbolType(_, _)
            | AsmItem::Size(_, _)
            | AsmItem::File(_, _)
            | AsmItem::Loc(_, _, _)
            | AsmItem::OptionDirective(_)
            | AsmItem::Symver(_, _, _)
            | AsmItem::Set(_, _)
            | AsmItem::Empty => CfiItemKind::NoBytes,
            _ => CfiItemKind::Bytes,
        }
    }
    fn as_label(&self) -> Option<&str> {
        match self {
            AsmItem::Label(l) => Some(l),
            _ => None,
        }
    }
    fn label(name: String) -> Self {
        AsmItem::Label(name)
    }
    fn parse(text: &str) -> Result<Vec<Self>, String> {
        parse_asm(text)
    }
    /// The distance between two code labels is only known after branch
    /// relaxation; the ELF writer sizes the advance once the layout is final.
    fn advance(from: String, to: String) -> Self {
        AsmItem::CfaAdvance { from, to }
    }
    fn pcrel32(sym: String) -> Result<Self, String> {
        Err(format!(
            "internal .eh_frame lowering: x86 references `{}` as `.long {} - .`",
            sym, sym
        ))
    }
}

/// Replace the CFI directives of `items` by location labels and append the
/// `.eh_frame` data they describe (see [`crate::backend::cfi_lower`]).
pub fn lower_cfi(items: Vec<AsmItem>, arch: CfiArch) -> Result<Vec<AsmItem>, String> {
    cfi_lower::lower_cfi(items, &arch.target())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Assemble `asm` with the integrated assembler for `arch` and return
    /// the raw bytes of its `.eh_frame` section (None when absent).
    fn eh_frame_of(asm: &str, arch: CfiArch) -> Option<Vec<u8>> {
        let path = std::env::temp_dir().join(format!(
            "lccc-cfi-test-{}-{:?}-{}.o",
            std::process::id(),
            arch,
            asm.len()
        ));
        let p = path.to_str().unwrap();
        match arch {
            CfiArch::X86_64 => crate::backend::x86::assembler::assemble(asm, p),
            CfiArch::I386 => crate::backend::i686::assembler::assemble(asm, p),
        }
        .expect("assembles");
        let elf = std::fs::read(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        section_bytes(&elf, ".eh_frame")
    }

    /// Minimal ELF32/ELF64 little-endian section lookup by name.
    fn section_bytes(elf: &[u8], want: &str) -> Option<Vec<u8>> {
        let u16_at = |o: usize| u16::from_le_bytes([elf[o], elf[o + 1]]) as usize;
        let u32_at = |o: usize| u32::from_le_bytes(elf[o..o + 4].try_into().unwrap()) as usize;
        let u64_at = |o: usize| u64::from_le_bytes(elf[o..o + 8].try_into().unwrap()) as usize;
        let is64 = elf[4] == 2;
        let (shoff, shentsize, shnum, shstrndx) = if is64 {
            (u64_at(0x28), u16_at(0x3a), u16_at(0x3c), u16_at(0x3e))
        } else {
            (u32_at(0x20), u16_at(0x2e), u16_at(0x30), u16_at(0x32))
        };
        let hdr = |i: usize| -> (usize, usize, usize) {
            let b = shoff + i * shentsize;
            if is64 {
                (u32_at(b), u64_at(b + 0x18), u64_at(b + 0x20))
            } else {
                (u32_at(b), u32_at(b + 0x10), u32_at(b + 0x14))
            }
        };
        let (_, stroff, _) = hdr(shstrndx);
        (0..shnum).find_map(|i| {
            let (name, off, size) = hdr(i);
            let n = &elf[stroff + name..];
            let n = &n[..n.iter().position(|&c| c == 0).unwrap()];
            (n == want.as_bytes()).then(|| elf[off..off + size].to_vec())
        })
    }

    const X64_FN: &str = "\
.text
.globl f
.type f, @function
f:
.cfi_startproc
pushq %rbx
.cfi_def_cfa_offset 16
.cfi_offset %rbx, -16
popq %rbx
.cfi_def_cfa_offset 8
ret
.cfi_endproc
.size f, .-f
";

    #[test]
    fn x86_64_cie_and_fde_match_gnu_as_layout() {
        let eh = eh_frame_of(X64_FN, CfiArch::X86_64).expect(".eh_frame emitted");
        // GNU as 2.47 (`as --64`), byte for byte: CIE "zR" (data align -8,
        // RA column 16, def_cfa rsp+8, rip at cfa-8, nop-padded to 4), then
        // the FDE: pc_range 3, advance_loc 1; def_cfa_offset 16; rbx(3) at
        // cfa-16; advance_loc 1; def_cfa_offset 8; padded to 8 as the last.
        let gnu_as: &[u8] = &[
            0x14, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x7a, 0x52, 0x00, 0x01, 0x78,
            0x10, 0x01, 0x1b, 0x0c, 0x07, 0x08, 0x90, 0x01, 0x00, 0x00, 0x1c, 0x00, 0x00, 0x00,
            0x1c, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0x00, 0x00, 0x00, 0x00, 0x41,
            0x0e, 0x10, 0x83, 0x02, 0x41, 0x0e, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        assert_eq!(eh, gnu_as);
    }

    /// A CFA offset above signed imm32 must be encoded as
    /// `DW_CFA_def_cfa_offset` (0x0e) with the exact ULEB128 value.
    ///
    /// The parser used to hand this directive through `i32`, which turned
    /// 2147483664 into -2147483632 *and* switched the opcode to
    /// `DW_CFA_def_cfa_offset_sf` (0x13, a FACTORED operand). Read back by an
    /// unwinder that is `rsp - 17179869056`, not `rsp + 2147483664`: the
    /// unwind table for every frame above 2 GiB described the wrong stack.
    /// GNU as encodes the directive as 0x0e + ULEB128, so that is the byte
    /// sequence asserted here.
    #[test]
    fn a_cfa_offset_past_imm32_stays_a_def_cfa_offset_with_the_exact_value() {
        let asm = "\
.text
.globl f
.type f, @function
f:
.cfi_startproc
subq $2147483640, %rsp
.cfi_def_cfa_offset 2147483664
addq $2147483640, %rsp
.cfi_def_cfa_offset 8
ret
.cfi_endproc
.size f, .-f
";
        let eh = eh_frame_of(asm, CfiArch::X86_64).expect(".eh_frame emitted");
        // `DW_CFA_def_cfa_offset` (0x0e) then ULEB128 2147483664 =
        // 0x90 0x80 0x80 0x80 0x08, never the 0x13 factored form.
        let wanted: &[u8] = &[0x0e, 0x90, 0x80, 0x80, 0x80, 0x08];
        assert!(
            eh.windows(wanted.len()).any(|w| w == wanted),
            "expected {wanted:02x?} in .eh_frame, got {:02x?}",
            eh
        );
        assert!(
            !eh.contains(&0x13),
            "no factored CFA operand may appear: {:02x?}",
            eh
        );
    }

    #[test]
    fn i386_uses_its_own_register_numbers_and_alignment() {
        let asm = "\
.text
f:
.cfi_startproc
pushl %ebx
.cfi_def_cfa_offset 8
.cfi_offset %ebx, -8
pushl %ebp
.cfi_def_cfa_offset 12
.cfi_offset %ebp, -12
movl %esp, %ebp
.cfi_def_cfa_register %ebp
leave
popl %ebx
ret
.cfi_endproc
";
        let eh = eh_frame_of(asm, CfiArch::I386).expect(".eh_frame emitted");
        // GNU as 2.47 (`as --32`), byte for byte: data align -4, RA column
        // 8 (eip), def_cfa esp(4)+4, eip at cfa-4; ebx(3) at cfa-8, ebp(5)
        // at cfa-12, CFA register ebp(5); everything padded to 4.
        let gnu_as: &[u8] = &[
            0x14, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x7a, 0x52, 0x00, 0x01, 0x7c,
            0x08, 0x01, 0x1b, 0x0c, 0x04, 0x04, 0x88, 0x01, 0x00, 0x00, 0x1c, 0x00, 0x00, 0x00,
            0x1c, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x07, 0x00, 0x00, 0x00, 0x00, 0x41,
            0x0e, 0x08, 0x83, 0x02, 0x41, 0x0e, 0x0c, 0x85, 0x03, 0x42, 0x0d, 0x05, 0x00, 0x00,
        ];
        assert_eq!(eh, gnu_as);
    }

    /// Location advances at every encoding boundary (63/64, 255/256,
    /// 65535/65536 bytes and zero), a second CIE emitted at its first use
    /// between FDEs, and only the last FDE padded to the pointer size.
    const ADVANCES: &str = "\
.text
f:
.cfi_startproc
.skip 0x3f
.cfi_def_cfa_offset 16
.skip 0x40
.cfi_def_cfa_offset 24
.skip 0xff
.cfi_def_cfa_offset 32
.skip 0x100
.cfi_def_cfa_offset 40
.skip 0xffff
.cfi_def_cfa_offset 48
.skip 0x10000
.cfi_def_cfa_offset 56
.cfi_def_cfa_offset 64
ret
.cfi_endproc
g:
.cfi_startproc
.cfi_signal_frame
nop
.cfi_def_cfa_offset 16
ret
.cfi_endproc
h:
.cfi_startproc
ret
.cfi_endproc
";

    #[test]
    fn advances_take_the_smallest_form_like_gnu_as() {
        // GNU as 2.47 output for ADVANCES, byte for byte. Advances:
        // 0x7f (loc 63), 02 40, 02 ff, 03 00 01, 03 ff ff, 04 00 00 01 00,
        // none for the zero distance; CIE "zRS" right before g's FDE.
        let gnu_as_64: &[u8] = &[
            0x14, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x7a, 0x52, 0x00, 0x01, 0x78,
            0x10, 0x01, 0x1b, 0x0c, 0x07, 0x08, 0x90, 0x01, 0x00, 0x00, 0x2c, 0x00, 0x00, 0x00,
            0x1c, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x7e, 0x02, 0x02, 0x00, 0x00, 0x7f,
            0x0e, 0x10, 0x02, 0x40, 0x0e, 0x18, 0x02, 0xff, 0x0e, 0x20, 0x03, 0x00, 0x01, 0x0e,
            0x28, 0x03, 0xff, 0xff, 0x0e, 0x30, 0x04, 0x00, 0x00, 0x01, 0x00, 0x0e, 0x38, 0x0e,
            0x40, 0x00, 0x14, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x7a, 0x52, 0x53,
            0x00, 0x01, 0x78, 0x10, 0x01, 0x1b, 0x0c, 0x07, 0x08, 0x90, 0x01, 0x00, 0x10, 0x00,
            0x00, 0x00, 0x1c, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00,
            0x00, 0x41, 0x0e, 0x10, 0x10, 0x00, 0x00, 0x00, 0x78, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        let gnu_as_32: &[u8] = &[
            0x14, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x7a, 0x52, 0x00, 0x01, 0x7c,
            0x08, 0x01, 0x1b, 0x0c, 0x04, 0x04, 0x88, 0x01, 0x00, 0x00, 0x2c, 0x00, 0x00, 0x00,
            0x1c, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x7e, 0x02, 0x02, 0x00, 0x00, 0x7f,
            0x0e, 0x10, 0x02, 0x40, 0x0e, 0x18, 0x02, 0xff, 0x0e, 0x20, 0x03, 0x00, 0x01, 0x0e,
            0x28, 0x03, 0xff, 0xff, 0x0e, 0x30, 0x04, 0x00, 0x00, 0x01, 0x00, 0x0e, 0x38, 0x0e,
            0x40, 0x00, 0x14, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x7a, 0x52, 0x53,
            0x00, 0x01, 0x7c, 0x08, 0x01, 0x1b, 0x0c, 0x04, 0x04, 0x88, 0x01, 0x00, 0x10, 0x00,
            0x00, 0x00, 0x1c, 0x00, 0x00, 0x00, 0x7e, 0x02, 0x02, 0x00, 0x02, 0x00, 0x00, 0x00,
            0x00, 0x41, 0x0e, 0x10, 0x10, 0x00, 0x00, 0x00, 0x78, 0x00, 0x00, 0x00, 0x80, 0x02,
            0x02, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        assert_eq!(eh_frame_of(ADVANCES, CfiArch::X86_64).unwrap(), gnu_as_64);
        assert_eq!(eh_frame_of(ADVANCES, CfiArch::I386).unwrap(), gnu_as_32);
    }

    #[test]
    fn no_cfi_means_no_eh_frame_and_untouched_items() {
        let asm = ".text\nf:\nret\n";
        assert!(eh_frame_of(asm, CfiArch::X86_64).is_none());
        let items = parse_asm(asm).unwrap();
        let n = items.len();
        assert_eq!(lower_cfi(items, CfiArch::X86_64).unwrap().len(), n);
    }

    #[test]
    fn debug_frame_only_sections_suppress_eh_frame() {
        let asm = format!(".cfi_sections .debug_frame\n{}", X64_FN);
        assert!(eh_frame_of(&asm, CfiArch::X86_64).is_none());
    }

    #[test]
    fn personality_and_lsda_select_zplr_cie() {
        let asm = "\
.text
f:
.cfi_startproc
.cfi_personality 0x9b,DW.ref.__gcc_personality_v0
.cfi_lsda 0x1b,.LLSDA0
ret
.cfi_endproc
.section .gcc_except_table,\"a\",@progbits
.LLSDA0:
.byte 0xff
.data
DW.ref.__gcc_personality_v0:
.quad __gcc_personality_v0
";
        let eh = eh_frame_of(asm, CfiArch::X86_64).expect(".eh_frame emitted");
        assert_eq!(&eh[8..14], b"\x01zPLR\0");
        // aug data length: P (1 + 4) + L (1) + R (1) = 7
        assert_eq!(eh[17], 7);
        assert_eq!(eh[18], 0x9b);
        assert_eq!(eh[23], 0x1b, "LSDA encoding");
        assert_eq!(eh[24], 0x1b, "FDE encoding");
    }

    #[test]
    fn malformed_regions_are_rejected() {
        let lower = |s: &str| lower_cfi(parse_asm(s).unwrap(), CfiArch::X86_64);
        assert!(
            lower("f:\n.cfi_startproc\nret\n")
                .unwrap_err()
                .contains("missing .cfi_endproc")
        );
        assert!(lower("f:\n.cfi_startproc\n.cfi_startproc\n").is_err());
        assert!(
            lower("f:\n.cfi_startproc\n.cfi_window_save\n.cfi_endproc\n")
                .unwrap_err()
                .contains("unsupported CFI directive")
        );
    }
}
