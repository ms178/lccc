//! Overflow-checking arithmetic builtins for the lowerer.
//!
//! Handles __builtin_{s,u}{add,sub,mul}{,l,ll}_overflow and the generic
//! __builtin_{add,sub,mul}_overflow variants. These builtins perform an
//! arithmetic operation, store the result through a pointer, and return
//! 1 (bool true) if the operation overflowed.
//!
//! Also handles __builtin_{add,sub,mul}_overflow_p(a, b, (T)0) predicate-only
//! variants (GCC 7+) which return 1 if the operation would overflow type T,
//! without storing the result.

use super::lower::Lowerer;
use crate::common::types::{AddressSpace, CType, IrType, target_int_ir_type, target_is_32bit};
use crate::frontend::parser::ast::Expr;
use crate::ir::reexports::{Instruction, IrBinOp, IrCmpOp, IrConst, Operand, Value};

impl Lowerer {
    /// Lower __builtin_{add,sub,mul}_overflow(a, b, result_ptr) and type-specific variants.
    ///
    /// These builtins perform an arithmetic operation, store the result through
    /// the pointer, and return 1 (bool true) if the operation overflowed.
    pub(super) fn lower_overflow_builtin(
        &mut self,
        name: &str,
        args: &[Expr],
        op: IrBinOp,
    ) -> Option<Operand> {
        if args.len() < 3 {
            return Some(Operand::Const(IrConst::I64(0)));
        }

        // Determine the result type from the builtin name or from the pointer argument type.
        // Type-specific variants: __builtin_sadd_overflow (signed int), __builtin_uaddll_overflow (unsigned long long)
        // Generic variants: __builtin_add_overflow - type from pointer arg
        let (result_ir_ty, is_signed) = self.overflow_result_type(name, &args[2]);

        // Detect generic variants: __builtin_{add,sub,mul}_overflow
        // (as opposed to type-specific like __builtin_sadd_overflow, __builtin_uaddll_overflow)
        let is_generic = name == "__builtin_add_overflow"
            || name == "__builtin_sub_overflow"
            || name == "__builtin_mul_overflow";

        // Lower the two operands
        let lhs_raw = self.lower_expr(&args[0]);
        let rhs_raw = self.lower_expr(&args[1]);

        // For generic variants, operands may have different types than the result.
        // We need to perform the computation in a type wide enough to hold all
        // possible values from both operand types, then check if the result fits
        // in the target result type.
        let lhs_src_ctype = self.expr_ctype(&args[0]);
        let rhs_src_ctype = self.expr_ctype(&args[1]);

        // Get the result pointer
        let result_ptr_val = self.lower_expr(&args[2]);
        let result_ptr = self.operand_to_value(result_ptr_val);

        if is_generic {
            let lhs_src_ir = IrType::from_ctype(&lhs_src_ctype);
            let rhs_src_ir = IrType::from_ctype(&rhs_src_ctype);

            // Determine a computation type wide enough to hold the exact mathematical
            // result. We need a signed type that can represent all values of both
            // operands without loss.
            let max_src_size = lhs_src_ir
                .size()
                .max(rhs_src_ir.size())
                .max(result_ir_ty.size());
            // If any source is unsigned and as wide as max_src_size, we need one
            // extra width level to avoid losing the unsigned range.
            let needs_wider = (!lhs_src_ctype.is_signed() && lhs_src_ir.size() >= max_src_size)
                || (!rhs_src_ctype.is_signed() && rhs_src_ir.size() >= max_src_size);
            let compute_size = if needs_wider {
                max_src_size * 2
            } else {
                max_src_size
            };
            let compute_size = compute_size.max(result_ir_ty.size());
            // Use a signed compute type so we can detect negative results
            let compute_ty = match compute_size {
                1 => IrType::I16, // widen 8-bit to at least 16
                2 => IrType::I32,
                4 => IrType::I64,
                8 => IrType::I64,
                _ => IrType::I128,
            };

            if Self::overflow_needs_u64_pair(
                op,
                compute_ty,
                &[lhs_src_ir, rhs_src_ir, result_ir_ty],
            ) {
                let (truncated, overflow) = self.lower_overflow_u64_pair(
                    op,
                    (lhs_raw, lhs_src_ir),
                    (rhs_raw, rhs_src_ir),
                    result_ir_ty,
                );
                self.emit(Instruction::Store {
                    volatile: false,
                    val: Operand::Value(truncated),
                    ptr: result_ptr,
                    ty: result_ir_ty,
                    seg_override: AddressSpace::Default,
                });
                return Some(Operand::Value(overflow));
            }

            // Check if all operands can be represented exactly in compute_ty.
            // If an unsigned operand is the same size as compute_ty, it can't be
            // represented in the signed compute type (values > SIGNED_MAX are lost).
            let all_operands_fit = (lhs_src_ctype.is_signed()
                || lhs_src_ir.size() < compute_ty.size())
                && (rhs_src_ctype.is_signed() || rhs_src_ir.size() < compute_ty.size());

            // True widening: compute_ty is strictly wider than result_ir_ty AND
            // all operands fit in compute_ty, so the computation is exact.
            if all_operands_fit && compute_ty.size() > result_ir_ty.size() {
                // Cast operands to the compute type respecting their signedness
                let lhs_compute = if lhs_src_ir != compute_ty {
                    Operand::Value(self.emit_cast_val(lhs_raw, lhs_src_ir, compute_ty))
                } else {
                    lhs_raw
                };
                let rhs_compute = if rhs_src_ir != compute_ty {
                    Operand::Value(self.emit_cast_val(rhs_raw, rhs_src_ir, compute_ty))
                } else {
                    rhs_raw
                };

                // Compute in the wide type, truncate, and check whether the
                // truncated result round-trips back to the wide value.
                let wide_result = self.emit_binop_val(op, lhs_compute, rhs_compute, compute_ty);

                let truncated =
                    self.emit_cast_val(Operand::Value(wide_result), compute_ty, result_ir_ty);

                let extended_back =
                    self.emit_cast_val(Operand::Value(truncated), result_ir_ty, compute_ty);
                let mut overflow = self.emit_cmp_val(
                    IrCmpOp::Ne,
                    Operand::Value(wide_result),
                    Operand::Value(extended_back),
                    compute_ty,
                );

                // The round-trip test is only a complete overflow test when the
                // operation itself cannot wrap in `compute_ty`. `compute_ty` is
                // strictly wider than the RESULT, but not necessarily wider
                // than the OPERANDS: `__builtin_mul_overflow(int64, int64,
                // &uint32)` computes in I64, where 10 * (INT64_MIN + 1) wraps
                // to 10, which round-trips through uint32 and reported "no
                // overflow" (stress lab builtins seed 5 case 19; GCC/Clang
                // report 1 and store 10). Exactness, in signed bits: a signed
                // n-bit operand needs n bits, an unsigned one n+1; a sum or
                // difference needs one bit more than the wider operand, a
                // product the sum of both. When the compute type is not that
                // wide, OR in the compute type's own signed-overflow flag
                // (imul/add OF via compute_signed_overflow), which is cheaper
                // than re-widening to I128.
                let signed_bits = |ir: IrType, signed: bool| ir.size() * 8 + usize::from(!signed);
                let lb = signed_bits(lhs_src_ir, lhs_src_ctype.is_signed());
                let rb = signed_bits(rhs_src_ir, rhs_src_ctype.is_signed());
                let cb = compute_ty.size() * 8;
                let exact = if op == IrBinOp::Mul {
                    cb >= lb + rb
                } else {
                    cb > lb.max(rb)
                };
                if !exact {
                    let signed_ov = self.compute_signed_overflow(
                        op,
                        lhs_compute,
                        rhs_compute,
                        wide_result,
                        compute_ty,
                    );
                    overflow = self.emit_binop_val(
                        IrBinOp::Or,
                        Operand::Value(overflow),
                        Operand::Value(signed_ov),
                        IrType::I32,
                    );
                }

                // Store the truncated result
                self.emit(Instruction::Store {
                    volatile: false,
                    val: Operand::Value(truncated),
                    ptr: result_ptr,
                    ty: result_ir_ty,
                    seg_override: AddressSpace::Default,
                });

                Some(Operand::Value(overflow))
            } else if all_operands_fit
                && compute_ty.size() == result_ir_ty.size()
                && compute_ty.is_signed()
                && !is_signed
            {
                // Signed compute type, unsigned result type, same size.
                // All operands fit in the signed compute type, so we can compute
                // the exact mathematical result. Overflow iff the result is negative
                // (negative values can't fit in unsigned types).
                let lhs_compute = if lhs_src_ir != compute_ty {
                    Operand::Value(self.emit_cast_val(lhs_raw, lhs_src_ir, compute_ty))
                } else {
                    lhs_raw
                };
                let rhs_compute = if rhs_src_ir != compute_ty {
                    Operand::Value(self.emit_cast_val(rhs_raw, rhs_src_ir, compute_ty))
                } else {
                    rhs_raw
                };

                let signed_result = self.emit_binop_val(op, lhs_compute, rhs_compute, compute_ty);

                // Also check for signed overflow: if the signed operation itself overflows,
                // then we can't trust the result sign. Use signed overflow detection too.
                let signed_ov = self.compute_signed_overflow(
                    op,
                    lhs_compute,
                    rhs_compute,
                    signed_result,
                    compute_ty,
                );

                // Check if result is negative (doesn't fit in unsigned)
                let is_negative = self.emit_cmp_val(
                    IrCmpOp::Slt,
                    Operand::Value(signed_result),
                    Operand::Const(IrConst::I64(0)),
                    compute_ty,
                );

                // Overflow if result is negative OR if the signed operation overflowed
                let overflow = self.emit_binop_val(
                    IrBinOp::Or,
                    Operand::Value(is_negative),
                    Operand::Value(signed_ov),
                    IrType::I32,
                );

                // Store the result (reinterpreted as unsigned)
                let unsigned_result =
                    self.emit_cast_val(Operand::Value(signed_result), compute_ty, result_ir_ty);
                self.emit(Instruction::Store {
                    volatile: false,
                    val: Operand::Value(unsigned_result),
                    ptr: result_ptr,
                    ty: result_ir_ty,
                    seg_override: AddressSpace::Default,
                });

                Some(Operand::Value(overflow))
            } else {
                // Cannot widen, or widening wouldn't help (e.g., u128 operand can't fit in I128).
                // Fall back to computing directly in the result type with same-type overflow detection.
                let lhs_result = if lhs_src_ir != result_ir_ty {
                    Operand::Value(self.emit_cast_val(lhs_raw, lhs_src_ir, result_ir_ty))
                } else {
                    lhs_raw
                };
                let rhs_result = if rhs_src_ir != result_ir_ty {
                    Operand::Value(self.emit_cast_val(rhs_raw, rhs_src_ir, result_ir_ty))
                } else {
                    rhs_raw
                };

                let result = self.emit_binop_val(op, lhs_result, rhs_result, result_ir_ty);

                let overflow = if is_signed {
                    self.compute_signed_overflow(op, lhs_result, rhs_result, result, result_ir_ty)
                } else {
                    self.compute_unsigned_overflow(op, lhs_result, rhs_result, result, result_ir_ty)
                };

                self.emit(Instruction::Store {
                    volatile: false,
                    val: Operand::Value(result),
                    ptr: result_ptr,
                    ty: result_ir_ty,
                    seg_override: AddressSpace::Default,
                });

                Some(Operand::Value(overflow))
            }
        } else {
            // Type-specific variants: operands already match the result type
            let (lhs_val, rhs_val) = (lhs_raw, rhs_raw);

            if Self::overflow_needs_u64_pair(op, result_ir_ty, &[result_ir_ty]) {
                let (truncated, overflow) = self.lower_overflow_u64_pair(
                    op,
                    (lhs_val, result_ir_ty),
                    (rhs_val, result_ir_ty),
                    result_ir_ty,
                );
                self.emit(Instruction::Store {
                    volatile: false,
                    val: Operand::Value(truncated),
                    ptr: result_ptr,
                    ty: result_ir_ty,
                    seg_override: AddressSpace::Default,
                });
                return Some(Operand::Value(overflow));
            }

            // Perform the operation in the result type
            let result = self.emit_binop_val(op, lhs_val, rhs_val, result_ir_ty);

            // Compute the overflow flag BEFORE storing the result, because the
            // output pointer may alias an input.
            let overflow = if is_signed {
                self.compute_signed_overflow(op, lhs_val, rhs_val, result, result_ir_ty)
            } else {
                self.compute_unsigned_overflow(op, lhs_val, rhs_val, result, result_ir_ty)
            };

            // Store the result
            self.emit(Instruction::Store {
                volatile: false,
                val: Operand::Value(result),
                ptr: result_ptr,
                ty: result_ir_ty,
                seg_override: AddressSpace::Default,
            });

            Some(Operand::Value(overflow))
        }
    }

    /// Lower __builtin_{add,sub,mul}_overflow_p(a, b, (T)0) predicate-only variants.
    ///
    /// These return 1 if the operation would overflow type T, 0 otherwise.
    /// The third argument is a value expression whose type determines T (the value itself is ignored).
    pub(super) fn lower_overflow_p_builtin(
        &mut self,
        args: &[Expr],
        op: IrBinOp,
    ) -> Option<Operand> {
        if args.len() < 3 {
            return Some(Operand::Const(IrConst::I64(0)));
        }

        // The third argument determines the result type (only its type matters, not its value).
        let result_ctype = self.expr_ctype(&args[2]);
        let result_ir_ty = IrType::from_ctype(&result_ctype);

        // Lower the two operands
        let lhs_raw = self.lower_expr(&args[0]);
        let rhs_raw = self.lower_expr(&args[1]);

        // Lower but discard the third argument (it may have side effects, though
        // in practice it's always a cast of 0)
        let _ = self.lower_expr(&args[2]);

        // Use the same wider-type computation as the non-_p generic variant:
        // compute in a type wide enough to hold the exact result, then check
        // if truncating to the result type loses information.
        let lhs_src_ctype = self.expr_ctype(&args[0]);
        let rhs_src_ctype = self.expr_ctype(&args[1]);
        let lhs_src_ir = IrType::from_ctype(&lhs_src_ctype);
        let rhs_src_ir = IrType::from_ctype(&rhs_src_ctype);

        let max_src_size = lhs_src_ir
            .size()
            .max(rhs_src_ir.size())
            .max(result_ir_ty.size());
        let needs_wider = (!lhs_src_ctype.is_signed() && lhs_src_ir.size() >= max_src_size)
            || (!rhs_src_ctype.is_signed() && rhs_src_ir.size() >= max_src_size);
        let compute_size = if needs_wider {
            max_src_size * 2
        } else {
            max_src_size
        };
        let compute_size = compute_size.max(result_ir_ty.size());
        let compute_ty = match compute_size {
            1 => IrType::I16,
            2 => IrType::I32,
            4 => IrType::I64,
            8 => IrType::I64,
            _ => IrType::I128,
        };

        if Self::overflow_needs_u64_pair(op, compute_ty, &[lhs_src_ir, rhs_src_ir, result_ir_ty]) {
            let (_, overflow) = self.lower_overflow_u64_pair(
                op,
                (lhs_raw, lhs_src_ir),
                (rhs_raw, rhs_src_ir),
                result_ir_ty,
            );
            return Some(Operand::Value(overflow));
        }

        let is_signed_result = result_ctype.is_signed();
        let all_operands_fit = (lhs_src_ctype.is_signed() || lhs_src_ir.size() < compute_ty.size())
            && (rhs_src_ctype.is_signed() || rhs_src_ir.size() < compute_ty.size());

        if all_operands_fit && compute_ty.size() > result_ir_ty.size() {
            // True widening: compute exactly, truncate, and check round-trip
            let lhs_compute = if lhs_src_ir != compute_ty {
                Operand::Value(self.emit_cast_val(lhs_raw, lhs_src_ir, compute_ty))
            } else {
                lhs_raw
            };
            let rhs_compute = if rhs_src_ir != compute_ty {
                Operand::Value(self.emit_cast_val(rhs_raw, rhs_src_ir, compute_ty))
            } else {
                rhs_raw
            };

            let wide_result = self.emit_binop_val(op, lhs_compute, rhs_compute, compute_ty);
            let truncated =
                self.emit_cast_val(Operand::Value(wide_result), compute_ty, result_ir_ty);
            let extended_back =
                self.emit_cast_val(Operand::Value(truncated), result_ir_ty, compute_ty);
            let overflow = self.emit_cmp_val(
                IrCmpOp::Ne,
                Operand::Value(wide_result),
                Operand::Value(extended_back),
                compute_ty,
            );
            Some(Operand::Value(overflow))
        } else if all_operands_fit
            && compute_ty.size() == result_ir_ty.size()
            && compute_ty.is_signed()
            && !is_signed_result
        {
            // Signed compute, unsigned result, same size: overflow iff signed result < 0
            let lhs_compute = if lhs_src_ir != compute_ty {
                Operand::Value(self.emit_cast_val(lhs_raw, lhs_src_ir, compute_ty))
            } else {
                lhs_raw
            };
            let rhs_compute = if rhs_src_ir != compute_ty {
                Operand::Value(self.emit_cast_val(rhs_raw, rhs_src_ir, compute_ty))
            } else {
                rhs_raw
            };

            let signed_result = self.emit_binop_val(op, lhs_compute, rhs_compute, compute_ty);
            let signed_ov = self.compute_signed_overflow(
                op,
                lhs_compute,
                rhs_compute,
                signed_result,
                compute_ty,
            );
            let is_negative = self.emit_cmp_val(
                IrCmpOp::Slt,
                Operand::Value(signed_result),
                Operand::Const(IrConst::I64(0)),
                compute_ty,
            );
            let overflow = self.emit_binop_val(
                IrBinOp::Or,
                Operand::Value(is_negative),
                Operand::Value(signed_ov),
                IrType::I32,
            );
            Some(Operand::Value(overflow))
        } else {
            // Fall back to computing in the result type
            let lhs_result = if lhs_src_ir != result_ir_ty {
                Operand::Value(self.emit_cast_val(lhs_raw, lhs_src_ir, result_ir_ty))
            } else {
                lhs_raw
            };
            let rhs_result = if rhs_src_ir != result_ir_ty {
                Operand::Value(self.emit_cast_val(rhs_raw, rhs_src_ir, result_ir_ty))
            } else {
                rhs_raw
            };

            let result = self.emit_binop_val(op, lhs_result, rhs_result, result_ir_ty);
            let overflow = if is_signed_result {
                self.compute_signed_overflow(op, lhs_result, rhs_result, result, result_ir_ty)
            } else {
                self.compute_unsigned_overflow(op, lhs_result, rhs_result, result, result_ir_ty)
            };
            Some(Operand::Value(overflow))
        }
    }

    /// Whether an overflow builtin must take the 64-bit-pair lowering
    /// (`lower_overflow_u64_pair`) instead of the double-width one.
    ///
    /// The generic lowering computes in a type wide enough for the exact
    /// result, and `compute_{signed,unsigned}_overflow` widen a 64-bit
    /// multiply to I128.  On 32-bit targets I128 is not an arithmetic type:
    /// GCC has no `__int128` there and the i686 backend carries only the low
    /// 64 bits of it (its i128 -> narrow cast even hit the scalar-cast
    /// assertion).  So `__builtin_mul_overflow(i64, i64, &i64)` reported "no
    /// overflow" for 2^40 * 2^30, `__builtin_add_overflow(~0ULL, 1, &u64)`
    /// reported none either, and the u16-result forms ICEd (torture
    /// pr91450-1/-2, pr93494).  Exactly those shapes need 128 bits: a
    /// compute type wider than 64 bits, or a multiply that involves a
    /// 64-bit operand or result.  Everything else keeps the existing
    /// (cheaper) single-width lowering.  Operand or result types wider than
    /// 64 bits are out of scope here (they are I128 by construction).
    fn overflow_needs_u64_pair(op: IrBinOp, compute_ty: IrType, types: &[IrType]) -> bool {
        target_is_32bit()
            && types.iter().all(|t| t.size() <= 8)
            && (compute_ty.size() > 8
                || (op == IrBinOp::Mul && types.iter().any(|t| t.size() == 8)))
    }

    /// Split an integer operand of at most 64 bits into the two 64-bit halves
    /// of its exact 128-bit two's-complement value: `lo` is the operand
    /// extended per its own type, `hi` its sign fill (`None` = zero, for
    /// unsigned operands).
    fn overflow_wide_operand(&mut self, raw: Operand, ty: IrType) -> (Operand, Option<Operand>) {
        let wide_ty = if ty.is_signed() {
            IrType::I64
        } else {
            IrType::U64
        };
        let lo = if ty.size() == 8 {
            raw
        } else {
            Operand::Value(self.emit_cast_val(raw, ty, wide_ty))
        };
        let hi = ty.is_signed().then(|| {
            Operand::Value(self.emit_binop_val(
                IrBinOp::AShr,
                lo,
                Operand::Const(IrConst::I64(63)),
                IrType::I64,
            ))
        });
        (lo, hi)
    }

    /// High 64 bits of the unsigned 64x64-bit product, from four 32x32->64
    /// partial products (every intermediate fits in 64 bits: `mid` is at
    /// most 3 * (2^32 - 1)).
    fn overflow_umulhi64(&mut self, a: Operand, b: Operand) -> Value {
        let u64t = IrType::U64;
        let mask = Operand::Const(IrConst::I64(0xFFFF_FFFF));
        let s32 = Operand::Const(IrConst::I64(32));
        let bin = |this: &mut Self, op, l: Operand, r: Operand| {
            Operand::Value(this.emit_binop_val(op, l, r, u64t))
        };
        let al = bin(self, IrBinOp::And, a, mask);
        let ah = bin(self, IrBinOp::LShr, a, s32);
        let bl = bin(self, IrBinOp::And, b, mask);
        let bh = bin(self, IrBinOp::LShr, b, s32);
        let ll = bin(self, IrBinOp::Mul, al, bl);
        let lh = bin(self, IrBinOp::Mul, al, bh);
        let hl = bin(self, IrBinOp::Mul, ah, bl);
        let hh = bin(self, IrBinOp::Mul, ah, bh);
        let ll_hi = bin(self, IrBinOp::LShr, ll, s32);
        let lh_lo = bin(self, IrBinOp::And, lh, mask);
        let hl_lo = bin(self, IrBinOp::And, hl, mask);
        let mid = bin(self, IrBinOp::Add, ll_hi, lh_lo);
        let mid = bin(self, IrBinOp::Add, mid, hl_lo);
        let lh_hi = bin(self, IrBinOp::LShr, lh, s32);
        let hl_hi = bin(self, IrBinOp::LShr, hl, s32);
        let mid_hi = bin(self, IrBinOp::LShr, mid, s32);
        let hi = bin(self, IrBinOp::Add, hh, lh_hi);
        let hi = bin(self, IrBinOp::Add, hi, hl_hi);
        let Operand::Value(hi) = bin(self, IrBinOp::Add, hi, mid_hi) else {
            unreachable!()
        };
        hi
    }

    /// Exact add/sub/mul overflow for operands and result of at most 64 bits
    /// using only 64-bit IR arithmetic (see `overflow_needs_u64_pair`).
    ///
    /// The exact result is formed as a 128-bit two's-complement pair
    /// `(hi, lo)`: add/sub carry or borrow out of `lo`; a product is
    /// `umulhi(alo, blo)` corrected by the sign fills
    /// (`- (ahi & blo) - (bhi & alo)`, the standard signed-high-multiply
    /// identity; the fills are 0 or all-ones).  Every exact result fits:
    /// sums need 65 bits, and a product of two unsigned 64-bit values stays
    /// below 2^128 - 2^65 + 2, so it can never alias a value that fits the
    /// (at most 64-bit) result type.  The result type is then tested
    /// directly: unsigned T fits iff `hi == 0` and `lo` survives the
    /// truncation round trip; signed T iff `hi` is `lo`'s sign fill and `lo`
    /// survives the round trip.  Returns (truncated result, overflow flag).
    fn lower_overflow_u64_pair(
        &mut self,
        op: IrBinOp,
        lhs: (Operand, IrType),
        rhs: (Operand, IrType),
        result_ty: IrType,
    ) -> (Value, Value) {
        let u64t = IrType::U64;
        let zero = Operand::Const(IrConst::I64(0));
        let (alo, ahi) = self.overflow_wide_operand(lhs.0, lhs.1);
        let (blo, bhi) = self.overflow_wide_operand(rhs.0, rhs.1);
        let (lo, hi) = match op {
            IrBinOp::Add | IrBinOp::Sub => {
                let lo = self.emit_binop_val(op, alo, blo, u64t);
                // Add: carry iff lo < alo.  Sub: borrow iff alo < blo.
                let flag = if op == IrBinOp::Add {
                    self.emit_cmp_val(IrCmpOp::Ult, Operand::Value(lo), alo, u64t)
                } else {
                    self.emit_cmp_val(IrCmpOp::Ult, alo, blo, u64t)
                };
                let flag = self.emit_cast_val(Operand::Value(flag), IrType::I8, u64t);
                let halves =
                    self.emit_binop_val(op, ahi.unwrap_or(zero), bhi.unwrap_or(zero), u64t);
                let hi =
                    self.emit_binop_val(op, Operand::Value(halves), Operand::Value(flag), u64t);
                (lo, hi)
            }
            IrBinOp::Mul => {
                let lo = self.emit_binop_val(IrBinOp::Mul, alo, blo, u64t);
                let mut hi = self.overflow_umulhi64(alo, blo);
                for (fill, other) in [(ahi, blo), (bhi, alo)] {
                    if let Some(fill) = fill {
                        let corr = self.emit_binop_val(IrBinOp::And, fill, other, u64t);
                        hi = self.emit_binop_val(
                            IrBinOp::Sub,
                            Operand::Value(hi),
                            Operand::Value(corr),
                            u64t,
                        );
                    }
                }
                (lo, hi)
            }
            _ => unreachable!("overflow only for add/sub/mul"),
        };

        let truncated = self.emit_cast_val(Operand::Value(lo), u64t, result_ty);
        let expected_hi = if result_ty.is_signed() {
            Operand::Value(self.emit_binop_val(
                IrBinOp::AShr,
                Operand::Value(lo),
                Operand::Const(IrConst::I64(63)),
                IrType::I64,
            ))
        } else {
            zero
        };
        let hi_bad = self.emit_cmp_val(IrCmpOp::Ne, Operand::Value(hi), expected_hi, u64t);
        if result_ty.size() == 8 {
            return (truncated, hi_bad);
        }
        let back_ty = if result_ty.is_signed() {
            IrType::I64
        } else {
            u64t
        };
        let back = self.emit_cast_val(Operand::Value(truncated), result_ty, back_ty);
        let lo_bad = self.emit_cmp_val(IrCmpOp::Ne, Operand::Value(back), Operand::Value(lo), u64t);
        let overflow = self.emit_binop_val(
            IrBinOp::Or,
            Operand::Value(hi_bad),
            Operand::Value(lo_bad),
            IrType::I32,
        );
        (truncated, overflow)
    }

    /// Determine the result IrType and signedness for an overflow builtin.
    fn overflow_result_type(&self, name: &str, result_ptr_expr: &Expr) -> (IrType, bool) {
        // Check type-specific variants first
        // __builtin_sadd_overflow -> signed int
        // __builtin_saddl_overflow -> signed long
        // __builtin_saddll_overflow -> signed long long
        // __builtin_uadd_overflow -> unsigned int
        // etc.
        if name.starts_with("__builtin_s") && name != "__builtin_sub_overflow" {
            // Signed type-specific variant
            let is_signed = true;
            let ty = if name.ends_with("ll_overflow") {
                IrType::I64
            } else if name.ends_with("l_overflow") {
                target_int_ir_type() // long: I64 on LP64, I32 on ILP32
            } else {
                IrType::I32
            };
            return (ty, is_signed);
        }
        if name.starts_with("__builtin_u") {
            // Unsigned type-specific variant
            let is_signed = false;
            let ty = if name.ends_with("ll_overflow") {
                IrType::U64
            } else if name.ends_with("l_overflow") {
                if target_is_32bit() {
                    IrType::U32
                } else {
                    IrType::U64
                } // unsigned long
            } else {
                IrType::U32
            };
            return (ty, is_signed);
        }

        // Generic variant: determine type from the pointer argument's pointee type
        let result_ctype = self.expr_ctype(result_ptr_expr);
        if let CType::Pointer(pointee, _) = &result_ctype {
            let is_signed = pointee.is_signed();
            let ir_ty = IrType::from_ctype(pointee);
            return (ir_ty, is_signed);
        }

        // If we can't determine the type (e.g., &local), try to get the
        // pointee from the expression structure
        if let Expr::AddressOf(inner, _) = result_ptr_expr {
            let inner_ctype = self.expr_ctype(inner);
            let is_signed = inner_ctype.is_signed();
            let ir_ty = IrType::from_ctype(&inner_ctype);
            return (ir_ty, is_signed);
        }

        // Default to signed long if we can't determine
        (target_int_ir_type(), true)
    }

    /// Compute signed overflow flag for add/sub/mul.
    fn compute_signed_overflow(
        &mut self,
        op: IrBinOp,
        lhs: Operand,
        rhs: Operand,
        result: Value,
        ty: IrType,
    ) -> Value {
        let bits = (ty.size() * 8) as i64;
        match op {
            IrBinOp::Add => {
                // Signed add overflow: ((result ^ lhs) & (result ^ rhs)) < 0
                // i.e., overflow if both operands have same sign and result has different sign
                let xor_lhs = self.emit_binop_val(IrBinOp::Xor, Operand::Value(result), lhs, ty);
                let xor_rhs = self.emit_binop_val(IrBinOp::Xor, Operand::Value(result), rhs, ty);
                let and_val = self.emit_binop_val(
                    IrBinOp::And,
                    Operand::Value(xor_lhs),
                    Operand::Value(xor_rhs),
                    ty,
                );
                // Check sign bit: shift right by (bits-1) and check if non-zero
                let shifted = self.emit_binop_val(
                    IrBinOp::AShr,
                    Operand::Value(and_val),
                    Operand::Const(IrConst::I64(bits - 1)),
                    ty,
                );
                // The shifted value is all-ones (-1) if overflow, all-zeros (0) if not
                // Convert to 0/1 by AND with 1
                self.emit_binop_val(
                    IrBinOp::And,
                    Operand::Value(shifted),
                    Operand::Const(IrConst::I64(1)),
                    ty,
                )
            }
            IrBinOp::Sub => {
                // Signed sub overflow: ((lhs ^ rhs) & (result ^ lhs)) < 0
                // overflow if operands have different signs and result sign differs from lhs
                let xor_ops = self.emit_binop_val(IrBinOp::Xor, lhs, rhs, ty);
                let xor_res = self.emit_binop_val(IrBinOp::Xor, Operand::Value(result), lhs, ty);
                let and_val = self.emit_binop_val(
                    IrBinOp::And,
                    Operand::Value(xor_ops),
                    Operand::Value(xor_res),
                    ty,
                );
                let shifted = self.emit_binop_val(
                    IrBinOp::AShr,
                    Operand::Value(and_val),
                    Operand::Const(IrConst::I64(bits - 1)),
                    ty,
                );
                self.emit_binop_val(
                    IrBinOp::And,
                    Operand::Value(shifted),
                    Operand::Const(IrConst::I64(1)),
                    ty,
                )
            }
            IrBinOp::Mul => {
                // For signed multiply overflow, we widen to double width, multiply, then
                // check if the result fits in the original width by checking that
                // sign-extending the truncated result gives back the full result.
                let wide_ty = Self::double_width_type(ty, true);
                let lhs_wide = self.emit_cast_val(lhs, ty, wide_ty);
                let rhs_wide = self.emit_cast_val(rhs, ty, wide_ty);
                let wide_result = self.emit_binop_val(
                    IrBinOp::Mul,
                    Operand::Value(lhs_wide),
                    Operand::Value(rhs_wide),
                    wide_ty,
                );
                // Truncate back to original width
                let truncated = self.emit_cast_val(Operand::Value(wide_result), wide_ty, ty);
                // Sign-extend truncated back to wide type
                let sign_extended = self.emit_cast_val(Operand::Value(truncated), ty, wide_ty);
                // Overflow if wide_result != sign_extended
                self.emit_cmp_val(
                    IrCmpOp::Ne,
                    Operand::Value(wide_result),
                    Operand::Value(sign_extended),
                    wide_ty,
                )
            }
            _ => unreachable!("overflow only for add/sub/mul"),
        }
    }

    /// Compute unsigned overflow flag for add/sub/mul.
    fn compute_unsigned_overflow(
        &mut self,
        op: IrBinOp,
        lhs: Operand,
        rhs: Operand,
        result: Value,
        ty: IrType,
    ) -> Value {
        // Make sure we're comparing in unsigned type
        let unsigned_ty = ty.to_unsigned();
        match op {
            IrBinOp::Add => {
                // Unsigned add overflow: result < lhs (wrapping means the sum is smaller)
                self.emit_cmp_val(IrCmpOp::Ult, Operand::Value(result), lhs, unsigned_ty)
            }
            IrBinOp::Sub => {
                // Unsigned sub overflow: lhs < rhs (borrow occurred)
                self.emit_cmp_val(IrCmpOp::Ult, lhs, rhs, unsigned_ty)
            }
            IrBinOp::Mul => {
                // Unsigned multiply overflow: widen to double width, multiply, check upper half is zero
                let wide_ty = Self::double_width_type(ty, false);
                let lhs_wide = self.emit_cast_val(lhs, unsigned_ty, wide_ty);
                let rhs_wide = self.emit_cast_val(rhs, unsigned_ty, wide_ty);
                let wide_result = self.emit_binop_val(
                    IrBinOp::Mul,
                    Operand::Value(lhs_wide),
                    Operand::Value(rhs_wide),
                    wide_ty,
                );
                // Truncate back to original width
                let truncated =
                    self.emit_cast_val(Operand::Value(wide_result), wide_ty, unsigned_ty);
                // Zero-extend truncated back to wide type
                let zero_extended =
                    self.emit_cast_val(Operand::Value(truncated), unsigned_ty, wide_ty);
                // Overflow if wide_result != zero_extended
                self.emit_cmp_val(
                    IrCmpOp::Ne,
                    Operand::Value(wide_result),
                    Operand::Value(zero_extended),
                    wide_ty,
                )
            }
            _ => unreachable!("overflow only for add/sub/mul"),
        }
    }

    /// Get the double-width integer type for overflow multiply detection.
    fn double_width_type(ty: IrType, signed: bool) -> IrType {
        match ty {
            IrType::I8 | IrType::U8 => {
                if signed {
                    IrType::I16
                } else {
                    IrType::U16
                }
            }
            IrType::I16 | IrType::U16 => {
                if signed {
                    IrType::I32
                } else {
                    IrType::U32
                }
            }
            IrType::I32 | IrType::U32 => {
                if signed {
                    IrType::I64
                } else {
                    IrType::U64
                }
            }
            IrType::I64 | IrType::U64 => {
                if signed {
                    IrType::I128
                } else {
                    IrType::U128
                }
            }
            // For 128-bit, we can't widen further; use a different strategy
            // (but this is extremely rare in practice)
            _ => {
                if signed {
                    IrType::I128
                } else {
                    IrType::U128
                }
            }
        }
    }
}
