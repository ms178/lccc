//! I686Codegen: global address operations (global, label, TLS) and the
//! folded absolute-address load/store forms.
//!
//! All address-formation paths follow the register-direct convention of
//! the folded load: a register-assigned destination is written directly,
//! avoiding the `movl %eax, %reg` copy the accumulator staging would add.
//! When the destination has no register, the value is staged through
//! `%eax` and `store_eax_to` (which already elides the copy for a
//! destination homed in `%eax` itself).
//!
//! # Fold gates (defense in depth)
//!
//! The absolute load/store folds are gated upstream: `is_foldable_mem_ty`
//! excludes `F128` and 128-bit integers, `rip_rel_blocked` excludes GOT,
//! TLS and absolute symbols, and `supports_global_addr_fold` is false in
//! PIC mode. The `assert!`s below are a second line of defense for a
//! miscompile-critical boundary — the failure mode of a leaked fold is a
//! silently wrong address, so the checks stay in release builds too.
//! Each runs once per folded instruction; the cost is noise next to
//! string emission.

use super::emit::I686Codegen;
use crate::backend::traits::ArchCodegen;
use crate::common::types::IrType;
use crate::emit;
use crate::ir::reexports::{IrConst, Operand, Value};

/// Return the register spelling needed for a scalar store of `ty`.
///
/// In 32-bit mode only eax, ebx, ecx and edx have encodable low-byte
/// registers; in particular sil/dil/bpl/spl require a REX prefix and must
/// not be used here. Every 16-bit partial is encodable; `%sp` is listed
/// for totality even though the stack pointer is never a value home.
///
/// This is an encoding helper, not a register-allocation policy:
/// recognizing esp/sp does not make the stack pointer available for
/// allocation.
fn i686_store_register(ty: IrType, name32: &str) -> Option<&str> {
    match ty {
        IrType::I8 | IrType::U8 => match name32 {
            "eax" => Some("al"),
            "ebx" => Some("bl"),
            "ecx" => Some("cl"),
            "edx" => Some("dl"),
            _ => None,
        },
        IrType::I16 | IrType::U16 => match name32 {
            "eax" => Some("ax"),
            "ebx" => Some("bx"),
            "ecx" => Some("cx"),
            "edx" => Some("dx"),
            "esi" => Some("si"),
            "edi" => Some("di"),
            "ebp" => Some("bp"),
            "esp" => Some("sp"),
            _ => None,
        },
        _ => Some(name32),
    }
}

impl I686Codegen {
    pub(super) fn emit_global_addr_impl(&mut self, dest: &Value, name: &str) {
        let (reg, needs_store) = match self.dest_reg(dest) {
            Some(phys) => (super::emit::phys_reg_name(phys), false),
            None => ("eax", true),
        };

        if self.state.pic_mode {
            if self.state.needs_got(name) {
                emit!(self.state, "    movl {}@GOT(%ebx), %{}", name, reg);
            } else {
                emit!(self.state, "    leal {}@GOTOFF(%ebx), %{}", name, reg);
            }
        } else {
            emit!(self.state, "    movl ${}, %{}", name, reg);
        }

        // Register-direct cache discipline: a destination written into
        // %eax/%edx now holds its own value there, so the matching cache
        // entry is refreshed; a write to any other register leaves the
        // tracked pair untouched, so cached entries stay valid.
        if needs_store {
            self.state.reg_cache.invalidate_acc();
            self.store_eax_to(dest);
        } else {
            match reg {
                "eax" => self.state.reg_cache.set_acc(dest.0, false),
                "edx" => self.state.reg_cache.set_sec(dest.0, false),
                _ => {}
            }
        }
    }

    /// Folded GlobalAddr+Load using absolute addressing.
    ///
    /// The caller must restrict this fold to supported scalar types and
    /// non-PIC mode. `sym` may include a constant offset, e.g. "sym+4".
    ///
    /// Extended floating-point values require a representation-aware path;
    /// the generic scalar load must not silently truncate them.
    pub(super) fn emit_global_load_abs_impl(&mut self, dest: &Value, sym: &str, ty: IrType) {
        assert!(
            !self.state.pic_mode,
            "i686 absolute global load fold is not valid in PIC mode"
        );
        assert!(
            ty != IrType::F128,
            "i686 absolute global load fold does not support F128"
        );

        if matches!(ty, IrType::I64 | IrType::U64 | IrType::F64) {
            emit!(self.state, "    movl {}, %eax", sym);
            emit!(self.state, "    movl {}+4, %edx", sym);

            // The loads have replaced both halves of the accumulator pair.
            // Do not expose stale pair metadata to the store helper.
            self.state.reg_cache.invalidate_acc();
            self.state.reg_cache.invalidate_sec();
            self.emit_store_acc_pair_impl(dest);
            return;
        }

        let load_instr = self.mov_load_for_type(ty);

        // Register-direct: load straight into the destination register.
        // `mov_load_for_type` provides the sign/zero extension for narrow
        // integer loads into the 32-bit register.
        if let Some(d_reg) = self.dest_reg(dest) {
            let d_name = super::emit::phys_reg_name(d_reg);
            emit!(self.state, "    {} {}, %{}", load_instr, sym, d_name);
            match d_name {
                "eax" => self.state.reg_cache.set_acc(dest.0, false),
                "edx" => self.state.reg_cache.set_sec(dest.0, false),
                _ => {}
            }
            return;
        }

        emit!(self.state, "    {} {}, %eax", load_instr, sym);
        self.state.reg_cache.invalidate_acc();
        self.store_eax_to(dest);
    }

    /// Folded GlobalAddr+Store using absolute addressing.
    ///
    /// As with the load fold, the caller must establish that the type and
    /// memory-access semantics are supported. Splitting a 64-bit operation
    /// into two movl instructions does not provide atomicity.
    pub(super) fn emit_global_store_abs_impl(&mut self, val: &Operand, sym: &str, ty: IrType) {
        assert!(
            !self.state.pic_mode,
            "i686 absolute global store fold is not valid in PIC mode"
        );
        assert!(
            ty != IrType::F128,
            "i686 absolute global store fold does not support F128"
        );

        if matches!(ty, IrType::I64 | IrType::U64 | IrType::F64) {
            self.emit_load_acc_pair_impl(val);
            emit!(self.state, "    movl %eax, {}", sym);
            emit!(self.state, "    movl %edx, {}+4", sym);
            self.state.reg_cache.invalidate_acc();
            self.state.reg_cache.invalidate_sec();
            return;
        }

        // Integer constants can be stored without a scratch register.
        // Truncate to the actual store width and print a signed immediate;
        // this preserves the low bits without relying on assembler
        // acceptance of out-of-range positive decimal immediates.
        //
        // Floating-point constants must retain their bit representation,
        // not undergo a numerical conversion through to_i64().
        if let Operand::Const(c) = val {
            if !matches!(c, IrConst::F32(_) | IrConst::F64(_)) {
                if let Some(v) = c.to_i64() {
                    let (mnem, imm) = match ty {
                        IrType::I8 | IrType::U8 => ("movb", (v as i8) as i64),
                        IrType::I16 | IrType::U16 => ("movw", (v as i16) as i64),
                        _ => ("movl", (v as i32) as i64),
                    };
                    emit!(self.state, "    {} ${}, {}", mnem, imm, sym);
                    return;
                }
            }
        }

        let store_instr = self.mov_store_for_type(ty);

        if let Operand::Value(v) = val {
            if let Some(&phys) = self.reg_assignments.get(&v.0) {
                let name32 = super::emit::phys_reg_name(phys);
                if let Some(reg) = i686_store_register(ty, name32) {
                    emit!(self.state, "    {} %{}, {}", store_instr, reg, sym);
                    return;
                }
            }
        }

        // In particular, byte values allocated to esi/edi/ebp must be
        // staged through a register with an encodable low-byte form.
        self.operand_to_eax(val);
        let src = self.eax_for_type(ty);
        emit!(self.state, "    {} {}, {}", store_instr, src, sym);
    }

    pub(super) fn emit_label_addr_impl(&mut self, dest: &Value, label: &str) {
        let (reg, needs_store) = match self.dest_reg(dest) {
            Some(phys) => (super::emit::phys_reg_name(phys), false),
            None => ("eax", true),
        };

        if self.state.pic_mode {
            emit!(self.state, "    leal {}@GOTOFF(%ebx), %{}", label, reg);
        } else {
            emit!(self.state, "    movl ${}, %{}", label, reg);
        }

        if needs_store {
            self.state.reg_cache.invalidate_acc();
            self.store_eax_to(dest);
        } else {
            match reg {
                "eax" => self.state.reg_cache.set_acc(dest.0, false),
                "edx" => self.state.reg_cache.set_sec(dest.0, false),
                _ => {}
            }
        }
    }

    /// Emit the existing ELF i386 TLS addressing sequences:
    /// initial-exec in PIC mode, local-exec otherwise.
    ///
    /// Selecting these models is an upstream responsibility. PIC alone
    /// does not prove that initial-exec is suitable for every shared
    /// object. `%ebx` cannot be the destination in a GOT-live function
    /// (the allocator reserves it), so writing the destination register
    /// directly is safe in every mode.
    pub(super) fn emit_tls_global_addr_impl(&mut self, dest: &Value, name: &str) {
        let (reg, needs_store) = match self.dest_reg(dest) {
            Some(phys) => (super::emit::phys_reg_name(phys), false),
            None => ("eax", true),
        };

        if self.state.pic_mode {
            emit!(self.state, "    movl {}@GOTNTPOFF(%ebx), %{}", name, reg);
            emit!(self.state, "    addl %gs:0, %{}", reg);
        } else {
            emit!(self.state, "    movl %gs:0, %{}", reg);
            emit!(self.state, "    addl ${}@NTPOFF, %{}", name, reg);
        }

        if needs_store {
            self.state.reg_cache.invalidate_acc();
            self.store_eax_to(dest);
        } else {
            match reg {
                "eax" => self.state.reg_cache.set_acc(dest.0, false),
                "edx" => self.state.reg_cache.set_sec(dest.0, false),
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_store_registers_are_encodable_in_i386_mode() {
        let cases = [
            ("eax", Some("al")),
            ("ebx", Some("bl")),
            ("ecx", Some("cl")),
            ("edx", Some("dl")),
            ("esi", None),
            ("edi", None),
            ("ebp", None),
            ("esp", None),
        ];

        for ty in [IrType::I8, IrType::U8] {
            for (reg, expected) in cases {
                assert_eq!(i686_store_register(ty, reg), expected, "{reg}");
            }
        }
    }

    #[test]
    fn word_store_registers_use_16_bit_names() {
        let cases = [
            ("eax", "ax"),
            ("ebx", "bx"),
            ("ecx", "cx"),
            ("edx", "dx"),
            ("esi", "si"),
            ("edi", "di"),
            ("ebp", "bp"),
            ("esp", "sp"),
        ];

        for ty in [IrType::I16, IrType::U16] {
            for (reg, expected) in cases {
                assert_eq!(i686_store_register(ty, reg), Some(expected), "{reg}");
            }
        }
    }

    #[test]
    fn dword_stores_preserve_register_names() {
        for ty in [IrType::I32, IrType::U32, IrType::F32] {
            for reg in ["eax", "ebx", "ecx", "edx", "esi", "edi", "ebp", "esp"] {
                assert_eq!(i686_store_register(ty, reg), Some(reg));
            }
        }
    }

    #[test]
    fn unknown_partial_registers_are_rejected() {
        for ty in [IrType::I8, IrType::U8, IrType::I16, IrType::U16] {
            assert_eq!(i686_store_register(ty, "not_a_register"), None);
        }
    }
}
