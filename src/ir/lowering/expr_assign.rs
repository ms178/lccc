//! Assignment and compound assignment lowering, including bitfield operations.
//!
//! Extracted from expr.rs. This module handles:
//! - `lower_assign`: simple assignment (lhs = rhs)
//! - `lower_struct_assign`: struct/union copy via memcpy
//! - `lower_compound_assign`: compound assignment (+=, -=, *=, etc.)
//! - Bitfield helpers: resolve_bitfield_lvalue, store_bitfield, extract_bitfield
//! - Arithmetic conversion helpers: usual_arithmetic_conversions, promote_for_op, narrow_from_op

use super::lower::Lowerer;
use crate::common::types::{widened_op_type, AddressSpace, CType, IrType, SsoMode};
use crate::frontend::parser::ast::{BinOp, Expr};
use crate::ir::reexports::{Instruction, IrBinOp, IrCmpOp, IrConst, IrUnaryOp, Operand, Value};

impl Lowerer {
    pub(super) fn lower_assign(&mut self, lhs: &Expr, rhs: &Expr) -> Operand {
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
        let lv = self.lower_lvalue(lhs);

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
            self.emit_bool_normalize_typed(rhs_val, rhs_ty)
        } else {
            let rhs_val = self.lower_expr(rhs);
            let rhs_ty = self.value_ir_type(rhs);
            if lhs_ct == CType::Float128 || rhs_ct == CType::Float128 {
                // _Float128 assignments route through the soft-float helpers.
                self.convert_scalar_ctype(rhs_val, rhs_ty, &rhs_ct, &lhs_ct)
            } else {
                self.emit_implicit_cast(rhs_val, rhs_ty, lhs_ty)
            }
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
            if let Some(lv) = self.lower_lvalue(lhs) {
                let dest_addr = self.lvalue_addr(&lv);
                self.store_packed_data_exact(dest_addr, rhs_val, struct_size);
                return Operand::Value(dest_addr);
            }
            return rhs_val;
        }

        let rhs_val = self.lower_expr(rhs);
        if let Some(lv) = self.lower_lvalue(lhs) {
            let dest_addr = self.lvalue_addr(&lv);
            let src_addr = self.operand_to_value(rhs_val);
            if let Some(size_val) = dynamic_size {
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
    ) -> Option<(Value, IrType, u32, u32, SsoMode)> {
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

        Some((field_addr, storage_ty, bit_offset, bit_width, sso))
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
            if is_signed {
                IrType::I64
            } else {
                IrType::U64
            }
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
        let (field_addr, storage_ty, bit_offset, bit_width, sso) =
            self.resolve_bitfield_lvalue(lhs)?;
        let is_bool = self.is_bool_lvalue(lhs);
        let rhs_val = self.lower_expr(rhs);
        let rhs_ty = self.value_ir_type(rhs);
        // C standard 6.3.1.2: assigning to _Bool converts the value to 0 or 1
        // (any nonzero becomes 1) BEFORE bitfield truncation. Without this,
        // e.g. `s.bool_bf = 2` would mask 2 (0b10) to 1 bit = 0, not 1.
        let store_val = if is_bool {
            let normalized = self.emit_bool_normalize_typed(rhs_val, rhs_ty);
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
        );
        Some(self.truncate_to_bitfield_value(
            store_val,
            storage_ty,
            bit_width,
            storage_ty.is_signed(),
        ))
    }

    /// Try to lower compound assignment to a bitfield member (e.g., s.bf += val).
    pub(super) fn try_lower_bitfield_compound_assign(
        &mut self,
        op: &BinOp,
        lhs: &Expr,
        rhs: &Expr,
    ) -> Option<Operand> {
        let (field_addr, storage_ty, bit_offset, bit_width, sso) =
            self.resolve_bitfield_lvalue(lhs)?;
        let is_bool = self.is_bool_lvalue(lhs);

        let current_val =
            self.extract_bitfield_from_addr(field_addr, storage_ty, bit_offset, bit_width, sso);
        let current_ty = crate::ir::lowering::expr_types::bitfield_promoted_type(
            storage_ty,
            Some((bit_offset, bit_width)),
        );

        let rhs_val = self.lower_expr(rhs);
        let rhs_ty = self.value_ir_type(rhs);

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
                volatile: false,
                val: stored,
                ptr: addr,
                ty: storage_ty,
                seg_override: AddressSpace::Default,
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
            seg_override: AddressSpace::Default,
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
            seg_override: AddressSpace::Default,
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
            volatile: false,
            dest: old_low,
            ptr: addr,
            ty: storage_ty,
            seg_override: AddressSpace::Default,
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
            volatile: false,
            val: new_low_fixed,
            ptr: addr,
            ty: storage_ty,
            seg_override: AddressSpace::Default,
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
            volatile: false,
            dest: old_high,
            ptr: high_addr,
            ty: storage_ty,
            seg_override: AddressSpace::Default,
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
            volatile: false,
            val: new_high_fixed,
            ptr: high_addr,
            ty: storage_ty,
            seg_override: AddressSpace::Default,
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
                volatile: false,
                dest: low_loaded,
                ptr: addr,
                ty: storage_ty,
                seg_override: AddressSpace::Default,
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
                volatile: false,
                dest: high_loaded,
                ptr: high_addr,
                ty: storage_ty,
                seg_override: AddressSpace::Default,
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
                volatile: false,
                dest: loaded,
                ptr: addr,
                ty: storage_ty,
                seg_override: AddressSpace::Default,
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

        let op_lhs_ptr = if result_ct != *lhs_ct {
            let lhs_converted =
                self.convert_to_complex(Operand::Value(lhs_ptr), lhs_ct, &result_ct);
            self.operand_to_value(lhs_converted)
        } else {
            lhs_ptr
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
        let comp_size = Self::complex_component_size(lhs_ct);
        self.emit(Instruction::Memcpy {
            dest: lhs_ptr,
            src: store_ptr,
            size: comp_size * 2,
        });

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

        if let Some(lv) = self.lower_lvalue(lhs) {
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
        let ty = self.get_expr_type(lhs);
        let lhs_ir_ty = self.infer_expr_type(lhs);
        let rhs_ir_ty = self.infer_expr_type(rhs);
        let rhs_ty = self.get_expr_type(rhs);

        let (common_ty, op_ty) =
            Self::usual_arithmetic_conversions(ty, rhs_ty, lhs_ir_ty, rhs_ir_ty);

        let rhs_val = self.lower_expr_with_type(rhs, op_ty);
        if let Some(lv) = self.lower_lvalue(lhs) {
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
                let elem_size = self.get_pointer_elem_size_from_expr(lhs);
                let ptr_int_ty = crate::common::types::target_int_ir_type();
                let rhs_wide = self.emit_implicit_cast(rhs_val, rhs_ir_ty, ptr_int_ty);
                self.scale_index(rhs_wide, elem_size)
            } else {
                rhs_val
            };

            let result = self.emit_binop_val(ir_op, loaded_promoted, actual_rhs, op_ty);

            let store_val = self.narrow_from_op(Operand::Value(result), ty, lhs_ir_ty, op_ty);
            let store_val = if self.is_bool_lvalue(lhs) {
                self.emit_bool_normalize_typed(store_val, op_ty)
            } else {
                store_val
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
        let (_, total_size) = match lhs_ct {
            CType::Vector(_, total_size) => ((), *total_size),
            _ => unreachable!(
                "lower_vector_assign called with non-vector type: {:?}",
                lhs_ct
            ),
        };
        let lhs_ptr = self.lower_expr(lhs);
        let lhs_ptr_val = self.operand_to_value(lhs_ptr);

        // Check if the RHS is a function call returning a small vector in a register.
        // Small vectors (<=8 bytes) are returned as packed I64 values, not pointers.
        let rhs_is_small_vec_call = self.rhs_is_small_vector_call(rhs, total_size);

        let rhs_val = self.lower_expr(rhs);
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
        self.emit(Instruction::Memcpy {
            dest: lhs_ptr_val,
            src: rhs_ptr_val,
            size: total_size,
        });
        Operand::Value(lhs_ptr_val)
    }

    /// Check if an expression is a function call that returns a small vector in a register.
    pub(super) fn rhs_is_small_vector_call(&self, rhs: &Expr, vec_size: usize) -> bool {
        if vec_size > 8 {
            return false; // Large vectors use sret, returned as pointer
        }
        match rhs {
            Expr::FunctionCall(func_expr, _, _) => {
                // Check if it's a direct call to a known function with vector return
                let stripped = {
                    let mut e = func_expr.as_ref();
                    while let Expr::Deref(inner, _) = e {
                        e = inner;
                    }
                    e
                };
                if let Expr::Identifier(name, _) = stripped {
                    if self.is_func_ptr_variable(name) {
                        // Indirect call: small vectors returned in register
                        return true;
                    }
                    if let Some(sig) = self.func_meta.sigs.get(name.as_str()) {
                        // Direct call: check if sret/two_reg are None (small struct/vector return)
                        return sig.sret_size.is_none() && sig.two_reg_ret_size.is_none();
                    }
                    // Unknown function: assume small vector returned in register
                    return true;
                }
                // Indirect call through complex expression
                true
            }
            _ => false,
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
        let elem_ir_ty = Self::vector_elem_ir_type(&elem_ct);
        let elem_size = elem_ct.size();
        let is_unsigned = elem_ct.is_unsigned();

        // Get the IR binary operation
        let ir_op = Self::binop_to_ir(*op, is_unsigned);

        // LHS is always a vector for compound assignment
        let lhs_ptr = self.lower_expr(lhs);
        let lhs_ptr_val = self.operand_to_value(lhs_ptr);

        // RHS may be a scalar or a vector
        let rhs_is_vector = self.expr_ctype(rhs).is_vector();
        let rhs_lowered = self.lower_expr(rhs);
        let rhs_val = self.operand_to_value(rhs_lowered);

        // For scalar RHS, cast to element type once (broadcast value)
        let rhs_scalar = if !rhs_is_vector {
            let src_ty = self.get_expr_type(rhs);
            Some(self.emit_implicit_cast(Operand::Value(rhs_val), src_ty, elem_ir_ty))
        } else {
            None
        };

        // Element-wise operation
        let ptr_int_ty = crate::common::types::target_int_ir_type();
        for i in 0..num_elems {
            let offset = i * elem_size;
            // GEP to element i for LHS
            let lhs_elem_ptr = if offset > 0 {
                self.emit_binop_val(
                    IrBinOp::Add,
                    Operand::Value(lhs_ptr_val),
                    Operand::Const(IrConst::ptr_int(offset as i64)),
                    ptr_int_ty,
                )
            } else {
                lhs_ptr_val
            };
            // Load LHS element
            let lhs_elem = self.fresh_value();
            self.emit(Instruction::Load {
                volatile: false,
                dest: lhs_elem,
                ptr: lhs_elem_ptr,
                ty: elem_ir_ty,
                seg_override: AddressSpace::Default,
            });
            // Get RHS element: splatted scalar or loaded from vector
            let rhs_elem_op = if let Some(ref scalar_op) = rhs_scalar {
                *scalar_op
            } else {
                let rhs_elem_ptr = if offset > 0 {
                    self.emit_binop_val(
                        IrBinOp::Add,
                        Operand::Value(rhs_val),
                        Operand::Const(IrConst::ptr_int(offset as i64)),
                        ptr_int_ty,
                    )
                } else {
                    rhs_val
                };
                let rhs_elem = self.fresh_value();
                self.emit(Instruction::Load {
                    volatile: false,
                    dest: rhs_elem,
                    ptr: rhs_elem_ptr,
                    ty: elem_ir_ty,
                    seg_override: AddressSpace::Default,
                });
                Operand::Value(rhs_elem)
            };
            // Compute result
            let result_elem =
                self.emit_binop_val(ir_op, Operand::Value(lhs_elem), rhs_elem_op, elem_ir_ty);
            // Store back to LHS
            self.emit(Instruction::Store {
                volatile: false,
                val: Operand::Value(result_elem),
                ptr: lhs_elem_ptr,
                ty: elem_ir_ty,
                seg_override: AddressSpace::Default,
            });
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
        let elem_ir_ty = Self::vector_elem_ir_type(&elem_ct);
        let elem_size = elem_ct.size();
        let total_size = match vec_ct {
            CType::Vector(_, ts) => *ts,
            _ => unreachable!(
                "lower_vector_binary_op called with non-vector type: {:?}",
                vec_ct
            ),
        };
        let is_unsigned = elem_ct.is_unsigned();
        let ir_op = Self::binop_to_ir(*op, is_unsigned);

        // Determine if each operand is a vector or scalar.
        let lhs_is_vector = self.expr_ctype(lhs).is_vector();
        let rhs_is_vector = self.expr_ctype(rhs).is_vector();

        // Allocate result vector on stack
        let result_alloca = self.emit_entry_alloca(IrType::Ptr, total_size, total_size, false);

        // Lower operands. Vectors produce pointers; scalars produce values.
        let lhs_lowered = self.lower_expr(lhs);
        let lhs_val = self.operand_to_value(lhs_lowered);
        let rhs_lowered = self.lower_expr(rhs);
        let rhs_val = self.operand_to_value(rhs_lowered);

        // For scalar operands, cast to the element type once (broadcast value).
        let lhs_scalar = if !lhs_is_vector {
            let src_ty = self.get_expr_type(lhs);
            Some(self.emit_implicit_cast(Operand::Value(lhs_val), src_ty, elem_ir_ty))
        } else {
            None
        };
        let rhs_scalar = if !rhs_is_vector {
            let src_ty = self.get_expr_type(rhs);
            Some(self.emit_implicit_cast(Operand::Value(rhs_val), src_ty, elem_ir_ty))
        } else {
            None
        };

        // Element-wise operation
        let ptr_int_ty = crate::common::types::target_int_ir_type();
        for i in 0..num_elems {
            let offset = i * elem_size;

            // Get LHS element: load from vector, or use splatted scalar
            let lhs_elem_op = if let Some(ref scalar_op) = lhs_scalar {
                *scalar_op
            } else {
                let lhs_elem_ptr = if offset > 0 {
                    self.emit_binop_val(
                        IrBinOp::Add,
                        Operand::Value(lhs_val),
                        Operand::Const(IrConst::ptr_int(offset as i64)),
                        ptr_int_ty,
                    )
                } else {
                    lhs_val
                };
                let lhs_elem = self.fresh_value();
                self.emit(Instruction::Load {
                    volatile: false,
                    dest: lhs_elem,
                    ptr: lhs_elem_ptr,
                    ty: elem_ir_ty,
                    seg_override: AddressSpace::Default,
                });
                Operand::Value(lhs_elem)
            };

            // Get RHS element: load from vector, or use splatted scalar
            let rhs_elem_op = if let Some(ref scalar_op) = rhs_scalar {
                *scalar_op
            } else {
                let rhs_elem_ptr = if offset > 0 {
                    self.emit_binop_val(
                        IrBinOp::Add,
                        Operand::Value(rhs_val),
                        Operand::Const(IrConst::ptr_int(offset as i64)),
                        ptr_int_ty,
                    )
                } else {
                    rhs_val
                };
                let rhs_elem = self.fresh_value();
                self.emit(Instruction::Load {
                    volatile: false,
                    dest: rhs_elem,
                    ptr: rhs_elem_ptr,
                    ty: elem_ir_ty,
                    seg_override: AddressSpace::Default,
                });
                Operand::Value(rhs_elem)
            };

            let result_elem_ptr = if offset > 0 {
                self.emit_binop_val(
                    IrBinOp::Add,
                    Operand::Value(result_alloca),
                    Operand::Const(IrConst::ptr_int(offset as i64)),
                    ptr_int_ty,
                )
            } else {
                result_alloca
            };

            // Compute result
            let result_elem = self.emit_binop_val(ir_op, lhs_elem_op, rhs_elem_op, elem_ir_ty);
            // Store to result
            self.emit(Instruction::Store {
                volatile: false,
                val: Operand::Value(result_elem),
                ptr: result_elem_ptr,
                ty: elem_ir_ty,
                seg_override: AddressSpace::Default,
            });
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
        let elem_ir_ty = Self::vector_elem_ir_type(&elem_ct);
        let elem_size = elem_ct.size();
        let total_size = match vec_ct {
            CType::Vector(_, ts) => *ts,
            _ => unreachable!("lower_vector_cmp called with non-vector type: {:?}", vec_ct),
        };
        let is_unsigned = elem_ct.is_unsigned();
        let cmp_op = Self::binop_to_cmp(op, is_unsigned);

        let lhs_is_vector = self.expr_ctype(lhs).is_vector();
        let rhs_is_vector = self.expr_ctype(rhs).is_vector();

        // Allocate the result mask vector on the stack.
        let result_alloca = self.emit_entry_alloca(IrType::Ptr, total_size, total_size, false);

        let lhs_lowered = self.lower_expr(lhs);
        let lhs_val = self.operand_to_value(lhs_lowered);
        let rhs_lowered = self.lower_expr(rhs);
        let rhs_val = self.operand_to_value(rhs_lowered);

        // For a scalar operand, cast it to the element type once (the
        // broadcast value); for a vector operand we load each lane.
        let lhs_scalar = if !lhs_is_vector {
            let src_ty = self.get_expr_type(lhs);
            Some(self.emit_implicit_cast(Operand::Value(lhs_val), src_ty, elem_ir_ty))
        } else {
            None
        };
        let rhs_scalar = if !rhs_is_vector {
            let src_ty = self.get_expr_type(rhs);
            Some(self.emit_implicit_cast(Operand::Value(rhs_val), src_ty, elem_ir_ty))
        } else {
            None
        };

        let ptr_int_ty = crate::common::types::target_int_ir_type();
        for i in 0..num_elems {
            let offset = i * elem_size;

            // LHS element: load the lane from the vector, or reuse the
            // broadcast scalar.
            let lhs_elem_op = if let Some(ref scalar_op) = lhs_scalar {
                *scalar_op
            } else {
                let lhs_elem_ptr = if offset > 0 {
                    self.emit_binop_val(
                        IrBinOp::Add,
                        Operand::Value(lhs_val),
                        Operand::Const(IrConst::ptr_int(offset as i64)),
                        ptr_int_ty,
                    )
                } else {
                    lhs_val
                };
                let lhs_elem = self.fresh_value();
                self.emit(Instruction::Load {
                    volatile: false,
                    dest: lhs_elem,
                    ptr: lhs_elem_ptr,
                    ty: elem_ir_ty,
                    seg_override: AddressSpace::Default,
                });
                Operand::Value(lhs_elem)
            };

            // RHS element.
            let rhs_elem_op = if let Some(ref scalar_op) = rhs_scalar {
                *scalar_op
            } else {
                let rhs_elem_ptr = if offset > 0 {
                    self.emit_binop_val(
                        IrBinOp::Add,
                        Operand::Value(rhs_val),
                        Operand::Const(IrConst::ptr_int(offset as i64)),
                        ptr_int_ty,
                    )
                } else {
                    rhs_val
                };
                let rhs_elem = self.fresh_value();
                self.emit(Instruction::Load {
                    volatile: false,
                    dest: rhs_elem,
                    ptr: rhs_elem_ptr,
                    ty: elem_ir_ty,
                    seg_override: AddressSpace::Default,
                });
                Operand::Value(rhs_elem)
            };

            let result_elem_ptr = if offset > 0 {
                self.emit_binop_val(
                    IrBinOp::Add,
                    Operand::Value(result_alloca),
                    Operand::Const(IrConst::ptr_int(offset as i64)),
                    ptr_int_ty,
                )
            } else {
                result_alloca
            };

            // Lane predicate -> all-ones/all-zeros mask.  `Cmp` yields an I32
            // 0/1 boolean; widen to the element type, then `0 - x` produces
            // the all-ones mask (two's complement) for the true lane and 0 for
            // the false lane, at any element width.
            let cond = self.emit_cmp_val(cmp_op, lhs_elem_op, rhs_elem_op, elem_ir_ty);
            let cond_ext = self.emit_cast_val(Operand::Value(cond), IrType::I32, elem_ir_ty);
            let zero = Operand::Const(Self::ir_zero_const(elem_ir_ty));
            let mask =
                self.emit_binop_val(IrBinOp::Sub, zero, Operand::Value(cond_ext), elem_ir_ty);
            self.emit(Instruction::Store {
                volatile: false,
                val: Operand::Value(mask),
                ptr: result_elem_ptr,
                ty: elem_ir_ty,
                seg_override: AddressSpace::Default,
            });
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
        let elem_ir_ty = Self::vector_elem_ir_type(&elem_ct);
        let elem_size = elem_ct.size();
        let total_size = match vec_ct {
            CType::Vector(_, ts) => *ts,
            _ => unreachable!(
                "lower_vector_unary called with non-vector type: {:?}",
                vec_ct
            ),
        };
        let result_alloca = self.emit_entry_alloca(IrType::Ptr, total_size, total_size, false);
        let val = self.lower_expr(inner);
        let val = self.operand_to_value(val);
        let ptr_int_ty = crate::common::types::target_int_ir_type();

        for i in 0..num_elems {
            let offset = i * elem_size;
            let elem_ptr = if offset > 0 {
                self.emit_binop_val(
                    IrBinOp::Add,
                    Operand::Value(val),
                    Operand::Const(IrConst::ptr_int(offset as i64)),
                    ptr_int_ty,
                )
            } else {
                val
            };
            let elem = self.fresh_value();
            self.emit(Instruction::Load {
                volatile: false,
                dest: elem,
                ptr: elem_ptr,
                ty: elem_ir_ty,
                seg_override: AddressSpace::Default,
            });
            let result_elem_ptr = if offset > 0 {
                self.emit_binop_val(
                    IrBinOp::Add,
                    Operand::Value(result_alloca),
                    Operand::Const(IrConst::ptr_int(offset as i64)),
                    ptr_int_ty,
                )
            } else {
                result_alloca
            };
            let result_elem = self.fresh_value();
            self.emit(Instruction::UnaryOp {
                dest: result_elem,
                op: ir_unary,
                src: Operand::Value(elem),
                ty: elem_ir_ty,
            });
            self.emit(Instruction::Store {
                volatile: false,
                val: Operand::Value(result_elem),
                ptr: result_elem_ptr,
                ty: elem_ir_ty,
                seg_override: AddressSpace::Default,
            });
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
