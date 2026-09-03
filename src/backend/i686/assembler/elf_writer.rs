//! 32-bit ELF relocatable object file writer for i686.
//!
//! Thin wrapper around `ElfWriterCore` that provides i686-specific
//! instruction encoding and relocation types. Uses ELFCLASS32, EM_386,
//! and REL (not RELA) relocation format. All shared logic lives in
//! `backend::elf_writer_common`.

use super::encoder::*;
use crate::backend::elf::{ELFCLASS32, EM_386};
use crate::backend::elf_writer_common::{
    ElfWriterCore, EncodeResult, EncoderReloc, JumpDetection, X86Arch,
};
use crate::backend::x86::assembler::encoder::{
    InstructionEncoder as X86_64Encoder, R_X86_64_32, R_X86_64_32S, R_X86_64_64, R_X86_64_PC32,
    R_X86_64_PLT32,
};
use crate::backend::x86::assembler::parser::*;

/// i686 architecture implementation for the shared ELF writer.
pub struct I686Arch;

impl X86Arch for I686Arch {
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
            let is_jump =
                mnem == "jmp" || mnem == "loop" || (mnem.starts_with('j') && mnem.len() >= 2);
            if is_jump && instr.operands.len() == 1 {
                if let Operand::Label(_) = &instr.operands[0] {
                    let is_short_only = matches!(mnem.as_str(), "jecxz" | "jcxz" | "loop");
                    let is_conditional = mnem != "jmp";
                    if is_short_only && instr_len == 2 {
                        // Short-only jumps have no long form; register as already relaxed
                        Some(JumpDetection {
                            is_conditional: true,
                            already_short: true,
                        })
                    } else {
                        let expected_len = if is_conditional { 6 } else { 5 };
                        if instr_len == expected_len {
                            Some(JumpDetection {
                                is_conditional,
                                already_short: false,
                            })
                        } else {
                            None
                        }
                    }
                } else {
                    None
                }
            } else {
                None
            }
        };

        let relocations = encoder
            .relocations
            .into_iter()
            .map(|r| EncoderReloc {
                offset: r.offset,
                symbol: r.symbol,
                reloc_type: r.reloc_type,
                addend: r.addend,
                diff_symbol: r.diff_symbol,
            })
            .collect();

        Ok(EncodeResult {
            bytes: encoder.bytes,
            relocations,
            jump,
        })
    }

    fn elf_machine() -> u16 {
        EM_386
    }
    fn elf_class() -> u8 {
        ELFCLASS32
    }

    fn reloc_abs(size: usize) -> u32 {
        // `.word sym` must carry R_386_16, not R_386_32: a 32-bit patch of a
        // 2-byte slot would overwrite the neighboring field, and the kernel's
        // relocs tool special-cases R_386_16 against 16-bit segment symbols
        // (`.word (to), real_mode_seg` in realmode.h LJMPW_RM) while rejecting
        // R_386_32 against them as "Invalid absolute relocation".
        match size {
            2 => R_386_16,
            _ => R_386_32,
        }
    }
    fn reloc_abs64() -> u32 {
        R_386_32
    } // i686 doesn't have 64-bit relocs
    fn reloc_pc32() -> u32 {
        R_386_PC32
    }
    fn reloc_pc16() -> Option<u32> {
        Some(R_386_PC16)
    }
    fn reloc_pc8() -> Option<u32> {
        Some(23)
    } // R_386_PC8
    fn reloc_patch_size(reloc_type: u32) -> u8 {
        match reloc_type {
            // R_386_8 / R_386_PC8: 1-byte fields. A 4-byte addend patch of a
            // 1-byte slot clobbers the three bytes after it — the kernel's
            // arch/x86/boot/header.S lost the 'H' of its "HdrS" signature
            // exactly this way (`.byte start_of_setup-1f` precedes the
            // `.ascii "HdrS"`), and QEMU refused the bzImage as "too old".
            22 | 23 => 1,
            // R_386_16 / R_386_PC16: 2-byte fields (real-mode disp16/rel16).
            20 | 21 => 2,
            _ => 4,
        }
    }
    fn reloc_plt32() -> u32 {
        R_386_PLT32
    }

    fn uses_rel_format() -> bool {
        true
    }
    fn supports_deferred_skips() -> bool {
        true
    }
    fn resolve_set_aliases_in_data() -> bool {
        true
    }

    fn default_code_mode() -> u8 {
        32
    }

    /// Encode an instruction using the x86-64 encoder for .code64 sections.
    /// This is needed for kernel realmode trampoline code (trampoline_64.S)
    /// which is compiled with -m16 but has .code64 sections containing
    /// 64-bit instructions like jmpq, lidt with RIP-relative addressing, etc.
    /// `.code16`: real-mode encoding.
    ///
    /// Sets the encoder's 16-bit flag, which switches the ModR/M table to the
    /// 16-bit form (no SIB, disp16) and inverts the 0x66/0x67 override logic,
    /// since the default operand and address size in real mode is 16 bits.
    /// The kernel's arch/x86/boot/header.S depends on this exact behaviour.
    fn encode_instruction_code16(
        instr: &Instruction,
        section_data_len: u64,
    ) -> Result<EncodeResult, String> {
        code16_encode_inner(instr, section_data_len, false)
    }

    fn encode_instruction_code16_gcc(
        instr: &Instruction,
        section_data_len: u64,
    ) -> Result<EncodeResult, String> {
        code16_encode_inner(instr, section_data_len, true)
    }

    fn encode_instruction_code64(
        instr: &Instruction,
        _section_data_len: u64,
    ) -> Result<EncodeResult, String> {
        let mut encoder = X86_64Encoder::new();
        // Set offset to 0 so relocation offsets are relative to instruction start.
        // The ElfWriterCore will add base_offset when recording the relocations.
        encoder.offset = 0;
        encoder.encode(instr)?;

        let instr_len = encoder.bytes.len();

        // Detect jump instructions for relaxation (same logic as x86-64)
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

        // Convert x86-64 relocations. Since we're in a .code64 section of an
        // i686 object, we need to keep i686 relocation types (R_386_*) because
        // the object file is still ELF32. The linker (ld -m elf_i386) expects
        // R_386_* relocations.
        let relocations = encoder
            .relocations
            .into_iter()
            .map(|r| {
                // Map x86-64 reloc types to i686 equivalents
                let reloc_type = match r.reloc_type {
                    R_X86_64_PC32 | R_X86_64_PLT32 => R_386_PC32,
                    R_X86_64_64 | R_X86_64_32 | R_X86_64_32S => R_386_32,
                    other => other,
                };
                EncoderReloc {
                    offset: r.offset,
                    symbol: r.symbol,
                    reloc_type,
                    addend: r.addend,
                    diff_symbol: None,
                }
            })
            .collect();

        Ok(EncodeResult {
            bytes: encoder.bytes,
            relocations,
            jump,
        })
    }
}

/// Builds a 32-bit ELF relocatable object file from parsed assembly items.
pub type ElfWriter = ElfWriterCore<I686Arch>;

#[cfg(test)]
mod tests {
    use super::super::assemble;

    /// R_386_PC8 owns a 1-byte field: the REL-format addend patch must write
    /// exactly one byte, or the bytes after the field get clobbered. Linux
    /// arch/x86/boot/header.S puts `.byte start_of_setup-1f` immediately
    /// before `.ascii "HdrS"`; the old 4-byte patch turned "HdrS" into
    /// "GdrS" and QEMU refused the bzImage ("kernel too old").
    #[test]
    fn rel_pc8_addend_patch_writes_one_byte() {
        let dir = std::env::temp_dir().join(format!(
            "lccc_i686_pc8_{}_{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("t")
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("hdr.o");
        let asm = concat!(
            ".code16\n",
            ".section \".header\", \"a\"\n",
            "hdr:\n",
            "\t.byte 0xeb\n",
            "\t.byte start_of_setup-1f\n",
            "1:\n",
            "\t.ascii \"HdrS\"\n",
            "\t.word 0x020f\n",
            ".section \".entrytext\", \"ax\"\n",
            "start_of_setup:\n",
            "\t.byte 0x90\n",
        );
        assemble(asm, out.to_str().unwrap()).unwrap();
        let data = std::fs::read(&out).unwrap();
        // The 1-byte addend (-1 = 0xff) must sit in the field, with the
        // string "HdrS" intact immediately after it.
        assert!(
            data.windows(6).any(|w| w == b"\xeb\xffHdrS"),
            "PC8 addend patch clobbered bytes after the field"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// R_386_PC16 keeps working through the same patch path (2-byte slot).
    #[test]
    fn rel_pc16_addend_patch_writes_two_bytes() {
        let dir = std::env::temp_dir().join(format!(
            "lccc_i686_pc16_{}_{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("t")
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("j.o");
        let asm = concat!(
            ".code16\n",
            ".section \".text\", \"ax\"\n",
            "\t.word ext_sym\n",
            "\t.byte 0xaa\n",
            ".section \".data\"\n",
            "ext_sym:\n",
            "\t.byte 0xbb\n",
        );
        assemble(asm, out.to_str().unwrap()).unwrap();
        let data = std::fs::read(&out).unwrap();
        // `.word ext_sym` is an R_386_16 with addend 0: the 2-byte patch must
        // write 00 00 into the field and leave the 0xaa marker byte intact.
        assert!(
            data.windows(3).any(|w| w == b"\x00\x00\xaa"),
            "R_386_16 addend patch clobbered bytes after the field"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

fn code16_encode_inner(
    instr: &Instruction,
    section_data_len: u64,
    gcc_mode: bool,
) -> Result<EncodeResult, String> {
    let mut encoder = InstructionEncoder::new();
    encoder.offset = section_data_len;
    encoder.code16 = true;
    encoder.code16gcc = gcc_mode;
    encoder.encode(instr)?;

    let instr_len = encoder.bytes.len();
    let jump = {
        let mnem = &instr.mnemonic;
        let is_jump = mnem.starts_with('j') && mnem.len() >= 2;
        if is_jump && instr.operands.len() == 1 {
            if let Operand::Label(_) = &instr.operands[0] {
                let is_conditional = mnem != "jmp";
                // In 16-bit mode a near jump carries a 16-bit
                // displacement, so the long forms are one byte shorter
                // than in 32-bit mode.
                let expected_len = if is_conditional { 4 } else { 3 };
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

    let relocations = encoder
        .relocations
        .into_iter()
        .map(|r| EncoderReloc {
            offset: r.offset,
            symbol: r.symbol,
            reloc_type: r.reloc_type,
            addend: r.addend,
            diff_symbol: None,
        })
        .collect();

    Ok(EncodeResult {
        bytes: encoder.bytes,
        relocations,
        jump,
    })
}
