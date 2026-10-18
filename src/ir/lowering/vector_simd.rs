//! Whole-vector lowering of GCC vector-extension arithmetic.
//!
//! A `vector_size` / `ext_vector_type` operation is ONE operation on ONE
//! object in the language (`r[i] = a[i] + b[i]` over `float
//! __attribute__((vector_size(32)))` is a 32-byte load, a 32-byte load, a
//! packed add and a 32-byte store).  The per-lane scalar lowering in
//! `expr_assign.rs` loses exactly that fact: every lane becomes an
//! independent scalar load/op/store on an over-aligned stack temporary,
//! and the only way back to a packed instruction is the BB-SLP
//! vectorizer re-discovering the shape lane by lane (which it cannot do
//! for a `char` vector of 32 lanes, a compare mask, a shift, a possibly
//! aliasing pointer stream, or anything the scalar passes rewrote first).
//! Measured on the 214-function vector-extension matrix in
//! `tests/regression/check_vector_ext_codegen.sh` the lane form was 8.6×
//! GCC's instruction count at `-O2 -mavx2` (11553 vs 1340) with not one
//! function at parity.
//!
//! This module lowers the operation directly to the x86-64 `Vec*`
//! intrinsic families the loop and SLP vectorizers already emit and the
//! backend already homes in `%xmm`/`%ymm`:
//!
//! ```text
//!   %a = VecLoad<fam>(ptr_a, 0)        ; whole operand
//!   %b = VecLoad<fam>(ptr_b, 0)        ; or VecBroadcast<fam>(scalar)
//!   %r = VecAdd<fam>(%a, %b)
//!   VecStore<fam>(%r -> result_alloca)
//! ```
//!
//! The result is still an addressable object (the vector-extension
//! expression language is address-based: lane subscripts, `&v`, struct
//! members and the ABI all want memory), so a temporary alloca remains;
//! the `vec_ssa` pass (`src/passes/vec_ssa.rs`) promotes every alloca
//! whose only accesses are whole-width `VecLoad`/`VecStore` to SSA
//! afterwards, which is what lets `add8(a, b) { return a + b; }` become
//! `vaddps %ymm1, %ymm0, %ymm0; ret`.
//!
//! Legality is the family table the SLP vectorizer owns
//! (`slp_vectorizer::family_for` / `packed_binop` / `packed_shift` /
//! `packed_cmp_blendv`): a shape without an exact packed instruction on
//! the code-generation ISA (`target_vector_lowering_bytes`: 0 off, 16
//! SSE2, 32 AVX2) falls back to the per-lane scalar lowering unchanged.
//! The fallback is total — nothing here may reject a program that the
//! scalar path accepts — and semantics-preserving: every packed
//! instruction chosen computes bit-for-bit what the scalar lane
//! operation computes (IEEE packed add/sub/mul/div are the scalar
//! operations per lane; integer lanes wrap identically; shifts are only
//! taken for in-range immediates; compares produce the all-ones/zero
//! masks `lower_vector_cmp` produces).
//!
//! Volatile vector accesses and non-default address spaces keep the
//! scalar lowering (the `Vec*` families carry neither), as do padded
//! `ext_vector_type` shapes (`float3`: three lanes in sixteen bytes) and
//! bit-packed `_Bool` vectors.

use super::lower::Lowerer;
use crate::common::types::{AddressSpace, CType, IrType, target_vector_lowering_bytes};
use crate::frontend::parser::ast::{BinOp, Expr, Initializer, InitializerItem};
use crate::ir::intrinsics::IntrinsicOp;
use crate::ir::reexports::{Instruction, IrBinOp, IrConst, IrUnaryOp, Operand, Terminator, Value};
use crate::passes::slp_vectorizer::{
    VecFamily, all_ones_const, cmp_predicate_imm, family_for, minus_zero_const, packed_binop,
    packed_cmp_blendv, packed_fpxor, packed_shift,
};

/// One packed-lowering shape: the family plus the facts the emitters need.
#[derive(Clone, Copy)]
pub(super) struct SimdShape {
    pub(super) fam: VecFamily,
    pub(super) elem_ir: IrType,
    pub(super) lanes: usize,
    pub(super) bytes: usize,
}

/// The packed family for a vector CType, or `None` when the shape has no
/// exact single-register form on this target (the caller scalarizes).
pub(super) fn simd_shape(vec_ct: &CType) -> Option<SimdShape> {
    let ceiling = target_vector_lowering_bytes();
    if ceiling == 0 {
        return None;
    }
    let (elem_ct, lanes) = vec_ct.vector_info()?;
    if vec_ct.is_bitpacked_bool_vector() || vec_ct.is_x87_lane_vector() {
        return None;
    }
    let bytes = vec_ct.size();
    if bytes > ceiling || !matches!(bytes, 16 | 32) {
        return None;
    }
    // Padded ext-vector storage (`float3` in 16 bytes) is not a dense
    // register image; the lane helpers know the stride, the families do not.
    if vec_ct.vector_element_storage_size()? != elem_ct.size() || lanes * elem_ct.size() != bytes {
        return None;
    }
    let elem_ir = IrType::from_ctype(elem_ct);
    if !matches!(
        elem_ir,
        IrType::I8
            | IrType::U8
            | IrType::I16
            | IrType::U16
            | IrType::I32
            | IrType::U32
            | IrType::I64
            | IrType::U64
            | IrType::F32
            | IrType::F64
    ) {
        return None;
    }
    let fam = family_for(elem_ir, lanes)?;
    Some(SimdShape {
        fam,
        elem_ir,
        lanes,
        bytes,
    })
}

/// Two vector operands may meet in one packed op when they have the same
/// register image: same byte size, same lane count, same lane width and
/// the same float/integer domain (GCC permits `int4 + unsigned4`; the
/// result type decides signedness-sensitive ops, which the caller passes).
fn same_register_image(a: &CType, b: &CType) -> bool {
    match (a.vector_info(), b.vector_info()) {
        (Some((ea, la)), Some((eb, lb))) => {
            a.size() == b.size()
                && la == lb
                && ea.size() == eb.size()
                && ea.is_floating() == eb.is_floating()
                && a.vector_element_storage_size() == b.vector_element_storage_size()
        }
        _ => false,
    }
}

impl Lowerer {
    /// `%v = VecLoad<fam>(ptr, 0)`.
    pub(super) fn emit_vec_load(&mut self, shape: &SimdShape, ptr: Value) -> Value {
        let dest = self.fresh_value();
        self.emit(Instruction::Intrinsic {
            dest: Some(dest),
            op: shape.fam.load,
            dest_ptr: None,
            args: vec![Operand::Value(ptr), Operand::Const(IrConst::I64(0))],
        });
        dest
    }

    /// `VecStore<fam>(%v -> ptr)`. `dest_ptr` is set so every memory-effect
    /// contract (`may_write_memory`, GVN's clobber scan, DSE) sees the write.
    pub(super) fn emit_vec_store(&mut self, shape: &SimdShape, val: Value, ptr: Value) {
        self.emit(Instruction::Intrinsic {
            dest: None,
            op: shape.fam.store,
            dest_ptr: Some(ptr),
            args: vec![
                Operand::Value(val),
                Operand::Value(ptr),
                Operand::Const(IrConst::I64(0)),
            ],
        });
    }

    /// `%v = VecBroadcast<fam>(scalar)` — `scalar` already in the lane type.
    pub(super) fn emit_vec_splat(&mut self, shape: &SimdShape, scalar: Operand) -> Value {
        let dest = self.fresh_value();
        let is_zero = matches!(&scalar, Operand::Const(c) if c.to_i64() == Some(0))
            || matches!(&scalar, Operand::Const(IrConst::F32(f)) if f.to_bits() == 0)
            || matches!(&scalar, Operand::Const(IrConst::F64(f)) if f.to_bits() == 0);
        self.emit(Instruction::Intrinsic {
            dest: Some(dest),
            op: if is_zero {
                shape.fam.zero
            } else {
                shape.fam.broadcast
            },
            dest_ptr: None,
            // The zero families take no operand; the halfword/byte families
            // spell zero as a broadcast of the constant (family table).
            args: if is_zero && shape.fam.zero != shape.fam.broadcast {
                vec![]
            } else {
                vec![scalar]
            },
        });
        dest
    }

    /// `%r = op(%a, %b)` for a two-operand packed family op.
    fn emit_vec_binary(&mut self, op: IntrinsicOp, a: Value, b: Value) -> Value {
        let dest = self.fresh_value();
        self.emit(Instruction::Intrinsic {
            dest: Some(dest),
            op,
            dest_ptr: None,
            args: vec![Operand::Value(a), Operand::Value(b)],
        });
        // Statement-root tag (OP-36): the packed FMA contraction
        // (`passes::vec_fma_contract`) honours `-ffp-contract=on` through
        // the same per-expression tags the scalar mul+add fusion reads.
        self.tag_fp_expr(dest);
        dest
    }

    /// A fresh result temporary for a packed op (the vector's own alignment;
    /// no zero-init is needed: the whole image is written by one store).
    fn vec_result_alloca(&mut self, vec_ct: &CType) -> Value {
        let align = self.ctype_align(vec_ct);
        self.emit_entry_alloca(IrType::Ptr, vec_ct.size(), align, false)
    }

    /// One operand of a packed binary op as a register value: a vector
    /// operand is loaded whole, a scalar one is converted to the lane type
    /// and splat. `None` means "not packable" (volatile access, foreign
    /// register image) and NOTHING has been emitted yet for this operand.
    fn vec_operand_plan(&mut self, e: &Expr, shape: &SimdShape, result_ct: &CType) -> Option<bool> {
        let ct = self.expr_ctype(e);
        if ct.is_vector() {
            if !same_register_image(&ct, result_ct) || self.expr_access_is_volatile(e) {
                return None;
            }
            Some(true)
        } else {
            // Scalar splat: the lane type is an integer or float the
            // implicit-cast machinery converts; complex/decimal/long double
            // scalars have no packed form.
            let src_ty = self.get_expr_type(e);
            if !(src_ty.is_integer() || src_ty.is_float()) || src_ty.is_long_double() {
                return None;
            }
            let _ = shape;
            Some(false)
        }
    }

    /// Materialize one operand per its plan.
    fn vec_operand_value(&mut self, e: &Expr, is_vector: bool, shape: &SimdShape) -> Value {
        if is_vector {
            let ct = self.expr_ctype(e);
            let ptr = self.vector_operand_ptr(e, &ct);
            self.emit_vec_load(shape, ptr)
        } else {
            let src_ty = self.get_expr_type(e);
            let lowered = self.lower_expr(e);
            let scalar = self.emit_implicit_cast(lowered, src_ty, shape.elem_ir);
            self.emit_vec_splat(shape, scalar)
        }
    }

    /// The packed intrinsic for `op` on `shape`, when one exists. Shifts are
    /// answered separately (`vec_shift_plan`): they take an immediate.
    fn vec_binop_intrinsic(
        op: &BinOp,
        shape: &SimdShape,
        is_unsigned: bool,
    ) -> Option<IntrinsicOp> {
        let ir_op = Self::binop_to_ir(*op, is_unsigned);
        match ir_op {
            IrBinOp::Add
            | IrBinOp::Sub
            | IrBinOp::Mul
            | IrBinOp::And
            | IrBinOp::Or
            | IrBinOp::Xor => packed_binop(ir_op, shape.elem_ir, shape.lanes),
            // Packed FP division is exact per lane; integer division has no
            // packed form (GCC scalarizes it too).
            IrBinOp::SDiv | IrBinOp::UDiv if shape.elem_ir.is_float() => {
                packed_binop(ir_op, shape.elem_ir, shape.lanes)
            }
            _ => None,
        }
    }

    /// Shift by a uniform in-range constant: `(intrinsic, amount)`.
    /// GCC's vector shift by a scalar shifts every lane by that scalar;
    /// an amount at or beyond the lane width is undefined for the scalar
    /// lanes too, so only `0 <= amount < lane_bits` is taken packed
    /// (`psll*`/`psrl*`/`psra*` would otherwise differ from the scalar
    /// `shl`/`shr` wrap behaviour, which is what the fallback computes).
    fn vec_shift_plan(
        &mut self,
        op: &BinOp,
        rhs: &Expr,
        shape: &SimdShape,
        is_unsigned: bool,
    ) -> Option<(IntrinsicOp, i64)> {
        if shape.elem_ir.is_float() {
            return None;
        }
        let rhs_ct = self.expr_ctype(rhs);
        if rhs_ct.is_vector() {
            return None;
        }
        let amount = self.eval_const_expr(rhs)?.to_i64()?;
        let lane_bits = (shape.elem_ir.size() * 8) as i64;
        if !(0..lane_bits).contains(&amount) {
            return None;
        }
        let ir_op = Self::binop_to_ir(*op, is_unsigned);
        let packed = packed_shift(ir_op, shape.elem_ir, shape.lanes)?;
        Some((packed, amount))
    }

    /// Packed form of `lhs op rhs` (one side a vector of type `vec_ct`).
    /// Returns the result temporary's address, or `None` with nothing
    /// emitted so the caller scalarizes.
    pub(super) fn try_lower_vector_binary_simd(
        &mut self,
        op: &BinOp,
        lhs: &Expr,
        rhs: &Expr,
        vec_ct: &CType,
    ) -> Option<Operand> {
        let shape = simd_shape(vec_ct)?;
        let (elem_ct, _) = vec_ct.vector_info()?;
        let is_unsigned = elem_ct.is_unsigned();
        let lhs_plan = self.vec_operand_plan(lhs, &shape, vec_ct)?;
        if matches!(op, BinOp::Shl | BinOp::Shr) {
            // Only `vector << const` is packed (the lhs must be the vector:
            // `scalar << vector` is a per-lane shift of a splat — rare, and
            // the scalar path is exact for it).
            if !lhs_plan {
                return None;
            }
            let Some((packed, amount)) = self.vec_shift_plan(op, rhs, &shape, is_unsigned) else {
                return self.try_lower_vector_variable_shift(
                    op,
                    lhs,
                    rhs,
                    &shape,
                    vec_ct,
                    is_unsigned,
                );
            };
            let a = self.vec_operand_value(lhs, true, &shape);
            let dest = self.fresh_value();
            self.emit(Instruction::Intrinsic {
                dest: Some(dest),
                op: packed,
                dest_ptr: None,
                args: vec![Operand::Value(a), Operand::Const(IrConst::I64(amount))],
            });
            let result = self.vec_result_alloca(vec_ct);
            self.emit_vec_store(&shape, dest, result);
            return Some(Operand::Value(result));
        }
        let packed = Self::vec_binop_intrinsic(op, &shape, is_unsigned)?;
        let rhs_plan = self.vec_operand_plan(rhs, &shape, vec_ct)?;
        // Evaluation order: C leaves it unspecified between the operands;
        // the scalar lowering evaluates lhs then rhs, keep that.
        let a = self.vec_operand_value(lhs, lhs_plan, &shape);
        let b = self.vec_operand_value(rhs, rhs_plan, &shape);
        let r = self.emit_vec_binary(packed, a, b);
        let result = self.vec_result_alloca(vec_ct);
        self.emit_vec_store(&shape, r, result);
        Some(Operand::Value(result))
    }

    /// Per-lane variable shift (`lhs` a vector; `rhs` a vector of counts
    /// or a non-constant scalar splat), AVX2 `vpsllv*`/`vpsrlv*`/`vpsravd`.
    /// Lanes are 32-bit (any direction) or 64-bit (left/logical right:
    /// there is no `vpsravq` below AVX-512VL). A count at or beyond the
    /// lane width is undefined for GNU vector shifts (as for the scalar
    /// lanes), so the ISA's "shift out to zero / sign fill" is admissible.
    fn try_lower_vector_variable_shift(
        &mut self,
        op: &BinOp,
        lhs: &Expr,
        rhs: &Expr,
        shape: &SimdShape,
        vec_ct: &CType,
        is_unsigned: bool,
    ) -> Option<Operand> {
        // Every variable-shift encoding (256- and VEX.128) is AVX2: the
        // 32-byte lowering ceiling is exactly the AVX2 code-generation
        // level (`Driver::run`), so a 16-byte shape is packed only there.
        if shape.elem_ir.is_float() || target_vector_lowering_bytes() != 32 {
            return None;
        }
        let ir_op = Self::binop_to_ir(*op, is_unsigned);
        let packed = match (shape.bytes, shape.elem_ir.size(), ir_op) {
            (32, 4, IrBinOp::Shl) => IntrinsicOp::VecShlvI32x8,
            (32, 4, IrBinOp::LShr) => IntrinsicOp::VecLShrvI32x8,
            (32, 4, IrBinOp::AShr) => IntrinsicOp::VecAShrvI32x8,
            (32, 8, IrBinOp::Shl) => IntrinsicOp::VecShlvI64x4,
            (32, 8, IrBinOp::LShr) => IntrinsicOp::VecLShrvI64x4,
            (16, 4, IrBinOp::Shl) => IntrinsicOp::VecShlvI32x4,
            (16, 4, IrBinOp::LShr) => IntrinsicOp::VecLShrvI32x4,
            (16, 4, IrBinOp::AShr) => IntrinsicOp::VecAShrvI32x4,
            (16, 8, IrBinOp::Shl) => IntrinsicOp::VecShlvI64x2,
            (16, 8, IrBinOp::LShr) => IntrinsicOp::VecLShrvI64x2,
            _ => return None,
        };
        // The count operand: a vector of the same register image (any
        // signedness — only the lane width matters) or a scalar splat.
        let rhs_ct = self.expr_ctype(rhs);
        let rhs_plan = if rhs_ct.is_vector() {
            let (rhs_elem, rhs_lanes) = rhs_ct.vector_info()?;
            if rhs_lanes != shape.lanes
                || rhs_elem.size() != shape.elem_ir.size()
                || !rhs_elem.is_integer()
                || self.expr_access_is_volatile(rhs)
            {
                return None;
            }
            true
        } else {
            self.vec_operand_plan(rhs, shape, vec_ct)?
        };
        let a = self.vec_operand_value(lhs, true, shape);
        let b = self.vec_operand_value(rhs, rhs_plan, shape);
        let r = self.emit_vec_binary(packed, a, b);
        let result = self.vec_result_alloca(vec_ct);
        self.emit_vec_store(shape, r, result);
        Some(Operand::Value(result))
    }

    /// Packed form of the element-wise comparison `lhs op rhs` producing the
    /// all-ones/all-zeros lane mask (`lower_vector_cmp` semantics).
    pub(super) fn try_lower_vector_cmp_simd(
        &mut self,
        op: BinOp,
        lhs: &Expr,
        rhs: &Expr,
        vec_ct: &CType,
    ) -> Option<Operand> {
        let shape = simd_shape(vec_ct)?;
        let (elem_ct, _) = vec_ct.vector_info()?;
        let mask_ct = vec_ct.vector_comparison_result()?;
        if mask_ct.size() != vec_ct.size() {
            return None;
        }
        let is_unsigned = elem_ct.is_unsigned();
        let cmp_op = Self::binop_to_cmp(op, is_unsigned);
        let (pred, swap) = cmp_predicate_imm(cmp_op, shape.elem_ir.is_float())?;
        let (cmp_intrinsic, _) = packed_cmp_blendv(shape.elem_ir, shape.lanes)?;
        let lhs_plan = self.vec_operand_plan(lhs, &shape, vec_ct)?;
        let rhs_plan = self.vec_operand_plan(rhs, &shape, vec_ct)?;
        let a = self.vec_operand_value(lhs, lhs_plan, &shape);
        let b = self.vec_operand_value(rhs, rhs_plan, &shape);
        let (x, y) = if swap { (b, a) } else { (a, b) };
        let mask = self.fresh_value();
        self.emit(Instruction::Intrinsic {
            dest: Some(mask),
            op: cmp_intrinsic,
            dest_ptr: None,
            args: vec![
                Operand::Value(x),
                Operand::Value(y),
                Operand::Const(IrConst::I32(pred as i32)),
            ],
        });
        // The mask is stored with the MASK type's family when it has one
        // (so the consumer's reload -- `v32i m = a < b; m & x` loads
        // `I32x8` -- pairs with this store and `vec_ssa` dissolves the
        // temporary), else with the operand family's (bit-identical image;
        // the I64 lanes of a double compare have no family on SSE2).
        let result = self.vec_result_alloca(&mask_ct);
        let store_shape = simd_shape(&mask_ct).unwrap_or(shape);
        self.emit_vec_store(&store_shape, mask, result);
        Some(Operand::Value(result))
    }

    /// Packed form of `-v` / `~v`.
    pub(super) fn try_lower_vector_unary_simd(
        &mut self,
        ir_unary: IrUnaryOp,
        inner: &Expr,
        vec_ct: &CType,
    ) -> Option<Operand> {
        let shape = simd_shape(vec_ct)?;
        let src_ct = self.expr_ctype(inner);
        if !same_register_image(&src_ct, vec_ct) || self.expr_access_is_volatile(inner) {
            return None;
        }
        // Resolve the packed form BEFORE emitting anything.
        enum Plan {
            FpNeg(IntrinsicOp, IrConst),
            IntNeg(IntrinsicOp),
            Not(IntrinsicOp, IrConst),
        }
        let plan = match ir_unary {
            IrUnaryOp::Neg if shape.elem_ir.is_float() => Plan::FpNeg(
                packed_fpxor(shape.elem_ir, shape.lanes)?,
                minus_zero_const(shape.elem_ir)?,
            ),
            IrUnaryOp::Neg => Plan::IntNeg(packed_binop(IrBinOp::Sub, shape.elem_ir, shape.lanes)?),
            IrUnaryOp::Not if !shape.elem_ir.is_float() => Plan::Not(
                packed_binop(IrBinOp::Xor, shape.elem_ir, shape.lanes)?,
                all_ones_const(shape.elem_ir)?,
            ),
            _ => return None,
        };
        let ptr = self.vector_operand_ptr(inner, &src_ct);
        let v = self.emit_vec_load(&shape, ptr);
        let r = match plan {
            Plan::FpNeg(xor, mz) => {
                // `v ^ -0.0` flips exactly the sign bit: `-(+0.0)` is `-0.0`
                // and NaN payloads are preserved, which `0 - v` would not do.
                let dest = self.fresh_value();
                self.emit(Instruction::Intrinsic {
                    dest: Some(dest),
                    op: xor,
                    dest_ptr: None,
                    args: vec![Operand::Value(v), Operand::Const(mz)],
                });
                dest
            }
            Plan::IntNeg(sub) => {
                let zero = self.emit_vec_splat(&shape, Operand::Const(IrConst::I64(0)));
                self.emit_vec_binary(sub, zero, v)
            }
            Plan::Not(xor, ones) => {
                let ones = self.emit_vec_splat(&shape, Operand::Const(ones));
                self.emit_vec_binary(xor, v, ones)
            }
        };
        let result = self.vec_result_alloca(vec_ct);
        self.emit_vec_store(&shape, r, result);
        Some(Operand::Value(result))
    }

    /// Packed form of `lhs op= rhs` on a vector lvalue: one load of the
    /// object, the packed op, one store back. The lvalue keeps its own
    /// address; volatile objects and non-default address spaces scalarize.
    pub(super) fn try_lower_vector_compound_assign_simd(
        &mut self,
        op: &BinOp,
        lhs: &Expr,
        rhs: &Expr,
        lhs_ct: &CType,
    ) -> Option<Operand> {
        let shape = simd_shape(lhs_ct)?;
        if self.expr_access_is_volatile(lhs)
            || self.get_addr_space_of_struct_expr(lhs) != AddressSpace::Default
        {
            return None;
        }
        let (elem_ct, _) = lhs_ct.vector_info()?;
        let is_unsigned = elem_ct.is_unsigned();
        enum Plan {
            Bin(IntrinsicOp, bool),
            Shift(IntrinsicOp, i64),
        }
        let plan = if matches!(op, BinOp::Shl | BinOp::Shr) {
            let (packed, amount) = self.vec_shift_plan(op, rhs, &shape, is_unsigned)?;
            Plan::Shift(packed, amount)
        } else {
            let packed = Self::vec_binop_intrinsic(op, &shape, is_unsigned)?;
            let rhs_plan = self.vec_operand_plan(rhs, &shape, lhs_ct)?;
            Plan::Bin(packed, rhs_plan)
        };
        // Same operand order as the scalar lowering: lvalue address first,
        // then the rhs.
        let lhs_ptr = self.lower_expr(lhs);
        let lhs_ptr_val = self.operand_to_value(lhs_ptr);
        let r = match plan {
            Plan::Bin(packed, rhs_is_vector) => {
                let b = self.vec_operand_value(rhs, rhs_is_vector, &shape);
                let a = self.emit_vec_load(&shape, lhs_ptr_val);
                self.emit_vec_binary(packed, a, b)
            }
            Plan::Shift(packed, amount) => {
                let a = self.emit_vec_load(&shape, lhs_ptr_val);
                let dest = self.fresh_value();
                self.emit(Instruction::Intrinsic {
                    dest: Some(dest),
                    op: packed,
                    dest_ptr: None,
                    args: vec![Operand::Value(a), Operand::Const(IrConst::I64(amount))],
                });
                dest
            }
        };
        self.emit_vec_store(&shape, r, lhs_ptr_val);
        Some(Operand::Value(lhs_ptr_val))
    }
}

impl Lowerer {
    /// `return <vector>` as a register value.
    ///
    /// The psABI returns a 16-byte vector in `%xmm0` and, at the AVX ABI
    /// level, a 32-byte vector in `%ymm0`.  The generic aggregate paths
    /// reach those registers through memory: the 16-byte case packs two
    /// `I64` loads into an `I128` (which the backend then spills to
    /// reassemble `%xmm0`), the 32-byte case copies the object into the
    /// hidden-pointer buffer and reloads it.  For a SIMD-shaped vector
    /// return type the value is a `VecLoad<fam>` of the object instead and
    /// `Return` carries the vector value; `emit_return_impl` moves it to
    /// `%xmm0`/`%ymm0`.  With `vec_ssa` the load forwards to the producing
    /// packed instruction and the memory object disappears.
    ///
    /// Inline candidates keep the aggregate path: the inliner substitutes
    /// the `Return` operand for the call result, and the call-site lowering
    /// consumes that result as the `I128`/pointer carrier the signature
    /// advertises.
    pub(super) fn try_lower_vector_return_simd(&mut self, e: &Expr) -> Option<Operand> {
        if self.target != crate::backend::Target::X86_64 || self.func().is_inline_candidate {
            return None;
        }
        let ret_ct = self.func().return_ctype.clone()?;
        if !ret_ct.is_vector() {
            return None;
        }
        let shape = simd_shape(&ret_ct)?;
        let fname = self.func().name.clone();
        let sig = self.func_meta.sigs.get(fname.as_str())?;
        let register_return = match shape.bytes {
            16 => {
                sig.two_reg_ret_size == Some(16)
                    && sig.ret_eightbyte_classes[..]
                        == [
                            crate::common::types::EightbyteClass::Sse,
                            crate::common::types::EightbyteClass::SseUp,
                        ]
            }
            32 => {
                sig.sret_size == Some(32)
                    && crate::backend::call_abi::hidden_vector_return_bytes(
                        true,
                        &sig.ret_eightbyte_classes,
                    ) == Some(32)
            }
            _ => false,
        };
        if !register_return {
            return None;
        }
        // The expression's own type must be the same dense object (a
        // `return` of a differently typed vector goes through the generic
        // conversion paths).
        let expr_ct = self.expr_ctype(e);
        if expr_ct.size() != shape.bytes || !expr_ct.is_vector() {
            return None;
        }
        if self.aggregate_lvalue_space(e) != AddressSpace::Default {
            return None;
        }
        let addr = self.get_struct_base_addr(e);
        let value = self.emit_vec_load(&shape, addr);
        Some(Operand::Value(value))
    }

    /// The single predicate behind `try_lower_vector_conditional_simd`:
    /// `Some((arm type, shape))` when a vector `?:` takes the packed
    /// path.  `get_struct_base_addr` asks the same question so it can use
    /// the packed result (always a fresh merge temporary that nothing else
    /// names) in place without the defensive aggregate copy it must make
    /// for a generic ternary (whose arms may yield the address of a live
    /// object).  One predicate, two callers: they cannot disagree.
    pub(super) fn packed_vector_conditional_shape(
        &mut self,
        cond: &Expr,
        then_expr: &Expr,
        else_expr: &Expr,
    ) -> Option<(CType, SimdShape)> {
        let then_ct = self.expr_ctype(then_expr);
        let else_ct = self.expr_ctype(else_expr);
        if !then_ct.is_vector() || !else_ct.is_vector() || !same_register_image(&then_ct, &else_ct)
        {
            return None;
        }
        let shape = simd_shape(&then_ct)?;
        if self.eval_const_expr(cond).is_some()
            || self.expr_access_is_volatile(then_expr)
            || self.expr_access_is_volatile(else_expr)
            || self.aggregate_lvalue_space(then_expr) != AddressSpace::Default
            || self.aggregate_lvalue_space(else_expr) != AddressSpace::Default
        {
            return None;
        }
        Some((then_ct, shape))
    }

    /// `cond ? a : b` on two vectors of one register image: each arm is
    /// loaded whole and stored to ONE merge temporary, which the join reads
    /// as the (address-valued) result.  Replaces the generic aggregate
    /// ternary -- a pointer phi over the two arm temporaries followed by a
    /// 32-byte `Memcpy` into a third one -- with a single object the
    /// alignment relaxation and `vec_ssa` can then dissolve.  Only the
    /// taken arm is evaluated (C evaluation rules); a constant condition
    /// is left to the generic dead-arm fold.
    pub(super) fn try_lower_vector_conditional_simd(
        &mut self,
        cond: &Expr,
        then_expr: &Expr,
        else_expr: &Expr,
    ) -> Option<Operand> {
        let (then_ct, shape) = self.packed_vector_conditional_shape(cond, then_expr, else_expr)?;
        let merge = self.vec_result_alloca(&then_ct);
        let then_label = self.fresh_label();
        let else_label = self.fresh_label();
        let end_label = self.fresh_label();
        self.lower_condition_branch(cond, then_label, else_label);

        self.start_block(then_label);
        let then_ptr = self.get_struct_base_addr(then_expr);
        let then_val = self.emit_vec_load(&shape, then_ptr);
        self.emit_vec_store(&shape, then_val, merge);
        self.terminate(Terminator::Branch(end_label));

        self.start_block(else_label);
        let else_ptr = self.get_struct_base_addr(else_expr);
        let else_val = self.emit_vec_load(&shape, else_ptr);
        self.emit_vec_store(&shape, else_val, merge);
        self.terminate(Terminator::Branch(end_label));

        self.start_block(end_label);
        Some(Operand::Value(merge))
    }

    /// `lhs = rhs` on one register image: one whole-width load of the
    /// right-hand object and one whole-width store into the left-hand
    /// object (the generic path copies through a 32-byte `Memcpy`, which
    /// the backend expands as two 16-byte moves through a scratch pointer
    /// -- `r[i] = a[i] * b[i] + r[i]` paid 7 instructions per iteration for
    /// what is one `vmovups`).  Load before store, so an overlapping
    /// right-hand object (a union member, `v = v`) reads its old bytes.
    /// Returns `false` (nothing emitted) when the shape is not packable or
    /// either side is volatile / in a named address space.
    pub(super) fn try_lower_vector_assign_simd(
        &mut self,
        lhs: &Expr,
        lhs_ptr: Value,
        rhs: &Expr,
        lhs_ct: &CType,
    ) -> bool {
        let Some(shape) = simd_shape(lhs_ct) else {
            return false;
        };
        let rhs_ct = self.expr_ctype(rhs);
        if !rhs_ct.is_vector()
            || !same_register_image(lhs_ct, &rhs_ct)
            || self.expr_access_is_volatile(lhs)
            || self.expr_access_is_volatile(rhs)
            || self.aggregate_lvalue_space(lhs) != AddressSpace::Default
            || self.aggregate_lvalue_space(rhs) != AddressSpace::Default
        {
            return false;
        }
        let rhs_ptr = self.vector_operand_ptr(rhs, &rhs_ct);
        let value = self.emit_vec_load(&shape, rhs_ptr);
        self.emit_vec_store(&shape, value, lhs_ptr);
        true
    }

    /// `T v = rhs;` for a packable vector local: the initializer object is
    /// loaded whole and stored whole into the fresh object (the generic
    /// path copies with a `Memcpy`).  Initialization is not a volatile
    /// access of the new object; a volatile right-hand access keeps the
    /// generic path.
    pub(super) fn try_lower_vector_init_simd(
        &mut self,
        rhs: &Expr,
        destination: Value,
        vector_ct: &CType,
    ) -> bool {
        let Some(shape) = simd_shape(vector_ct) else {
            return false;
        };
        let rhs_ct = self.expr_ctype(rhs);
        if !rhs_ct.is_vector()
            || !same_register_image(vector_ct, &rhs_ct)
            || self.expr_access_is_volatile(rhs)
            || self.aggregate_lvalue_space(rhs) != AddressSpace::Default
        {
            return false;
        }
        let rhs_ptr = self.vector_operand_ptr(rhs, &rhs_ct);
        let value = self.emit_vec_load(&shape, rhs_ptr);
        self.emit_vec_store(&shape, value, destination);
        true
    }

    /// Packed form of a uniform constant init list — `v = {}`, `{0}`,
    /// `{0, 0, 0, 0}` or `{c, c, c, c}` — as ONE `VecZero`/`VecBroadcast`
    /// of the vector's own family plus one full-width store.
    ///
    /// The general lowering writes the zero ladder (`I64` stores) and then
    /// each mentioned lane; the SLP later re-packs the ladder as an
    /// INTEGER-family zero store, and the explicit `{0}` lane store stays a
    /// scalar `Store`. Either one is a foreign access for `vec_ssa`, which
    /// is why `v32f acc = {0}; for (...) acc += ...` kept the accumulator in
    /// memory across the whole loop (one load and one store per iteration,
    /// `vmovdqu %ymm0, 88(%rsp); movl $0, 88(%rsp)` in the prologue).
    /// Returns `false` with nothing emitted for every other list (designated
    /// lanes, non-constant or non-uniform lanes, `_Bool` lanes, no SIMD
    /// shape) so the caller takes the general path.
    pub(super) fn try_lower_vector_init_list_simd(
        &mut self,
        items: &[InitializerItem],
        destination: Value,
        vector_ct: &CType,
    ) -> bool {
        let Some(shape) = simd_shape(vector_ct) else {
            return false;
        };
        let Some((elem_ct, num_elems)) = vector_ct.vector_info() else {
            return false;
        };
        if matches!(elem_ct, CType::Bool)
            || vector_ct.is_bitpacked_bool_vector()
            || items.len() > num_elems
            || items.iter().any(|it| !it.designators.is_empty())
        {
            return false;
        }
        // Every mentioned lane as a lane-typed constant.
        let mut lanes: Vec<IrConst> = Vec::with_capacity(items.len());
        for item in items {
            let expr = match &item.init {
                Initializer::Expr(e) => e,
                Initializer::List(sub) => match Self::unwrap_nested_init_expr(sub) {
                    Some(e) => e,
                    None => return false,
                },
            };
            let Some(c) = self.eval_const_expr(expr) else {
                return false;
            };
            let Some(c) = Self::const_as_lane(c, shape.elem_ir) else {
                return false;
            };
            lanes.push(c);
        }
        let is_zero = |c: &IrConst| match c {
            IrConst::F32(f) => f.to_bits() == 0,
            IrConst::F64(f) => f.to_bits() == 0,
            other => other.to_i64() == Some(0),
        };
        let splat = if lanes.iter().all(is_zero) {
            // Short lists are zero-extended by C; an all-zero list of any
            // length is the zero image.
            match shape.elem_ir {
                IrType::F32 => IrConst::F32(0.0),
                IrType::F64 => IrConst::F64(0.0),
                _ => IrConst::I64(0),
            }
        } else if lanes.len() == num_elems && lanes.iter().all(|c| *c == lanes[0]) {
            lanes[0]
        } else {
            return false;
        };
        let value = self.emit_vec_splat(&shape, Operand::Const(splat));
        self.emit_vec_store(&shape, value, destination);
        true
    }

    /// `c` converted to the lane type `elem_ir` exactly as the scalar init
    /// path's `emit_implicit_cast` would, or `None` when the conversion is
    /// not a plain constant fold (then the general path handles it).
    fn const_as_lane(c: IrConst, elem_ir: IrType) -> Option<IrConst> {
        let int_lane = |bits: i64| match elem_ir.size() {
            1 => IrConst::I8(bits as i8),
            2 => IrConst::I16(bits as i16),
            4 => IrConst::I32(bits as i32),
            8 => IrConst::I64(bits),
            _ => IrConst::I128(bits as i128),
        };
        Some(match (c, elem_ir) {
            (IrConst::F32(_), IrType::F32) | (IrConst::F64(_), IrType::F64) => c,
            (IrConst::F64(f), IrType::F32) => IrConst::F32(f as f32),
            (IrConst::F32(f), IrType::F64) => IrConst::F64(f64::from(f)),
            (
                IrConst::F32(_)
                | IrConst::F64(_)
                | IrConst::D32(_)
                | IrConst::D64(_)
                | IrConst::LongDouble(..),
                _,
            ) => return None,
            (_, IrType::F32) => IrConst::F32(c.to_i64()? as f32),
            (_, IrType::F64) => IrConst::F64(c.to_i64()? as f64),
            (_, ty) if ty.is_float() => return None,
            (_, _) => int_lane(c.to_i64()?),
        })
    }
}
