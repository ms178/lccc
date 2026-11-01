//! Floating-point classification builtins for the lowerer.
//!
//! Handles __builtin_fpclassify, __builtin_isnan, __builtin_isinf,
//! __builtin_isfinite, __builtin_isnormal, __builtin_signbit, and
//! __builtin_isinf_sign. Uses bit manipulation for F32/F64 and
//! delegates to libc for F128 (long double).

use super::lower::Lowerer;
use crate::common::types::{AddressSpace, IrType};
use crate::frontend::parser::ast::Expr;
use crate::ir::reexports::{CallInfo, Instruction, IrBinOp, IrCmpOp, IrConst, Operand, Value};

/// IEEE 754 binary128 field geometry as seen through the HIGH 64-bit word:
/// bit 63 is the sign, bits 62..48 are the 15-bit exponent, bits 47..0 are the
/// top 48 mantissa bits. The remaining 64 mantissa bits live in the low word,
/// so every field test below is 64-bit arithmetic -- no 128-bit operation, no
/// memory round trip, no endianness assumption (the two words are produced by an
/// IR-level shift and truncation, not by reinterpreting stored bytes).
/// A binary128 object is 16 bytes, which is also its natural alignment.
const SIZE_OF_BINARY128: usize = 16;
const BINARY128_EXP_FIELD: i64 = 0x7FFF_0000_0000_0000;
const BINARY128_MANT_HI: i64 = 0x0000_FFFF_FFFF_FFFF;

/// The field tests every binary128 classification is built from, each either 0
/// or 1 in an `I32`.
struct Binary128Fields {
    /// Exponent field is all ones: the value is a NaN or an infinity.
    exp_is_max: Value,
    /// Exponent field is zero: the value is a zero or a subnormal.
    exp_is_zero: Value,
    /// Mantissa field is zero: combined with an all-ones exponent this is an
    /// infinity, combined with a zero exponent it is a zero.
    mant_is_zero: Value,
    /// The sign bit as 0 or 1 (not the C truth value 0 or -1).
    sign: Value,
}

impl Lowerer {
    /// Helper: get the type, CType, and lowered value of the last argument for fp classification builtins.
    fn lower_fp_classify_arg(
        &mut self,
        args: &[Expr],
    ) -> (IrType, crate::common::types::CType, Operand) {
        let arg_expr = args
            .last()
            .expect("fp classify builtin requires at least one argument");
        let arg_ct = self.expr_ctype(arg_expr);
        let arg_ty = self.get_expr_type(arg_expr);
        let arg_val = self.lower_expr(arg_expr);
        (arg_ty, arg_ct, arg_val)
    }

    /// Emit a call to a libc FP classification function for F128 (long double) arguments.
    /// Returns the call result as a Value. The libc functions take a long double and return int.
    fn emit_f128_classify_libcall(&mut self, func_name: &str, arg_val: Operand) -> Value {
        let dest = self.fresh_value();
        self.emit(Instruction::Call {
            func: func_name.to_string(),
            info: CallInfo {
                dest: Some(dest),
                args: vec![arg_val],
                arg_types: vec![IrType::F128],
                return_type: IrType::I32,
                is_variadic: false,
                num_fixed_args: 1,
                struct_arg_sizes: vec![None],
                struct_arg_aligns: vec![],
                struct_arg_classes: Vec::new(),
                struct_arg_riscv_float_classes: Vec::new(),
                struct_arg_is_f128_sse: Vec::new(),
                is_sret: false,
                is_fastcall: false,
                regparm: None,
                is_pure: false,
                is_const: false,
                assume_library: false,
                ret_eightbyte_classes: Vec::new(),
                ret_is_f128_sse: false,
                cold: false,
            },
        });
        dest
    }

    /// Convert a libc classification result to a boolean (non-zero -> 1, zero -> 0).
    fn classify_result_to_bool(&mut self, result: Value) -> Value {
        self.emit_cmp_val(
            IrCmpOp::Ne,
            Operand::Value(result),
            Operand::Const(IrConst::I32(0)),
            IrType::I32,
        )
    }

    /// Zero-extend a narrow value (I8 from Cmp, or I32 from BinOp) to I64.
    /// On 32-bit targets (i686), I64 values occupy two 32-bit stack slots;
    /// without this cast, loading a narrow value as I64 reads uninitialized
    /// bytes from the upper slot, causing incorrect results in I64 arithmetic.
    fn widen_to_i64(&mut self, val: Value) -> Value {
        self.emit_cast_val(Operand::Value(val), IrType::I32, IrType::I64)
    }

    /// Widen a comparison result (IR comparisons produce `I8`) to a 0/1 `I32`
    /// flag, so that flag arithmetic runs in a type the backend materialises
    /// with defined upper bits.
    fn bool32(&mut self, flag: Value) -> Value {
        self.emit_cast_val(Operand::Value(flag), IrType::I8, IrType::I32)
    }

    fn flag_not(&mut self, flag: Value) -> Value {
        self.emit_binop_val(
            IrBinOp::Xor,
            Operand::Value(flag),
            Operand::Const(IrConst::I32(1)),
            IrType::I32,
        )
    }

    fn flag_and(&mut self, lhs: Value, rhs: Value) -> Value {
        self.emit_binop_val(
            IrBinOp::And,
            Operand::Value(lhs),
            Operand::Value(rhs),
            IrType::I32,
        )
    }

    fn flag_or(&mut self, lhs: Value, rhs: Value) -> Value {
        self.emit_binop_val(
            IrBinOp::Or,
            Operand::Value(lhs),
            Operand::Value(rhs),
            IrType::I32,
        )
    }

    /// Whether this argument is classified by the libc `long double`
    /// classifiers (`__isnanl`, `__isinfl`, ...).
    ///
    /// `long double` and `_Float128` are both `IrType::F128` (16-byte storage),
    /// but they are DIFFERENT formats: `CType::LongDouble` is the target's own
    /// long double, which on x86-64 is the 80-bit x87 extended format stored in
    /// a 16-byte slot, while `CType::Float128` is always IEEE binary128. The
    /// libc classifiers match the target's long double and therefore only tell
    /// the truth for the former. Feeding them binary128 bits made
    /// `__builtin_isnan(nan)` return 0 and `__builtin_isfinite(inf)` return 1
    /// (measured against GCC 16.2); binary128 is classified from its bits
    /// instead, which is exact, libc-free and target-independent.
    fn uses_libc_long_double_classifier(
        arg_ty: IrType,
        arg_ct: &crate::common::types::CType,
    ) -> bool {
        arg_ty == IrType::F128 && !matches!(arg_ct, crate::common::types::CType::Float128)
    }

    /// Whether this argument is IEEE binary128, classified from its bits.
    ///
    /// Decided on the C type alone: `_Float128` reaches the IR in an integer
    /// carrier (`U128`, 16-byte bit-exact moves) in most positions, and only
    /// sometimes as `IrType::F128`, so an `arg_ty` test misses it.
    fn is_binary128(arg_ct: &crate::common::types::CType) -> bool {
        matches!(arg_ct, crate::common::types::CType::Float128)
    }

    /// The bit pattern of a value that is not carried as an integer, as an
    /// unsigned 128-bit carrier: the 16 bytes are stored and reloaded through a
    /// temporary, so no FP conversion can silently round them.
    fn bitcast_f128_to_u128(&mut self, val: Operand) -> Value {
        let tmp_alloca = self.fresh_value();
        self.emit(Instruction::Alloca {
            dest: tmp_alloca,
            size: SIZE_OF_BINARY128,
            ty: IrType::F128,
            align: SIZE_OF_BINARY128,
            volatile: false,
            semantic_volatile: false,
        });
        let val_v = self.operand_to_value(val);
        self.emit(Instruction::Store {
            volatile: false,
            val: Operand::Value(val_v),
            ptr: tmp_alloca,
            ty: IrType::F128,
            seg_override: AddressSpace::Default,
        });
        let result = self.fresh_value();
        self.emit(Instruction::Load {
            volatile: false,
            dest: result,
            ptr: tmp_alloca,
            ty: IrType::U128,
            seg_override: AddressSpace::Default,
        });
        result
    }

    /// Split a binary128 value into its high and low 64-bit words.
    ///
    /// The split is an IR-level shift and truncation of the 128-bit value, not a
    /// reinterpretation of stored bytes, so it is independent of the target's byte
    /// order and needs no aligned temporary when the value already lives in an
    /// integer carrier (which is how `_Float128` normally reaches the IR).
    fn binary128_halves(&mut self, arg_val: Operand, arg_ty: IrType) -> (Value, Value) {
        let bits = match arg_ty {
            // Already the raw bit pattern (or its signed twin): reuse it.
            IrType::U128 | IrType::I128 => arg_val,
            // A narrower float carrier: widen it to binary128 first, so the
            // 16-byte store below stores a real binary128 value.
            IrType::F32 | IrType::F64 => {
                let widened = self.emit_cast_val(arg_val, arg_ty, IrType::F128);
                Operand::Value(self.bitcast_f128_to_u128(Operand::Value(widened)))
            }
            _ => Operand::Value(self.bitcast_f128_to_u128(arg_val)),
        };
        let bits = self.operand_to_value(bits);
        let hi_wide = self.emit_binop_val(
            IrBinOp::LShr,
            Operand::Value(bits),
            Operand::Const(IrConst::I64(64)),
            IrType::U128,
        );
        let hi = self.emit_cast_val(Operand::Value(hi_wide), IrType::U128, IrType::I64);
        let lo = self.emit_cast_val(Operand::Value(bits), IrType::U128, IrType::I64);
        (hi, lo)
    }

    /// Split a binary128 value into the field tests every classifier needs.
    ///
    /// Six 64-bit operations per value ("is the exponent all ones", "is it zero",
    /// "is the mantissa zero", "is the sign set") answer all seven builtins, so a
    /// classification never touches 128-bit arithmetic, memory, or libc.
    fn binary128_fields(&mut self, arg_val: Operand, arg_ty: IrType) -> Binary128Fields {
        let (hi, lo) = self.binary128_halves(arg_val, arg_ty);
        // Sign and exponent share the top 16 bits of the high word: one mask
        // answers both exponent questions and leaves the sign bit untouched.
        let exponent = self.emit_binop_val(
            IrBinOp::And,
            Operand::Value(hi),
            Operand::Const(IrConst::I64(BINARY128_EXP_FIELD)),
            IrType::I64,
        );
        let exp_is_max = self.emit_cmp_val(
            IrCmpOp::Eq,
            Operand::Value(exponent),
            Operand::Const(IrConst::I64(BINARY128_EXP_FIELD)),
            IrType::I64,
        );
        let exp_is_zero = self.emit_cmp_val(
            IrCmpOp::Eq,
            Operand::Value(exponent),
            Operand::Const(IrConst::I64(0)),
            IrType::I64,
        );
        // The mantissa is zero iff both words are: fold the masked high word and
        // the low word into one value and compare it once.
        let mantissa_hi = self.emit_binop_val(
            IrBinOp::And,
            Operand::Value(hi),
            Operand::Const(IrConst::I64(BINARY128_MANT_HI)),
            IrType::I64,
        );
        let mantissa = self.emit_binop_val(
            IrBinOp::Or,
            Operand::Value(mantissa_hi),
            Operand::Value(lo),
            IrType::I64,
        );
        let mant_is_zero = self.emit_cmp_val(
            IrCmpOp::Eq,
            Operand::Value(mantissa),
            Operand::Const(IrConst::I64(0)),
            IrType::I64,
        );
        // A signed compare against zero reads the sign bit in one instruction.
        let sign = self.emit_cmp_val(
            IrCmpOp::Slt,
            Operand::Value(hi),
            Operand::Const(IrConst::I64(0)),
            IrType::I64,
        );
        let exp_is_max = self.bool32(exp_is_max);
        let exp_is_zero = self.bool32(exp_is_zero);
        let mant_is_zero = self.bool32(mant_is_zero);
        let sign = self.bool32(sign);
        Binary128Fields {
            exp_is_max,
            exp_is_zero,
            mant_is_zero,
            sign,
        }
    }

    /// `__builtin_fpclassify` class flags for binary128, in the order the
    /// builtin's arguments arrive: nan, infinity, normal, subnormal, zero.
    fn binary128_class_flags(&mut self, arg_val: Operand, arg_ty: IrType) -> [Value; 5] {
        let f = self.binary128_fields(arg_val, arg_ty);
        let mant_not_zero = self.flag_not(f.mant_is_zero);
        let exp_not_max = self.flag_not(f.exp_is_max);
        let exp_not_zero = self.flag_not(f.exp_is_zero);
        let is_nan = self.flag_and(f.exp_is_max, mant_not_zero);
        let is_inf = self.flag_and(f.exp_is_max, f.mant_is_zero);
        let is_zero = self.flag_and(f.exp_is_zero, f.mant_is_zero);
        let is_subnormal = self.flag_and(f.exp_is_zero, mant_not_zero);
        let is_normal = self.flag_and(exp_not_zero, exp_not_max);
        [is_nan, is_inf, is_normal, is_subnormal, is_zero]
    }

    /// Bitcast a float/double to its integer bit representation via memory.
    /// F32 -> I32, F64 -> I64. Uses alloca+store+load for a true bitcast.
    /// Note: F128 classification builtins use libc calls instead of this path.
    fn bitcast_float_to_int(
        &mut self,
        val: Operand,
        float_ty: IrType,
        arg_ct: &crate::common::types::CType,
    ) -> (Operand, IrType) {
        if arg_ct.is_half_float() {
            let zext = self.emit_cast_val(val, IrType::U16, IrType::I32);
            return (Operand::Value(zext), IrType::I32);
        }
        let (int_ty, store_ty, size) = match float_ty {
            IrType::F32 => (IrType::I32, IrType::F32, 4),
            // F128 is stored as F64 at backend level, treat identically
            _ => (IrType::I64, IrType::F64, 8),
        };

        let tmp_alloca = self.fresh_value();
        self.emit(Instruction::Alloca {
            dest: tmp_alloca,
            size,
            ty: store_ty,
            align: 0,
            volatile: false,
            semantic_volatile: false,
        });

        let val_v = self.operand_to_value(val);
        self.emit(Instruction::Store {
            volatile: false,
            val: Operand::Value(val_v),
            ptr: tmp_alloca,
            ty: store_ty,
            seg_override: AddressSpace::Default,
        });

        let result = self.fresh_value();
        self.emit(Instruction::Load {
            volatile: false,
            dest: result,
            ptr: tmp_alloca,
            ty: int_ty,
            seg_override: AddressSpace::Default,
        });

        (Operand::Value(result), int_ty)
    }

    /// Floating-point bit-layout constants for classification.
    /// Returns (abs_mask, exp_only, exp_shift, exp_field_max, mant_mask).
    /// Note: F128 classification builtins use libc calls instead of these masks.
    pub(super) fn fp_masks_for(
        float_ty: IrType,
        arg_ct: &crate::common::types::CType,
    ) -> (i64, i64, i64, i64, i64) {
        use crate::common::types::CType;
        match arg_ct {
            CType::Float16 => (0x7FFF, 0x7C00, 10, 0x1F, 0x03FF),
            CType::BFloat16 => (0x7FFF, 0x7F80, 7, 0xFF, 0x007F),
            _ => match float_ty {
                IrType::F32 => (0x7FFFFFFF_i64, 0x7F800000_i64, 23, 0xFF, 0x007FFFFF_i64),
                _ => (
                    0x7FFFFFFFFFFFFFFF_i64,
                    0x7FF0000000000000_u64 as i64,
                    52,
                    0x7FF,
                    0x000FFFFFFFFFFFFF_u64 as i64,
                ),
            },
        }
    }

    fn fp_sign_shift(float_ty: IrType, arg_ct: &crate::common::types::CType) -> i64 {
        if arg_ct.is_half_float() {
            15
        } else if float_ty == IrType::F32 {
            31
        } else {
            63
        }
    }

    /// Compute the absolute value (sign-stripped) of float bits: bits & abs_mask.
    fn fp_abs_bits(&mut self, bits: &Operand, int_ty: IrType, abs_mask: i64) -> Value {
        self.emit_binop_val(
            IrBinOp::And,
            *bits,
            Operand::Const(IrConst::I64(abs_mask)),
            int_ty,
        )
    }

    /// Lower __builtin_fpclassify(nan_val, inf_val, norm_val, subnorm_val, zero_val, x).
    /// Uses arithmetic select: result = sum of (class_val[i] * is_class[i]).
    /// For F128 (long double), calls __fpclassifyl from libc and maps the result
    /// to the user-provided class values via comparisons.
    pub(super) fn lower_builtin_fpclassify(&mut self, args: &[Expr]) -> Option<Operand> {
        if args.len() < 6 {
            return Some(Operand::Const(IrConst::I64(0)));
        }

        let class_vals: Vec<_> = (0..5).map(|i| self.lower_expr(&args[i])).collect();
        let arg_ct = self.expr_ctype(&args[5]);
        let arg_ty = self.get_expr_type(&args[5]);
        let arg_val = self.lower_expr(&args[5]);

        if Self::is_binary128(&arg_ct) {
            // Same argument order as the integer paths below.
            let flags = self.binary128_class_flags(arg_val, arg_ty);
            let widened: Vec<Value> = flags.iter().map(|&flag| self.widen_to_i64(flag)).collect();
            let mut result = self.emit_binop_val(
                IrBinOp::Mul,
                class_vals[0],
                Operand::Value(widened[0]),
                IrType::I64,
            );
            for idx in 1..5 {
                let contrib = self.emit_binop_val(
                    IrBinOp::Mul,
                    class_vals[idx],
                    Operand::Value(widened[idx]),
                    IrType::I64,
                );
                result = self.emit_binop_val(
                    IrBinOp::Add,
                    Operand::Value(result),
                    Operand::Value(contrib),
                    IrType::I64,
                );
            }
            return Some(Operand::Value(result));
        }
        if Self::uses_libc_long_double_classifier(arg_ty, &arg_ct) {
            // TODO: FP_* constants assume glibc/Linux; may differ on musl/BSD
            // __fpclassifyl returns: FP_NAN=0, FP_INFINITE=1, FP_ZERO=2, FP_SUBNORMAL=3, FP_NORMAL=4
            let cls = self.emit_f128_classify_libcall("__fpclassifyl", arg_val);
            // Map libc class to user-provided values:
            // class_vals[0]=nan, [1]=inf, [2]=normal, [3]=subnormal, [4]=zero
            // libc: 0=nan, 1=inf, 2=zero, 3=subnormal, 4=normal
            // So: libc_0->class_vals[0], libc_1->class_vals[1], libc_2->class_vals[4],
            //     libc_3->class_vals[3], libc_4->class_vals[2]
            let libc_to_user = [0usize, 1, 4, 3, 2]; // libc class index -> user class_vals index
            // Comparison results (I8) must be widened to I64 before I64 multiply,
            // because on 32-bit targets I64 occupies two stack slots and loading
            // a narrow value as I64 reads uninitialized upper bytes.
            let is_class: Vec<Value> = (0..5)
                .map(|libc_cls| {
                    let cmp = self.emit_cmp_val(
                        IrCmpOp::Eq,
                        Operand::Value(cls),
                        Operand::Const(IrConst::I32(libc_cls)),
                        IrType::I32,
                    );
                    self.widen_to_i64(cmp)
                })
                .collect();

            let mut result = self.emit_binop_val(
                IrBinOp::Mul,
                class_vals[libc_to_user[0]],
                Operand::Value(is_class[0]),
                IrType::I64,
            );
            for libc_cls in 1..5 {
                let user_idx = libc_to_user[libc_cls];
                let contrib = self.emit_binop_val(
                    IrBinOp::Mul,
                    class_vals[user_idx],
                    Operand::Value(is_class[libc_cls]),
                    IrType::I64,
                );
                result = self.emit_binop_val(
                    IrBinOp::Add,
                    Operand::Value(result),
                    Operand::Value(contrib),
                    IrType::I64,
                );
            }
            return Some(Operand::Value(result));
        }

        let (bits, int_ty) = self.bitcast_float_to_int(arg_val, arg_ty, &arg_ct);
        let (_abs_mask, _exp_only, exp_shift, exp_field_max, mant_mask) =
            Self::fp_masks_for(arg_ty, &arg_ct);

        let shifted = self.emit_binop_val(
            IrBinOp::LShr,
            bits,
            Operand::Const(IrConst::I64(exp_shift)),
            int_ty,
        );
        let exponent = self.emit_binop_val(
            IrBinOp::And,
            Operand::Value(shifted),
            Operand::Const(IrConst::I64(exp_field_max)),
            int_ty,
        );
        let mantissa = self.emit_binop_val(
            IrBinOp::And,
            bits,
            Operand::Const(IrConst::I64(mant_mask)),
            int_ty,
        );

        let exp_is_max = self.emit_cmp_val(
            IrCmpOp::Eq,
            Operand::Value(exponent),
            Operand::Const(IrConst::I64(exp_field_max)),
            int_ty,
        );
        let exp_is_zero = self.emit_cmp_val(
            IrCmpOp::Eq,
            Operand::Value(exponent),
            Operand::Const(IrConst::I64(0)),
            int_ty,
        );
        let mant_is_zero = self.emit_cmp_val(
            IrCmpOp::Eq,
            Operand::Value(mantissa),
            Operand::Const(IrConst::I64(0)),
            int_ty,
        );
        let mant_not_zero = self.emit_cmp_val(
            IrCmpOp::Ne,
            Operand::Value(mantissa),
            Operand::Const(IrConst::I64(0)),
            int_ty,
        );

        let is_nan = self.emit_binop_val(
            IrBinOp::And,
            Operand::Value(exp_is_max),
            Operand::Value(mant_not_zero),
            IrType::I32,
        );
        let is_inf = self.emit_binop_val(
            IrBinOp::And,
            Operand::Value(exp_is_max),
            Operand::Value(mant_is_zero),
            IrType::I32,
        );
        let is_zero = self.emit_binop_val(
            IrBinOp::And,
            Operand::Value(exp_is_zero),
            Operand::Value(mant_is_zero),
            IrType::I32,
        );
        let is_subnorm = self.emit_binop_val(
            IrBinOp::And,
            Operand::Value(exp_is_zero),
            Operand::Value(mant_not_zero),
            IrType::I32,
        );

        let mut special = is_nan;
        for &flag in &[is_inf, is_zero, is_subnorm] {
            special = self.emit_binop_val(
                IrBinOp::Or,
                Operand::Value(special),
                Operand::Value(flag),
                IrType::I32,
            );
        }
        let is_normal = self.emit_binop_val(
            IrBinOp::Xor,
            Operand::Value(special),
            Operand::Const(IrConst::I64(1)),
            IrType::I32,
        );

        // Order: nan=0, inf=1, normal=2, subnormal=3, zero=4
        // Widen I32 flags to I64 before multiply on 32-bit targets.
        let class_flags = [is_nan, is_inf, is_normal, is_subnorm, is_zero];
        let contribs: Vec<_> = class_vals
            .iter()
            .zip(class_flags.iter())
            .map(|(val, &flag)| (*val, self.widen_to_i64(flag)))
            .collect();
        let mut result = self.emit_binop_val(
            IrBinOp::Mul,
            contribs[0].0,
            Operand::Value(contribs[0].1),
            IrType::I64,
        );
        for &(ref val, flag) in &contribs[1..] {
            let contrib =
                self.emit_binop_val(IrBinOp::Mul, *val, Operand::Value(flag), IrType::I64);
            result = self.emit_binop_val(
                IrBinOp::Add,
                Operand::Value(result),
                Operand::Value(contrib),
                IrType::I64,
            );
        }

        Some(Operand::Value(result))
    }

    /// __builtin_isnan(x) -> 1 if x is NaN. NaN: abs(bits) > exp_only.
    /// For F128 (long double), delegates to libc __isnanl for correct 80-bit/128-bit handling.
    pub(super) fn lower_builtin_isnan(&mut self, args: &[Expr]) -> Option<Operand> {
        if args.is_empty() {
            return Some(Operand::Const(IrConst::I64(0)));
        }
        let (arg_ty, arg_ct, arg_val) = self.lower_fp_classify_arg(args);
        if Self::is_binary128(&arg_ct) {
            let f = self.binary128_fields(arg_val, arg_ty);
            let mant_not_zero = self.flag_not(f.mant_is_zero);
            return Some(Operand::Value(self.flag_and(f.exp_is_max, mant_not_zero)));
        }
        if Self::uses_libc_long_double_classifier(arg_ty, &arg_ct) {
            let result = self.emit_f128_classify_libcall("__isnanl", arg_val);
            let bool_result = self.classify_result_to_bool(result);
            return Some(Operand::Value(bool_result));
        }
        let (bits, int_ty) = self.bitcast_float_to_int(arg_val, arg_ty, &arg_ct);
        let masks = Self::fp_masks_for(arg_ty, &arg_ct);
        let abs_val = self.fp_abs_bits(&bits, int_ty, masks.0);
        let result = self.emit_cmp_val(
            IrCmpOp::Ugt,
            Operand::Value(abs_val),
            Operand::Const(IrConst::I64(masks.1)),
            int_ty,
        );
        Some(Operand::Value(result))
    }

    /// __builtin_isinf(x) -> nonzero if +/-infinity. Inf: abs(bits) == exp_only.
    /// For F128 (long double), delegates to libc __isinfl.
    pub(super) fn lower_builtin_isinf(&mut self, args: &[Expr]) -> Option<Operand> {
        if args.is_empty() {
            return Some(Operand::Const(IrConst::I64(0)));
        }
        let (arg_ty, arg_ct, arg_val) = self.lower_fp_classify_arg(args);
        if Self::is_binary128(&arg_ct) {
            let f = self.binary128_fields(arg_val, arg_ty);
            return Some(Operand::Value(self.flag_and(f.exp_is_max, f.mant_is_zero)));
        }
        if Self::uses_libc_long_double_classifier(arg_ty, &arg_ct) {
            let result = self.emit_f128_classify_libcall("__isinfl", arg_val);
            let bool_result = self.classify_result_to_bool(result);
            return Some(Operand::Value(bool_result));
        }
        let (bits, int_ty) = self.bitcast_float_to_int(arg_val, arg_ty, &arg_ct);
        let masks = Self::fp_masks_for(arg_ty, &arg_ct);
        let abs_val = self.fp_abs_bits(&bits, int_ty, masks.0);
        let result = self.emit_cmp_val(
            IrCmpOp::Eq,
            Operand::Value(abs_val),
            Operand::Const(IrConst::I64(masks.1)),
            int_ty,
        );
        Some(Operand::Value(result))
    }

    /// __builtin_isfinite(x) -> 1 if finite. Finite: (bits & exp_only) != exp_only.
    /// For F128 (long double), delegates to libc __finitel.
    pub(super) fn lower_builtin_isfinite(&mut self, args: &[Expr]) -> Option<Operand> {
        if args.is_empty() {
            return Some(Operand::Const(IrConst::I64(0)));
        }
        let (arg_ty, arg_ct, arg_val) = self.lower_fp_classify_arg(args);
        if Self::is_binary128(&arg_ct) {
            let f = self.binary128_fields(arg_val, arg_ty);
            // Finite iff the exponent is not all ones: an all-ones exponent
            // is an infinity or a NaN whatever the mantissa says.
            return Some(Operand::Value(self.flag_not(f.exp_is_max)));
        }
        if Self::uses_libc_long_double_classifier(arg_ty, &arg_ct) {
            let result = self.emit_f128_classify_libcall("__finitel", arg_val);
            let bool_result = self.classify_result_to_bool(result);
            return Some(Operand::Value(bool_result));
        }
        let (bits, int_ty) = self.bitcast_float_to_int(arg_val, arg_ty, &arg_ct);
        let masks = Self::fp_masks_for(arg_ty, &arg_ct);
        let exp_bits = self.emit_binop_val(
            IrBinOp::And,
            bits,
            Operand::Const(IrConst::I64(masks.1)),
            int_ty,
        );
        let result = self.emit_cmp_val(
            IrCmpOp::Ne,
            Operand::Value(exp_bits),
            Operand::Const(IrConst::I64(masks.1)),
            int_ty,
        );
        Some(Operand::Value(result))
    }

    /// __builtin_isnormal(x) -> 1 if normal. Normal: exp != 0 AND exp != max.
    /// For F128 (long double), delegates to libc __fpclassifyl and compares to FP_NORMAL.
    pub(super) fn lower_builtin_isnormal(&mut self, args: &[Expr]) -> Option<Operand> {
        if args.is_empty() {
            return Some(Operand::Const(IrConst::I64(0)));
        }
        let (arg_ty, arg_ct, arg_val) = self.lower_fp_classify_arg(args);
        if Self::is_binary128(&arg_ct) {
            let f = self.binary128_fields(arg_val, arg_ty);
            let exp_not_zero = self.flag_not(f.exp_is_zero);
            let exp_not_max = self.flag_not(f.exp_is_max);
            return Some(Operand::Value(self.flag_and(exp_not_zero, exp_not_max)));
        }
        if Self::uses_libc_long_double_classifier(arg_ty, &arg_ct) {
            // TODO: FP_NORMAL = 4 assumes glibc/Linux FP_* constants; may differ on musl/BSD
            let cls = self.emit_f128_classify_libcall("__fpclassifyl", arg_val);
            let result = self.emit_cmp_val(
                IrCmpOp::Eq,
                Operand::Value(cls),
                Operand::Const(IrConst::I32(4)),
                IrType::I32,
            );
            return Some(Operand::Value(result));
        }
        let (bits, int_ty) = self.bitcast_float_to_int(arg_val, arg_ty, &arg_ct);
        let masks = Self::fp_masks_for(arg_ty, &arg_ct);
        let exp_bits = self.emit_binop_val(
            IrBinOp::And,
            bits,
            Operand::Const(IrConst::I64(masks.1)),
            int_ty,
        );
        let exponent = self.emit_binop_val(
            IrBinOp::LShr,
            Operand::Value(exp_bits),
            Operand::Const(IrConst::I64(masks.2)),
            int_ty,
        );
        let exp_nonzero = self.emit_cmp_val(
            IrCmpOp::Ne,
            Operand::Value(exponent),
            Operand::Const(IrConst::I64(0)),
            int_ty,
        );
        let exp_not_max = self.emit_cmp_val(
            IrCmpOp::Ne,
            Operand::Value(exponent),
            Operand::Const(IrConst::I64(masks.3)),
            int_ty,
        );
        let result = self.emit_binop_val(
            IrBinOp::And,
            Operand::Value(exp_nonzero),
            Operand::Value(exp_not_max),
            IrType::I32,
        );
        Some(Operand::Value(result))
    }

    /// __builtin_signbit(x) -> nonzero if sign bit set.
    /// For F128 (long double), delegates to libc __signbitl.
    pub(super) fn lower_builtin_signbit(&mut self, args: &[Expr]) -> Option<Operand> {
        if args.is_empty() {
            return Some(Operand::Const(IrConst::I64(0)));
        }
        let (arg_ty, arg_ct, arg_val) = self.lower_fp_classify_arg(args);
        if Self::is_binary128(&arg_ct) {
            let f = self.binary128_fields(arg_val, arg_ty);
            return Some(Operand::Value(f.sign));
        }
        if Self::uses_libc_long_double_classifier(arg_ty, &arg_ct) {
            let result = self.emit_f128_classify_libcall("__signbitl", arg_val);
            let bool_result = self.classify_result_to_bool(result);
            return Some(Operand::Value(bool_result));
        }
        let (bits, int_ty) = self.bitcast_float_to_int(arg_val, arg_ty, &arg_ct);
        let sign_shift = Self::fp_sign_shift(arg_ty, &arg_ct);
        let shifted = self.emit_binop_val(
            IrBinOp::LShr,
            bits,
            Operand::Const(IrConst::I64(sign_shift)),
            int_ty,
        );
        let result = self.emit_binop_val(
            IrBinOp::And,
            Operand::Value(shifted),
            Operand::Const(IrConst::I64(1)),
            int_ty,
        );
        Some(Operand::Value(result))
    }

    /// __builtin_isinf_sign(x) -> -1 if -inf, +1 if +inf, 0 otherwise.
    /// For F128 (long double), delegates to libc __isinfl and __signbitl.
    pub(super) fn lower_builtin_isinf_sign(&mut self, args: &[Expr]) -> Option<Operand> {
        if args.is_empty() {
            return Some(Operand::Const(IrConst::I64(0)));
        }
        let (arg_ty, arg_ct, arg_val) = self.lower_fp_classify_arg(args);
        if Self::is_binary128(&arg_ct) {
            let f = self.binary128_fields(arg_val, arg_ty);
            let is_inf = self.flag_and(f.exp_is_max, f.mant_is_zero);
            // 1 - 2 * sign
            let two_sign = self.emit_binop_val(
                IrBinOp::Mul,
                Operand::Value(f.sign),
                Operand::Const(IrConst::I32(2)),
                IrType::I32,
            );
            let direction = self.emit_binop_val(
                IrBinOp::Sub,
                Operand::Const(IrConst::I32(1)),
                Operand::Value(two_sign),
                IrType::I32,
            );
            let result = self.emit_binop_val(
                IrBinOp::Mul,
                Operand::Value(is_inf),
                Operand::Value(direction),
                IrType::I32,
            );
            return Some(Operand::Value(result));
        }
        if Self::uses_libc_long_double_classifier(arg_ty, &arg_ct) {
            // __isinfl returns nonzero if inf
            let isinf_result = self.emit_f128_classify_libcall("__isinfl", arg_val);
            let is_inf_narrow = self.classify_result_to_bool(isinf_result);
            // __signbitl returns nonzero if negative
            let signbit_result = self.emit_f128_classify_libcall("__signbitl", arg_val);
            let is_neg_narrow = self.classify_result_to_bool(signbit_result);
            let is_inf = self.widen_to_i64(is_inf_narrow);
            let is_neg = self.widen_to_i64(is_neg_narrow);
            let neg_x2 = self.emit_binop_val(
                IrBinOp::Mul,
                Operand::Value(is_neg),
                Operand::Const(IrConst::I64(2)),
                IrType::I64,
            );
            let direction = self.emit_binop_val(
                IrBinOp::Sub,
                Operand::Const(IrConst::I64(1)),
                Operand::Value(neg_x2),
                IrType::I64,
            );
            let result = self.emit_binop_val(
                IrBinOp::Mul,
                Operand::Value(is_inf),
                Operand::Value(direction),
                IrType::I64,
            );
            return Some(Operand::Value(result));
        }
        let (bits, int_ty) = self.bitcast_float_to_int(arg_val, arg_ty, &arg_ct);
        let masks = Self::fp_masks_for(arg_ty, &arg_ct);
        let sign_shift = Self::fp_sign_shift(arg_ty, &arg_ct);
        let abs_val = self.fp_abs_bits(&bits, int_ty, masks.0);
        let is_inf_narrow = self.emit_cmp_val(
            IrCmpOp::Eq,
            Operand::Value(abs_val),
            Operand::Const(IrConst::I64(masks.1)),
            int_ty,
        );
        let is_inf = self.widen_to_i64(is_inf_narrow);
        let sign_shifted = self.emit_binop_val(
            IrBinOp::LShr,
            bits,
            Operand::Const(IrConst::I64(sign_shift)),
            int_ty,
        );
        let sign_bit_raw = self.emit_binop_val(
            IrBinOp::And,
            Operand::Value(sign_shifted),
            Operand::Const(IrConst::I64(1)),
            int_ty,
        );
        let sign_bit = if int_ty == IrType::I32 {
            self.widen_to_i64(sign_bit_raw)
        } else {
            sign_bit_raw
        };
        let sign_x2 = self.emit_binop_val(
            IrBinOp::Mul,
            Operand::Value(sign_bit),
            Operand::Const(IrConst::I64(2)),
            IrType::I64,
        );
        let direction = self.emit_binop_val(
            IrBinOp::Sub,
            Operand::Const(IrConst::I64(1)),
            Operand::Value(sign_x2),
            IrType::I64,
        );
        let result = self.emit_binop_val(
            IrBinOp::Mul,
            Operand::Value(is_inf),
            Operand::Value(direction),
            IrType::I64,
        );
        Some(Operand::Value(result))
    }
}
