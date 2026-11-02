//! I686Codegen: atomic operations (RMW, cmpxchg, load, store, fence).
//!
//! Ordinary 8-, 16-, and 32-bit atomic objects must satisfy the alignment
//! contract enforced by the frontend/IR. 64-bit operations are delegated to
//! the CMPXCHG8B helpers in `emit.rs`.
//!
//! The operand materializer only defines `%eax`; `%edx` may therefore carry a
//! value across `operand_to_eax`. Temporary changes to `%esp`, however, must be
//! reflected in `esp_adjust` before materializing an ESP-relative operand.

use super::emit::I686Codegen;
use crate::common::types::IrType;
use crate::emit;
use crate::ir::reexports::{AtomicOrdering, AtomicRmwOp, Operand, Value};

/// Register view of EDX matching a narrow atomic memory operand.
fn atomic_edx_register(ty: IrType) -> &'static str {
    match ty {
        IrType::I8 | IrType::U8 => "%dl",
        IrType::I16 | IrType::U16 => "%dx",
        _ => "%edx",
    }
}

impl I686Codegen {
    /// Canonicalize a narrow result in AL/AX to the IR type held in EAX.
    ///
    /// Narrow CMPXCHG/XADD/XCHG only overwrite the low byte or word of their
    /// register operand. The stale upper bits must never escape as part of the
    /// returned SSA value.
    fn extend_atomic_eax_result(&mut self, ty: IrType) {
        match ty {
            IrType::I8 => self.state.emit("    movsbl %al, %eax"),
            IrType::U8 => self.state.emit("    movzbl %al, %eax"),
            IrType::I16 => self.state.emit("    movswl %ax, %eax"),
            IrType::U16 => self.state.emit("    movzwl %ax, %eax"),
            _ => {}
        }
    }

    /// Move the old value returned in EDX by XADD/XCHG to EAX, canonicalizing
    /// narrow signed and unsigned values at the same time.
    fn move_atomic_edx_result_to_eax(&mut self, ty: IrType) {
        match ty {
            IrType::I8 => self.state.emit("    movsbl %dl, %eax"),
            IrType::U8 => self.state.emit("    movzbl %dl, %eax"),
            IrType::I16 => self.state.emit("    movswl %dx, %eax"),
            IrType::U16 => self.state.emit("    movzwl %dx, %eax"),
            _ => self.state.emit("    movl %edx, %eax"),
        }
    }

    /// Emit a generic-i686 full hardware fence without requiring SSE2 or a
    /// scratch register. The aligned word at ESP is already writable stack
    /// storage; OR with zero leaves its value unchanged.
    ///
    /// This deliberately clobbers EFLAGS. Flags are not values in the LCCC IR,
    /// and the i686 backend materializes every comparison before moving to the
    /// next IR instruction, so no flag value is live across a Fence. Keeping
    /// flags would add PUSHF/POPF or a slower XCHG sequence to every fence for
    /// no semantic benefit.
    fn emit_atomic_full_fence(&mut self) {
        self.state.emit("    lock orl $0, (%esp)");
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
        if self.is_atomic_wide(ty) {
            self.emit_atomic_rmw_wide(dest, op, ptr, val);
            return;
        }

        // GCC's test-and-set builtin is byte-sized, stores one, and returns
        // the previous byte. The IR deliberately carries a dummy value of one;
        // do not materialize it.
        if matches!(op, AtomicRmwOp::TestAndSet) {
            self.operand_to_eax(ptr);
            self.state.emit("    movl %eax, %ecx");
            self.state.emit("    movb $1, %al");
            self.state.emit("    xchgb %al, (%ecx)");
            self.state.emit("    movzbl %al, %eax");
            self.state.reg_cache.invalidate_acc();
            self.store_eax_to(dest);
            return;
        }

        // EDX is preserved by operand_to_eax, avoiding stack traffic on the
        // single-instruction XCHG/XADD paths.
        self.operand_to_eax(val);
        self.state.emit("    movl %eax, %edx");
        self.operand_to_eax(ptr);

        let suffix = self.type_suffix(ty);
        let reg = atomic_edx_register(ty);

        match op {
            AtomicRmwOp::Xchg => {
                // A memory XCHG is implicitly locked.
                emit!(self.state, "    xchg{} {}, (%eax)", suffix, reg);
                self.move_atomic_edx_result_to_eax(ty);
            }
            AtomicRmwOp::Add | AtomicRmwOp::Sub => {
                if matches!(op, AtomicRmwOp::Sub) {
                    // low_w(-x) == -low_w(x) modulo 2^w, so one 32-bit NEG
                    // permits SUB to use the same single XADD as ADD.
                    self.state.emit("    negl %edx");
                }
                emit!(self.state, "    lock xadd{} {}, (%eax)", suffix, reg);
                self.move_atomic_edx_result_to_eax(ty);
            }
            AtomicRmwOp::And | AtomicRmwOp::Or | AtomicRmwOp::Xor | AtomicRmwOp::Nand => {
                // Preserve the operand over retries. Track the slot even though
                // no operand helper is called while it exists: doing so keeps
                // the backend's ESP accounting invariant explicit.
                self.state.emit("    pushl %edx");
                self.esp_adjust += 4;
                self.state.emit("    movl %eax, %ecx");

                let load = self.mov_load_for_type(ty);
                emit!(self.state, "    {} (%ecx), %eax", load);

                let loop_label = format!(".Latomic_{}", self.state.next_label_id());
                emit!(self.state, "{}:", loop_label);
                self.state.emit("    movl %eax, %edx");

                // Candidate arithmetic can be 32-bit: truncation commutes with
                // AND/OR/XOR/NOT. Only CMPXCHG touches the user's object, and it
                // retains the object's exact width.
                let instruction = match op {
                    AtomicRmwOp::And | AtomicRmwOp::Nand => "andl",
                    AtomicRmwOp::Or => "orl",
                    AtomicRmwOp::Xor => "xorl",
                    _ => unreachable!(),
                };
                emit!(self.state, "    {} (%esp), %edx", instruction);
                if matches!(op, AtomicRmwOp::Nand) {
                    self.state.emit("    notl %edx");
                }

                emit!(self.state, "    lock cmpxchg{} {}, (%ecx)", suffix, reg);
                emit!(self.state, "    jne {}", loop_label);

                self.state.emit("    addl $4, %esp");
                self.esp_adjust -= 4;
                self.extend_atomic_eax_result(ty);
            }
            AtomicRmwOp::TestAndSet => unreachable!(),
        }

        self.state.reg_cache.invalidate_acc();
        self.store_eax_to(dest);
    }

    pub(super) fn emit_atomic_cmpxchg_impl(
        &mut self,
        dest: &Value,
        ptr: &Operand,
        expected: &Operand,
        desired: &Operand,
        ty: IrType,
        _success: AtomicOrdering,
        _failure: AtomicOrdering,
        returns_bool: bool,
    ) {
        if self.is_atomic_wide(ty) {
            self.emit_atomic_cmpxchg_wide(dest, ptr, expected, desired, returns_bool);
            return;
        }

        // operand_to_eax preserves EDX, so expected can remain there while the
        // desired value and pointer are materialized. Desired needs one stack
        // slot because CMPXCHG requires expected in EAX and desired in EDX.
        self.operand_to_eax(expected);
        self.state.emit("    movl %eax, %edx");
        self.operand_to_eax(desired);
        self.state.emit("    pushl %eax");
        self.esp_adjust += 4;
        self.operand_to_eax(ptr);
        self.state.emit("    movl %eax, %ecx");
        self.state.emit("    movl %edx, %eax");
        self.state.emit("    popl %edx");
        self.esp_adjust -= 4;

        let suffix = self.type_suffix(ty);
        let reg = atomic_edx_register(ty);
        emit!(self.state, "    lock cmpxchg{} {}, (%ecx)", suffix, reg);

        if returns_bool {
            // Consume CMPXCHG's ZF before any flag-changing instruction.
            self.state.emit("    sete %al");
            self.state.emit("    movzbl %al, %eax");
        } else {
            // On failure only AL/AX is replaced for narrow operations; on
            // success expected remains in EAX. Canonicalize both cases.
            self.extend_atomic_eax_result(ty);
        }

        self.state.reg_cache.invalidate_acc();
        self.store_eax_to(dest);
    }

    pub(super) fn emit_atomic_load_impl(
        &mut self,
        dest: &Value,
        ptr: &Operand,
        ty: IrType,
        _ordering: AtomicOrdering,
    ) {
        if self.is_atomic_wide(ty) {
            self.emit_atomic_load_wide(dest, ptr);
            return;
        }

        // Plain aligned MOV is the standard x86-TSO mapping for atomic loads,
        // including SeqCst loads when SeqCst stores use locked XCHG below.
        self.operand_to_eax(ptr);
        let load = self.mov_load_for_type(ty);
        emit!(self.state, "    {} (%eax), %eax", load);
        self.state.reg_cache.invalidate_acc();
        self.store_eax_to(dest);
    }

    pub(super) fn emit_atomic_store_impl(
        &mut self,
        ptr: &Operand,
        val: &Operand,
        ty: IrType,
        ordering: AtomicOrdering,
    ) {
        if self.is_atomic_wide(ty) {
            // The helper's successful locked CMPXCHG8B already supplies full
            // ordering, including for SeqCst; a trailing fence is redundant.
            self.emit_atomic_store_wide(ptr, val);
            return;
        }

        self.operand_to_eax(val);
        self.state.emit("    movl %eax, %edx");
        self.operand_to_eax(ptr);

        let reg = atomic_edx_register(ty);
        if matches!(ordering, AtomicOrdering::SeqCst) {
            // One implicitly locked XCHG is both the store and the StoreLoad
            // barrier. This is shorter and faster than MOV followed by MFENCE.
            let suffix = self.type_suffix(ty);
            emit!(self.state, "    xchg{} {}, (%eax)", suffix, reg);
        } else {
            let store = self.mov_store_for_type(ty);
            emit!(self.state, "    {} {}, (%eax)", store, reg);
        }
    }

    pub(super) fn emit_fence_impl(&mut self, ordering: AtomicOrdering) {
        if matches!(ordering, AtomicOrdering::Relaxed) {
            return;
        }

        // Keep a hardware barrier for every non-relaxed fence. The i686
        // backend exposes non-temporal-store intrinsics, so weakening release
        // fences to a compiler barrier requires a broader ordering audit.
        self.emit_atomic_full_fence();
        // No register changes, so all register-cache entries remain valid.
    }
}
