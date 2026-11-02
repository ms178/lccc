//! X86Codegen: floating-point binary operations and F128 negation.

use super::emit::X86Codegen;

use super::emit::{is_xmm_reg, phys_reg_name, phys_reg_name_32, typed_phys_reg_name};

use crate::backend::cast::FloatOp;

use crate::common::types::IrType;

use crate::ir::reexports::{IrConst, IrUnaryOp, Operand, Value};

/// Which bit-count instruction a `BitCountMnem` call site is emitting.
///
/// This is an enum, not a `&str`, on purpose: the false-dependency break below
/// dispatches on it, and a string match with a `_ =>` arm silently gives any
/// *new* mnemonic whatever the fallback row says.  With an enum the compiler
/// rejects an unhandled case, and adding an instruction to the tree means
/// choosing its dependency row explicitly.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum BitCountMnem {
    Popcnt,
    Lzcnt,
    Tzcnt,
}

impl BitCountMnem {
    fn text(self) -> &'static str {
        match self {
            BitCountMnem::Popcnt => "popcnt",
            BitCountMnem::Lzcnt => "lzcnt",
            BitCountMnem::Tzcnt => "tzcnt",
        }
    }
}

impl X86Codegen {
    /// Load an F32/F64 operand into %xmm0, honoring a register-allocated XMM

    /// home first. The F64 register allocator uses xmm3-xmm7 only (never

    /// clobbered by codegen scratch), and an allocated value has no stack slot,

    /// so the register IS the value's only home — a `movsd %xmmN, %xmm0` is the

    /// whole load. Falls back to a direct slot load, then to the GPR round-trip

    /// for constants and values without a home.

    /// Move a 64-bit *bit container* (an I64/U64 IR value that the psABI
    /// carries in the low half of `%xmm0` — an 8-byte vector return) into
    /// `%xmm0` without routing it through the accumulator.
    ///
    /// The value is integer-typed in the IR, so it normally sits in a GP
    /// register or in a stack slot, not in an FP home; the FP loader cannot be
    /// reused because it has no GP-register case. Every path here is a single
    /// `movq`: a GP bounce (`movq %rax, %xmm0`) is only needed when the value is
    /// in the accumulator already, and even then the direct move from the GP
    /// register wins.
    pub(super) fn load_bits64_to_xmm0(&mut self, op: &Operand) {
        if !self.isa.simd {
            self.load_bits64_to_xmm0_sse1(op);
            return;
        }
        if let Operand::Value(v) = op {
            let is_alloca = self.state.is_alloca(v.0);
            if self.state.acc_has_verified(v.0, is_alloca) {
                self.state.emit("    movq %rax, %xmm0");
                return;
            }
            // Freshness-gated home, the same resolution `operand_to_rax` uses:
            // a stale assignment must fall through to the slot path.
            if let Some(reg) = self.fresh_home_of(v.0) {
                let name = phys_reg_name(reg);
                if name != "xmm0" {
                    self.state
                        .emit_fmt(format_args!("    movq %{}, %xmm0", name));
                }
                return;
            }
            if let Some(slot) = self.state.get_slot(v.0) {
                self.state
                    .out
                    .emit_instr_rbp_reg("    movq", slot.0, "xmm0");
                return;
            }
        }
        // Constants and anything else: materialize in %rax and move across.
        self.operand_to_rax(op);
        self.state.emit("    movq %rax, %xmm0");
    }

    /// `load_bits64_to_xmm0` without SSE2 (`-mno-sse2 -msse`, an x86-64
    /// 8-byte vector or SSE-class struct result). SSE1 has no MOVQ between a
    /// GPR or memory and an XMM register: a slot loads with MOVLPS (the upper
    /// lanes are not part of the eightbyte), an XMM home copies with MOVAPS,
    /// and a GPR value crosses through the balanced stack bridge.
    fn load_bits64_to_xmm0_sse1(&mut self, op: &Operand) {
        if let Operand::Value(v) = op {
            let is_alloca = self.state.is_alloca(v.0);
            if self.state.acc_has_verified(v.0, is_alloca) {
                self.emit_scalar_gpr_to_xmm("rax", "eax", "xmm0", false);
                return;
            }
            if let Some(reg) = self.fresh_home_of(v.0) {
                let name = phys_reg_name(reg);
                if is_xmm_reg(reg) {
                    if name != "xmm0" {
                        self.state
                            .emit_fmt(format_args!("    movaps %{}, %xmm0", name));
                    }
                } else {
                    self.emit_scalar_gpr_to_xmm(name, phys_reg_name_32(reg), "xmm0", false);
                }
                return;
            }
            if let Some(slot) = self.state.get_slot(v.0) {
                self.state
                    .out
                    .emit_instr_rbp_reg("    movlps", slot.0, "xmm0");
                return;
            }
        }
        self.operand_to_rax(op);
        self.emit_scalar_gpr_to_xmm("rax", "eax", "xmm0", false);
    }

    pub(super) fn load_fp_to_xmm0(&mut self, op: &Operand, ty: IrType) {
        // Decimal carriers are bit containers: D64 moves exactly like a
        // 64-bit double (movsd), D32 like a 32-bit float (movss).
        let mov_instr = if matches!(ty, IrType::F64 | IrType::D64) {
            "movsd"
        } else {
            "movss"
        };
        if let Operand::Value(v) = op {
            if let Some(&reg) = self.reg_assignments.get(&v.0) {
                if is_xmm_reg(reg) {
                    let name = phys_reg_name(reg);
                    if name != "xmm0" {
                        // Full-register copy (as `store_xmm_to`): the
                        // reg-reg `movsd`/`movss` merge into %xmm0's old
                        // upper bits, a false dependency move elimination
                        // cannot remove; the scalar's upper lanes are dead.
                        let copy = if mov_instr == "movsd" {
                            "movapd"
                        } else {
                            "movaps"
                        };
                        // VEX spelling under AVX: no legacy-SSE write in VEX code.
                        let v = if self.isa.avx { "v" } else { "" };
                        self.state
                            .emit_fmt(format_args!("    {v}{copy} %{name}, %xmm0"));
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
            Operand::Const(IrConst::D64(v)) => {
                if *v == 0 {
                    self.state.emit("    xorpd %xmm0, %xmm0");
                } else {
                    let label = self.state.get_fp_const_label(*v);
                    self.state
                        .emit_fmt(format_args!("    movsd {}(%rip), %xmm0", label));
                }
            }
            Operand::Const(IrConst::D32(v)) => {
                if *v == 0 {
                    self.state.emit("    xorps %xmm0, %xmm0");
                } else {
                    let label = self.state.get_fp_const_label(*v as u64);
                    self.state
                        .emit_fmt(format_args!("    movss {}(%rip), %xmm0", label));
                }
            }
            _ => {
                self.operand_to_rax(op);
                self.emit_scalar_gpr_to_xmm(
                    "rax",
                    "eax",
                    "xmm0",
                    matches!(ty, IrType::F32 | IrType::D32),
                );
            }
        }
    }

    /// Store %xmm0 to an F32/F64 destination, honoring a register-allocated XMM

    /// home first (mirrors load_fp_to_xmm0; the result stays in the XMM domain).

    pub(super) fn store_xmm0_fp_dest(&mut self, dest: &Value, ty: IrType) {
        let mov_instr = if matches!(ty, IrType::F64 | IrType::D64) {
            "movsd"
        } else {
            "movss"
        };
        if let Some(&reg) = self.reg_assignments.get(&dest.0) {
            if is_xmm_reg(reg) {
                let name = phys_reg_name(reg);
                if name != "xmm0" {
                    // Full-register copy: a merging `movsd %xmm0, %home`
                    // would read the home's previous value -- in a loop the
                    // previous iteration's -- serialising iterations.
                    let copy = if mov_instr == "movsd" {
                        "movapd"
                    } else {
                        "movaps"
                    };
                    let v = if self.isa.avx { "v" } else { "" };
                    self.state
                        .emit_fmt(format_args!("    {v}{copy} %xmm0, %{name}"));
                }
                self.state.reg_cache.invalidate_acc();
                // Definition-writes-home accounting: the store wrote the
                // dest's XMM home with its value.
                self.note_inplace_compute(reg, dest.0);
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
        let f32ish = matches!(ty, IrType::F32 | IrType::D32);
        self.emit_scalar_xmm_to_gpr("xmm0", "rax", f32ish.then_some("eax"));
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
                        // Register-to-register scalar FP copies use the
                        // full-register packed move (`vmovapd`/`vmovaps`): the
                        // legacy `movsd %src, %dst` is a MERGING move that
                        // preserves the destination's upper 64 bits, i.e. it
                        // READS the destination's previous content. That false
                        // dependence chains onto whatever last wrote the
                        // destination — in loops, typically the previous
                        // iteration's FP result — and serialises otherwise
                        // independent iterations at full FP latency (nbody's
                        // sqrt: 3.1x vs GCC purely from this). The VEX form
                        // reads only the source. Scalar consumers only read
                        // the low lane, so the upper-bit difference is
                        // unobservable; the vectorizer's packed pairs never
                        // route through this scalar helper. The 3-operand
                        // `vmovsd %src, %src, %dst` also reads only the
                        // source, but cores do not reliably eliminate it at
                        // rename (the 2026-09-06 mandelbrot A/B recorded at
                        // `MachInst::FMov`: ~7% in the float loop), while
                        // `vmovapd` is GCC's spelling and is eliminated.
                        let movap = if ty == IrType::F64 {
                            "movapd"
                        } else {
                            "movaps"
                        };
                        let v = if self.isa.avx { "v" } else { "" };
                        self.state
                            .emit_fmt(format_args!("    {v}{movap} %{name}, %{reg}"));
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
                self.emit_scalar_gpr_to_xmm(gpr, gpr32, reg, ty == IrType::F32);
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
            this.emit_fp_op3(mnemonic, suffix, src2, src1, reg, commutative);
        };

        if dest_holds_lhs && dest_holds_rhs {
            emit_vop(self, &format!("%{}", reg), reg);
            self.state.reg_cache.invalidate_acc();
            return true;
        }
        if dest_holds_lhs {
            // dest already has lhs: dest = lhs op rhs.
            self.emit_fp_src2_then_vop(rhs, ty, reg, mnemonic, suffix, commutative);
            self.state.reg_cache.invalidate_acc();
            return true;
        }
        if dest_holds_rhs && commutative {
            // dest already has rhs: dest = rhs op lhs == lhs op rhs.
            self.emit_fp_src2_then_vop(lhs, ty, reg, mnemonic, suffix, commutative);
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

        // dest holds neither operand. The VEX 3-operand form
        // `vop src2, src1, dest` can take src1 and src2 from ANY XMM
        // register distinct from dest, so when both operands are XMM-homed
        // we emit the operation directly — no `vmovsd` staging copy into
        // dest first. This is the dominant case in FP-heavy inner loops
        // (mandelbrot's `zr*zr`, `zi*zi`, `tr*tr`, `zi*zi`): each squared
        // term dropped a redundant `vmovsd %x,%x,%tmp` before the `vmulsd`.
        // NOTE: `emit_vop`'s format string prepends `%` to src1 and dest
        // itself, so src1 is passed as the bare register name (no `%`).
        if let (Some(l), Some(r)) = (lhs_home, rhs_home) {
            emit_vop(self, &format!("%{}", r), l);
            self.state.reg_cache.invalidate_acc();
            return true;
        }
        // dest holds neither operand, and at least one operand is not
        // XMM-homed (memory/const). Prefer the XMM-homed operand as src1
        // so the 3-operand form folds the memory/const source directly:
        //   vop <mem/const src2>, %xmm_src1, %dest   (one instruction)
        // For non-commutative ops (sub/div), src1 MUST be lhs, so only the
        // lhs-homed case applies; the rhs-homed commutative case swaps.
        if let Some(l) = lhs_home {
            // src1 = lhs (in a register); src2 = rhs via the mem/const folder.
            self.emit_fp_src2_then_vop_with_src1(rhs, ty, reg, mnemonic, suffix, l, commutative);
            self.state.reg_cache.invalidate_acc();
            return true;
        }
        if commutative {
            if let Some(r) = rhs_home {
                // commutative: dest = rhs op lhs == lhs op rhs.
                // src1 = rhs (register), src2 = lhs (mem/const).
                self.emit_fp_src2_then_vop_with_src1(
                    lhs,
                    ty,
                    reg,
                    mnemonic,
                    suffix,
                    r,
                    commutative,
                );
                self.state.reg_cache.invalidate_acc();
                return true;
            }
        }

        // dest holds neither operand and neither is XMM-homed (both memory
        // or constants). Load lhs into dest, then op with rhs.
        // SAME-VALUE SHAPE: `x * x` / `x + x` / `x - x` / `x / x` with x
        // slot-homed reads the slot TWICE — once staged into dest, once as
        // the memory source. The staged register already holds the value;
        // both operands read it (operand order is irrelevant when both are
        // the same value, so non-commutative sub/div are equally exact).
        // The nbody d² chain's `movsd -152(%rbp),%xmm5; vmulsd
        // -152(%rbp),%xmm5,%xmm5` becomes one load + one register mul.
        if let (Operand::Value(l), Operand::Value(r)) = (lhs, rhs) {
            if l.0 == r.0 {
                self.load_fp_to_reg(lhs, ty, reg);
                emit_vop(self, &format!("%{}", reg), reg);
                self.state.reg_cache.invalidate_acc();
                return true;
            }
        }
        // NOTE: `0.0 + rhs` is NOT folded to `rhs` here: +0.0 + (-0.0) = +0.0
        // per IEEE-754 (observable via signbit), and GCC keeps the add for
        // exactly this reason at -O2. Only -fno-signed-zeros may fold it,
        // which LCCC does not plumb into codegen yet.
        self.load_fp_to_reg(lhs, ty, reg);
        self.emit_fp_src2_then_vop(rhs, ty, reg, mnemonic, suffix, commutative);
        self.state.reg_cache.invalidate_acc();
        true
    }

    /// Emit `vop src2, src1, dest` where src1 is an EXPLICIT named XMM
    /// register (not necessarily dest), and src2 is the rhs/lhs operand
    /// resolved via the memory/const/register folder. At most one memory
    /// operand is legal per x86 instruction; if the explicit src1 is a
    /// register, a memory src2 is always sound. The operand folder falls
    /// back to xmm0 only when src2 is a second memory/const source AND no
    /// XMM home exists — the caller guarantees src1 is register-homed.
    fn emit_fp_src2_then_vop_with_src1(
        &mut self,
        src: &Operand,
        ty: IrType,
        dest: &str,
        mnemonic: &str,
        suffix: &str,
        src1: &'static str,
        commutative: bool,
    ) {
        let vop = |this: &mut Self, src2: String| {
            this.emit_fp_op3(mnemonic, suffix, &src2, src1, dest, commutative);
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
                // No XMM home and no slot: load into xmm0 (scratch).
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

    /// Emit `vop src2, %reg, %reg` where src2 is rhs (mem / xmm / const / scratch).

    /// Constants — including +0.0 — are never written into `reg`.

    fn emit_fp_src2_then_vop(
        &mut self,
        src: &Operand,
        ty: IrType,
        dest: &str,
        mnemonic: &str,
        suffix: &str,
        commutative: bool,
    ) {
        let vop = |this: &mut Self, src2: String| {
            this.emit_fp_op3(mnemonic, suffix, &src2, dest, dest, commutative);
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
        // `inst` is the VEX spelling (`vsqrtsd`). Legacy scalar sqrt merges
        // the destination's old upper lanes, even when its low-lane source
        // is a different register. For distinct XMM homes, zero the unused
        // upper lanes independently before consuming the scalar source.
        // Other sources retain the full-register copy fallback; an aliased
        // source must not be zeroed. Scalar IR observes only the low lane.
        // Keep the VEX path below
        // source-direct: its explicit upper-lane source already avoids this.
        if !self.isa.avx {
            let legacy = inst.strip_prefix('v').unwrap_or(inst);
            if let Some(d) = dest {
                if let Some(&reg) = self.reg_assignments.get(&d.0) {
                    if is_xmm_reg(reg) {
                        let dname = phys_reg_name(reg);
                        // A zero idiom makes the unused upper lanes independent
                        // of both the old destination and the source's upper
                        // lanes. Never zero an aliased scalar input. XORPS is
                        // SSE1; using PXOR here would incorrectly require SSE2
                        // for F32 on an SSE1-only target.
                        if let Operand::Value(source) = arg {
                            if let Some(&sreg) = self.reg_assignments.get(&source.0) {
                                if is_xmm_reg(sreg) && sreg != reg {
                                    let zero = if ty == IrType::F32 { "xorps" } else { "xorpd" };
                                    self.state.emit_fmt(format_args!(
                                        "    {} %{}, %{}",
                                        zero, dname, dname
                                    ));
                                    self.state.emit_fmt(format_args!(
                                        "    {} %{}, %{}",
                                        legacy,
                                        phys_reg_name(sreg),
                                        dname
                                    ));
                                    self.state.reg_cache.invalidate_acc();
                                    return;
                                }
                            }
                        }
                        self.load_fp_to_reg(arg, ty, dname);
                        self.state
                            .emit_fmt(format_args!("    {} %{}, %{}", legacy, dname, dname));
                        self.state.reg_cache.invalidate_acc();
                        return;
                    }
                }
            }
            self.load_fp_to_xmm0(arg, ty);
            self.state
                .emit_fmt(format_args!("    {} %xmm0, %xmm0", legacy));
            if let Some(d) = dest {
                self.store_xmm0_fp_dest(d, ty);
            }
            return;
        }
        if let Some(d) = dest {
            if let Some(&reg) = self.reg_assignments.get(&d.0) {
                if is_xmm_reg(reg) {
                    let dname = phys_reg_name(reg);
                    // Source-direct 3-operand form: when the argument already
                    // lives in a different XMM register, compute straight from
                    // it — `vsqrtsd %src, %src, %dst`. The old copy-first form
                    // (`load_fp_to_reg` → legacy `movsd %src, %dst`; then
                    // `vsqrtsd %dst, %dst`) cost more than one instruction:
                    // legacy movsd is a MERGING move that preserves the
                    // destination's upper 64 bits, so it carries a WAR/WAW
                    // dependence on the destination's previous content. In a
                    // loop where the destination held the previous iteration's
                    // sqrt result, every iteration's sqrt serialized on the
                    // last one at full latency instead of pipelining in the
                    // divider (nbody: 668 ms → near-GCC-scalar once the false
                    // loop-carried dependency is gone). Same shape as the
                    // vroundsd source-direct path below.
                    if let Operand::Value(sv) = arg {
                        if let Some(&sreg) = self.reg_assignments.get(&sv.0) {
                            if is_xmm_reg(sreg) && sreg != reg {
                                let sname = phys_reg_name(sreg);
                                self.state.emit_fmt(format_args!(
                                    "    {} %{}, %{}, %{}",
                                    inst, sname, sname, dname
                                ));
                                self.state.reg_cache.invalidate_acc();
                                return;
                            }
                        }
                    }
                    self.load_fp_to_reg(arg, ty, dname);
                    // VEX scalar unaries are three-operand only: the middle
                    // operand supplies the upper lanes.  The old two-operand
                    // spelling `vsqrtsd %d, %d` is not valid AT&T syntax —
                    // GAS 2.44 rejects it ("number of operands mismatch") and
                    // the builtin assembler silently encoded it as
                    // `vsqrtsd %d, %xmm0, %d`, i.e. with a spurious upper-lane
                    // dependency on %xmm0.
                    self.state.emit_fmt(format_args!(
                        "    {} %{}, %{}, %{}",
                        inst, dname, dname, dname
                    ));
                    self.state.reg_cache.invalidate_acc();
                    return;
                }
            }
        }
        self.load_fp_to_xmm0(arg, ty);
        self.state
            .emit_fmt(format_args!("    {} %xmm0, %xmm0, %xmm0", inst));
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
        // The simplify fold that produces RoundScalarF* is gated on
        // `set_has_round_insn` (SSE4.1), so reaching here without SSE4.1 is
        // a pipeline bug, not a user-visible condition.
        debug_assert!(
            self.isa.sse41,
            "RoundScalar intrinsic reached the x86 emitter without SSE4.1"
        );
        if !self.isa.avx {
            // Legacy SSE4.1 `roundsd $imm, %src, %dst` (two operands + imm8).
            let legacy = if ty == IrType::F32 {
                "roundss"
            } else {
                "roundsd"
            };
            if let Some(d) = dest {
                if let Some(&reg) = self.reg_assignments.get(&d.0) {
                    if is_xmm_reg(reg) {
                        let dname = phys_reg_name(reg);
                        if let Operand::Value(sv) = arg {
                            if let Some(&sreg) = self.reg_assignments.get(&sv.0) {
                                if is_xmm_reg(sreg) && sreg != reg {
                                    self.state.emit_fmt(format_args!(
                                        "    {} ${}, %{}, %{}",
                                        legacy,
                                        imm,
                                        phys_reg_name(sreg),
                                        dname
                                    ));
                                    self.state.reg_cache.invalidate_acc();
                                    return;
                                }
                            }
                        }
                        self.load_fp_to_reg(arg, ty, dname);
                        self.state.emit_fmt(format_args!(
                            "    {} ${}, %{}, %{}",
                            legacy, imm, dname, dname
                        ));
                        self.state.reg_cache.invalidate_acc();
                        return;
                    }
                }
            }
            self.load_fp_to_xmm0(arg, ty);
            self.state
                .emit_fmt(format_args!("    {} ${}, %xmm0, %xmm0", legacy, imm));
            if let Some(d) = dest {
                self.store_xmm0_fp_dest(d, ty);
            }
            return;
        }
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
        if !self.isa.avx {
            // Legacy SSE: the bit ops are destructive two-operand forms, so
            // both inputs are staged into the xmm0/xmm1 scratch pair first
            // (movsd/movss from an XMM home is a single merging move).  The
            // pool constants are `.p2align 4` (emit_fp_const_pool), as the
            // aligned memory operand of `andpd m128` requires.
            let (andl, orl) = (&andp[1..], &orp[1..]);
            self.load_fp_to_reg(y, ty, "xmm1");
            self.state
                .emit_fmt(format_args!("    {} {}(%rip), %xmm1", andl, sign_label));
            self.load_fp_to_xmm0(x, ty);
            self.state
                .emit_fmt(format_args!("    {} {}(%rip), %xmm0", andl, abs_label));
            self.state
                .emit_fmt(format_args!("    {} %xmm1, %xmm0", orl));
            if let Some(d) = dest {
                self.store_xmm0_fp_dest(d, ty);
            }
            self.state.reg_cache.invalidate_acc();
            return;
        }
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
                self.state.park_acc(dest.0, false);
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
        let commutative = matches!(op, FloatOp::Add | FloatOp::Mul);

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
                self.emit_fp_op3(mnemonic, suffix, "%xmm1", "xmm0", "xmm0", commutative);
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
                self.emit_fp_op3(mnemonic, suffix, "%xmm1", "xmm0", "xmm0", commutative);
            }
            Operand::Value(v) => {
                // XMM register direct: if RHS is in xmm3-xmm7 (register allocator
                // assigned), operate directly — avoids GPR domain crossing. Safe
                // because xmm3-xmm7 are never clobbered by codegen scratch.
                if let Some(&reg) = self.reg_assignments.get(&v.0) {
                    if is_xmm_reg(reg) {
                        let src_name = phys_reg_name(reg);
                        self.emit_fp_op3(
                            mnemonic,
                            suffix,
                            &format!("%{}", src_name),
                            "xmm0",
                            "xmm0",
                            commutative,
                        );
                        self.store_xmm0_fp_dest(dest, ty);
                        return;
                    }
                }
                if let Some(slot) = self.state.get_slot(v.0) {
                    let sr = self.slot_ref(slot.0);
                    self.emit_fp_op3(mnemonic, suffix, &sr, "xmm0", "xmm0", commutative);
                } else {
                    self.operand_to_rcx(rhs);
                    if ty == IrType::F32 {
                        self.state.emit("    movd %ecx, %xmm1");
                    } else {
                        self.state.emit("    movq %rcx, %xmm1");
                    }
                    self.emit_fp_op3(mnemonic, suffix, "%xmm1", "xmm0", "xmm0", commutative);
                }
            }
            _ => {
                self.operand_to_rcx(rhs);
                if ty == IrType::F32 {
                    self.state.emit("    movd %ecx, %xmm1");
                } else {
                    self.state.emit("    movq %rcx, %xmm1");
                }
                self.emit_fp_op3(mnemonic, suffix, "%xmm1", "xmm0", "xmm0", commutative);
            }
        }

        // Store the result to the destination (register-allocated XMM home
        // first, then stack slot, then GPR round-trip).
        self.store_xmm0_fp_dest(dest, ty);
    }

    pub(super) fn emit_float_binop_impl_impl(&mut self, _mnemonic: &str, _ty: IrType) {
        unreachable!("x86 emit_float_binop_impl should not be called directly");
    }

    /// Scalar FP arithmetic mnemonic stem for the current ISA: `vadd` (VEX
    /// 3-operand) under AVX, `add` (legacy SSE 2-operand) otherwise.  Every
    /// emission goes through [`Self::emit_fp_op3`], which turns the
    /// 3-operand shape into a legal legacy sequence when needed.
    pub(super) fn emit_float_binop_mnemonic_impl(&self, op: FloatOp) -> &'static str {
        match (op, self.isa.avx) {
            (FloatOp::Add, true) => "vadd",
            (FloatOp::Sub, true) => "vsub",
            (FloatOp::Mul, true) => "vmul",
            (FloatOp::Div, true) => "vdiv",
            (FloatOp::Add, false) => "add",
            (FloatOp::Sub, false) => "sub",
            (FloatOp::Mul, false) => "mul",
            (FloatOp::Div, false) => "div",
        }
    }

    /// Emit `dest = src1 OP src2` for a scalar FP op.
    ///
    /// * AVX: the non-destructive `v{op}{suffix} src2, %src1, %dest`.
    /// * Legacy SSE (`-mno-avx`, `-march=x86-64`, kernel): the two-operand
    ///   `{op}{suffix} src2, %dest` is destructive, so
    ///   - `src1 == dest` → one instruction;
    ///   - `src2 == %dest` and OP commutative → `{op} %src1, %dest`;
    ///   - `src2 == %dest`, non-commutative → compute in scratch `%xmm0`
    ///     (`movap? %src1, %xmm0; {op} %dest, %xmm0; movap? %xmm0, %dest`);
    ///     `dest` is an allocator register (xmm2+) so it never aliases the
    ///     scratch — asserted below;
    ///   - otherwise `movap? %src1, %dest; {op} src2, %dest`.  The copy is
    ///     a full-width `movaps`/`movapd` (GCC's choice): unlike the merging
    ///     `movsd` it does not read `dest`'s previous contents, so no false
    ///     loop-carried dependence is introduced; the upper lanes hold
    ///     garbage, which scalar consumers never read.
    ///
    /// `src2` is a complete AT&T operand (`%xmm3`, `-8(%rbp)`, `.LC0(%rip)`);
    /// `src1` and `dest` are bare register names.
    pub(super) fn emit_fp_op3(
        &mut self,
        mnemonic: &str,
        suffix: &str,
        src2: &str,
        src1: &str,
        dest: &str,
        commutative: bool,
    ) {
        if self.isa.avx {
            self.state.emit_fmt(format_args!(
                "    {}{} {}, %{}, %{}",
                mnemonic, suffix, src2, src1, dest
            ));
            return;
        }
        let legacy = mnemonic.strip_prefix('v').unwrap_or(mnemonic);
        let movap = if suffix == "ss" { "movaps" } else { "movapd" };
        let src2_is_dest = src2.strip_prefix('%') == Some(dest);
        if src1 == dest {
            self.state
                .emit_fmt(format_args!("    {}{} {}, %{}", legacy, suffix, src2, dest));
        } else if src2_is_dest && commutative {
            self.state.emit_fmt(format_args!(
                "    {}{} %{}, %{}",
                legacy, suffix, src1, dest
            ));
        } else if src2_is_dest {
            debug_assert!(dest != "xmm0" && src1 != "xmm0");
            self.state
                .emit_fmt(format_args!("    {} %{}, %xmm0", movap, src1));
            self.state
                .emit_fmt(format_args!("    {}{} %{}, %xmm0", legacy, suffix, dest));
            self.state
                .emit_fmt(format_args!("    {} %xmm0, %{}", movap, dest));
        } else {
            self.state
                .emit_fmt(format_args!("    {} %{}, %{}", movap, src1, dest));
            self.state
                .emit_fmt(format_args!("    {}{} {}, %{}", legacy, suffix, src2, dest));
        }
    }

    /// Dest-only ABM/BMI: `popcnt`/`lzcnt`/`tzcnt %src, %dest`.
    ///
    /// Intel (pre Ice Lake) has a false dependency on the destination of these
    /// instructions; GCC breaks it with `xorl %dest32, %dest32` whenever dest
    /// is distinct from src. We do the same. When dest has no GPR home the
    /// result is written to `%eax`/`%rax` so the return path does not emit a
    /// staging `movq %src, %rax` that the peephole then fails to delete.
    fn emit_bitcount_direct(
        &mut self,
        dest: &Value,
        src: &Operand,
        ty: IrType,
        mnem: BitCountMnem,
    ) -> bool {
        let use_32bit = matches!(ty, IrType::I32 | IrType::U32);
        let suffix = if use_32bit { "l" } else { "q" };
        let src_gpr = self.operand_reg(src).filter(|r| !is_xmm_reg(*r));
        let dest_gpr = self.dest_reg(dest).filter(|r| !is_xmm_reg(*r));

        let src_name: String = if let Some(s_reg) = src_gpr {
            if use_32bit {
                phys_reg_name_32(s_reg).to_string()
            } else {
                phys_reg_name(s_reg).to_string()
            }
        } else if let Some(d_reg) = dest_gpr {
            self.operand_to_callee_reg(src, d_reg);
            if use_32bit {
                phys_reg_name_32(d_reg).to_string()
            } else {
                phys_reg_name(d_reg).to_string()
            }
        } else {
            return false;
        };

        let (dst_name, dst32): (&str, &str) = if let Some(d_reg) = dest_gpr {
            if use_32bit {
                (phys_reg_name_32(d_reg), phys_reg_name_32(d_reg))
            } else {
                (phys_reg_name(d_reg), phys_reg_name_32(d_reg))
            }
        } else if use_32bit {
            ("eax", "eax")
        } else {
            ("rax", "eax")
        };

        // False-dependency break, only where the tuning row says the core
        // architecturally reads the destination (uops.info lat 1->1 = 3:
        // POPCNT on SNB..CLX, LZCNT/TZCNT on SNB..BDW) and only when dest is
        // a different register from src (xor would otherwise destroy the
        // input).  Ice Lake+, Alder/Raptor Lake and every Zen skip it: there
        // the xor is a pure front-end/code-size cost.  GCC 16.2 still emits
        // it for tzcnt on Skylake, where uops.info measures no dependency.
        let needs_break = match mnem {
            BitCountMnem::Popcnt => self.tune.break_popcnt_dep(),
            BitCountMnem::Lzcnt => self.tune.break_lzcnt_dep(),
            BitCountMnem::Tzcnt => self.tune.break_tzcnt_dep(),
        };
        if needs_break && src_name != dst_name && src_name != dst32 {
            self.state
                .emit_fmt(format_args!("    xorl %{0}, %{0}", dst32));
        }
        self.state.emit_fmt(format_args!(
            "    {}{} %{}, %{}",
            mnem.text(),
            suffix,
            src_name,
            dst_name
        ));
        self.state.reg_cache.invalidate_acc();
        if dest_gpr.is_none() {
            self.store_rax_to(dest);
        }
        true
    }

    /// Baseline BSR/BSF direct form for a source that is explicit nonzero IR.
    /// Both instructions are two-operand and do not require staging through
    /// `%rax`; keeping the allocator's source and destination registers saves
    /// a move in hot bit-walking loops.
    fn emit_nonzero_bitcount_direct(
        &mut self,
        dest: &Value,
        src: &Operand,
        ty: IrType,
        leading: bool,
    ) -> bool {
        let use_32bit = matches!(ty, IrType::I32 | IrType::U32);
        if !use_32bit && !matches!(ty, IrType::I64 | IrType::U64) {
            return false;
        }
        let Some(s_reg) = self.operand_reg(src).filter(|r| !is_xmm_reg(*r)) else {
            return false;
        };
        let dest_gpr = self.dest_reg(dest).filter(|r| !is_xmm_reg(*r));
        let (s_name, d_name, suffix) = if use_32bit {
            (
                phys_reg_name_32(s_reg),
                dest_gpr.map(phys_reg_name_32).unwrap_or("eax"),
                "l",
            )
        } else {
            (
                phys_reg_name(s_reg),
                dest_gpr.map(phys_reg_name).unwrap_or("rax"),
                "q",
            )
        };
        self.state.emit_fmt(format_args!(
            "    {}{} %{}, %{}",
            if leading { "bsr" } else { "bsf" },
            suffix,
            s_name,
            d_name
        ));
        if leading {
            self.state.emit_fmt(format_args!(
                "    xor{} ${}, %{}",
                suffix,
                if use_32bit { 31 } else { 63 },
                d_name
            ));
        }
        self.state.reg_cache.invalidate_acc();
        if dest_gpr.is_none() {
            self.store_rax_to(dest);
        }
        true
    }

    /// Zero-defined CLZ/CTZ without LZCNT/TZCNT and without a branch.
    ///
    /// BSR/BSF leave their destination unmodified when the source is zero:
    /// AMD has always documented this and Intel's SDM now does too (LLVM
    /// relies on it for x86-64 codegen since llvm/llvm-project#123623). So
    /// the zero result is PRELOADED and the scan keeps it: CTZ preloads the
    /// width; CLZ preloads `2*width - 1`, which the closing `xor width-1`
    /// turns into the width exactly like it turns an MSB index into a count.
    /// The IR's `Clz(0) == Ctz(0) == width` contract is preserved without the
    /// `test; jnz; mov $w; jmp` fix-up (3 executed instructions and a branch
    /// per count in zstd's `ZSTD_highbit32` price loops).
    ///
    /// The preload is a 32-bit `movl`, so the whole 64-bit destination holds
    /// the preload; the SDM's footnote that some older CPUs clear bits 63:32
    /// on a 32-bit zero-source scan is therefore moot. The source must be a
    /// GPR home distinct from the destination (the preload writes the
    /// destination before the scan reads the source); otherwise the caller
    /// keeps the accumulator forms. x86-64 only: i686 output may run on
    /// pre-x86-64 processors, where the passthrough is not guaranteed.
    fn emit_zero_defined_scan_direct(
        &mut self,
        dest: &Value,
        src: &Operand,
        ty: IrType,
        leading: bool,
    ) -> bool {
        let use_32bit = matches!(ty, IrType::I32 | IrType::U32);
        if !use_32bit && !matches!(ty, IrType::I64 | IrType::U64) {
            return false;
        }
        let Some(s_reg) = self.operand_reg(src).filter(|r| !is_xmm_reg(*r)) else {
            return false;
        };
        let dest_gpr = self
            .dest_reg(dest)
            .filter(|r| !is_xmm_reg(*r) && *r != s_reg);
        let bits: u32 = if use_32bit { 32 } else { 64 };
        let preload = if leading { 2 * bits - 1 } else { bits };
        let d32 = dest_gpr.map(phys_reg_name_32).unwrap_or("eax");
        let (s_name, d_name, suffix) = if use_32bit {
            (phys_reg_name_32(s_reg), d32, "l")
        } else {
            (
                phys_reg_name(s_reg),
                dest_gpr.map(phys_reg_name).unwrap_or("rax"),
                "q",
            )
        };
        self.state
            .emit_fmt(format_args!("    movl ${}, %{}", preload, d32));
        self.state.emit_fmt(format_args!(
            "    {}{} %{}, %{}",
            if leading { "bsr" } else { "bsf" },
            suffix,
            s_name,
            d_name
        ));
        if leading {
            self.state
                .emit_fmt(format_args!("    xor{} ${}, %{}", suffix, bits - 1, d_name));
        }
        self.state.reg_cache.invalidate_acc();
        if dest_gpr.is_none() {
            self.store_rax_to(dest);
        }
        true
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
                self.state.park_acc(dest.0, false);
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
        // Scalar FP negate: one sign-bit flip in the SSE domain.  The default
        // path moves the value to %rax, materialises the sign mask in %rcx,
        // xors, and moves back -- five instructions and two domain crossings
        // for what GCC, Clang and ICX all emit as a single `vxorpd` against a
        // rodata mask, which is what `fabs` already does here with `andpd`.
        // Computing into the value's XMM home keeps it out of the GPR
        // shuttle that the FMA emitters then cannot see through: `-(a*b) - c`
        // stayed a multiply, a five-instruction negate and a subtract while
        // the oracles fused it into one `vfnmsub`.
        if matches!(ty, IrType::F32 | IrType::F64) && op == IrUnaryOp::Neg {
            if self.emit_fp_neg_direct(dest, src, ty) {
                return;
            }
        }
        // Without the counting instruction (LZCNT for Clz, BMI1 TZCNT for
        // Ctz) a proven-nonzero count is a bare BSR/BSF.
        let leading = match op {
            IrUnaryOp::ClzNonZero if !self.lzcnt_enabled => Some(true),
            IrUnaryOp::CtzNonZero if !self.bmi1_enabled => Some(false),
            _ => None,
        };
        if let Some(leading) = leading
            && self.emit_nonzero_bitcount_direct(dest, src, ty, leading)
        {
            return;
        }
        let zero_defined = match op {
            IrUnaryOp::Clz if !self.lzcnt_enabled => Some(true),
            IrUnaryOp::Ctz if !self.bmi1_enabled => Some(false),
            _ => None,
        };
        if let Some(leading) = zero_defined
            && self.emit_zero_defined_scan_direct(dest, src, ty, leading)
        {
            return;
        }

        // Integer dest-only two-operand ABM/BMI: popcnt/lzcnt/tzcnt write dest
        // without reading it. Emit `mnem %src, %dest` (or `%src, %eax` when dest
        // is the accumulator / return) so we never stage through `movq %src, %rax`
        // then `mnem %eax, %eax`. Sub-32-bit types still take the default path
        // (they need an explicit zero-extend first).
        if !ty.is_float() && !matches!(ty, IrType::I128 | IrType::U128 | IrType::F128) {
            if matches!(ty, IrType::I32 | IrType::U32 | IrType::I64 | IrType::U64) {
                let bitcount = match op {
                    IrUnaryOp::Popcount if self.popcnt_enabled => Some(BitCountMnem::Popcnt),
                    IrUnaryOp::Clz | IrUnaryOp::ClzNonZero if self.lzcnt_enabled => {
                        Some(BitCountMnem::Lzcnt)
                    }
                    IrUnaryOp::Ctz | IrUnaryOp::CtzNonZero if self.bmi1_enabled => {
                        Some(BitCountMnem::Tzcnt)
                    }
                    _ => None,
                };
                if let Some(mnem) = bitcount {
                    if self.emit_bitcount_direct(dest, src, ty, mnem) {
                        return;
                    }
                }
            }
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
                        _ => {} // Popcount without -mpopcnt / Clz without -mlzcnt / Ctz without -mbmi
                                // fall through to the baseline sequences.
                    }
                }
            }
        }

        crate::backend::traits::emit_unaryop_default(self, dest, op, src, ty);
    }

    /// `dest = -src` for F32/F64 as a sign-bit xor in the SSE domain, into the
    /// destination's XMM home or, failing one, through the `%xmm0` scratch and
    /// [`Self::store_xmm0_fp_dest`].  Always emits; the `bool` is the
    /// caller's "handled" convention.
    ///
    /// The mask is the same 16-byte-aligned rodata slot the `fabs` emitter uses
    /// for its `andpd`, with the sign bit set instead of cleared; the pad
    /// qword is zero, so the xor leaves the upper lane of a scalar-typed
    /// register as it was, exactly as `andpd`'s pad does for `fabs`.  With AVX
    /// the three-operand `vxorpd mask(%rip), %src, %dst` reads the source in
    /// place and writes the destination without a copy; without it, the
    /// destructive legacy form needs the value in the destination first.
    fn emit_fp_neg_direct(&mut self, dest: &Value, src: &Operand, ty: IrType) -> bool {
        let (mask, xor) = if ty == IrType::F32 {
            (0x8000_0000u64, "xorps")
        } else {
            (0x8000_0000_0000_0000u64, "xorpd")
        };
        let src_home = match src {
            Operand::Value(v) => self
                .reg_assignments
                .get(&v.0)
                .copied()
                .filter(|r| is_xmm_reg(*r))
                .map(phys_reg_name),
            _ => None,
        };
        // The destination's home, or the `%xmm0` scratch when it has none
        // (a slot-homed value, typically because it lives across a call):
        // the xor still runs in the SSE domain and the scalar store to the
        // slot is one instruction, where the accumulator path is a GPR round
        // trip plus the same store.
        let (dname, d_reg) = match self.dest_reg(dest).filter(|r| is_xmm_reg(*r)) {
            Some(r) => (phys_reg_name(r), Some(r)),
            None => ("xmm0", None),
        };
        let label = self.state.get_fp_const_label(mask);
        if self.isa.avx {
            let sname = match src_home {
                Some(name) => name,
                None => {
                    self.load_fp_to_reg(src, ty, dname);
                    dname
                }
            };
            self.state.emit_fmt(format_args!(
                "    v{} {}(%rip), %{}, %{}",
                xor, label, sname, dname
            ));
        } else {
            self.load_fp_to_reg(src, ty, dname);
            self.state
                .emit_fmt(format_args!("    {} {}(%rip), %{}", xor, label, dname));
        }
        self.state.reg_cache.invalidate_acc();
        match d_reg {
            Some(r) => self.note_inplace_compute(r, dest.0),
            None => self.store_xmm0_fp_dest(dest, ty),
        }
        true
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
            // Decimal BID bit patterns: opaque bits through the same
            // constant-pool / GPR-shuttle machinery as F64/F32.
            Operand::Const(IrConst::D64(v)) => {
                if *v == 0 {
                    self.state
                        .emit_fmt(format_args!("    xorpd %{}, %{}", xmm, xmm));
                } else {
                    let label = self.state.get_fp_const_label(*v);
                    self.state
                        .emit_fmt(format_args!("    movsd {}(%rip), %{}", label, xmm));
                }
            }
            Operand::Const(IrConst::D32(v)) => {
                if *v == 0 {
                    self.state
                        .emit_fmt(format_args!("    xorps %{}, %{}", xmm, xmm));
                } else {
                    let label = self.state.get_fp_const_label(*v as u64);
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
                            // Full-register packed move (`vmovapd`/`movapd`;
                            // `movaps` for F32): writes the destination from
                            // the SOURCE alone. The legacy 2-operand `movsd
                            // %src, %dst` only zero-extends from MEMORY;
                            // register-to-register it MERGES, reading the
                            // destination's previous content -- a false
                            // dependence on whatever last wrote it (in loops,
                            // the previous iteration's FP result). The VEX
                            // `vmovsd %src, %src, %dst` has the same low-128
                            // result bit for bit but is a port-5 shuffle uop,
                            // while the packed move is eliminated at rename
                            // (Intel since Ivy Bridge, AMD Zen) -- GCC's
                            // choice. Scalar consumers ignore the upper lanes.
                            let mv = match (ty == IrType::F32, self.isa.avx) {
                                (true, true) => "vmovaps",
                                (true, false) => "movaps",
                                (false, true) => "vmovapd",
                                (false, false) => "movapd",
                            };
                            self.state
                                .emit_fmt(format_args!("    {mv} %{name}, %{xmm}"));
                        }
                    } else {
                        let gpr = phys_reg_name(reg);
                        let gpr32 = phys_reg_name_32(reg);
                        self.emit_scalar_gpr_to_xmm(gpr, gpr32, xmm, ty == IrType::F32);
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
                if self.state.acc_has_verified(v.0, is_alloca) {
                    self.emit_scalar_gpr_to_xmm("rax", "eax", xmm, ty == IrType::F32);
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
                self.emit_scalar_gpr_to_xmm(gpr, gpr32, xmm, ty == IrType::F32);
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
                self.emit_scalar_gpr_to_xmm(gpr, gpr32, xmm, ty == IrType::F32);
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
        // `acc - product` negates the product; `product - acc` negates the
        // addend.  Both flags false is the plain fmadd.
        self.emit_scalar_fma_signed(mul_lhs, mul_rhs, acc, sub_dest, ty, !mul_is_lhs, mul_is_lhs);
    }

    /// Any of the four signed FMA3 families in one rounding:
    /// `dest = ±(lhs*rhs) ± acc`, the signs chosen by the two flags.  The
    /// x86 naming composes exactly: `n` negates the PRODUCT and `sub`
    /// subtracts the ADDEND, so
    /// `(false,false) vfmadd · (false,true) vfmsub · (true,false) vfnmadd ·
    /// (true,true) vfnmsub` — the complete sign algebra behind the source
    /// shapes `-(a*b) - c`, `-(a*b) + c`, `-(a*b + c)` and `-(c - a*b)`,
    /// which GCC contracts to one instruction each under
    /// `-ffp-contract=fast` (oracle-verified; the same algebra covers
    /// AArch64 with the `sub`/`n` roles swapped — there `sub` negates the
    /// product and `n` the addend: fmadd·fnmsub·fmsub·fnmadd).
    pub(super) fn emit_scalar_fma_signed(
        &mut self,
        mul_lhs: &Operand,
        mul_rhs: &Operand,
        acc: &Operand,
        dest: &Value,
        ty: IrType,
        negate_product: bool,
        negate_addend: bool,
    ) {
        let fma = crate::backend::x86_common::fma231_mnemonic(
            negate_product,
            negate_addend,
            matches!(ty, IrType::F64),
        );
        self.emit_scalar_fma231_with(fma, mul_lhs, mul_rhs, acc, dest, ty);
    }

    /// The four-way signed FMA hook (`emit_fused_fma` on the trait): any
    /// negated family routes here; the plain family keeps its own path.
    pub(super) fn emit_fused_fma_impl(
        &mut self,
        mul_lhs: &Operand,
        mul_rhs: &Operand,
        acc: &Operand,
        dest: &Value,
        ty: IrType,
        negate_product: bool,
        negate_addend: bool,
    ) {
        debug_assert!(matches!(ty, IrType::F32 | IrType::F64));
        debug_assert!(negate_product || negate_addend);
        self.emit_scalar_fma_signed(
            mul_lhs,
            mul_rhs,
            acc,
            dest,
            ty,
            negate_product,
            negate_addend,
        );
    }

    /// Multiply sources for an in-place 231-form FMA into `dest_name`:
    /// `src1` must be a register, `src2` may be a register, a slot or a
    /// RIP-relative constant (one memory operand per instruction). Neither
    /// may read `dest_name`, which holds (or receives) the accumulator.
    ///
    /// A register-homed factor is `src1` directly. With only the other
    /// factor homed, the factors are swapped -- IEEE multiplication is
    /// exactly commutative, so the fused result is bit-identical. With
    /// neither homed, `lhs` is staged in the scratch %xmm1 and `rhs` rides
    /// as the memory operand (or reads %xmm1 again for `x*x`). Before the
    /// staging arm, a slot-resident factor sent the whole FMA through the
    /// 4-instruction acc->xmm0 / lhs->xmm1 / fma / xmm0->dest scratch path
    /// (struct_copy's `dy*dy + acc`: `vmovsd %xmm3,%xmm0; movsd N(%rsp),
    /// %xmm1; vfmadd231sd %xmm1,%xmm1,%xmm0; vmovsd %xmm0,%xmm3`).
    fn fma_mul_sources(
        &mut self,
        mul_lhs: &Operand,
        mul_rhs: &Operand,
        dest_name: &str,
    ) -> Option<(FmaSrc1, String)> {
        let (lh, rh) = (self.fma_xmm_home(mul_lhs), self.fma_xmm_home(mul_rhs));
        if lh == Some(dest_name) || rh == Some(dest_name) {
            return None; // a factor lives in dest: path 2b's 132-form shape
        }
        // Decide the shape with the pure feasibility test first, so a
        // declined shape never interns an unused pool constant.
        match (lh, rh) {
            (Some(l), Some(r)) => Some((FmaSrc1::Home(l), format!("%{}", r))),
            (Some(l), None) => {
                let m = self.fma_mem(mul_rhs)?;
                Some((FmaSrc1::Home(l), self.fma_mem_text(m)))
            }
            (None, Some(r)) => {
                let m = self.fma_mem(mul_lhs)?;
                Some((FmaSrc1::Home(r), self.fma_mem_text(m)))
            }
            (None, None) => {
                self.fma_mem(mul_lhs)?;
                let same = matches!((mul_lhs, mul_rhs),
                    (Operand::Value(a), Operand::Value(b)) if a.0 == b.0);
                let src2 = if same {
                    "%xmm1".to_string()
                } else {
                    let m = self.fma_mem(mul_rhs)?;
                    self.fma_mem_text(m)
                };
                Some((FmaSrc1::Stage(mul_lhs.clone()), src2))
            }
        }
    }

    /// The operand's XMM home, if it still holds the operand's bits
    /// (`fresh_home_of`: a home already overwritten by an in-place
    /// definition is not a readable source -- the same guard path 2b uses).
    fn fma_xmm_home(&self, op: &Operand) -> Option<&'static str> {
        let Operand::Value(v) = op else { return None };
        let reg = self.fresh_home_of(v.0)?;
        is_xmm_reg(reg).then(|| phys_reg_name(reg))
    }

    /// `op` as a legal FMA memory source that is not a register home: a
    /// slot-resident value or an F32/F64 pool constant. Pure (interns no
    /// constant), so a declined shape leaves the pool untouched.
    fn fma_mem(&self, op: &Operand) -> Option<FmaMem> {
        match op {
            Operand::Value(v) if !self.reg_assignments.contains_key(&v.0) => {
                self.state.get_slot(v.0).map(|slot| FmaMem::Slot(slot.0))
            }
            Operand::Const(IrConst::F64(c)) => Some(FmaMem::Pool(c.to_bits())),
            Operand::Const(IrConst::F32(c)) => Some(FmaMem::Pool(c.to_bits() as u64)),
            _ => None,
        }
    }

    /// The memory operand text of an `fma_mem` source (interns a constant).
    fn fma_mem_text(&mut self, m: FmaMem) -> String {
        match m {
            FmaMem::Slot(off) => self.slot_ref(off),
            FmaMem::Pool(bits) => format!("{}(%rip)", self.state.get_fp_const_label(bits)),
        }
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
        if let Some(dest_reg) = self.dest_reg(add_dest).filter(|r| is_xmm_reg(*r)) {
            let dest_name = phys_reg_name(dest_reg);
            let acc_in_place = self.fma_xmm_home(acc) == Some(dest_name);
            if let Some((src1, src2)) = self.fma_mul_sources(mul_lhs, mul_rhs, dest_name) {
                // In place: the accumulator already lives in the destination
                // (path 1, the dot-product/reduction shape) or is preloaded
                // into it (one movsd, or one slot/constant load). Neither
                // multiply source reads `dest_name` (`fma_mul_sources`), so
                // the preload cannot clobber one.
                if !acc_in_place {
                    self.load_fp_to_reg(acc, ty, dest_name);
                }
                let src1_name = match src1 {
                    FmaSrc1::Home(name) => name,
                    FmaSrc1::Stage(op) => {
                        self.load_fp_to_reg(&op, ty, "xmm1");
                        "xmm1"
                    }
                };
                self.state.emit_fmt(format_args!(
                    "    {} {}, %{}, %{}",
                    fma, src2, src1_name, dest_name
                ));
                self.state.reg_cache.invalidate_acc();
                // Definition-writes-home accounting: the destructive 231-form
                // wrote the fused dest's XMM home (and destroyed the acc's
                // bits there).
                self.note_inplace_compute(dest_reg, add_dest.0);
                return;
            }
        }

        // Path 2b — 132-form: the destination shares its home with one of
        // the MULTIPLY operands (regalloc coalesced the fused result onto a
        // dying multiplicand) while the accumulator does NOT share it. The
        // accumulator then rides the AT&T-second (addend) position of
        // `vfmadd132sd mul_other, addend, dest`, which computes
        // `dest = dest * mul_other + acc` — zero staging when the acc is
        // itself XMM-homed (one pool/slot load through xmm0 otherwise, with
        // no store-back in either case). This is the recurrence shape every
        // FP loop produces (`x = x * k + c`, `zi = zi * 2zr + ci`):
        // mandelbrot's inner loop previously staged `movsd ci,xmm0; fma;
        // movsd xmm0,zi` per iteration and measured 1.51s; the 132-form
        // reaches 1.02s.
        //
        // Aliasing proof: dest holds mul_operand's bits because the
        // allocator coalesced the fma's destination onto that operand's
        // register — the operand's live range ends at this fma, exactly the
        // invariant the 231-coalescing path above relies on for acc. The
        // OTHER multiply operand must be readable from a home or slot
        // distinct from dest_name; the accumulator must be a fresh home
        // distinct from dest_name (it cannot equal dest_name here — that
        // is path 1's shape, which already returned), a pool/slot load
        // through xmm0, or a GPR-shuttle case we decline.
        {
            let dest_xmm = self.dest_reg(add_dest).filter(|r| is_xmm_reg(*r));
            if let Some(dest_reg) = dest_xmm {
                let dest_name = phys_reg_name(dest_reg);
                // The multiply operand homed AT the destination (lhs first,
                // then rhs); `other` is the m64-capable first position.
                let lhs_at_dest = match mul_lhs {
                    Operand::Value(v) => {
                        self.fresh_home_of(v.0).map(phys_reg_name) == Some(dest_name)
                    }
                    _ => false,
                };
                let rhs_at_dest = match mul_rhs {
                    Operand::Value(v) => {
                        self.fresh_home_of(v.0).map(phys_reg_name) == Some(dest_name)
                    }
                    _ => false,
                };
                let other: Option<&Operand> = if lhs_at_dest {
                    Some(mul_rhs)
                } else if rhs_at_dest {
                    Some(mul_lhs)
                } else {
                    None
                };
                if let Some(other_op) = other {
                    // First (AT&T) position: register home, slot, or
                    // RIP-relative constant — all legal m64 sources.
                    let other_src: Option<String> = match other_op {
                        Operand::Value(v) => match self.fresh_home_of(v.0) {
                            Some(reg) if is_xmm_reg(reg) => {
                                let name = phys_reg_name(reg);
                                if name != dest_name {
                                    Some(format!("%{}", name))
                                } else {
                                    None
                                }
                            }
                            _ => self
                                .state
                                .get_slot(v.0)
                                .filter(|_| !self.reg_assignments.contains_key(&v.0))
                                .map(|slot| self.slot_ref(slot.0)),
                        },
                        Operand::Const(IrConst::F64(c)) => Some(format!(
                            "{}(%rip)",
                            self.state.get_fp_const_label(c.to_bits())
                        )),
                        Operand::Const(IrConst::F32(c)) => Some(format!(
                            "{}(%rip)",
                            self.state.get_fp_const_label(c.to_bits() as u64)
                        )),
                        _ => None,
                    };
                    // Second (AT&T) position: the addend must be a register.
                    // acc homed in a distinct XMM → direct; acc const → one
                    // load into xmm0 (never a home — guarded by other_src
                    // being usable so the load is never orphaned); anything
                    // else declines to the scratch path below.
                    if let Some(o_src) = other_src {
                        let addend: Option<String> = match acc {
                            Operand::Value(v) => match self.fresh_home_of(v.0) {
                                Some(reg) if is_xmm_reg(reg) => {
                                    let name = phys_reg_name(reg);
                                    if name != dest_name {
                                        Some(format!("%{}", name))
                                    } else {
                                        None // acc at dest: path 1's shape
                                    }
                                }
                                // A slot-resident addend is one load into
                                // the xmm0 scratch, like a constant addend:
                                // the other factor is a home distinct from
                                // xmm0 or a memory operand, so the load
                                // clobbers no source. Before, this shape
                                // fell to the 4-instruction scratch path.
                                _ if self.fma_mem(acc).is_some() => {
                                    self.load_fp_to_xmm0(acc, ty);
                                    Some("%xmm0".to_string())
                                }
                                _ => None,
                            },
                            Operand::Const(IrConst::F64(_)) | Operand::Const(IrConst::F32(_)) => {
                                self.load_fp_to_xmm0(acc, ty);
                                Some("%xmm0".to_string())
                            }
                            _ => None,
                        };
                        if let Some(a_src) = addend {
                            let fma132 = fma.replace("231", "132");
                            self.state.emit_fmt(format_args!(
                                "    {} {}, {}, %{}",
                                fma132, o_src, a_src, dest_name
                            ));
                            self.state.reg_cache.invalidate_acc();
                            // Definition-writes-home accounting: the 132-form
                            // derived the fused dest in place over the dying
                            // multiplicand occupying that home.
                            self.note_inplace_compute(dest_reg, add_dest.0);
                            return;
                        }
                    }
                }
            }
        }

        // Path 3 — constant accumulator, lhs and destination share an XMM
        // home (the shape path 2 must refuse: its accumulator preload would
        // clobber the lhs sitting in that home).  Use the 213 form instead:
        // dest = src1 * dest + src2, i.e. home = rhs * lhs + acc, with the
        // constant accumulator materialised in the xmm0 scratch.  Two
        // instructions, result lands in the destination home directly — no
        // xmm0 store-back.  Requires the rhs in a distinct XMM home.
        if matches!(
            acc,
            Operand::Const(IrConst::F64(_)) | Operand::Const(IrConst::F32(_))
        ) {
            if let (Some(dest_reg), Operand::Value(rv)) = (self.dest_reg(add_dest), mul_rhs) {
                if is_xmm_reg(dest_reg) {
                    let dest_name = phys_reg_name(dest_reg);
                    let lhs_home = match mul_lhs {
                        Operand::Value(v) => self
                            .reg_assignments
                            .get(&v.0)
                            .copied()
                            .filter(|r| is_xmm_reg(*r))
                            .map(phys_reg_name),
                        _ => None,
                    };
                    let rhs_home = self
                        .reg_assignments
                        .get(&rv.0)
                        .copied()
                        .filter(|r| is_xmm_reg(*r))
                        .map(phys_reg_name);
                    if lhs_home == Some(dest_name) {
                        if let Some(rhs_name) = rhs_home.filter(|n| *n != dest_name) {
                            self.load_fp_to_xmm0(acc, ty);
                            let fma213 = fma.replace("231", "213");
                            // AT&T operand order is reversed: this encodes
                            // dest = src1 * dest + src2, i.e.
                            // home = rhs * lhs + acc (acc in xmm0 = src2,
                            // rhs = src1, lhs lives in the dest home).
                            self.state.emit_fmt(format_args!(
                                "    {} %xmm0, %{}, %{}",
                                fma213, rhs_name, dest_name
                            ));
                            self.state.reg_cache.invalidate_acc();
                            // Definition-writes-home accounting: the 213-form
                            // wrote the fused dest's XMM home.
                            self.note_inplace_compute(dest_reg, add_dest.0);
                            return;
                        }
                    }
                }
            }
        }

        self.load_fp_to_xmm0(acc, ty);
        // src1 for the scratch path: use the lhs's existing XMM home when it
        // has one instead of staging through xmm1 (a `vmovsd home, xmm1` per
        // FMA).  xmm0 is pure scratch — linear scan never assigns it as a
        // home — so a home-sourced src1 cannot alias the FMA destination.
        // One exception: the rhs arms below stage unsupported sources through
        // xmm2, which IS a possible home; keep the xmm1 staging when the lhs
        // lives in xmm2 and such a staging may occur.
        let lhs_home_name: Option<&'static str> = match mul_lhs {
            Operand::Value(v) => self
                .reg_assignments
                .get(&v.0)
                .copied()
                .filter(|r| is_xmm_reg(*r))
                .map(phys_reg_name),
            _ => None,
        };
        let rhs_may_stage_xmm2 = match mul_rhs {
            Operand::Value(v) => {
                let has_home = self
                    .reg_assignments
                    .get(&v.0)
                    .copied()
                    .map(is_xmm_reg)
                    .unwrap_or(false);
                !has_home && self.state.get_slot(v.0).is_none()
            }
            Operand::Const(IrConst::F64(_)) | Operand::Const(IrConst::F32(_)) => false,
            _ => true,
        };
        let lhs_src: String = match lhs_home_name {
            Some(name) if name != "xmm2" || !rhs_may_stage_xmm2 => format!("%{}", name),
            _ => {
                self.load_fp_to_reg(mul_lhs, ty, "xmm1");
                "%xmm1".to_string()
            }
        };
        match mul_rhs {
            Operand::Value(v) => {
                if let Some(&reg) = self.reg_assignments.get(&v.0) {
                    if is_xmm_reg(reg) {
                        let n = phys_reg_name(reg);
                        // reg form: src2=n, src1=xmm1, dest=xmm0
                        self.state
                            .emit_fmt(format_args!("    {} %{}, {}, %xmm0", fma, n, lhs_src));
                        self.store_xmm0_fp_dest(add_dest, ty);
                        return;
                    }
                }
                if let Some(slot) = self.state.get_slot(v.0) {
                    // SAME-VALUE FMA (acc += x*x with x slot-homed): the
                    // staged src1 already holds x — folding the slot for
                    // src2 is the second memory read of one value on the
                    // critical d² chain. Both operands read the register
                    // (same value — operand order is irrelevant).
                    if let Operand::Value(lv) = mul_lhs {
                        if lv.0 == v.0 {
                            self.state.emit_fmt(format_args!(
                                "    {} {}, {}, %xmm0",
                                fma, lhs_src, lhs_src
                            ));
                            self.store_xmm0_fp_dest(add_dest, ty);
                            return;
                        }
                    }
                    let sr = self.slot_ref(slot.0);
                    // mem form: encoder wants (mem, vvvv, dst) = src2, src1, dest
                    self.state
                        .emit_fmt(format_args!("    {} {}, {}, %xmm0", fma, sr, lhs_src));
                    self.store_xmm0_fp_dest(add_dest, ty);
                    return;
                }
                self.load_fp_to_reg(mul_rhs, ty, "xmm2");
                self.state
                    .emit_fmt(format_args!("    {} %xmm2, {}, %xmm0", fma, lhs_src));
            }
            Operand::Const(IrConst::F64(v)) => {
                let bits = v.to_bits();
                if bits != 0 {
                    let label = self.state.get_fp_const_label(bits);
                    self.state.emit_fmt(format_args!(
                        "    {} {}(%rip), {}, %xmm0",
                        fma, label, lhs_src
                    ));
                } else {
                    // +0.0 multiplier: the FMA must still execute. Skipping
                    // it silently returned `acc` unchanged, which is wrong
                    // for x*0.0 with x = +/-Inf or NaN (product is NaN, so
                    // acc+NaN = NaN) and for the -0.0 sum rules
                    // (-0.0 + 0.0 = +0.0, not -0.0). Materialise +0.0 in
                    // xmm2 (VEX xor keeps the unified AVX domain) and fuse.
                    self.state.emit("    vxorpd %xmm2, %xmm2, %xmm2");
                    self.state
                        .emit_fmt(format_args!("    {} %xmm2, {}, %xmm0", fma, lhs_src));
                }
            }
            Operand::Const(IrConst::F32(v)) => {
                let bits = v.to_bits() as u64;
                if bits != 0 {
                    let label = self.state.get_fp_const_label(bits);
                    self.state.emit_fmt(format_args!(
                        "    {} {}(%rip), {}, %xmm0",
                        fma, label, lhs_src
                    ));
                } else {
                    // Same +0.0 rule as the F64 arm above.
                    self.state.emit("    vxorps %xmm2, %xmm2, %xmm2");
                    self.state
                        .emit_fmt(format_args!("    {} %xmm2, {}, %xmm0", fma, lhs_src));
                }
            }
            _ => {
                self.load_fp_to_reg(mul_rhs, ty, "xmm2");
                self.state
                    .emit_fmt(format_args!("    {} %xmm2, {}, %xmm0", fma, lhs_src));
            }
        }
        self.store_xmm0_fp_dest(add_dest, ty);
    }
}

/// A legal FMA memory source that is not a register home (`fma_mem`).
#[derive(Clone, Copy)]
enum FmaMem {
    /// Slot-resident value at this frame offset.
    Slot(i64),
    /// F32/F64 pool constant (bit pattern; F32 bits zero-extended).
    Pool(u64),
}

/// The register source of an in-place 231-form FMA (`fma_mul_sources`).
enum FmaSrc1 {
    /// The factor's own XMM home.
    Home(&'static str),
    /// A slot- or pool-resident factor, loaded into the scratch %xmm1.
    Stage(Operand),
}

#[cfg(test)]
mod scalar_sqrt_dependency_tests {
    use super::*;
    use crate::backend::regalloc::PhysReg;

    #[test]
    fn legacy_sqrt_zero_seed_preserves_input_and_sse1_permission() {
        for (ty, sqrt, zero) in [
            (IrType::F32, "sqrtss", "xorps"),
            (IrType::F64, "sqrtsd", "xorpd"),
        ] {
            for (source, destination) in [(20, 21), (33, 20), (20, 20)] {
                let mut cg = X86Codegen::new();
                cg.isa.avx = false;
                cg.reg_assignments.insert(1, PhysReg(source));
                cg.reg_assignments.insert(2, PhysReg(destination));
                cg.emit_fp_scalar_unary(
                    &Some(Value(2)),
                    &Operand::Value(Value(1)),
                    ty,
                    &format!("v{sqrt}"),
                );
                let text = &cg.state.out.buf;
                let dst = phys_reg_name(PhysReg(destination));
                let src = phys_reg_name(PhysReg(source));
                assert!(text.contains(&format!("{sqrt} %{src}, %{dst}")), "{text}");
                if source != destination {
                    let seed = format!("{zero} %{dst}, %{dst}");
                    assert!(
                        text.find(&seed).unwrap() < text.find(sqrt).unwrap(),
                        "{text}"
                    );
                } else {
                    assert!(!text.contains(zero), "{text}");
                }
                assert!(
                    !text.contains("    movsd %") && !text.contains("    movss %"),
                    "{text}"
                );
            }
        }
    }

    #[test]
    fn vex_sqrt_keeps_its_explicit_source_dependency() {
        for (ty, sqrt) in [(IrType::F32, "vsqrtss"), (IrType::F64, "vsqrtsd")] {
            let mut cg = X86Codegen::new();
            cg.isa.avx = true;
            cg.reg_assignments.insert(1, PhysReg(20));
            cg.reg_assignments.insert(2, PhysReg(21));
            cg.emit_fp_scalar_unary(&Some(Value(2)), &Operand::Value(Value(1)), ty, sqrt);
            let text = &cg.state.out.buf;
            assert!(
                text.contains(&format!("{sqrt} %xmm2, %xmm2, %xmm3")),
                "{text}"
            );
            assert!(!text.contains("movap"), "{text}");
        }
    }
}
