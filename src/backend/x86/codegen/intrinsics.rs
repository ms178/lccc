//! x86-64 SSE/AES/CRC intrinsic emission and floating-point math intrinsics.
//!
//! Handles the `emit_intrinsic` trait method for the x86-64 backend, covering:
//! - Memory fences (lfence, mfence, sfence, pause, clflush)
//! - Non-temporal stores (movnti, movntdq, movntpd)
//! - SSE/SSE2 128-bit packed operations (arithmetic, compare, shuffle, shift)
//! - SSE2 element insertion/extraction and type conversion
//! - AES-NI encryption/decryption and key generation
//! - CLMUL carry-less multiplication
//! - CRC32 instructions
//! - Frame/return address intrinsics
//! - SSE scalar float math (sqrt, fabs) for F32/F64

use crate::ir::reexports::{
    IntrinsicOp,
    IrConst,
    Operand,
    Value,
};
use crate::backend::state::StackSlot;
use super::emit::{is_xmm_reg, phys_reg_name, X86Codegen};

impl X86Codegen {
    /// Load a float operand into %xmm0. Handles both Value operands (from stack)
    /// and float constants (loaded via their bit pattern into rax first).
    fn float_operand_to_xmm0(&mut self, op: &Operand, is_f32: bool) {
        // This writes %xmm0 outside the sse_load_arg/store_dest cache
        // protocol: any vector last-store entry would become stale.
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
        match op {
            Operand::Const(c) => {
                match c {
                    IrConst::F64(v) => {
                        let bits = v.to_bits() as i64;
                        if bits == 0 {
                            self.state.emit("    xorpd %xmm0, %xmm0");
                        } else if bits >= i32::MIN as i64 && bits <= i32::MAX as i64 {
                            self.state.out.emit_instr_imm_reg("    movq", bits, "rax");
                            self.state.emit("    movq %rax, %xmm0");
                        } else {
                            self.state.out.emit_instr_imm_reg("    movabsq", bits, "rax");
                            self.state.emit("    movq %rax, %xmm0");
                        }
                    }
                    IrConst::F32(v) => {
                        let bits = v.to_bits() as i32;
                        if bits == 0 {
                            self.state.emit("    xorps %xmm0, %xmm0");
                        } else {
                            self.state.out.emit_instr_imm_reg("    movl", bits as i64, "eax");
                            self.state.emit("    movd %eax, %xmm0");
                        }
                    }
                    _ => {
                        // Integer or other constants - load to rax and move to xmm
                        self.operand_to_reg(op, "rax");
                        if is_f32 {
                            self.state.emit("    movd %eax, %xmm0");
                        } else {
                            self.state.emit("    movq %rax, %xmm0");
                        }
                    }
                }
            }
            Operand::Value(_) => {
                // Load from stack slot to rax, then to xmm0
                self.operand_to_reg(op, "rax");
                if is_f32 {
                    self.state.emit("    movd %eax, %xmm0");
                } else {
                    self.state.emit("    movq %rax, %xmm0");
                }
            }
        }
    }

    fn emit_nontemporal_store(&mut self, op: &IntrinsicOp, dest_ptr: &Option<Value>, args: &[Operand]) {
        let Some(ptr) = dest_ptr else { return };
        match op {
            IntrinsicOp::Movnti => {
                self.operand_to_reg(&args[0], "rcx");
                self.value_to_reg(ptr, "rax");
                self.state.emit("    movnti %ecx, (%rax)");
            }
            IntrinsicOp::Movnti64 => {
                self.operand_to_reg(&args[0], "rcx");
                self.value_to_reg(ptr, "rax");
                self.state.emit("    movnti %rcx, (%rax)");
            }
            IntrinsicOp::Movntdq | IntrinsicOp::Movntpd => {
                // Register-aware source load (GPR bases / direct slots /
                // last-stored values). The DESTINATION may only use the
                // GPR-base form "(%r10)": movntdq/movntpd REQUIRE 16-byte
                // alignment, and a stack slot's address is only 8-aligned —
                // the direct-slot form "movntdq %xmm0, slot(%rsp)" faults
                // (#GP) on _Alignas(16) destinations. value_to_reg
                // materializes the address with the alignment dance.
                let is_pd = matches!(op, IntrinsicOp::Movntpd);
                if let Some(mem) = self.vec_arg_mem(&args[0]) {
                    self.state
                        .emit_fmt(format_args!("    movdqu {}, %xmm0", mem));
                } else {
                    self.sse_load_arg(&args[0], "xmm0");
                }
                let inst = if is_pd { "movntpd" } else { "movntdq" };
                let mut stored = false;
                if let Some(reg) = self.dest_reg(ptr) {
                    if !is_xmm_reg(reg) && X86Codegen::VEC_BASE_SAFE_REGS.contains(&reg.0) {
                        self.state
                            .emit_fmt(format_args!("    {} %xmm0, (%{})", inst, phys_reg_name(reg)));
                        stored = true;
                    }
                }
                if !stored {
                    self.value_to_reg(ptr, "rax");
                    self.state
                        .emit_fmt(format_args!("    {} %xmm0, (%rax)", inst));
                }
                self.state.sse_last_store_reg = false;
            }
            _ => {}
        }
    }

    /// Emit SSE binary 128-bit op: load xmm0 from arg0 ptr, xmm1 from arg1 ptr,
    /// apply the given SSE instruction, store result xmm0 to dest_ptr.
    /// Load a vector operand into an XMM register.
    ///
    /// When the operand is a slot-resolvable address (the common case for
    /// vector temporaries), emits a single `movdqu slot(%rbp), %xmmN`,
    /// skipping the address materialization + indirect load that the legacy
    /// path emitted. Falls back to the legacy address-in-%rax path for
    /// computed pointers, constants, and over-aligned allocas. The slot
    /// remains the value's home, so this is purely an addressing
    /// optimization — no semantic change.
    pub(super) fn sse_load_arg(&mut self, arg: &Operand, xmm: &'static str) {
        if let Operand::Value(v) = arg {
            // CCC_ENABLE_VECREG: value provably in its allocated XMM register.
            if let Some(&held) = self.state.vec_live_regs.get(&v.0) {
                if held != xmm {
                    self.state
                        .emit_fmt(format_args!("    movdqa %{}, %{}", held, xmm));
                    self.state.sse_last_store_val = Some(v.0);
                    self.state.sse_last_store_reg = true;
                    self.state.sse_last_store_reg_name = Some(xmm);
                }
                return;
            }
            // Single-entry last-store peephole, generalized to ANY source
            // register (pblendvb results live in %xmm2). A real load of some
            // other value clears `sse_last_store_reg`, so the held register is
            // provably untouched whenever this fires.
            if self.state.sse_last_store_reg && self.state.sse_last_store_val == Some(v.0) {
                let held = self.state.sse_last_store_reg_name.unwrap_or("xmm0");
                if held != xmm {
                    self.state
                        .emit_fmt(format_args!("    movdqa %{}, %{}", held, xmm));
                    self.state.sse_last_store_reg_name = Some(xmm);
                } else {
                    self.state.sse_last_store_reg = false;
                }
                // The deferred value really flowed through the register: the
                // pending store is never needed (the v5 win).
                if self.state.pending_vec_store.map(|(p, _, _)| p) == Some(v.0) {
                    self.state.pending_vec_store = None;
                }
                return;
            }
            // Real memory load: flush any pending deferred store first — this
            // load either reads the never-written slot or clobbers the holding
            // register (lazy-flush soundness rule).
            self.flush_pending_vec_store_impl();
            // GPR-base / direct-slot addressing: single memory operand, no
            // `movq %r10, %rax` round trip (the v5 compare-chain win).
            if let Some(mem) = self.value_ptr_mem_operand(v.0) {
                self.state
                    .emit_fmt(format_args!("    movdqu {}, %{}", mem, xmm));
                self.state.sse_last_store_reg = false;
                return;
            }
        }
        self.flush_pending_vec_store_impl();
        self.operand_to_reg(arg, "rax");
        self.state.emit_fmt(format_args!("    movdqu (%rax), %{}", xmm));
        self.state.sse_last_store_reg = false;
    }

    /// Store an XMM register to a vector operand's home slot.
    pub(super) fn sse_store_dest(&mut self, dest_ptr: &Value, xmm: &'static str) {
        // CCC_ENABLE_VECREG redirect: keep the result in its allocated register.
        if let Some(&reg) = self.reg_assignments.get(&dest_ptr.0) {
            if is_xmm_reg(reg) {
                let name = phys_reg_name(reg);
                if name != xmm {
                    self.state
                        .emit_fmt(format_args!("    movdqa %{}, %{}", xmm, name));
                }
                self.state.vec_live_regs.insert(dest_ptr.0, name);
                self.state.sse_last_store_val = Some(dest_ptr.0);
                self.state.sse_last_store_reg = true;
                self.state.sse_last_store_reg_name = Some(name);
                return;
            }
        }
        let deferred = self.state.vector_defer_values.contains(&dest_ptr.0);
        if deferred && std::env::var("CCC_DEBUG_VDEFER").is_ok() {
            eprintln!("[VDEFER-EMIT] deferring result store for dest_ptr={}", dest_ptr.0);
        }
        use crate::backend::state::SlotAddr;
        if let Some(addr) = self.state.resolve_slot_addr(dest_ptr.0) {
            if let SlotAddr::Direct(slot) = addr {
                if !deferred {
                    self.state.emit_fmt(format_args!(
                        "    movdqu %{}, {}",
                        xmm,
                        self.slot_ref(slot.0)
                    ));
                } else {
                    // Lazy flush: keep the store pending; emitted only if the
                    // value does not flow into its consumer via the register.
                    self.state.pending_vec_store = Some((dest_ptr.0, xmm, false));
                }
                self.state.sse_last_store_slot = Some(slot.0);
                self.state.sse_last_store_val = Some(dest_ptr.0);
                self.state.sse_last_store_reg = true;
                self.state.sse_last_store_reg_name = Some(xmm);
                return;
            }
        }
        if !deferred {
            self.value_to_reg(dest_ptr, "rax");
            self.state.emit_fmt(format_args!("    movdqu %{}, (%rax)", xmm));
        } else {
            self.state.pending_vec_store = Some((dest_ptr.0, xmm, false));
        }
        self.state.sse_last_store_val = Some(dest_ptr.0);
        self.state.sse_last_store_reg = true;
        self.state.sse_last_store_reg_name = Some(xmm);
    }

    /// Lazy-flush a pending deferred vector-result store (v5): the producer
    /// kept its result in the holding register instead of storing it. Called
    /// whenever anything is about to clobber that register, read the slot, or
    /// leave the block without the consumer having taken the value — emits the
    /// store that was originally skipped, making deferred stores sound by
    /// construction. No-op when nothing is pending.
    pub(super) fn flush_pending_vec_store_impl(&mut self) {
        let Some((val_id, reg, is_256)) = self.state.pending_vec_store.take() else {
            return;
        };
        if std::env::var("CCC_DEBUG_VDEFER").is_ok() {
            eprintln!("[VDEFER-EMIT] flushing pending store for val={}", val_id);
        }
        let val = Value(val_id);
        use crate::backend::state::SlotAddr;
        let inst = if is_256 { "vmovdqu" } else { "movdqu" };
        if let Some(addr) = self.state.resolve_slot_addr(val_id) {
            if let SlotAddr::Direct(slot) = addr {
                self.state.emit_fmt(format_args!(
                    "    {} %{}, {}",
                    inst,
                    reg,
                    self.slot_ref(slot.0)
                ));
                if is_256 {
                    self.state.vec_last_store_slot = Some(slot.0);
                    self.state.vec_last_store_val = Some(val_id);
                    self.state.vec_last_store_reg = true;
                    self.state.vec_last_store_reg_name = Some(reg);
                } else {
                    self.state.sse_last_store_slot = Some(slot.0);
                    self.state.sse_last_store_val = Some(val_id);
                    self.state.sse_last_store_reg = true;
                    self.state.sse_last_store_reg_name = Some(reg);
                }
                return;
            }
        }
        self.value_to_reg(&val, "rax");
        self.state
            .emit_fmt(format_args!("    {} %{}, (%rax)", inst, reg));
        if is_256 {
            self.state.vec_last_store_val = Some(val_id);
            self.state.vec_last_store_reg = true;
            self.state.vec_last_store_reg_name = Some(reg);
        } else {
            self.state.sse_last_store_val = Some(val_id);
            self.state.sse_last_store_reg = true;
            self.state.sse_last_store_reg_name = Some(reg);
        }
    }

    /// v5 lazy-flush entry check for an intrinsic about to be emitted: if a
    /// deferred vector result is pending and this instruction is NOT the
    /// cache-aware consumer of that value, flush the store first — this
    /// instruction may clobber the holding register or rely on slot contents.
    fn service_pending_vec_store(&mut self, op: &IntrinsicOp, args: &[Operand]) {
        use crate::backend::stack_layout::copy_coalescing::is_raw_reader_intrinsic;
        let Some((pval, _, _)) = self.state.pending_vec_store else {
            return;
        };
        use crate::ir::intrinsics::IntrinsicOp as O;
        let consumed_here = args
            .iter()
            .any(|a| matches!(a, Operand::Value(v) if v.0 == pval))
            && !is_raw_reader_intrinsic(op)
            && !matches!(
                op,
                O::Pblendvb128
                    | O::Storedqu
                    | O::Storeu256
                    | O::Store256
                    | O::Storeldi128
                    | O::Movntdq
                    | O::Movntpd
            );
        if !consumed_here {
            self.flush_pending_vec_store_impl();
        }
    }

    pub(super) fn emit_sse_binary_128(&mut self, dest_ptr: &Value, args: &[Operand], sse_inst: &str) {
        // Load operands into separate registers (direct slot addressing when
        // possible), perform the op, store the result to the destination slot.
        //
        // If args[1] is still provably in %xmm0 (the last-stored / deferred
        // value), load it FIRST into %xmm1 (a reg-to-reg move) so that
        // args[0]'s load into %xmm0 cannot clobber it. This makes the v5
        // deferred-store sound for the common `r = op(x, fresh_result)` shape.
        assert!(
            args.len() >= 2,
            "emit_sse_binary_128: malformed intrinsic {} ({} args)",
            sse_inst,
            args.len()
        );
        let a1_last = matches!(&args[1], Operand::Value(v)
            if self.state.sse_last_store_reg && self.state.sse_last_store_val == Some(v.0));
        if a1_last {
            self.sse_load_arg(&args[1], "xmm1");
            self.sse_load_arg(&args[0], "xmm0");
        } else {
            self.sse_load_arg(&args[0], "xmm0");
            self.sse_load_arg(&args[1], "xmm1");
        }
        self.state.emit_fmt(format_args!("    {} %xmm1, %xmm0", sse_inst));
        self.sse_store_dest(dest_ptr, "xmm0");
    }

    /// Emit SSE unary 128-bit op with immediate: load xmm0 from arg0 ptr,
    /// apply `inst $imm, %xmm0`, store result xmm0 to dest_ptr.
    fn emit_sse_unary_imm_128(&mut self, dest_ptr: &Value, args: &[Operand], sse_inst: &str) {
        self.sse_load_arg(&args[0], "xmm0");
        let imm = self.operand_to_imm_i64(&args[1]);
        self.state.emit_fmt(format_args!("    {} ${}, %xmm0", sse_inst, imm));
        self.sse_store_dest(dest_ptr, "xmm0");
    }

    /// Emit SSE shuffle with immediate: load xmm0, apply `inst $imm, %xmm0, %xmm0`,
    /// store result. Used for pshufd/pshuflw/pshufhw which read and write same register.

    pub(super) fn avx_load_arg_to(&mut self, arg: &Operand, ymm: &'static str) {
        if let Operand::Value(v) = arg {
            // CCC_ENABLE_VECREG (reserved for 256-bit values; 128-bit-only pool
            // in v5, so this is currently inert for ymm targets).
            if let Some(&held) = self.state.vec_live_regs.get(&v.0) {
                if held != ymm {
                    self.state
                        .emit_fmt(format_args!("    vmovdqa %{}, %{}", held, ymm));
                    self.state.vec_last_store_val = Some(v.0);
                    self.state.vec_last_store_reg = true;
                    self.state.vec_last_store_reg_name = Some(ymm);
                }
                return;
            }
            // Vector-register peephole (value-based): if the very last emitted
            // vector store wrote THIS value into its register, that register
            // still holds it — skip the reload and rename into the target
            // register. Applies to ANY target register (not just ymm0): the
            // binary emitters' load-order swap loads a deferred args[1] FIRST
            // into %ymm1, and gating this on ymm0 made that path reload the
            // never-written slot (miscompile in 256-bit defer chains).
            if self.state.vec_last_store_reg
                && self.state.vec_last_store_val == Some(v.0)
            {
                let held = self.state.vec_last_store_reg_name.unwrap_or("ymm0");
                if held != ymm {
                    self.state
                        .emit_fmt(format_args!("    vmovdqa %{}, %{}", held, ymm));
                    self.state.vec_last_store_reg_name = Some(ymm);
                } else {
                    self.state.vec_last_store_reg = false;
                }
                // The deferred value really flowed through the register: the
                // pending store is never needed (the v5 win).
                if self.state.pending_vec_store.map(|(p, _, _)| p) == Some(v.0) {
                    self.state.pending_vec_store = None;
                }
                return;
            }
            // Real memory load: flush any pending deferred store first
            // (lazy-flush soundness rule).
            self.flush_pending_vec_store_impl();
            // GPR-base / direct-slot addressing (v5 compare-chain win).
            if let Some(mem) = self.value_ptr_mem_operand(v.0) {
                self.state
                    .emit_fmt(format_args!("    vmovdqu {}, %{}", mem, ymm));
                self.state.vec_last_store_reg = false;
                return;
            }
        }
        self.flush_pending_vec_store_impl();
        self.operand_to_reg(arg, "rax");
        self.state.emit_fmt(format_args!("    vmovdqu (%rax), %{}", ymm));
        self.state.vec_last_store_reg = false;
    }
    pub(super) fn avx_load_arg(&mut self, arg: &Operand) { self.avx_load_arg_to(arg, "ymm0"); }

    /// Store %ymm0 to a 256-bit operand's home slot.
    pub(super) fn avx_store_dest(&mut self, dest_ptr: &Value) {
        let deferred = self.state.vector_defer_values.contains(&dest_ptr.0);
        if deferred && std::env::var("CCC_DEBUG_VDEFER").is_ok() {
            eprintln!("[VDEFER-EMIT] deferring result store for dest_ptr={}", dest_ptr.0);
        }
        use crate::backend::state::SlotAddr;
        if let Some(addr) = self.state.resolve_slot_addr(dest_ptr.0) {
            if let SlotAddr::Direct(slot) = addr {
                if !deferred {
                    self.state
                        .emit_fmt(format_args!("    vmovdqu %ymm0, {}", self.slot_ref(slot.0)));
                } else {
                    self.state.pending_vec_store = Some((dest_ptr.0, "ymm0", true));
                }
                self.state.vec_last_store_slot = Some(slot.0);
                self.state.vec_last_store_val = Some(dest_ptr.0);
                self.state.vec_last_store_reg = true;
                self.state.vec_last_store_reg_name = Some("ymm0");
                return;
            }
        }
        if !deferred {
            self.value_to_reg(dest_ptr, "rax");
            self.state.emit("    vmovdqu %ymm0, (%rax)");
        } else {
            self.state.pending_vec_store = Some((dest_ptr.0, "ymm0", true));
        }
        self.state.vec_last_store_val = Some(dest_ptr.0);
        self.state.vec_last_store_reg = true;
        self.state.vec_last_store_reg_name = Some("ymm0");
    }

    /// Memory operand for a vector arg, unless the value is still provably in a
    /// register (last-stored/deferred or vec_live) — its slot contents may be
    /// stale under the v5 deferred-store optimization. Returns None then, so
    /// the caller routes through the register cache instead of reading memory.
    fn vec_arg_mem(&self, arg: &Operand) -> Option<String> {
        match arg {
            Operand::Value(v) => {
                if self.state.vec_live_regs.contains_key(&v.0)
                    || (self.state.vec_last_store_reg && self.state.vec_last_store_val == Some(v.0))
                    || (self.state.sse_last_store_reg && self.state.sse_last_store_val == Some(v.0))
                {
                    return None;
                }
                self.operand_ptr_mem_operand(arg)
            }
            _ => None,
        }
    }

    /// Extract the 16 payload bytes from any _Float128 constant form
    /// (LongDouble carries them explicitly; I128 carries them as bits).
    fn f128_const_bytes(op: &Operand) -> Option<[u8; 16]> {
        match op {
            Operand::Const(IrConst::LongDouble(_, bytes)) => Some(*bytes),
            Operand::Const(IrConst::I128(v)) => {
                Some((*v as u128).to_le_bytes())
            }
            _ => None,
        }
    }

    /// Load a 16-byte _Float128 operand into an XMM register. Deterministic
    /// slot addressing; falls back to a pointer load only for register-held
    /// values (avoids sse_load_arg's assumptions for F128 slots).
    fn emit_f128_operand_to_xmm(&mut self, arg: &Operand, xmm: &str) {
        if let Operand::Value(v) = arg {
            if let Some(slot) = self.state.get_slot(v.0) {
                self.state.out.emit_instr_rbp_reg("    movdqu", slot.0, xmm);
                return;
            }
            if let Some(&held) = self.state.vec_live_regs.get(&v.0) {
                if held != xmm {
                    self.state.emit_fmt(format_args!("    movdqa %{}, %{}", held, xmm));
                }
                return;
            }
        }
        self.operand_to_reg(arg, "rax");
        self.state.emit_fmt(format_args!("    movdqu (%rax), %{}", xmm));
    }

    /// Materialize a 16-byte _Float128 constant into an XMM register via a
    /// 16-byte stack scratch (constant propagation inlines literals into
    /// F128Fabs/F128Copysign intrinsics).
    fn emit_f128_const_to_xmm(&mut self, bytes: [u8; 16], xmm: &str) {
        let low = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
        let high = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
        self.state.emit("    subq $16, %rsp");
        self.state.emit_fmt(format_args!("    movabsq ${}, %rax", low as i64));
        self.state.emit("    movq %rax, (%rsp)");
        self.state.emit_fmt(format_args!("    movabsq ${}, %rax", high as i64));
        self.state.emit("    movq %rax, 8(%rsp)");
        self.state.emit_fmt(format_args!("    movdqu (%rsp), %{}", xmm));
        self.state.emit("    addq $16, %rsp");
    }

    /// Store a long-double (10-byte) operand into (%rsp)+off. Handles stack
    /// slots and _Float128/long-double constants; values without a slot are
    /// copied via %xmm0 (16 bytes, pad bytes harmless).
    fn emit_ld10_to_rsp(&mut self, op: &Operand, off: i64) {
        match op {
            Operand::Value(v) => {
                if let Some(slot) = self.state.get_slot(v.0) {
                    self.state.out.emit_instr_rbp_reg("    movq", slot.0, "rax");
                    self.state.emit_fmt(format_args!("    movq %rax, {}(%rsp)", off));
                    self.state.out.emit_instr_rbp_reg("    movzbl", slot.0 + 8, "ecx");
                    self.state.emit_fmt(format_args!("    movb %cl, {}(%rsp)", off + 8));
                    self.state.out.emit_instr_rbp_reg("    movzbl", slot.0 + 9, "ecx");
                    self.state.emit_fmt(format_args!("    movb %cl, {}(%rsp)", off + 9));
                } else {
                    // Register-held value: 16-byte copy through %xmm0.
                    self.emit_store_f128_xmm0_dest_to_rsp(v, off);
                }
            }
            Operand::Const(IrConst::LongDouble(_, bytes)) => {
                let low = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
                let high = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
                self.state.emit_fmt(format_args!("    movabsq ${}, %rax", low as i64));
                self.state.emit_fmt(format_args!("    movq %rax, {}(%rsp)", off));
                self.state.emit_fmt(format_args!("    movabsq ${}, %rax", high as i64));
                self.state.emit_fmt(format_args!("    movq %rax, {}(%rsp)", off + 8));
            }
            _ => {
                // Should not happen (frontend only produces these two shapes).
                self.state.emit("    ud2");
            }
        }
    }

    /// Copy a register-held 16-byte value to (%rsp)+off via %xmm0.
    fn emit_store_f128_xmm0_dest_to_rsp(&mut self, v: &Value, off: i64) {
        if let Some(&held) = self.state.vec_live_regs.get(&v.0) {
            self.state.emit_fmt(format_args!("    movdqa %{}, %xmm0", held));
        } else if let Some(&reg) = self.reg_assignments.get(&v.0) {
            if is_xmm_reg(reg) {
                self.state.emit_fmt(format_args!("    movdqa %{}, %xmm0", phys_reg_name(reg)));
            } else {
                self.state.emit_fmt(format_args!("    movq %{}, %xmm0", phys_reg_name(reg)));
            }
        } else {
            self.operand_to_reg(&Operand::Value(*v), "rax");
            self.state.emit("    movdqu (%rax), %xmm0");
        }
        self.state.emit_fmt(format_args!("    movdqu %xmm0, {}(%rsp)", off));
    }

    /// AVX2 256-bit unary op with immediate, 3-operand form: `inst $imm, %ymm1, %ymm0`.
    fn emit_avx_unary_imm_256(&mut self, dest_ptr: &Value, args: &[Operand], inst: &str) {
        self.avx_load_arg(&args[0]);
        // The assembler's vpsll*/vpsrl* encoders use the VEX 3-operand form
        // ($imm, src, dst); the 2-operand legacy form is not accepted.
        self.state.emit_fmt(format_args!("    {} %ymm0, %ymm0", inst));
        self.avx_store_dest(dest_ptr);
    }
    fn emit_sse_shuffle_imm_128(&mut self, dest_ptr: &Value, args: &[Operand], sse_inst: &str) {
        self.sse_load_arg(&args[0], "xmm0");
        let imm = self.operand_to_imm_i64(&args[1]);
        self.state.emit_fmt(format_args!("    {} ${}, %xmm0, %xmm0", sse_inst, imm));
        self.sse_store_dest(dest_ptr, "xmm0");
    }

    pub(super) fn emit_intrinsic_impl(&mut self, dest: &Option<Value>, op: &IntrinsicOp, dest_ptr: &Option<Value>, args: &[Operand]) {
        // v5 lazy flush: a deferred vector result may be pending in a register.
        // Flush it before any intrinsic that is not its cache-aware consumer
        // (fences/pause/rdtsc neither clobber XMM regs nor read vector slots,
        // so they let the pending value pass untouched).
        match op {
            IntrinsicOp::Lfence | IntrinsicOp::Mfence | IntrinsicOp::Sfence
            | IntrinsicOp::Pause | IntrinsicOp::Rdtsc | IntrinsicOp::Vzeroupper => {}
            _ => self.service_pending_vec_store(op, args),
        }
        match op {
            IntrinsicOp::Lfence => { self.state.emit("    lfence"); }
            IntrinsicOp::Mfence => { self.state.emit("    mfence"); }
            IntrinsicOp::Sfence => { self.state.emit("    sfence"); }
            IntrinsicOp::Pause => { self.state.emit("    pause"); }
            IntrinsicOp::Vzeroupper => { self.state.emit("    vzeroupper"); }
            IntrinsicOp::Rdtsc => {
                // rdtsc: EDX:EAX -> RAX (matches GCC __builtin_ia32_rdtsc)
                self.state.emit("    rdtsc");
                self.state.emit("    shlq $32, %rdx");
                self.state.emit("    orq %rdx, %rax");
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
            }
            IntrinsicOp::Rdtscp => {
                // rdtscp: EDX:EAX -> RAX, IA32_TSC_AUX (ecx) -> *args[0]
                self.state.emit("    rdtscp");
                self.operand_to_reg(&args[0], "rdi");
                self.state.emit("    movl %ecx, (%rdi)");
                self.state.emit("    shlq $32, %rdx");
                self.state.emit("    orq %rdx, %rax");
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
            }
            IntrinsicOp::Clflush => {
                // args[0] = pointer to flush
                self.operand_to_reg(&args[0], "rax");
                self.state.emit("    clflush (%rax)");
            }
            IntrinsicOp::Movnti | IntrinsicOp::Movnti64
            | IntrinsicOp::Movntdq | IntrinsicOp::Movntpd => {
                self.emit_nontemporal_store(op, dest_ptr, args);
            }
            IntrinsicOp::Loaddqu => {
                if let Some(dptr) = dest_ptr {
                    self.sse_load_arg(&args[0], "xmm0");
                    self.sse_store_dest(dptr, "xmm0");
                }
            }
            IntrinsicOp::Storedqu => {
                if let Some(ptr) = dest_ptr {
                    self.sse_load_arg(&args[0], "xmm0");
                    self.sse_store_dest(ptr, "xmm0");
                }
            }
            IntrinsicOp::Pcmpeqb128 | IntrinsicOp::Pcmpeqd128
            | IntrinsicOp::Psubusb128 | IntrinsicOp::Psubsb128
            | IntrinsicOp::Por128
            | IntrinsicOp::Pand128 | IntrinsicOp::Pxor128
            | IntrinsicOp::AddPs128 | IntrinsicOp::SubPs128 | IntrinsicOp::MulPs128
            | IntrinsicOp::AddPd128 | IntrinsicOp::SubPd128 | IntrinsicOp::MulPd128 => {
                if let Some(dptr) = dest_ptr {
                    let inst = match op {
                        IntrinsicOp::Pcmpeqb128 => "pcmpeqb",
                        IntrinsicOp::Pcmpeqd128 => "pcmpeqd",
                        IntrinsicOp::Psubusb128 => "psubusb",
                        IntrinsicOp::Psubsb128 => "psubsb",
                        IntrinsicOp::Por128 => "por",
                        IntrinsicOp::Pand128 => "pand",
                        IntrinsicOp::Pxor128 => "pxor",
                        IntrinsicOp::AddPs128 => "addps",
                        IntrinsicOp::SubPs128 => "subps",
                        IntrinsicOp::MulPs128 => "mulps",
                        IntrinsicOp::AddPd128 => "addpd",
                        IntrinsicOp::SubPd128 => "subpd",
                        IntrinsicOp::MulPd128 => "mulpd",
                        _ => unreachable!("unexpected SSE binary op: {:?}", op),
                    };
                    self.emit_sse_binary_128(dptr, args, inst);
                }
            }
            IntrinsicOp::Pmovmskb128 => {
                self.sse_load_arg(&args[0], "xmm0");
                self.state.emit("    pmovmskb %xmm0, %eax");
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
            }
            IntrinsicOp::SetEpi8 => {
                if let Some(dptr) = dest_ptr {
                    self.operand_to_reg(&args[0], "rax");
                    self.state.emit("    movd %eax, %xmm0");
                    self.state.emit("    punpcklbw %xmm0, %xmm0");
                    self.state.emit("    punpcklwd %xmm0, %xmm0");
                    self.state.emit("    pshufd $0, %xmm0, %xmm0");
                    self.sse_store_dest(dptr, "xmm0");
                }
            }
            IntrinsicOp::SetEpi32 => {
                if let Some(dptr) = dest_ptr {
                    self.operand_to_reg(&args[0], "rax");
                    self.state.emit("    movd %eax, %xmm0");
                    self.state.emit("    pshufd $0, %xmm0, %xmm0");
                    self.sse_store_dest(dptr, "xmm0");
                }
            }
            IntrinsicOp::Crc32_8 | IntrinsicOp::Crc32_16
            | IntrinsicOp::Crc32_32 | IntrinsicOp::Crc32_64 => {
                self.operand_to_reg(&args[0], "rax");
                self.operand_to_reg(&args[1], "rcx");
                let inst = match op {
                    IntrinsicOp::Crc32_8  => "crc32b %cl, %eax",
                    IntrinsicOp::Crc32_16 => "crc32w %cx, %eax",
                    IntrinsicOp::Crc32_32 => "crc32l %ecx, %eax",
                    IntrinsicOp::Crc32_64 => "crc32q %rcx, %rax",
                    _ => unreachable!("CRC32 dispatch matched non-CRC32 op: {:?}", op),
                };
                self.state.emit_fmt(format_args!("    {}", inst));
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
            }
            IntrinsicOp::FrameAddress => {
                // __builtin_frame_address(0): return current frame pointer (rbp)
                self.state.emit("    movq %rbp, %rax");
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
            }
            IntrinsicOp::ReturnAddress => {
                // __builtin_return_address(0): return address is above the frame
                if self.state.out.use_rsp_addressing {
                    let off = self.state.out.rsp_frame_size;
                    self.state.emit_fmt(format_args!("    movq {}(%rsp), %rax", off));
                } else {
                    self.state.emit("    movq 8(%rbp), %rax");
                }
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
            }
            IntrinsicOp::ThreadPointer => {
                // __builtin_thread_pointer(): read the TLS base from %fs:0
                self.state.emit("    movq %fs:0, %rax");
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
            }
            IntrinsicOp::SqrtF64 => {
                // sqrtsd: scalar double-precision square root
                self.float_operand_to_xmm0(&args[0], false);
                self.state.emit("    sqrtsd %xmm0, %xmm0");
                self.state.emit("    movq %xmm0, %rax");
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
            }
            IntrinsicOp::SqrtF32 => {
                // sqrtss: scalar single-precision square root
                self.float_operand_to_xmm0(&args[0], true);
                self.state.emit("    sqrtss %xmm0, %xmm0");
                self.state.emit("    movd %xmm0, %eax");
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
            }
            IntrinsicOp::FabsF64 => {
                // Clear sign bit for double-precision absolute value
                self.float_operand_to_xmm0(&args[0], false);
                self.state.emit("    movabsq $0x7FFFFFFFFFFFFFFF, %rcx");
                self.state.emit("    movq %rcx, %xmm1");
                self.state.emit("    andpd %xmm1, %xmm0");
                self.state.emit("    movq %xmm0, %rax");
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
            }
            IntrinsicOp::F128Fabs => {
                // _Float128 (16B in %xmm0): clear sign bit 127 (bit 63 of the
                // high qword). 5 uops, no memory operand, no rodata mask.
                if let Some(bytes) = Self::f128_const_bytes(&args[0]) {
                    self.emit_f128_const_to_xmm(bytes, "xmm0");
                } else {
                    self.emit_f128_operand_to_xmm(&args[0], "xmm0");
                }
                // Sign bit 127 lives in the HIGH qword: move it down with
                // movhlps, clear bit 63 with btrq (no 64-bit constant load —
                // the peephole must not eat the mask), re-insert via punpcklqdq.
                self.state.emit("    movhlps %xmm0, %xmm1");
                self.state.emit("    movq %xmm1, %rax");
                self.state.emit("    btrq $63, %rax");
                self.state.emit("    movq %rax, %xmm1");
                self.state.emit("    punpcklqdq %xmm1, %xmm0");
                self.state.sse_last_store_reg = false;
                if let Some(d) = dest {
                    // 16-byte bit-pattern value: mark so all copy/load/store
                    // paths treat it as 128-bit (not low-64-only).
                    self.state.i128_values.insert(d.0);
                    self.emit_store_f128_xmm0(d);
                }
            }
            IntrinsicOp::F128Neg => {
                // Toggle sign bit 127 (HIGH qword bit 63), constant-free
                // (btcq), matching fabs's structure. 6 uops, no memory.
                if let Some(bytes) = Self::f128_const_bytes(&args[0]) {
                    self.emit_f128_const_to_xmm(bytes, "xmm0");
                } else {
                    self.emit_f128_operand_to_xmm(&args[0], "xmm0");
                }
                self.state.emit("    movhlps %xmm0, %xmm1");
                self.state.emit("    movq %xmm1, %rax");
                // btcq TOGGLES bit 63: btsq only SETS it, so negating an
                // already-negative value (-4.0, bit set) left it negative.
                self.state.emit("    btcq $63, %rax");
                self.state.emit("    movq %rax, %xmm1");
                self.state.emit("    punpcklqdq %xmm1, %xmm0");
                self.state.sse_last_store_reg = false;
                if let Some(d) = dest {
                    // 16-byte bit-pattern value: mark so all copy/load/store
                    // paths treat it as 128-bit (not low-64-only).
                    self.state.i128_values.insert(d.0);
                    self.emit_store_f128_xmm0(d);
                }
            }
            IntrinsicOp::F128Copysign => {
                // _Float128 copysign(x, y): high qword = (x_hi & ~sign) | (y_hi & sign),
                // low qword = x_lo. 9 uops, no memory operands.
                if let Some(bytes) = Self::f128_const_bytes(&args[0]) {
                    self.emit_f128_const_to_xmm(bytes, "xmm0");
                } else {
                    self.emit_f128_operand_to_xmm(&args[0], "xmm0");
                }
                if let Some(bytes) = Self::f128_const_bytes(&args[1]) {
                    self.emit_f128_const_to_xmm(bytes, "xmm1");
                } else {
                    self.emit_f128_operand_to_xmm(&args[1], "xmm1");
                }
                // y's sign bit (bit 127, HIGH qword) and x's magnitude —
                // constant-free (btrq/shrq/shlq) so no 64-bit mask load can
                // be dropped by the peephole.
                self.state.emit("    movhlps %xmm1, %xmm2");
                self.state.emit("    movq %xmm2, %rdx");
                self.state.emit("    shrq $63, %rdx");
                self.state.emit("    movhlps %xmm0, %xmm2");
                self.state.emit("    movq %xmm2, %rax");
                self.state.emit("    btrq $63, %rax");
                self.state.emit("    shlq $63, %rdx");
                self.state.emit("    orq %rdx, %rax");
                self.state.emit("    movq %rax, %xmm1");
                self.state.emit("    punpcklqdq %xmm1, %xmm0");
                self.state.sse_last_store_reg = false;
                if let Some(d) = dest {
                    // 16-byte bit-pattern value: mark so all copy/load/store
                    // paths treat it as 128-bit (not low-64-only).
                    self.state.i128_values.insert(d.0);
                    self.emit_store_f128_xmm0(d);
                }
            }
            IntrinsicOp::LDFabs => {
                // long double (80-bit x87, 10 bytes in a 16-byte slot): clear
                // bit 79 (byte 9 bit 7). Pure GPR, no x87 round-trip.
                if let Some(d) = dest {
                    let sx = match &args[0] {
                        Operand::Value(v) => self.state.get_slot(v.0).map(|s| s.0),
                        _ => None,
                    };
                    let sd = self.state.get_slot(d.0).map(|s| s.0);
                    if let (Some(sx), Some(sd)) = (sx, sd) {
                        self.state.out.emit_instr_rbp_reg("    movq", sx, "rax");
                        self.state.out.emit_instr_reg_rbp("    movq", "rax", sd);
                        self.state.out.emit_instr_rbp_reg("    movzbl", sx + 8, "ecx");
                        self.state.out.emit_instr_reg_rbp("    movb", "cl", sd + 8);
                        self.state.out.emit_instr_rbp_reg("    movzbl", sx + 9, "ecx");
                        self.state.emit("    andb $0x7f, %cl");
                        self.state.out.emit_instr_reg_rbp("    movb", "cl", sd + 9);
                        // x87-format result: the F128 return path must fldt
                        // from this slot, not fildq-convert a U128 integer.
                        if let Some(d) = dest {
                            self.state.f128_direct_slots.insert(d.0);
                        }
                        return;
                    }
                    // Constant operand: emit the 16-byte payload directly.
                    if let (Operand::Const(IrConst::LongDouble(_, bytes)), Some(sd)) = (&args[0], sd) {
                        let low = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
                        let high = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
                        self.state.emit_fmt(format_args!("    movabsq ${}, %rax", low as i64));
                        self.state.out.emit_instr_reg_rbp("    movq", "rax", sd);
                        self.state.emit_fmt(format_args!("    movabsq ${}, %rax", high as i64));
                        self.state.out.emit_instr_reg_rbp("    movq", "rax", sd + 8);
                        self.state.out.emit_instr_rbp_reg("    movzbl", sd + 9, "ecx");
                        self.state.emit("    andb $0x7f, %cl");
                        self.state.out.emit_instr_reg_rbp("    movb", "cl", sd + 9);
                        return;
                    }
                    panic!("LDFabs: unsupported operand shape");
                }
            }
            IntrinsicOp::LDCopysign => {
                // long double copysign(x, y): byte9 = (y9 & 0x80) | (x9 & 0x7f),
                // bytes 0..8 copied from x. Pure GPR, no x87 round-trip.
                if let Some(d) = dest {
                    let sx = match &args[0] {
                        Operand::Value(v) => self.state.get_slot(v.0).map(|s| s.0),
                        _ => None,
                    };
                    let sy = match &args[1] {
                        Operand::Value(v) => self.state.get_slot(v.0).map(|s| s.0),
                        _ => None,
                    };
                    let sd = self.state.get_slot(d.0).map(|s| s.0);
                    if let (Some(sx), Some(sy), Some(sd)) = (sx, sy, sd) {
                        self.state.out.emit_instr_rbp_reg("    movq", sx, "rax");
                        self.state.out.emit_instr_reg_rbp("    movq", "rax", sd);
                        self.state.out.emit_instr_rbp_reg("    movzbl", sx + 8, "ecx");
                        self.state.out.emit_instr_reg_rbp("    movb", "cl", sd + 8);
                        self.state.out.emit_instr_rbp_reg("    movzbl", sy + 9, "eax");
                        self.state.emit("    andb $0x80, %al");
                        self.state.out.emit_instr_rbp_reg("    movzbl", sx + 9, "ecx");
                        self.state.emit("    andb $0x7f, %cl");
                        self.state.emit("    orb %cl, %al");
                        self.state.out.emit_instr_reg_rbp("    movb", "al", sd + 9);
                        if let Some(d) = dest {
                            self.state.f128_direct_slots.insert(d.0);
                        }
                        return;
                    }
                    // Generic path: materialize both operands in a 32-byte
                    // scratch buffer at (%rsp), combine, copy to dest
                    // (glibc k_casinhl: copysignl(x, const)).
                    self.state.emit("    subq $32, %rsp");
                    self.emit_ld10_to_rsp(&args[0], 0);
                    self.emit_ld10_to_rsp(&args[1], 16);
                    self.state.emit("    movzbl 25(%rsp), %eax");
                    self.state.emit("    andb $0x80, %al");
                    self.state.emit("    movzbl 9(%rsp), %ecx");
                    self.state.emit("    andb $0x7f, %cl");
                    self.state.emit("    orb %cl, %al");
                    self.state.emit("    movb %al, 9(%rsp)");
                    if let Some(sd) = sd {
                        self.state.emit("    movq (%rsp), %rax");
                        self.state.out.emit_instr_reg_rbp("    movq", "rax", sd);
                        self.state.emit("    movzbl 8(%rsp), %ecx");
                        self.state.out.emit_instr_reg_rbp("    movb", "cl", sd + 8);
                        self.state.emit("    movzbl 9(%rsp), %ecx");
                        self.state.out.emit_instr_reg_rbp("    movb", "cl", sd + 9);
                    } else {
                        // dest without slot: 16-byte copy through %xmm0
                        self.state.emit("    movdqu (%rsp), %xmm0");
                        self.state.f128_direct_slots.insert(d.0);
                        self.emit_store_f128_xmm0(d);
                    }
                    self.state.emit("    addq $32, %rsp");
                    return;
                }
            }
            IntrinsicOp::FabsF32 => {
                // Clear sign bit for single-precision absolute value
                self.float_operand_to_xmm0(&args[0], true);
                self.state.emit("    movl $0x7FFFFFFF, %ecx");
                self.state.emit("    movd %ecx, %xmm1");
                self.state.emit("    andps %xmm1, %xmm0");
                self.state.emit("    movd %xmm0, %eax");
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
            }
            // AES-NI binary ops: aesenc, aesenclast, aesdec, aesdeclast
            IntrinsicOp::Aesenc128 | IntrinsicOp::Aesenclast128
            | IntrinsicOp::Aesdec128 | IntrinsicOp::Aesdeclast128 => {
                if let Some(dptr) = dest_ptr {
                    let inst = match op {
                        IntrinsicOp::Aesenc128 => "aesenc",
                        IntrinsicOp::Aesenclast128 => "aesenclast",
                        IntrinsicOp::Aesdec128 => "aesdec",
                        IntrinsicOp::Aesdeclast128 => "aesdeclast",
                        _ => unreachable!("AES-NI dispatch matched non-AES op: {:?}", op),
                    };
                    self.emit_sse_binary_128(dptr, args, inst);
                }
            }
            // AES-NI unary: aesimc
            IntrinsicOp::Aesimc128 => {
                if let Some(dptr) = dest_ptr {
                    self.sse_load_arg(&args[0], "xmm0");
                    self.state.emit("    aesimc %xmm0, %xmm0");
                    self.sse_store_dest(dptr, "xmm0");
                }
            }
            // AES-NI: aeskeygenassist with immediate
            IntrinsicOp::Aeskeygenassist128 => {
                if let Some(dptr) = dest_ptr {
                    self.sse_load_arg(&args[0], "xmm0");
                    // args[1] is the immediate value
                    let imm = self.operand_to_imm_i64(&args[1]);
                    self.state.emit_fmt(format_args!("    aeskeygenassist ${}, %xmm0, %xmm0", imm));
                    self.sse_store_dest(dptr, "xmm0");
                }
            }
            // CLMUL: pclmulqdq with immediate
            IntrinsicOp::Pclmulqdq128 => {
                if let Some(dptr) = dest_ptr {
                    self.sse_load_arg(&args[0], "xmm0");
                    self.sse_load_arg(&args[1], "xmm1");
                    let imm = self.operand_to_imm_i64(&args[2]);
                    self.state.emit_fmt(format_args!("    pclmulqdq ${}, %xmm1, %xmm0", imm));
                    self.sse_store_dest(dptr, "xmm0");
                }
            }
            // SSE2 shift-by-immediate operations
            IntrinsicOp::Pslldqi128 | IntrinsicOp::Psrldqi128
            | IntrinsicOp::Psllqi128 | IntrinsicOp::Psrlqi128 => {
                if let Some(dptr) = dest_ptr {
                    let inst = match op {
                        IntrinsicOp::Pslldqi128 => "pslldq",
                        IntrinsicOp::Psrldqi128 => "psrldq",
                        IntrinsicOp::Psllqi128 => "psllq",
                        IntrinsicOp::Psrlqi128 => "psrlq",
                        _ => unreachable!("unexpected SSE shift-by-immediate op: {:?}", op),
                    };
                    self.emit_sse_unary_imm_128(dptr, args, inst);
                }
            }
            // SSE2 shuffle with immediate (3-operand form: inst $imm, %src, %dst)
            IntrinsicOp::Pshufd128 => {
                if let Some(dptr) = dest_ptr {
                    self.emit_sse_shuffle_imm_128(dptr, args, "pshufd");
                }
            }
            // Load low 64 bits, zero upper (MOVQ)
            IntrinsicOp::Loadldi128 => {
                if let Some(dptr) = dest_ptr {
                    let mut loaded = false;
                    if let Some(mem) = self.vec_arg_mem(&args[0]) {
                        self.state.emit_fmt(format_args!("    movq {}, %xmm0", mem));
                        loaded = true;
                    } else if let Operand::Value(v) = &args[0] {
                        if let Some(&held) = self.state.vec_live_regs.get(&v.0) {
                            if held != "xmm0" {
                                self.state
                                    .emit_fmt(format_args!("    movq %{}, %xmm0", held));
                            }
                            loaded = true;
                        }
                        if !loaded {
                            loaded = (self.state.sse_last_store_reg
                                && self.state.sse_last_store_val == Some(v.0))
                                || (self.state.vec_last_store_reg
                                    && self.state.vec_last_store_val == Some(v.0));
                        }
                    }
                    if !loaded {
                        self.operand_to_reg(&args[0], "rax");
                        self.state.emit("    movq (%rax), %xmm0");
                    }
                    self.sse_store_dest(dptr, "xmm0");
                }
            }

            // Legacy PBLENDVB reads its mask implicitly from XMM0.
            IntrinsicOp::Pblendvb128 => {
                if let Some(dptr) = dest_ptr {
                    // args = [a, b, mask]; result = mask ? b : a
                    assert_eq!(args.len(), 3, "Pblendvb128 requires a, b, mask");
                    self.sse_load_arg(&args[2], "xmm0");
                    self.sse_load_arg(&args[1], "xmm1");
                    self.sse_load_arg(&args[0], "xmm2");
                    self.state.emit("    pblendvb %xmm1, %xmm2");
                    self.sse_store_dest(dptr, "xmm2");
                }
            }
            // PBLENDW: packed 16-bit blend with immediate mask.
            // _mm_blend_epi16(a, b, imm8) → pblendw $imm8, b, a
            IntrinsicOp::Pblendw128 => {
                if let Some(dptr) = dest_ptr {
                    // args = [a, b, imm8]; result = blend words per imm8 bits
                    self.sse_load_arg(&args[0], "xmm0");
                    self.sse_load_arg(&args[1], "xmm1");
                    let imm = self.operand_to_imm_i64(&args[2]);
                    self.state.emit_fmt(format_args!("    pblendw ${}, %xmm1, %xmm0", imm));
                    self.sse_store_dest(dptr, "xmm0");
                }
            }
            // SSE2 binary 128-bit operations
            IntrinsicOp::Paddw128 | IntrinsicOp::Psubw128 | IntrinsicOp::Pmulhw128
            | IntrinsicOp::Pmullw128
            | IntrinsicOp::Pmuludq128 | IntrinsicOp::Pmuldq128 | IntrinsicOp::Pmulld128
            | IntrinsicOp::Pmaddwd128 | IntrinsicOp::Pmaddubsw128
            | IntrinsicOp::Pcmpgtw128 | IntrinsicOp::Pcmpgtb128
            | IntrinsicOp::Paddd128 | IntrinsicOp::Psubd128
            | IntrinsicOp::Paddb128 | IntrinsicOp::Psubb128 | IntrinsicOp::Psubusw128
            | IntrinsicOp::Psadbw128
            | IntrinsicOp::Pshufb128
            | IntrinsicOp::Pmaxub128 | IntrinsicOp::Pminub128
            | IntrinsicOp::Pmovzxbw128 | IntrinsicOp::Pmovzxwd128
            | IntrinsicOp::Packssdw128 | IntrinsicOp::Packsswb128 | IntrinsicOp::Packuswb128
            | IntrinsicOp::Punpcklbw128 | IntrinsicOp::Punpckhbw128
            | IntrinsicOp::Punpcklwd128 | IntrinsicOp::Punpckhwd128 => {
                if let Some(dptr) = dest_ptr {
                    let inst = match op {
                        IntrinsicOp::Paddw128 => "paddw",
                        IntrinsicOp::Psubw128 => "psubw",
                        IntrinsicOp::Pmulhw128 => "pmulhw",
                        IntrinsicOp::Pmuludq128 => "pmuludq",
                        IntrinsicOp::Pmuldq128 => "pmuldq",
                        IntrinsicOp::Pmulld128 => "pmulld",
                        IntrinsicOp::Pmullw128 => "pmullw",
                        IntrinsicOp::Pmaddwd128 => "pmaddwd",
                        IntrinsicOp::Pmaddubsw128 => "pmaddubsw",
                        IntrinsicOp::Pcmpgtw128 => "pcmpgtw",
                        IntrinsicOp::Pcmpgtb128 => "pcmpgtb",
                        IntrinsicOp::Paddd128 => "paddd",
                        IntrinsicOp::Psubd128 => "psubd",
                        IntrinsicOp::Paddb128 => "paddb",
                        IntrinsicOp::Psubb128 => "psubb",
                        IntrinsicOp::Psubusw128 => "psubusw",
                        IntrinsicOp::Psadbw128 => "psadbw",
                        IntrinsicOp::Pshufb128 => "pshufb",
                        IntrinsicOp::Pmaxub128 => "pmaxub",
                        IntrinsicOp::Pminub128 => "pminub",
                        IntrinsicOp::Pmovzxbw128 => "pmovzxbw",
                        IntrinsicOp::Pmovzxwd128 => "pmovzxwd",
                        IntrinsicOp::Packssdw128 => "packssdw",
                        IntrinsicOp::Packsswb128 => "packsswb",
                        IntrinsicOp::Packuswb128 => "packuswb",
                        IntrinsicOp::Punpcklbw128 => "punpcklbw",
                        IntrinsicOp::Punpckhbw128 => "punpckhbw",
                        IntrinsicOp::Punpcklwd128 => "punpcklwd",
                        IntrinsicOp::Punpckhwd128 => "punpckhwd",
                        _ => unreachable!("unexpected SSE binary op: {:?}", op),
                    };
                    self.emit_sse_binary_128(dptr, args, inst);
                }
            }
            // SSSE3 horizontal add and alignr (imm arg in args[2] for alignr)
            IntrinsicOp::Phaddw128 | IntrinsicOp::Phaddd128 | IntrinsicOp::Palignr128 => {
                if let Some(dptr) = dest_ptr {
                    let inst = match op {
                        IntrinsicOp::Phaddw128 => "phaddw",
                        IntrinsicOp::Phaddd128 => "phaddd",
                        IntrinsicOp::Palignr128 => "palignr",
                        _ => unreachable!(),
                    };
                    self.sse_load_arg(&args[0], "xmm0");
                    self.sse_load_arg(&args[1], "xmm1");
                    if matches!(op, IntrinsicOp::Palignr128) {
                        let imm = self.operand_to_imm_i64(&args[2]);
                        self.state.emit_fmt(format_args!("    palignr ${}, %xmm1, %xmm0", imm));
                    } else {
                        self.state.emit_fmt(format_args!("    {} %xmm1, %xmm0", inst));
                    }
                    self.sse_store_dest(dptr, "xmm0");
                }
            }
            // SSE2 variable-shift (count in xmm register)
            IntrinsicOp::Psllw128 | IntrinsicOp::Psrlw128 => {
                if let Some(dptr) = dest_ptr {
                    let inst = if matches!(op, IntrinsicOp::Psllw128) { "psllw" } else { "psrlw" };
                    self.sse_load_arg(&args[0], "xmm0");
                    self.sse_load_arg(&args[1], "xmm1");
                    self.state.emit_fmt(format_args!("    {} %xmm1, %xmm0", inst));
                    self.sse_store_dest(dptr, "xmm0");
                }
            }

            // pabsb/pabsw/pabsd are UNARY (2-operand AT&T: src, dst). Fixes the
            // v4 latent bug where Pabsb128 was dispatched through the binary
            // emitter (panicked / malformed on `_mm_abs_epi8`).
            IntrinsicOp::Pabsb128 | IntrinsicOp::Pabsw128 | IntrinsicOp::Pabsd128 => {
                if let Some(dptr) = dest_ptr {
                    let inst = match op {
                        IntrinsicOp::Pabsb128 => "pabsb",
                        IntrinsicOp::Pabsw128 => "pabsw",
                        IntrinsicOp::Pabsd128 => "pabsd",
                        _ => unreachable!(),
                    };
                    self.sse_load_arg(&args[0], "xmm0");
                    self.state.emit_fmt(format_args!("    {} %xmm0, %xmm0", inst));
                    self.sse_store_dest(dptr, "xmm0");
                }
            }
            // --- AVX2 256-bit integer ops ---
            IntrinsicOp::Paddb256 | IntrinsicOp::Paddw256 | IntrinsicOp::Paddd256
            | IntrinsicOp::Psubb256 | IntrinsicOp::Psubw256 | IntrinsicOp::Psubusw256
            | IntrinsicOp::Psadbw256 | IntrinsicOp::Pmaddubsw256 | IntrinsicOp::Pmaddwd256
            | IntrinsicOp::Pcmpeqb256 | IntrinsicOp::Pcmpgtb256 | IntrinsicOp::Pshufb256
            | IntrinsicOp::Pmaxub256 | IntrinsicOp::Pminub256
            | IntrinsicOp::Pxor256 | IntrinsicOp::Por256 | IntrinsicOp::Pand256 => {
                if let Some(dptr) = dest_ptr {
                    let inst = match op {
                        IntrinsicOp::Paddb256 => "vpaddb",
                        IntrinsicOp::Paddw256 => "vpaddw",
                        IntrinsicOp::Paddd256 => "vpaddd",
                        IntrinsicOp::Psubb256 => "vpsubb",
                        IntrinsicOp::Psubw256 => "vpsubw",
                        IntrinsicOp::Psubusw256 => "vpsubusw",
                        IntrinsicOp::Psadbw256 => "vpsadbw",
                        IntrinsicOp::Pmaddubsw256 => "vpmaddubsw",
                        IntrinsicOp::Pmaddwd256 => "vpmaddwd",
                        IntrinsicOp::Pcmpeqb256 => "vpcmpeqb",
                        IntrinsicOp::Pcmpgtb256 => "vpcmpgtb",
                        IntrinsicOp::Pshufb256 => "vpshufb",
                        IntrinsicOp::Pmaxub256 => "vpmaxub",
                        IntrinsicOp::Pminub256 => "vpminub",
                        IntrinsicOp::Pxor256 => "vpxor",
                        IntrinsicOp::Por256 => "vpor",
                        IntrinsicOp::Pand256 => "vpand",
                        _ => unreachable!("unexpected AVX2 binary op: {:?}", op),
                    };
                    // vpsadbw/padd*/pcmpeqb/pmaxub/pminub/vpxor/vpor/vpand/vpmaddwd
                    // are commutative -> args[0] may fold into the memory operand.
                    let commutative = matches!(
                        op,
                        IntrinsicOp::Paddb256 | IntrinsicOp::Paddw256 | IntrinsicOp::Paddd256
                            | IntrinsicOp::Psadbw256 | IntrinsicOp::Pmaddwd256
                            | IntrinsicOp::Pcmpeqb256 | IntrinsicOp::Pmaxub256
                            | IntrinsicOp::Pminub256 | IntrinsicOp::Pxor256
                            | IntrinsicOp::Por256 | IntrinsicOp::Pand256
                    );
                    self.emit_avx_binary_256(dptr, args, inst, commutative);
                }
            }
            // vpabsb/vpabsw are UNARY (2-operand AT&T: src, dst).
            IntrinsicOp::Pabsb256 | IntrinsicOp::Pabsw256 => {
                if let Some(dptr) = dest_ptr {
                    let inst = if matches!(op, IntrinsicOp::Pabsb256) { "vpabsb" } else { "vpabsw" };
                    if let Some(mem) = self.vec_arg_mem(&args[0]) {
                        self.state.emit_fmt(format_args!("    {} {}, %ymm0", inst, mem));
                    } else {
                        self.avx_load_arg(&args[0]);
                        self.state.emit_fmt(format_args!("    {} %ymm0, %ymm0", inst));
                    }
                    self.avx_store_dest(dptr);
                }
            }
            IntrinsicOp::Psllidi256 | IntrinsicOp::Psrlidi256
            | IntrinsicOp::Psllwi256 | IntrinsicOp::Psrlwi256 => {
                if let Some(dptr) = dest_ptr {
                    let inst = match op {
                        IntrinsicOp::Psllidi256 => "vpslld",
                        IntrinsicOp::Psrlidi256 => "vpsrld",
                        IntrinsicOp::Psllwi256 => "vpsllw",
                        IntrinsicOp::Psrlwi256 => "vpsrlw",
                        _ => unreachable!(),
                    };
                    self.avx_load_arg(&args[0]);
                    let imm = self.operand_to_imm_i64(&args[1]);
                    self.state
                        .emit_fmt(format_args!("    {} ${}, %ymm0, %ymm0", inst, imm));
                    self.avx_store_dest(dptr);
                }
            }
            IntrinsicOp::Loadu256 | IntrinsicOp::Load256 => {
                if let Some(dptr) = dest_ptr {
                    let inst = if matches!(op, IntrinsicOp::Loadu256) { "vmovdqu" } else { "vmovdqa" };
                    self.avx_load_arg(&args[0]);
                    self.state.emit_fmt(format_args!("    {} %ymm0, %ymm0", inst));
                    self.avx_store_dest(dptr);
                }
            }
            IntrinsicOp::Storeu256 | IntrinsicOp::Store256 => {
                if let Some(dptr) = dest_ptr {
                    self.avx_load_arg(&args[0]);
                    self.avx_store_dest(dptr);
                }
            }
            IntrinsicOp::Broadcast128to256 => {
                if let Some(dptr) = dest_ptr {
                    // vbroadcasti128 only has a memory-source form. When the
                    // source is slot-resolvable, broadcast from the slot
                    // directly (1 uop on the load port); otherwise spill the
                    // 128 bits to a scratch slot and broadcast from there.
                    use crate::backend::state::SlotAddr;
                    let mut broadcasted = false;
                    if let Operand::Value(v) = &args[0] {
                        if let Some(SlotAddr::Direct(slot)) = self.state.resolve_slot_addr(v.0) {
                            self.state.emit_fmt(format_args!(
                                "    vbroadcasti128 {}, %ymm0",
                                self.slot_ref(slot.0)
                            ));
                            broadcasted = true;
                        }
                    }
                    if !broadcasted {
                        self.sse_load_arg(&args[0], "xmm0");
                        self.state.emit("    vinserti128 $1, %xmm0, %ymm0, %ymm0");
                    }
                    self.avx_store_dest(dptr);
                }
            }
            IntrinsicOp::Zext128to256 => {
                if let Some(dptr) = dest_ptr {
                    // Zero-extend 128→256: VEX insert of the 128-bit value into
                    // a zeroed ymm. A lone vinserti128 $1 of the same xmm would
                    // DUPLICATE the low lane into the high lane (miscompile).
                    self.sse_load_arg(&args[0], "xmm0");
                    self.state.emit("    vpxor %ymm1, %ymm1, %ymm1");
                    self.state.emit("    vinserti128 $0, %xmm0, %ymm1, %ymm0");
                    self.avx_store_dest(dptr);
                }
            }
            IntrinsicOp::CastReinterpret128 => {
                // Free reinterpret cast: the operand pointer IS the result.
                // Nothing to emit (lowering returns the operand directly).
            }
            IntrinsicOp::Cast256to128 => {
                if let Some(dptr) = dest_ptr {
                    self.avx_load_arg(&args[0]);
                    self.state.emit("    vextracti128 $0, %ymm0, %xmm0");
                    self.sse_store_dest(dptr, "xmm0");
                }
            }
            IntrinsicOp::Insert128to256 => {
                if let Some(dptr) = dest_ptr {
                    self.avx_load_arg(&args[0]);
                    self.sse_load_arg(&args[1], "xmm1");
                    let lane = self.operand_to_imm_i64(&args[2]) & 1;
                    self.state.emit_fmt(format_args!("    vinserti128 ${}, %xmm1, %ymm0, %ymm0", lane));
                    self.avx_store_dest(dptr);
                }
            }
            IntrinsicOp::SetEpi8_256 | IntrinsicOp::SetEpi16_256 | IntrinsicOp::SetEpi32_256 | IntrinsicOp::SetEpi64x256 => {
                if let Some(dptr) = dest_ptr {
                    // Constant splats lower to a single memory-source broadcast:
                    //   vpbroadcast{b,w,d,q} .LvcN(%rip), %ymm0
                    // which is 1 uop handled by the load port (the modern best
                    // choice — no movd, no port-5 shuffle). Runtime values use
                    // vmovd+vbroadcast (matches GCC/LLVM on AVX2).
                    let (inst, bits) = match op {
                        IntrinsicOp::SetEpi8_256 => ("vpbroadcastb", 1u8),
                        IntrinsicOp::SetEpi16_256 => ("vpbroadcastw", 2u8),
                        IntrinsicOp::SetEpi32_256 => ("vpbroadcastd", 4u8),
                        IntrinsicOp::SetEpi64x256 => ("vpbroadcastq", 8u8),
                        _ => unreachable!(),
                    };
                    if let Operand::Const(_) = &args[0] {
                        let v = self.operand_to_imm_i64(&args[0]);
                        let mask = if bits == 8 { u64::MAX } else { (1u64 << (bits * 8)) - 1 };
                        let operand = self.vec_const_rip_operand((v as u64) & mask, bits);
                        self.state.emit_fmt(format_args!("    {} {}, %ymm0", inst, operand));
                    } else if self.avx512_enabled {
                        // EVEX GPR-source vpbroadcast: 1 uop on port 5, no movd.
                        let reg = if bits == 8 { "rax" } else { "eax" };
                        self.operand_to_reg(&args[0], reg);
                        self.state.emit_fmt(format_args!("    {} %{}, %ymm0", inst, reg));
                    } else {
                        // AVX2: vmovd + xmm-source vpbroadcast (matches GCC/LLVM).
                        let reg = if bits == 8 { "rax" } else { "eax" };
                        self.operand_to_reg(&args[0], reg);
                        if bits == 8 {
                            self.state.emit("    movq %rax, %xmm0");
                        } else {
                            self.state.emit("    movd %eax, %xmm0");
                        }
                        self.state.emit_fmt(format_args!("    {} %xmm0, %ymm0", inst));
                    }
                    self.avx_store_dest(dptr);
                }
            }
            // --- AVX-VNNI / INT8 / INT16 3-op dot products (128 + 256) ---
            IntrinsicOp::Dpbusd128 | IntrinsicOp::Dpbusds128 | IntrinsicOp::Dpwusd128
            | IntrinsicOp::Dpwusds128 | IntrinsicOp::Dpbssd128 | IntrinsicOp::Dpbssds128
            | IntrinsicOp::Dpbsud128 | IntrinsicOp::Dpbsuds128 | IntrinsicOp::Dpbuud128
            | IntrinsicOp::Dpbuuds128 | IntrinsicOp::Dpwuud128 | IntrinsicOp::Dpwuuds128
            | IntrinsicOp::Dpwssd128 | IntrinsicOp::Dpwssds128
            | IntrinsicOp::Dpbusd256 | IntrinsicOp::Dpbusds256 | IntrinsicOp::Dpwusd256
            | IntrinsicOp::Dpwusds256 | IntrinsicOp::Dpbssd256 | IntrinsicOp::Dpbssds256
            | IntrinsicOp::Dpbsud256 | IntrinsicOp::Dpbsuds256 | IntrinsicOp::Dpbuud256
            | IntrinsicOp::Dpbuuds256 | IntrinsicOp::Dpwuud256 | IntrinsicOp::Dpwuuds256
            | IntrinsicOp::Dpwssd256 | IntrinsicOp::Dpwssds256 => {
                if let Some(dptr) = dest_ptr {
                    let inst = match op {
                        IntrinsicOp::Dpbusd128 | IntrinsicOp::Dpbusd256 => "vpdpbusd",
                        IntrinsicOp::Dpbusds128 | IntrinsicOp::Dpbusds256 => "vpdpbusds",
                        IntrinsicOp::Dpwusd128 | IntrinsicOp::Dpwusd256 => "vpdpwusd",
                        IntrinsicOp::Dpwusds128 | IntrinsicOp::Dpwusds256 => "vpdpwusds",
                        IntrinsicOp::Dpbssd128 | IntrinsicOp::Dpbssd256 => "vpdpbssd",
                        IntrinsicOp::Dpbssds128 | IntrinsicOp::Dpbssds256 => "vpdpbssds",
                        IntrinsicOp::Dpbsud128 | IntrinsicOp::Dpbsud256 => "vpdpbsud",
                        IntrinsicOp::Dpbsuds128 | IntrinsicOp::Dpbsuds256 => "vpdpbsuds",
                        IntrinsicOp::Dpbuud128 | IntrinsicOp::Dpbuud256 => "vpdpbuud",
                        IntrinsicOp::Dpbuuds128 | IntrinsicOp::Dpbuuds256 => "vpdpbuuds",
                        IntrinsicOp::Dpwuud128 | IntrinsicOp::Dpwuud256 => "vpdpwuud",
                        IntrinsicOp::Dpwuuds128 | IntrinsicOp::Dpwuuds256 => "vpdpwuuds",
                        IntrinsicOp::Dpwssd128 | IntrinsicOp::Dpwssd256 => "vpdpwssd",
                        IntrinsicOp::Dpwssds128 | IntrinsicOp::Dpwssds256 => "vpdpwssds",
                        _ => unreachable!(),
                    };
                    let is256 = matches!(op, IntrinsicOp::Dpbusd256 | IntrinsicOp::Dpbusds256
                        | IntrinsicOp::Dpwusd256 | IntrinsicOp::Dpwusds256
                        | IntrinsicOp::Dpbssd256 | IntrinsicOp::Dpbssds256
                        | IntrinsicOp::Dpbsud256 | IntrinsicOp::Dpbsuds256
                        | IntrinsicOp::Dpbuud256 | IntrinsicOp::Dpbuuds256
                        | IntrinsicOp::Dpwuud256 | IntrinsicOp::Dpwuuds256
                        | IntrinsicOp::Dpwssd256 | IntrinsicOp::Dpwssds256);
                    if is256 {
                        self.avx_load_arg_to(&args[0], "ymm0"); self.avx_load_arg_to(&args[1], "ymm1"); self.avx_load_arg_to(&args[2], "ymm2");
                        self.state.emit_fmt(format_args!("    {} %ymm2, %ymm1, %ymm0", inst)); self.avx_store_dest(dptr);
                    } else {
                        self.sse_load_arg(&args[0], "xmm0"); self.sse_load_arg(&args[1], "xmm1"); self.sse_load_arg(&args[2], "xmm2");
                        self.state.emit_fmt(format_args!("    {} %xmm2, %xmm1, %xmm0", inst)); self.sse_store_dest(dptr, "xmm0");
                    }
                }
            }
            // --- GFNI ---
            IntrinsicOp::Gf2p8mulb128 => {
                if let Some(dptr) = dest_ptr {
                    self.sse_load_arg(&args[0], "xmm0");
                    self.sse_load_arg(&args[1], "xmm1");
                    self.state.emit("    gf2p8mulb %xmm1, %xmm0");
                    self.sse_store_dest(dptr, "xmm0");
                }
            }
            IntrinsicOp::Gf2p8affineqb128 | IntrinsicOp::Gf2p8affineinvqb128 => {
                if let Some(dptr) = dest_ptr {
                    let inst = if matches!(op, IntrinsicOp::Gf2p8affineqb128) {
                        "gf2p8affineqb"
                    } else {
                        "gf2p8affineinvqb"
                    };
                    self.sse_load_arg(&args[0], "xmm0");
                    self.sse_load_arg(&args[1], "xmm1");
                    let imm = self.operand_to_imm_i64(&args[2]);
                    // AT&T legacy form: imm, src, dst (dest is also NDS src).
                    self.state.emit_fmt(format_args!("    {} ${}, %xmm1, %xmm0", inst, imm));
                    self.sse_store_dest(dptr, "xmm0");
                }
            }
            // --- VAES 256-bit + VPCLMULQDQ 256-bit ---
            IntrinsicOp::Aesenc256 | IntrinsicOp::Aesenclast256
            | IntrinsicOp::Aesdec256 | IntrinsicOp::Aesdeclast256 => {
                if let Some(dptr) = dest_ptr {
                    let inst = match op {
                        IntrinsicOp::Aesenc256 => "vaesenc",
                        IntrinsicOp::Aesenclast256 => "vaesenclast",
                        IntrinsicOp::Aesdec256 => "vaesdec",
                        IntrinsicOp::Aesdeclast256 => "vaesdeclast",
                        _ => unreachable!(),
                    };
                    self.avx_load_arg(&args[0]);
                    if let Operand::Value(v) = &args[1] {
                        if let crate::backend::state::SlotAddr::Direct(slot) =
                            self.state.resolve_slot_addr(v.0).unwrap()
                        {
                            self.state
                                .emit_fmt(format_args!("    vmovdqu {}, %ymm1", self.slot_ref(slot.0)));
                        } else {
                            self.operand_to_reg(&args[1], "rcx");
                            self.state.emit("    vmovdqu (%rcx), %ymm1");
                        }
                    } else {
                        self.operand_to_reg(&args[1], "rcx");
                        self.state.emit("    vmovdqu (%rcx), %ymm1");
                    }
                    self.state.emit_fmt(format_args!("    {} %ymm1, %ymm0, %ymm0", inst));
                    self.avx_store_dest(dptr);
                }
            }
            IntrinsicOp::Vpclmulqdq256 => {
                if let Some(dptr) = dest_ptr {
                    self.avx_load_arg(&args[0]);
                    if let Operand::Value(v) = &args[1] {
                        if let crate::backend::state::SlotAddr::Direct(slot) =
                            self.state.resolve_slot_addr(v.0).unwrap()
                        {
                            self.state
                                .emit_fmt(format_args!("    vmovdqu {}, %ymm1", self.slot_ref(slot.0)));
                        } else {
                            self.operand_to_reg(&args[1], "rcx");
                            self.state.emit("    vmovdqu (%rcx), %ymm1");
                        }
                    } else {
                        self.operand_to_reg(&args[1], "rcx");
                        self.state.emit("    vmovdqu (%rcx), %ymm1");
                    }
                    let imm = self.operand_to_imm_i64(&args[2]);
                    self.state.emit_fmt(format_args!("    vpclmulqdq ${}, %ymm1, %ymm0, %ymm0", imm));
                    self.avx_store_dest(dptr);
                }
            }
            IntrinsicOp::Pmovmskb256 => {
                self.avx_load_arg(&args[0]);
                self.state.emit("    vpmovmskb %ymm0, %eax");
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
            }

            // --- Newly wired SSE2 ops (were scalar header loops) ---
            IntrinsicOp::Paddusb128 | IntrinsicOp::Paddsb128 | IntrinsicOp::Paddusw128
            | IntrinsicOp::Paddsw128 | IntrinsicOp::Psubsw128 | IntrinsicOp::Pandn128
            | IntrinsicOp::Pcmpeqw128 | IntrinsicOp::Pcmpgtd128 | IntrinsicOp::Pavgb128
            | IntrinsicOp::Pavgw128 | IntrinsicOp::Pminsw128 | IntrinsicOp::Pmaxsw128
            | IntrinsicOp::Pmulhuw128 | IntrinsicOp::Paddq128 | IntrinsicOp::Psubq128
            | IntrinsicOp::Punpckldq128 | IntrinsicOp::Punpckhdq128
            | IntrinsicOp::Punpcklqdq128 | IntrinsicOp::Punpckhqdq128 => {
                if let Some(dptr) = dest_ptr {
                    let inst = match op {
                        IntrinsicOp::Paddusb128 => "paddusb",
                        IntrinsicOp::Paddsb128 => "paddsb",
                        IntrinsicOp::Paddusw128 => "paddusw",
                        IntrinsicOp::Paddsw128 => "paddsw",
                        IntrinsicOp::Psubsw128 => "psubsw",
                        IntrinsicOp::Pandn128 => "pandn",
                        IntrinsicOp::Pcmpeqw128 => "pcmpeqw",
                        IntrinsicOp::Pcmpgtd128 => "pcmpgtd",
                        IntrinsicOp::Pavgb128 => "pavgb",
                        IntrinsicOp::Pavgw128 => "pavgw",
                        IntrinsicOp::Pminsw128 => "pminsw",
                        IntrinsicOp::Pmaxsw128 => "pmaxsw",
                        IntrinsicOp::Pmulhuw128 => "pmulhuw",
                        IntrinsicOp::Paddq128 => "paddq",
                        IntrinsicOp::Psubq128 => "psubq",
                        IntrinsicOp::Punpckldq128 => "punpckldq",
                        IntrinsicOp::Punpckhdq128 => "punpckhdq",
                        IntrinsicOp::Punpcklqdq128 => "punpcklqdq",
                        IntrinsicOp::Punpckhqdq128 => "punpckhqdq",
                        _ => unreachable!(),
                    };
                    self.emit_sse_binary_128(dptr, args, inst);
                }
            }
            IntrinsicOp::Setzero128 => {
                if let Some(dptr) = dest_ptr {
                    self.state.emit("    pxor %xmm0, %xmm0");
                    self.sse_store_dest(dptr, "xmm0");
                }
            }
            IntrinsicOp::Testz128 => {
                self.sse_load_arg(&args[0], "xmm0");
                self.sse_load_arg(&args[1], "xmm1");
                self.state.emit("    ptest %xmm1, %xmm0");
                self.state.emit("    sete %al");
                self.state.emit("    movzbl %al, %eax");
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
            }

            // --- Newly wired AVX/AVX2 ops ---
            IntrinsicOp::Pmulld256 | IntrinsicOp::Psubd256 | IntrinsicOp::Paddq256
            | IntrinsicOp::Psubq256 | IntrinsicOp::Pandn256 | IntrinsicOp::Pcmpeqd256
            | IntrinsicOp::Pcmpeqq256 | IntrinsicOp::Pcmpgtd256 | IntrinsicOp::Pcmpgtq256
            | IntrinsicOp::AddPs256 | IntrinsicOp::SubPs256 | IntrinsicOp::MulPs256
            | IntrinsicOp::AddPd256 | IntrinsicOp::SubPd256 | IntrinsicOp::MulPd256
            | IntrinsicOp::Punpcklbw256 | IntrinsicOp::Punpckhbw256
            | IntrinsicOp::Punpcklwd256 | IntrinsicOp::Punpckhwd256
            | IntrinsicOp::Punpckldq256 | IntrinsicOp::Punpckhdq256
            | IntrinsicOp::Punpcklqdq256 | IntrinsicOp::Punpckhqdq256
            | IntrinsicOp::Pmullw256 | IntrinsicOp::Pmulhw256
            | IntrinsicOp::Pminsd256 | IntrinsicOp::Pmaxsd256
            | IntrinsicOp::Packssdw256 | IntrinsicOp::Packuswb256
            | IntrinsicOp::Phaddw256 | IntrinsicOp::Phaddd256
            | IntrinsicOp::Pmuludq256 => {
                if let Some(dptr) = dest_ptr {
                    let (inst, comm) = match op {
                        IntrinsicOp::Pmulld256 => ("vpmulld", true),
                        IntrinsicOp::Psubd256 => ("vpsubd", false),
                        IntrinsicOp::Paddq256 => ("vpaddq", true),
                        IntrinsicOp::Psubq256 => ("vpsubq", false),
                        IntrinsicOp::Pandn256 => ("vpandn", false),
                        IntrinsicOp::Pcmpeqd256 => ("vpcmpeqd", true),
                        IntrinsicOp::Pcmpeqq256 => ("vpcmpeqq", true),
                        IntrinsicOp::Pcmpgtd256 => ("vpcmpgtd", false),
                        IntrinsicOp::Pcmpgtq256 => ("vpcmpgtq", false),
                        IntrinsicOp::AddPs256 => ("vaddps", true),
                        IntrinsicOp::SubPs256 => ("vsubps", false),
                        IntrinsicOp::MulPs256 => ("vmulps", true),
                        IntrinsicOp::AddPd256 => ("vaddpd", true),
                        IntrinsicOp::SubPd256 => ("vsubpd", false),
                        IntrinsicOp::MulPd256 => ("vmulpd", true),
                        IntrinsicOp::Punpcklbw256 => ("vpunpcklbw", false),
                        IntrinsicOp::Punpckhbw256 => ("vpunpckhbw", false),
                        IntrinsicOp::Punpcklwd256 => ("vpunpcklwd", false),
                        IntrinsicOp::Punpckhwd256 => ("vpunpckhwd", false),
                        IntrinsicOp::Punpckldq256 => ("vpunpckldq", false),
                        IntrinsicOp::Punpckhdq256 => ("vpunpckhdq", false),
                        IntrinsicOp::Punpcklqdq256 => ("vpunpcklqdq", false),
                        IntrinsicOp::Punpckhqdq256 => ("vpunpckhqdq", false),
                        IntrinsicOp::Pmullw256 => ("vpmullw", true),
                        IntrinsicOp::Pmulhw256 => ("vpmulhw", true),
                        IntrinsicOp::Pminsd256 => ("vpminsd", true),
                        IntrinsicOp::Pmaxsd256 => ("vpmaxsd", true),
                        IntrinsicOp::Packssdw256 => ("vpackssdw", false),
                        IntrinsicOp::Packuswb256 => ("vpackuswb", false),
                        IntrinsicOp::Phaddw256 => ("vphaddw", false),
                        IntrinsicOp::Phaddd256 => ("vphaddd", false),
                        IntrinsicOp::Pmuludq256 => ("vpmuludq", true),
                        _ => unreachable!(),
                    };
                    self.emit_avx_binary_256(dptr, args, inst, comm);
                }
            }
            IntrinsicOp::Setzero256 => {
                if let Some(dptr) = dest_ptr {
                    self.state.emit("    vpxor %ymm0, %ymm0, %ymm0");
                    self.avx_store_dest(dptr);
                }
            }
            IntrinsicOp::Extracti128 => {
                if let Some(dptr) = dest_ptr {
                    self.avx_load_arg(&args[0]);
                    let lane = self.operand_to_imm_i64(&args[1]) & 1;
                    self.state.emit_fmt(format_args!("    vextracti128 ${}, %ymm0, %xmm0", lane));
                    self.sse_store_dest(dptr, "xmm0");
                }
            }
            IntrinsicOp::LoaduPs256 | IntrinsicOp::LoaduPd256 => {
                if let Some(dptr) = dest_ptr {
                    self.avx_load_arg(&args[0]);
                    self.avx_store_dest(dptr);
                }
            }
            IntrinsicOp::StoreuPs256 | IntrinsicOp::StoreuPd256 => {
                if let Some(dptr) = dest_ptr {
                    self.avx_load_arg(&args[0]);
                    self.avx_store_dest(dptr);
                }
            }
            IntrinsicOp::Permute2x128 => {
                if let Some(dptr) = dest_ptr {
                    self.avx_load_arg(&args[0]);
                    self.avx_load_arg_to(&args[1], "ymm1");
                    let imm = self.operand_to_imm_i64(&args[2]);
                    self.state.emit_fmt(format_args!("    vperm2i128 ${}, %ymm1, %ymm0, %ymm0", imm));
                    self.avx_store_dest(dptr);
                }
            }
            IntrinsicOp::Permute4x64 => {
                if let Some(dptr) = dest_ptr {
                    self.avx_load_arg(&args[0]);
                    let imm = self.operand_to_imm_i64(&args[1]);
                    self.state.emit_fmt(format_args!("    vpermq ${}, %ymm0, %ymm0", imm));
                    self.avx_store_dest(dptr);
                }
            }
            // VPERMD: dest[i] = src[idx[i] & 7]. AT&T: vpermd %src, %idx, %dest
            // (Intel VPERMD dest, idx, src).
            IntrinsicOp::Permutevar8x32 => {
                if let Some(dptr) = dest_ptr {
                    self.avx_load_arg(&args[0]);
                    self.avx_load_arg_to(&args[1], "ymm1");
                    self.state.emit("    vpermd %ymm0, %ymm1, %ymm0");
                    self.avx_store_dest(dptr);
                }
            }
            IntrinsicOp::Pshufd256 => {
                if let Some(dptr) = dest_ptr {
                    self.avx_load_arg(&args[0]);
                    let imm = self.operand_to_imm_i64(&args[1]);
                    self.state.emit_fmt(format_args!("    vpshufd ${}, %ymm0, %ymm0", imm));
                    self.avx_store_dest(dptr);
                }
            }
            IntrinsicOp::Pslldqi256 | IntrinsicOp::Psrldqi256 => {
                if let Some(dptr) = dest_ptr {
                    self.avx_load_arg(&args[0]);
                    let imm = self.operand_to_imm_i64(&args[1]) & 0xff;
                    let inst = if matches!(op, IntrinsicOp::Pslldqi256) { "vpslldq" } else { "vpsrldq" };
                    self.state.emit_fmt(format_args!("    {} ${}, %ymm0, %ymm0", inst, imm));
                    self.avx_store_dest(dptr);
                }
            }
            IntrinsicOp::Psllqi256 | IntrinsicOp::Psrlqi256
            | IntrinsicOp::Psrawi256 | IntrinsicOp::Psradi256 => {
                if let Some(dptr) = dest_ptr {
                    self.avx_load_arg(&args[0]);
                    let imm = self.operand_to_imm_i64(&args[1]);
                    let inst = match op {
                        IntrinsicOp::Psllqi256 => "vpsllq",
                        IntrinsicOp::Psrlqi256 => "vpsrlq",
                        IntrinsicOp::Psrawi256 => "vpsraw",
                        IntrinsicOp::Psradi256 => "vpsrad",
                        _ => unreachable!(),
                    };
                    self.state.emit_fmt(format_args!("    {} ${}, %ymm0, %ymm0", inst, imm));
                    self.avx_store_dest(dptr);
                }
            }
            IntrinsicOp::Pmovzxbw256 | IntrinsicOp::Pmovzxbd256 | IntrinsicOp::Pmovzxwd256
            | IntrinsicOp::Pmovsxbw256 | IntrinsicOp::Pmovsxbd256 | IntrinsicOp::Pmovsxwd256 => {
                if let Some(dptr) = dest_ptr {
                    self.sse_load_arg(&args[0], "xmm0");
                    let inst = match op {
                        IntrinsicOp::Pmovzxbw256 => "vpmovzxbw",
                        IntrinsicOp::Pmovzxbd256 => "vpmovzxbd",
                        IntrinsicOp::Pmovzxwd256 => "vpmovzxwd",
                        IntrinsicOp::Pmovsxbw256 => "vpmovsxbw",
                        IntrinsicOp::Pmovsxbd256 => "vpmovsxbd",
                        IntrinsicOp::Pmovsxwd256 => "vpmovsxwd",
                        _ => unreachable!(),
                    };
                    self.state.emit_fmt(format_args!("    {} %xmm0, %ymm0", inst));
                    self.avx_store_dest(dptr);
                }
            }
            IntrinsicOp::Pabsd256 => {
                if let Some(dptr) = dest_ptr {
                    self.avx_load_arg(&args[0]);
                    self.state.emit("    vpabsd %ymm0, %ymm0");
                    self.avx_store_dest(dptr);
                }
            }

            // SSE2 element shift-by-immediate operations
            IntrinsicOp::Psllwi128 | IntrinsicOp::Psrlwi128 | IntrinsicOp::Psrawi128
            | IntrinsicOp::Psradi128 | IntrinsicOp::Pslldi128 | IntrinsicOp::Psrldi128 => {
                if let Some(dptr) = dest_ptr {
                    let inst = match op {
                        IntrinsicOp::Psllwi128 => "psllw",
                        IntrinsicOp::Psrlwi128 => "psrlw",
                        IntrinsicOp::Psrawi128 => "psraw",
                        IntrinsicOp::Psradi128 => "psrad",
                        IntrinsicOp::Pslldi128 => "pslld",
                        IntrinsicOp::Psrldi128 => "psrld",
                        _ => unreachable!("unexpected SSE element shift op: {:?}", op),
                    };
                    self.emit_sse_unary_imm_128(dptr, args, inst);
                }
            }

            // --- SSE2 set/insert/extract/convert ---
            IntrinsicOp::SetEpi16 => {
                // Broadcast 16-bit value to all 8 lanes
                if let Some(dptr) = dest_ptr {
                    self.operand_to_reg(&args[0], "rax");
                    self.state.emit("    movd %eax, %xmm0");
                    self.state.emit("    punpcklwd %xmm0, %xmm0");
                    self.state.emit("    pshufd $0, %xmm0, %xmm0");
                    self.sse_store_dest(dptr, "xmm0");
                }
            }
            IntrinsicOp::Pinsrw128 | IntrinsicOp::Pinsrd128
            | IntrinsicOp::Pinsrb128 | IntrinsicOp::Pinsrq128 => {
                // Insert scalar at lane: pinsrX $imm, %ecx, %xmm0
                if let Some(dptr) = dest_ptr {
                    let (inst, reg) = match op {
                        IntrinsicOp::Pinsrw128 => ("pinsrw", "ecx"),
                        IntrinsicOp::Pinsrd128 => ("pinsrd", "ecx"),
                        IntrinsicOp::Pinsrb128 => ("pinsrb", "ecx"),
                        IntrinsicOp::Pinsrq128 => ("pinsrq", "rcx"),
                        _ => unreachable!(),
                    };
                    self.sse_load_arg(&args[0], "xmm0");
                    self.operand_to_reg(&args[1], "rcx");
                    let imm = self.operand_to_imm_i64(&args[2]);
                    self.state
                        .emit_fmt(format_args!("    {} ${}, %{}, %xmm0", inst, imm, reg));
                    self.sse_store_dest(dptr, "xmm0");
                }
            }
            IntrinsicOp::Pextrw128 | IntrinsicOp::Pextrd128
            | IntrinsicOp::Pextrb128 | IntrinsicOp::Pextrq128 => {
                // Extract scalar at lane: pextrX $imm, %xmm0, %eax/%rax
                let (inst, reg) = match op {
                    IntrinsicOp::Pextrw128 => ("pextrw", "eax"),
                    IntrinsicOp::Pextrd128 => ("pextrd", "eax"),
                    IntrinsicOp::Pextrb128 => ("pextrb", "eax"),
                    IntrinsicOp::Pextrq128 => ("pextrq", "rax"),
                    _ => unreachable!(),
                };
                self.sse_load_arg(&args[0], "xmm0");
                let imm = self.operand_to_imm_i64(&args[1]);
                self.state
                    .emit_fmt(format_args!("    {} ${}, %xmm0, %{}", inst, imm, reg));
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
            }
            IntrinsicOp::Storeldi128 => {
                // Store low 64 bits to memory (MOVQ)
                if let Some(ptr) = dest_ptr {
                    if let Some(mem) = self.vec_arg_mem(&args[0]) {
                        self.state.emit_fmt(format_args!("    movdqu {}, %xmm0", mem));
                    } else {
                        self.sse_load_arg(&args[0], "xmm0");
                    }
                    if let Some(mem) = self.value_ptr_mem_operand(ptr.0) {
                        self.state.emit_fmt(format_args!("    movq %xmm0, {}", mem));
                    } else {
                        self.value_to_reg(ptr, "rax");
                        self.state.emit("    movq %xmm0, (%rax)");
                    }
                    self.state.sse_last_store_reg = false;
                }
            }
            IntrinsicOp::Cvtsi128Si32 => {
                // Extract low 32-bit integer (MOVD)
                self.sse_load_arg(&args[0], "xmm0");
                self.state.emit("    movd %xmm0, %eax");
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
            }
            IntrinsicOp::Cvtsi32Si128 => {
                // Convert int to __m128i (MOVD, zero-extends upper bits)
                if let Some(dptr) = dest_ptr {
                    self.operand_to_reg(&args[0], "rax");
                    self.state.emit("    movd %eax, %xmm0");
                    self.sse_store_dest(dptr, "xmm0");
                }
            }
            IntrinsicOp::Cvtsi128Si64 => {
                // Extract low 64-bit integer (MOVQ)
                self.sse_load_arg(&args[0], "xmm0");
                self.state.emit("    movq %xmm0, %rax");
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
            }
            IntrinsicOp::Pshuflw128 | IntrinsicOp::Pshufhw128 => {
                if let Some(dptr) = dest_ptr {
                    let inst = match op {
                        IntrinsicOp::Pshuflw128 => "pshuflw",
                        IntrinsicOp::Pshufhw128 => "pshufhw",
                        _ => unreachable!("unexpected SSE shuffle op: {:?}", op),
                    };
                    self.emit_sse_shuffle_imm_128(dptr, args, inst);
                }
            }
            IntrinsicOp::FmaF64x2 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                // dest_ptr[0..2] += broadcast(args[0]) * args[1][0..2]
                // args[0] = A pointer (scalar F64, broadcast to both lanes)
                // args[1] = B pointer (2×F64)
                // dest_ptr = C pointer (read+write, 2×F64)
                if let Some(c_ptr) = dest_ptr {
                    // Use the register allocator's assignments when available
                    // (mirrors FmaF64x4). The old operand_to_reg/value_to_reg
                    // path consulted the transient value cache, which conflated
                    // B and C GEPs sharing an offset but not a base, so C was
                    // loaded and stored through B's address (SSE2 matmul
                    // miscompile: every FMA wrote the B array).
                    let a_name = if let Some(r) = self.operand_reg(&args[0]) {
                        super::emit::phys_reg_name(r)
                    } else {
                        self.operand_to_reg(&args[0], "rcx");
                        "rcx"
                    };
                    let b_name = if let Some(r) = self.operand_reg(&args[1]) {
                        super::emit::phys_reg_name(r)
                    } else {
                        self.operand_to_reg(&args[1], "rdx");
                        "rdx"
                    };
                    let c_name = if let Some(r) = self.dest_reg(c_ptr) {
                        super::emit::phys_reg_name(r)
                    } else {
                        self.value_to_reg(c_ptr, "rax");
                        "rax"
                    };

                    // FMA3: load C, fused multiply-add with B, store back
                    self.state.emit_fmt(format_args!("    movsd (%{}), %xmm1", a_name));  // xmm1 = A scalar
                    self.state.emit("    unpcklpd %xmm1, %xmm1");                         // xmm1 = {A, A}
                    self.state.emit_fmt(format_args!("    movupd (%{}), %xmm0", c_name)); // xmm0 = {C[j], C[j+1]}
                    self.state.emit_fmt(format_args!("    vfmadd231pd (%{}), %xmm1, %xmm0", b_name));
                    self.state.emit_fmt(format_args!("    movupd %xmm0, (%{})", c_name)); // store back

                    self.state.reg_cache.invalidate_all();
                }
            }
            IntrinsicOp::FmaF64x4 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                // dest_ptr[0..4] += broadcast(args[0]) * args[1][0..4]
                // args[0] = A pointer (scalar F64, broadcast to all 4 lanes)
                // args[1] = B pointer (4×F64)
                // dest_ptr = C pointer (read+write, 4×F64)
                if let Some(c_ptr) = dest_ptr {
                    // Try SIB addressing: if B and C pointers come from GEPs
                    // with the same offset value, use (%base, %offset) directly.
                    let b_val = match &args[1] { Operand::Value(v) => Some(v.0), _ => None };
                    let c_val = c_ptr.0;

                    let b_gep = b_val.and_then(|bv| self.find_gep_base_offset(bv));
                    let c_gep = self.find_gep_base_offset(c_val);

                    // SIB disabled: GEP leaq optimization handles the address
                    // computation more reliably (no value ID matching needed).
                    let use_sib = false;
                    let _ = (&b_gep, &c_gep);

                    if use_sib {
                        let (c_base, offset) = c_gep.unwrap();
                        let (b_base, _) = b_gep.unwrap();

                        // Load order matters: load byte offset and B base first
                        // (these don't use rax as intermediate), then load C base
                        // and A ptr last (these may clobber rax).
                        // Final register assignment:
                        //   rsi = byte offset, rdx = B base, rax = C base, rcx = A ptr
                        self.value_to_reg(&Value(offset), "rsi");
                        self.value_to_reg(&Value(b_base), "rdx");
                        self.value_to_reg(&Value(c_base), "rax");
                        self.operand_to_reg(&args[0], "rcx");

                        self.state.emit("    movsd (%rcx), %xmm1");
                        self.state.emit("    vbroadcastsd %xmm1, %ymm1");
                        self.state.emit("    vmovupd (%rax,%rsi), %ymm0");
                        self.state.emit("    vfmadd231pd (%rdx,%rsi), %ymm1, %ymm0");
                        self.state.emit("    vmovupd %ymm0, (%rax,%rsi)");
                    } else {
                        // Use register-allocated pointers directly when available.
                        // This avoids unnecessary movq copies (e.g., movq %rbx, %rcx)
                        // when the pointer is already in a callee-saved register.
                        let a_name = if let Some(r) = self.operand_reg(&args[0]) {
                            super::emit::phys_reg_name(r)
                        } else {
                            self.operand_to_reg(&args[0], "rcx");
                            "rcx"
                        };
                        let b_name = if let Some(r) = self.operand_reg(&args[1]) {
                            super::emit::phys_reg_name(r)
                        } else {
                            self.operand_to_reg(&args[1], "rdx");
                            "rdx"
                        };
                        let c_name = if let Some(r) = self.dest_reg(c_ptr) {
                            super::emit::phys_reg_name(r)
                        } else {
                            self.value_to_reg(c_ptr, "rax");
                            "rax"
                        };

                        self.state.emit_fmt(format_args!("    movsd (%{}), %xmm1", a_name));
                        self.state.emit("    vbroadcastsd %xmm1, %ymm1");
                        self.state.emit_fmt(format_args!("    vmovupd (%{}), %ymm0", c_name));
                        self.state.emit_fmt(format_args!("    vfmadd231pd (%{}), %ymm1, %ymm0", b_name));
                        self.state.emit_fmt(format_args!("    vmovupd %ymm0, (%{})", c_name));
                    }

                    self.state.reg_cache.invalidate_all();
                }
            }
            IntrinsicOp::FmaF64x4Hoisted => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                // Like FmaF64x4, but A[i][k] broadcast already in ymm1.
                // args[0] = B pointer (4×F64)
                // dest_ptr = C pointer (read+write, 4×F64)
                if let Some(c_ptr) = dest_ptr {
                    self.operand_to_reg(&args[0], "rdx");      // B ptr → %rdx
                    self.value_to_reg(c_ptr, "rax");           // C ptr → %rax

                    self.state.emit("    vmovupd (%rax), %ymm0");            // Load C[j..j+3]
                    self.state.emit("    vfmadd231pd (%rdx), %ymm1, %ymm0"); // ymm0 = ymm1*B + ymm0
                    self.state.emit("    vmovupd %ymm0, (%rax)");            // Store C[j..j+3]
                }
            }
            IntrinsicOp::BroadcastLoadF64 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                // Load scalar F64 from pointer and broadcast to ymm1.
                // Placed before the vectorized j-loop.
                self.operand_to_reg(&args[0], "rcx");
                self.state.emit("    movsd (%rcx), %xmm1");
                self.state.emit("    vbroadcastsd %xmm1, %ymm1");
            }
            IntrinsicOp::FmaF64x4SIB => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                // FMA with SIB addressing: C[base+off] += broadcast(A) * B[base+off]
                // args[0] = A pointer (scalar F64)
                // args[1] = C base pointer
                // args[2] = B base pointer
                // args[3] = byte offset (j-loop IV)
                //
                // Uses SIB addressing: (%base, %offset) instead of computing
                // the full address. This eliminates ~5 address computation
                // instructions from the inner loop.
                self.operand_to_reg(&args[0], "rcx");          // A ptr → %rcx
                self.operand_to_reg(&args[1], "rax");          // C base → %rax
                self.operand_to_reg(&args[2], "rdx");          // B base → %rdx
                self.operand_to_reg(&args[3], "rsi");          // byte offset → %rsi

                // Load A, broadcast
                self.state.emit("    movsd (%rcx), %xmm1");
                self.state.emit("    vbroadcastsd %xmm1, %ymm1");

                // FMA with SIB addressing
                self.state.emit("    vmovupd (%rax,%rsi), %ymm0");            // Load C[j..j+3]
                self.state.emit("    vfmadd231pd (%rdx,%rsi), %ymm1, %ymm0"); // ymm0 = A*B + C
                self.state.emit("    vmovupd %ymm0, (%rax,%rsi)");            // Store C[j..j+3]

                self.state.reg_cache.invalidate_all();
            }

            // --- Vector loads for reduction patterns ---
            IntrinsicOp::LoadF64x4 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                // Load 4 packed doubles: vmovupd (%base + %offset), %ymm0
                if let Some(dptr) = dest_ptr {
                    self.value_to_reg(dptr, "rdx");          // Load dest FIRST into %rdx
                    self.operand_to_reg(&args[0], "rax");    // base pointer
                    self.operand_to_reg(&args[1], "rcx");    // byte offset
                    self.state.emit("    vmovupd (%rax,%rcx), %ymm0");
                    self.state.emit("    vmovupd %ymm0, (%rdx)");  // Store to %rdx
                }
            }
            IntrinsicOp::LoadF64x2 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                // Load 2 packed doubles: movupd (%base + %offset), %xmm0
                if let Some(dptr) = dest_ptr {
                    self.value_to_reg(dptr, "rdx");          // Load dest FIRST into %rdx
                    self.operand_to_reg(&args[0], "rax");
                    self.operand_to_reg(&args[1], "rcx");
                    self.state.emit("    movupd (%rax,%rcx), %xmm0");
                    self.state.emit("    movupd %xmm0, (%rdx)");  // Store to %rdx
                }
            }
            IntrinsicOp::LoadI32x8 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                // Load 8 packed ints: vmovdqu (%base + %offset), %ymm0
                if let Some(dptr) = dest_ptr {
                    self.value_to_reg(dptr, "rdx");          // Load dest FIRST into %rdx
                    self.operand_to_reg(&args[0], "rax");
                    self.operand_to_reg(&args[1], "rcx");
                    self.state.emit("    vmovdqu (%rax,%rcx), %ymm0");
                    self.state.emit("    vmovdqu %ymm0, (%rdx)");  // Store to %rdx
                }
            }
            IntrinsicOp::LoadI32x4 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                // Load 4 packed ints: movdqu (%base + %offset), %xmm0
                if let Some(dptr) = dest_ptr {
                    self.value_to_reg(dptr, "rdx");          // Load dest FIRST into %rdx
                    self.operand_to_reg(&args[0], "rax");
                    self.operand_to_reg(&args[1], "rcx");
                    self.state.emit("    movdqu (%rax,%rcx), %xmm0");
                    self.state.emit("    movdqu %xmm0, (%rdx)");  // Store to %rdx
                }
            }

            // --- Vector arithmetic ---
            IntrinsicOp::AddF64x4 => {
                if let Some(dptr) = dest_ptr {
                    self.emit_avx_binary_256(dptr, args, "vaddpd", true);
                }
            }
            IntrinsicOp::AddF64x2 => {
                if let Some(dptr) = dest_ptr {
                    self.emit_sse_binary_128(dptr, args, "addpd");
                }
            }
            IntrinsicOp::MulF64x4 => {
                if let Some(dptr) = dest_ptr {
                    self.emit_avx_binary_256(dptr, args, "vmulpd", true);
                }
            }
            IntrinsicOp::MulF64x2 => {
                if let Some(dptr) = dest_ptr {
                    self.emit_sse_binary_128(dptr, args, "mulpd");
                }
            }
            IntrinsicOp::AddI32x8 => {
                if let Some(dptr) = dest_ptr {
                    self.emit_avx_binary_256(dptr, args, "vpaddd", true);
                }
            }
            IntrinsicOp::AddI32x4 => {
                if let Some(dptr) = dest_ptr {
                    self.emit_sse_binary_128(dptr, args, "paddd");
                }
            }

            // --- Horizontal reduction ---
            IntrinsicOp::HorizontalAddF64x4 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                // Reduce 4×F64 → 1×F64
                self.operand_to_reg(&args[0], "rax");
                self.state.emit("    vmovupd (%rax), %ymm0");        // Load 4 doubles
                self.state.emit("    vextractf128 $1, %ymm0, %xmm1"); // Extract upper 128 bits
                self.state.emit("    vaddpd %xmm1, %xmm0, %xmm0");    // Add upper + lower (4→2)
                self.state.emit("    vunpckhpd %xmm0, %xmm0, %xmm1"); // Shuffle element 1 to position 0
                self.state.emit("    vaddsd %xmm1, %xmm0, %xmm0");    // Final scalar add (2→1)
                self.state.emit("    vmovq %xmm0, %rax");             // Extract to GPR
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
            }
            IntrinsicOp::HorizontalAddF64x2 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                // Reduce 2×F64 → 1×F64
                self.operand_to_reg(&args[0], "rax");
                self.state.emit("    movupd (%rax), %xmm0");   // Load {lo, hi}
                self.state.emit("    movapd %xmm0, %xmm1");   // copy
                self.state.emit("    unpckhpd %xmm0, %xmm1"); // xmm1 = {hi, hi}
                self.state.emit("    addsd %xmm1, %xmm0");    // xmm0.lo = lo + hi
                self.state.emit("    movq %xmm0, %rax");      // Extract to GPR
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
            }
            IntrinsicOp::HorizontalAddI32x8 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                // Reduce 8×I32 → 1×I32
                self.operand_to_reg(&args[0], "rax");
                self.state.emit("    vmovdqu (%rax), %ymm0");         // Load 8 ints
                self.state.emit("    vextracti128 $1, %ymm0, %xmm1"); // Extract upper 128 (8→4)
                self.state.emit("    vpaddd %xmm1, %xmm0, %xmm0");    // Add halves (8→4)
                self.state.emit("    vpsrldq $8, %xmm0, %xmm1");      // Shift 8 bytes (4→2)
                self.state.emit("    vpaddd %xmm1, %xmm0, %xmm0");    // Add (4→2)
                self.state.emit("    vpsrldq $4, %xmm0, %xmm1");      // Shift 4 bytes (2→1)
                self.state.emit("    vpaddd %xmm1, %xmm0, %xmm0");    // Add (2→1)
                self.state.emit("    vmovd %xmm0, %eax");             // Extract to GPR
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
            }
            IntrinsicOp::HorizontalAddI32x4 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                // Reduce 4×I32 → 1×I32
                self.operand_to_reg(&args[0], "rax");
                self.state.emit("    movdqu (%rax), %xmm0");          // Load 4 ints
                self.state.emit("    movdqa %xmm0, %xmm1");           // copy
                self.state.emit("    psrldq $8, %xmm1");              // xmm1 = {0,0,a,b}
                self.state.emit("    paddd %xmm1, %xmm0");            // Add (4→2)
                self.state.emit("    movdqa %xmm0, %xmm1");           // copy
                self.state.emit("    psrldq $4, %xmm1");              // xmm1 = {0,a,b,a+c}
                self.state.emit("    paddd %xmm1, %xmm0");            // Add (2→1)
                self.state.emit("    movd %xmm0, %eax");              // Extract to GPR
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
            }

            // --- Register-based vector operations (SSA-friendly) ---

            IntrinsicOp::VecLoadF64x4 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                // %dest_vec = load_vector(base_ptr, offset) - AVX2 4×F64
                // Load from memory array into ymm0, then store directly to stack slot
                self.operand_to_reg(&args[0], "rax");  // base pointer
                self.operand_to_reg(&args[1], "rcx");  // offset
                self.state.emit("    vmovupd (%rax,%rcx), %ymm0");
                if let Some(d) = dest {
                    self.state.vector_values.insert(d.0);
                    if let Some(slot) = self.state.get_slot(d.0) {
                        // Store vector directly to stack slot (not via pointer indirection)
                        self.state.out.emit_instr_reg_rbp("    vmovupd", "ymm0", slot.0 as i64);
                    }
                }
            }
            IntrinsicOp::VecLoadF64x2 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                // %dest_vec = load_vector(base_ptr, offset) - SSE2 2×F64
                self.operand_to_reg(&args[0], "rax");
                self.operand_to_reg(&args[1], "rcx");
                self.state.emit("    movupd (%rax,%rcx), %xmm0");
                if let Some(d) = dest {
                    self.state.vector_values.insert(d.0);
                    if let Some(slot) = self.state.get_slot(d.0) {
                        // Store vector directly to stack slot
                        self.state.out.emit_instr_reg_rbp("    movupd", "xmm0", slot.0 as i64);
                    }
                }
            }
            IntrinsicOp::VecLoadI32x8 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                // %dest_vec = load_vector(base_ptr, offset) - AVX2 8×I32
                self.operand_to_reg(&args[0], "rax");
                self.operand_to_reg(&args[1], "rcx");
                self.state.emit("    vmovdqu (%rax,%rcx), %ymm0");
                if let Some(d) = dest {
                    self.state.vector_values.insert(d.0);
                    if let Some(slot) = self.state.get_slot(d.0) {
                        self.state.out.emit_instr_rbp_reg("    leaq", slot.0 as i64, "rdx");
                        self.state.emit("    vmovdqu %ymm0, (%rdx)");
                    }
                }
            }
            IntrinsicOp::VecLoadI32x4 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                // %dest_vec = load_vector(base_ptr, offset) - SSE2 4×I32
                self.operand_to_reg(&args[0], "rax");
                self.operand_to_reg(&args[1], "rcx");
                self.state.emit("    movdqu (%rax,%rcx), %xmm0");
                if let Some(d) = dest {
                    if let Some(slot) = self.state.get_slot(d.0) {
                        self.state.out.emit_instr_rbp_reg("    leaq", slot.0 as i64, "rdx");
                        self.state.emit("    movdqu %xmm0, (%rdx)");
                    }
                }
            }

            IntrinsicOp::VecAddF64x4 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                // %dest_vec = %src1_vec + %src2_vec - AVX2 4×F64
                // Load both source vectors directly from their stack slots and add.
                // Vector values are stored directly in stack slots (not as pointers),
                // so we load them with offset(%rbp) addressing, not pointer indirection.
                if let Some(slot) = self.get_slot_for_operand(&args[0]) {
                    // Vector operand: load directly from stack slot
                    self.state.out.emit_instr_rbp_reg("    vmovupd", slot.0 as i64, "ymm0");
                } else {
                    // Non-vector operand (shouldn't happen for VecAdd, but handle gracefully)
                    self.operand_to_reg(&args[0], "rax");
                    self.state.emit("    vmovupd (%rax), %ymm0");
                }
                if let Some(slot) = self.get_slot_for_operand(&args[1]) {
                    // Vector operand: load directly from stack slot
                    self.state.out.emit_instr_rbp_reg("    vmovupd", slot.0 as i64, "ymm1");
                } else {
                    // Non-vector operand (shouldn't happen for VecAdd, but handle gracefully)
                    self.operand_to_reg(&args[1], "rcx");
                    self.state.emit("    vmovupd (%rcx), %ymm1");
                }
                self.state.emit("    vaddpd %ymm1, %ymm0, %ymm0");
                if let Some(d) = dest {
                    self.state.vector_values.insert(d.0);
                    if let Some(slot) = self.state.get_slot(d.0) {
                        // Store result directly to stack slot
                        self.state.out.emit_instr_reg_rbp("    vmovupd", "ymm0", slot.0 as i64);
                    }
                }
            }
            IntrinsicOp::VecAddF64x2 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                // %dest_vec = %src1_vec + %src2_vec - SSE2 2×F64
                // Load both source vectors directly from stack slots
                if let Some(slot) = self.get_slot_for_operand(&args[0]) {
                    self.state.out.emit_instr_rbp_reg("    movupd", slot.0 as i64, "xmm0");
                } else {
                    self.operand_to_reg(&args[0], "rax");
                    self.state.emit("    movupd (%rax), %xmm0");
                }
                if let Some(slot) = self.get_slot_for_operand(&args[1]) {
                    self.state.out.emit_instr_rbp_reg("    movupd", slot.0 as i64, "xmm1");
                } else {
                    self.operand_to_reg(&args[1], "rcx");
                    self.state.emit("    movupd (%rcx), %xmm1");
                }
                self.state.emit("    addpd %xmm1, %xmm0");
                if let Some(d) = dest {
                    self.state.vector_values.insert(d.0);
                    if let Some(slot) = self.state.get_slot(d.0) {
                        // Store result directly to stack slot
                        self.state.out.emit_instr_reg_rbp("    movupd", "xmm0", slot.0 as i64);
                    }
                }
            }
            IntrinsicOp::VecMulF64x4 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                // %dest_vec = %src1_vec * %src2_vec - AVX2 4×F64
                // Load both source vectors directly from stack slots
                if let Some(slot) = self.get_slot_for_operand(&args[0]) {
                    self.state.out.emit_instr_rbp_reg("    vmovupd", slot.0 as i64, "ymm0");
                } else {
                    self.operand_to_reg(&args[0], "rax");
                    self.state.emit("    vmovupd (%rax), %ymm0");
                }
                if let Some(slot) = self.get_slot_for_operand(&args[1]) {
                    self.state.out.emit_instr_rbp_reg("    vmovupd", slot.0 as i64, "ymm1");
                } else {
                    self.operand_to_reg(&args[1], "rcx");
                    self.state.emit("    vmovupd (%rcx), %ymm1");
                }
                self.state.emit("    vmulpd %ymm1, %ymm0, %ymm0");
                if let Some(d) = dest {
                    self.state.vector_values.insert(d.0);
                    if let Some(slot) = self.state.get_slot(d.0) {
                        // Store result directly to stack slot
                        self.state.out.emit_instr_reg_rbp("    vmovupd", "ymm0", slot.0 as i64);
                    }
                }
            }
            IntrinsicOp::VecMulF64x2 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                // %dest_vec = %src1_vec * %src2_vec - SSE2 2×F64
                // Load both source vectors directly from stack slots
                if let Some(slot) = self.get_slot_for_operand(&args[0]) {
                    self.state.out.emit_instr_rbp_reg("    movupd", slot.0 as i64, "xmm0");
                } else {
                    self.operand_to_reg(&args[0], "rax");
                    self.state.emit("    movupd (%rax), %xmm0");
                }
                if let Some(slot) = self.get_slot_for_operand(&args[1]) {
                    self.state.out.emit_instr_rbp_reg("    movupd", slot.0 as i64, "xmm1");
                } else {
                    self.operand_to_reg(&args[1], "rcx");
                    self.state.emit("    movupd (%rcx), %xmm1");
                }
                self.state.emit("    mulpd %xmm1, %xmm0");
                if let Some(d) = dest {
                    self.state.vector_values.insert(d.0);
                    if let Some(slot) = self.state.get_slot(d.0) {
                        // Store result directly to stack slot
                        self.state.out.emit_instr_reg_rbp("    movupd", "xmm0", slot.0 as i64);
                    }
                }
            }
            IntrinsicOp::VecAddI32x8 | IntrinsicOp::VecAddI32x4 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                // Vector integer add. paddd is a legacy 2-operand instruction;
                // only its VEX form (vpaddd) takes three operands. The old
                // code emitted `paddd %xmm1, %xmm0, %xmm0` for the SSE2 case,
                // which the assembler rejects ("SSE op requires 2 operands").
                let (load_inst, add_inst, store_inst, reg, is_avx) = match op {
                    IntrinsicOp::VecAddI32x8 => ("vmovdqu", "vpaddd", "vmovdqu", "ymm", true),
                    IntrinsicOp::VecAddI32x4 => ("movdqu", "paddd", "movdqu", "xmm", false),
                    _ => unreachable!(),
                };
                self.operand_to_reg(&args[0], "rax");
                if let Some(slot) = self.get_slot_for_operand(&args[0]) {
                    self.state.out.emit_instr_rbp_reg("    leaq", slot.0 as i64, "rax");
                }
                self.state.emit_fmt(format_args!("    {} (%rax), %{}0", load_inst, reg));
                self.operand_to_reg(&args[1], "rcx");
                if let Some(slot) = self.get_slot_for_operand(&args[1]) {
                    self.state.out.emit_instr_rbp_reg("    leaq", slot.0 as i64, "rcx");
                }
                self.state.emit_fmt(format_args!("    {} (%rcx), %{}1", load_inst, reg));
                if is_avx {
                    self.state.emit_fmt(format_args!("    {} %{}1, %{}0, %{}0", add_inst, reg, reg, reg));
                } else {
                    // Legacy SSE2: 2-operand form, src then dst.
                    self.state.emit_fmt(format_args!("    {} %{}1, %{}0", add_inst, reg, reg));
                }
                if let Some(d) = dest {
                    self.state.vector_values.insert(d.0);
                    if let Some(slot) = self.state.get_slot(d.0) {
                        self.state.out.emit_instr_rbp_reg("    leaq", slot.0 as i64, "rdx");
                        self.state.emit_fmt(format_args!("    {} %{}0, (%rdx)", store_inst, reg));
                    }
                }
            }

            IntrinsicOp::VecHorizontalAddF64x4 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                // %scalar = horizontal_add(%vec) - AVX2 4×F64 → F64
                // Load vector from operand and reduce
                if let Some(slot) = self.get_slot_for_operand(&args[0]) {
                    // Direct load from slot
                    self.state.out.emit_instr_rbp_reg("    vmovupd", slot.0 as i64, "ymm0");
                } else {
                    // Fallback: load pointer then dereference
                    self.operand_to_reg(&args[0], "rax");
                    self.state.emit("    vmovupd (%rax), %ymm0");
                }
                self.state.emit("    vextractf128 $1, %ymm0, %xmm1");
                self.state.emit("    vaddpd %xmm1, %xmm0, %xmm0");
                self.state.emit("    vunpckhpd %xmm0, %xmm0, %xmm1");
                self.state.emit("    vaddsd %xmm1, %xmm0, %xmm0");
                self.state.emit("    vmovq %xmm0, %rax");
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
            }
            IntrinsicOp::VecHorizontalAddF64x2 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                // %scalar = horizontal_add(%vec) - SSE2 2×F64 → F64
                if let Some(slot) = self.get_slot_for_operand(&args[0]) {
                    // Direct load from slot
                    self.state.out.emit_instr_rbp_reg("    movupd", slot.0 as i64, "xmm0");
                } else {
                    // Fallback: load pointer then dereference
                    self.operand_to_reg(&args[0], "rax");
                    self.state.emit("    movupd (%rax), %xmm0");
                }
                self.state.emit("    movapd %xmm0, %xmm1");
                self.state.emit("    unpckhpd %xmm0, %xmm1"); // xmm1 = {hi, hi}
                self.state.emit("    addsd %xmm1, %xmm0");    // xmm0.lo = lo + hi
                self.state.emit("    movq %xmm0, %rax");
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
            }
            IntrinsicOp::VecHorizontalAddI32x8 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                // %scalar = horizontal_add(%vec) - AVX2 8×I32 → I32
                if let Some(slot) = self.get_slot_for_operand(&args[0]) {
                    // Direct load from slot
                    self.state.out.emit_instr_rbp_reg("    vmovdqu", slot.0 as i64, "ymm0");
                } else {
                    // Fallback: load pointer then dereference
                    self.operand_to_reg(&args[0], "rax");
                    self.state.emit("    vmovdqu (%rax), %ymm0");
                }
                self.state.emit("    vextracti128 $1, %ymm0, %xmm1");
                self.state.emit("    vpaddd %xmm1, %xmm0, %xmm0");
                self.state.emit("    vpsrldq $8, %xmm0, %xmm1");
                self.state.emit("    vpaddd %xmm1, %xmm0, %xmm0");
                self.state.emit("    vpsrldq $4, %xmm0, %xmm1");
                self.state.emit("    vpaddd %xmm1, %xmm0, %xmm0");
                self.state.emit("    vmovd %xmm0, %eax");
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
            }
            IntrinsicOp::VecHorizontalAddI32x4 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                // %scalar = horizontal_add(%vec) - SSE2 4×I32 → I32
                if let Some(slot) = self.get_slot_for_operand(&args[0]) {
                    // Direct load from slot
                    self.state.out.emit_instr_rbp_reg("    movdqu", slot.0 as i64, "xmm0");
                } else {
                    // Fallback: load pointer then dereference
                    self.operand_to_reg(&args[0], "rax");
                    self.state.emit("    movdqu (%rax), %xmm0");
                }
                self.state.emit("    movdqa %xmm0, %xmm1");
                self.state.emit("    psrldq $8, %xmm1");   // xmm1 = {0,0,a,b}
                self.state.emit("    paddd %xmm1, %xmm0");
                self.state.emit("    movdqa %xmm0, %xmm1");
                self.state.emit("    psrldq $4, %xmm1");   // xmm1 = {0,a,b,a+c}
                self.state.emit("    paddd %xmm1, %xmm0");
                self.state.emit("    movd %xmm0, %eax");
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
            }

            IntrinsicOp::VecZeroF64x4 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                // %dest_vec = {0.0, 0.0, 0.0, 0.0} - AVX2 4×F64
                self.state.emit("    vxorpd %ymm0, %ymm0, %ymm0");
                if let Some(d) = dest {
                    self.state.vector_values.insert(d.0);
                    if let Some(slot) = self.state.get_slot(d.0) {
                        // Store zero vector directly to stack slot
                        self.state.out.emit_instr_reg_rbp("    vmovupd", "ymm0", slot.0 as i64);
                    }
                }
            }
            IntrinsicOp::VecZeroF64x2 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                // %dest_vec = {0.0, 0.0} - SSE2 2×F64
                self.state.emit("    xorpd %xmm0, %xmm0");
                if let Some(d) = dest {
                    self.state.vector_values.insert(d.0);
                    if let Some(slot) = self.state.get_slot(d.0) {
                        // Store zero vector directly to stack slot
                        self.state.out.emit_instr_reg_rbp("    movupd", "xmm0", slot.0 as i64);
                    }
                }
            }
            IntrinsicOp::VecZeroI32x8 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                // %dest_vec = {0, 0, 0, 0, 0, 0, 0, 0} - AVX2 8×I32
                self.state.emit("    vpxor %ymm0, %ymm0, %ymm0");
                if let Some(d) = dest {
                    self.state.vector_values.insert(d.0);
                    if let Some(slot) = self.state.get_slot(d.0) {
                        self.state.out.emit_instr_rbp_reg("    leaq", slot.0 as i64, "rdx");
                        self.state.emit("    vmovdqu %ymm0, (%rdx)");
                    }
                }
            }
            IntrinsicOp::VecZeroI32x4 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                // %dest_vec = {0, 0, 0, 0} - SSE2 4×I32
                self.state.emit("    pxor %xmm0, %xmm0");
                if let Some(d) = dest {
                    self.state.vector_values.insert(d.0);
                    if let Some(slot) = self.state.get_slot(d.0) {
                        self.state.out.emit_instr_rbp_reg("    leaq", slot.0 as i64, "rdx");
                        self.state.emit("    movdqu %xmm0, (%rdx)");
                    }
                }
            }
            // Generic SIMD family (512-bit + FP): emitted by intrinsics_simd.rs.
            _ => {
                if !self.emit_simd_op(dest, op, dest_ptr, args) {
                    eprintln!(
                        "ccc: internal: unhandled intrinsic op {:?} (dest_ptr={}, args={})",
                        op,
                        dest_ptr.is_some(),
                        args.len()
                    );
                }
            }
        }
    }

    /// Helper: Get stack slot for an operand if it's a Value
    fn get_slot_for_operand(&self, op: &Operand) -> Option<StackSlot> {
        match op {
            Operand::Value(v) => self.state.get_slot(v.0),
            _ => None,
        }
    }

    /// Emit AVX binary 256-bit op: load ymm0 from arg0 ptr, ymm1 from arg1 ptr,
    /// apply the given AVX instruction, store result ymm0 to dest_ptr.
    pub(super) fn emit_avx_binary_256(&mut self, dest_ptr: &Value, args: &[Operand], avx_inst: &str, commutative: bool) {
        // In AT&T VEX syntax only the first textual source may be memory.
        // Preserve operand order for non-commutative operations.
        //
        // A value still provably in a register (last-stored/deferred) must NOT
        // be folded as a memory operand — its slot contents may be stale under
        // the v5 deferred-store optimization. The loaders route those through
        // the register cache instead. This also removes the old slot-only
        // reuse check, which could fire across coalesced slots holding
        // DIFFERENT values (latent miscompile class).
        assert!(
            args.len() >= 2,
            "emit_avx_binary_256: malformed intrinsic {} ({} args)",
            avx_inst,
            args.len()
        );
        let mem_of = |this: &Self, arg: &Operand| -> Option<String> {
            match arg {
                Operand::Value(v) => {
                    if this.state.vec_live_regs.contains_key(&v.0)
                        || (this.state.vec_last_store_reg
                            && this.state.vec_last_store_val == Some(v.0))
                    {
                        return None;
                    }
                    this.value_ptr_mem_operand(v.0)
                }
                _ => None,
            }
        };
        // If args[1] is still provably in %ymm0 (last-stored / deferred), load
        // it FIRST into %ymm1 so args[0]'s load into %ymm0 cannot clobber it
        // (sound deferred-store for `r = op(x, fresh_result)` in the AVX path).
        if matches!(&args[1], Operand::Value(v)
            if self.state.vec_last_store_reg && self.state.vec_last_store_val == Some(v.0))
        {
            self.avx_load_arg_to(&args[1], "ymm1");
            self.avx_load_arg(&args[0]);
            self.state
                .emit_fmt(format_args!("    {} %ymm1, %ymm0, %ymm0", avx_inst));
            self.state.vec_last_store_reg = false;
            self.avx_store_dest(dest_ptr);
            return;
        }
        let m0 = mem_of(self, &args[0]);
        let m1 = mem_of(self, &args[1]);
        match (m0, m1) {
            (Some(m0), Some(m1)) => {
                self.avx_load_arg(&args[0]);
                self.state
                    .emit_fmt(format_args!("    {} {}, %ymm0, %ymm0", avx_inst, m1));
                self.state.vec_last_store_reg = false;
            }
            (Some(m0), None) if commutative => {
                self.avx_load_arg_to(&args[1], "ymm1");
                self.state
                    .emit_fmt(format_args!("    {} {}, %ymm1, %ymm0", avx_inst, m0));
                self.state.vec_last_store_reg = false;
            }
            (_, Some(m1)) => {
                self.avx_load_arg(&args[0]);
                self.state
                    .emit_fmt(format_args!("    {} {}, %ymm0, %ymm0", avx_inst, m1));
                self.state.vec_last_store_reg = false;
            }
            (_, None) => {
                self.avx_load_arg(&args[0]);
                self.avx_load_arg_to(&args[1], "ymm1");
                self.state
                    .emit_fmt(format_args!("    {} %ymm1, %ymm0, %ymm0", avx_inst));
                self.state.vec_last_store_reg = false;
            }
        }
        self.avx_store_dest(dest_ptr);
    }

    /// Look up GEP decomposition for a value: returns (base_id, offset_id) if the
    /// value was produced by a GEP(base, offset) instruction with a variable offset.
    fn find_gep_base_offset(&self, val_id: u32) -> Option<(u32, u32)> {
        self.state.gep_base_offset.get(&val_id).copied()
    }
}
