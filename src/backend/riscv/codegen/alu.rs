//! RiscvCodegen: integer/float arithmetic, unary ops, binop, copy.

use super::emit::RiscvCodegen;
use crate::common::types::IrType;
use crate::ir::reexports::{IrBinOp, IrConst, Operand, Value};

/// The I-type spelling of `lhs op rhs` when `rhs` is a constant the encoding
/// holds: `(mnemonic, immediate)`.
///
/// Each form computes, bit for bit, what the R-type form computes on the
/// value `operand_to_t0` would stage for the constant (`li` of its signed
/// value, so `K` here *is* that 64-bit value):
///
/// * `addi`/`addiw`, `andi`, `ori`, `xori` take a sign-extended 12-bit
///   immediate, exactly the register value when `-2048 <= K <= 2047`;
///   `addiw`, like `addw`, adds the low 32 bits and sign-extends, and the
///   logical forms are 64-bit in both spellings (there is no `andw`);
/// * `x - K` is `x + (-K)` in the ring, so it is `addi[w]` of `-K` when that
///   fits (it does not for `K == -2048`);
/// * `sll`/`srl`/`sra` read the low six bits of the count register and the
///   `w` forms the low five, so the immediate is `K & 63` (`K & 31`) --
///   the same count, including for an out-of-range constant.
///
/// Everything else (multiply, divide, remainder, rotates, the bit
/// operations, a constant that does not fit) keeps the register form.
fn immediate_form(op: IrBinOp, rhs: &Operand, use_32bit: bool) -> Option<(&'static str, i64)> {
    let Operand::Const(c) = rhs else {
        return None;
    };
    let k = match c {
        IrConst::I8(v) => i64::from(*v),
        IrConst::I16(v) => i64::from(*v),
        IrConst::I32(v) => i64::from(*v),
        IrConst::I64(v) => *v,
        IrConst::Zero => 0,
        _ => return None,
    };
    let fits = |v: i64| (-2048..=2047).contains(&v);
    let form = match (op, use_32bit) {
        (IrBinOp::Add, false) if fits(k) => ("addi", k),
        (IrBinOp::Add, true) if fits(k) => ("addiw", k),
        (IrBinOp::Sub, false) if k != i64::MIN && fits(-k) => ("addi", -k),
        (IrBinOp::Sub, true) if k != i64::MIN && fits(-k) => ("addiw", -k),
        (IrBinOp::And, _) if fits(k) => ("andi", k),
        (IrBinOp::Or, _) if fits(k) => ("ori", k),
        (IrBinOp::Xor, _) if fits(k) => ("xori", k),
        (IrBinOp::Shl, false) => ("slli", k & 63),
        (IrBinOp::Shl, true) => ("slliw", k & 31),
        (IrBinOp::LShr, false) => ("srli", k & 63),
        (IrBinOp::LShr, true) => ("srliw", k & 31),
        (IrBinOp::AShr, false) => ("srai", k & 63),
        (IrBinOp::AShr, true) => ("sraiw", k & 31),
        _ => return None,
    };
    Some(form)
}

/// How `x * c` is computed without `mul` and without the `li` that stages
/// `c`: a shift of `x` into t1 and one add/sub with `x` itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MulPlan {
    /// `c = 2^k + 1`: `slli t1, x, k; add t0, t1, x`.
    AddShifted(u32),
    /// `c = 2^k - 1`: `slli t1, x, k; sub t0, t1, x`.
    ShiftedMinus(u32),
    /// `c = 1 - 2^k`: `slli t1, x, k; sub t0, x, t1`.
    MinusShifted(u32),
}

/// The plan for multiplying by `c` modulo 2^`width`, if `c` has one.
///
/// `slli` shifts all 64 bits, and the 32-bit view adds or subtracts with
/// `addw`/`subw`, which take the low 32 bits of the 64-bit operands and
/// sign-extend the result -- the low 32 bits of `(x << k) ± x` are those of
/// the exact product, so the register ends up bit for bit what `mulw`
/// writes (and the 64-bit forms what `mul` writes). 0, 1 and powers of two
/// (already shifts when they reach the backend) are left to the generic
/// path.
fn mul_plan(c: u64, width: u32) -> Option<MulPlan> {
    let mask = if width == 32 {
        u64::from(u32::MAX)
    } else {
        u64::MAX
    };
    let c = c & mask;
    if c == 0 || c.is_power_of_two() {
        return None;
    }
    let shift = |n: u64| {
        let k = n.trailing_zeros();
        (n.is_power_of_two() && (1..width).contains(&k)).then_some(k)
    };
    // `c` is at least 3 here, and `c + 1` cannot wrap unless `c == mask`.
    if let Some(k) = shift(c - 1) {
        return Some(MulPlan::AddShifted(k));
    }
    if c != mask {
        if let Some(k) = shift(c + 1) {
            return Some(MulPlan::ShiftedMinus(k));
        }
    }
    shift(1u64.wrapping_sub(c) & mask).map(MulPlan::MinusShifted)
}

/// The view `emit_int_binop_impl` multiplies an integer type in: the W
/// forms for `int`/`unsigned`, the 64-bit forms otherwise.
fn mul_width(ty: IrType) -> u32 {
    if matches!(ty, IrType::I32 | IrType::U32) {
        32
    } else {
        64
    }
}

/// Whether this backend multiplies an integer of type `ty` by the
/// (sign-extended) constant `c` without a register for `c` (see
/// [`mul_plan`]).
///
/// The loop constant hoist (`passes::int_const_hoist`) leaves such a factor
/// in place; every other multiplier is staged with `li` per use and is
/// worth a register across the loop. The plan's two single-cycle
/// instructions replace `li` + `mul` one for one, off the multiplier (3+
/// cycles of latency on SiFive U74- and C910-class cores): GCC 16.2 and
/// Clang 23.1.5 expand the same factors (`x * 31` is `slli a4, a2, 5; sub
/// a2, a4, a2` in both).
pub(crate) fn mul_needs_no_constant(c: u64, ty: IrType) -> bool {
    ty.is_integer() && ty.size() <= 8 && mul_plan(c, mul_width(ty)).is_some()
}

/// An integer constant operand, sign-extended to 64 bits.
fn int_const_bits(op: &Operand) -> Option<u64> {
    match op {
        Operand::Const(IrConst::I8(v)) => Some(i64::from(*v) as u64),
        Operand::Const(IrConst::I16(v)) => Some(i64::from(*v) as u64),
        Operand::Const(IrConst::I32(v)) => Some(i64::from(*v) as u64),
        Operand::Const(IrConst::I64(v)) => Some(*v as u64),
        _ => None,
    }
}

impl RiscvCodegen {
    // ---- Unary ----

    pub(super) fn emit_float_neg_impl(&mut self, ty: IrType) {
        if ty == IrType::F64 {
            self.state.emit("    fmv.d.x ft0, t0");
            self.state.emit("    fneg.d ft0, ft0");
            self.state.emit("    fmv.x.d t0, ft0");
        } else {
            self.state.emit("    fmv.w.x ft0, t0");
            self.state.emit("    fneg.s ft0, ft0");
            self.state.emit("    fmv.x.w t0, ft0");
        }
    }

    pub(super) fn emit_int_neg_impl(&mut self, ty: IrType) {
        // Canonical-form discipline (rv64 codegen invariant): every <=32-bit
        // integer in a 64-bit register equals signext(low32); signed narrow
        // values are additionally sign-filled to their own width, unsigned
        // narrow/U32 keep an explicit zero-extension (sltu staging and U64
        // widening read the raw low bits).  Two's-complement negation
        // preserves sign-extension, so the refill below only re-establishes
        // the width canonical the consumers expect — and for I32 the *w
        // form IS the canonical signed-32 op (GCC-identical).
        match ty {
            IrType::I32 => self.state.emit("    negw t0, t0"),
            IrType::I8 => {
                self.state.emit("    neg t0, t0");
                self.state.emit("    slli t0, t0, 56");
                self.state.emit("    srai t0, t0, 56");
            }
            IrType::I16 => {
                self.state.emit("    neg t0, t0");
                self.state.emit("    slli t0, t0, 48");
                self.state.emit("    srai t0, t0, 48");
            }
            IrType::U8 | IrType::U16 | IrType::U32 => {
                // Zero-extend to 32 bits, matching the convention used by
                // emit_bswap/clz (the upper half of a 64-bit neg on a
                // zero-extended U32 is 0xFFFFFFFF — x86-64 audit, claim 1).
                self.state.emit("    neg t0, t0");
                self.state.emit("    slli t0, t0, 32");
                self.state.emit("    srli t0, t0, 32");
            }
            _ => self.state.emit("    neg t0, t0"),
        }
    }

    pub(super) fn emit_int_not_impl(&mut self, ty: IrType) {
        // Bitwise complement also preserves sign-extension of a canonical
        // input: ~sextN(x) = sextN(~x).  I32 therefore needs NO truncation
        // pair at all — plain 64-bit `not` (xori -1) is already sext32
        // canonical and GCC-identical; the old unconditional zext pair
        // violated the invariant.  Signed narrow widths refill their own
        // width; unsigned keep the explicit zero-extension.
        match ty {
            IrType::I32 => self.state.emit("    not t0, t0"),
            IrType::I8 => {
                self.state.emit("    not t0, t0");
                self.state.emit("    slli t0, t0, 56");
                self.state.emit("    srai t0, t0, 56");
            }
            IrType::I16 => {
                self.state.emit("    not t0, t0");
                self.state.emit("    slli t0, t0, 48");
                self.state.emit("    srai t0, t0, 48");
            }
            IrType::U8 | IrType::U16 | IrType::U32 => {
                self.state.emit("    not t0, t0");
                self.state.emit("    slli t0, t0, 32");
                self.state.emit("    srli t0, t0, 32");
            }
            _ => self.state.emit("    not t0, t0"),
        }
    }

    // ---- Integer binop ----

    pub(super) fn emit_int_binop_impl(
        &mut self,
        dest: &Value,
        op: IrBinOp,
        lhs: &Operand,
        rhs: &Operand,
        ty: IrType,
    ) {
        // Note: i128 dispatch is handled by the shared emit_binop default in traits.rs.
        let use_32bit = ty == IrType::I32 || ty == IrType::U32;

        // `x op K` whose constant the I-type encoding holds is one
        // instruction instead of four (`li` of K, the copy of `x` into t1,
        // the copy of K into t2, the R-type form): see `immediate_form`.
        if let Some((mnemonic, imm)) = immediate_form(op, rhs, use_32bit) {
            self.operand_to_t0(lhs);
            self.state
                .emit_fmt(format_args!("    {mnemonic} t0, t0, {imm}"));
            self.state.reg_cache.invalidate_acc();
            self.store_t0_to(dest);
            return;
        }

        // `x * c` with a shift-and-add plan: two single-cycle instructions
        // in place of `li` + `mul` (see `mul_plan`). The W view is the one
        // the `mulw` below would use.
        if op == IrBinOp::Mul && ty.is_integer() && ty.size() <= 8 {
            let factor = match (int_const_bits(lhs), int_const_bits(rhs)) {
                (None, Some(c)) => Some((lhs, c)),
                (Some(c), None) => Some((rhs, c)),
                _ => None,
            };
            if let Some((x, plan)) =
                factor.and_then(|(x, c)| mul_plan(c, mul_width(ty)).map(|plan| (x, plan)))
            {
                let w = if use_32bit { "w" } else { "" };
                let (k, op, operands) = match plan {
                    MulPlan::AddShifted(k) => (k, "add", "t1, t0"),
                    MulPlan::ShiftedMinus(k) => (k, "sub", "t1, t0"),
                    MulPlan::MinusShifted(k) => (k, "sub", "t0, t1"),
                };
                self.operand_to_t0(x);
                self.state.emit_fmt(format_args!("    slli t1, t0, {k}"));
                self.state
                    .emit_fmt(format_args!("    {op}{w} t0, {operands}"));
                self.state.reg_cache.invalidate_acc();
                self.store_t0_to(dest);
                return;
            }
        }

        self.operand_to_t0(lhs);
        self.state.emit("    mv t1, t0");
        self.operand_to_t0(rhs);
        self.state.emit("    mv t2, t0");

        if op == IrBinOp::BitTest {
            // RISC-V has no BT (Zbs `bext` is not modelled): logical-shift the
            // base right by the index modulo the *operation* width and isolate
            // bit zero.  `srlw` reads only the low 5 bits of its count and
            // `srl` the low 6, which IS the modulo at 32/64 bits; subword
            // widths reduce explicitly and then use the XLEN shift.  The `andi`
            // result is 0 or 1, canonical in every integer type's register
            // form (signext(low32) and zero-extension agree on it), so no
            // re-extension follows.
            let bits = (ty.size() * 8) as u32;
            debug_assert!(matches!(bits, 8 | 16 | 32 | 64));
            if bits < 32 {
                self.state
                    .emit_fmt(format_args!("    andi t2, t2, {}", bits - 1));
            }
            let shift = if use_32bit { "srlw" } else { "srl" };
            self.state.emit_fmt(format_args!("    {shift} t0, t1, t2"));
            self.state.emit("    andi t0, t0, 1");
            self.store_t0_to(dest);
            return;
        }

        if matches!(op, IrBinOp::RotateLeft | IrBinOp::RotateRight) {
            // RV64I has no rotate: `rol`/`ror` are Zbb, and this backend does
            // not yet model Z-extension availability, so emit the portable
            // funnel shift.  It needs no scratch beyond the three registers
            // this path already stages into, because the second half's count
            // is `W - n` and `sll`/`srl` take their count from the low
            // lg2(XLEN) bits of the register -- so `-n` already IS `W - n`
            // modulo W and `neg` supplies the complement in one instruction.
            // t1 (the source) stays live until both halves are computed.
            let (first, second) = match (op, use_32bit) {
                (IrBinOp::RotateLeft, true) => ("sllw", "srlw"),
                (IrBinOp::RotateRight, true) => ("srlw", "sllw"),
                (IrBinOp::RotateLeft, false) => ("sll", "srl"),
                (IrBinOp::RotateRight, false) => ("srl", "sll"),
                _ => unreachable!("guarded by the matches! above"),
            };
            self.state.emit_fmt(format_args!("    {first} t0, t1, t2"));
            self.state.emit("    neg t2, t2");
            self.state.emit_fmt(format_args!("    {second} t2, t1, t2"));
            self.state.emit("    or t0, t0, t2");
            if use_32bit {
                // `*w` forms sign-extend their 32-bit result; the canonical
                // rotate value is the unsigned bit pattern, so normalize the
                // same way the BitTest path above does.
                self.state.emit("    slli t0, t0, 32");
                self.state.emit("    srli t0, t0, 32");
            }
            self.store_t0_to(dest);
            return;
        }

        let mnemonic = match (op, use_32bit) {
            (IrBinOp::Add, false) => "add",
            (IrBinOp::Add, true) => "addw",
            (IrBinOp::Sub, false) => "sub",
            (IrBinOp::Sub, true) => "subw",
            (IrBinOp::Mul, false) => "mul",
            (IrBinOp::Mul, true) => "mulw",
            (IrBinOp::SDiv, false) => "div",
            (IrBinOp::SDiv, true) => "divw",
            (IrBinOp::UDiv, false) => "divu",
            (IrBinOp::UDiv, true) => "divuw",
            (IrBinOp::SRem, false) => "rem",
            (IrBinOp::SRem, true) => "remw",
            (IrBinOp::URem, false) => "remu",
            (IrBinOp::URem, true) => "remuw",
            (IrBinOp::And, _) => "and",
            (IrBinOp::Or, _) => "or",
            (IrBinOp::Xor, _) => "xor",
            (IrBinOp::Shl, false) => "sll",
            (IrBinOp::Shl, true) => "sllw",
            (IrBinOp::AShr, false) => "sra",
            (IrBinOp::AShr, true) => "sraw",
            (IrBinOp::LShr, false) => "srl",
            (IrBinOp::LShr, true) => "srlw",
            (IrBinOp::BitTest, _) => unreachable!("BitTest handled above"),
            (IrBinOp::BitSet | IrBinOp::BitClear | IrBinOp::BitToggle, _) => {
                self.emit_bit_modify_impl(dest, op, ty);
                return;
            }
            (IrBinOp::Bzhi, _) => crate::ir::ops::bzhi_outside_x86_64(),
            (IrBinOp::UMulHigh, false) => "mulhu",
            (IrBinOp::SMulHigh, false) => "mulh",
            (IrBinOp::UMulHigh | IrBinOp::SMulHigh, true) => {
                crate::ir::ops::mul_high_unsupported_here()
            }
            (IrBinOp::RotateLeft, _) | (IrBinOp::RotateRight, _) => {
                unreachable!("rotate handled above")
            }
        };
        self.state
            .emit_fmt(format_args!("    {} t0, t1, t2", mnemonic));

        self.store_t0_to(dest);
    }

    /// `BitSet`/`BitClear`/`BitToggle` with the base already staged in `t1`
    /// and the index in `t2`.  RV64I has no Zbs `bset`/`bclr`/`bext`-modify,
    /// so build a one-hot mask and OR/AND-NOT/XOR it.  The shift count is
    /// reduced modulo the *operation* width, not XLEN: an 8/16/32-bit
    /// operation must wrap at 8/16/32 even though `sll` itself masks at 64.
    ///
    /// A 32-bit operation builds its mask with `sllw`, which reads only the
    /// low five bits of the count -- that IS the modulo-32 reduction, so no
    /// `andi` -- and sign-extends the word. The bitwise result of two
    /// sign-extended words is a sign-extended word, so an `I32` result stays
    /// canonical; a `U32` result is valid in its low 32 bits, the register
    /// form every `U32` consumer reads (a widening cast or an unsigned compare
    /// re-zero-extends; the plain `1u << n` shift is the same `sllw`), and
    /// without the `andi` the folded `x | 1u << n` is no longer one
    /// instruction longer than the unfolded shift-and-or it replaces.
    ///
    /// Narrow results are put back into the rv64 canonical form for their
    /// type (signed sign-filled, unsigned zero-extended), which downstream
    /// comparisons and widening reads of 8/16-bit values trust.
    fn emit_bit_modify_impl(&mut self, dest: &Value, op: IrBinOp, ty: IrType) {
        let bits = (ty.size() * 8) as u32;
        debug_assert!(matches!(bits, 8 | 16 | 32 | 64));
        if bits < 32 {
            self.state
                .emit_fmt(format_args!("    andi t2, t2, {}", bits - 1));
        }
        self.state.emit("    li t0, 1");
        if bits == 32 {
            self.state.emit("    sllw t0, t0, t2");
        } else {
            self.state.emit("    sll t0, t0, t2");
        }
        if op == IrBinOp::BitSet {
            self.state.emit("    or t0, t1, t0");
        } else if op == IrBinOp::BitClear {
            self.state.emit("    xori t0, t0, -1");
            self.state.emit("    and t0, t1, t0");
        } else {
            // The only remaining caller-gated operation is BitToggle.
            self.state.emit("    xor t0, t1, t0");
        }
        match ty {
            IrType::I8 => {
                self.state.emit("    slli t0, t0, 56");
                self.state.emit("    srai t0, t0, 56");
            }
            IrType::I16 => {
                self.state.emit("    slli t0, t0, 48");
                self.state.emit("    srai t0, t0, 48");
            }
            IrType::U8 => self.state.emit("    andi t0, t0, 255"),
            IrType::U16 => {
                self.state.emit("    slli t0, t0, 48");
                self.state.emit("    srli t0, t0, 48");
            }
            // 32-bit: see above. 64-bit results need no normalization.
            _ => {}
        }
        self.store_t0_to(dest);
    }

    // ---- Copy i128 ----

    pub(super) fn emit_copy_i128_impl(&mut self, dest: &Value, src: &Operand) {
        self.operand_to_t0_t1(src);
        self.store_t0_t1_to(dest);
    }
}

#[cfg(test)]
mod bit_modify_width_tests {
    use super::*;
    use crate::backend::regalloc::PhysReg;

    fn emit(op: IrBinOp, ty: IrType) -> String {
        let mut cg = RiscvCodegen::new();
        // a0/a1 hold the base/index; a2 receives the result. The index is a
        // runtime Value, so the emitter must perform the modulo-W reduction
        // itself rather than relying on a constant folder.
        cg.reg_assignments.insert(1, PhysReg(12));
        cg.reg_assignments.insert(2, PhysReg(13));
        cg.reg_assignments.insert(3, PhysReg(14));
        cg.emit_int_binop_impl(
            &Value(3),
            op,
            &Operand::Value(Value(1)),
            &Operand::Value(Value(2)),
            ty,
        );
        cg.state.out.buf
    }

    #[test]
    fn index_is_reduced_modulo_the_operation_width_not_xlen() {
        for ty in [
            IrType::I8,
            IrType::U8,
            IrType::I16,
            IrType::U16,
            IrType::I32,
            IrType::U32,
            IrType::I64,
            IrType::U64,
        ] {
            let width = (ty.size() * 8) as u32;
            for op in [IrBinOp::BitSet, IrBinOp::BitClear, IrBinOp::BitToggle] {
                let asm = emit(op, ty);
                match width {
                    // `sllw` reads count bits 4:0 (modulo 32), `sll` bits 5:0
                    // (modulo 64): an explicit mask would be dead code.
                    32 => assert!(
                        asm.contains("sllw t0, t0, t2") && !asm.contains("andi t2"),
                        "{op:?} {ty:?} reduces modulo 32 through sllw:\n{asm}"
                    ),
                    64 => assert!(
                        asm.contains("sll t0, t0, t2") && !asm.contains("andi t2"),
                        "{op:?} {ty:?} reduces modulo 64 through sll:\n{asm}"
                    ),
                    _ => assert!(
                        asm.contains(&format!("andi t2, t2, {}", width - 1)),
                        "{op:?} {ty:?} must mask the index to {width} bits:\n{asm}"
                    ),
                }
            }
        }
    }

    #[test]
    fn narrow_bit_test_reduces_the_index_modulo_the_operation_width() {
        for ty in [IrType::I8, IrType::U8, IrType::I16, IrType::U16] {
            let width = (ty.size() * 8) as u32;
            let asm = emit(IrBinOp::BitTest, ty);
            assert!(
                asm.contains(&format!("andi t2, t2, {}", width - 1)),
                "BitTest {ty:?} must mask the index to {width} bits:\n{asm}"
            );
            assert!(
                asm.contains("srl t0, t1, t2") && asm.contains("andi t0, t0, 1"),
                "BitTest {ty:?} must extract a canonical 0/1 result:\n{asm}"
            );
        }
    }

    #[test]
    fn result_keeps_the_rv64_canonical_integer_form() {
        for op in [IrBinOp::BitSet, IrBinOp::BitClear, IrBinOp::BitToggle] {
            let asm8 = emit(op, IrType::I8);
            assert!(
                asm8.contains("slli t0, t0, 56") && asm8.contains("srai t0, t0, 56"),
                "I8 {op:?} must sign-extend the result:\n{asm8}"
            );
            let asm16 = emit(op, IrType::I16);
            assert!(
                asm16.contains("slli t0, t0, 48") && asm16.contains("srai t0, t0, 48"),
                "I16 {op:?} must sign-extend the result:\n{asm16}"
            );
            // Both 32-bit types build the sign-extended word mask: canonical
            // for I32, low-32-valid (the U32 register form) for U32.
            for ty in [IrType::I32, IrType::U32] {
                let asm32 = emit(op, ty);
                assert!(
                    asm32.contains("sllw t0, t0, t2"),
                    "{ty:?} {op:?} must build a sign-extended word mask:\n{asm32}"
                );
            }
        }
    }
}

#[cfg(test)]
mod immediate_form_tests {
    use super::*;
    use crate::backend::regalloc::PhysReg;

    /// `a2 = a0 op K`, through the real emitter.
    fn emit(op: IrBinOp, ty: IrType, k: IrConst) -> String {
        let mut cg = RiscvCodegen::new();
        cg.reg_assignments.insert(1, PhysReg(12));
        cg.reg_assignments.insert(3, PhysReg(14));
        cg.emit_int_binop_impl(
            &Value(3),
            op,
            &Operand::Value(Value(1)),
            &Operand::Const(k),
            ty,
        );
        cg.state.out.buf
    }

    #[test]
    fn a_fitting_constant_selects_the_i_type_form() {
        let cases: [(IrBinOp, IrType, IrConst, &str); 16] = [
            (
                IrBinOp::Add,
                IrType::I32,
                IrConst::I32(1),
                "addiw t0, t0, 1",
            ),
            (
                IrBinOp::Add,
                IrType::U32,
                IrConst::I32(-7),
                "addiw t0, t0, -7",
            ),
            (
                IrBinOp::Add,
                IrType::I64,
                IrConst::I64(2047),
                "addi t0, t0, 2047",
            ),
            (
                IrBinOp::Add,
                IrType::I64,
                IrConst::I64(-2048),
                "addi t0, t0, -2048",
            ),
            (
                IrBinOp::Sub,
                IrType::I32,
                IrConst::I32(5),
                "addiw t0, t0, -5",
            ),
            (
                IrBinOp::Sub,
                IrType::I64,
                IrConst::I64(2048),
                "addi t0, t0, -2048",
            ),
            (
                IrBinOp::And,
                IrType::I32,
                IrConst::I32(255),
                "andi t0, t0, 255",
            ),
            (
                IrBinOp::And,
                IrType::U64,
                IrConst::I64(-16),
                "andi t0, t0, -16",
            ),
            (IrBinOp::Or, IrType::I64, IrConst::I64(16), "ori t0, t0, 16"),
            (
                IrBinOp::Xor,
                IrType::I32,
                IrConst::I32(-1),
                "xori t0, t0, -1",
            ),
            (
                IrBinOp::Shl,
                IrType::I32,
                IrConst::I32(3),
                "slliw t0, t0, 3",
            ),
            (
                IrBinOp::Shl,
                IrType::I64,
                IrConst::I64(40),
                "slli t0, t0, 40",
            ),
            (
                IrBinOp::LShr,
                IrType::U32,
                IrConst::I32(31),
                "srliw t0, t0, 31",
            ),
            (
                IrBinOp::LShr,
                IrType::U64,
                IrConst::I64(63),
                "srli t0, t0, 63",
            ),
            (
                IrBinOp::AShr,
                IrType::I32,
                IrConst::I32(31),
                "sraiw t0, t0, 31",
            ),
            (
                IrBinOp::AShr,
                IrType::I64,
                IrConst::I64(3),
                "srai t0, t0, 3",
            ),
        ];
        for (op, ty, k, want) in cases {
            let asm = emit(op, ty, k);
            assert!(
                asm.contains(want),
                "{op:?} {ty:?} {k:?}: want `{want}`:\n{asm}"
            );
            assert!(
                !asm.contains("t2") && !asm.lines().any(|l| l.trim_start().starts_with("li ")),
                "{op:?} {ty:?} {k:?}: no staging left:\n{asm}"
            );
        }
    }

    #[test]
    fn a_shift_count_is_reduced_like_the_register_form_reduces_it() {
        // `sll`/`srl`/`sra` read six count bits and the `w` forms five, so
        // an out-of-range constant count means the same reduced count.
        for (op, ty, k, want) in [
            (
                IrBinOp::Shl,
                IrType::I32,
                IrConst::I32(33),
                "slliw t0, t0, 1",
            ),
            (
                IrBinOp::Shl,
                IrType::I64,
                IrConst::I64(65),
                "slli t0, t0, 1",
            ),
            (
                IrBinOp::AShr,
                IrType::I32,
                IrConst::I32(-1),
                "sraiw t0, t0, 31",
            ),
            (
                IrBinOp::LShr,
                IrType::I64,
                IrConst::I64(-1),
                "srli t0, t0, 63",
            ),
            (IrBinOp::Shl, IrType::I8, IrConst::I8(9), "slli t0, t0, 9"),
        ] {
            let asm = emit(op, ty, k);
            assert!(
                asm.contains(want),
                "{op:?} {ty:?} {k:?}: want `{want}`:\n{asm}"
            );
        }
    }

    #[test]
    fn a_constant_the_encoding_cannot_hold_keeps_the_register_form() {
        for (op, ty, k) in [
            (IrBinOp::Add, IrType::I32, IrConst::I32(2048)),
            (IrBinOp::Add, IrType::I64, IrConst::I64(-2049)),
            (IrBinOp::Sub, IrType::I64, IrConst::I64(-2048)), // -K == 2048
            (IrBinOp::Sub, IrType::I64, IrConst::I64(i64::MIN)),
            (IrBinOp::And, IrType::U32, IrConst::I64(4_294_967_295)),
            (IrBinOp::Or, IrType::I64, IrConst::I64(1 << 40)),
            (IrBinOp::Xor, IrType::I32, IrConst::I32(4096)),
        ] {
            let asm = emit(op, ty, k);
            assert!(
                asm.contains("t0, t1, t2"),
                "{op:?} {ty:?} {k:?}: register form expected:\n{asm}"
            );
        }
    }

    /// 6 has no shift-and-add plan, so the multiply keeps `mul` too (the
    /// planned factors are `mul_plan_tests`' subject).
    #[test]
    fn operations_without_an_i_type_twin_keep_the_register_form() {
        for op in [
            IrBinOp::Mul,
            IrBinOp::SDiv,
            IrBinOp::UDiv,
            IrBinOp::SRem,
            IrBinOp::URem,
        ] {
            let asm = emit(op, IrType::I64, IrConst::I64(6));
            assert!(asm.contains("t0, t1, t2"), "{op:?}:\n{asm}");
        }
        let rot = emit(IrBinOp::RotateLeft, IrType::I64, IrConst::I64(3));
        assert!(
            rot.contains("neg t2, t2"),
            "rotate keeps its funnel:\n{rot}"
        );
        let bt = emit(IrBinOp::BitTest, IrType::I64, IrConst::I64(3));
        assert!(
            bt.contains("srl t0, t1, t2"),
            "bit test keeps its shift:\n{bt}"
        );
    }

    #[test]
    fn every_i_type_form_equals_its_register_twin_on_rv64() {
        // A model of the two RV64 semantics over boundary operands: the
        // register form sees `li`'s value K; the I-type form sees the
        // sign-extended 12-bit immediate (or the 5/6-bit count).
        fn sext32(v: i64) -> i64 {
            i64::from(v as i32)
        }
        let xs = [
            0i64,
            1,
            -1,
            0x7fff_ffff,
            -0x8000_0000,
            0x1234_5678_9abc_def0,
            i64::MIN,
            i64::MAX,
        ];
        let ks = [0i64, 1, -1, 31, 32, 63, 64, 2047, -2048, 2048, -2049];
        for &x in &xs {
            for &k in &ks {
                for (op, w) in [
                    (IrBinOp::Add, false),
                    (IrBinOp::Add, true),
                    (IrBinOp::Sub, false),
                    (IrBinOp::Sub, true),
                    (IrBinOp::And, false),
                    (IrBinOp::Or, true),
                    (IrBinOp::Xor, false),
                    (IrBinOp::Shl, false),
                    (IrBinOp::Shl, true),
                    (IrBinOp::LShr, false),
                    (IrBinOp::LShr, true),
                    (IrBinOp::AShr, false),
                    (IrBinOp::AShr, true),
                ] {
                    let Some((mnem, imm)) = immediate_form(op, &Operand::Const(IrConst::I64(k)), w)
                    else {
                        continue;
                    };
                    let reg = match (op, w) {
                        (IrBinOp::Add, false) => x.wrapping_add(k),
                        (IrBinOp::Add, true) => sext32(x.wrapping_add(k)),
                        (IrBinOp::Sub, false) => x.wrapping_sub(k),
                        (IrBinOp::Sub, true) => sext32(x.wrapping_sub(k)),
                        (IrBinOp::And, _) => x & k,
                        (IrBinOp::Or, _) => x | k,
                        (IrBinOp::Xor, _) => x ^ k,
                        (IrBinOp::Shl, false) => x.wrapping_shl((k & 63) as u32),
                        (IrBinOp::Shl, true) => {
                            sext32((x as i32).wrapping_shl((k & 31) as u32) as i64)
                        }
                        (IrBinOp::LShr, false) => ((x as u64) >> (k & 63)) as i64,
                        (IrBinOp::LShr, true) => sext32(((x as u32) >> (k & 31)) as i64),
                        (IrBinOp::AShr, false) => x >> (k & 63),
                        (IrBinOp::AShr, true) => sext32(((x as i32) >> (k & 31)) as i64),
                        _ => unreachable!(),
                    };
                    assert!((-2048..=2047).contains(&imm) || mnem.starts_with('s'));
                    let ity = match mnem {
                        "addi" => x.wrapping_add(imm),
                        "addiw" => sext32(x.wrapping_add(imm)),
                        "andi" => x & imm,
                        "ori" => x | imm,
                        "xori" => x ^ imm,
                        "slli" => x.wrapping_shl(imm as u32),
                        "slliw" => sext32((x as i32).wrapping_shl(imm as u32) as i64),
                        "srli" => ((x as u64) >> imm) as i64,
                        "srliw" => sext32(((x as u32) >> imm) as i64),
                        "srai" => x >> imm,
                        "sraiw" => sext32(((x as i32) >> imm) as i64),
                        _ => unreachable!("{mnem}"),
                    };
                    assert_eq!(reg, ity, "{op:?} w={w} x={x:#x} k={k} -> {mnem} {imm}");
                }
            }
        }
    }
}

#[cfg(test)]
mod mul_plan_tests {
    use super::*;
    use crate::backend::regalloc::PhysReg;
    use std::collections::HashMap;

    /// `%3 = %1 * c` through the real emitter, `%1` and `%3` in registers.
    fn emit(ty: IrType, c: i64, constant_first: bool) -> Vec<String> {
        let mut cg = RiscvCodegen::new();
        cg.reg_assignments.insert(1, PhysReg(12));
        cg.reg_assignments.insert(3, PhysReg(14));
        let x = Operand::Value(Value(1));
        let k = Operand::Const(IrConst::I64(c));
        let (lhs, rhs) = if constant_first { (k, x) } else { (x, k) };
        cg.emit_int_binop_impl(&Value(3), IrBinOp::Mul, &lhs, &rhs, ty);
        cg.state
            .out
            .buf
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect()
    }

    /// Execute the register-only RV64 lines the multiply paths emit (`mv`,
    /// `li`, `slli`, `add[w]`, `sub[w]`, `mul[w]`): a `w` form operates on
    /// the low 32 bits and sign-extends its result.
    fn exec(lines: &[String], regs: &mut HashMap<String, u64>) {
        for line in lines {
            let (mnem, rest) = line.split_once(' ').unwrap_or((line.as_str(), ""));
            let ops: Vec<&str> = rest.split(',').map(str::trim).collect();
            let get = |regs: &HashMap<String, u64>, r: &str| {
                *regs
                    .get(r)
                    .unwrap_or_else(|| panic!("read of unset {r} in {line:?}"))
            };
            let sext32 = |v: u64| i64::from(v as u32 as i32) as u64;
            let value = match mnem {
                "mv" => get(regs, ops[1]),
                "li" => ops[1]
                    .parse::<i64>()
                    .unwrap_or_else(|_| panic!("li {line:?}")) as u64,
                "slli" => {
                    let k: u32 = ops[2].parse().unwrap_or_else(|_| panic!("{line:?}"));
                    get(regs, ops[1]) << k
                }
                "add" => get(regs, ops[1]).wrapping_add(get(regs, ops[2])),
                "sub" => get(regs, ops[1]).wrapping_sub(get(regs, ops[2])),
                "mul" => get(regs, ops[1]).wrapping_mul(get(regs, ops[2])),
                "addw" => sext32(get(regs, ops[1]).wrapping_add(get(regs, ops[2]))),
                "subw" => sext32(get(regs, ops[1]).wrapping_sub(get(regs, ops[2]))),
                "mulw" => sext32(get(regs, ops[1]).wrapping_mul(get(regs, ops[2]))),
                _ => panic!("unexpected line {line:?}"),
            };
            regs.insert(ops[0].to_string(), value);
        }
    }

    /// The result register's name (the destination of the last line).
    fn dest_of(lines: &[String]) -> String {
        let last = lines.last().expect("no lines");
        let (_, rest) = last.split_once(' ').expect("operands");
        rest.split(',').next().expect("dest").trim().to_string()
    }

    /// The source register's name, read from the staging `mv t0, <reg>`.
    fn source_of(lines: &[String]) -> String {
        lines
            .iter()
            .find_map(|l| l.strip_prefix("mv t0, ").map(str::to_string))
            .expect("staging of x")
    }

    /// Every constant in -1100..=1100 and every `2^k ± 1` / `1 - 2^k`
    /// family member, in every integer view, from either operand side: a
    /// planned factor is `slli` + `add[w]`/`sub[w]` with no `li` and no
    /// `mul`, every other one keeps `li` + `mul[w]`, and both produce the
    /// register value of the exact product (`mulw`'s sign-extended word
    /// for `int`/`unsigned`, the 64-bit product otherwise).
    #[test]
    fn planned_factors_multiply_exactly_without_li_or_mul() {
        let xs = [
            0u64,
            1,
            3,
            0x7fff_ffff,
            0xffff_ffff_8000_0001,
            0xffff_ffff_ffff_ffff,
            0x1234_5678_9abc_def1,
            0x0000_0000_dead_beef,
        ];
        let mut planned = 0usize;
        for ty in [
            IrType::I32,
            IrType::U32,
            IrType::I64,
            IrType::U64,
            IrType::I16,
        ] {
            let width = mul_width(ty);
            let mut constants: Vec<i64> = (-1100..=1100).collect();
            for k in 1..62 {
                constants.extend([(1i64 << k) + 1, (1i64 << k) - 1, 1 - (1i64 << k)]);
            }
            for &c in &constants {
                let has_plan = mul_plan(c as u64, width).is_some();
                assert_eq!(mul_needs_no_constant(c as u64, ty), has_plan, "{c} {ty:?}");
                for constant_first in [false, true] {
                    let lines = emit(ty, c, constant_first);
                    let uses_mul = lines.iter().any(|l| l.starts_with("mul"));
                    let uses_li = lines.iter().any(|l| l.starts_with("li "));
                    if has_plan {
                        planned += 1;
                        assert!(!uses_mul && !uses_li, "x * {c} ({ty:?}): {lines:?}");
                        assert_eq!(lines.len(), 4, "mv, slli, add/sub, mv: {lines:?}");
                    } else {
                        assert!(uses_mul, "x * {c} ({ty:?}): {lines:?}");
                    }
                    let src = source_of(&lines);
                    let dst = dest_of(&lines);
                    for &x in &xs {
                        // The rv64 register form of an `int`/`unsigned` value
                        // is its sign-extended word.
                        let x = if width == 32 {
                            i64::from(x as u32 as i32) as u64
                        } else {
                            x
                        };
                        let mut regs = HashMap::new();
                        regs.insert(src.clone(), x);
                        exec(&lines, &mut regs);
                        let product = x.wrapping_mul(c as u64);
                        let want = if width == 32 {
                            i64::from(product as u32 as i32) as u64
                        } else {
                            product
                        };
                        assert_eq!(regs[&dst], want, "{x:#x} * {c} ({ty:?}): {lines:?}");
                    }
                }
            }
        }
        // Each type sees 3 + 5 + 9 + 17 + ... in the range and both sides.
        assert!(planned > 1000, "{planned}");
    }

    #[test]
    fn plan_shapes_match_the_constant_families() {
        assert_eq!(mul_plan(3, 64), Some(MulPlan::AddShifted(1)));
        assert_eq!(mul_plan(33, 32), Some(MulPlan::AddShifted(5)));
        assert_eq!(mul_plan(31, 32), Some(MulPlan::ShiftedMinus(5)));
        assert_eq!(mul_plan(7, 64), Some(MulPlan::ShiftedMinus(3)));
        assert_eq!(mul_plan((-3i64) as u64, 64), Some(MulPlan::MinusShifted(2)));
        assert_eq!(mul_plan(u64::MAX, 64), Some(MulPlan::MinusShifted(1)));
        assert_eq!(mul_plan(0xffff_fffd, 32), Some(MulPlan::MinusShifted(2)));
        assert_eq!(mul_plan(0x8000_0001, 32), Some(MulPlan::AddShifted(31)));
        assert_eq!(
            mul_plan(0x8000_0000_0000_0001, 64),
            Some(MulPlan::AddShifted(63))
        );
        // Not one shift and one add/sub away, a power of two, 0 or 1.
        for c in [0u64, 1, 2, 8, 6, 10, 12, 1000, (-5i64) as u64] {
            assert_eq!(mul_plan(c, 64), None, "{c}");
        }
        // 0xffff_fffd is -3 in the 32-bit view only.
        assert_eq!(mul_plan(0xffff_fffd, 64), None);
    }
}
