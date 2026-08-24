//! X86Codegen: floating-point binary operations and F128 negation.

use super::emit::X86Codegen;

use super::emit::{is_xmm_reg, phys_reg_name, phys_reg_name_32, typed_phys_reg_name};

use crate::backend::cast::FloatOp;

use crate::common::types::IrType;

use crate::ir::reexports::{IrConst, IrUnaryOp, Operand, Value};

impl X86Codegen {
    /// Load an F32/F64 operand into %xmm0, honoring a register-allocated XMM

    /// home first. The F64 register allocator uses xmm3-xmm7 only (never

    /// clobbered by codegen scratch), and an allocated value has no stack slot,

    /// so the register IS the value's only home — a `movsd %xmmN, %xmm0` is the

    /// whole load. Falls back to a direct slot load, then to the GPR round-trip

    /// for constants and values without a home.

    pub(super) fn load_fp_to_xmm0(&mut self, op: &Operand, ty: IrType) {
        let mov_instr = if ty == IrType::F64 { "movsd" } else { "movss" };
        if let Operand::Value(v) = op {
            if let Some(&reg) = self.reg_assignments.get(&v.0) {
                if is_xmm_reg(reg) {
                    let name = phys_reg_name(reg);
                    if name != "xmm0" {
                        self.state
                            .emit_fmt(format_args!("    {} %{}, %xmm0", mov_instr, name));
                    }
                    return;
                }
            }
            if let Some(slot) = self.state.get_slot(v.0) {
                let sr = self.slot_ref(slot.0);
                self.state
                    .emit_fmt(format_args!("    {} {}, %xmm0", mov_instr, sr));
                return;
            }
        }
        match op {
            Operand::Const(IrConst::F64(v)) => {
                let bits = v.to_bits();
                if bits == 0 {
                    self.state.emit("    xorpd %xmm0, %xmm0");
                } else {
                    let label = self.state.get_fp_const_label(bits);
                    self.state
                        .emit_fmt(format_args!("    movsd {}(%rip), %xmm0", label));
                }
            }
            Operand::Const(IrConst::F32(v)) => {
                let bits = v.to_bits() as u64;
                if bits == 0 {
                    self.state.emit("    xorps %xmm0, %xmm0");
                } else {
                    let label = self.state.get_fp_const_label(bits);
                    self.state
                        .emit_fmt(format_args!("    movss {}(%rip), %xmm0", label));
                }
            }
            _ => {
                self.operand_to_rax(op);
                if ty == IrType::F32 {
                    self.state.emit("    movd %eax, %xmm0");
                } else {
                    self.state.emit("    movq %rax, %xmm0");
                }
            }
        }
    }

    /// Store %xmm0 to an F32/F64 destination, honoring a register-allocated XMM

    /// home first (mirrors load_fp_to_xmm0; the result stays in the XMM domain).

    pub(super) fn store_xmm0_fp_dest(&mut self, dest: &Value, ty: IrType) {
        let mov_instr = if ty == IrType::F64 { "movsd" } else { "movss" };
        if let Some(&reg) = self.reg_assignments.get(&dest.0) {
            if is_xmm_reg(reg) {
                let name = phys_reg_name(reg);
                if name != "xmm0" {
                    self.state
                        .emit_fmt(format_args!("    {} %xmm0, %{}", mov_instr, name));
                }
                self.state.reg_cache.invalidate_acc();
                return;
            }
        }
        if let Some(slot) = self.state.get_slot(dest.0) {
            let sr = self.slot_ref(slot.0);
            self.state
                .emit_fmt(format_args!("    {} %xmm0, {}", mov_instr, sr));
            self.state.reg_cache.invalidate_acc();
            return;
        }
        if ty == IrType::F32 {
            self.state.emit("    movd %xmm0, %eax");
        } else {
            self.state.emit("    movq %xmm0, %rax");
        }
        self.state.reg_cache.invalidate_acc();
        self.store_rax_to(dest);
    }

    /// Load an F32/F64 operand into a NAMED xmm register (xmm3-xmm7 target for

    /// the compute-into-dest path; xmm0 for the scratch path). Honors a

    /// register-allocated XMM home, then a direct slot, then constants and the

    /// GPR fallback.

    pub(super) fn load_fp_to_reg(&mut self, op: &Operand, ty: IrType, reg: &str) {
        let mov_instr = if ty == IrType::F64 { "movsd" } else { "movss" };
        if let Operand::Value(v) = op {
            if let Some(&r) = self.reg_assignments.get(&v.0) {
                if is_xmm_reg(r) {
                    let name = phys_reg_name(r);
                    if name != reg {
                        self.state
                            .emit_fmt(format_args!("    {} %{}, %{}", mov_instr, name, reg));
                    }
                    return;
                }
            }
            if let Some(slot) = self.state.get_slot(v.0) {
                let sr = self.slot_ref(slot.0);
                self.state
                    .emit_fmt(format_args!("    {} {}, %{}", mov_instr, sr, reg));
                return;
            }
        }
        match op {
            Operand::Const(IrConst::F64(v)) => {
                let bits = v.to_bits();
                if bits == 0 {
                    self.state
                        .emit_fmt(format_args!("    xorpd %{}, %{}", reg, reg));
                } else {
                    let label = self.state.get_fp_const_label(bits);
                    self.state
                        .emit_fmt(format_args!("    movsd {}(%rip), %{}", label, reg));
                }
            }
            Operand::Const(IrConst::F32(v)) => {
                let bits = v.to_bits() as u64;
                if bits == 0 {
                    self.state
                        .emit_fmt(format_args!("    xorps %{}, %{}", reg, reg));
                } else {
                    let label = self.state.get_fp_const_label(bits);
                    self.state
                        .emit_fmt(format_args!("    movss {}(%rip), %{}", label, reg));
                }
            }
            _ => {
                // Pick a scratch register that does not alias the final
                // target. Before this fix the F32/F64 fallback hard-coded
                // %xmm1, clobbering the destination when callers passed
                // reg == "xmm1" (e.g. the FMA path loading mul_lhs into xmm1).
                // We simply load directly into the target via rax/rcx.
                let gpr = if reg == "xmm0" { "rax" } else { "rcx" };
                let gpr32 = if reg == "xmm0" { "eax" } else { "ecx" };
                if gpr == "rax" {
                    self.operand_to_rax(op);
                } else {
                    self.operand_to_rcx(op);
                }
                if ty == IrType::F32 {
                    self.state
                        .emit_fmt(format_args!("    movd %{}, %{}", gpr32, reg));
                } else {
                    self.state
                        .emit_fmt(format_args!("    movq %{}, %{}", gpr, reg));
                }
            }
        }
    }

    /// Emit an FP binary op with the result left in the named `reg` (an

    /// XMM-allocated destination), avoiding the store-to-home + reload-from-home

    /// pair the xmm0-scratch path pays on every link of an FP chain.

    /// Returns true when the op was emitted into `reg`.

    fn emit_float_binop_into_reg(
        &mut self,
        dest: &Value,
        op: FloatOp,
        lhs: &Operand,
        rhs: &Operand,
        ty: IrType,
        reg: &str,
    ) -> bool {
        let Some(&dreg) = self.reg_assignments.get(&dest.0) else {
            return false;
        };
        if !is_xmm_reg(dreg) || phys_reg_name(dreg) != reg {
            return false;
        }
        let mnemonic = self.emit_float_binop_mnemonic_impl(op);
        let suffix = if ty == IrType::F64 { "sd" } else { "ss" };
        let commutative = matches!(op, FloatOp::Add | FloatOp::Mul);

        let xmm_of = |this: &Self, opnd: &Operand| -> Option<&'static str> {
            let Operand::Value(v) = opnd else { return None };
            let r = this.reg_assignments.get(&v.0).copied()?;
            is_xmm_reg(r).then(|| phys_reg_name(r))
        };
        let lhs_home = xmm_of(self, lhs);
        let rhs_home = xmm_of(self, rhs);
        let dest_holds_lhs = lhs_home == Some(reg);
        let dest_holds_rhs = rhs_home == Some(reg);

        // VEX 3-operand: `vop src2, src1, dest`  =>  dest = src1 `op` src2
        // (AT&T). Never materialise a constant 0 by xor'ing `reg` after
        // `reg` already holds the other operand (dot-product: mul into
        // xmm2 then `s=0 + p` xorpd'd the product away).
        let emit_vop = |this: &mut Self, src2: &str, src1: &str| {
            this.state.emit_fmt(format_args!(
                "    {}{} {}, %{}, %{}",
                mnemonic, suffix, src2, src1, reg
            ));
        };

        if dest_holds_lhs && dest_holds_rhs {
            emit_vop(self, &format!("%{}", reg), reg);
            self.state.reg_cache.invalidate_acc();
            return true;
        }
        if dest_holds_lhs {
            // dest already has lhs: dest = lhs op rhs.
            self.emit_fp_src2_then_vop(rhs, ty, reg, mnemonic, suffix);
            self.state.reg_cache.invalidate_acc();
            return true;
        }
        if dest_holds_rhs && commutative {
            // dest already has rhs: dest = rhs op lhs == lhs op rhs.
            self.emit_fp_src2_then_vop(lhs, ty, reg, mnemonic, suffix);
            self.state.reg_cache.invalidate_acc();
            return true;
        }
        if dest_holds_rhs && !commutative {
            // dest has rhs, need lhs op rhs. Stage lhs in xmm0 (scratch).
            self.load_fp_to_reg(lhs, ty, "xmm0");
            emit_vop(self, &format!("%{}", reg), "xmm0");
            self.state.reg_cache.invalidate_acc();
            return true;
        }

        // dest holds neither operand. Load lhs into dest, then op with rhs.
        // NOTE: `0.0 + rhs` is NOT folded to `rhs` here: +0.0 + (-0.0) = +0.0
        // per IEEE-754 (observable via signbit), and GCC keeps the add for
        // exactly this reason at -O2. Only -fno-signed-zeros may fold it,
        // which LCCC does not plumb into codegen yet.
        self.load_fp_to_reg(lhs, ty, reg);
        self.emit_fp_src2_then_vop(rhs, ty, reg, mnemonic, suffix);
        self.state.reg_cache.invalidate_acc();
        true
    }

    /// Emit `vop src2, %reg, %reg` where src2 is rhs (mem / xmm / const / scratch).

    /// Constants — including +0.0 — are never written into `reg`.

    fn emit_fp_src2_then_vop(
        &mut self,
        src: &Operand,
        ty: IrType,
        dest: &str,
        mnemonic: &str,
        suffix: &str,
    ) {
        let vop = |this: &mut Self, src2: String| {
            this.state.emit_fmt(format_args!(
                "    {}{} {}, %{}, %{}",
                mnemonic, suffix, src2, dest, dest
            ));
        };
        match src {
            Operand::Value(v) => {
                if let Some(&r) = self.reg_assignments.get(&v.0) {
                    if is_xmm_reg(r) {
                        vop(self, format!("%{}", phys_reg_name(r)));
                        return;
                    }
                }
                if let Some(slot) = self.state.get_slot(v.0) {
                    vop(self, self.slot_ref(slot.0));
                    return;
                }
                // Scratch xmm0: dest is live with lhs; do not reload src into dest.
                self.load_fp_to_reg(src, ty, "xmm0");
                vop(self, "%xmm0".into());
            }
            Operand::Const(IrConst::F64(c)) => {
                let label = self.state.get_fp_const_label(c.to_bits());
                vop(self, format!("{}(%rip)", label));
            }
            Operand::Const(IrConst::F32(c)) => {
                let label = self.state.get_fp_const_label(c.to_bits() as u64);
                vop(self, format!("{}(%rip)", label));
            }
            _ => {
                self.load_fp_to_reg(src, ty, "xmm0");
                vop(self, "%xmm0".into());
            }
        }
    }

    /// Emit a scalar unary FP op (sqrtsd/sqrtss/roundsd/...) honoring an

    /// XMM-allocated destination: compute straight into the dest register

    /// instead of the xmm0 scratch + GPR/store round-trip. Falls back to the

    /// xmm0 path when dest has no XMM home.

    pub(super) fn emit_fp_scalar_unary(
        &mut self,
        dest: &Option<Value>,
        arg: &Operand,
        ty: IrType,
        inst: &str,
    ) {
        if let Some(d) = dest {
            if let Some(&reg) = self.reg_assignments.get(&d.0) {
                if is_xmm_reg(reg) {
                    let dname = phys_reg_name(reg);
                    self.load_fp_to_reg(arg, ty, dname);
                    self.state
                        .emit_fmt(format_args!("    {} %{}, %{}", inst, dname, dname));
                    self.state.reg_cache.invalidate_acc();
                    return;
                }
            }
        }
        self.load_fp_to_xmm0(arg, ty);
        self.state
            .emit_fmt(format_args!("    {} %xmm0, %xmm0", inst));
        if let Some(d) = dest {
            self.store_xmm0_fp_dest(d, ty);
        }
    }

    /// Scalar directed rounding (vroundsd/vroundss imm). VEX 3-operand form

    /// to stay in the VEX domain with the vmul/vadd/vsqrt paths (no

    /// SSE/VEX transition stalls). Computes straight into an XMM-allocated

    /// destination when there is one, mirroring emit_fp_scalar_unary.

    pub(super) fn emit_fp_scalar_round(
        &mut self,
        dest: &Option<Value>,
        arg: &Operand,
        ty: IrType,
        imm: u8,
    ) {
        let inst = if ty == IrType::F32 {
            "vroundss"
        } else {
            "vroundsd"
        };
        if let Some(d) = dest {
            if let Some(&reg) = self.reg_assignments.get(&d.0) {
                if is_xmm_reg(reg) {
                    let dname = phys_reg_name(reg);
                    // Source-direct 3-operand form: when the argument already
                    // lives in an XMM register, read it in place —
                    // `vroundsd $9, %src, %src, %dst` — instead of copying
                    // it into the destination first. Eliminates one movsd
                    // per rounding op; the glibc round-family kernel emits
                    // six of these per iteration (GCC parity).
                    if let Operand::Value(sv) = arg {
                        if let Some(&sreg) = self.reg_assignments.get(&sv.0) {
                            if is_xmm_reg(sreg) && sreg != reg {
                                let sname = phys_reg_name(sreg);
                                self.state.emit_fmt(format_args!(
                                    "    {} ${}, %{}, %{}, %{}",
                                    inst, imm, sname, sname, dname
                                ));
                                self.state.reg_cache.invalidate_acc();
                                return;
                            }
                        }
                    }
                    self.load_fp_to_reg(arg, ty, dname);
                    self.state.emit_fmt(format_args!(
                        "    {} ${}, %{}, %{}, %{}",
                        inst, imm, dname, dname, dname
                    ));
                    self.state.reg_cache.invalidate_acc();
                    return;
                }
            }
        }
        self.load_fp_to_xmm0(arg, ty);
        self.state
            .emit_fmt(format_args!("    {} ${}, %xmm0, %xmm0, %xmm0", inst, imm));
        if let Some(d) = dest {
            self.store_xmm0_fp_dest(d, ty);
        }
    }

    /// copysign(x, y): |x| bits OR'd with y's sign bit — three VEX bit ops

    /// against rodata masks, no branches, no libm call. The libm call is not

    /// merely slow: inside glibc's own s_copysign.c it recurses.

    pub(super) fn emit_fp_copysign(
        &mut self,
        dest: &Option<Value>,
        x: &Operand,
        y: &Operand,
        ty: IrType,
    ) {
        let (andp, orp, abs_mask, sign_mask) = if ty == IrType::F32 {
            ("vandps", "vorps", 0x7FFF_FFFFu64, 0x8000_0000u64)
        } else {
            (
                "vandpd",
                "vorpd",
                0x7FFF_FFFF_FFFF_FFFFu64,
                0x8000_0000_0000_0000u64,
            )
        };
        let abs_label = self.state.get_fp_const_label(abs_mask);
        let sign_label = self.state.get_fp_const_label(sign_mask);
        // Source-direct: masks are non-destructive 3-operand VEX ops, so
        // XMM-homed operands are read in place (no movsd staging copies) —
        // sign bits land in scratch xmm1, magnitude in scratch xmm0.
        let xh = self.fp_arg_xmm_home(x);
        let yh = self.fp_arg_xmm_home(y);
        match yh {
            Some(yname) => self.state.emit_fmt(format_args!(
                "    {} {}(%rip), %{}, %xmm1",
                andp, sign_label, yname
            )),
            None => {
                self.load_fp_to_reg(y, ty, "xmm1");
                self.state.emit_fmt(format_args!(
                    "    {} {}(%rip), %xmm1, %xmm1",
                    andp, sign_label
                ));
            }
        }
        match xh {
            Some(xname) => self.state.emit_fmt(format_args!(
                "    {} {}(%rip), %{}, %xmm0",
                andp, abs_label, xname
            )),
            None => {
                self.load_fp_to_xmm0(x, ty);
                self.state.emit_fmt(format_args!(
                    "    {} {}(%rip), %xmm0, %xmm0",
                    andp, abs_label
                ));
            }
        }
        self.state
            .emit_fmt(format_args!("    {} %xmm1, %xmm0, %xmm0", orp));
        if let Some(d) = dest {
            self.store_xmm0_fp_dest(d, ty);
        }
        self.state.reg_cache.invalidate_acc();
    }

    /// XMM register home of an FP operand, if it has one (excluding the

    /// xmm0/xmm1 scratch pair which the copysign sequence overwrites).

    fn fp_arg_xmm_home(&self, arg: &Operand) -> Option<&'static str> {
        if let Operand::Value(v) = arg {
            if let Some(&reg) = self.reg_assignments.get(&v.0) {
                if is_xmm_reg(reg) {
                    let name = phys_reg_name(reg);
                    if name != "xmm0" && name != "xmm1" {
                        return Some(name);
                    }
                }
            }
        }
        None
    }

    pub(super) fn emit_float_binop_impl(
        &mut self,
        dest: &Value,
        op: FloatOp,
        lhs: &Operand,
        rhs: &Operand,
        ty: IrType,
    ) {
        if ty == IrType::F128 {
            let x87_op = match op {
                FloatOp::Add => "faddp",
                FloatOp::Sub => "fsubrp",
                FloatOp::Mul => "fmulp",
                FloatOp::Div => "fdivrp",
            };
            self.emit_f128_load_to_x87(lhs);
            self.emit_f128_load_to_x87(rhs);
            self.state
                .emit_fmt(format_args!("    {} %st, %st(1)", x87_op));
            if let Some(dest_slot) = self.state.get_slot(dest.0) {
                self.state.out.emit_instr_rbp("    fstpt", dest_slot.0);
                self.state.out.emit_instr_rbp("    fldt", dest_slot.0);
                self.state.emit("    subq $8, %rsp");
                self.state.emit("    fstpl (%rsp)");
                self.state.emit("    popq %rax");
                self.state.reg_cache.set_acc(dest.0, false);
                self.state.f128_direct_slots.insert(dest.0);
            } else {
                self.state.emit("    subq $8, %rsp");
                self.state.emit("    fstpl (%rsp)");
                self.state.emit("    popq %rax");
                self.state.reg_cache.invalidate_acc();
                self.store_rax_to(dest);
            }
            return;
        }
        // compute directly into an XMM-allocated destination register
        // (one reg-reg move per chain link instead of store + reload).
        if let Some(&dreg) = self.reg_assignments.get(&dest.0) {
            if is_xmm_reg(dreg) {
                let dname = phys_reg_name(dreg);
                if self.emit_float_binop_into_reg(dest, op, lhs, rhs, ty, dname) {
                    return;
                }
            }
        }

        let mnemonic = self.emit_float_binop_mnemonic_impl(op);
        let suffix = if ty == IrType::F64 { "sd" } else { "ss" };
        let mov_instr = if ty == IrType::F64 { "movsd" } else { "movss" };

        // Load LHS to %xmm0 (register-allocated XMM home first, then slot,
        // then GPR round-trip for constants / homless values).
        self.load_fp_to_xmm0(lhs, ty);

        // Use a legal RHS memory source when the value has a stack home.
        match rhs {
            Operand::Const(IrConst::F64(v)) => {
                let bits = v.to_bits();
                if bits == 0 {
                    self.state.emit("    xorpd %xmm1, %xmm1");
                } else {
                    let label = self.state.get_fp_const_label(bits);
                    self.state
                        .emit_fmt(format_args!("    movsd {}(%rip), %xmm1", label));
                }
                self.state.emit_fmt(format_args!(
                    "    {}{} %xmm1, %xmm0, %xmm0",
                    mnemonic, suffix
                ));
            }
            Operand::Const(IrConst::F32(v)) => {
                let bits = v.to_bits() as u64;
                if bits == 0 {
                    self.state.emit("    xorps %xmm1, %xmm1");
                } else {
                    let label = self.state.get_fp_const_label(bits);
                    self.state
                        .emit_fmt(format_args!("    movss {}(%rip), %xmm1", label));
                }
                self.state.emit_fmt(format_args!(
                    "    {}{} %xmm1, %xmm0, %xmm0",
                    mnemonic, suffix
                ));
            }
            Operand::Value(v) => {
                // XMM register direct: if RHS is in xmm3-xmm7 (register allocator
                // assigned), operate directly — avoids GPR domain crossing. Safe
                // because xmm3-xmm7 are never clobbered by codegen scratch.
                if let Some(&reg) = self.reg_assignments.get(&v.0) {
                    if is_xmm_reg(reg) {
                        let src_name = phys_reg_name(reg);
                        self.state.emit_fmt(format_args!(
                            "    {}{} %{}, %xmm0, %xmm0",
                            mnemonic, suffix, src_name
                        ));
                        self.store_xmm0_fp_dest(dest, ty);
                        return;
                    }
                }
                if let Some(slot) = self.state.get_slot(v.0) {
                    let sr = self.slot_ref(slot.0);
                    self.state.emit_fmt(format_args!(
                        "    {}{} {}, %xmm0, %xmm0",
                        mnemonic, suffix, sr
                    ));
                } else {
                    self.operand_to_rcx(rhs);
                    if ty == IrType::F32 {
                        self.state.emit("    movd %ecx, %xmm1");
                    } else {
                        self.state.emit("    movq %rcx, %xmm1");
                    }
                    self.state.emit_fmt(format_args!(
                        "    {}{} %xmm1, %xmm0, %xmm0",
                        mnemonic, suffix
                    ));
                }
            }
            _ => {
                self.operand_to_rcx(rhs);
                if ty == IrType::F32 {
                    self.state.emit("    movd %ecx, %xmm1");
                } else {
                    self.state.emit("    movq %rcx, %xmm1");
                }
                self.state.emit_fmt(format_args!(
                    "    {}{} %xmm1, %xmm0, %xmm0",
                    mnemonic, suffix
                ));
            }
        }

        // Store the result to the destination (register-allocated XMM home
        // first, then stack slot, then GPR round-trip).
        self.store_xmm0_fp_dest(dest, ty);
    }

    pub(super) fn emit_float_binop_impl_impl(&mut self, _mnemonic: &str, _ty: IrType) {
        unreachable!("x86 emit_float_binop_impl should not be called directly");
    }

    pub(super) fn emit_float_binop_mnemonic_impl(&self, op: FloatOp) -> &'static str {
        match op {
            FloatOp::Add => "vadd",
            FloatOp::Sub => "vsub",
            FloatOp::Mul => "vmul",
            FloatOp::Div => "vdiv",
        }
    }

    pub(super) fn emit_unaryop_impl(
        &mut self,
        dest: &Value,
        op: IrUnaryOp,
        src: &Operand,
        ty: IrType,
    ) {
        if ty == IrType::F128 && op == IrUnaryOp::Neg {
            self.emit_f128_load_to_x87(src);
            self.state.emit("    fchs");
            if let Some(dest_slot) = self.state.get_slot(dest.0) {
                self.state.out.emit_instr_rbp("    fstpt", dest_slot.0);
                self.state.out.emit_instr_rbp("    fldt", dest_slot.0);
                self.state.emit("    subq $8, %rsp");
                self.state.emit("    fstpl (%rsp)");
                self.state.emit("    popq %rax");
                self.state.reg_cache.set_acc(dest.0, false);
                self.state.f128_direct_slots.insert(dest.0);
            } else {
                self.state.emit("    subq $8, %rsp");
                self.state.emit("    fstpl (%rsp)");
                self.state.emit("    popq %rax");
                self.state.reg_cache.invalidate_acc();
                self.store_rax_to(dest);
            }
            return;
        }
        // Register-direct path for integer unary ops when dest has a register.
        if !ty.is_float() && !matches!(ty, IrType::I128 | IrType::U128 | IrType::F128) {
            if let Some(d_reg) = self.dest_reg(dest) {
                if !is_xmm_reg(d_reg) {
                    let use_32bit = matches!(ty, IrType::I32 | IrType::U32);
                    let d_name = if use_32bit {
                        phys_reg_name_32(d_reg)
                    } else {
                        phys_reg_name(d_reg)
                    };
                    let suffix = if use_32bit { "l" } else { "q" };

                    match op {
                        IrUnaryOp::Neg => {
                            self.operand_to_callee_reg(src, d_reg);
                            self.state
                                .emit_fmt(format_args!("    neg{} %{}", suffix, d_name));
                            self.state.reg_cache.invalidate_acc();
                            return;
                        }
                        IrUnaryOp::Not => {
                            self.operand_to_callee_reg(src, d_reg);
                            self.state
                                .emit_fmt(format_args!("    not{} %{}", suffix, d_name));
                            self.state.reg_cache.invalidate_acc();
                            return;
                        }
                        IrUnaryOp::Bswap => {
                            self.operand_to_callee_reg(src, d_reg);
                            if matches!(ty, IrType::I16 | IrType::U16) {
                                // bswap doesn't exist for 16-bit; use rolw with 16-bit register
                                let reg16 = typed_phys_reg_name(d_reg, ty);
                                self.state.emit_fmt(format_args!("    rolw $8, %{}", reg16));
                            } else {
                                let bswap_name = if use_32bit {
                                    phys_reg_name_32(d_reg)
                                } else {
                                    phys_reg_name(d_reg)
                                };
                                let bswap_suffix = if use_32bit { "l" } else { "q" };
                                self.state.emit_fmt(format_args!(
                                    "    bswap{} %{}",
                                    bswap_suffix, bswap_name
                                ));
                            }
                            self.state.reg_cache.invalidate_acc();
                            return;
                        }
                        IrUnaryOp::Popcount => {
                            // popcnt is two-operand: popcnt %src, %dest
                            if let Some(s_reg) = self.operand_reg(src) {
                                if !is_xmm_reg(s_reg) {
                                    let s_name = if use_32bit {
                                        phys_reg_name_32(s_reg)
                                    } else {
                                        phys_reg_name(s_reg)
                                    };
                                    self.state.emit_fmt(format_args!(
                                        "    popcnt{} %{}, %{}",
                                        suffix, s_name, d_name
                                    ));
                                    self.state.reg_cache.invalidate_acc();
                                    return;
                                }
                            }
                            self.operand_to_callee_reg(src, d_reg);
                            self.state.emit_fmt(format_args!(
                                "    popcnt{} %{}, %{}",
                                suffix, d_name, d_name
                            ));
                            self.state.reg_cache.invalidate_acc();
                            return;
                        }
                        _ => {} // Clz, Ctz — complex multi-instruction, fall through to default
                    }
                }
            }
        }

        crate::backend::traits::emit_unaryop_default(self, dest, op, src, ty);
    }

    /// Load an FP operand directly into an XMM register, using the constant pool

    /// for FP literal constants instead of going through a GPR. Value operands

    /// allocated to an XMM register or a stack slot are loaded directly with

    /// `movsd`/`movss`; the GPR path is only a last-resort fallback.

    pub(super) fn emit_fp_operand_to_xmm(&mut self, op: &Operand, ty: IrType, xmm: &str) {
        match op {
            Operand::Const(IrConst::F64(v)) => {
                let bits = v.to_bits();
                if bits == 0 {
                    self.state
                        .emit_fmt(format_args!("    xorpd %{}, %{}", xmm, xmm));
                } else {
                    let label = self.state.get_fp_const_label(bits);
                    self.state
                        .emit_fmt(format_args!("    movsd {}(%rip), %{}", label, xmm));
                }
            }
            Operand::Const(IrConst::F32(v)) => {
                let bits = v.to_bits() as u64;
                if bits == 0 {
                    self.state
                        .emit_fmt(format_args!("    xorps %{}, %{}", xmm, xmm));
                } else {
                    let label = self.state.get_fp_const_label(bits);
                    self.state
                        .emit_fmt(format_args!("    movss {}(%rip), %{}", label, xmm));
                }
            }
            Operand::Value(v) => {
                let is_alloca = self.state.is_alloca(v.0);
                // REGISTER ASSIGNMENT first: a value assigned to an XMM reg
                // lives there (store_rax_to honors the assignment), so move it
                // directly — never through the GPR shuttle. A value assigned
                // to a GPR is moved to the target XMM from that GPR. Without
                // this, an FP compare's second operand read GARBAGE %rcx
                // (frexp_volatile_global: inf != inf was true at -O0).
                if let Some(&reg) = self.reg_assignments.get(&v.0) {
                    if is_xmm_reg(reg) {
                        let name = phys_reg_name(reg);
                        if name != xmm {
                            // Use the type-appropriate scalar move (movsd for
                            // F64, movss for F32). `movaps` here was a domain-
                            // crossing bug: on scalar FP values it is 1) a
                            // partial-register dependency on the old content
                            // of `xmm` (since movaps writes the full register
                            // but downstream code only consumes the low 64/32
                            // bits, creating false dependencies) and 2) can
                            // trigger SSE/AVX transition stalls on non-VEX
                            // paths. Scalar moves zero-extend the high lanes
                            // exactly like GCC/Clang/ICX produce.
                            let mv = if ty == IrType::F32 { "movss" } else { "movsd" };
                            self.state
                                .emit_fmt(format_args!("    {} %{}, %{}", mv, name, xmm));
                        }
                    } else {
                        let gpr = phys_reg_name(reg);
                        let gpr32 = phys_reg_name_32(reg);
                        if ty == IrType::F32 {
                            self.state
                                .emit_fmt(format_args!("    movd %{}, %{}", gpr32, xmm));
                        } else {
                            self.state
                                .emit_fmt(format_args!("    movq %{}, %{}", gpr, xmm));
                        }
                    }
                    return;
                }
                // Stack slot path: load directly from memory to XMM (no GPR intermediate).
                // This is always safe because stack slots are stable locations.
                if let Some(slot) = self.state.get_slot(v.0) {
                    let mov = if ty == IrType::F32 { "movss" } else { "movsd" };
                    let sr = self.slot_ref(slot.0);
                    self.state
                        .emit_fmt(format_args!("    {} {}, %{}", mov, sr, xmm));
                    return;
                }
                // Reg cache check for accumulator. The acc cache means the value
                // is in %rax (NOT %rcx regardless of the target xmm): reading
                // %rcx for an xmm1 target would load garbage. Fix: always
                // source the accumulator.
                if self.state.reg_cache.acc_has(v.0, is_alloca) {
                    if ty == IrType::F32 {
                        self.state.emit_fmt(format_args!("    movd %eax, %{}", xmm));
                    } else {
                        self.state.emit_fmt(format_args!("    movq %rax, %{}", xmm));
                    }
                    return;
                }
                // Fallback: GPR → XMM path (handles any other case not covered
                // above — loads through the target's staging register).
                let gpr = if xmm == "xmm0" { "rax" } else { "rcx" };
                let gpr32 = if xmm == "xmm0" { "eax" } else { "ecx" };
                if gpr == "rax" {
                    self.operand_to_rax(op);
                } else {
                    self.operand_to_rcx(op);
                }
                if ty == IrType::F32 {
                    self.state
                        .emit_fmt(format_args!("    movd %{}, %{}", gpr32, xmm));
                } else {
                    self.state
                        .emit_fmt(format_args!("    movq %{}, %{}", gpr, xmm));
                }
            }
            _ => {
                // Other operand kinds (e.g. Const::I64 for bitcast): GPR → XMM
                let gpr = if xmm == "xmm0" { "rax" } else { "rcx" };
                let gpr32 = if xmm == "xmm0" { "eax" } else { "ecx" };
                if gpr == "rax" {
                    self.operand_to_rax(op);
                } else {
                    self.operand_to_rcx(op);
                }
                if ty == IrType::F32 {
                    self.state
                        .emit_fmt(format_args!("    movd %{}, %{}", gpr32, xmm));
                } else {
                    self.state
                        .emit_fmt(format_args!("    movq %{}, %{}", gpr, xmm));
                }
            }
        }
    }

    /// Stage an FP store value in an XMM register: returns the value's own

    /// XMM register when it has one, otherwise loads it into %xmm0 and

    /// returns "xmm0". Never clobbers %rcx (safe after address setup).

    pub(super) fn fp_store_value_xmm(&mut self, val: &Operand, ty: IrType) -> &'static str {
        if let Operand::Value(v) = val {
            if let Some(&reg) = self.reg_assignments.get(&v.0) {
                if is_xmm_reg(reg) {
                    return phys_reg_name(reg);
                }
            }
        }
        self.emit_fp_operand_to_xmm(val, ty, "xmm0");
        "xmm0"
    }

    /// `add_dest = acc + mul_lhs * mul_rhs` via vfmadd231sd/ss.

    /// AT&T: vfmadd231sd src2, src1, dest  with dest holding acc.

    pub(super) fn emit_scalar_fma231(
        &mut self,
        mul_lhs: &Operand,
        mul_rhs: &Operand,
        acc: &Operand,
        add_dest: &Value,
        ty: IrType,
    ) {
        let fma = if matches!(ty, IrType::F64) {
            "vfmadd231sd"
        } else {
            "vfmadd231ss"
        };
        self.emit_scalar_fma231_with(fma, mul_lhs, mul_rhs, acc, add_dest, ty);
    }

    /// Fused multiply-subtract via FMA3 (levkropp e3b21b8f, audited port).

    /// FMA3 231-form semantics: dest = (src2 * src1) OP dest, dest preloaded

    /// with `acc`:

    ///   mul_is_lhs == true  (product - acc): vfmsub231  = z*y - x

    ///   mul_is_lhs == false (acc - product): vfnmadd231 = -(z*y) + x

    pub(super) fn emit_scalar_fms231(
        &mut self,
        mul_lhs: &Operand,
        mul_rhs: &Operand,
        acc: &Operand,
        sub_dest: &Value,
        ty: IrType,
        mul_is_lhs: bool,
    ) {
        let fma = match (mul_is_lhs, matches!(ty, IrType::F64)) {
            (true, true) => "vfmsub231sd",
            (true, false) => "vfmsub231ss",
            (false, true) => "vfnmadd231sd",
            (false, false) => "vfnmadd231ss",
        };
        self.emit_scalar_fma231_with(fma, mul_lhs, mul_rhs, acc, sub_dest, ty);
    }

    fn emit_scalar_fma231_with(
        &mut self,
        fma: &str,
        mul_lhs: &Operand,
        mul_rhs: &Operand,
        acc: &Operand,
        add_dest: &Value,
        ty: IrType,
    ) {
        // Destructive-FMA coalescing: when linear scan assigned the result to
        // the accumulator's existing XMM home, compute there directly instead
        // of acc->xmm0, lhs->xmm1, FMA, xmm0->dest. This is the dominant shape
        // in dot products and short FP reductions. It is safe only when neither
        // multiply source aliases the destructive destination; otherwise keep
        // the scratch path below.
        let xmm_home = |op: &Operand| -> Option<&'static str> {
            let Operand::Value(v) = op else { return None };
            let reg = self.reg_assignments.get(&v.0).copied()?;
            is_xmm_reg(reg).then(|| phys_reg_name(reg))
        };
        if let Some(dest_reg) = self.dest_reg(add_dest).filter(|r| is_xmm_reg(*r)) {
            let dest_name = phys_reg_name(dest_reg);
            if xmm_home(acc) == Some(dest_name) {
                if let Some(lhs_name) = xmm_home(mul_lhs).filter(|name| *name != dest_name) {
                    match mul_rhs {
                        Operand::Value(v) => {
                            if let Some(rhs_name) =
                                xmm_home(mul_rhs).filter(|name| *name != dest_name)
                            {
                                self.state.emit_fmt(format_args!(
                                    "    {} %{}, %{}, %{}",
                                    fma, rhs_name, lhs_name, dest_name
                                ));
                                self.state.reg_cache.invalidate_acc();
                                return;
                            }
                            if let Some(slot) = self.state.get_slot(v.0) {
                                let sr = self.slot_ref(slot.0);
                                self.state.emit_fmt(format_args!(
                                    "    {} {}, %{}, %{}",
                                    fma, sr, lhs_name, dest_name
                                ));
                                self.state.reg_cache.invalidate_acc();
                                return;
                            }
                        }
                        _ => {}
                    }
                }
            }
            // General XMM-homed destination: preload the accumulator into
            // the destination register (one movsd — or zero when the
            // accumulator is slot-resident) and run the destructive
            // 231-form in place. Replaces the acc->xmm0 / lhs->xmm1 /
            // FMA / xmm0->dest scratch dance (4 insns -> <=2). Aliasing
            // guards: both multiply sources must be readable from homes or
            // slots DISTINCT from the destination, and the accumulator
            // preload must not clobber a source (dest_name is excluded as
            // either source's home below).
            if xmm_home(acc) != Some(dest_name) {
                if let Some(lhs_name) = xmm_home(mul_lhs).filter(|name| *name != dest_name) {
                    let rhs_src: Option<String> = match mul_rhs {
                        Operand::Value(v) => match xmm_home(mul_rhs) {
                            Some(n) if n != dest_name => Some(format!("%{}", n)),
                            Some(_) => None,
                            None => self.state.get_slot(v.0).map(|slot| self.slot_ref(slot.0)),
                        },
                        // FP constant: RIP-relative memory operand, exactly
                        // what GCC feeds vfmadd231sd for literal multipliers.
                        Operand::Const(IrConst::F64(c)) => {
                            let label = self.state.get_fp_const_label(c.to_bits());
                            Some(format!("{}(%rip)", label))
                        }
                        _ => None,
                    };
                    if let Some(rhs) = rhs_src {
                        self.load_fp_to_reg(acc, ty, dest_name);
                        self.state.emit_fmt(format_args!(
                            "    {} {}, %{}, %{}",
                            fma, rhs, lhs_name, dest_name
                        ));
                        self.state.reg_cache.invalidate_acc();
                        return;
                    }
                }
            }
        }

        self.load_fp_to_xmm0(acc, ty);
        self.load_fp_to_reg(mul_lhs, ty, "xmm1");
        match mul_rhs {
            Operand::Value(v) => {
                if let Some(&reg) = self.reg_assignments.get(&v.0) {
                    if is_xmm_reg(reg) {
                        let n = phys_reg_name(reg);
                        // reg form: src2=n, src1=xmm1, dest=xmm0
                        self.state
                            .emit_fmt(format_args!("    {} %{}, %xmm1, %xmm0", fma, n));
                        self.store_xmm0_fp_dest(add_dest, ty);
                        return;
                    }
                }
                if let Some(slot) = self.state.get_slot(v.0) {
                    let sr = self.slot_ref(slot.0);
                    // mem form: encoder wants (mem, vvvv, dst) = src2, src1, dest
                    self.state
                        .emit_fmt(format_args!("    {} {}, %xmm1, %xmm0", fma, sr));
                    self.store_xmm0_fp_dest(add_dest, ty);
                    return;
                }
                self.load_fp_to_reg(mul_rhs, ty, "xmm2");
                self.state
                    .emit_fmt(format_args!("    {} %xmm2, %xmm1, %xmm0", fma));
            }
            Operand::Const(IrConst::F64(v)) => {
                let bits = v.to_bits();
                if bits != 0 {
                    let label = self.state.get_fp_const_label(bits);
                    self.state
                        .emit_fmt(format_args!("    {} {}(%rip), %xmm1, %xmm0", fma, label));
                } else {
                    // +0.0 multiplier: the FMA must still execute. Skipping
                    // it silently returned `acc` unchanged, which is wrong
                    // for x*0.0 with x = +/-Inf or NaN (product is NaN, so
                    // acc+NaN = NaN) and for the -0.0 sum rules
                    // (-0.0 + 0.0 = +0.0, not -0.0). Materialise +0.0 in
                    // xmm2 (VEX xor keeps the unified AVX domain) and fuse.
                    self.state.emit("    vxorpd %xmm2, %xmm2, %xmm2");
                    self.state
                        .emit_fmt(format_args!("    {} %xmm2, %xmm1, %xmm0", fma));
                }
            }
            Operand::Const(IrConst::F32(v)) => {
                let bits = v.to_bits() as u64;
                if bits != 0 {
                    let label = self.state.get_fp_const_label(bits);
                    self.state
                        .emit_fmt(format_args!("    {} {}(%rip), %xmm1, %xmm0", fma, label));
                } else {
                    // Same +0.0 rule as the F64 arm above.
                    self.state.emit("    vxorps %xmm2, %xmm2, %xmm2");
                    self.state
                        .emit_fmt(format_args!("    {} %xmm2, %xmm1, %xmm0", fma));
                }
            }
            _ => {
                self.load_fp_to_reg(mul_rhs, ty, "xmm2");
                self.state
                    .emit_fmt(format_args!("    {} %xmm2, %xmm1, %xmm0", fma));
            }
        }
        self.store_xmm0_fp_dest(add_dest, ty);
    }
}
