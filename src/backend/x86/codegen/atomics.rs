//! X86Codegen: atomic operations (RMW, cmpxchg, load, store, fence).
//!
//! Locked instructions provide the required ordering for every RMW. Narrow
//! register operands only overwrite AL/AX/EAX, so results must be canonicalized
//! before they are stored as the backend's 64-bit SSA representation.

use super::emit::X86Codegen;
use crate::common::types::IrType;
use crate::ir::reexports::{AtomicOrdering, AtomicRmwOp, Operand, Value};

impl X86Codegen {
    /// Canonicalize an atomic result in RAX to the signedness and width of `ty`.
    fn canonicalize_atomic_rax_result(&mut self, ty: IrType) {
        match ty {
            IrType::I8 => self.state.emit("    movsbq %al, %rax"),
            IrType::U8 => self.state.emit("    movzbl %al, %eax"),
            IrType::I16 => self.state.emit("    movswq %ax, %rax"),
            IrType::U16 => self.state.emit("    movzwl %ax, %eax"),
            IrType::I32 => self.state.emit("    movslq %eax, %rax"),
            // A 32-bit destination write already zero-extends U32; 64-bit
            // operations overwrite all of RAX.
            _ => {}
        }
    }

    /// Prepare the x86 machine state for an atomic side effect.
    fn prepare_atomic_side_effect(&mut self) {
        debug_assert!(
            self.pending_cmp.is_none() && self.pending_fp_cmp.is_none(),
            "atomic side effect between fused comparison and consumer"
        );
        self.state.reg_cache.invalidate_all();
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
    }

    /// PGO counter increment as a SINGLE instruction: `[lock] incq sym+off(%rip)`.
    /// `incq` clobbers the flags, so the instrumentation pass never places a
    /// counter between a fused Cmp and its branch/select consumer.
    pub(super) fn emit_pgo_counter_inc_impl(&mut self, name: &str, offset: i64, atomic: bool) {
        debug_assert!(
            self.pending_cmp.is_none(),
            "counter between fused Cmp and consumer"
        );
        self.prepare_atomic_side_effect();
        let op = if atomic { "lock incq" } else { "incq" };
        if offset == 0 {
            self.state
                .emit_fmt(format_args!("    {} {}(%rip)", op, name));
        } else {
            self.state
                .emit_fmt(format_args!("    {} {}+{}(%rip)", op, name, offset));
        }
    }

    /// NOP-mode counter (debug): used to isolate instruction vs CFG bugs.
    #[expect(dead_code)]
    pub(super) fn emit_pgo_counter_nop_impl(&mut self, _name: &str, _offset: i64, _atomic: bool) {
        self.state.emit("    nop");
        self.state.reg_cache.invalidate_all();
    }

    /// Result-less atomic increment used by PGO. No virtual destination or
    /// spill slot is created, so the counter cannot perturb live value
    /// allocation in large C functions.
    pub(super) fn emit_atomic_inc_impl(
        &mut self,
        ptr: &Operand,
        offset: i64,
        ty: IrType,
        _ordering: AtomicOrdering,
    ) {
        self.operand_to_rax(ptr);
        self.prepare_atomic_side_effect();
        let suffix = Self::type_suffix(ty);
        self.state
            .emit_fmt(format_args!("    lock inc{} {}(%rax)", suffix, offset));
    }

    pub(super) fn emit_atomic_rmw_impl(
        &mut self,
        dest: &Value,
        op: AtomicRmwOp,
        ptr: &Operand,
        val: &Operand,
        ty: IrType,
        _ordering: AtomicOrdering,
    ) {
        self.operand_to_rax(ptr);
        self.state.emit("    movq %rax, %rcx");

        // TestAndSet carries a dummy IR operand. Avoid materializing a value
        // that is unconditionally replaced by one.
        if matches!(op, AtomicRmwOp::TestAndSet) {
            self.prepare_atomic_side_effect();
            self.state.emit("    movb $1, %al");
            self.state.emit("    xchgb %al, (%rcx)");
            // The builtin tests the old byte for zero, irrespective of whether
            // plain `char` is signed on the source target.
            self.state.emit("    movzbl %al, %eax");
            self.store_rax_to(dest);
            return;
        }

        self.operand_to_rax(val);
        self.prepare_atomic_side_effect();

        let suffix = Self::type_suffix(ty);
        let val_reg = Self::reg_for_type("rax", ty);
        match op {
            AtomicRmwOp::Add | AtomicRmwOp::Sub => {
                if matches!(op, AtomicRmwOp::Sub) {
                    // The low w bits of a 64-bit negation equal negation modulo
                    // 2^w, allowing subtraction to use one XADD at every width.
                    self.state.emit("    negq %rax");
                }
                self.state
                    .emit_fmt(format_args!("    lock xadd{} %{}, (%rcx)", suffix, val_reg));
                self.canonicalize_atomic_rax_result(ty);
            }
            AtomicRmwOp::Xchg => {
                self.state
                    .emit_fmt(format_args!("    xchg{} %{}, (%rcx)", suffix, val_reg));
                self.canonicalize_atomic_rax_result(ty);
            }
            AtomicRmwOp::And => self.emit_x86_atomic_op_loop(ty, "and"),
            AtomicRmwOp::Or => self.emit_x86_atomic_op_loop(ty, "or"),
            AtomicRmwOp::Xor => self.emit_x86_atomic_op_loop(ty, "xor"),
            AtomicRmwOp::Nand => self.emit_x86_atomic_op_loop(ty, "nand"),
            AtomicRmwOp::TestAndSet => unreachable!(),
        }
        if matches!(
            op,
            AtomicRmwOp::And | AtomicRmwOp::Or | AtomicRmwOp::Xor | AtomicRmwOp::Nand
        ) {
            self.canonicalize_atomic_rax_result(ty);
        }
        self.store_rax_to(dest);
    }

    pub(super) fn emit_atomic_cmpxchg_impl(
        &mut self,
        dest: &Value,
        ptr: &Operand,
        expected: &Operand,
        desired: &Operand,
        ty: IrType,
        _success_ordering: AtomicOrdering,
        _failure_ordering: AtomicOrdering,
        returns_bool: bool,
    ) {
        self.operand_to_rax(ptr);
        self.state.emit("    movq %rax, %rcx");
        self.operand_to_rax(desired);
        self.state.emit("    movq %rax, %rdx");
        self.operand_to_rax(expected);
        self.prepare_atomic_side_effect();

        let suffix = Self::type_suffix(ty);
        let desired_reg = Self::reg_for_type("rdx", ty);
        self.state.emit_fmt(format_args!(
            "    lock cmpxchg{} %{}, (%rcx)",
            suffix, desired_reg
        ));
        if returns_bool {
            // Consume CMPXCHG's ZF before any flag-changing instruction.
            self.state.emit("    sete %al");
            self.state.emit("    movzbl %al, %eax");
        } else {
            self.canonicalize_atomic_rax_result(ty);
        }
        self.store_rax_to(dest);
    }

    pub(super) fn emit_atomic_load_impl(
        &mut self,
        dest: &Value,
        ptr: &Operand,
        ty: IrType,
        _ordering: AtomicOrdering,
    ) {
        self.operand_to_rax(ptr);
        self.prepare_atomic_side_effect();
        let load = Self::mov_load_for_type(ty);
        let dest_reg = Self::load_dest_reg(ty);
        self.state
            .emit_fmt(format_args!("    {} (%rax), {}", load, dest_reg));
        self.store_rax_to(dest);
    }

    pub(super) fn emit_atomic_store_impl(
        &mut self,
        ptr: &Operand,
        val: &Operand,
        ty: IrType,
        ordering: AtomicOrdering,
    ) {
        self.operand_to_rax(val);
        self.state.emit("    movq %rax, %rdx");
        self.operand_to_rax(ptr);
        self.prepare_atomic_side_effect();

        let reg = Self::reg_for_type("rdx", ty);
        if matches!(ordering, AtomicOrdering::SeqCst) {
            // One implicitly locked XCHG performs the store and supplies the
            // StoreLoad barrier. MOV+MFENCE is two instructions and slower on
            // current Intel and AMD cores.
            let suffix = Self::type_suffix(ty);
            self.state
                .emit_fmt(format_args!("    xchg{} %{}, (%rax)", suffix, reg));
        } else {
            let store = Self::mov_store_for_type(ty);
            self.state
                .emit_fmt(format_args!("    {} %{}, (%rax)", store, reg));
        }
    }

    pub(super) fn emit_fence_impl(&mut self, ordering: AtomicOrdering) {
        if matches!(ordering, AtomicOrdering::Relaxed) {
            return;
        }

        // Flush deferred stores before the barrier. This also proves that no
        // fused comparison flags are live, so the faster locked stack no-op may
        // clobber EFLAGS safely. The backend exposes non-temporal stores, hence
        // the conservative full barrier for every non-relaxed fence.
        self.prepare_atomic_side_effect();
        self.state.emit("    lock orq $0, (%rsp)");
    }
}
