//! ArmCodegen: atomic operations.

use super::emit::ArmCodegen;
use crate::common::types::IrType;
use crate::ir::reexports::{AtomicOrdering, AtomicRmwOp, Operand, Value};

impl ArmCodegen {
    /// Canonicalize an exclusive-operation result in X0 to the IR type.
    fn canonicalize_atomic_x0_result(&mut self, ty: IrType) {
        match ty {
            IrType::I8 => self.state.emit("    sxtb x0, w0"),
            IrType::I16 => self.state.emit("    sxth x0, w0"),
            IrType::I32 => self.state.emit("    sxtw x0, w0"),
            // Byte/halfword loads and every W-register write already clear
            // the upper bits for unsigned types.
            _ => {}
        }
    }

    pub(super) fn emit_atomic_rmw_impl(
        &mut self,
        dest: &Value,
        op: AtomicRmwOp,
        ptr: &Operand,
        val: &Operand,
        ty: IrType,
        ordering: AtomicOrdering,
    ) {
        self.operand_to_x0(ptr);
        self.state.emit("    mov x1, x0");
        if !matches!(op, AtomicRmwOp::TestAndSet) {
            self.operand_to_x0(val);
            self.state.emit("    mov x2, x0");
        }

        let (ldxr, stxr, reg_prefix) = Self::exclusive_instrs(ty, ordering);
        let val_reg = format!("{}2", reg_prefix);
        let old_reg = format!("{}0", reg_prefix);
        let tmp_reg = format!("{}3", reg_prefix);

        match op {
            AtomicRmwOp::Xchg => {
                let label_id = self.state.next_label_id();
                let loop_label = format!(".Latomic_{}", label_id);
                self.state.emit_fmt(format_args!("{}:", loop_label));
                self.state
                    .emit_fmt(format_args!("    {} {}, [x1]", ldxr, old_reg));
                self.state
                    .emit_fmt(format_args!("    {} w4, {}, [x1]", stxr, val_reg));
                self.state
                    .emit_fmt(format_args!("    cbnz w4, {}", loop_label));
            }
            AtomicRmwOp::TestAndSet => {
                let label_id = self.state.next_label_id();
                let loop_label = format!(".Latomic_{}", label_id);
                self.state.emit("    mov w3, #1");
                self.state.emit_fmt(format_args!("{}:", loop_label));
                self.state
                    .emit_fmt(format_args!("    {} {}, [x1]", ldxr, old_reg));
                self.state
                    .emit_fmt(format_args!("    {} w4, w3, [x1]", stxr));
                self.state
                    .emit_fmt(format_args!("    cbnz w4, {}", loop_label));
            }
            _ => {
                let label_id = self.state.next_label_id();
                let loop_label = format!(".Latomic_{}", label_id);
                self.state.emit_fmt(format_args!("{}:", loop_label));
                self.state
                    .emit_fmt(format_args!("    {} {}, [x1]", ldxr, old_reg));
                Self::emit_atomic_op_arm(&mut self.state, op, &tmp_reg, &old_reg, &val_reg);
                self.state
                    .emit_fmt(format_args!("    {} w4, {}, [x1]", stxr, tmp_reg));
                self.state
                    .emit_fmt(format_args!("    cbnz w4, {}", loop_label));
            }
        }
        if !matches!(op, AtomicRmwOp::TestAndSet) {
            self.canonicalize_atomic_x0_result(ty);
        }
        self.store_x0_to(dest);
    }

    pub(super) fn emit_atomic_cmpxchg_impl(
        &mut self,
        dest: &Value,
        ptr: &Operand,
        expected: &Operand,
        desired: &Operand,
        ty: IrType,
        success_ordering: AtomicOrdering,
        _failure_ordering: AtomicOrdering,
        returns_bool: bool,
    ) {
        self.operand_to_x0(ptr);
        self.state.emit("    mov x1, x0");
        self.operand_to_x0(desired);
        self.state.emit("    mov x3, x0");
        self.operand_to_x0(expected);
        self.state.emit("    mov x2, x0");
        // LDXRB/LDXRH zero-extend, so compare the expected value at the
        // atomic object's width rather than against a sign-extended source.
        match ty {
            IrType::I8 | IrType::U8 => self.state.emit("    and w2, w2, #0xff"),
            IrType::I16 | IrType::U16 => self.state.emit("    uxth w2, w2"),
            _ => {}
        }

        let (ldxr, stxr, reg_prefix) = Self::exclusive_instrs(ty, success_ordering);
        let old_reg = format!("{}0", reg_prefix);
        let desired_reg = format!("{}3", reg_prefix);
        let expected_reg = format!("{}2", reg_prefix);

        let label_id = self.state.next_label_id();
        let loop_label = format!(".Lcas_loop_{}", label_id);
        let fail_label = format!(".Lcas_fail_{}", label_id);
        let done_label = format!(".Lcas_done_{}", label_id);

        self.state.emit_fmt(format_args!("{}:", loop_label));
        self.state
            .emit_fmt(format_args!("    {} {}, [x1]", ldxr, old_reg));
        self.state
            .emit_fmt(format_args!("    cmp {}, {}", old_reg, expected_reg));
        self.state.emit_fmt(format_args!("    b.ne {}", fail_label));
        self.state
            .emit_fmt(format_args!("    {} w4, {}, [x1]", stxr, desired_reg));
        self.state
            .emit_fmt(format_args!("    cbnz w4, {}", loop_label));
        if returns_bool {
            self.state.emit("    mov x0, #1");
        }
        self.state.emit_fmt(format_args!("    b {}", done_label));
        self.state.emit_fmt(format_args!("{}:", fail_label));
        if returns_bool {
            self.state.emit("    mov x0, #0");
            self.state.emit("    clrex");
        } else {
            self.state.emit("    clrex");
        }
        self.state.emit_fmt(format_args!("{}:", done_label));
        if !returns_bool {
            self.canonicalize_atomic_x0_result(ty);
        }
        self.store_x0_to(dest);
    }

    pub(super) fn emit_atomic_load_impl(
        &mut self,
        dest: &Value,
        ptr: &Operand,
        ty: IrType,
        ordering: AtomicOrdering,
    ) {
        self.operand_to_x0(ptr);
        let need_acquire = matches!(
            ordering,
            AtomicOrdering::Acquire | AtomicOrdering::AcqRel | AtomicOrdering::SeqCst
        );
        let instr = match (ty, need_acquire) {
            (IrType::I8 | IrType::U8, true) => "ldarb",
            (IrType::I8 | IrType::U8, false) => "ldrb",
            (IrType::I16 | IrType::U16, true) => "ldarh",
            (IrType::I16 | IrType::U16, false) => "ldrh",
            (IrType::I32 | IrType::U32, true) => "ldar",
            (IrType::I32 | IrType::U32, false) => "ldr",
            (_, true) => "ldar",
            (_, false) => "ldr",
        };
        let dest_reg = match ty {
            IrType::I8 | IrType::U8 | IrType::I16 | IrType::U16 | IrType::I32 | IrType::U32 => "w0",
            _ => "x0",
        };
        self.state
            .emit_fmt(format_args!("    {} {}, [x0]", instr, dest_reg));
        self.canonicalize_atomic_x0_result(ty);
        self.store_x0_to(dest);
    }

    pub(super) fn emit_atomic_store_impl(
        &mut self,
        ptr: &Operand,
        val: &Operand,
        ty: IrType,
        ordering: AtomicOrdering,
    ) {
        self.operand_to_x0(val);
        self.state.emit("    mov x1, x0");
        self.operand_to_x0(ptr);
        let need_release = matches!(
            ordering,
            AtomicOrdering::Release | AtomicOrdering::AcqRel | AtomicOrdering::SeqCst
        );
        let instr = match (ty, need_release) {
            (IrType::I8 | IrType::U8, true) => "stlrb",
            (IrType::I8 | IrType::U8, false) => "strb",
            (IrType::I16 | IrType::U16, true) => "stlrh",
            (IrType::I16 | IrType::U16, false) => "strh",
            (IrType::I32 | IrType::U32, true) => "stlr",
            (IrType::I32 | IrType::U32, false) => "str",
            (_, true) => "stlr",
            (_, false) => "str",
        };
        let val_reg = match ty {
            IrType::I8 | IrType::U8 | IrType::I16 | IrType::U16 | IrType::I32 | IrType::U32 => "w1",
            _ => "x1",
        };
        self.state
            .emit_fmt(format_args!("    {} {}, [x0]", instr, val_reg));
    }

    pub(super) fn emit_fence_impl(&mut self, ordering: AtomicOrdering) {
        match ordering {
            AtomicOrdering::Relaxed => {}
            AtomicOrdering::Acquire => self.state.emit("    dmb ishld"),
            // A C release fence orders earlier loads as well as stores before
            // a following publishing store; DMB ISHST would omit load->store.
            AtomicOrdering::Release | AtomicOrdering::AcqRel | AtomicOrdering::SeqCst => {
                self.state.emit("    dmb ish")
            }
        }
    }
}
