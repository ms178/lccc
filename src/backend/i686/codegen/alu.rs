//! I686Codegen: ALU operations (integer arithmetic, bitwise, shifts).
//!
//! Size-first 32-bit integer isel (the kernel boot code is -Os and bounded by
//! the 32 KiB setup gate, so every pattern is chosen for BOTH size and speed):
//! - Direct-to-dest (no %eax round-trip) when the dest is register-allocated
//! - LEA 3-operand add for reg+reg / reg+imm (shorter than mov+op)
//! - Multiply-by-constant strength reduction for the single-instruction LEA/
//!   shift/add forms ({0,1,-1,2,3,5,9} and *4/*8 in place) — same size as
//!   `imull`, one cycle instead of three
//! - Narrow-width correctness for clz/ctz/popcount/bswap (defensive; C never
//!   reaches them, but they must be right if the IR does)
//! - Shift counts are masked to 5 bits so `x << 300` encodes (GAS interop)
//!
//! Deliberately NOT done here (documented, measured):
//! - Memory-operand folds (`addl mem,%eax`, `addl %reg,mem`): unsound in
//!   codegen — a value's `get_slot` is not guaranteed to hold the value at
//!   emission time (copy-alias/coalesced slots), and the peephole's store
//!   forwarding eliminates the materializing store when the read is folded
//!   early (fuzz seeds 11/34/140/215/236). The peephole folds slots correctly
//!   post-materialization; registers are folded here because they are
//!   authoritative.
//! - Constant division/remainder (magic numbers, signed pow-of-2 bias): every
//!   one of those sequences is LONGER than `idivl`/`divl` at -Os (magic udiv
//!   ~10 insns vs 3; signed /2^k bias 5 insns vs 3). Unsigned pow-of-2 div/rem
//!   is already lowered to shrl/andl in the IR. A speed-only build would want
//!   them behind a size-vs-speed switch.
//! - Long LEA chains for *6/*7/*10/... (two insns vs one `imull`).
//! - ALU identities (+0/|0/^0/&-1/<<0) and commutative-immediate
//!   canonicalisation: the IR already folds/canonicalises these (verified).

use crate::ir::reexports::{IrBinOp, Operand, Value};
use crate::common::types::IrType;
use crate::emit;
use super::emit::{I686Codegen, alu_mnemonic, shift_mnemonic};

/// Location of a 32-bit binop operand for the direct-to-dest path.
enum BinopLoc {
    Reg(&'static str),
    Slot(String),
    Imm(i64),
}

impl I686Codegen {
    pub(super) fn emit_float_neg_impl(&mut self, ty: IrType) {
        // Only F32 reaches this hook: F64/F128 negation is handled by the x87
        // `fchs` path in emit_unaryop before this is called. An f32 lives in
        // %eax as its IEEE-754 bit pattern, whose sign is bit 31 — a single
        // xor flips it. The previous SSE round-trip (movd/movd/xorps/movd plus
        // the two movl copies) was 5+ instructions for the identical result.
        let _ = ty;
        self.state.emit("    xorl $0x80000000, %eax");
    }

    pub(super) fn emit_int_neg_impl(&mut self, _ty: IrType) {
        self.state.emit("    negl %eax");
    }

    pub(super) fn emit_int_not_impl(&mut self, _ty: IrType) {
        self.state.emit("    notl %eax");
    }

    pub(super) fn emit_int_clz_impl(&mut self, ty: IrType) {
        match ty {
            IrType::I8 | IrType::U8 => {
                // clz8(x) = lzcnt32(x & 0xff) - 24. The mask kills dirty high
                // bits so the count is width-correct.
                self.state.emit("    andl $0xff, %eax");
                self.state.emit("    lzcntl %eax, %eax");
                self.state.emit("    subl $24, %eax");
            }
            IrType::I16 | IrType::U16 => {
                self.state.emit("    lzcntw %ax, %ax");
                // lzcntw leaves %eax[31:16] unchanged — zero-extend so the
                // stored 32-bit result is width-correct.
                self.state.emit("    movzwl %ax, %eax");
            }
            _ => {
                self.state.emit("    lzcntl %eax, %eax");
            }
        }
    }

    pub(super) fn emit_int_ctz_impl(&mut self, ty: IrType) {
        match ty {
            IrType::I8 | IrType::U8 => {
                // Force a stop bit above the value so tzcnt(0) yields 8, not 32.
                self.state.emit("    orl $0x100, %eax");
                self.state.emit("    tzcntl %eax, %eax");
            }
            IrType::I16 | IrType::U16 => {
                self.state.emit("    orl $0x10000, %eax");
                self.state.emit("    tzcntl %eax, %eax");
            }
            _ => {
                self.state.emit("    tzcntl %eax, %eax");
            }
        }
    }

    pub(super) fn emit_int_bswap_impl(&mut self, ty: IrType) {
        match ty {
            IrType::I8 | IrType::U8 => {
                // Byte-swap of a single byte is a no-op.
            }
            IrType::I16 | IrType::U16 => {
                self.state.emit("    rolw $8, %ax");
                // rolw leaves %eax[31:16] unchanged (stale sign bits on a
                // sign-extended I16) — clear them for a correct 32-bit result.
                self.state.emit("    movzwl %ax, %eax");
            }
            IrType::I32 | IrType::U32 | IrType::Ptr => self.state.emit("    bswapl %eax"),
            _ => self.state.emit("    bswapl %eax"),
        }
    }

    pub(super) fn emit_int_popcount_impl(&mut self, ty: IrType) {
        match ty {
            IrType::I8 | IrType::U8 => {
                // The value may be sign-extended; mask to the width so the
                // sign bits are not counted.
                self.state.emit("    andl $0xff, %eax");
                self.state.emit("    popcntl %eax, %eax");
            }
            IrType::I16 | IrType::U16 => {
                self.state.emit("    andl $0xffff, %eax");
                self.state.emit("    popcntl %eax, %eax");
            }
            _ => self.state.emit("    popcntl %eax, %eax"),
        }
    }

    /// Operand location for the direct-to-dest ALU path.
    fn binop_loc(&self, op: &Operand) -> Option<BinopLoc> {
        match op {
            Operand::Const(_) => Self::const_as_imm32(op).map(BinopLoc::Imm),
            Operand::Value(v) => {
                if self.state.wide_values.contains(&v.0)
                    || self.state.is_alloca(v.0)
                    || self.state.f128_direct_slots.contains(&v.0)
                {
                    // Alloca "values" are addresses (leal), wide values are
                    // pairs — neither fits a single 32-bit operand.
                    return None;
                }
                if let Some(&phys) = self.reg_assignments.get(&v.0) {
                    return Some(BinopLoc::Reg(super::emit::phys_reg_name(phys)));
                }
                self.state.get_slot(v.0).map(|slot| BinopLoc::Slot(self.slot_ref(slot)))
            }
        }
    }

    /// Single-instruction multiply-by-constant into `dest` from register `src`
    /// (bare names; they may alias). Only the forms that are ≤ the size of
    /// `imull $imm, %src, %dest` (3 bytes) and one cycle:
    ///   0 → xorl d,d          (2 bytes)
    ///   1 → movl n,d          (2 bytes, only when n != d)
    ///  -1 → negl d            (2 bytes) / movl n,d; negl d
    ///   2 → addl d,d          (2 bytes, n == d) / leal (n,n),d (3 bytes)
    ///   3 → leal (n,n,2),d    (3 bytes)
    ///   5 → leal (n,n,4),d    (3 bytes)
    ///   9 → leal (n,n,8),d    (3 bytes)
    ///   4/8 → shll $2/$3, d   (3 bytes, only when n == d; the n != d form
    ///         needs an SIB+disp32 leal of 7 bytes, so imull stays smaller)
    /// Everything else (including negatives beyond -1, which would need a
    /// negl on top) stays `imull`. No scratch register is ever used, so the
    /// direct path's "%eax/%ecx/%edx untouched" invariant holds.
    fn try_emit_mul_imm_reg(&mut self, src: &str, dest: &str, imm: i64) -> bool {
        let same = src == dest;
        let handled = match imm {
            0 => {
                emit!(self.state, "    xorl %{}, %{}", dest, dest);
                true
            }
            1 => {
                if !same {
                    emit!(self.state, "    movl %{}, %{}", src, dest);
                }
                true
            }
            -1 => {
                if !same {
                    emit!(self.state, "    movl %{}, %{}", src, dest);
                }
                emit!(self.state, "    negl %{}", dest);
                true
            }
            2 => {
                if same {
                    emit!(self.state, "    addl %{}, %{}", dest, dest);
                } else {
                    emit!(self.state, "    leal (%{}, %{}), %{}", src, src, dest);
                }
                true
            }
            3 => {
                emit!(self.state, "    leal (%{}, %{}, 2), %{}", src, src, dest);
                true
            }
            4 if same => {
                emit!(self.state, "    shll $2, %{}", dest);
                true
            }
            5 => {
                emit!(self.state, "    leal (%{}, %{}, 4), %{}", src, src, dest);
                true
            }
            8 if same => {
                emit!(self.state, "    shll $3, %{}", dest);
                true
            }
            9 => {
                emit!(self.state, "    leal (%{}, %{}, 8), %{}", src, src, dest);
                true
            }
            _ => false,
        };
        handled
    }

    /// Compute a 32-bit int binop straight into the destination's register,
    /// bypassing the %eax round-trip (movl src,%eax; op ...; movl %eax,%dst
    /// -> op src,%dst). Returns false when the shape doesn't fit; the caller
    /// falls back to the accumulator path. %eax/%ecx/%edx are untouched, so
    /// the accumulator cache stays valid.
    fn try_emit_int_binop_direct(&mut self, dest: &Value, op: IrBinOp, lhs: &Operand, rhs: &Operand) -> bool {
        use BinopLoc::*;
        // Only simple two-address ALU ops, shifts, and mul.
        let is_alu = matches!(op, IrBinOp::Add | IrBinOp::Sub | IrBinOp::And | IrBinOp::Or | IrBinOp::Xor);
        let is_mul = op == IrBinOp::Mul;
        let is_shift = matches!(op, IrBinOp::Shl | IrBinOp::AShr | IrBinOp::LShr);
        if !is_alu && !is_mul && !is_shift {
            return false;
        }
        let Some(dphys) = self.dest_reg(dest) else { return false };
        let d = super::emit::phys_reg_name(dphys);
        let Some(l) = self.binop_loc(lhs) else { return false };
        let Some(r) = self.binop_loc(rhs) else { return false };

        let commutative = matches!(op, IrBinOp::Add | IrBinOp::And | IrBinOp::Or | IrBinOp::Xor | IrBinOp::Mul);
        let lhs_is_d = matches!(&l, Reg(n) if *n == d);
        let rhs_is_d = matches!(&r, Reg(n) if *n == d);

        // ── 3-operand LEA add: dest = reg + reg / reg + imm ──
        // `leal (%a,%b),%d` (3 bytes) beats `movl %a,%d; addl %b,%d` (4+),
        // and `leal imm(%a),%d` (3 bytes) beats `movl %a,%d; addl $imm,%d`.
        if op == IrBinOp::Add {
            if let (Reg(a), Reg(b)) = (&l, &r) {
                if *a != d && *b != d {
                    emit!(self.state, "    leal (%{}, %{}), %{}", a, b, d);
                    return true;
                }
            }
            if let (Reg(a), Imm(i)) = (&l, &r) {
                if *a != d && (-128..=127).contains(i) {
                    emit!(self.state, "    leal {}(%{}), %{}", i, a, d);
                    return true;
                }
            }
            if let (Imm(i), Reg(b)) = (&l, &r) {
                if *b != d && (-128..=127).contains(i) {
                    emit!(self.state, "    leal {}(%{}), %{}", i, b, d);
                    return true;
                }
            }
        }

        // ── Immediate multiply: LEA / shift / addl strength reduction ──
        if is_mul {
            if let Imm(i) = r {
                match &l {
                    Reg(n) => {
                        if self.try_emit_mul_imm_reg(n, d, i) {
                            return true;
                        }
                        emit!(self.state, "    imull ${}, %{}, %{}", i, n, d);
                    }
                    Slot(sr) => {
                        match i {
                            0 => emit!(self.state, "    xorl %{}, %{}", d, d),
                            1 => emit!(self.state, "    movl {}, %{}", sr, d),
                            -1 => {
                                emit!(self.state, "    movl {}, %{}", sr, d);
                                emit!(self.state, "    negl %{}", d);
                            }
                            _ => emit!(self.state, "    imull ${}, {}, %{}", i, sr, d),
                        }
                    }
                    Imm(li) => {
                        let prod = li.wrapping_mul(i);
                        if prod == 0 {
                            emit!(self.state, "    xorl %{}, %{}", d, d);
                        } else {
                            emit!(self.state, "    movl ${}, %{}", prod, d);
                        }
                    }
                }
                return true;
            }
        }

        // ── Variable shifts: count in %cl, value in d. %ecx is clobbered, so
        // refuse when d == %ecx (the count would overwrite the value). The
        // scratch-hazard analysis already treats variable shifts as %ecx
        // hazards, so no other live value may sit in %ecx across this point.
        if is_shift {
            if let Imm(i) = r {
                let sh = (i as u32) & 31;
                let mnem = shift_mnemonic(op);
                if sh == 0 {
                    // x << 0 == x: copy lhs into dest if needed.
                    if !lhs_is_d {
                        match &l {
                            Reg(n) => emit!(self.state, "    movl %{}, %{}", n, d),
                            Slot(sr) => emit!(self.state, "    movl {}, %{}", sr, d),
                            Imm(v) => emit!(self.state, "    movl ${}, %{}", v, d),
                        }
                    }
                    return true;
                }
                if lhs_is_d {
                    emit!(self.state, "    {} ${}, %{}", mnem, sh, d);
                    return true;
                }
                if rhs_is_d {
                    return false; // count is dest: needs a temp; let the acc path handle it
                }
                match &l {
                    Reg(n) => emit!(self.state, "    movl %{}, %{}", n, d),
                    Slot(sr) => emit!(self.state, "    movl {}, %{}", sr, d),
                    Imm(v) => emit!(self.state, "    movl ${}, %{}", v, d),
                }
                emit!(self.state, "    {} ${}, %{}", mnem, sh, d);
                return true;
            }
            // Variable count.
            if d == "ecx" {
                return false;
            }
            if rhs_is_d {
                return false; // value == count in dest: needs a temp
            }
            // Copy lhs into d first (may read %ecx), then load the count.
            if !lhs_is_d {
                match &l {
                    Reg(n) => emit!(self.state, "    movl %{}, %{}", n, d),
                    Slot(sr) => emit!(self.state, "    movl {}, %{}", sr, d),
                    Imm(v) => emit!(self.state, "    movl ${}, %{}", v, d),
                }
            }
            match &r {
                Reg(n) => emit!(self.state, "    movl %{}, %ecx", n),
                Slot(sr) => emit!(self.state, "    movl {}, %ecx", sr),
                Imm(_) => unreachable!("imm handled above"),
            }
            let mnem = shift_mnemonic(op);
            emit!(self.state, "    {} %cl, %{}", mnem, d);
            // %ecx clobbered, but %eax untouched — the acc cache stays valid.
            return true;
        }

        let mnem: String = if is_mul {
            "imull".to_string()
        } else {
            format!("{}l", alu_mnemonic(op))
        };
        let rhs_str = |loc: &BinopLoc| -> String {
            match loc {
                Reg(n) => format!("%{}", n),
                Slot(sr) => sr.clone(),
                Imm(i) => format!("${}", i),
            }
        };

        if lhs_is_d {
            // dest op= rhs. rhs may be d itself (fine: d op= d).
            emit!(self.state, "    {} {}, %{}", mnem, rhs_str(&r), d);
            return true;
        }
        if rhs_is_d {
            if !commutative {
                return false; // sub with dest as rhs: needs a temp.
            }
            // dest op= lhs (commutative).
            emit!(self.state, "    {} {}, %{}", mnem, rhs_str(&l), d);
            return true;
        }
        // dest gets a fresh copy of lhs, then op rhs. rhs proven != d above.
        match &l {
            Reg(n) => emit!(self.state, "    movl %{}, %{}", n, d),
            Slot(sr) => emit!(self.state, "    movl {}, %{}", sr, d),
            Imm(i) => emit!(self.state, "    movl ${}, %{}", i, d),
        }
        emit!(self.state, "    {} {}, %{}", mnem, rhs_str(&r), d);
        true
    }

    pub(super) fn emit_int_binop_impl(&mut self, dest: &Value, op: IrBinOp, lhs: &Operand, rhs: &Operand, _ty: IrType) {
        // Direct-to-dest path: skip the accumulator entirely when the dest
        // has a register and both operands are register/slot/imm-resident.
        if self.try_emit_int_binop_direct(dest, op, lhs, rhs) {
            return;
        }
        // Immediate optimization for ALU ops
        if matches!(op, IrBinOp::Add | IrBinOp::Sub | IrBinOp::And | IrBinOp::Or | IrBinOp::Xor) {
            if let Some(imm) = Self::const_as_imm32(rhs) {
                self.operand_to_eax(lhs);
                let mnem = alu_mnemonic(op);
                emit!(self.state, "    {}l ${}, %eax", mnem, imm);
                self.state.reg_cache.invalidate_acc();
                self.store_eax_to(dest);
                return;
            }
        }

        // Immediate multiply
        if op == IrBinOp::Mul {
            if let Some(imm) = Self::const_as_imm32(rhs) {
                self.operand_to_eax(lhs);
                if !self.try_emit_mul_imm_reg("eax", "eax", imm) {
                    emit!(self.state, "    imull ${}, %eax, %eax", imm);
                }
                self.state.reg_cache.invalidate_acc();
                self.store_eax_to(dest);
                return;
            }
        }

        // Immediate shift
        if matches!(op, IrBinOp::Shl | IrBinOp::AShr | IrBinOp::LShr) {
            if let Some(imm) = Self::const_as_imm32(rhs) {
                self.operand_to_eax(lhs);
                let mnem = shift_mnemonic(op);
                let shift_amount = (imm as u32) & 31;
                if shift_amount != 0 {
                    emit!(self.state, "    {} ${}, %eax", mnem, shift_amount);
                }
                self.state.reg_cache.invalidate_acc();
                self.store_eax_to(dest);
                return;
            }
        }

        // General case: load lhs to eax; a REGISTER-resident rhs is used in
        // place (`op %reg,%eax` — the read is identical to the `movl
        // %reg,%ecx` staging it replaces), anything else stages through %ecx.
        // Variable shifts need the count in %cl, and div/idiv must not take
        // its divisor from %edx (the high half of the dividend), so those two
        // shapes always stage. Slot values deliberately stage too: their slots
        // are finalized lazily (see direct_reg_src_ref); the peephole folds
        // them after materialization.
        self.operand_to_eax(lhs);
        let var_shift = matches!(op, IrBinOp::Shl | IrBinOp::AShr | IrBinOp::LShr);
        let div_like = matches!(
            op,
            IrBinOp::SDiv | IrBinOp::UDiv | IrBinOp::SRem | IrBinOp::URem
        );
        let rhs_ref: String = if let Operand::Value(rv) = rhs {
            let direct = self.direct_reg_src_ref(rv);
            if var_shift || (div_like && matches!(&direct, Some(r) if r == "%edx")) {
                self.operand_to_ecx(rhs);
                "%ecx".to_string()
            } else {
                match direct {
                    Some(r) => r,
                    None => {
                        self.operand_to_ecx(rhs);
                        "%ecx".to_string()
                    }
                }
            }
        } else {
            self.operand_to_ecx(rhs);
            "%ecx".to_string()
        };

        match op {
            IrBinOp::Add => emit!(self.state, "    addl {}, %eax", rhs_ref),
            IrBinOp::Sub => emit!(self.state, "    subl {}, %eax", rhs_ref),
            IrBinOp::Mul => emit!(self.state, "    imull {}, %eax", rhs_ref),
            IrBinOp::And => emit!(self.state, "    andl {}, %eax", rhs_ref),
            IrBinOp::Or => emit!(self.state, "    orl {}, %eax", rhs_ref),
            IrBinOp::Xor => emit!(self.state, "    xorl {}, %eax", rhs_ref),
            IrBinOp::Shl => self.state.emit("    shll %cl, %eax"),
            IrBinOp::AShr => self.state.emit("    sarl %cl, %eax"),
            IrBinOp::LShr => self.state.emit("    shrl %cl, %eax"),
            IrBinOp::SDiv => {
                self.state.emit("    cltd");
                emit!(self.state, "    idivl {}", rhs_ref);
            }
            IrBinOp::UDiv => {
                self.state.emit("    xorl %edx, %edx");
                emit!(self.state, "    divl {}", rhs_ref);
            }
            IrBinOp::SRem => {
                self.state.emit("    cltd");
                emit!(self.state, "    idivl {}", rhs_ref);
                self.state.emit("    movl %edx, %eax");
            }
            IrBinOp::URem => {
                self.state.emit("    xorl %edx, %edx");
                emit!(self.state, "    divl {}", rhs_ref);
                self.state.emit("    movl %edx, %eax");
            }
        }
        self.state.reg_cache.invalidate_acc();
        self.store_eax_to(dest);
    }
}
