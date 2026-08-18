//! I686Codegen: global address operations (global, label, TLS).

use crate::ir::reexports::Value;
use crate::emit;
use super::emit::I686Codegen;
use crate::backend::traits::ArchCodegen;

impl I686Codegen {
    pub(super) fn emit_global_addr_impl(&mut self, dest: &Value, name: &str) {
        if self.state.pic_mode {
            if self.state.needs_got(name) {
                emit!(self.state, "    movl {}@GOT(%ebx), %eax", name);
            } else {
                emit!(self.state, "    leal {}@GOTOFF(%ebx), %eax", name);
            }
        } else {
            emit!(self.state, "    movl ${}, %eax", name);
        }
        self.state.reg_cache.invalidate_acc();
        self.store_eax_to(dest);
    }

    /// Folded GlobalAddr+Load: `movl sym, %eax` (absolute addressing).
    /// Only reachable in non-PIC mode (supports_global_addr_fold gates on it).
    /// The `sym` may carry a constant offset ("sym+4") from the GEP fold.
    pub(super) fn emit_global_load_abs_impl(&mut self, dest: &Value, sym: &str, ty: crate::common::types::IrType) {
        use crate::common::types::IrType;
        if ty == IrType::I64 || ty == IrType::U64 || ty == IrType::F64 {
            emit!(self.state, "    movl {}, %eax", sym);
            emit!(self.state, "    movl {}+4, %edx", sym);
            self.emit_store_acc_pair_impl(dest);
            self.state.reg_cache.invalidate_acc();
            return;
        }
        let load_instr = self.mov_load_for_type(ty);
        // Register-direct: load straight into the destination register.
        if let Some(d_reg) = self.dest_reg(dest) {
            let d_name = super::emit::phys_reg_name(d_reg);
            emit!(self.state, "    {} {}, %{}", load_instr, sym, d_name);
            self.state.reg_cache.invalidate_acc();
            return;
        }
        emit!(self.state, "    {} {}, %eax", load_instr, sym);
        self.state.reg_cache.invalidate_acc();
        self.store_eax_to(dest);
    }

    /// Folded GlobalAddr+Store: `movl %eax, sym` (absolute addressing).
    pub(super) fn emit_global_store_abs_impl(&mut self, val: &crate::ir::reexports::Operand, sym: &str, ty: crate::common::types::IrType) {
        use crate::common::types::IrType;
        use crate::ir::reexports::{Operand, IrConst};
        if ty == IrType::I64 || ty == IrType::U64 || ty == IrType::F64 {
            self.emit_load_acc_pair_impl(val);
            emit!(self.state, "    movl %eax, {}", sym);
            emit!(self.state, "    movl %edx, {}+4", sym);
            self.state.reg_cache.invalidate_acc();
            return;
        }
        // Constant store folds to an immediate store — no register at all.
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
        // Register-direct: store straight from the value's register.
        let store_instr = self.mov_store_for_type(ty);
        if let Operand::Value(v) = val {
            if let Some(&phys) = self.reg_assignments.get(&v.0) {
                let name32 = super::emit::phys_reg_name(phys);
                // Sub-word stores need the matching partial register; ebx/ecx/edx
                // have byte forms, esi/edi/ebp do not — fall back to %eax staging.
                let partial = match (ty, name32) {
                    (IrType::I8 | IrType::U8, "ebx") => Some("bl"),
                    (IrType::I8 | IrType::U8, "ecx") => Some("cl"),
                    (IrType::I8 | IrType::U8, "edx") => Some("dl"),
                    (IrType::I8 | IrType::U8, _) => None,
                    (IrType::I16 | IrType::U16, "ebx") => Some("bx"),
                    (IrType::I16 | IrType::U16, "ecx") => Some("cx"),
                    (IrType::I16 | IrType::U16, "edx") => Some("dx"),
                    (IrType::I16 | IrType::U16, "esi") => Some("si"),
                    (IrType::I16 | IrType::U16, "edi") => Some("di"),
                    (IrType::I16 | IrType::U16, "ebp") => Some("bp"),
                    (IrType::I16 | IrType::U16, _) => None,
                    _ => Some(name32),
                };
                if let Some(r) = partial {
                    emit!(self.state, "    {} %{}, {}", store_instr, r, sym);
                    return;
                }
            }
        }
        self.operand_to_eax(val);
        let src = self.eax_for_type(ty);
        emit!(self.state, "    {} {}, {}", store_instr, src, sym);
    }

    pub(super) fn emit_label_addr_impl(&mut self, dest: &Value, label: &str) {
        if self.state.pic_mode {
            emit!(self.state, "    leal {}@GOTOFF(%ebx), %eax", label);
        } else {
            emit!(self.state, "    movl ${}, %eax", label);
        }
        self.state.reg_cache.invalidate_acc();
        self.store_eax_to(dest);
    }

    pub(super) fn emit_tls_global_addr_impl(&mut self, dest: &Value, name: &str) {
        if self.state.pic_mode {
            emit!(self.state, "    movl {}@GOTNTPOFF(%ebx), %eax", name);
            self.state.emit("    addl %gs:0, %eax");
        } else {
            self.state.emit("    movl %gs:0, %eax");
            emit!(self.state, "    addl ${}@NTPOFF, %eax", name);
        }
        self.state.reg_cache.invalidate_acc();
        self.store_eax_to(dest);
    }
}
