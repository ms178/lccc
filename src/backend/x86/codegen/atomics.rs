//! X86Codegen: atomic operations (RMW, cmpxchg, load, store, fence).

use super::emit::X86Codegen;
use crate::common::types::IrType;
use crate::ir::reexports::{AtomicOrdering, AtomicRmwOp, IrConst, Operand, Value};

impl X86Codegen {
    /// PGO counter increment as a SINGLE instruction: `[lock] incq sym+off(%rip)`.
    /// `incq` clobbers the flags, so the instrumentation pass never places a
    /// counter between a fused Cmp and its branch/select consumer.
    pub(super) fn emit_pgo_counter_inc_impl(&mut self, name: &str, offset: i64, atomic: bool) {
        debug_assert!(
            self.pending_cmp.is_none(),
            "counter between fused Cmp and consumer"
        );
        let op = if atomic { "lock incq" } else { "incq" };
        if offset == 0 {
            self.state
                .emit_fmt(format_args!("    {} {}(%rip)", op, name));
        } else {
            self.state
                .emit_fmt(format_args!("    {} {}+{}(%rip)", op, name, offset));
        }
        self.state.reg_cache.invalidate_all();
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
    }

    /// NOP-mode counter (debug): used to isolate instruction vs CFG bugs.
    #[allow(dead_code)]
    pub(super) fn emit_pgo_counter_nop_impl(&mut self, _name: &str, _offset: i64, _atomic: bool) {
        self.state.emit("    nop");
        self.state.reg_cache.invalidate_all();
    }

    /// Result-less atomic increment used by PGO.  No virtual destination or
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
        // Use the address register directly; borrowing RCX without a
        // register-allocation clobber declaration corrupts live values in
        // spill-heavy functions.
        let suffix = match ty {
            IrType::I64 | IrType::U64 => "q",
            IrType::I32 | IrType::U32 => "l",
            IrType::I16 | IrType::U16 => "w",
            _ => "b",
        };
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
        // Fast path: atomic add of the constant 1 (very common for refcounts /
        // PGO counters). `lock incq`/`lock addq $1` is a single micro-op that
        // does not return the old value, unlike `lock xadd` which is slower and
        // forces a value through rax. The destination is defined but its value
        // is irrelevant to such callers; we produce 0 (xor eax,eax) so the
        // register allocator still has a definition.
        if matches!(op, AtomicRmwOp::Add) {
            if let Operand::Const(c) = val {
                let one = match c {
                    IrConst::I8(1) | IrConst::I16(1) | IrConst::I32(1) | IrConst::I64(1) => true,
                    _ => false,
                };
                if one {
                    self.operand_to_rax(ptr);
                    self.state.emit("    movq %rax, %rcx");
                    self.state.reg_cache.invalidate_all();
                    self.flush_pending_vec_store_impl();
                    self.state.invalidate_vec_peephole();
                    // Single-byte atomic increment. PGO counters and most
                    // refcounts use I64; for other widths pick the right suffix.
                    let suffix = match ty {
                        IrType::I64 | IrType::U64 => "q",
                        IrType::I32 | IrType::U32 => "l",
                        IrType::I16 | IrType::U16 => "w",
                        _ => "b",
                    };
                    self.state
                        .emit_fmt(format_args!("    lock inc{} (%rcx)", suffix));
                    // Produce a defined (but meaningless) value for dest.
                    self.state.emit("    xorl %eax, %eax");
                    self.store_rax_to(dest);
                    return;
                }
            }
        }
        self.operand_to_rax(ptr);
        self.state.emit("    movq %rax, %rcx");
        self.operand_to_rax(val);
        self.state.reg_cache.invalidate_all();
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
        let size_suffix = Self::type_suffix(ty);
        let val_reg = Self::reg_for_type("rax", ty);
        match op {
            AtomicRmwOp::Add => {
                self.state.emit_fmt(format_args!(
                    "    lock xadd{} %{}, (%rcx)",
                    size_suffix, val_reg
                ));
            }
            AtomicRmwOp::Xchg => {
                self.state
                    .emit_fmt(format_args!("    xchg{} %{}, (%rcx)", size_suffix, val_reg));
            }
            AtomicRmwOp::TestAndSet => {
                self.state.emit("    movb $1, %al");
                self.state.emit("    xchgb %al, (%rcx)");
                // Zero-extend %al to %eax: xchgb only sets the low byte,
                // leaving upper bytes with garbage from prior register usage.
                self.state.emit("    movzbl %al, %eax");
            }
            AtomicRmwOp::Sub => {
                self.emit_x86_atomic_op_loop(ty, "sub");
            }
            AtomicRmwOp::And => {
                self.emit_x86_atomic_op_loop(ty, "and");
            }
            AtomicRmwOp::Or => {
                self.emit_x86_atomic_op_loop(ty, "or");
            }
            AtomicRmwOp::Xor => {
                self.emit_x86_atomic_op_loop(ty, "xor");
            }
            AtomicRmwOp::Nand => {
                self.emit_x86_atomic_op_loop(ty, "nand");
            }
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
        self.state.reg_cache.invalidate_all();
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
        let size_suffix = Self::type_suffix(ty);
        let desired_reg = Self::reg_for_type("rdx", ty);
        self.state.emit_fmt(format_args!(
            "    lock cmpxchg{} %{}, (%rcx)",
            size_suffix, desired_reg
        ));
        if returns_bool {
            self.state.emit("    sete %al");
            self.state.emit("    movzbl %al, %eax");
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
        self.state.reg_cache.invalidate_all();
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
        let load_instr = Self::mov_load_for_type(ty);
        let dest_reg = Self::load_dest_reg(ty);
        self.state
            .emit_fmt(format_args!("    {} (%rax), {}", load_instr, dest_reg));
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
        self.state.reg_cache.invalidate_all();
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
        let store_reg = Self::reg_for_type("rdx", ty);
        let store_instr = Self::mov_store_for_type(ty);
        self.state
            .emit_fmt(format_args!("    {} %{}, (%rax)", store_instr, store_reg));
        if matches!(ordering, AtomicOrdering::SeqCst) {
            self.state.emit("    mfence");
        }
    }

    pub(super) fn emit_fence_impl(&mut self, ordering: AtomicOrdering) {
        match ordering {
            AtomicOrdering::Relaxed => {}
            _ => self.state.emit("    mfence"),
        }
    }
}
