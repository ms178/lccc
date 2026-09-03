//! X86Codegen: global address, label address, TLS global address operations.

use super::emit::{is_xmm_reg, phys_reg_name, phys_reg_name_32, typed_phys_reg_name, X86Codegen};
use crate::common::types::IrType;
use crate::ir::reexports::{Operand, Value};

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
                    self.state
                        .emit_fmt(format_args!("    movq {}@GOTPCREL(%rip), %{}", n, d_name));
                } else {
                    self.state
                        .out
                        .emit_instr_sym_base_reg("    leaq", name, "rip", d_name);
                }
                self.state.reg_cache.invalidate_acc();
                return;
            }
        }
        if self.state.needs_got_for_addr(name) {
            let n = self.got_name(name);
            self.state
                .emit_fmt(format_args!("    movq {}@GOTPCREL(%rip), %rax", n));
        } else {
            self.state
                .out
                .emit_instr_sym_base_reg("    leaq", name, "rip", "rax");
        }
        self.store_rax_to(dest);
    }

    /// Recreate an omitted executable-data address at its audited derivation
    /// use. Offsets are staged first so loading the symbol base into %rax cannot
    /// destroy an immediately-consumed value. The result then follows the
    /// ordinary destination-location contract.
    pub(super) fn emit_rematerialized_global_addr_impl(
        &mut self,
        dest: &Value,
        name: &str,
        offset: &Operand,
        subtract: bool,
    ) -> bool {
        if self.state.needs_got_for_addr(name)
            || self.state.tls_symbols.contains(name)
            || self.state.absolute_symbols.contains(name)
        {
            return false;
        }

        // Constant displacements belong in the LEA itself. This is both
        // shorter and avoids creating a temporary live range for the offset.
        if let Operand::Const(c) = offset {
            if let Some(raw) = c.to_i64() {
                let disp = if subtract {
                    raw.checked_neg()
                } else {
                    Some(raw)
                };
                if let Some(disp) = disp.filter(|v| (i32::MIN as i64..=i32::MAX as i64).contains(v))
                {
                    let target = self
                        .dest_reg(dest)
                        .filter(|r| !is_xmm_reg(*r))
                        .map(phys_reg_name)
                        .unwrap_or("rax");
                    let sym = if disp == 0 {
                        name.to_string()
                    } else if disp > 0 {
                        format!("{name}+{disp}")
                    } else {
                        format!("{name}{disp}")
                    };
                    self.state
                        .out
                        .emit_instr_sym_base_reg("    leaq", &sym, "rip", target);
                    if target == "rax" {
                        self.store_rax_to(dest);
                    } else {
                        self.state.reg_cache.invalidate_acc();
                    }
                    return true;
                }
            }
        }

        // The common case has both the offset and result in allocated GPRs.
        // Materialize the base straight into the result and consume the offset
        // there, unless the two homes alias (which would destroy the offset).
        if let (Some(off_reg), Some(dst_reg)) = (self.operand_reg(offset), self.dest_reg(dest)) {
            if !is_xmm_reg(off_reg) && !is_xmm_reg(dst_reg) && off_reg != dst_reg {
                let off_name = phys_reg_name(off_reg);
                let dst_name = phys_reg_name(dst_reg);
                self.state
                    .out
                    .emit_instr_sym_base_reg("    leaq", name, "rip", dst_name);
                self.state.emit_fmt(format_args!(
                    "    {}q %{}, %{}",
                    if subtract { "sub" } else { "add" },
                    off_name,
                    dst_name,
                ));
                self.state.reg_cache.invalidate_acc();
                return true;
            }
        }

        // Alias/slot/immediately-consumed fallback: preserve the offset in the
        // secondary scratch before %rax receives the symbol base.
        self.operand_to_rcx(offset);
        self.state
            .out
            .emit_instr_sym_base_reg("    leaq", name, "rip", "rax");
        self.state.emit(if subtract {
            "    subq %rcx, %rax"
        } else {
            "    addq %rcx, %rax"
        });
        self.store_rax_to(dest);
        true
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
            self.state
                .emit_fmt(format_args!("    movq {}@GOTTPOFF(%rip), %rax", name));
            self.state.emit("    addq %fs:0, %rax");
        } else {
            self.state.emit("    movq %fs:0, %rax");
            self.state
                .emit_fmt(format_args!("    leaq {}@TPOFF(%rax), %rax", name));
        }
        self.store_rax_to(dest);
    }

    /// Materialise a global symbol's runtime address directly into a named
    /// 64-bit register, honouring GOT / TLS / absolute addressing exactly like
    /// `emit_global_addr_impl` / `emit_tls_global_addr_impl`, but without
    /// routing through a value's home.
    ///
    /// This is the safe reconstruction for a *rematerialisable* `GlobalAddr`
    /// (a value whose address computation was deliberately omitted because it
    /// has no stack/register home). Every generic value-load path that can
    /// encounter such a value must rebuild it here instead of fabricating 0 —
    /// a silent `xorl %ecx,%ecx` for a global address corrupted the base of
    /// `&buf[k]` pointer arithmetic (torture execute/20000412-6.c: `buf+3`
    /// decoded as `6`, not `&buf[3]`). Credit: Agent C torture triage.
    ///
    /// TLS model selection mirrors emit_tls_global_addr_impl: Local-Exec for
    /// local symbols (even under -fPIC, like GCC), Initial-Exec via GOTTPOFF
    /// only for external TLS symbols in PIC mode.
    pub(super) fn emit_global_addr_into_reg(&mut self, name: &str, reg: &str) {
        if self.state.tls_symbols.contains(name) {
            if self.state.pic_mode && !self.state.local_symbols.contains(name) {
                self.state
                    .emit_fmt(format_args!("    movq {}@GOTTPOFF(%rip), %{}", name, reg));
                self.state
                    .emit_fmt(format_args!("    addq %fs:0, %{}", reg));
            } else {
                self.state
                    .emit_fmt(format_args!("    movq %fs:0, %{}", reg));
                self.state
                    .emit_fmt(format_args!("    leaq {}@TPOFF(%{}), %{}", name, reg, reg));
            }
        } else if self.state.needs_got_for_addr(name) {
            let n = self.got_name(name);
            self.state
                .emit_fmt(format_args!("    movq {}@GOTPCREL(%rip), %{}", n, reg));
        } else if self.state.absolute_symbols.contains(name) {
            self.state.out.emit_instr_sym_imm_reg("    movq", name, reg);
        } else {
            self.state
                .out
                .emit_instr_sym_base_reg("    leaq", name, "rip", reg);
        }
        // Conservative cache hygiene: this helper writes an arbitrary named
        // register from a generic load path whose callers manage caches
        // differently; drop whichever cached mapping the write could stale.
        if reg == "rax" {
            self.state.reg_cache.invalidate_acc();
        } else if reg == "rcx" {
            self.state.reg_cache.invalidate_sec();
        }
    }

    pub(super) fn emit_global_addr_absolute_impl(&mut self, dest: &Value, name: &str) {
        // Register-direct: emit directly to dest register.
        if let Some(d_reg) = self.dest_reg(dest) {
            if !is_xmm_reg(d_reg) {
                let d_name = phys_reg_name(d_reg);
                self.state
                    .out
                    .emit_instr_sym_imm_reg("    movq", name, d_name);
                self.state.reg_cache.invalidate_acc();
                return;
            }
        }
        self.state
            .out
            .emit_instr_sym_imm_reg("    movq", name, "rax");
        self.store_rax_to(dest);
    }

    pub(super) fn emit_global_load_rip_rel_impl(&mut self, dest: &Value, sym: &str, ty: IrType) {
        // Register-direct: load directly to dest register.
        if let Some(d_reg) = self.dest_reg(dest) {
            if is_xmm_reg(d_reg) {
                // Scalar FP straight into the XMM home. `mov_load_for_type`
                // returns the INTEGER form (movq/movl) for F64/F32, and the
                // old path routed them through %rax: `movq sym(%rip), %rax;
                // movq %rax, %xmmN` — two instructions and a GPR shuttle
                // (nbody's `bodies[i].x` loads, +258 bytes). Emit the native
                // memory→XMM form instead.
                let d_name = phys_reg_name(d_reg);
                if ty == IrType::F64 {
                    self.state
                        .emit_fmt(format_args!("    movsd {}(%rip), %{}", sym, d_name));
                    return;
                }
                if ty == IrType::F32 {
                    self.state
                        .emit_fmt(format_args!("    movss {}(%rip), %{}", sym, d_name));
                    return;
                }
            } else {
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
                self.state.emit_fmt(format_args!(
                    "    {} {}(%rip), %{}",
                    load_instr, sym, d_name
                ));
                self.state.reg_cache.invalidate_acc();
                return;
            }
        }
        // No register home: scalar FP still loads memory→%xmm0 (native) and
        // stores to the slot/register via the XMM-aware store path.
        if ty == IrType::F64 {
            self.state
                .emit_fmt(format_args!("    movsd {}(%rip), %xmm0", sym));
            self.store_xmm_to(dest, "xmm0", IrType::F64);
            return;
        }
        if ty == IrType::F32 {
            self.state
                .emit_fmt(format_args!("    movss {}(%rip), %xmm0", sym));
            self.store_xmm_to(dest, "xmm0", IrType::F32);
            return;
        }
        let load_instr = Self::mov_load_for_type(ty);
        let dest_reg = Self::load_dest_reg(ty);
        self.state.emit_fmt(format_args!(
            "    {} {}(%rip), {}",
            load_instr, sym, dest_reg
        ));
        self.emit_store_result_impl(dest);
    }

    pub(super) fn emit_global_store_rip_rel_impl(&mut self, val: &Operand, sym: &str, ty: IrType) {
        // Constant-immediate direct store: `movX $imm, sym(%rip)`. Without this
        // arm EVERY constant store to a global — even `g = 5` — relayed the
        // value through %rax (`movabsq $imm,%rax; movX %eax, sym(%rip)`), and
        // unsigned 32-bit constants above i32::MAX could never take the direct
        // form at all. `direct_store_imm` is the same raw-field width contract
        // the base-register/stack destinations apply (GCC/Clang both emit the
        // single movl for `g = 3041712678u`).
        if let Operand::Const(c) = val {
            if !ty.is_float() && !matches!(ty, IrType::I128 | IrType::U128 | IrType::F128) {
                if let Some(imm) = c.to_i64() {
                    if let Some(imm_print) = super::memory::direct_store_imm(imm, ty) {
                        let store_instr = Self::mov_store_for_type(ty);
                        self.state.emit_fmt(format_args!(
                            "    {} ${}, {}(%rip)",
                            store_instr, imm_print, sym
                        ));
                        self.state.reg_cache.invalidate_all();
                        self.flush_pending_vec_store_impl();
                        self.state.invalidate_vec_peephole();
                        return;
                    }
                }
            }
        }
        // Register-direct: store directly from val register, skip operand_to_rax.
        if let Operand::Value(v) = val {
            if let Some(v_reg) = self.reg_assignments.get(&v.0).copied() {
                if is_xmm_reg(v_reg) {
                    // Scalar FP straight out of the XMM home (`movsd %xmmN,
                    // sym(%rip)`) instead of the %rax shuttle.
                    let v_name = phys_reg_name(v_reg);
                    if ty == IrType::F64 {
                        self.state
                            .emit_fmt(format_args!("    movsd %{}, {}(%rip)", v_name, sym));
                        return;
                    }
                    if ty == IrType::F32 {
                        self.state
                            .emit_fmt(format_args!("    movss %{}, {}(%rip)", v_name, sym));
                        return;
                    }
                } else {
                    let store_instr = Self::mov_store_for_type(ty);
                    let v_name = typed_phys_reg_name(v_reg, ty);
                    self.state.emit_fmt(format_args!(
                        "    {} %{}, {}(%rip)",
                        store_instr, v_name, sym
                    ));
                    return;
                }
            }
        }
        // No register home: stage scalar FP through %xmm0 (native) and store
        // memory→memory via the XMM form instead of the %rax shuttle.
        if ty == IrType::F64 {
            self.emit_fp_operand_to_xmm(val, IrType::F64, "xmm0");
            self.state
                .emit_fmt(format_args!("    movsd %xmm0, {}(%rip)", sym));
            return;
        }
        if ty == IrType::F32 {
            self.emit_fp_operand_to_xmm(val, IrType::F32, "xmm0");
            self.state
                .emit_fmt(format_args!("    movss %xmm0, {}(%rip)", sym));
            return;
        }
        self.emit_load_operand_impl(val);
        let store_instr = Self::mov_store_for_type(ty);
        let store_reg = Self::reg_for_type("rax", ty);
        self.state.emit_fmt(format_args!(
            "    {} %{}, {}(%rip)",
            store_instr, store_reg, sym
        ));
    }

    pub(super) fn emit_label_addr_impl(&mut self, dest: &Value, label: &str) {
        // Register-direct: emit directly to dest register.
        if let Some(d_reg) = self.dest_reg(dest) {
            if !is_xmm_reg(d_reg) {
                let d_name = phys_reg_name(d_reg);
                self.state
                    .out
                    .emit_instr_sym_base_reg("    leaq", label, "rip", d_name);
                self.state.reg_cache.invalidate_acc();
                return;
            }
        }
        self.state
            .out
            .emit_instr_sym_base_reg("    leaq", label, "rip", "rax");
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
            self.state
                .out
                .emit_instr_reg_rbp("    movdqu", "xmm0", slot.0);
            return;
        }
        self.operand_to_reg(&Operand::Value(*dest), "rax");
        self.state.emit("    movdqu %xmm0, (%rax)");
    }

    fn emit_load_operand_impl(&mut self, op: &Operand) {
        self.operand_to_rax(op);
    }
}
