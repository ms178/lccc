//! RiscvCodegen: comparisons, fused cmp+branch, select.

use super::emit::RiscvCodegen;
use crate::common::types::IrType;
use crate::ir::reexports::{IrCmpOp, IrConst, Operand, Value};

/// An integer compare against a constant, as the I-type sequence that leaves
/// the 0/1 result in `t0` from the extended register operand in `t1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ImmCmp {
    /// `seqz`/`snez t0, t1`: (in)equality with zero, and `x <=u 0`/`x >u 0`.
    Zero { eq: bool },
    /// `slt t0, zero, t1`: signed `x > 0`.
    Positive,
    /// `xori t0, t1, k`, then `seqz`/`snez t0, t0`.
    Xor { k: i64, eq: bool },
    /// `slti`/`sltiu t0, t1, k`, then `xori t0, t0, 1` when `negate`.
    Below {
        unsigned: bool,
        k: i64,
        negate: bool,
    },
}

/// The register value `emit_cmp_operand_load` gives the constant `c` as a
/// compare operand of type `ty`: `li` materializes the signed value, then
/// [`RiscvCodegen::emit_cmp_extend`] reduces it to the type's width and
/// re-extends it the way the type compares. `None` for a non-integer
/// constant or a type the extension does not model.
pub(super) fn cmp_const_value(c: &IrConst, ty: IrType) -> Option<i64> {
    let raw = match *c {
        IrConst::I8(v) => i64::from(v),
        IrConst::I16(v) => i64::from(v),
        IrConst::I32(v) => i64::from(v),
        IrConst::I64(v) => v,
        IrConst::Zero => 0,
        _ => return None,
    };
    Some(match ty {
        IrType::U8 => raw & 0xff,
        IrType::U16 => raw & 0xffff,
        IrType::U32 => raw & 0xffff_ffff,
        IrType::I8 => i64::from(raw as i8),
        IrType::I16 => i64::from(raw as i16),
        IrType::I32 => i64::from(raw as i32),
        IrType::I64 | IrType::U64 | IrType::Ptr => raw,
        _ => return None,
    })
}

/// The I-type form of `x op v` for the extended constant `v`, or `None` when
/// only the register form computes it. Every form is exact on the extended
/// 64-bit operands the register form compares:
///
/// * `x == v` / `x != v`: `x ^ v` is zero exactly when they are equal
///   (`xori` sign-extends its imm12 exactly as `li` would); zero needs no
///   `xori` at all.
/// * `x < v`: `slti`/`sltiu` (the unsigned form compares against the
///   sign-extended imm12 as an unsigned value, which is `v` itself).
///   `x >= v` is its negation.
/// * `x <= v` is `x < v + 1` unless `v` is the type's maximum (`v + 1`
///   would wrap): `checked_add` refuses `i64::MAX`, and the unsigned
///   maximum `-1` is refused explicitly. `x > v` is its negation, except
///   signed `x > 0`, which is the single `slt t0, zero, t1`, and unsigned
///   `x > 0`/`x <= 0`, which are `snez`/`seqz`.
pub(super) fn imm_cmp_form(op: IrCmpOp, v: i64) -> Option<ImmCmp> {
    let fits = |k: i64| (-2048..=2047).contains(&k);
    let below = |unsigned: bool, k: i64, negate: bool| {
        fits(k).then_some(ImmCmp::Below {
            unsigned,
            k,
            negate,
        })
    };
    match op {
        IrCmpOp::Eq | IrCmpOp::Ne => {
            let eq = op == IrCmpOp::Eq;
            if v == 0 {
                Some(ImmCmp::Zero { eq })
            } else {
                fits(v).then_some(ImmCmp::Xor { k: v, eq })
            }
        }
        IrCmpOp::Slt => below(false, v, false),
        IrCmpOp::Sge => below(false, v, true),
        IrCmpOp::Ult => below(true, v, false),
        IrCmpOp::Uge => below(true, v, true),
        IrCmpOp::Sgt if v == 0 => Some(ImmCmp::Positive),
        IrCmpOp::Sle | IrCmpOp::Sgt => below(false, v.checked_add(1)?, op == IrCmpOp::Sgt),
        IrCmpOp::Ule | IrCmpOp::Ugt if v == 0 => Some(ImmCmp::Zero {
            eq: op == IrCmpOp::Ule,
        }),
        IrCmpOp::Ule | IrCmpOp::Ugt if v == -1 => None,
        IrCmpOp::Ule | IrCmpOp::Ugt => below(true, v.wrapping_add(1), op == IrCmpOp::Ugt),
    }
}

/// The register operand and I-type form of `lhs op rhs` at type `ty` when
/// exactly one side is a constant the form holds (a constant left side
/// compares with the swapped predicate).
fn imm_cmp<'a>(
    op: IrCmpOp,
    lhs: &'a Operand,
    rhs: &'a Operand,
    ty: IrType,
) -> Option<(&'a Operand, ImmCmp)> {
    let (reg, op, c) = match (lhs, rhs) {
        (Operand::Value(_), Operand::Const(c)) => (lhs, op, c),
        (Operand::Const(c), Operand::Value(_)) => (rhs, op.swapped(), c),
        _ => return None,
    };
    Some((reg, imm_cmp_form(op, cmp_const_value(c, ty)?)?))
}

/// Whether the RV64 backend compares against `c` (at `ty`, as the
/// `const_on_rhs` operand of `op`) without materializing it, when the
/// compare's boolean is materialized rather than fused into a branch (a
/// branch compares two registers). `int_const_hoist` asks this before
/// hoisting a compare constant into a register.
pub(crate) fn cmp_needs_no_register(
    op: IrCmpOp,
    const_on_rhs: bool,
    c: &IrConst,
    ty: IrType,
) -> bool {
    let op = if const_on_rhs { op } else { op.swapped() };
    cmp_const_value(c, ty).is_some_and(|v| imm_cmp_form(op, v).is_some())
}

impl RiscvCodegen {
    // ---- Float comparison ----

    pub(super) fn emit_float_cmp_impl(
        &mut self,
        dest: &Value,
        op: IrCmpOp,
        lhs: &Operand,
        rhs: &Operand,
        ty: IrType,
    ) {
        // Float comparison (F32/F64): load operands into t1/t2, then move to float regs.
        self.operand_to_t0(lhs);
        self.state.emit("    mv t1, t0");
        self.operand_to_t0(rhs);
        self.state.emit("    mv t2, t0");
        let s = if ty == IrType::F64 { "d" } else { "s" };
        let fmv = if s == "d" { "fmv.d.x" } else { "fmv.w.x" };
        self.state.emit_fmt(format_args!("    {} ft0, t1", fmv));
        self.state.emit_fmt(format_args!("    {} ft1, t2", fmv));
        match op {
            IrCmpOp::Eq => self
                .state
                .emit_fmt(format_args!("    feq.{} t0, ft0, ft1", s)),
            IrCmpOp::Ne => {
                self.state
                    .emit_fmt(format_args!("    feq.{} t0, ft0, ft1", s));
                self.state.emit("    xori t0, t0, 1");
            }
            IrCmpOp::Slt | IrCmpOp::Ult => self
                .state
                .emit_fmt(format_args!("    flt.{} t0, ft0, ft1", s)),
            IrCmpOp::Sle | IrCmpOp::Ule => self
                .state
                .emit_fmt(format_args!("    fle.{} t0, ft0, ft1", s)),
            IrCmpOp::Sgt | IrCmpOp::Ugt => self
                .state
                .emit_fmt(format_args!("    flt.{} t0, ft1, ft0", s)),
            IrCmpOp::Sge | IrCmpOp::Uge => self
                .state
                .emit_fmt(format_args!("    fle.{} t0, ft1, ft0", s)),
        }
        self.store_t0_to(dest);
    }

    // ---- Integer comparison ----

    pub(super) fn emit_int_cmp_impl(
        &mut self,
        dest: &Value,
        op: IrCmpOp,
        lhs: &Operand,
        rhs: &Operand,
        ty: IrType,
    ) {
        self.emit_int_cmp_to_t0(op, lhs, rhs, ty);
        self.store_t0_to(dest);
    }

    /// Integer comparison into `t0` (0 or 1): a constant operand an I-type
    /// form holds is compared in place ([`imm_cmp_form`]); otherwise both
    /// operands are loaded and extended into `t1`/`t2` for the register
    /// form.
    fn emit_int_cmp_to_t0(&mut self, op: IrCmpOp, lhs: &Operand, rhs: &Operand, ty: IrType) {
        if let Some((reg, form)) = imm_cmp(op, lhs, rhs, ty) {
            self.operand_to_t0(reg);
            self.state.emit("    mv t1, t0");
            self.emit_cmp_extend("t1", ty);
            match form {
                ImmCmp::Zero { eq } => self.state.emit(if eq {
                    "    seqz t0, t1"
                } else {
                    "    snez t0, t1"
                }),
                ImmCmp::Positive => self.state.emit("    slt t0, zero, t1"),
                ImmCmp::Xor { k, eq } => {
                    self.state.emit_fmt(format_args!("    xori t0, t1, {k}"));
                    self.state.emit(if eq {
                        "    seqz t0, t0"
                    } else {
                        "    snez t0, t0"
                    });
                }
                ImmCmp::Below {
                    unsigned,
                    k,
                    negate,
                } => {
                    let mnemonic = if unsigned { "sltiu" } else { "slti" };
                    self.state
                        .emit_fmt(format_args!("    {mnemonic} t0, t1, {k}"));
                    if negate {
                        self.state.emit("    xori t0, t0, 1");
                    }
                }
            }
            self.state.reg_cache.invalidate_acc();
            return;
        }
        // Integer comparison: load + sign/zero-extend, then compare
        self.emit_cmp_operand_load(lhs, rhs, ty);
        match op {
            IrCmpOp::Eq => {
                self.state.emit("    sub t0, t1, t2");
                self.state.emit("    seqz t0, t0");
            }
            IrCmpOp::Ne => {
                self.state.emit("    sub t0, t1, t2");
                self.state.emit("    snez t0, t0");
            }
            IrCmpOp::Slt => self.state.emit("    slt t0, t1, t2"),
            IrCmpOp::Ult => self.state.emit("    sltu t0, t1, t2"),
            IrCmpOp::Sge => {
                self.state.emit("    slt t0, t1, t2");
                self.state.emit("    xori t0, t0, 1");
            }
            IrCmpOp::Uge => {
                self.state.emit("    sltu t0, t1, t2");
                self.state.emit("    xori t0, t0, 1");
            }
            IrCmpOp::Sgt => self.state.emit("    slt t0, t2, t1"),
            IrCmpOp::Ugt => self.state.emit("    sltu t0, t2, t1"),
            IrCmpOp::Sle => {
                self.state.emit("    slt t0, t2, t1");
                self.state.emit("    xori t0, t0, 1");
            }
            IrCmpOp::Ule => {
                self.state.emit("    sltu t0, t2, t1");
                self.state.emit("    xori t0, t0, 1");
            }
        }
        self.state.reg_cache.invalidate_acc();
    }

    // ---- Conditional increment (`select(c, base + 1, base)`) ----

    /// `dest = base + (cond != 0)` (`== 0` when the increment is the false
    /// arm), branch-free: the generic SSA proof (`generation.rs`) showed the
    /// Add has one use and the other arm is its base. The branchy select
    /// this replaces costs a data-dependent branch per evaluation, and in a
    /// counting loop over text that branch is unpredictable. Clang 23.1.5
    /// emits the same `seqz`/`snez` + `addw` for `n += c == ' '`.
    pub(super) fn emit_conditional_increment_select_impl(
        &mut self,
        dest: &Value,
        cond: &Operand,
        base: &Operand,
        increment_on_true: bool,
        ty: IrType,
    ) {
        self.operand_to_t0(cond);
        self.state.emit(if increment_on_true {
            "    snez t0, t0"
        } else {
            "    seqz t0, t0"
        });
        self.state.reg_cache.invalidate_acc();
        self.emit_add_boolean_t0(dest, base, ty);
    }

    /// As above, computing the comparison (inverted when the increment is
    /// the false arm) straight into the 0/1 addend.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn emit_fused_cmp_conditional_increment_select_impl(
        &mut self,
        op: IrCmpOp,
        lhs: &Operand,
        rhs: &Operand,
        cmp_ty: IrType,
        base: &Operand,
        increment_on_true: bool,
        dest: &Value,
        sel_ty: IrType,
    ) {
        let op = if increment_on_true { op } else { op.inverse() };
        self.emit_int_cmp_to_t0(op, lhs, rhs, cmp_ty);
        self.emit_add_boolean_t0(dest, base, sel_ty);
    }

    /// `dest = base + t0` at the Add's width: `addw` for 32-bit types (the
    /// register form the unfused `addw` and the select's copy produce),
    /// `add` otherwise.
    fn emit_add_boolean_t0(&mut self, dest: &Value, base: &Operand, ty: IrType) {
        self.state.emit("    mv t1, t0");
        self.operand_to_t0(base);
        let w = if matches!(ty, IrType::I32 | IrType::U32) {
            "w"
        } else {
            ""
        };
        self.state.emit_fmt(format_args!("    add{w} t0, t0, t1"));
        self.state.reg_cache.invalidate_acc();
        self.store_t0_to(dest);
    }

    // ---- Fused compare-and-branch ----

    pub(super) fn emit_fused_cmp_branch_impl(
        &mut self,
        op: IrCmpOp,
        lhs: &Operand,
        rhs: &Operand,
        ty: IrType,
        true_label: &str,
        false_label: &str,
    ) {
        // Load operands into t1, t2 (with sign/zero-extension for sub-64-bit types)
        self.emit_cmp_operand_load(lhs, rhs, ty);

        // Emit inverted branch to skip over the true-path jump.
        let (inv_branch, r1, r2) = match op {
            IrCmpOp::Eq => ("bne", "t1", "t2"),
            IrCmpOp::Ne => ("beq", "t1", "t2"),
            IrCmpOp::Slt => ("bge", "t1", "t2"),
            IrCmpOp::Sge => ("blt", "t1", "t2"),
            IrCmpOp::Ult => ("bgeu", "t1", "t2"),
            IrCmpOp::Uge => ("bltu", "t1", "t2"),
            // Swap operands for > and <=, and invert
            IrCmpOp::Sgt => ("bge", "t2", "t1"), // NOT(a > b)  = b >= a
            IrCmpOp::Sle => ("blt", "t2", "t1"), // NOT(a <= b) = b < a
            IrCmpOp::Ugt => ("bgeu", "t2", "t1"), // NOT(a > b)  = b >= a (unsigned)
            IrCmpOp::Ule => ("bltu", "t2", "t1"), // NOT(a <= b) = b < a (unsigned)
        };
        let skip = self.state.fresh_label("skip");
        self.state
            .emit_fmt(format_args!("    {} {}, {}, {}", inv_branch, r1, r2, skip));
        self.state
            .emit_fmt(format_args!("    jump {}, t6", true_label));
        self.state.emit_fmt(format_args!("{}:", skip));
        self.state
            .emit_fmt(format_args!("    jump {}, t6", false_label));
        self.state.reg_cache.invalidate_all();
    }

    // ---- Select ----

    pub(super) fn emit_select_impl(
        &mut self,
        dest: &Value,
        cond: &Operand,
        true_val: &Operand,
        false_val: &Operand,
        _ty: IrType,
    ) {
        let label_id = self.state.next_label_id();
        let skip_label = format!(".Lsel_skip_{}", label_id);

        // Load false_val (default) into t0
        self.operand_to_t0(false_val);
        // Save to t2 so we can test the condition
        self.state.emit("    mv t2, t0");

        // Load condition
        self.operand_to_t0(cond);
        // Branch to skip if condition is zero (keep false_val)
        self.state
            .emit_fmt(format_args!("    beqz t0, {}", skip_label));

        // Load true_val into t2 (overrides the false_val)
        self.operand_to_t0(true_val);
        self.state.emit("    mv t2, t0");

        // Skip label
        self.state.emit_fmt(format_args!("{}:", skip_label));

        // Move result to t0 and store
        self.state.emit("    mv t0, t2");
        self.state.reg_cache.invalidate_acc();
        self.store_t0_to(dest);
    }

    // ---- F128 comparison ----

    pub(super) fn emit_f128_cmp_impl(
        &mut self,
        dest: &Value,
        op: IrCmpOp,
        lhs: &Operand,
        rhs: &Operand,
    ) {
        crate::backend::f128_softfloat::f128_cmp(self, dest, op, lhs, rhs);
    }
}

#[cfg(test)]
mod imm_cmp_tests {
    use super::*;

    const OPS: [IrCmpOp; 10] = [
        IrCmpOp::Eq,
        IrCmpOp::Ne,
        IrCmpOp::Slt,
        IrCmpOp::Sle,
        IrCmpOp::Sgt,
        IrCmpOp::Sge,
        IrCmpOp::Ult,
        IrCmpOp::Ule,
        IrCmpOp::Ugt,
        IrCmpOp::Uge,
    ];
    const TYPES: [IrType; 9] = [
        IrType::U8,
        IrType::U16,
        IrType::U32,
        IrType::U64,
        IrType::I8,
        IrType::I16,
        IrType::I32,
        IrType::I64,
        IrType::Ptr,
    ];
    /// Constants around every boundary the forms care about: zero, the
    /// imm12 edges (and one past them, where `v + 1` stops fitting), the
    /// narrow types' extremes, and the 64-bit extremes.
    const VALUES: [i64; 26] = [
        0,
        1,
        -1,
        2,
        -2,
        32,
        127,
        128,
        -128,
        255,
        256,
        2046,
        2047,
        2048,
        -2047,
        -2048,
        -2049,
        32767,
        65535,
        i32::MAX as i64,
        i32::MIN as i64,
        u32::MAX as i64,
        i64::MAX,
        i64::MIN,
        i64::MAX - 1,
        0x1_0000_0000,
    ];

    /// What the register form computes on the two extended operands.
    fn register_form(op: IrCmpOp, a: i64, b: i64) -> bool {
        let (ua, ub) = (a as u64, b as u64);
        match op {
            IrCmpOp::Eq => a == b,
            IrCmpOp::Ne => a != b,
            IrCmpOp::Slt => a < b,
            IrCmpOp::Sle => a <= b,
            IrCmpOp::Sgt => a > b,
            IrCmpOp::Sge => a >= b,
            IrCmpOp::Ult => ua < ub,
            IrCmpOp::Ule => ua <= ub,
            IrCmpOp::Ugt => ua > ub,
            IrCmpOp::Uge => ua >= ub,
        }
    }

    /// Execute the emitted I-type sequence on `t1`.
    fn execute(form: ImmCmp, t1: i64) -> bool {
        let fits = |k: i64| (-2048..=2047).contains(&k);
        match form {
            ImmCmp::Zero { eq } => (t1 == 0) == eq,
            ImmCmp::Positive => 0 < t1,
            ImmCmp::Xor { k, eq } => {
                assert!(fits(k), "xori immediate {k}");
                ((t1 ^ k) == 0) == eq
            }
            ImmCmp::Below {
                unsigned,
                k,
                negate,
            } => {
                assert!(fits(k), "slti/sltiu immediate {k}");
                let below = if unsigned {
                    (t1 as u64) < (k as u64)
                } else {
                    t1 < k
                };
                below != negate
            }
        }
    }

    /// Every immediate form, at every compare type, agrees with the
    /// register form on the operands the backend actually compares (both
    /// sides extended by the type), with the constant on either side.
    #[test]
    fn every_immediate_form_matches_the_register_compare() {
        let mut checked = 0usize;
        for ty in TYPES {
            for &c in &VALUES {
                let Some(v) = cmp_const_value(&IrConst::I64(c), ty) else {
                    panic!("{ty:?} is a modelled compare type");
                };
                for op in OPS {
                    for (form, on_rhs) in [
                        (imm_cmp_form(op, v), true),
                        (imm_cmp_form(op.swapped(), v), false),
                    ] {
                        assert_eq!(
                            form.is_some(),
                            cmp_needs_no_register(op, on_rhs, &IrConst::I64(c), ty),
                            "{op:?} {ty:?} {c} rhs={on_rhs}"
                        );
                        let Some(form) = form else { continue };
                        for &x in &VALUES {
                            let a = cmp_const_value(&IrConst::I64(x), ty).unwrap_or(x);
                            let want = if on_rhs {
                                register_form(op, a, v)
                            } else {
                                register_form(op, v, a)
                            };
                            assert_eq!(
                                execute(form, a),
                                want,
                                "{op:?} {ty:?}: x={x} c={c} rhs={on_rhs} form={form:?}"
                            );
                            checked += 1;
                        }
                    }
                }
            }
        }
        assert!(checked > 20_000, "{checked}");
    }

    /// The boundaries where only the register form is exact.
    #[test]
    fn the_register_form_remains_where_no_immediate_is_exact() {
        // `x <= 2047` would need `slti 2048`; `x <=u MAX` would wrap to 0.
        assert_eq!(imm_cmp_form(IrCmpOp::Sle, 2047), None);
        assert_eq!(imm_cmp_form(IrCmpOp::Sgt, 2047), None);
        assert_eq!(imm_cmp_form(IrCmpOp::Ule, -1), None);
        assert_eq!(imm_cmp_form(IrCmpOp::Ugt, -1), None);
        assert_eq!(imm_cmp_form(IrCmpOp::Sle, i64::MAX), None);
        // A U32 all-ones constant is 0xffffffff after zero-extension.
        assert_eq!(
            cmp_const_value(&IrConst::I32(-1), IrType::U32),
            Some(0xffff_ffff)
        );
        assert!(!cmp_needs_no_register(
            IrCmpOp::Ne,
            true,
            &IrConst::I32(-1),
            IrType::U32
        ));
        // Floats and 128-bit constants never take this path.
        assert_eq!(cmp_const_value(&IrConst::F64(1.0), IrType::F64), None);
        assert_eq!(cmp_const_value(&IrConst::I64(1), IrType::I128), None);
    }

    /// The forms the lexer loops reach: `c == ' '` is `xori` + `seqz`,
    /// `c - 'a' <= 25` is one `sltiu 26`, signed `x > 0` is one `slt`.
    #[test]
    fn the_common_shapes_take_one_or_two_instructions() {
        assert_eq!(
            imm_cmp_form(IrCmpOp::Eq, 32),
            Some(ImmCmp::Xor { k: 32, eq: true })
        );
        assert_eq!(
            imm_cmp_form(IrCmpOp::Ule, 25),
            Some(ImmCmp::Below {
                unsigned: true,
                k: 26,
                negate: false
            })
        );
        assert_eq!(imm_cmp_form(IrCmpOp::Sgt, 0), Some(ImmCmp::Positive));
        assert_eq!(
            imm_cmp_form(IrCmpOp::Ne, 0),
            Some(ImmCmp::Zero { eq: false })
        );
    }
}
