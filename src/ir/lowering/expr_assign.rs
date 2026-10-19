//! Assignment and compound assignment lowering, including bitfield operations.
//!
//! Extracted from expr.rs. This module handles:
//! - `lower_assign`: simple assignment (lhs = rhs)
//! - `lower_struct_assign`: struct/union copy via memcpy
//! - `lower_compound_assign`: compound assignment (+=, -=, *=, etc.)
//! - Bitfield helpers: resolve_bitfield_lvalue, store_bitfield, extract_bitfield
//! - Arithmetic conversion helpers: usual_arithmetic_conversions, promote_for_op, narrow_from_op

use super::lower::Lowerer;
use super::structs::ExtVectorSelection;
use crate::common::types::{AddressSpace, CType, IrType, SsoMode, widened_op_type};
use crate::frontend::parser::ast::{BinOp, Expr};
use crate::ir::reexports::{Instruction, IrBinOp, IrCmpOp, IrConst, IrUnaryOp, Operand, Value};

impl Lowerer {
    /// `v.yz = w` / `p->xy = w` / `n.wzyx.zw = w`: a multi-lane ext-vector
    /// selection on the left stores each selected lane of its root vector
    /// from the matching lane of the right-hand vector (a scalar is
    /// splatted). Sema rejects a repeated lane in the target (Clang).
    /// `None` when `lhs` is not such a selection (a one-lane selection is
    /// an ordinary element lvalue).
    fn try_lower_swizzle_assign(&mut self, lhs: &Expr, rhs: &Expr) -> Option<Operand> {
        let discard_result = std::mem::replace(&mut self.discard_assign_result, false);
        let sel = self.ext_vector_selection_expr(lhs)?;
        if sel.lanes.len() < 2 {
            return None;
        }
        debug_assert!(
            sel.lanes
                .iter()
                .enumerate()
                .all(|(i, l)| !sel.lanes[..i].contains(l)),
            "sema rejects an ext-vector selection target with a repeated lane"
        );
        let elem_ir_ty = Self::vector_elem_ir_type(sel.elem_ct());
        let rhs_ct = self.expr_ctype(rhs);
        let (addr_space, volatile) = self.ext_vector_selection_access(&sel);
        // Evaluate the right side first (C: the store is not sequenced
        // before the operand evaluation), then the destination address.
        // Every source lane is loaded before the first store: the source
        // may overlap the destination (`v.xy = v.yx` swaps; `d->xyz =
        // s->zyx` with `d == s`). A selection on the right reads its lanes
        // straight out of its root vector instead of through a temporary.
        let rhs_selection = self
            .ext_vector_selection_expr(rhs)
            .filter(|src| src.lanes.len() == sel.lanes.len());
        let mut values: Vec<Operand> = Vec::with_capacity(sel.lanes.len());
        if let Some(src) = rhs_selection {
            let (src_space, src_volatile) = self.ext_vector_selection_access(&src);
            let src_base = self.ext_vector_selection_base(&src);
            let src_elem = Self::vector_elem_ir_type(src.elem_ct());
            for &sl in &src.lanes {
                let v =
                    self.load_vector_lane(src_base, &src.vector_ct, sl, src_space, src_volatile);
                values.push(self.emit_implicit_cast(v, src_elem, elem_ir_ty));
            }
        } else if rhs_ct.is_vector() {
            let ptr = self.vector_operand_ptr(rhs, &rhs_ct);
            let src_elem = rhs_ct
                .vector_info()
                .map_or(elem_ir_ty, |(e, _)| Self::vector_elem_ir_type(e));
            for i in 0..sel.lanes.len() {
                let v = self.load_vector_lane(ptr, &rhs_ct, i, AddressSpace::Default, false);
                values.push(self.emit_implicit_cast(v, src_elem, elem_ir_ty));
            }
        } else {
            // Scalar right operand: Clang splats it over the named lanes.
            let raw = self.lower_expr(rhs);
            let raw_ty = self.value_ir_type(rhs);
            let v = self.emit_implicit_cast(raw, raw_ty, elem_ir_ty);
            values.resize(sel.lanes.len(), v);
        }
        let dest = self.ext_vector_selection_base(&sel);
        for (&lane, v) in sel.lanes.iter().zip(values.iter()) {
            self.store_vector_lane(
                dest,
                &sel.vector_ct,
                lane,
                v.clone(),
                elem_ir_ty,
                addr_space,
                volatile,
            );
        }
        // The value of the assignment is the right operand converted to
        // the selection's type. An expression statement never reads it; a
        // consumer gets a fresh vector built from the values just stored
        // (not re-read from the destination: a volatile object must not
        // see an access the source does not contain).
        if discard_result {
            return Some(Operand::Const(IrConst::I64(0)));
        }
        Some(self.ext_vector_from_lanes(&sel.result_ct, values, elem_ir_ty))
    }

    /// A fresh vector of type `vector_ct` whose lane `i` is `values[i]`
    /// (of IR type `elem_ir_ty`), in the vector-value convention of
    /// `lower_vector_binary_op` (the temporary's address).
    pub(super) fn ext_vector_from_lanes(
        &mut self,
        vector_ct: &CType,
        values: Vec<Operand>,
        elem_ir_ty: IrType,
    ) -> Operand {
        let size = vector_ct.size();
        let align = self.ctype_align(vector_ct);
        let out = self.emit_entry_alloca(IrType::Ptr, size, align, false);
        if crate::ir::lowering::lower::vector_temp_needs_zero_init(vector_ct, values.len()) {
            self.zero_init_alloca(out, size);
        }
        for (i, v) in values.into_iter().enumerate() {
            self.store_vector_lane(
                out,
                vector_ct,
                i,
                v,
                elem_ir_ty,
                AddressSpace::Default,
                false,
            );
        }
        Operand::Value(out)
    }

    /// `v.zx op= w` (Clang): each selected lane of the root vector becomes
    /// `lane op w[i]`, at the element width (a scalar `w` is splatted). The
    /// root is evaluated once, before the right operand, as for a whole
    /// vector. The operand's lanes are loaded first, then every selected
    /// lane, and only then is anything stored, so an operand overlapping the
    /// target (`v.zx += v.xz`, `p->yx += *q` with `q` aliasing `p`) sees
    /// only old values. The value is a fresh vector of the new lanes.
    fn lower_ext_vector_selection_compound_assign(
        &mut self,
        op: &BinOp,
        sel: &ExtVectorSelection<'_>,
        rhs: &Expr,
    ) -> Operand {
        let discard_result = std::mem::replace(&mut self.discard_assign_result, false);
        let elem_ct = sel.elem_ct().clone();
        let elem_ir_ty = Self::vector_elem_ir_type(&elem_ct);
        let ir_op = Self::binop_to_ir(*op, elem_ct.is_unsigned());
        let count = sel.lanes.len();
        let (seg, volatile) = self.ext_vector_selection_access(sel);
        let base = self.ext_vector_selection_base(sel);
        let rhs_ct = self.expr_ctype(rhs);
        let mut operands = Vec::with_capacity(count);
        if rhs_ct.is_vector() {
            let ptr = self.vector_operand_ptr(rhs, &rhs_ct);
            let src_elem = rhs_ct
                .vector_info()
                .map_or(elem_ir_ty, |(e, _)| Self::vector_elem_ir_type(e));
            for i in 0..count {
                let v = self.load_vector_lane(ptr, &rhs_ct, i, AddressSpace::Default, false);
                operands.push(self.emit_implicit_cast(v, src_elem, elem_ir_ty));
            }
        } else {
            let raw = self.lower_expr(rhs);
            let raw_ty = self.value_ir_type(rhs);
            let v = self.emit_implicit_cast(raw, raw_ty, elem_ir_ty);
            operands.resize(count, v);
        }
        let mut old = Vec::with_capacity(count);
        for &lane in &sel.lanes {
            old.push(self.load_vector_lane(base, &sel.vector_ct, lane, seg, volatile));
        }
        let mut new = Vec::with_capacity(count);
        for (cur, r) in old.into_iter().zip(operands) {
            new.push(Operand::Value(
                self.emit_binop_val(ir_op, cur, r, elem_ir_ty),
            ));
        }
        for (&lane, v) in sel.lanes.iter().zip(new.iter()) {
            self.store_vector_lane(
                base,
                &sel.vector_ct,
                lane,
                v.clone(),
                elem_ir_ty,
                seg,
                volatile,
            );
        }
        if discard_result {
            return Operand::Const(IrConst::I64(0));
        }
        self.ext_vector_from_lanes(&sel.result_ct, new, elem_ir_ty)
    }

    pub(super) fn lower_assign(&mut self, lhs: &Expr, rhs: &Expr) -> Operand {
        if let Some(result) = self.try_lower_swizzle_assign(lhs, rhs) {
            return result;
        }
        // A file-scope register variable has no addressable storage. Stage the
        // assigned value into its fixed physical register with an empty inline
        // asm input constraint; this is the standard compiler representation
        // of `register void *p asm("rbx")` writes.
        if let Expr::Identifier(name, _) = lhs {
            if let Some((reg_name, ty)) = self.globals.get(name).and_then(|info| {
                info.asm_register
                    .as_ref()
                    .filter(|reg| crate::ir::lowering::lower::is_x86_register_name(reg))
                    .map(|reg| (reg.clone(), info.ty))
            }) {
                let raw = self.lower_expr(rhs);
                let rhs_ty = self.value_ir_type(rhs);
                let value = self.emit_implicit_cast(raw, rhs_ty, ty);
                self.emit(Instruction::InlineAsm {
                    template: String::new(),
                    outputs: vec![],
                    inputs: vec![(format!("{{{}}}", reg_name), value, None)],
                    clobbers: vec![],
                    operand_types: vec![ty],
                    goto_labels: vec![],
                    input_symbols: vec![],
                    seg_overrides: vec![AddressSpace::Default],
                });
                return value;
            }
        }

        // Vector assignment: memcpy the whole vector
        let lhs_ct = self.expr_ctype(lhs);
        if let Some((_, _num_elems)) = lhs_ct.vector_info() {
            return self.lower_vector_assign(lhs, rhs, &lhs_ct);
        }

        // Complex assignment: copy real/imag parts
        if lhs_ct.is_complex() {
            return self.lower_complex_assign(lhs, rhs, &lhs_ct);
        }

        if self.struct_value_size(lhs).is_some() {
            return self.lower_struct_assign(lhs, rhs);
        }

        // Check for bitfield assignment
        if let Some(result) = self.try_lower_bitfield_assign(lhs, rhs) {
            return result;
        }

        // Evaluate the LHS lvalue address BEFORE the RHS expression.
        // This matches GCC's evaluation order: the address where we store is
        // computed using the state before any RHS side effects (e.g., i++ in
        // `a[i] = i++` uses the pre-increment value of i for the index).
        let lhs_ty = self.get_expr_type(lhs);
        let lv = self.lower_lvalue_for_mutation(lhs);

        // When assigning a complex RHS to a non-complex LHS, extract the real part first
        let rhs_ct = self.expr_ctype(rhs);
        let is_bool_target = self.is_bool_lvalue(lhs);
        let rhs_val = if rhs_ct.is_complex() && !lhs_ct.is_complex() {
            let complex_val = self.lower_expr(rhs);
            let ptr = self.operand_to_value(complex_val);
            if is_bool_target {
                // For _Bool targets, check both real and imag parts per C11 6.3.1.2
                self.lower_complex_to_bool(ptr, &rhs_ct)
            } else {
                let real_part = self.load_complex_real(ptr, &rhs_ct);
                let from_ty = Self::complex_component_ir_type(&rhs_ct);
                self.emit_implicit_cast(real_part, from_ty, lhs_ty)
            }
        } else if is_bool_target {
            // For _Bool targets, normalize at the source type before any truncation.
            // Truncating first (e.g. 0x100 -> U8 = 0) then normalizing gives wrong results.
            let rhs_val = self.lower_expr(rhs);
            let rhs_ty = self.value_ir_type(rhs);
            self.emit_expr_bool_conversion(rhs_val, rhs_ty, rhs)
        } else {
            let rhs_val = self.lower_expr(rhs);
            let rhs_ty = self.value_ir_type(rhs);
            self.convert_assign_value(rhs_val, rhs_ty, Some(&rhs_ct), &lhs_ct, lhs_ty)
        };

        if let Some(lv) = lv {
            self.store_lvalue_typed(&lv, rhs_val, lhs_ty);
            return rhs_val;
        }
        rhs_val
    }

    /// Lower struct/union assignment using memcpy.
    fn lower_struct_assign(&mut self, lhs: &Expr, rhs: &Expr) -> Operand {
        let struct_size = self.struct_value_size(lhs).unwrap_or(8);
        let dynamic_size = self
            .dynamic_struct_value_size(lhs)
            .or_else(|| self.dynamic_struct_value_size(rhs));

        // For expressions producing packed struct data (small struct function
        // call returns, ternaries over them, etc.), store directly
        if dynamic_size.is_none() && self.expr_produces_packed_struct_data(rhs) && struct_size <= 8
        {
            let rhs_val = self.lower_expr(rhs);
            if let Some(lv) = self.lower_lvalue_for_mutation(lhs) {
                let dest_addr = self.lvalue_addr(&lv);
                let dest_space = self.aggregate_lvalue_space(lhs);
                if dest_space == AddressSpace::Default {
                    self.store_packed_data_exact(dest_addr, rhs_val, struct_size);
                } else {
                    // Assemble the value in a generic slot, then copy it into
                    // the segment object.
                    let slot = self.emit_entry_alloca(IrType::I64, 8, 8, false);
                    self.store_packed_data_exact(slot, rhs_val, struct_size);
                    let vol = self.expr_access_is_volatile(lhs);
                    self.emit_segment_copy(
                        dest_addr,
                        dest_space,
                        slot,
                        AddressSpace::Default,
                        struct_size,
                        vol,
                    );
                }
                return Operand::Value(dest_addr);
            }
            return rhs_val;
        }

        let rhs_val = self.lower_aggregate_rvalue(rhs);
        if let Some(lv) = self.lower_lvalue_for_mutation(lhs) {
            let dest_addr = self.lvalue_addr(&lv);
            let src_addr = self.operand_to_value(rhs_val);
            let dest_space = self.aggregate_lvalue_space(lhs);
            if dest_space != AddressSpace::Default {
                // Destination object lives in a named address space: write it
                // through the segment (see seg_copy.rs). A dynamic-size copy
                // into a segment has no lowering yet, so reject it loudly.
                if dynamic_size.is_some() {
                    self.diagnostics.borrow_mut().error(
                        "assignment to a segment-qualified aggregate with a variable-length member is not supported",
                        lhs.span(),
                    );
                } else {
                    let vol = self.expr_access_is_volatile(lhs);
                    self.emit_segment_copy(
                        dest_addr,
                        dest_space,
                        src_addr,
                        AddressSpace::Default,
                        struct_size,
                        vol,
                    );
                }
            } else if let Some(size_val) = dynamic_size {
                self.emit_dynamic_memcpy(dest_addr, src_addr, size_val);
            } else {
                self.emit(Instruction::Memcpy {
                    dest: dest_addr,
                    src: src_addr,
                    size: struct_size,
                });
            }
            return Operand::Value(dest_addr);
        }
        rhs_val
    }

    // -----------------------------------------------------------------------
    // Bitfield operations
    // -----------------------------------------------------------------------

    /// Resolve a bitfield member access expression, returning the field's address and
    /// bitfield metadata. Returns None if the expression is not a bitfield access.
    pub(super) fn resolve_bitfield_lvalue(
        &mut self,
        expr: &Expr,
    ) -> Option<(Value, IrType, u32, u32, SsoMode, AddressSpace)> {
        let (base_expr, field_name, is_pointer) = match expr {
            Expr::MemberAccess(base, field, _) => (base.as_ref(), field.as_str(), false),
            Expr::PointerMemberAccess(base, field, _) => (base.as_ref(), field.as_str(), true),
            _ => return None,
        };

        let (field_offset, declared_ty, bitfield) = if is_pointer {
            self.resolve_pointer_member_access_full(base_expr, field_name)
        } else {
            self.resolve_member_access_full(base_expr, field_name)
        };

        let (bit_offset, bit_width) = bitfield?;

        // Reverse SSO bitfields in a multi-byte unit load the WHOLE unit.
        let (sso, sso_unit_ty) = self.sso_storage_of_member(base_expr, is_pointer, field_name);
        let storage_ty = match (&sso, bitfield.is_some()) {
            (SsoMode::ByteSwapUnit(_), true) => sso_unit_ty
                .as_ref()
                .map(IrType::from_ctype)
                .unwrap_or(declared_ty),
            _ => declared_ty,
        };

        let base_addr = if is_pointer {
            let ptr_val = self.lower_expr(base_expr);
            self.operand_to_value(ptr_val)
        } else {
            self.get_struct_base_addr(base_expr)
        };

        let field_addr = self.fresh_value();
        self.emit(Instruction::GetElementPtr {
            dest: field_addr,
            base: base_addr,
            offset: Operand::Const(IrConst::ptr_int(field_offset as i64)),
            ty: storage_ty,
        });

        // The storage unit lives in the object's address space: `gbf.x` takes
        // it from `gbf`, `p->x` from the pointee of `p`. Every load and store of
        // this bitfield must carry it, or a segment object is accessed generically.
        let seg = self.get_addr_space_of_struct_expr(expr);
        Some((field_addr, storage_ty, bit_offset, bit_width, sso, seg))
    }

    /// Integer-promotion result used by arithmetic on a bit-field. Fields wider
    /// than int retain their declared 64-bit type; forcing every operation to
    /// I32 discards the high half of 33..64-bit fields before read-modify-write.
    pub(super) fn bitfield_arithmetic_type(storage_ty: IrType, bit_width: u32) -> IrType {
        if bit_width <= 31 || (bit_width <= 32 && storage_ty.is_signed()) {
            IrType::I32
        } else if bit_width == 32 {
            IrType::U32
        } else {
            storage_ty
        }
    }

    /// Truncate a value to bit_width bits and sign-extend if the bitfield is signed.
    pub(super) fn truncate_to_bitfield_value(
        &mut self,
        val: Operand,
        val_ty: IrType,
        bit_width: u32,
        is_signed: bool,
    ) -> Operand {
        // Widths above int precision use the declared 64-bit extended
        // bit-field type; narrower fields use ordinary integer promotion.
        let op_ty = if bit_width > 32 {
            if is_signed { IrType::I64 } else { IrType::U64 }
        } else {
            widened_op_type(IrType::I32)
        };
        let val = self.emit_implicit_cast(val, val_ty, op_ty);
        let op_bits = (op_ty.size() * 8) as u32;
        if bit_width >= op_bits {
            return val;
        }
        if is_signed {
            let shl_amount = op_bits - bit_width;
            let shifted = self.emit_binop_val(
                IrBinOp::Shl,
                val,
                Operand::Const(IrConst::I64(shl_amount as i64)),
                op_ty,
            );
            let result = self.emit_binop_val(
                IrBinOp::AShr,
                Operand::Value(shifted),
                Operand::Const(IrConst::I64(shl_amount as i64)),
                op_ty,
            );
            Operand::Value(result)
        } else {
            let mask = (1u64 << bit_width) - 1;
            let masked = self.emit_binop_val(
                IrBinOp::And,
                val,
                Operand::Const(IrConst::I64(mask as i64)),
                op_ty,
            );
            Operand::Value(masked)
        }
    }

    /// Try to lower assignment to a bitfield member.
    fn try_lower_bitfield_assign(&mut self, lhs: &Expr, rhs: &Expr) -> Option<Operand> {
        let (field_addr, storage_ty, bit_offset, bit_width, sso, seg) =
            self.resolve_bitfield_lvalue(lhs)?;
        let is_bool = self.is_bool_lvalue(lhs);
        let rhs_val = self.lower_expr(rhs);
        let rhs_ty = self.value_ir_type(rhs);
        // C standard 6.3.1.2: assigning to _Bool converts the value to 0 or 1
        // (any nonzero becomes 1) BEFORE bitfield truncation. Without this,
        // e.g. `s.bool_bf = 2` would mask 2 (0b10) to 1 bit = 0, not 1.
        let store_val = if is_bool {
            let normalized = self.emit_expr_bool_conversion(rhs_val, rhs_ty, rhs);
            self.emit_implicit_cast(normalized, IrType::I32, storage_ty)
        } else {
            self.emit_implicit_cast(rhs_val, rhs_ty, storage_ty)
        };
        self.store_bitfield(
            field_addr,
            storage_ty,
            bit_offset,
            bit_width,
            store_val,
            self.expr_access_is_volatile(lhs),
            sso,
            seg,
        );
        Some(self.truncate_to_bitfield_value(
            store_val,
            storage_ty,
            bit_width,
            storage_ty.is_signed(),
        ))
    }

    /// The C integer type of a promoted bitfield value's IR type.
    fn promoted_int_ctype(ty: IrType) -> CType {
        match ty {
            IrType::U32 => CType::UInt,
            IrType::I64 => CType::LongLong,
            IrType::U64 => CType::ULongLong,
            IrType::I128 => CType::Int128,
            IrType::U128 => CType::UInt128,
            _ => CType::Int,
        }
    }

    /// Try to lower compound assignment to a bitfield member (e.g., s.bf += val).
    pub(super) fn try_lower_bitfield_compound_assign(
        &mut self,
        op: &BinOp,
        lhs: &Expr,
        rhs: &Expr,
    ) -> Option<Operand> {
        let (field_addr, storage_ty, bit_offset, bit_width, sso, seg) =
            self.resolve_bitfield_lvalue(lhs)?;
        let is_bool = self.is_bool_lvalue(lhs);

        let is_vol = self.expr_access_is_volatile(lhs);
        let current_val = self.extract_bitfield_from_addr(
            field_addr, storage_ty, bit_offset, bit_width, sso, is_vol, seg,
        );
        let current_ty = crate::ir::lowering::expr_types::bitfield_promoted_type(
            storage_ty,
            Some((bit_offset, bit_width)),
        );

        let rhs_val = self.lower_expr(rhs);
        let rhs_ty = self.value_ir_type(rhs);

        // A floating right operand (binary, _Float128 or decimal) makes the
        // floating type the common type (C11 6.5.16.2p3, 6.3.1.8): the field
        // value converts to it, the operation runs there, and only the
        // result converts back to the field. Converting the operand to the
        // field's integer type first truncated it (`bf *= 0.5` gave 0, not
        // bf / 2).
        let rhs_ct = self.expr_ctype(rhs);
        if matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div)
            && (rhs_ct.is_floating() || rhs_ct.is_soft_float())
        {
            let field_ct = Self::promoted_int_ctype(current_ty);
            let common_ct = CType::usual_arithmetic_conversion(&field_ct, &rhs_ct);
            let l = self.convert_scalar_ctype(current_val, current_ty, &field_ct, &common_ct);
            let r = self.convert_scalar_ctype(rhs_val, rhs_ty, &rhs_ct, &common_ct);
            let result = self.emit_arith_in_ctype(op, l, r, &common_ct);
            let common_ir = IrType::from_ctype(&common_ct);
            let store_val = if is_bool && common_ct.is_floating() {
                // Conversion to _Bool is `!= 0` (C11 6.3.1.2), in the
                // floating type itself.
                let truth = self.emit_bool_normalize_typed(Operand::Value(result), common_ir);
                self.emit_implicit_cast(truth, IrType::I32, storage_ty)
            } else {
                // _Float128 and decimal conversions to _Bool compare with
                // zero themselves.
                let target_ct = if is_bool { CType::Bool } else { field_ct };
                let back = self.convert_scalar_ctype(
                    Operand::Value(result),
                    common_ir,
                    &common_ct,
                    &target_ct,
                );
                self.emit_implicit_cast(back, IrType::from_ctype(&target_ct), storage_ty)
            };
            self.store_bitfield(
                field_addr, storage_ty, bit_offset, bit_width, store_val, is_vol, sso, seg,
            );
            return Some(self.truncate_to_bitfield_value(
                store_val,
                storage_ty,
                bit_width,
                storage_ty.is_signed(),
            ));
        }

        let is_unsigned = storage_ty.is_unsigned();
        let ir_op = Self::binop_to_ir(*op, is_unsigned);
        let wt = widened_op_type(Self::bitfield_arithmetic_type(storage_ty, bit_width));
        let current_casted = self.emit_implicit_cast(current_val, current_ty, wt);
        let rhs_casted = self.emit_implicit_cast(rhs_val, rhs_ty, wt);
        let result = self.emit_binop_val(ir_op, current_casted, rhs_casted, wt);

        // C standard 6.3.1.2: when the target is _Bool, normalize the result
        // to 0 or 1 before storing into the bitfield.
        let store_val = if is_bool {
            let normalized = self.emit_bool_normalize_typed(Operand::Value(result), wt);
            self.emit_implicit_cast(normalized, IrType::I32, storage_ty)
        } else {
            self.emit_implicit_cast(Operand::Value(result), wt, storage_ty)
        };

        self.store_bitfield(
            field_addr,
            storage_ty,
            bit_offset,
            bit_width,
            store_val,
            self.expr_access_is_volatile(lhs),
            sso,
            seg,
        );
        Some(self.truncate_to_bitfield_value(
            store_val,
            storage_ty,
            bit_width,
            storage_ty.is_signed(),
        ))
    }

    /// Store a value into a bitfield: load storage unit, clear field bits, OR in new value, store back.
    /// Handles packed bitfields that span beyond the storage type (bit_offset + bit_width > storage_bits).
    /// `sso` applies the reverse-storage-order byte fixups on both the loaded
    /// old unit and the new unit about to be stored.
    pub(super) fn store_bitfield(
        &mut self,
        addr: Value,
        storage_ty: IrType,
        bit_offset: u32,
        bit_width: u32,
        val: Operand,
        volatile: bool,
        sso: SsoMode,
        seg: AddressSpace,
    ) {
        if bit_width >= 64 && bit_offset == 0 {
            // Full-width field: the store covers the whole storage unit, so
            // `val` can go straight into the Store.  Every caller passes a
            // value already converted to `storage_ty` (plain assign converts
            // the RHS; compound assign/inc-dec convert the arithmetic result;
            // the initializer paths convert via `field_ty`), and the previous
            // code here re-cast it "from the machine word type" —
            // `widened_op_type(I32)` is I32, so on x86-64 a full-width
            // unsigned field's U64 value was re-declared as I32 and the
            // backend emitted a `cltq` sign-extend that wiped the upper half
            // of every `s.f += x` on a `unsigned long long f:64` (stress lab
            // bitfield_8/16 at -O0).  Only a narrower-typed CONSTANT needs
            // normalizing, which is a compile-time width change.
            let widened = match &val {
                Operand::Const(c) => {
                    if let Some(v64) = c.to_i64() {
                        Operand::Const(IrConst::I64(v64))
                    } else {
                        val
                    }
                }
                _ => val,
            };
            let stored = self.emit_sso_store_fixup(widened, storage_ty, sso);
            self.emit(Instruction::Store {
                volatile,
                val: stored,
                ptr: addr,
                ty: storage_ty,
                seg_override: seg,
            });
            return;
        }

        let storage_bits = (storage_ty.size() * 8) as u32;

        // Check if the bitfield spans beyond the storage type boundary (packed bitfields)
        if storage_bits > 0 && bit_offset + bit_width > storage_bits {
            self.store_bitfield_split(
                addr,
                storage_ty,
                storage_bits,
                bit_offset,
                bit_width,
                val,
                volatile,
                sso,
                seg,
            );
            return;
        }

        // Use the target-appropriate operation type: I32 on i686 for <=32-bit storage, I64 on 64-bit targets.
        let op_ty = widened_op_type(storage_ty);
        let op_bits = (op_ty.size() * 8) as u32;

        let val = self.emit_implicit_cast(val, storage_ty, op_ty);

        let mask = if bit_width >= op_bits {
            u64::MAX
        } else {
            (1u64 << bit_width) - 1
        };
        let masked_val = self.emit_binop_val(
            IrBinOp::And,
            val,
            Operand::Const(IrConst::I64(mask as i64)),
            op_ty,
        );

        let shifted_val = if bit_offset > 0 {
            self.emit_binop_val(
                IrBinOp::Shl,
                Operand::Value(masked_val),
                Operand::Const(IrConst::I64(bit_offset as i64)),
                op_ty,
            )
        } else {
            masked_val
        };

        let old_val = self.fresh_value();
        self.emit(Instruction::Load {
            volatile,
            dest: old_val,
            ptr: addr,
            ty: storage_ty,
            seg_override: seg,
        });
        let old_fixed = self.emit_sso_load_fixup(Operand::Value(old_val), storage_ty, sso);

        let clear_mask = if bit_width >= op_bits {
            0u64
        } else {
            !(mask << bit_offset)
        };
        let cleared = self.emit_binop_val(
            IrBinOp::And,
            old_fixed,
            Operand::Const(IrConst::I64(clear_mask as i64)),
            op_ty,
        );
        let new_val = self.emit_binop_val(
            IrBinOp::Or,
            Operand::Value(cleared),
            Operand::Value(shifted_val),
            op_ty,
        );

        let stored = self.emit_sso_store_fixup(Operand::Value(new_val), storage_ty, sso);
        self.emit(Instruction::Store {
            volatile,
            val: stored,
            ptr: addr,
            ty: storage_ty,
            seg_override: seg,
        });
    }

    /// Store a bitfield that spans across two storage units (packed bitfields).
    /// Splits the value into low and high parts and does two read-modify-write operations.
    fn store_bitfield_split(
        &mut self,
        addr: Value,
        storage_ty: IrType,
        storage_bits: u32,
        bit_offset: u32,
        bit_width: u32,
        val: Operand,
        volatile: bool,
        sso: SsoMode,
        seg: AddressSpace,
    ) {
        let low_bits = storage_bits - bit_offset;
        let high_bits = bit_width - low_bits;

        // Use the target-appropriate operation type: I32 on i686 for <=32-bit storage, I64 on 64-bit targets.
        let op_ty = widened_op_type(storage_ty);
        let op_bits = (op_ty.size() * 8) as u32;

        let val = self.emit_implicit_cast(val, storage_ty, op_ty);

        let mask = if bit_width >= op_bits {
            u64::MAX
        } else {
            (1u64 << bit_width) - 1
        };
        let masked_val = self.emit_binop_val(
            IrBinOp::And,
            val,
            Operand::Const(IrConst::I64(mask as i64)),
            op_ty,
        );

        // Low part: take low_bits from masked_val, shift left by bit_offset
        let low_mask = if low_bits >= op_bits {
            u64::MAX
        } else {
            (1u64 << low_bits) - 1
        };
        let low_val = self.emit_binop_val(
            IrBinOp::And,
            Operand::Value(masked_val),
            Operand::Const(IrConst::I64(low_mask as i64)),
            op_ty,
        );
        let shifted_low = if bit_offset > 0 {
            self.emit_binop_val(
                IrBinOp::Shl,
                Operand::Value(low_val),
                Operand::Const(IrConst::I64(bit_offset as i64)),
                op_ty,
            )
        } else {
            low_val
        };

        // Read-modify-write low storage unit
        let old_low = self.fresh_value();
        self.emit(Instruction::Load {
            volatile,
            dest: old_low,
            ptr: addr,
            ty: storage_ty,
            seg_override: seg,
        });
        let old_low_fixed = self.emit_sso_load_fixup(Operand::Value(old_low), storage_ty, sso);
        let low_clear = !(low_mask << bit_offset);
        let cleared_low = self.emit_binop_val(
            IrBinOp::And,
            old_low_fixed,
            Operand::Const(IrConst::I64(low_clear as i64)),
            op_ty,
        );
        let new_low = self.emit_binop_val(
            IrBinOp::Or,
            Operand::Value(cleared_low),
            Operand::Value(shifted_low),
            op_ty,
        );
        let new_low_fixed = self.emit_sso_store_fixup(Operand::Value(new_low), storage_ty, sso);
        self.emit(Instruction::Store {
            volatile,
            val: new_low_fixed,
            ptr: addr,
            ty: storage_ty,
            seg_override: seg,
        });

        // High part: take remaining bits from masked_val >> low_bits, store at bit 0 of next unit
        let high_val = self.emit_binop_val(
            IrBinOp::LShr,
            Operand::Value(masked_val),
            Operand::Const(IrConst::I64(low_bits as i64)),
            op_ty,
        );
        let high_mask = if high_bits >= op_bits {
            u64::MAX
        } else {
            (1u64 << high_bits) - 1
        };
        let masked_high = self.emit_binop_val(
            IrBinOp::And,
            Operand::Value(high_val),
            Operand::Const(IrConst::I64(high_mask as i64)),
            op_ty,
        );

        let high_addr = self.emit_gep_offset(addr, storage_ty.size(), IrType::I8);

        // Read-modify-write high storage unit
        let old_high = self.fresh_value();
        self.emit(Instruction::Load {
            volatile,
            dest: old_high,
            ptr: high_addr,
            ty: storage_ty,
            seg_override: seg,
        });
        let old_high_fixed = self.emit_sso_load_fixup(Operand::Value(old_high), storage_ty, sso);
        let high_clear = !high_mask;
        let cleared_high = self.emit_binop_val(
            IrBinOp::And,
            old_high_fixed,
            Operand::Const(IrConst::I64(high_clear as i64)),
            op_ty,
        );
        let new_high = self.emit_binop_val(
            IrBinOp::Or,
            Operand::Value(cleared_high),
            Operand::Value(masked_high),
            op_ty,
        );
        let new_high_fixed = self.emit_sso_store_fixup(Operand::Value(new_high), storage_ty, sso);
        self.emit(Instruction::Store {
            volatile,
            val: new_high_fixed,
            ptr: high_addr,
            ty: storage_ty,
            seg_override: seg,
        });
    }

    /// Extract a bitfield value from a loaded (and SSO-fixed-up) storage unit.
    pub(super) fn extract_bitfield(
        &mut self,
        loaded: Operand,
        storage_ty: IrType,
        bit_offset: u32,
        bit_width: u32,
    ) -> Operand {
        // Use the target-appropriate operation type: I32 on i686 for <=32-bit storage, I64 on 64-bit targets.
        let op_ty = widened_op_type(storage_ty);
        let op_bits = (op_ty.size() * 8) as u32;

        let promoted_ty = crate::ir::lowering::expr_types::bitfield_promoted_type(
            storage_ty,
            Some((bit_offset, bit_width)),
        );

        if bit_width >= op_bits && bit_offset == 0 {
            return self.emit_implicit_cast(loaded, storage_ty, promoted_ty);
        }

        // Widen the loaded value to the operation type before performing shift/mask.
        // On i686 with I32 op_ty, a sub-32-bit storage (I8/I16) needs widening to I32.
        // On 64-bit targets, everything widens to I64.
        let widened: Operand = if storage_ty.size() < op_ty.size() {
            let unsigned_storage = storage_ty.to_unsigned();
            let target_unsigned = op_ty.to_unsigned();
            let v = self.emit_cast_val(loaded, unsigned_storage, target_unsigned);
            Operand::Value(v)
        } else {
            loaded
        };

        // If the loaded value doesn't cover all bits (split case), the caller
        // should use extract_bitfield_from_addr instead. But handle the non-split case.
        let raw = if storage_ty.is_signed() {
            let shl_amount = op_bits - bit_offset - bit_width;
            let ashr_amount = op_bits - bit_width;
            let mut val = widened.clone();
            if shl_amount > 0 && shl_amount < op_bits {
                let shifted = self.emit_binop_val(
                    IrBinOp::Shl,
                    val,
                    Operand::Const(IrConst::I64(shl_amount as i64)),
                    op_ty,
                );
                val = Operand::Value(shifted);
            }
            if ashr_amount > 0 && ashr_amount < op_bits {
                let result = self.emit_binop_val(
                    IrBinOp::AShr,
                    val,
                    Operand::Const(IrConst::I64(ashr_amount as i64)),
                    op_ty,
                );
                Operand::Value(result)
            } else {
                val
            }
        } else {
            let mut val = widened.clone();
            if bit_offset > 0 {
                let shifted = self.emit_binop_val(
                    IrBinOp::LShr,
                    val,
                    Operand::Const(IrConst::I64(bit_offset as i64)),
                    op_ty,
                );
                val = Operand::Value(shifted);
            }
            if bit_width >= op_bits {
                val
            } else {
                let mask = (1u64 << bit_width) - 1;
                let masked = self.emit_binop_val(
                    IrBinOp::And,
                    val,
                    Operand::Const(IrConst::I64(mask as i64)),
                    op_ty,
                );
                Operand::Value(masked)
            }
        };

        self.emit_implicit_cast(raw, op_ty, promoted_ty)
    }

    /// Extract a bitfield from memory, handling the case where it spans two storage units.
    /// `sso` is the field's reverse-scalar_storage_order mode: the loaded storage
    /// unit(s) must be byte-fixed-up (bswap / per-byte bit-reverse) before the
    /// bit math, which always operates on the logical (host-order) unit value.
    pub(super) fn extract_bitfield_from_addr(
        &mut self,
        addr: Value,
        storage_ty: IrType,
        bit_offset: u32,
        bit_width: u32,
        sso: SsoMode,
        volatile: bool,
        seg: AddressSpace,
    ) -> Operand {
        let storage_bits = (storage_ty.size() * 8) as u32;

        if storage_bits > 0 && bit_offset + bit_width > storage_bits {
            // Split extraction: load from two storage units and combine
            // Use the target-appropriate operation type for shift/mask operations.
            let op_ty = widened_op_type(storage_ty);
            let op_bits = (op_ty.size() * 8) as u32;
            let low_bits = storage_bits - bit_offset;
            let high_bits = bit_width - low_bits;

            // Load low part, shift right by bit_offset to get low_bits at bit 0
            let low_loaded = self.fresh_value();
            self.emit(Instruction::Load {
                volatile,
                dest: low_loaded,
                ptr: addr,
                ty: storage_ty,
                seg_override: seg,
            });
            let low_fixed = self.emit_sso_load_fixup(Operand::Value(low_loaded), storage_ty, sso);
            let low_fixed_v = match &low_fixed {
                Operand::Value(v) => *v,
                Operand::Const(_) => unreachable!(),
            };
            let low_val = if bit_offset > 0 {
                self.emit_binop_val(
                    IrBinOp::LShr,
                    low_fixed,
                    Operand::Const(IrConst::I64(bit_offset as i64)),
                    op_ty,
                )
            } else {
                low_fixed_v
            };
            let low_mask = if low_bits >= op_bits {
                u64::MAX
            } else {
                (1u64 << low_bits) - 1
            };
            let masked_low = self.emit_binop_val(
                IrBinOp::And,
                Operand::Value(low_val),
                Operand::Const(IrConst::I64(low_mask as i64)),
                op_ty,
            );

            // Load high part from next storage unit
            let high_addr = self.emit_gep_offset(addr, storage_ty.size(), IrType::I8);
            let high_loaded = self.fresh_value();
            self.emit(Instruction::Load {
                volatile,
                dest: high_loaded,
                ptr: high_addr,
                ty: storage_ty,
                seg_override: seg,
            });
            let high_fixed = self.emit_sso_load_fixup(Operand::Value(high_loaded), storage_ty, sso);
            let high_mask = if high_bits >= op_bits {
                u64::MAX
            } else {
                (1u64 << high_bits) - 1
            };
            let masked_high = self.emit_binop_val(
                IrBinOp::And,
                high_fixed,
                Operand::Const(IrConst::I64(high_mask as i64)),
                op_ty,
            );

            // Shift high part left by low_bits and OR with low part
            let shifted_high = self.emit_binop_val(
                IrBinOp::Shl,
                Operand::Value(masked_high),
                Operand::Const(IrConst::I64(low_bits as i64)),
                op_ty,
            );
            let combined = self.emit_binop_val(
                IrBinOp::Or,
                Operand::Value(masked_low),
                Operand::Value(shifted_high),
                op_ty,
            );

            // Sign extend if the field type is signed
            let raw = if storage_ty.is_signed() && bit_width < op_bits {
                let shl_amount = op_bits - bit_width;
                let shifted = self.emit_binop_val(
                    IrBinOp::Shl,
                    Operand::Value(combined),
                    Operand::Const(IrConst::I64(shl_amount as i64)),
                    op_ty,
                );
                let result = self.emit_binop_val(
                    IrBinOp::AShr,
                    Operand::Value(shifted),
                    Operand::Const(IrConst::I64(shl_amount as i64)),
                    op_ty,
                );
                Operand::Value(result)
            } else {
                Operand::Value(combined)
            };

            let promoted_ty = crate::ir::lowering::expr_types::bitfield_promoted_type(
                storage_ty,
                Some((bit_offset, bit_width)),
            );
            self.emit_implicit_cast(raw, op_ty, promoted_ty)
        } else {
            // Normal case: load single storage unit and extract
            let loaded = self.fresh_value();
            self.emit(Instruction::Load {
                volatile,
                dest: loaded,
                ptr: addr,
                ty: storage_ty,
                seg_override: seg,
            });
            let fixed = self.emit_sso_load_fixup(Operand::Value(loaded), storage_ty, sso);
            self.extract_bitfield(fixed, storage_ty, bit_offset, bit_width)
        }
    }

    // -----------------------------------------------------------------------
    // Compound assignment
    // -----------------------------------------------------------------------

    pub(super) fn lower_compound_assign(&mut self, op: &BinOp, lhs: &Expr, rhs: &Expr) -> Operand {
        // Vector compound assignment (element-wise)
        let lhs_ct = self.expr_ctype(lhs);
        if lhs_ct.is_vector() {
            if let Some(sel) = self
                .ext_vector_selection_expr(lhs)
                .filter(|sel| sel.lanes.len() > 1)
            {
                return self.lower_ext_vector_selection_compound_assign(op, &sel, rhs);
            }
            return self.lower_vector_compound_assign(op, lhs, rhs, &lhs_ct);
        }

        // Complex compound assignment
        if lhs_ct.is_complex() && matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div) {
            return self.lower_complex_compound_assign(op, lhs, rhs, &lhs_ct);
        }

        // Non-complex LHS but complex RHS: extract real part
        let rhs_ct = self.expr_ctype(rhs);
        if !lhs_ct.is_complex() && rhs_ct.is_complex() {
            return self.lower_scalar_compound_assign_complex_rhs(op, lhs, rhs, &rhs_ct);
        }

        // Check for bitfield compound assignment
        if let Some(result) = self.try_lower_bitfield_compound_assign(op, lhs, rhs) {
            return result;
        }

        // Standard scalar compound assignment
        self.lower_scalar_compound_assign(op, lhs, rhs)
    }

    /// Complex compound assignment (z += w, z -= w, z *= w, z /= w).
    fn lower_complex_compound_assign(
        &mut self,
        op: &BinOp,
        lhs: &Expr,
        rhs: &Expr,
        lhs_ct: &crate::common::types::CType,
    ) -> Operand {
        let rhs_ct = self.expr_ctype(rhs);
        let result_ct = self.common_complex_type(lhs_ct, &rhs_ct);

        let lhs_ptr = self.lower_complex_lvalue(lhs);

        // The LHS object's space: its current value is read from there, and
        // the result is written back there (a `__seg_gs` complex is %gs-resident).
        let lhs_space = self.get_addr_space_of_struct_expr(lhs);
        let lhs_size = Self::complex_component_size(lhs_ct) * 2;
        let lhs_read_ptr = if lhs_space == AddressSpace::Default {
            lhs_ptr
        } else {
            let tmp = self.emit_entry_alloca(IrType::I8, lhs_size, 8, false);
            let lhs_vol = self.expr_access_is_volatile(lhs);
            self.emit_segment_copy(
                tmp,
                AddressSpace::Default,
                lhs_ptr,
                lhs_space,
                lhs_size,
                lhs_vol,
            );
            tmp
        };
        let op_lhs_ptr = if result_ct != *lhs_ct {
            let lhs_converted =
                self.convert_to_complex(Operand::Value(lhs_read_ptr), lhs_ct, &result_ct);
            self.operand_to_value(lhs_converted)
        } else {
            lhs_read_ptr
        };

        let rhs_val = self.lower_expr(rhs);
        let rhs_complex = self.convert_to_complex(rhs_val, &rhs_ct, &result_ct);
        let rhs_ptr = self.operand_to_value(rhs_complex);

        let result = match op {
            BinOp::Add => self.lower_complex_add(op_lhs_ptr, rhs_ptr, &result_ct),
            BinOp::Sub => self.lower_complex_sub(op_lhs_ptr, rhs_ptr, &result_ct),
            BinOp::Mul => self.lower_complex_mul(op_lhs_ptr, rhs_ptr, &result_ct),
            BinOp::Div => self.lower_complex_div(op_lhs_ptr, rhs_ptr, &result_ct),
            _ => unreachable!("unsupported complex compound assignment op: {:?}", op),
        };

        let store_ptr = if result_ct != *lhs_ct {
            let result_ptr = self.operand_to_value(result);
            let converted_back =
                self.convert_to_complex(Operand::Value(result_ptr), &result_ct, lhs_ct);
            self.operand_to_value(converted_back)
        } else {
            self.operand_to_value(result)
        };
        self.copy_bytes_across_spaces(
            lhs_ptr,
            lhs_space,
            store_ptr,
            AddressSpace::Default,
            lhs_size,
            self.expr_access_is_volatile(lhs),
        );

        Operand::Value(lhs_ptr)
    }

    /// Scalar compound assignment with complex RHS: extract real part from RHS.
    fn lower_scalar_compound_assign_complex_rhs(
        &mut self,
        op: &BinOp,
        lhs: &Expr,
        rhs: &Expr,
        rhs_ct: &crate::common::types::CType,
    ) -> Operand {
        let rhs_val = self.lower_expr(rhs);
        let rhs_ptr = self.operand_to_value(rhs_val);
        let real_part = self.load_complex_real(rhs_ptr, rhs_ct);
        let real_ty = Self::complex_component_ir_type(rhs_ct);

        let ty = self.get_expr_type(lhs);
        let common_ty = if ty.is_float() || real_ty.is_float() {
            if ty == IrType::F128 || real_ty == IrType::F128 {
                IrType::F128
            } else if ty == IrType::F64 || real_ty == IrType::F64 {
                IrType::F64
            } else {
                IrType::F32
            }
        } else {
            widened_op_type(IrType::I32)
        };
        let op_ty = if common_ty.is_float() {
            common_ty
        } else {
            widened_op_type(IrType::I32)
        };

        let rhs_promoted = self.emit_implicit_cast(real_part, real_ty, op_ty);

        if let Some(lv) = self.lower_lvalue_for_mutation(lhs) {
            let loaded = self.load_lvalue_typed(&lv, ty);
            let loaded_promoted = self.emit_implicit_cast(loaded, ty, op_ty);
            let is_unsigned = self.infer_expr_type(lhs).is_unsigned();
            let ir_op = Self::binop_to_ir(*op, is_unsigned);
            let result = self.emit_binop_val(ir_op, loaded_promoted, rhs_promoted, op_ty);
            let result_cast = self.emit_implicit_cast(Operand::Value(result), op_ty, ty);
            self.store_lvalue_typed(&lv, result_cast, ty);
            return result_cast;
        }
        Operand::Const(IrConst::I64(0))
    }

    /// Standard scalar compound assignment.
    fn lower_scalar_compound_assign(&mut self, op: &BinOp, lhs: &Expr, rhs: &Expr) -> Operand {
        // C23 decimals and _Float128: route Add/Sub/Mul/Div through the
        // libbid / soft-float helpers. Both live in integer carriers, so raw
        // IR arithmetic or casts on them are a silent miscompile (`q += 1`
        // added 1 to the bit pattern). Remainder, bitwise and shift operands
        // of these types are constraint violations diagnosed by sema.
        if matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div) {
            let lhs_ct = self.expr_ctype(lhs);
            let rhs_ct = self.expr_ctype(rhs);
            if lhs_ct.is_soft_float() || rhs_ct.is_soft_float() {
                let common_ct = CType::usual_arithmetic_conversion(&lhs_ct, &rhs_ct);
                return self
                    .lower_libcall_compound_assign(op, lhs, rhs, &lhs_ct, &rhs_ct, &common_ct);
            }
        }
        let ty = self.get_expr_type(lhs);
        let lhs_ir_ty = self.infer_expr_type(lhs);
        let rhs_ir_ty = self.infer_expr_type(rhs);
        let rhs_ty = self.get_expr_type(rhs);

        let (common_ty, op_ty) =
            Self::usual_arithmetic_conversions(ty, rhs_ty, lhs_ir_ty, rhs_ir_ty);

        let rhs_val = self.lower_expr_with_type(rhs, op_ty);
        if let Some(lv) = self.lower_lvalue_for_mutation(lhs) {
            let loaded = self.load_lvalue_typed(&lv, ty);
            let is_shift = matches!(op, BinOp::Shl | BinOp::Shr);
            let loaded_promoted =
                self.promote_for_op(loaded, ty, lhs_ir_ty, op_ty, common_ty, is_shift);

            let is_unsigned = if is_shift {
                Self::integer_promote(lhs_ir_ty).is_unsigned()
            } else {
                common_ty.is_unsigned()
            };
            let ir_op = Self::binop_to_ir(*op, is_unsigned);

            // Scale RHS for pointer += and -=. Match the plain pointer +/-
            // lowering: widen the index to pointer width before multiplying
            // by the element size, preserving signedness. Without this,
            // `p += signed_i32_negative` is multiplied as a 64-bit value with
            // stale/zero high bits and can walk far past the object (SQLite's
            // yy_reduce shape: stack pointer += signed-char table entry + 1).
            let actual_rhs = if ty == IrType::Ptr && matches!(op, BinOp::Add | BinOp::Sub) {
                let ptr_int_ty = crate::common::types::target_int_ir_type();
                let rhs_wide = self.emit_implicit_cast(rhs_val, rhs_ir_ty, ptr_int_ty);
                // A pointer to a VLA scales by its runtime pointee size.
                match self.get_vla_stride_for_subscript(lhs) {
                    Some(stride) => Operand::Value(self.emit_binop_val(
                        IrBinOp::Mul,
                        rhs_wide,
                        Operand::Value(stride),
                        ptr_int_ty,
                    )),
                    None => {
                        let elem_size = self.get_pointer_elem_size_from_expr(lhs);
                        self.scale_index(rhs_wide, elem_size)
                    }
                }
            } else {
                rhs_val
            };

            let result = self.emit_binop_val(ir_op, loaded_promoted, actual_rhs, op_ty);

            // Conversion to _Bool is `result != 0` in the operation type
            // (C11 6.3.1.2): normalizing after narrowing truncated first, so
            // `b += 0.5` stored 0.
            let store_val = if self.is_bool_lvalue(lhs) {
                let truth = self.emit_bool_normalize_typed(Operand::Value(result), op_ty);
                self.emit_implicit_cast(truth, IrType::I32, ty)
            } else {
                self.narrow_from_op(Operand::Value(result), ty, lhs_ir_ty, op_ty)
            };
            self.store_lvalue_typed(&lv, store_val, ty);
            return store_val;
        }
        rhs_val
    }

    // -----------------------------------------------------------------------
    // Usual arithmetic conversions helpers
    // -----------------------------------------------------------------------

    /// Compute the common type and operation type for a binary operation
    /// using C's "usual arithmetic conversions".
    pub(super) fn usual_arithmetic_conversions(
        lhs_ty: IrType,
        rhs_ty: IrType,
        lhs_ir_ty: IrType,
        rhs_ir_ty: IrType,
    ) -> (IrType, IrType) {
        let common_ty = if lhs_ty.is_float() || rhs_ty.is_float() {
            if lhs_ty == IrType::F128 || rhs_ty == IrType::F128 {
                IrType::F128
            } else if lhs_ty == IrType::F64 || rhs_ty == IrType::F64 {
                IrType::F64
            } else {
                IrType::F32
            }
        } else {
            Self::common_type(
                Self::integer_promote(lhs_ir_ty),
                Self::integer_promote(rhs_ir_ty),
            )
        };
        let op_ty = if common_ty.is_float() {
            common_ty
        } else {
            widened_op_type(common_ty)
        };
        (common_ty, op_ty)
    }

    /// Promote a loaded value to the operation type for compound assignment.
    pub(super) fn promote_for_op(
        &mut self,
        loaded: Operand,
        val_ty: IrType,
        ir_ty: IrType,
        op_ty: IrType,
        common_ty: IrType,
        is_shift: bool,
    ) -> Operand {
        if val_ty != op_ty && op_ty.is_float() && !val_ty.is_float() {
            // Int to float: cast from the widened int type
            let wt = widened_op_type(ir_ty);
            let cast_from = if ir_ty.is_unsigned() {
                if wt == IrType::I32 {
                    IrType::U32
                } else {
                    IrType::U64
                }
            } else {
                wt
            };
            Operand::Value(self.emit_cast_val(loaded, cast_from, op_ty))
        } else if val_ty != op_ty && val_ty.is_float() && op_ty.is_float() {
            Operand::Value(self.emit_cast_val(loaded, val_ty, op_ty))
        } else if !op_ty.is_float() && ir_ty.size() < op_ty.size() {
            // Widen the LHS to the operation type.
            if op_ty == IrType::I128 || op_ty == IrType::U128 {
                // Widen to 128-bit: first promote to I64/U64, then cast to 128-bit
                let from_64 = if ir_ty.is_unsigned() || common_ty.is_unsigned() {
                    IrType::U64
                } else {
                    IrType::I64
                };
                let loaded_64 = if ir_ty.size() < 8 {
                    Operand::Value(self.emit_cast_val(loaded, ir_ty, from_64))
                } else {
                    loaded
                };
                Operand::Value(self.emit_cast_val(loaded_64, from_64, op_ty))
            } else {
                let extend_unsigned = if is_shift {
                    ir_ty.is_unsigned()
                } else {
                    common_ty.is_unsigned() || ir_ty.is_unsigned()
                };
                let from_ty = if extend_unsigned {
                    match ir_ty {
                        IrType::I32 => IrType::U32,
                        IrType::I16 => IrType::U16,
                        IrType::I8 => IrType::U8,
                        _ => ir_ty,
                    }
                } else {
                    ir_ty
                };
                Operand::Value(self.emit_cast_val(loaded, from_ty, op_ty))
            }
        } else {
            loaded
        }
    }

    /// Narrow a result from operation type back to the target type.
    pub(super) fn narrow_from_op(
        &mut self,
        result: Operand,
        target_ty: IrType,
        target_ir_ty: IrType,
        op_ty: IrType,
    ) -> Operand {
        if op_ty.is_float() && !target_ty.is_float() {
            // Float to int: cast to widened int type first, then narrow if needed
            let wt = widened_op_type(target_ir_ty);
            let cast_to = if target_ir_ty.is_unsigned() {
                if wt == IrType::I32 {
                    IrType::U32
                } else {
                    IrType::U64
                }
            } else {
                wt
            };
            let dest = self.emit_cast_val(result, op_ty, cast_to);
            if target_ir_ty.size() < cast_to.size() {
                Operand::Value(self.emit_cast_val(Operand::Value(dest), cast_to, target_ir_ty))
            } else {
                Operand::Value(dest)
            }
        } else if op_ty.is_float() && target_ty.is_float() && op_ty != target_ty {
            Operand::Value(self.emit_cast_val(result, op_ty, target_ty))
        } else if !op_ty.is_float() && target_ir_ty.size() < op_ty.size() {
            // Narrow from operation type to target type.
            if op_ty == IrType::I128 || op_ty == IrType::U128 {
                // Truncate from 128-bit to the target type
                let to_64 = if target_ir_ty.is_unsigned() {
                    IrType::U64
                } else {
                    IrType::I64
                };
                let narrowed = self.emit_cast_val(result, op_ty, to_64);
                if target_ir_ty.size() < 8 {
                    Operand::Value(self.emit_cast_val(
                        Operand::Value(narrowed),
                        to_64,
                        target_ir_ty,
                    ))
                } else {
                    Operand::Value(narrowed)
                }
            } else {
                Operand::Value(self.emit_cast_val(result, op_ty, target_ir_ty))
            }
        } else {
            result
        }
    }

    // ===== Vector type helpers =====

    /// Get the IR type for a vector element CType.
    pub(super) fn vector_elem_ir_type(elem_ct: &CType) -> IrType {
        IrType::from_ctype(elem_ct)
    }

    /// Lower vector assignment (a = b): memcpy the whole vector.
    /// When the RHS is a function call returning a small vector (<=8 bytes),
    /// the return value is packed into a register (I64), not a pointer.
    /// We detect this case and spill the packed value to an alloca first.
    fn lower_vector_assign(&mut self, lhs: &Expr, rhs: &Expr, lhs_ct: &CType) -> Operand {
        let total_size = lhs_ct.size();
        let lhs_ptr = self.lower_expr(lhs);
        let lhs_ptr_val = self.operand_to_value(lhs_ptr);

        // Scalar right operand: Clang splats it over every lane of an
        // ext_vector_type lvalue (sema accepts this shape ONLY for ext
        // vectors; plain GNU vectors are diagnosed). It is a VALUE
        // operation: convert the scalar to the lane type and store it into
        // each lane. The generic Memcpy path below would treat the scalar's
        // register value as the SOURCE ADDRESS of the copy — `v = 2.5f`
        // accepted but segfaulting at runtime before this branch existed —
        // so a scalar RHS must never reach it. Mirrors the splat in
        // `try_lower_swizzle_assign` and `copy_vector_expr_to_alloca`.
        let rhs_ct = self.expr_ctype(rhs);
        if !rhs_ct.is_vector() {
            if let CType::ExtVector(elem_ct, layout) = lhs_ct {
                let elem_ir_ty = Self::vector_elem_ir_type(elem_ct);
                let raw = self.lower_expr(rhs);
                let raw_ty = self.value_ir_type(rhs);
                let value = self.emit_implicit_cast(raw, raw_ty, elem_ir_ty);
                let addr_space = self.aggregate_lvalue_space(lhs);
                let volatile = self.expr_access_is_volatile(lhs);
                for lane in 0..layout.lanes {
                    self.store_vector_lane(
                        lhs_ptr_val,
                        lhs_ct,
                        lane,
                        value.clone(),
                        elem_ir_ty,
                        addr_space,
                        volatile,
                    );
                }
                return Operand::Value(lhs_ptr_val);
            }
        }

        // Packed whole-object copy (`vector_simd.rs`); total fallback below.
        if self.try_lower_vector_assign_simd(lhs, lhs_ptr_val, rhs, lhs_ct) {
            return Operand::Value(lhs_ptr_val);
        }

        // Check if the RHS is a function call returning a small vector in a register.
        // Small vectors (<=8 bytes) are returned as packed I64 values, not pointers.
        let rhs_is_small_vec_call = self.rhs_is_small_vector_call(rhs, total_size);

        let rhs_val = self.lower_aggregate_rvalue(rhs);
        let rhs_ptr_val = if rhs_is_small_vec_call {
            // The return value is packed vector data in a register.
            // Spill it to an alloca so we can memcpy from it.
            let alloca = self.fresh_value();
            let store_ty = Self::packed_store_type(total_size);
            self.emit(Instruction::Alloca {
                dest: alloca,
                size: total_size,
                ty: store_ty,
                align: 0,
                volatile: false,
                semantic_volatile: false,
            });
            self.emit(Instruction::Store {
                volatile: false,
                val: rhs_val,
                ptr: alloca,
                ty: store_ty,
                seg_override: AddressSpace::Default,
            });
            alloca
        } else {
            self.operand_to_value(rhs_val)
        };
        // A vector object in a named address space is written through its
        // segment; the generic Memcpy would store to the wrong memory.
        let dest_space = self.aggregate_lvalue_space(lhs);
        if dest_space == AddressSpace::Default {
            self.emit(Instruction::Memcpy {
                dest: lhs_ptr_val,
                src: rhs_ptr_val,
                size: total_size,
            });
        } else {
            self.emit_segment_copy(
                lhs_ptr_val,
                dest_space,
                rhs_ptr_val,
                AddressSpace::Default,
                total_size,
                self.expr_access_is_volatile(lhs),
            );
        }
        Operand::Value(lhs_ptr_val)
    }

    /// Check if an expression is a function call returning packed scalar vector data.
    pub(super) fn rhs_is_small_vector_call(&self, rhs: &Expr, vec_size: usize) -> bool {
        match rhs {
            Expr::FunctionCall(func_expr, _, _) => self.call_result_is_packed(func_expr, vec_size),
            _ => false,
        }
    }

    /// Materialize a vector rvalue as an address. Small vector-returning calls
    /// use a packed register carrier; elementwise operations need an address to
    /// inspect its lanes, so spill those carriers using the exact object size.
    pub(super) fn vector_operand_ptr(&mut self, expr: &Expr, vector_ct: &CType) -> Value {
        let lowered = self.lower_aggregate_rvalue(expr);
        if self.rhs_is_small_vector_call(expr, vector_ct.size()) {
            let size = vector_ct.size();
            let align = self.ctype_align(vector_ct);
            let alloca = self.emit_entry_alloca(IrType::Ptr, size, align, false);
            let store_ty = Self::packed_store_type(size);
            self.emit(Instruction::Store {
                volatile: false,
                val: lowered,
                ptr: alloca,
                ty: store_ty,
                seg_override: AddressSpace::Default,
            });
            alloca
        } else {
            self.operand_to_value(lowered)
        }
    }

    /// Lower vector compound assignment (a += b, a -= b, etc.): element-wise operation.
    /// The RHS may be a scalar, which is broadcast to all vector elements.
    fn lower_vector_compound_assign(
        &mut self,
        op: &BinOp,
        lhs: &Expr,
        rhs: &Expr,
        lhs_ct: &CType,
    ) -> Operand {
        let (elem_ct, num_elems) = match lhs_ct.vector_info() {
            Some((elem_ct, num_elems)) => (elem_ct.clone(), num_elems),
            None => return Operand::Const(IrConst::I64(0)),
        };
        if let Some(packed) = self.try_lower_vector_compound_assign_simd(op, lhs, rhs, lhs_ct) {
            return packed;
        }
        let elem_ir_ty = Self::vector_elem_ir_type(&elem_ct);
        let is_unsigned = elem_ct.is_unsigned();
        let ir_op = Self::binop_to_ir(*op, is_unsigned);

        // LHS remains an lvalue in its own address space. Lane helpers handle
        // both byte-strided numeric vectors and bit-packed bool ext-vectors.
        let lhs_ptr = self.lower_expr(lhs);
        let lhs_ptr_val = self.operand_to_value(lhs_ptr);
        let lhs_space = self.get_addr_space_of_struct_expr(lhs);
        let lhs_volatile = self.expr_access_is_volatile(lhs);

        let rhs_ct = self.expr_ctype(rhs);
        let rhs_is_vector = rhs_ct.is_vector();
        let rhs_val = if rhs_is_vector {
            self.vector_operand_ptr(rhs, &rhs_ct)
        } else {
            let lowered = self.lower_expr(rhs);
            self.operand_to_value(lowered)
        };
        let rhs_scalar = if !rhs_is_vector {
            let src_ty = self.value_ir_type(rhs);
            Some(self.emit_implicit_cast(Operand::Value(rhs_val), src_ty, elem_ir_ty))
        } else {
            None
        };

        for lane in 0..num_elems {
            let lhs_elem =
                self.load_vector_lane(lhs_ptr_val, lhs_ct, lane, lhs_space, lhs_volatile);
            let rhs_elem = if let Some(scalar) = rhs_scalar {
                scalar
            } else {
                self.load_vector_lane(rhs_val, &rhs_ct, lane, AddressSpace::Default, false)
            };
            let result = self.emit_binop_val(ir_op, lhs_elem, rhs_elem, elem_ir_ty);
            self.store_vector_lane(
                lhs_ptr_val,
                lhs_ct,
                lane,
                Operand::Value(result),
                elem_ir_ty,
                lhs_space,
                lhs_volatile,
            );
        }
        Operand::Value(lhs_ptr_val)
    }

    /// Lower vector binary operation (a + b, a - b, etc.): element-wise, returns new alloca.
    /// Per GCC vector extensions, if one operand is a scalar and the other is a
    /// vector, the scalar is broadcast (splatted) to all elements of the vector.
    pub(super) fn lower_vector_binary_op(
        &mut self,
        op: &BinOp,
        lhs: &Expr,
        rhs: &Expr,
        vec_ct: &CType,
    ) -> Operand {
        let (elem_ct, num_elems) = match vec_ct.vector_info() {
            Some((elem_ct, num_elems)) => (elem_ct.clone(), num_elems),
            None => return Operand::Const(IrConst::I64(0)),
        };
        // Whole-vector form first (see `vector_simd.rs`); the lane loop
        // below is the exact fallback for every shape it declines.
        if let Some(packed) = self.try_lower_vector_binary_simd(op, lhs, rhs, vec_ct) {
            return packed;
        }
        let elem_ir_ty = Self::vector_elem_ir_type(&elem_ct);
        let total_size = vec_ct.size();
        let result_align = self.ctype_align(vec_ct);
        let is_unsigned = elem_ct.is_unsigned();
        let ir_op = Self::binop_to_ir(*op, is_unsigned);

        let lhs_ct = self.expr_ctype(lhs);
        let rhs_ct = self.expr_ctype(rhs);
        let lhs_is_vector = lhs_ct.is_vector();
        let rhs_is_vector = rhs_ct.is_vector();

        let result_alloca = self.emit_entry_alloca(IrType::Ptr, total_size, result_align, false);
        // Padded bytes are not logical lanes, and packed bool stores use
        // read-modify-write. Initialize the complete object before lane writes
        // when either can be observed; see `vector_temp_needs_zero_init`.
        if crate::ir::lowering::lower::vector_temp_needs_zero_init(vec_ct, num_elems) {
            self.zero_init_alloca(result_alloca, total_size);
        }

        let lhs_val = if lhs_is_vector {
            self.vector_operand_ptr(lhs, &lhs_ct)
        } else {
            let lowered = self.lower_expr(lhs);
            self.operand_to_value(lowered)
        };
        let rhs_val = if rhs_is_vector {
            self.vector_operand_ptr(rhs, &rhs_ct)
        } else {
            let lowered = self.lower_expr(rhs);
            self.operand_to_value(lowered)
        };

        let lhs_scalar = if !lhs_is_vector {
            let src_ty = self.value_ir_type(lhs);
            Some(self.emit_implicit_cast(Operand::Value(lhs_val), src_ty, elem_ir_ty))
        } else {
            None
        };
        let rhs_scalar = if !rhs_is_vector {
            let src_ty = self.value_ir_type(rhs);
            Some(self.emit_implicit_cast(Operand::Value(rhs_val), src_ty, elem_ir_ty))
        } else {
            None
        };
        // `vector_operand_ptr` returns an address produced by
        // `lower_aggregate_rvalue`, which copies a segment-resident object into
        // a generic temporary (see `seg_copy::copy_segment_to_temp`). The
        // materialized operand therefore lives in the DEFAULT address space:
        // applying the expression's own segment to it would read `%gs:` at a
        // stack address and fault (caught by
        // tests/regression/seg_gs_runtime/seg_gs_audit.c).
        let lhs_volatile = self.expr_access_is_volatile(lhs);
        let rhs_volatile = self.expr_access_is_volatile(rhs);

        for lane in 0..num_elems {
            let lhs_elem = if let Some(scalar) = lhs_scalar {
                scalar
            } else {
                self.load_vector_lane(lhs_val, &lhs_ct, lane, AddressSpace::Default, lhs_volatile)
            };
            let rhs_elem = if let Some(scalar) = rhs_scalar {
                scalar
            } else {
                self.load_vector_lane(rhs_val, &rhs_ct, lane, AddressSpace::Default, rhs_volatile)
            };
            let result = self.emit_binop_val(ir_op, lhs_elem, rhs_elem, elem_ir_ty);
            self.store_vector_lane(
                result_alloca,
                vec_ct,
                lane,
                Operand::Value(result),
                elem_ir_ty,
                AddressSpace::Default,
                false,
            );
        }
        Operand::Value(result_alloca)
    }

    /// Lower an element-wise vector comparison (`==`, `!=`, `<`, `<=`, `>`,
    /// `>=`) to per-lane integer compares that produce an all-ones/all-zeros
    /// mask in the element type, matching GCC's vector-extension semantics.
    ///
    /// GCC vector comparison: each lane compares the corresponding element
    /// (with a scalar operand broadcast/splatted to all lanes) and yields a
    /// vector whose element is all-ones when the predicate is true and
    /// all-zeros otherwise.  This is the vector analogue of
    /// `lower_vector_binary_op`.
    ///
    /// Comparisons must take this path rather than the scalar/pointer
    /// fall-through in `lower_binary_op`: the fall-through lowers both operands
    /// to values and emits a single `Cmp`.  For a vector operand the lowered
    /// value is the vector's stack address, so the comparison would test the
    /// *pointer* against the scalar — miscompiling e.g. `((V){0} <= 0)` (PR
    /// 110817) into `ptr <= 0` and dereferencing the result mask as an address.
    pub(super) fn lower_vector_cmp(
        &mut self,
        op: BinOp,
        lhs: &Expr,
        rhs: &Expr,
        vec_ct: &CType,
    ) -> Operand {
        let (elem_ct, num_elems) = match vec_ct.vector_info() {
            Some((elem_ct, num_elems)) => (elem_ct.clone(), num_elems),
            None => return Operand::Const(IrConst::I64(0)),
        };
        if let Some(packed) = self.try_lower_vector_cmp_simd(op, lhs, rhs, vec_ct) {
            return packed;
        }
        let elem_ir_ty = Self::vector_elem_ir_type(&elem_ct);
        let total_size = vec_ct.size();
        let result_alloca =
            self.emit_entry_alloca(IrType::Ptr, total_size, self.ctype_align(vec_ct), false);
        if crate::ir::lowering::lower::vector_temp_needs_zero_init(vec_ct, num_elems) {
            self.zero_init_alloca(result_alloca, total_size);
        }

        let lhs_ct = self.expr_ctype(lhs);
        let rhs_ct = self.expr_ctype(rhs);
        let lhs_is_vector = lhs_ct.is_vector();
        let rhs_is_vector = rhs_ct.is_vector();
        let lhs_val = if lhs_is_vector {
            self.vector_operand_ptr(lhs, &lhs_ct)
        } else {
            let lowered = self.lower_expr(lhs);
            self.operand_to_value(lowered)
        };
        let rhs_val = if rhs_is_vector {
            self.vector_operand_ptr(rhs, &rhs_ct)
        } else {
            let lowered = self.lower_expr(rhs);
            self.operand_to_value(lowered)
        };
        let lhs_scalar = if !lhs_is_vector {
            let src_ty = self.value_ir_type(lhs);
            Some(self.emit_implicit_cast(Operand::Value(lhs_val), src_ty, elem_ir_ty))
        } else {
            None
        };
        let rhs_scalar = if !rhs_is_vector {
            let src_ty = self.value_ir_type(rhs);
            Some(self.emit_implicit_cast(Operand::Value(rhs_val), src_ty, elem_ir_ty))
        } else {
            None
        };
        // `vector_operand_ptr` returns an address produced by
        // `lower_aggregate_rvalue`, which copies a segment-resident object into
        // a generic temporary (see `seg_copy::copy_segment_to_temp`). The
        // materialized operand therefore lives in the DEFAULT address space:
        // applying the expression's own segment to it would read `%gs:` at a
        // stack address and fault (caught by
        // tests/regression/seg_gs_runtime/seg_gs_audit.c).
        let lhs_volatile = self.expr_access_is_volatile(lhs);
        let rhs_volatile = self.expr_access_is_volatile(rhs);
        let is_unsigned = elem_ct.is_unsigned();
        let cmp_op = Self::binop_to_cmp(op, is_unsigned);
        // The result is a vector of signed integer lanes of the element's
        // width (`CType::vector_comparison_result`): a float lane compares
        // as F32/F64 but its all-ones mask is an I32/I64 bit pattern, not
        // the float value `-1.0`.
        let mask_ct = vec_ct
            .vector_comparison_result()
            .unwrap_or_else(|| vec_ct.clone());
        let mask_ir_ty = match elem_ir_ty {
            IrType::F32 => IrType::I32,
            IrType::F64 => IrType::I64,
            other => other,
        };

        for lane in 0..num_elems {
            let lhs_elem = if let Some(scalar) = lhs_scalar {
                scalar
            } else {
                self.load_vector_lane(lhs_val, &lhs_ct, lane, AddressSpace::Default, lhs_volatile)
            };
            let rhs_elem = if let Some(scalar) = rhs_scalar {
                scalar
            } else {
                self.load_vector_lane(rhs_val, &rhs_ct, lane, AddressSpace::Default, rhs_volatile)
            };
            // Lane predicate -> all-ones/all-zeros mask. `Cmp` yields an I32
            // 0/1 boolean (an ordered compare for float lanes, as for scalar
            // `<`: NaN lanes are false, except `!=`); widen it to the
            // same-width signed integer lane, then `0 - x` is all-ones for a
            // true lane and 0 for a false one, at any lane width.
            let cond = self.emit_cmp_val(cmp_op, lhs_elem, rhs_elem, elem_ir_ty);
            let cond_ext = self.emit_cast_val(Operand::Value(cond), IrType::I32, mask_ir_ty);
            let mask = self.emit_binop_val(
                IrBinOp::Sub,
                Operand::Const(Self::ir_zero_const(mask_ir_ty)),
                Operand::Value(cond_ext),
                mask_ir_ty,
            );
            self.store_vector_lane(
                result_alloca,
                &mask_ct,
                lane,
                Operand::Value(mask),
                mask_ir_ty,
                AddressSpace::Default,
                false,
            );
        }
        Operand::Value(result_alloca)
    }

    /// Lower a unary vector operation (`-` negation, `~` bitwise-not) to an
    /// element-wise operation, producing a result vector.  Without this the
    /// generic scalar path in `lower_unary_op` applies the operation to the
    /// vector's stack *address*, so `~v` becomes `~&v` and the downstream
    /// memcpy/load further misuses it (PR 110817: `~((V){0} <= 0)` segfaulted
    /// because `Not` was applied to the alloca pointer).
    pub(super) fn lower_vector_unary(
        &mut self,
        ir_unary: IrUnaryOp,
        inner: &Expr,
        vec_ct: &CType,
    ) -> Operand {
        let (elem_ct, num_elems) = match vec_ct.vector_info() {
            Some((elem_ct, num_elems)) => (elem_ct.clone(), num_elems),
            None => return Operand::Const(IrConst::I64(0)),
        };
        if let Some(packed) = self.try_lower_vector_unary_simd(ir_unary, inner, vec_ct) {
            return packed;
        }
        let elem_ir_ty = Self::vector_elem_ir_type(&elem_ct);
        let total_size = vec_ct.size();
        let result_alloca =
            self.emit_entry_alloca(IrType::Ptr, total_size, self.ctype_align(vec_ct), false);
        if crate::ir::lowering::lower::vector_temp_needs_zero_init(vec_ct, num_elems) {
            self.zero_init_alloca(result_alloca, total_size);
        }

        let src_ct = self.expr_ctype(inner);
        let val = self.vector_operand_ptr(inner, &src_ct);
        // `vector_operand_ptr` returns an address produced by
        // `lower_aggregate_rvalue`, which copies a segment-resident object into
        // a generic temporary (see `seg_copy::copy_segment_to_temp`). The
        // materialized operand therefore lives in the DEFAULT address space:
        // applying the expression's own segment to it would read `%gs:` at a
        // stack address and fault (caught by
        // tests/regression/seg_gs_runtime/seg_gs_audit.c).
        let src_volatile = self.expr_access_is_volatile(inner);

        for lane in 0..num_elems {
            let elem =
                self.load_vector_lane(val, &src_ct, lane, AddressSpace::Default, src_volatile);
            // A bool vector's bitwise NOT is one-bit NOT, not an eight-bit
            // complement followed by truth normalization (which would make
            // both 0 and 1 true). XOR with one expresses the lane operation.
            let result_elem =
                if vec_ct.is_bitpacked_bool_vector() && matches!(ir_unary, IrUnaryOp::Not) {
                    let elem = self.emit_implicit_cast(elem, elem_ir_ty, IrType::U8);
                    Operand::Value(self.emit_binop_val(
                        IrBinOp::Xor,
                        elem,
                        Operand::Const(IrConst::I64(1)),
                        IrType::U8,
                    ))
                } else {
                    let dest = self.fresh_value();
                    self.emit(Instruction::UnaryOp {
                        dest,
                        op: ir_unary,
                        src: elem,
                        ty: elem_ir_ty,
                    });
                    Operand::Value(dest)
                };
            self.store_vector_lane(
                result_alloca,
                vec_ct,
                lane,
                result_elem,
                elem_ir_ty,
                AddressSpace::Default,
                false,
            );
        }
        Operand::Value(result_alloca)
    }

    /// Build an integer zero constant for the given IR type. Element types we
    /// lower vector compares on are always integer, so this is total over the
    /// integer set; anything else falls back to I64(0) defensively.
    fn ir_zero_const(ty: IrType) -> IrConst {
        match ty {
            IrType::I8 => IrConst::I8(0),
            IrType::I16 => IrConst::I16(0),
            IrType::I32 => IrConst::I32(0),
            IrType::I128 => IrConst::I128(0),
            _ => IrConst::I64(0),
        }
    }
}
