//! Switch/case/default statement lowering.
//!
//! Handles the full switch dispatch pipeline: evaluating the controlling expression,
//! setting up the switch context, collecting case/default labels during body lowering,
//! and emitting the dispatch chain (Switch terminator for exact cases, if-else chain
//! for GNU case ranges).

use super::definitions::SwitchFrame;
use super::lower::Lowerer;
use crate::common::source::Span;
use crate::common::types::{AddressSpace, IrType};
use crate::frontend::parser::ast::{Expr, Stmt};
use crate::ir::reexports::{BlockId, Instruction, IrCmpOp, IrConst, Operand, Terminator, Value};

impl Lowerer {
    pub(super) fn lower_switch_stmt(&mut self, expr: &Expr, body: &Stmt) {
        // C99 6.8.4.2: Integer promotions are performed on the controlling expression.
        // Non-integer types (pointers, floats, etc.) are rejected by sema; this Ptr
        // arm is a defensive fallback to avoid crashes if sema is bypassed.
        //
        // The controlling value is consumed at a CONVERSION BOUNDARY (the
        // `Switch` terminator compares exactly `ty` bits), so its type must
        // describe the bits the lowered expression really produces.
        // `get_expr_type` reports the widened *storage* type: on LP64 an
        // integer literal is I64, so `x + 0` / `x * 1` / `x + 1` with
        // `int x` was reported as I64 while `lower_arithmetic_binop` computed
        // the sum as a 32-bit `addl`.  The dispatch chain then compared the
        // full 64-bit register (`cmpq $-1, %rdi`), whose upper half is
        // ABI-undefined for an `int` argument (SysV: only the low 32 bits of
        // a 32-bit parameter are meaningful) or zero after a 32-bit ALU op —
        // so `case -1:` never matched and `case 5:` matched garbage.  Found
        // by tests/stress (switch family, every seed, every -O level).
        // `value_ir_type` mirrors lower_arithmetic_binop's exact type
        // computation and is the documented query for such boundaries.
        let raw_expr_ty = self.value_ir_type(expr);
        let switch_expr_ty = match raw_expr_ty {
            IrType::I8 | IrType::U8 | IrType::I16 | IrType::U16 => IrType::I32,
            IrType::Ptr => crate::common::types::target_int_ir_type(),
            _ => raw_expr_ty,
        };
        let val = self.lower_expr(expr);
        let val = if switch_expr_ty != raw_expr_ty {
            self.emit_implicit_cast(val, raw_expr_ty, switch_expr_ty)
        } else {
            val
        };

        // Store switch value in an alloca for dispatch chain reloading
        let switch_alloca = self.fresh_value();
        let switch_size = switch_expr_ty.size();
        self.emit(Instruction::Alloca {
            dest: switch_alloca,
            ty: switch_expr_ty,
            size: switch_size,
            align: 0,
            volatile: false,
            semantic_volatile: false,
        });
        self.emit(Instruction::Store {
            volatile: false,
            val,
            ptr: switch_alloca,
            ty: switch_expr_ty,
            seg_override: AddressSpace::Default,
        });

        let dispatch_label = self.fresh_label();
        let end_label = self.fresh_label();
        let body_label = self.fresh_label();

        // Push switch context
        self.func_mut().switch_stack.push(SwitchFrame {
            cases: Vec::new(),
            case_ranges: Vec::new(),
            covered: Default::default(),
            default_label: None,
            expr_type: switch_expr_ty,
        });
        let scope_depth = self.func().scope_stack.len();
        let names = std::mem::take(&mut self.func_mut().pending_loop_labels);
        self.func_mut()
            .break_labels
            .push((end_label, scope_depth, names));

        self.terminate(Terminator::Branch(dispatch_label));

        // Lower the body first; case/default stmts register their labels
        self.start_block(body_label);
        self.lower_substatement(body);
        self.terminate(Terminator::Branch(end_label));

        // Pop switch context and emit dispatch chain
        let switch_frame = self.func_mut().switch_stack.pop();
        self.func_mut().break_labels.pop();
        let cases = switch_frame
            .as_ref()
            .map(|f| f.cases.clone())
            .unwrap_or_default();
        let case_ranges = switch_frame
            .as_ref()
            .map(|f| f.case_ranges.clone())
            .unwrap_or_default();
        let default_label = switch_frame.as_ref().and_then(|f| f.default_label);

        let expr_type = switch_frame
            .as_ref()
            .map(|f| f.expr_type)
            .unwrap_or(IrType::I32);
        let fallback = default_label.unwrap_or(end_label);
        self.start_block(dispatch_label);
        self.emit_switch_dispatch(
            &val,
            switch_alloca,
            &cases,
            &case_ranges,
            fallback,
            expr_type,
        );
        self.start_block(end_label);
    }

    /// Emit the dispatch chain for a switch statement. For compile-time constant
    /// switch values, directly jumps to the matching case. Otherwise emits a
    /// chain of comparison branches.
    fn emit_switch_dispatch(
        &mut self,
        val: &Operand,
        switch_alloca: Value,
        cases: &[(i128, BlockId)],
        case_ranges: &[(i128, i128, BlockId)],
        fallback: BlockId,
        switch_ty: IrType,
    ) {
        let total_checks = cases.len() + case_ranges.len();
        let wide = matches!(switch_ty, IrType::I128 | IrType::U128);
        let is_unsigned = switch_ty.is_unsigned();

        // Constant folding: if switch expression is a compile-time constant,
        // jump directly to the matching case (avoids dead dispatch branches
        // with type-mismatched inline asm).
        if let Operand::Const(c) = val {
            let switch_int = if wide {
                c.to_i128()
            } else {
                self.const_to_i64(c).map(i128::from)
            };
            if let Some(switch_int) = switch_int {
                // Unsigned order at the switch type's width: a narrow value
                // is the sign extension of its i64 bits, so compare those.
                let unsigned_key = |v: i128| {
                    if wide {
                        v as u128
                    } else {
                        v as i64 as u64 as u128
                    }
                };
                let target = cases
                    .iter()
                    .find(|(cv, _)| *cv == switch_int)
                    .map(|(_, label)| *label)
                    .or_else(|| {
                        case_ranges
                            .iter()
                            .find(|(low, high, _)| {
                                if is_unsigned {
                                    let s = unsigned_key(switch_int);
                                    s >= unsigned_key(*low) && s <= unsigned_key(*high)
                                } else {
                                    switch_int >= *low && switch_int <= *high
                                }
                            })
                            .map(|(_, _, label)| *label)
                    })
                    .unwrap_or(fallback);
                self.terminate(Terminator::Branch(target));
                return;
            }
        }

        if total_checks == 0 {
            self.terminate(Terminator::Branch(fallback));
            return;
        }

        // The `Switch` terminator carries i64 case values, which a backend
        // matches against the low half of a 128-bit selector after checking
        // that the high half is the case's sign extension.  A 128-bit case
        // outside that set (2^64 - 1, 1 << 64, ...) is compared at full width
        // instead, one equality test per case: such switches are rare, and
        // GCC lowers them to compare chains as well.
        if wide && cases.iter().any(|&(v, _)| i128::from(v as i64) != v) {
            self.emit_wide_case_chain(switch_alloca, cases, case_ranges, fallback, switch_ty);
            return;
        }
        let cases: Vec<(i64, BlockId)> =
            cases.iter().map(|&(v, label)| (v as i64, label)).collect();
        let cases = cases.as_slice();

        // Use Switch terminator for non-range cases (enables jump table in backend).
        // Range cases still use the if-else chain since they can't be in a jump table.
        if !cases.is_empty() && case_ranges.is_empty() {
            // Load the switch value once and use Switch terminator
            let loaded = self.fresh_value();
            self.emit(Instruction::Load {
                volatile: false,
                dest: loaded,
                ptr: switch_alloca,
                ty: switch_ty,
                seg_override: AddressSpace::Default,
            });
            self.terminate(Terminator::Switch {
                val: Operand::Value(loaded),
                cases: cases.to_vec(),
                default: fallback,
                ty: switch_ty,
            });
            return;
        }

        // Mixed case: handle non-range cases with Switch if any, range cases with if-else.
        if !cases.is_empty() {
            // Switch terminator for the simple cases, with fallthrough to range checks
            let range_check_block = if !case_ranges.is_empty() {
                self.fresh_label()
            } else {
                fallback
            };
            let loaded = self.fresh_value();
            self.emit(Instruction::Load {
                volatile: false,
                dest: loaded,
                ptr: switch_alloca,
                ty: switch_ty,
                seg_override: AddressSpace::Default,
            });
            self.terminate(Terminator::Switch {
                val: Operand::Value(loaded),
                cases: cases.to_vec(),
                default: range_check_block,
                ty: switch_ty,
            });
            if !case_ranges.is_empty() {
                self.start_block(range_check_block);
            }
        }

        self.emit_case_range_checks(switch_alloca, case_ranges, fallback, switch_ty);
    }

    /// Full-width equality tests for a 128-bit switch whose case values the
    /// `Switch` terminator cannot carry, followed by the GNU range checks.
    fn emit_wide_case_chain(
        &mut self,
        switch_alloca: Value,
        cases: &[(i128, BlockId)],
        case_ranges: &[(i128, i128, BlockId)],
        fallback: BlockId,
        switch_ty: IrType,
    ) {
        let after_cases = if case_ranges.is_empty() {
            fallback
        } else {
            self.fresh_label()
        };
        for (ci, &(case_val, case_label)) in cases.iter().enumerate() {
            let loaded = self.fresh_value();
            self.emit(Instruction::Load {
                volatile: false,
                dest: loaded,
                ptr: switch_alloca,
                ty: switch_ty,
                seg_override: AddressSpace::Default,
            });
            let is_eq = self.emit_cmp_val(
                IrCmpOp::Eq,
                Operand::Value(loaded),
                Operand::Const(IrConst::I128(case_val)),
                switch_ty,
            );
            let next_check = if ci + 1 < cases.len() {
                self.fresh_label()
            } else {
                after_cases
            };
            self.terminate(Terminator::CondBranch {
                cond: Operand::Value(is_eq),
                true_label: case_label,
                false_label: next_check,
            });
            if next_check != fallback {
                self.start_block(next_check);
            }
        }
        self.emit_case_range_checks(switch_alloca, case_ranges, fallback, switch_ty);
    }

    /// GNU case ranges as `low <= val && val <= high` tests, in the switch
    /// type's signedness, each falling through to the next and the last to
    /// `fallback`.  The current block must be open unless there are none.
    fn emit_case_range_checks(
        &mut self,
        switch_alloca: Value,
        case_ranges: &[(i128, i128, BlockId)],
        fallback: BlockId,
        switch_ty: IrType,
    ) {
        // Emit range checks: val >= low && val <= high
        // Use unsigned comparisons when the switch expression type is unsigned
        let is_unsigned = switch_ty.is_unsigned();
        let ge_op = if is_unsigned {
            IrCmpOp::Uge
        } else {
            IrCmpOp::Sge
        };
        let le_op = if is_unsigned {
            IrCmpOp::Ule
        } else {
            IrCmpOp::Sle
        };
        let range_count = case_ranges.len();
        let make_case_const = |v: i128| -> IrConst {
            if matches!(switch_ty, IrType::I128 | IrType::U128) {
                IrConst::I128(v)
            } else {
                IrConst::from_i64(v as i64, switch_ty)
            }
        };
        for (ri, (low, high, range_label)) in case_ranges.iter().enumerate() {
            let loaded = self.fresh_value();
            self.emit(Instruction::Load {
                volatile: false,
                dest: loaded,
                ptr: switch_alloca,
                ty: switch_ty,
                seg_override: AddressSpace::Default,
            });
            let ge_result = self.emit_cmp_val(
                ge_op,
                Operand::Value(loaded),
                Operand::Const(make_case_const(*low)),
                switch_ty,
            );
            let le_result = self.emit_cmp_val(
                le_op,
                Operand::Value(loaded),
                Operand::Const(make_case_const(*high)),
                switch_ty,
            );
            let and_result = self.fresh_value();
            self.emit(Instruction::BinOp {
                dest: and_result,
                op: crate::ir::reexports::IrBinOp::And,
                lhs: Operand::Value(ge_result),
                rhs: Operand::Value(le_result),
                ty: IrType::I32,
            });

            let next_check = if ri + 1 < range_count {
                self.fresh_label()
            } else {
                fallback
            };
            self.terminate(Terminator::CondBranch {
                cond: Operand::Value(and_result),
                true_label: *range_label,
                false_label: next_check,
            });
            if ri + 1 < range_count {
                self.start_block(next_check);
            }
        }
    }

    /// Evaluate a case constant and convert it to the switch expression type.
    pub(super) fn eval_case_constant(&mut self, expr: &Expr) -> i128 {
        let switch_ty = self.func_mut().switch_stack.last().map(|f| f.expr_type);
        if matches!(switch_ty, Some(IrType::I128 | IrType::U128)) {
            // C11 6.8.4.2p5: the case constant is converted to the promoted
            // type of the controlling expression -- by value, not by carrier
            // bits.  `IrConst` carries no signedness (an `unsigned long`
            // 2^64 - 1 is `I64(-1)`), so the expression's own type decides
            // how its bits extend; re-extending from that width is the
            // identity on a correctly encoded constant either way.
            let wide = match self.eval_const_expr(expr) {
                Some(IrConst::I128(v)) => Some(v),
                Some(c) => c.to_i64().map(|v| match self.value_ir_type(expr) {
                    IrType::U8 => i128::from(v as u8),
                    IrType::U16 => i128::from(v as u16),
                    IrType::U32 => i128::from(v as u32),
                    IrType::U64 => i128::from(v as u64),
                    IrType::I8 => i128::from(v as i8),
                    IrType::I16 => i128::from(v as i16),
                    IrType::I32 => i128::from(v as i32),
                    _ => i128::from(v),
                }),
                None => None,
            };
            return wide.unwrap_or_else(|| {
                self.diagnostics.borrow_mut().error(
                    "case label does not reduce to an integer constant",
                    expr.span(),
                );
                0
            });
        }
        i128::from(self.eval_case_constant_narrow(expr))
    }

    /// A case constant for a switch type of at most 64 bits, truncated to it.
    fn eval_case_constant_narrow(&mut self, expr: &Expr) -> i64 {
        // A case label must be an integer constant expression (C11 6.8.4.2p3).
        // Folding a non-constant to 0 silently turns `case f(x):` into
        // `case 0:` — a different program, and one that can collide with a real
        // `case 0:` — which is exactly what GCC, Clang and ICX refuse to do
        // ("case label does not reduce to an integer constant"); measured
        // against all three.  Diagnose, then keep 0 so the enclosing switch
        // still lowers into well-formed IR.
        let constant = self
            .eval_const_expr(expr)
            .and_then(|c| self.const_to_i64(&c));
        let mut val = match constant {
            Some(val) => val,
            None => {
                self.diagnostics.borrow_mut().error(
                    "case label does not reduce to an integer constant",
                    expr.span(),
                );
                0
            }
        };
        if let Some(switch_ty) = self.func_mut().switch_stack.last().map(|f| &f.expr_type) {
            val = switch_ty.truncate_i64(val);
        }
        val
    }

    pub(super) fn lower_case_stmt(&mut self, expr: &Expr, stmt: &Stmt, span: Span) {
        let case_val = self.eval_case_constant(expr);
        let label = self.fresh_label();
        // C11 6.8.4.2p3: no two case constants of one switch may have the
        // same value after conversion.  Accepting one silently picked
        // whichever label the dispatch tested first; GCC, Clang and ICX all
        // refuse the program.
        let duplicate = self
            .func()
            .switch_stack
            .last()
            .is_some_and(|frame| frame.overlaps(case_val, case_val));
        if duplicate {
            self.emit_error("duplicate case value", span);
        }
        if let Some(frame) = self.func_mut().switch_stack.last_mut() {
            frame.add_case(case_val, label);
        }
        // Fallthrough from previous case
        self.terminate(Terminator::Branch(label));
        self.start_block(label);
        self.lower_stmt(stmt);
    }

    pub(super) fn lower_case_range_stmt(
        &mut self,
        low_expr: &Expr,
        high_expr: &Expr,
        stmt: &Stmt,
        span: Span,
    ) {
        let low_val = self.eval_case_constant(low_expr);
        let high_val = self.eval_case_constant(high_expr);
        let label = self.fresh_label();
        let (empty, overlapping) =
            self.func()
                .switch_stack
                .last()
                .map_or((false, false), |frame| {
                    (
                        frame.is_empty_range(low_val, high_val),
                        frame.overlaps(low_val, high_val),
                    )
                });
        if empty {
            // Matches nothing; GCC and Clang say so and drop the label.
            self.emit_warning("empty range specified", span);
        }
        if overlapping {
            self.emit_error("duplicate (or overlapping) case value", span);
        }
        if let Some(frame) = self.func_mut().switch_stack.last_mut() {
            frame.add_range(low_val, high_val, label);
        }
        self.terminate(Terminator::Branch(label));
        self.start_block(label);
        self.lower_stmt(stmt);
    }

    pub(super) fn lower_default_stmt(&mut self, stmt: &Stmt, span: Span) {
        let label = self.fresh_label();
        // C11 6.8.4.2p3: at most one default label per switch.  The second
        // one used to replace the first without a word.
        let repeated = self
            .func()
            .switch_stack
            .last()
            .is_some_and(|frame| frame.default_label.is_some());
        if repeated {
            self.emit_error("multiple default labels in one switch", span);
        }
        if let Some(frame) = self.func_mut().switch_stack.last_mut() {
            frame.default_label = Some(label);
        }
        self.terminate(Terminator::Branch(label));
        self.start_block(label);
        self.lower_stmt(stmt);
    }
}
