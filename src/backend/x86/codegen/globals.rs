//! X86Codegen: global address, label address, TLS global address operations.

use crate::ir::reexports::{Operand, Value};
use crate::common::types::IrType;
use super::emit::{X86Codegen, phys_reg_name, phys_reg_name_32, typed_phys_reg_name, is_xmm_reg};

impl X86Codegen {
    /// GOTPCREL references must use the BASE symbol name: for a versioned
    /// symbol (`printf@GLIBC_2.2.5`, bound by a `.symver` directive) GAS 2.47
    /// rejects `sym@ver@GOTPCREL` ("junk after expression") — GCC emits
    /// `sym@GOTPCREL` and the linker resolves the version. Splitting at the
    /// first '@' yields the base name; ELF symbol names cannot contain '@'.
    fn got_name<'a>(&self, name: &'a str) -> &'a str {
        name.split('@').next().unwrap_or(name)
    }

    pub(super) fn emit_global_addr_impl(&mut self, dest: &Value, name: &str) {
        // Register-direct: emit directly to dest register, skip %rax relay.
        if let Some(d_reg) = self.dest_reg(dest) {
            if !is_xmm_reg(d_reg) {
                let d_name = phys_reg_name(d_reg);
                if self.state.needs_got_for_addr(name) {
                    let n = self.got_name(name);
                    self.state.emit_fmt(format_args!("    movq {}@GOTPCREL(%rip), %{}", n, d_name));
                } else {
                    self.state.out.emit_instr_sym_base_reg("    leaq", name, "rip", d_name);
                }
                self.state.reg_cache.invalidate_acc();
                return;
            }
        }
        if self.state.needs_got_for_addr(name) {
            let n = self.got_name(name);
            self.state.emit_fmt(format_args!("    movq {}@GOTPCREL(%rip), %rax", n));
        } else {
            self.state.out.emit_instr_sym_base_reg("    leaq", name, "rip", "rax");
        }
        self.store_rax_to(dest);
    }

    pub(super) fn emit_tls_global_addr_impl(&mut self, dest: &Value, name: &str) {
        // TLS requires %rax for the fs:0 base — can't fully avoid accumulator.
        // But when dest has a register, we can skip the final store_rax_to relay.
        // TLS model selection: LOCAL symbols (static/visibility-hidden TLS, the
        // common `static __thread` case) use the Local-Exec model — a direct
        // %fs:offset with NO GOT entry — even under -fPIC, exactly like GCC.
        // Only external TLS symbols need the GOT-based General-Dynamic sequence
        // in PIC mode (globals_tls regression: static __thread read garbage
        // when GOTTPOFF was forced for a local symbol).
        if self.state.pic_mode && !self.state.local_symbols.contains(name) {
            self.state.emit_fmt(format_args!("    movq {}@GOTTPOFF(%rip), %rax", name));
            self.state.emit("    addq %fs:0, %rax");
        } else {
            self.state.emit("    movq %fs:0, %rax");
            self.state.emit_fmt(format_args!("    leaq {}@TPOFF(%rax), %rax", name));
        }
        self.store_rax_to(dest);
    }

    pub(super) fn emit_global_addr_absolute_impl(&mut self, dest: &Value, name: &str) {
        // Register-direct: emit directly to dest register.
        if let Some(d_reg) = self.dest_reg(dest) {
            if !is_xmm_reg(d_reg) {
                let d_name = phys_reg_name(d_reg);
                self.state.out.emit_instr_sym_imm_reg("    movq", name, d_name);
                self.state.reg_cache.invalidate_acc();
                return;
            }
        }
        self.state.out.emit_instr_sym_imm_reg("    movq", name, "rax");
        self.store_rax_to(dest);
    }

    pub(super) fn emit_global_load_rip_rel_impl(&mut self, dest: &Value, sym: &str, ty: IrType) {
        // Register-direct: load directly to dest register.
        if let Some(d_reg) = self.dest_reg(dest) {
            if !is_xmm_reg(d_reg) {
                let load_instr = Self::mov_load_for_type(ty);
                // The MNEMONIC decides the destination width, not the source
                // type. `mov_load_for_type` returns movzbl/movzwl/movl for
                // U8/U16/U32 (a 32-bit destination zero-extends to 64 bits for
                // free and saves a REX byte); pairing those with a 64-bit
                // register emits `movzbl sym(%rip), %rdi`, which GNU as rejects
                // ("incorrect register `%rdi' used with `l' suffix").
                let d_name = if matches!(load_instr, "movzbl" | "movzwl" | "movl") {
                    phys_reg_name_32(d_reg)
                } else {
                    phys_reg_name(d_reg)
                };
                self.state.emit_fmt(format_args!("    {} {}(%rip), %{}", load_instr, sym, d_name));
                self.state.reg_cache.invalidate_acc();
                return;
            }
        }
        let load_instr = Self::mov_load_for_type(ty);
        let dest_reg = Self::load_dest_reg(ty);
        self.state.emit_fmt(format_args!("    {} {}(%rip), {}", load_instr, sym, dest_reg));
        self.emit_store_result_impl(dest);
    }

    pub(super) fn emit_global_store_rip_rel_impl(&mut self, val: &Operand, sym: &str, ty: IrType) {
        // Register-direct: store directly from val register, skip operand_to_rax.
        if let Operand::Value(v) = val {
            if let Some(v_reg) = self.reg_assignments.get(&v.0).copied() {
                if !is_xmm_reg(v_reg) {
                    let store_instr = Self::mov_store_for_type(ty);
                    let v_name = typed_phys_reg_name(v_reg, ty);
                    self.state.emit_fmt(format_args!("    {} %{}, {}(%rip)", store_instr, v_name, sym));
                    return;
                }
            }
        }
        self.emit_load_operand_impl(val);
        let store_instr = Self::mov_store_for_type(ty);
        let store_reg = Self::reg_for_type("rax", ty);
        self.state.emit_fmt(format_args!("    {} %{}, {}(%rip)", store_instr, store_reg, sym));
    }

    pub(super) fn emit_label_addr_impl(&mut self, dest: &Value, label: &str) {
        // Register-direct: emit directly to dest register.
        if let Some(d_reg) = self.dest_reg(dest) {
            if !is_xmm_reg(d_reg) {
                let d_name = phys_reg_name(d_reg);
                self.state.out.emit_instr_sym_base_reg("    leaq", label, "rip", d_name);
                self.state.reg_cache.invalidate_acc();
                return;
            }
        }
        self.state.out.emit_instr_sym_base_reg("    leaq", label, "rip", "rax");
        self.store_rax_to(dest);
    }

    // These thin helpers avoid circular delegation issues:
    fn emit_store_result_impl(&mut self, dest: &Value) {
        self.store_rax_to(dest);
    }

    pub(super) fn emit_store_f128_xmm0(&mut self, dest: &Value) {
        if self
            .state
            .value_use_counts
            .get(dest.0 as usize)
            .copied()
            .unwrap_or(0)
            == 0
        {
            return;
        }
        if let Some(&reg) = self.reg_assignments.get(&dest.0) {
            if is_xmm_reg(reg) && reg.0 != 0 {
                self.state
                    .emit_fmt(format_args!("    movdqa %xmm0, %{}", phys_reg_name(reg)));
                return;
            }
        }
        if let Some(slot) = self.state.get_slot(dest.0) {
            self.state.out.emit_instr_reg_rbp("    movdqu", "xmm0", slot.0);
            return;
        }
        self.operand_to_reg(&Operand::Value(*dest), "rax");
        self.state.emit("    movdqu %xmm0, (%rax)");
    }

    fn emit_load_operand_impl(&mut self, op: &Operand) {
        self.operand_to_rax(op);
    }
}
