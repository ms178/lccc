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
        ordering: AtomicOrdering,
    ) {
        // 16-byte read-modify-write: there is no 16-byte `XADD`/`XCHG` and no
        // lock-prefixed 16-byte ALU form, so every op goes through libatomic's
        // sized entry — GCC 16.2's choice too, for every ordering.  The
        // previous value comes back in RDX:RAX.  Test-and-set is byte-wide by
        // definition and keeps the fast path below, which is also what keeps
        // `libatomic_fetch16_symbol`'s unreachable arm genuinely unreachable.
        if matches!(ty, IrType::I128 | IrType::U128) && !matches!(op, AtomicRmwOp::TestAndSet) {
            let sym = Self::libatomic_fetch16_symbol(op);
            self.emit_libatomic_16_binary(sym, ptr, val, ordering);
            self.store_rax_rdx_to(dest);
            return;
        }
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
        success_ordering: AtomicOrdering,
        failure_ordering: AtomicOrdering,
        returns_bool: bool,
    ) {
        // 16-byte compare-exchange: the 64-bit sequence below would compare and
        // write only the low half, so route it through its own lowering.  It is
        // also the only width whose lowering consumes the orderings: at 64 bits
        // and below the `lock` prefix alone supplies sequential consistency,
        // which is at least as strong as any ordering the caller can ask for.
        if matches!(ty, IrType::I128 | IrType::U128) {
            self.emit_atomic_cmpxchg16_impl(
                dest,
                ptr,
                expected,
                desired,
                success_ordering,
                failure_ordering,
                returns_bool,
            );
            return;
        }
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

    /// Run `f` with the rsp-relative slot addressing biased by `bias` bytes,
    /// for code that has pushed `bias` bytes of temporaries.
    fn with_rsp_bias(&mut self, bias: i64, f: impl FnOnce(&mut Self)) {
        let saved = self.state.out.rsp_frame_size;
        if self.state.out.use_rsp_addressing {
            self.state.out.rsp_frame_size += bias;
        }
        f(self);
        self.state.out.rsp_frame_size = saved;
    }

    /// Push a 16-byte operand: low half at the lower address, the stack
    /// grows by 16 bytes.  `bias` is what has already been pushed.
    fn push_16_operand(&mut self, op: &Operand, bias: i64) {
        self.with_rsp_bias(bias, |s| s.operand_to_rax_rdx(op));
        self.state.emit("    pushq %rdx");
        self.state.emit("    pushq %rax");
    }

    /// libatomic's memory-order encoding (`__ATOMIC_*`).  Shared by every
    /// sized entry point below, so the mapping cannot drift between them.
    fn libatomic_order(order: AtomicOrdering) -> u8 {
        match order {
            AtomicOrdering::Relaxed => 0,
            AtomicOrdering::Acquire => 2,
            AtomicOrdering::Release => 3,
            AtomicOrdering::AcqRel => 4,
            AtomicOrdering::SeqCst => 5,
        }
    }

    /// libatomic's sized 16-byte entry point for a read-modify-write.
    ///
    /// There is no 16-byte `XADD`/`XCHG` and no lock-prefixed 16-byte ALU form
    /// on x86-64, so every 128-bit RMW goes through libatomic.  This is also
    /// what GCC does — measured on gcc 16.2 and 12.2, `-O2`, with and without
    /// `-mcx16`/`-mavx2`: `jmp __atomic_fetch_add_16` for every ordering.
    fn libatomic_fetch16_symbol(op: AtomicRmwOp) -> &'static str {
        match op {
            AtomicRmwOp::Add => "__atomic_fetch_add_16",
            AtomicRmwOp::Sub => "__atomic_fetch_sub_16",
            AtomicRmwOp::And => "__atomic_fetch_and_16",
            AtomicRmwOp::Or => "__atomic_fetch_or_16",
            AtomicRmwOp::Xor => "__atomic_fetch_xor_16",
            AtomicRmwOp::Nand => "__atomic_fetch_nand_16",
            AtomicRmwOp::Xchg => "__atomic_exchange_16",
            // Test-and-set is byte-wide and returns from the fast path above.
            AtomicRmwOp::TestAndSet => unreachable!("test-and-set is byte-wide"),
        }
    }

    /// Call one of libatomic's `(void *mem, __int128 val, int model)` 16-byte
    /// entry points.  The fetch forms return the previous value in RDX:RAX;
    /// `__atomic_store_16` has the same shape and returns nothing, so it uses
    /// this too and simply leaves RAX/RDX undefined.
    ///
    /// ABI, measured on gcc 16.2 against the system libatomic.so.1: the
    /// 128-bit value is the SECOND parameter, so it occupies RSI (low) and RDX
    /// (high) — NOT the RDX:RCX pair, which only applies when the value is the
    /// first parameter after a pointer-of-pointer (as in the CAS below); the
    /// model is the third parameter, in ECX; and a 16-byte INTEGER return
    /// comes back in RAX (low) / RDX (high).
    ///
    /// RSP is not moved, so the frame's existing 16-byte call alignment holds
    /// at the `call`.  That depends on the function having been marked as one
    /// that calls out: `calculate_stack_space` sets `func_has_calls` for any
    /// 128-bit atomic, without which a leaf would take the frame-less path and
    /// reach this call with RSP ≡ 8 (mod 16).
    fn emit_libatomic_16_binary(
        &mut self,
        sym: &'static str,
        ptr: &Operand,
        val: &Operand,
        order: AtomicOrdering,
    ) {
        self.prepare_atomic_side_effect();
        self.operand_to_rax(ptr);
        self.state.emit("    movq %rax, %rdi");
        self.operand_to_rax_rdx(val);
        self.state.emit("    movq %rax, %rsi");
        // RDX already holds the high half exactly where the ABI wants it.
        self.state.emit_fmt(format_args!(
            "    movl ${}, %ecx",
            Self::libatomic_order(order)
        ));
        self.state.emit_fmt(format_args!("    call {}@PLT", sym));
    }

    /// `__int128 __atomic_load_16(void *mem, int model)`: RDI = mem, ESI =
    /// model, and the value returns in RAX (low) / RDX (high).  Measured on
    /// gcc 16.2 — GCC uses this for every ordering, with and without `-mcx16`.
    fn emit_libatomic_16_load(&mut self, dest: &Value, ptr: &Operand, order: AtomicOrdering) {
        self.prepare_atomic_side_effect();
        self.operand_to_rax(ptr);
        self.state.emit("    movq %rax, %rdi");
        self.state.emit_fmt(format_args!(
            "    movl ${}, %esi",
            Self::libatomic_order(order)
        ));
        self.state.emit("    call __atomic_load_16@PLT");
        self.store_rax_rdx_to(dest);
    }

    /// 16-byte `__atomic_compare_exchange_n`.  With the CMPXCHG16B permission
    /// (`-mcx16` or a v2+ profile) this is one `lock cmpxchg16b`; otherwise it
    /// calls libatomic's `__atomic_compare_exchange_16`, as GCC does.  The
    /// result is the bool (`returns_bool`) or the observed old value.
    fn emit_atomic_cmpxchg16_impl(
        &mut self,
        dest: &Value,
        ptr: &Operand,
        expected: &Operand,
        desired: &Operand,
        success_ordering: AtomicOrdering,
        failure_ordering: AtomicOrdering,
        returns_bool: bool,
    ) {
        self.prepare_atomic_side_effect();
        if super::super::isa::current().cx16 {
            // rbx is callee-saved and cmpxchg16b needs it for the desired low
            // half.  Stack after the pushes: [rsp] = expected (lo, hi),
            // [rsp+16] = desired (lo, hi), [rsp+32] = saved rbx.
            //
            // Both operands are staged through the stack rather than loaded
            // straight into RAX:RDX / RBX:RCX, which is deliberate.  The
            // instruction's register set is fixed (it compares RDX:RAX with
            // the memory operand and stores RCX:RBX on a match), so RBX and
            // RCX must be written before the last operand read.  Were an
            // operand ever homed in RBX or RCX, that read would see the value
            // we had just written.  Today `regalloc::is_non_gpr_type` keeps
            // every I128/U128 value in a slot, so the direct form would be
            // safe -- but it would couple this emitter to that allocator
            // predicate across two modules, and a future allocator change
            // would turn the violation into a silent miscompile.  The pushes
            // are L1-hot and store-forward, so they cost a few cycles against
            // the ~20-40 cycle locked operation; unconditional correctness is
            // worth more than that here.
            self.state.emit("    pushq %rbx");
            self.push_16_operand(desired, 8);
            self.push_16_operand(expected, 24);
            self.with_rsp_bias(40, |s| s.operand_to_rax(ptr));
            self.state.emit("    movq %rax, %r8");
            self.state.emit("    movq (%rsp), %rax");
            self.state.emit("    movq 8(%rsp), %rdx");
            self.state.emit("    movq 16(%rsp), %rbx");
            self.state.emit("    movq 24(%rsp), %rcx");
            self.state.emit("    lock cmpxchg16b (%r8)");
            if returns_bool {
                // Consume ZF here, before the stack cleanup: `addq` writes the
                // flags.  %al is dead on this path -- the bool is the whole
                // result -- so the flag lands straight in the result register
                // and the observed pair in RAX:RDX is discarded.  This also
                // keeps %r11 out of the clobber set.
                self.state.emit("    sete %al");
            }
            self.state.emit("    addq $32, %rsp");
            self.state.emit("    popq %rbx");
            if returns_bool {
                self.state.emit("    movzbl %al, %eax");
                self.store_rax_to(dest);
            } else {
                self.store_rax_rdx_to(dest);
            }
        } else {
            // libatomic's sized entry: bool __atomic_compare_exchange_16(
            // ptr, expected*, desired (by value: rdx:rcx), success, failure).
            // On failure it writes the observed value back into `expected`,
            // which is the old value returned here.  Stack after the pushes:
            // [rsp] = expected, [rsp+16] = ptr, [rsp+24] = pad (keeps the call
            // 16-byte aligned).
            self.state.emit("    subq $8, %rsp");
            self.with_rsp_bias(8, |s| s.operand_to_rax(ptr));
            self.state.emit("    pushq %rax");
            self.push_16_operand(expected, 16);
            self.with_rsp_bias(32, |s| s.operand_to_rax_rdx(desired));
            self.state.emit("    movq %rdx, %rcx");
            self.state.emit("    movq %rax, %rdx");
            self.state.emit("    movq 16(%rsp), %rdi");
            self.state.emit("    movq %rsp, %rsi");
            self.state.emit_fmt(format_args!(
                "    movl ${}, %r8d",
                Self::libatomic_order(success_ordering)
            ));
            self.state.emit_fmt(format_args!(
                "    movl ${}, %r9d",
                Self::libatomic_order(failure_ordering)
            ));
            self.state.emit("    call __atomic_compare_exchange_16@PLT");
            if returns_bool {
                self.state.emit("    movzbl %al, %eax");
                self.state.emit("    addq $32, %rsp");
                self.store_rax_to(dest);
            } else {
                self.state.emit("    movq (%rsp), %rax");
                self.state.emit("    movq 8(%rsp), %rdx");
                self.state.emit("    addq $32, %rsp");
                self.store_rax_rdx_to(dest);
            }
        }
    }

    pub(super) fn emit_atomic_load_impl(
        &mut self,
        dest: &Value,
        ptr: &Operand,
        ty: IrType,
        ordering: AtomicOrdering,
    ) {
        // 16-byte atomic load: the `movq` form below reads only the low half,
        // and no single 16-byte load is atomic by itself on x86-64 (an aligned
        // SSE move is an implementation guarantee, not an architectural one,
        // and is unavailable under `-mno-sse`).  Call libatomic, as GCC 16.2
        // does for every ordering.
        if matches!(ty, IrType::I128 | IrType::U128) {
            self.emit_libatomic_16_load(dest, ptr, ordering);
            return;
        }
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
        // 16-byte atomic store: there is no 16-byte `XCHG` on x86-64, and the
        // `xchgq`/`movq` forms below would write only the low half — both
        // half-wrong and non-atomic for the pair.  Call libatomic, as GCC 16.2
        // does for every ordering, with and without `-mcx16`.
        if matches!(ty, IrType::I128 | IrType::U128) {
            self.emit_libatomic_16_binary("__atomic_store_16", ptr, val, ordering);
            return;
        }
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
