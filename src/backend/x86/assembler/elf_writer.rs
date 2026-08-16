//! ELF relocatable object file writer for x86-64.
//!
//! Thin wrapper around `ElfWriterCore` that provides x86-64-specific
//! instruction encoding and relocation types. All shared logic (section
//! management, label tracking, jump relaxation, ELF emission) lives in
//! `backend::elf_writer_common`.

use super::encoder::*;
use crate::backend::elf::{ELFCLASS64, EM_X86_64};
use crate::backend::elf_writer_common::{
    X86Arch, ElfWriterCore, EncodeResult, EncoderReloc, JumpDetection,
};

/// x86-64 architecture implementation for the shared ELF writer.
pub struct X86_64Arch;

impl X86Arch for X86_64Arch {
    fn encode_instruction(
        instr: &Instruction,
        section_data_len: u64,
    ) -> Result<EncodeResult, String> {
        let mut encoder = InstructionEncoder::new();
        encoder.offset = section_data_len;
        encoder.encode(instr)?;

        let instr_len = encoder.bytes.len();

        // Detect jump instructions for relaxation
        let jump = {
            let mnem = &instr.mnemonic;
            let is_jump = mnem.starts_with('j') && mnem.len() >= 2;
            if is_jump && instr.operands.len() == 1 {
                if let Operand::Label(_) = &instr.operands[0] {
                    let is_conditional = mnem != "jmp";
                    let expected_len = if is_conditional { 6 } else { 5 };
                    if instr_len == expected_len {
                        Some(JumpDetection {
                            is_conditional,
                            already_short: false,
                        })
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            }
        };

        let relocations = encoder.relocations.into_iter().map(|r| {
            EncoderReloc {
                offset: r.offset,
                symbol: r.symbol,
                reloc_type: r.reloc_type,
                addend: r.addend,
                diff_symbol: r.diff_symbol,
            }
        }).collect();

        Ok(EncodeResult {
            bytes: encoder.bytes,
            relocations,
            jump,
        })
    }

    /// Encode a `.code32` instruction with the 32-bit encoder.
    ///
    /// The kernel's EFI mixed-mode entry (arch/x86/boot/startup/efi-mixed.S)
    /// and the realmode trampolines put genuine 32-bit assembly inside a
    /// 64-bit object via `.code32`. Encoding those with the 64-bit encoder is
    /// wrong twice over: it rejects legal 32-bit forms such as `popl %ecx`,
    /// and where it does not reject them it would emit 64-bit operand sizes.
    fn encode_instruction_code32(
        instr: &Instruction,
        _section_data_len: u64,
    ) -> Result<EncodeResult, String> {
        use crate::backend::i686::assembler::encoder::{
            InstructionEncoder as I686Encoder,
            R_386_32, R_386_PC32, R_386_PLT32,
        };
        let mut encoder = I686Encoder::new();
        encoder.offset = 0;
        encoder.encode(instr)?;

        let instr_len = encoder.bytes.len();
        // Jump relaxation detection: same rules as the native i686 writer,
        // including the short-only forms (jecxz/jcxz/loop have no long form).
        let jump = {
            let mnem = &instr.mnemonic;
            let is_jump = mnem == "jmp" || mnem == "loop"
                || (mnem.starts_with('j') && mnem.len() >= 2);
            if is_jump && instr.operands.len() == 1 {
                if let Operand::Label(_) = &instr.operands[0] {
                    let is_short_only = matches!(mnem.as_str(), "jecxz" | "jcxz" | "loop");
                    let is_conditional = mnem != "jmp";
                    if is_short_only && instr_len == 2 {
                        Some(JumpDetection { is_conditional: true, already_short: true })
                    } else {
                        let expected_len = if is_conditional { 6 } else { 5 };
                        if instr_len == expected_len {
                            Some(JumpDetection { is_conditional, already_short: false })
                        } else {
                            None
                        }
                    }
                } else { None }
            } else { None }
        };

        // The i686 encoder emits R_386_* relocation numbers, but the
        // containing object is ELF64 and the linker interprets reloc types
        // by machine. Passing them through raw is a type-pun across tables:
        // R_386_32(=1) would be read as R_X86_64_64(=1) — an 8-byte patch
        // that overwrites 4 bytes of the NEXT instruction — and
        // R_386_GOTOFF(=9) as R_X86_64_GOTPCREL(=9), a different computation
        // entirely. Translate the types with identical width and semantics
        // (R_386_32 = S+A word32 = R_X86_64_32; R_386_PC32 = S+A-P word32 =
        // R_X86_64_PC32) and reject anything else loudly rather than emit a
        // silently corrupt object.
        let mut relocations = Vec::with_capacity(encoder.relocations.len());
        for r in encoder.relocations {
            let reloc_type = match r.reloc_type {
                R_386_32 => R_X86_64_32,
                R_386_PC32 => R_X86_64_PC32,
                R_386_PLT32 => R_X86_64_PLT32,
                other => return Err(format!(
                    ".code32 in 64-bit object: relocation type {} for '{}' has no \
                     R_X86_64_* equivalent with matching semantics", other, r.symbol)),
            };
            relocations.push(EncoderReloc {
                offset: r.offset,
                symbol: r.symbol,
                reloc_type,
                addend: r.addend,
                diff_symbol: r.diff_symbol,
            });
        }

        Ok(EncodeResult { bytes: encoder.bytes, relocations, jump })
    }

    fn elf_machine() -> u16 { EM_X86_64 }
    fn elf_class() -> u8 { ELFCLASS64 }

    fn reloc_abs(size: usize) -> u32 {
        match size {
            2 => R_X86_64_16,
            4 => R_X86_64_32,
            _ => R_X86_64_64,
        }
    }
    fn reloc_abs64() -> u32 { R_X86_64_64 }
    fn reloc_pc32() -> u32 { R_X86_64_PC32 }
    fn reloc_pc64() -> u32 { R_X86_64_PC64 }
    fn reloc_plt32() -> u32 { R_X86_64_PLT32 }

    fn uses_rel_format() -> bool { false }

    fn reloc_pc8_internal() -> Option<u32> { Some(R_X86_64_PC8_INTERNAL) }
    fn reloc_abs32_for_internal() -> Option<u32> { Some(R_X86_64_32) }
    fn supports_deferred_skips() -> bool { true }
    fn resolve_set_aliases_in_data() -> bool { true }
}

/// Builds an ELF relocatable object file from parsed assembly items.
pub type ElfWriter = ElfWriterCore<X86_64Arch>;
