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

use super::emit::{is_xmm_reg, phys_reg_name, phys_reg_name_256, X86Codegen};
use crate::backend::regalloc::PhysReg;
use crate::backend::state::StackSlot;
use crate::common::types::IrType;
use crate::ir::reexports::{IntrinsicOp, IrConst, Operand, Value};

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
                    IrConst::F64(_) => {
                        self.emit_fp_operand_to_xmm(op, IrType::F64, "xmm0");
                    }
                    IrConst::F32(_) => {
                        self.emit_fp_operand_to_xmm(op, IrType::F32, "xmm0");
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

    fn emit_nontemporal_store(
        &mut self,
        op: &IntrinsicOp,
        dest_ptr: &Option<Value>,
        args: &[Operand],
    ) {
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
                        self.state.emit_fmt(format_args!(
                            "    {} %xmm0, (%{})",
                            inst,
                            phys_reg_name(reg)
                        ));
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
            // Register-allocated vector value (Phase 3b vecreg): its CONTENT
            // lives in the assigned XMM register across blocks. This check
            // must come first — the fallback path below would hand the value
            // to operand_to_reg, which consults the same assignment and emits
            // `movq %xmmN, %rax; movdqu (%rax), ...`, reinterpreting the
            // vector's low 64 bits as an ADDRESS (simd_movnt SIGSEGV: the
            // loop-carried accumulator held in xmm6 was dereferenced).
            // vec_live_regs only tracks within a block; reg_assignments is
            // the source of truth at block boundaries.
            if let Some(&reg) = self.reg_assignments.get(&v.0) {
                if is_xmm_reg(reg) {
                    let name = phys_reg_name(reg);
                    // A pending deferred store to this value flowed through
                    // the register; anything else must be flushed before we
                    // potentially clobber xmm0/xmm1 scratch.
                    if self.state.pending_vec_store.map(|(p, _, _)| p) == Some(v.0) {
                        self.state.pending_vec_store = None;
                    } else {
                        self.flush_pending_vec_store_impl();
                    }
                    if name != xmm {
                        self.state
                            .emit_fmt(format_args!("    movdqa %{}, %{}", name, xmm));
                    }
                    self.state.sse_last_store_reg = false;
                    return;
                }
            }
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
        self.state
            .emit_fmt(format_args!("    movdqu (%rax), %{}", xmm));
        self.state.sse_last_store_reg = false;
    }

    /// Store an XMM register to a vector operand's home slot.
    #[inline]
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
            eprintln!(
                "[VDEFER-EMIT] deferring result store for dest_ptr={}",
                dest_ptr.0
            );
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
            self.state
                .emit_fmt(format_args!("    movdqu %{}, (%rax)", xmm));
        } else {
            self.state.pending_vec_store = Some((dest_ptr.0, xmm, false));
        }
        self.state.sse_last_store_val = Some(dest_ptr.0);
        self.state.sse_last_store_reg = true;
        self.state.sse_last_store_reg_name = Some(xmm);
    }

    /// Lazy-flush a pending deferred vector-result store: the producer
    /// kept its result in the holding register instead of storing it. Called
    /// whenever anything is about to clobber that register, read the slot, or
    /// leave the block without the consumer having taken the value — emits the
    /// store that was originally skipped, making deferred stores sound by
    /// construction. No-op when nothing is pending.
    /// VLFOLD entry (see `compute_vector_memfold_values`): elide an eligible
    /// 256-bit load and remember its source memory operand for the adjacent
    /// consumer. Returns `true` when nothing must be emitted for this
    /// intrinsic. Falls back to the ordinary load path unless base/index are
    /// RA-homed GPRs (or a zero constant) — scratch `%rax`/`%rcx` addressing
    /// would not survive the intervening load — and unless the destination
    /// has no XMM home (a homed load is already a single instruction).
    fn try_elide_vec_load(
        &mut self,
        dest: &Option<Value>,
        op: &IntrinsicOp,
        args: &[Operand],
    ) -> bool {
        let Some(d) = dest else {
            return false;
        };
        if !self.state.vector_memfold_values.contains(&d.0)
            || self.state.pending_vec_memfold.is_some()
            || args.len() < 2
        {
            return false;
        }
        if self
            .reg_assignments
            .get(&d.0)
            .is_some_and(|r| is_xmm_reg(*r))
        {
            return false;
        }
        let mnemonic = match op {
            IntrinsicOp::VecLoadF64x4 => "vmovupd",
            IntrinsicOp::VecLoadF32x8 => "vmovups",
            IntrinsicOp::VecLoadI32x8 => "vmovdqu",
            _ => return false,
        };
        // The allocator only hands out rbx/r8-r15 (never rsp/rbp/rdi/rsi/rdx
        // and never the scratch pair), all legal base AND index registers.
        let is_gpr = |r: PhysReg| (1..=16).contains(&r.0);
        let Some(base) = self.operand_reg(&args[0]).filter(|r| is_gpr(*r)) else {
            return false;
        };
        let index = match &args[1] {
            Operand::Const(c) if c.to_i64() == Some(0) => None,
            Operand::Const(_) => return false,
            other => match self.operand_reg(other) {
                Some(r) if is_gpr(r) => Some(r),
                _ => return false,
            },
        };
        let disp = Self::vec_disp_arg(args, 2);
        let disp_str = if disp == 0 {
            String::new()
        } else {
            disp.to_string()
        };
        let mem = match index {
            Some(ix) => format!("{}(%{},%{})", disp_str, phys_reg_name(base), phys_reg_name(ix)),
            None => format!("{}(%{})", disp_str, phys_reg_name(base)),
        };
        if std::env::var("CCC_DEBUG_VLFOLD").is_ok() {
            eprintln!("[VLFOLD-EMIT] elide load %{} <- {} {}", d.0, mnemonic, mem);
        }
        self.state.vector_values.insert(d.0);
        self.state.pending_vec_memfold = Some((d.0, mem, mnemonic));
        true
    }

    /// Memory operand of a pending VLFOLD load if `arg` is that value.
    fn memfold_operand(&self, arg: &Operand) -> Option<String> {
        match (arg, &self.state.pending_vec_memfold) {
            (Operand::Value(v), Some((pv, mem, _))) if v.0 == *pv => Some(mem.clone()),
            _ => None,
        }
    }

    /// Materialise a pending VLFOLD load through `%ymm0` and its ordinary
    /// home (register or slot). Never expected on the analysed shapes; keeps
    /// the elision sound if an unexpected instruction intervenes.
    pub(super) fn materialize_pending_memfold(&mut self) {
        let Some((val, mem, mnemonic)) = self.state.pending_vec_memfold.take() else {
            return;
        };
        if std::env::var("CCC_DEBUG_VLFOLD").is_ok() {
            eprintln!("[VLFOLD-EMIT] materialising %{} (unexpected consumer)", val);
        }
        self.flush_pending_vec_store_impl();
        self.state
            .emit_fmt(format_args!("    {} {}, %ymm0", mnemonic, mem));
        self.state.dirty_upper_ymm = true;
        self.avx_store_dest(&Value(val));
        // The value now has a real home; a later deferral is not permitted
        // to skip the store again for this def.
        self.flush_pending_vec_store_impl();
    }

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

    /// Lazy-flush entry check for an intrinsic about to be emitted: if a
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

    #[inline]
    pub(super) fn emit_sse_binary_128(
        &mut self,
        dest_ptr: &Value,
        args: &[Operand],
        sse_inst: &str,
    ) {
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

        // A loop-carried accumulator and its backedge result are coalesced.
        // When args[1] is the freshly produced/deferred value in an XMM
        // register, update the assigned accumulator directly instead of
        // copying both inputs through xmm0/xmm1 and copying the result back.
        if let Some(&dest_reg) = self.reg_assignments.get(&dest_ptr.0) {
            if is_xmm_reg(dest_reg) {
                if let (Operand::Value(acc), Operand::Value(fresh)) = (&args[0], &args[1]) {
                    let acc_same = self
                        .reg_assignments
                        .get(&acc.0)
                        .is_some_and(|r| *r == dest_reg);
                    let fresh_held = self.state.sse_last_store_reg
                        && self.state.sse_last_store_val == Some(fresh.0);
                    if acc_same && fresh_held {
                        let held = self.state.sse_last_store_reg_name.unwrap_or("xmm0");
                        let target = phys_reg_name(dest_reg);
                        self.state
                            .emit_fmt(format_args!("    {} %{}, %{}", sse_inst, held, target));
                        if self.state.pending_vec_store.map(|(p, _, _)| p) == Some(fresh.0) {
                            self.state.pending_vec_store = None;
                        }
                        self.state.vec_live_regs.insert(dest_ptr.0, target);
                        self.state.sse_last_store_val = Some(dest_ptr.0);
                        self.state.sse_last_store_reg = true;
                        self.state.sse_last_store_reg_name = Some(target);
                        return;
                    }
                }
            }
        }

        // Map broadcast in an assigned XMM family plus a streamed current
        // value in xmm0: the legacy two-operand form can consume both directly.
        if let (Operand::Value(current), Operand::Value(invariant)) = (&args[0], &args[1]) {
            let current_held = self.state.sse_last_store_reg
                && self.state.sse_last_store_val == Some(current.0)
                && self.state.sse_last_store_reg_name == Some("xmm0");
            if current_held {
                if let Some(&reg) = self.reg_assignments.get(&invariant.0) {
                    if is_xmm_reg(reg) {
                        self.state.emit_fmt(format_args!(
                            "    {} %{}, %xmm0",
                            sse_inst,
                            phys_reg_name(reg)
                        ));
                        if self.state.pending_vec_store.map(|(p, _, _)| p) == Some(current.0) {
                            self.state.pending_vec_store = None;
                        }
                        self.state.sse_last_store_reg = false;
                        self.sse_store_dest(dest_ptr, "xmm0");
                        return;
                    }
                }
            }
        }

        let a1_last = matches!(&args[1], Operand::Value(v)
            if self.state.sse_last_store_reg && self.state.sse_last_store_val == Some(v.0));
        if a1_last {
            self.sse_load_arg(&args[1], "xmm1");
            self.sse_load_arg(&args[0], "xmm0");
        } else {
            self.sse_load_arg(&args[0], "xmm0");
            self.sse_load_arg(&args[1], "xmm1");
        }
        self.state
            .emit_fmt(format_args!("    {} %xmm1, %xmm0", sse_inst));
        self.sse_store_dest(dest_ptr, "xmm0");
    }

    /// Emit SSE unary 128-bit op with immediate: load xmm0 from arg0 ptr,
    /// apply `inst $imm, %xmm0`, store result xmm0 to dest_ptr.
    fn emit_sse_unary_imm_128(&mut self, dest_ptr: &Value, args: &[Operand], sse_inst: &str) {
        self.sse_load_arg(&args[0], "xmm0");
        let imm = self.operand_to_imm_i64(&args[1]);
        self.state
            .emit_fmt(format_args!("    {} ${}, %xmm0", sse_inst, imm));
        self.sse_store_dest(dest_ptr, "xmm0");
    }

    /// Emit SSE shuffle with immediate: load xmm0, apply `inst $imm, %xmm0, %xmm0`,
    /// store result. Used for pshufd/pshuflw/pshufhw which read and write same register.

    pub(super) fn avx_load_arg_to(&mut self, arg: &Operand, ymm: &'static str) {
        if let Operand::Value(v) = arg {
            // VLFOLD: a consumer path that needs the elided load in a
            // register performs the load itself (from the recorded source
            // operand, never from the never-written home slot).
            if self.memfold_operand(arg).is_some() {
                let (_, mem, mnemonic) = self.state.pending_vec_memfold.take().unwrap();
                if self.state.pending_vec_store.map(|(_, r, _)| r) == Some(ymm) {
                    self.flush_pending_vec_store_impl();
                }
                self.state
                    .emit_fmt(format_args!("    {} {}, %{}", mnemonic, mem, ymm));
                self.state.dirty_upper_ymm = true;
                self.state.vec_last_store_reg = false;
                return;
            }
            // Width-aware register allocation: PhysReg 20..33 names the SIMD
            // register family; this AVX helper selects its YMM view. Consult
            // the assignment at every block boundary because vec_live_regs is
            // intentionally block-local.
            if let Some(&reg) = self.reg_assignments.get(&v.0) {
                if is_xmm_reg(reg) {
                    let name = phys_reg_name_256(reg);
                    if self.state.pending_vec_store.map(|(p, _, _)| p) == Some(v.0) {
                        self.state.pending_vec_store = None;
                    } else {
                        self.flush_pending_vec_store_impl();
                    }
                    if name != ymm {
                        self.state
                            .emit_fmt(format_args!("    vmovdqa %{}, %{}", name, ymm));
                    }
                    self.state.vec_last_store_reg = false;
                    return;
                }
            }
            // Value proven live in a YMM register within this block.
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
            if self.state.vec_last_store_reg && self.state.vec_last_store_val == Some(v.0) {
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
        self.state
            .emit_fmt(format_args!("    vmovdqu (%rax), %{}", ymm));
        self.state.vec_last_store_reg = false;
    }
    pub(super) fn avx_load_arg(&mut self, arg: &Operand) {
        // 256-bit loads write the full YMM register: upper halves dirty.
        self.state.dirty_upper_ymm = true;
        self.avx_load_arg_to(arg, "ymm0");
    }

    /// Store %ymm0 to a 256-bit operand's register or home slot.
    pub(super) fn avx_store_dest(&mut self, dest_ptr: &Value) {
        // 256-bit store paths read the full YMM register; the earlier load
        // already dirtied it, but mark defensively (a broadcast-only body
        // could reach here through a register-copy path).
        self.state.dirty_upper_ymm = true;
        if let Some(&reg) = self.reg_assignments.get(&dest_ptr.0) {
            if is_xmm_reg(reg) {
                let name = phys_reg_name_256(reg);
                if name != "ymm0" {
                    self.state
                        .emit_fmt(format_args!("    vmovdqa %ymm0, %{}", name));
                }
                self.state.vec_live_regs.insert(dest_ptr.0, name);
                self.state.vec_last_store_val = Some(dest_ptr.0);
                self.state.vec_last_store_reg = true;
                self.state.vec_last_store_reg_name = Some(name);
                return;
            }
        }
        let deferred = self.state.vector_defer_values.contains(&dest_ptr.0);
        if deferred && std::env::var("CCC_DEBUG_VDEFER").is_ok() {
            eprintln!(
                "[VDEFER-EMIT] deferring result store for dest_ptr={}",
                dest_ptr.0
            );
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
    /// stale under the deferred-store optimization. Returns None then, so
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
            Operand::Const(IrConst::I128(v)) => Some((*v as u128).to_le_bytes()),
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
                    self.state
                        .emit_fmt(format_args!("    movdqa %{}, %{}", held, xmm));
                }
                return;
            }
        }
        self.operand_to_reg(arg, "rax");
        self.state
            .emit_fmt(format_args!("    movdqu (%rax), %{}", xmm));
    }

    /// Materialize a 16-byte _Float128 constant into an XMM register via a
    /// 16-byte stack scratch (constant propagation inlines literals into
    /// F128Fabs/F128Copysign intrinsics).
    fn emit_f128_const_to_xmm(&mut self, bytes: [u8; 16], xmm: &str) {
        let low = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
        let high = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
        self.state.emit("    subq $16, %rsp");
        self.state
            .emit_fmt(format_args!("    movabsq ${}, %rax", low as i64));
        self.state.emit("    movq %rax, (%rsp)");
        self.state
            .emit_fmt(format_args!("    movabsq ${}, %rax", high as i64));
        self.state.emit("    movq %rax, 8(%rsp)");
        self.state
            .emit_fmt(format_args!("    movdqu (%rsp), %{}", xmm));
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
                    self.state
                        .emit_fmt(format_args!("    movq %rax, {}(%rsp)", off));
                    self.state
                        .out
                        .emit_instr_rbp_reg("    movzbl", slot.0 + 8, "ecx");
                    self.state
                        .emit_fmt(format_args!("    movb %cl, {}(%rsp)", off + 8));
                    self.state
                        .out
                        .emit_instr_rbp_reg("    movzbl", slot.0 + 9, "ecx");
                    self.state
                        .emit_fmt(format_args!("    movb %cl, {}(%rsp)", off + 9));
                } else {
                    // Register-held value: 16-byte copy through %xmm0.
                    self.emit_store_f128_xmm0_dest_to_rsp(v, off);
                }
            }
            Operand::Const(IrConst::LongDouble(_, bytes)) => {
                let low = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
                let high = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
                self.state
                    .emit_fmt(format_args!("    movabsq ${}, %rax", low as i64));
                self.state
                    .emit_fmt(format_args!("    movq %rax, {}(%rsp)", off));
                self.state
                    .emit_fmt(format_args!("    movabsq ${}, %rax", high as i64));
                self.state
                    .emit_fmt(format_args!("    movq %rax, {}(%rsp)", off + 8));
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
            self.state
                .emit_fmt(format_args!("    movdqa %{}, %xmm0", held));
        } else if let Some(&reg) = self.reg_assignments.get(&v.0) {
            if is_xmm_reg(reg) {
                self.state
                    .emit_fmt(format_args!("    movdqa %{}, %xmm0", phys_reg_name(reg)));
            } else {
                self.state
                    .emit_fmt(format_args!("    movq %{}, %xmm0", phys_reg_name(reg)));
            }
        } else {
            self.operand_to_reg(&Operand::Value(*v), "rax");
            self.state.emit("    movdqu (%rax), %xmm0");
        }
        self.state
            .emit_fmt(format_args!("    movdqu %xmm0, {}(%rsp)", off));
    }

    /// AVX2 256-bit unary op with immediate, 3-operand form: `inst $imm, %ymm1, %ymm0`.
    fn emit_avx_unary_imm_256(&mut self, dest_ptr: &Value, args: &[Operand], inst: &str) {
        self.avx_load_arg(&args[0]);
        // The assembler's vpsll*/vpsrl* encoders use the VEX 3-operand form
        // ($imm, src, dst); the 2-operand legacy form is not accepted.
        self.state
            .emit_fmt(format_args!("    {} %ymm0, %ymm0", inst));
        self.avx_store_dest(dest_ptr);
    }
    fn emit_sse_shuffle_imm_128(&mut self, dest_ptr: &Value, args: &[Operand], sse_inst: &str) {
        self.sse_load_arg(&args[0], "xmm0");
        let imm = self.operand_to_imm_i64(&args[1]);
        self.state
            .emit_fmt(format_args!("    {} ${}, %xmm0, %xmm0", sse_inst, imm));
        self.sse_store_dest(dest_ptr, "xmm0");
    }

    /// Resolve the (base, index) GPR pair for a VecLoad memory operand, reusing
    /// register-allocated GPRs when the values already live in them (the
    /// vectorizer's loop-carried base pointers and byte-offset IV are
    /// register-allocated), instead of copying them into rax/rcx first — a
    /// 2–3 instruction-per-load win in reduction hot loops. Returns
    /// (base_reg, index_reg_or_none); index is None when the offset is the
    /// constant zero. Both returned names are valid x86 SIB components.
    fn vec_load_addr_regs(
        &mut self,
        base_arg: &Operand,
        off_arg: &Operand,
    ) -> (String, Option<String>) {
        // The allocator only hands out rbx/r12-r15/r11/r10/r8/r9 (never
        // rsp/rbp/rdi/rsi/rdx), all of which are legal base AND index regs.
        let is_gpr = |r: PhysReg| (1..=16).contains(&r.0);
        let base = if let Some(r) = self.operand_reg(base_arg) {
            if is_gpr(r) {
                phys_reg_name(r).to_string()
            } else {
                self.operand_to_reg(base_arg, "rax");
                "rax".to_string()
            }
        } else {
            self.operand_to_reg(base_arg, "rax");
            "rax".to_string()
        };
        let index = if let Operand::Const(c) = off_arg {
            if c.to_i64() == Some(0) {
                None
            } else {
                self.operand_to_reg(off_arg, "rcx");
                Some("rcx".to_string())
            }
        } else if let Some(r) = self.operand_reg(off_arg) {
            if is_gpr(r) {
                Some(phys_reg_name(r).to_string())
            } else {
                self.operand_to_reg(off_arg, "rcx");
                Some("rcx".to_string())
            }
        } else {
            self.operand_to_reg(off_arg, "rcx");
            Some("rcx".to_string())
        };
        (base, index)
    }

    /// Constant displacement argument of a vector memory intrinsic, if the
    /// caller appended one (stencil taps: `VecLoad(base, byte_iv, disp)`).
    /// `idx` is the argument position that carries it (2 for loads, 3 for
    /// stores).
    fn vec_disp_arg(args: &[Operand], idx: usize) -> i64 {
        args.get(idx)
            .and_then(|o| match o {
                Operand::Const(c) => c.to_i64(),
                _ => None,
            })
            .unwrap_or(0)
    }

    /// Full x86 memory operand for a vector access: `disp(%base,%index)`.
    /// A zero displacement omits the field, keeping the encoding
    /// byte-identical to the two-argument (map) form.
    fn vec_mem_operand(&mut self, base_arg: &Operand, off_arg: &Operand, disp: i64) -> String {
        let (base, index) = self.vec_load_addr_regs(base_arg, off_arg);
        let disp_str = if disp == 0 {
            String::new()
        } else {
            format!("{}", disp)
        };
        match index {
            Some(ix) => format!("{}(%{},%{})", disp_str, base, ix),
            None => format!("{}(%{})", disp_str, base),
        }
    }

    /// Store a packed map result either through its legacy materialized GEP or
    /// directly through `(base, byte_offset)` operands.  The latter lets the
    /// x86 SIB form replace two LEAs and a pointer shuttle in every iteration.
    /// A trailing constant argument (args[3]) is a displacement, folding the
    /// stencil tap offset into the same memory operand.
    fn emit_vec_store_addr(
        &mut self,
        args: &[Operand],
        dest_ptr: &Option<Value>,
        mnemonic: &str,
        source_reg: &str,
    ) {
        if args.len() >= 3 {
            let disp = Self::vec_disp_arg(args, 3);
            let mem = self.vec_mem_operand(&args[1], &args[2], disp);
            self.state
                .emit_fmt(format_args!("    {} %{}, {}", mnemonic, source_reg, mem));
        } else if let Some(ptr) = dest_ptr {
            self.operand_to_reg(&Operand::Value(*ptr), "rax");
            self.state
                .emit_fmt(format_args!("    {} %{}, (%rax)", mnemonic, source_reg));
        }
    }

    pub(super) fn emit_intrinsic_impl(
        &mut self,
        dest: &Option<Value>,
        op: &IntrinsicOp,
        dest_ptr: &Option<Value>,
        args: &[Operand],
    ) {
        // VLFOLD: an eligible single-use 256-bit load emits nothing; its
        // adjacent consumer folds the source memory operand.
        if self.try_elide_vec_load(dest, op, args) {
            return;
        }
        // VLFOLD safety net: only the registered consumer or an intervening
        // pure vector load may follow an elided load; anything else
        // materialises it first.
        if let Some((pv, _, _)) = &self.state.pending_vec_memfold {
            let pv = *pv;
            let consumes = args
                .iter()
                .any(|a| matches!(a, Operand::Value(v) if v.0 == pv));
            if !consumes
                && !crate::backend::stack_layout::copy_coalescing::is_pure_vec_load(op)
            {
                self.materialize_pending_memfold();
            }
        }
        // Lazy flush: a deferred vector result may be pending in a register.
        // Flush it before any intrinsic that is not its cache-aware consumer
        // (fences/pause/rdtsc neither clobber XMM regs nor read vector slots,
        // so they let the pending value pass untouched).
        match op {
            IntrinsicOp::Lfence
            | IntrinsicOp::Mfence
            | IntrinsicOp::Sfence
            | IntrinsicOp::Pause
            | IntrinsicOp::Rdtsc
            | IntrinsicOp::Vzeroupper => {}
            _ => self.service_pending_vec_store(op, args),
        }
        match op {
            IntrinsicOp::Lfence => {
                self.state.emit("    lfence");
            }
            IntrinsicOp::Mfence => {
                self.state.emit("    mfence");
            }
            IntrinsicOp::Sfence => {
                self.state.emit("    sfence");
            }
            IntrinsicOp::Pause => {
                self.state.emit("    pause");
            }
            IntrinsicOp::Vzeroupper => {
                self.state.emit("    vzeroupper");
            }
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
            IntrinsicOp::Movnti
            | IntrinsicOp::Movnti64
            | IntrinsicOp::Movntdq
            | IntrinsicOp::Movntpd => {
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
            IntrinsicOp::Pcmpeqb128
            | IntrinsicOp::Pcmpeqd128
            | IntrinsicOp::Psubusb128
            | IntrinsicOp::Psubsb128
            | IntrinsicOp::Por128
            | IntrinsicOp::Pand128
            | IntrinsicOp::Pxor128
            | IntrinsicOp::AddPs128
            | IntrinsicOp::SubPs128
            | IntrinsicOp::MulPs128
            | IntrinsicOp::AddPd128
            | IntrinsicOp::SubPd128
            | IntrinsicOp::MulPd128 => {
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
            IntrinsicOp::Crc32_8
            | IntrinsicOp::Crc32_16
            | IntrinsicOp::Crc32_32
            | IntrinsicOp::Crc32_64 => {
                self.operand_to_reg(&args[0], "rax");
                self.operand_to_reg(&args[1], "rcx");
                let inst = match op {
                    IntrinsicOp::Crc32_8 => "crc32b %cl, %eax",
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
            IntrinsicOp::BuiltinSetjmp => {
                let buffer = args.first().expect("BuiltinSetjmp requires a buffer");
                self.operand_to_reg(buffer, "r11");
                let resume = self.state.fresh_label("builtin_setjmp_resume");
                let done = self.state.fresh_label("builtin_setjmp_done");
                self.state.emit("    movq %rbp, 0(%r11)");
                self.state
                    .emit_fmt(format_args!("    leaq {}(%rip), %rax", resume));
                self.state.emit("    movq %rax, 8(%r11)");
                self.state.emit("    movq %rsp, 16(%r11)");
                self.state.emit("    xorl %eax, %eax");
                self.state.out.emit_jmp_label(&done);
                self.state.out.emit_named_label(&resume);
                self.state.emit("    movl $1, %eax");
                self.state.out.emit_named_label(&done);
                if let Some(dest) = dest {
                    self.store_rax_to(dest);
                }
            }
            IntrinsicOp::BuiltinLongjmp => {
                let buffer = args.first().expect("BuiltinLongjmp requires a buffer");
                self.operand_to_reg(buffer, "rax");
                self.state.emit("    movq 8(%rax), %rdx");
                self.state.emit("    movq 0(%rax), %rcx");
                self.state.emit("    movq 16(%rax), %rsp");
                self.state.emit("    movq %rcx, %rbp");
                self.state.emit("    jmp *%rdx");
                self.state.reg_cache.invalidate_all();
            }

            // --- GCC __builtin_apply family ---
            //
            // Save-area layout (x86-64 SysV, 184 bytes):
            //   [0..48)   rdi, rsi, rdx, rcx, r8, r9   (integer argument regs)
            //   [48]      al                            (SSE vararg count)
            //   [56..184) xmm0..xmm7                    (SSE argument regs)
            IntrinsicOp::ApplyArgsAreaSize => {
                if let Some(d) = dest {
                    self.state.emit("    movl $184, %eax");
                    self.store_rax_to(d);
                }
            }
            IntrinsicOp::SaveApplyArgs => {
                let area_owned: Operand = dest_ptr
                    .as_ref()
                    .map(|v| Operand::Value(*v))
                    .or_else(|| args.first().cloned())
                    .expect("SaveApplyArgs requires an area pointer");
                let area_op = &area_owned;
                // Read-only on the argument registers: no live value is
                // clobbered (r10/r11 are the reserved call-staging scratch
                // pair).  movups keeps this correct regardless of the area's
                // runtime alignment.
                self.operand_to_reg(area_op, "r10");
                self.state.emit("    movq %rdi, 0(%r10)");
                self.state.emit("    movq %rsi, 8(%r10)");
                self.state.emit("    movq %rdx, 16(%r10)");
                self.state.emit("    movq %rcx, 24(%r10)");
                self.state.emit("    movq %r8, 32(%r10)");
                self.state.emit("    movq %r9, 40(%r10)");
                self.state.emit("    movb %al, 48(%r10)");
                self.state.emit("    testb %al, %al");
                let no_sse = self.state.fresh_label("apply_args_no_sse");
                self.state.out.emit_jmp_label(&no_sse);
                for (i, reg) in [
                    "xmm0", "xmm1", "xmm2", "xmm3", "xmm4", "xmm5", "xmm6", "xmm7",
                ]
                .iter()
                .enumerate()
                {
                    self.state
                        .emit_fmt(format_args!("    movups %{}, {}(%r10)", reg, 56 + i * 16));
                }
                self.state.out.emit_named_label(&no_sse);
                self.state.reg_cache.invalidate_acc();
            }
            IntrinsicOp::DoBuiltinApply => {
                // args: [func, save_area, result_area, size (unused: the
                // SysV protocol is entirely register-passed; `size` is only
                // meaningful for i686 stack arguments)]
                let func = args.first().expect("DoBuiltinApply requires func");
                let area = args.get(1).expect("DoBuiltinApply requires save area");
                let result = args.get(2).expect("DoBuiltinApply requires result area");
                self.operand_to_reg(area, "r10");
                self.operand_to_reg(func, "r11");
                self.state.emit("    movq 0(%r10), %rdi");
                self.state.emit("    movq 8(%r10), %rsi");
                self.state.emit("    movq 16(%r10), %rdx");
                self.state.emit("    movq 24(%r10), %rcx");
                self.state.emit("    movq 32(%r10), %r8");
                self.state.emit("    movq 40(%r10), %r9");
                // al must hold the SSE argument count for the callee's
                // varargs register-save prologue, whether or not it uses it.
                self.state.emit("    movzbl 48(%r10), %eax");
                self.state.emit("    testb %al, %al");
                let no_sse = self.state.fresh_label("apply_no_sse");
                self.state.out.emit_jmp_label(&no_sse);
                for (i, reg) in [
                    "xmm0", "xmm1", "xmm2", "xmm3", "xmm4", "xmm5", "xmm6", "xmm7",
                ]
                .iter()
                .enumerate()
                {
                    self.state
                        .emit_fmt(format_args!("    movups {}(%r10), %{}", 56 + i * 16, reg));
                }
                self.state.out.emit_named_label(&no_sse);
                self.state.emit("    call *%r11");
                // Capture the return value: result[0]=rax, result[8]=rdx,
                // result[16]=xmm0 (movups: alignment-agnostic).
                self.operand_to_reg(result, "r10");
                self.state.emit("    movq %rax, 0(%r10)");
                self.state.emit("    movq %rdx, 8(%r10)");
                self.state.emit("    movups %xmm0, 16(%r10)");
                self.state.reg_cache.invalidate_all();
            }
            IntrinsicOp::RestoreApplyResult => {
                let block = args.first().expect("RestoreApplyResult requires a block");
                self.operand_to_reg(block, "r10");
                self.state.emit("    movq 0(%r10), %rax");
                self.state.emit("    movq 8(%r10), %rdx");
                self.state.emit("    movups 16(%r10), %xmm0");
                self.state.reg_cache.invalidate_acc();
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
                    self.state
                        .emit_fmt(format_args!("    movq {}(%rsp), %rax", off));
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
            IntrinsicOp::FmaScalarF64 => {
                // dest = args[0]*args[1] + args[2], single rounding.
                // vfmadd231 form: dest_reg preloaded with the addend, then
                // dest = src2*src1 + dest — exactly emit_scalar_fma231's
                // contract (shared with the mul+add fusion pass).
                self.emit_scalar_fma231(&args[0], &args[1], &args[2], &dest.unwrap(), IrType::F64);
            }
            IntrinsicOp::FmaScalarF32 => {
                self.emit_scalar_fma231(&args[0], &args[1], &args[2], &dest.unwrap(), IrType::F32);
            }
            IntrinsicOp::RoundScalarF64(imm) => {
                self.emit_fp_scalar_round(dest, &args[0], IrType::F64, *imm);
            }
            IntrinsicOp::RoundScalarF32(imm) => {
                self.emit_fp_scalar_round(dest, &args[0], IrType::F32, *imm);
            }
            IntrinsicOp::CopysignF64 => {
                self.emit_fp_copysign(dest, &args[0], &args[1], IrType::F64);
            }
            IntrinsicOp::CopysignF32 => {
                self.emit_fp_copysign(dest, &args[0], &args[1], IrType::F32);
            }
            IntrinsicOp::SqrtF64 => {
                // Prefer VEX scalar sqrt (ICX/GCC on x86-64-v3). Avoids the
                // legacy SSE encoding and matches the vmul/vadd path.
                self.emit_fp_scalar_unary(dest, &args[0], IrType::F64, "vsqrtsd");
            }
            IntrinsicOp::SqrtF32 => {
                self.emit_fp_scalar_unary(dest, &args[0], IrType::F32, "vsqrtss");
            }
            IntrinsicOp::FabsF64 => {
                // single andpd against a rodata mask, honoring the
                // XMM-allocated destination (was movabsq + movq + andpd +
                // GPR round-trip).
                if let Some(d) = dest {
                    if let Some(&reg) = self.reg_assignments.get(&d.0) {
                        if is_xmm_reg(reg) {
                            let dname = phys_reg_name(reg);
                            self.load_fp_to_reg(&args[0], IrType::F64, dname);
                            let label = self.state.get_fp_const_label(0x7FFF_FFFF_FFFF_FFFFu64);
                            self.state
                                .emit_fmt(format_args!("    andpd {}(%rip), %{}", label, dname));
                            self.state.reg_cache.invalidate_acc();
                            return;
                        }
                    }
                }
                self.load_fp_to_xmm0(&args[0], IrType::F64);
                let label = self.state.get_fp_const_label(0x7FFF_FFFF_FFFF_FFFFu64);
                self.state
                    .emit_fmt(format_args!("    andpd {}(%rip), %xmm0", label));
                if let Some(d) = dest {
                    self.store_xmm0_fp_dest(d, IrType::F64);
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
                        self.state
                            .out
                            .emit_instr_rbp_reg("    movzbl", sx + 8, "ecx");
                        self.state.out.emit_instr_reg_rbp("    movb", "cl", sd + 8);
                        self.state
                            .out
                            .emit_instr_rbp_reg("    movzbl", sx + 9, "ecx");
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
                    if let (Operand::Const(IrConst::LongDouble(_, bytes)), Some(sd)) =
                        (&args[0], sd)
                    {
                        let low = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
                        let high = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
                        self.state
                            .emit_fmt(format_args!("    movabsq ${}, %rax", low as i64));
                        self.state.out.emit_instr_reg_rbp("    movq", "rax", sd);
                        self.state
                            .emit_fmt(format_args!("    movabsq ${}, %rax", high as i64));
                        self.state.out.emit_instr_reg_rbp("    movq", "rax", sd + 8);
                        self.state
                            .out
                            .emit_instr_rbp_reg("    movzbl", sd + 9, "ecx");
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
                        self.state
                            .out
                            .emit_instr_rbp_reg("    movzbl", sx + 8, "ecx");
                        self.state.out.emit_instr_reg_rbp("    movb", "cl", sd + 8);
                        self.state
                            .out
                            .emit_instr_rbp_reg("    movzbl", sy + 9, "eax");
                        self.state.emit("    andb $0x80, %al");
                        self.state
                            .out
                            .emit_instr_rbp_reg("    movzbl", sx + 9, "ecx");
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
                // single andps against a rodata mask, honoring the
                // XMM-allocated destination.
                if let Some(d) = dest {
                    if let Some(&reg) = self.reg_assignments.get(&d.0) {
                        if is_xmm_reg(reg) {
                            let dname = phys_reg_name(reg);
                            self.load_fp_to_reg(&args[0], IrType::F32, dname);
                            let label = self.state.get_fp_const_label(0x7FFF_FFFFu64);
                            self.state
                                .emit_fmt(format_args!("    andps {}(%rip), %{}", label, dname));
                            self.state.reg_cache.invalidate_acc();
                            return;
                        }
                    }
                }
                self.load_fp_to_xmm0(&args[0], IrType::F32);
                let label = self.state.get_fp_const_label(0x7FFF_FFFFu64);
                self.state
                    .emit_fmt(format_args!("    andps {}(%rip), %xmm0", label));
                if let Some(d) = dest {
                    self.store_xmm0_fp_dest(d, IrType::F32);
                }
            }
            // AES-NI binary ops: aesenc, aesenclast, aesdec, aesdeclast
            IntrinsicOp::Aesenc128
            | IntrinsicOp::Aesenclast128
            | IntrinsicOp::Aesdec128
            | IntrinsicOp::Aesdeclast128 => {
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
                    self.state
                        .emit_fmt(format_args!("    aeskeygenassist ${}, %xmm0, %xmm0", imm));
                    self.sse_store_dest(dptr, "xmm0");
                }
            }
            // CLMUL: pclmulqdq with immediate
            IntrinsicOp::Pclmulqdq128 => {
                if let Some(dptr) = dest_ptr {
                    self.sse_load_arg(&args[0], "xmm0");
                    self.sse_load_arg(&args[1], "xmm1");
                    let imm = self.operand_to_imm_i64(&args[2]);
                    self.state
                        .emit_fmt(format_args!("    pclmulqdq ${}, %xmm1, %xmm0", imm));
                    self.sse_store_dest(dptr, "xmm0");
                }
            }
            // SSE2 shift-by-immediate operations
            IntrinsicOp::Pslldqi128
            | IntrinsicOp::Psrldqi128
            | IntrinsicOp::Psllqi128
            | IntrinsicOp::Psrlqi128 => {
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
                    self.state
                        .emit_fmt(format_args!("    pblendw ${}, %xmm1, %xmm0", imm));
                    self.sse_store_dest(dptr, "xmm0");
                }
            }
            // SSE2 binary 128-bit operations
            IntrinsicOp::Paddw128
            | IntrinsicOp::Psubw128
            | IntrinsicOp::Pmulhw128
            | IntrinsicOp::Pmullw128
            | IntrinsicOp::Pmuludq128
            | IntrinsicOp::Pmuldq128
            | IntrinsicOp::Pmulld128
            | IntrinsicOp::Pmaddwd128
            | IntrinsicOp::Pmaddubsw128
            | IntrinsicOp::Pcmpgtw128
            | IntrinsicOp::Pcmpgtb128
            | IntrinsicOp::Paddd128
            | IntrinsicOp::Psubd128
            | IntrinsicOp::Paddb128
            | IntrinsicOp::Psubb128
            | IntrinsicOp::Psubusw128
            | IntrinsicOp::Psadbw128
            | IntrinsicOp::Pshufb128
            | IntrinsicOp::Pmaxub128
            | IntrinsicOp::Pminub128
            | IntrinsicOp::Packssdw128
            | IntrinsicOp::Packsswb128
            | IntrinsicOp::Packuswb128
            | IntrinsicOp::Punpcklbw128
            | IntrinsicOp::Punpckhbw128
            | IntrinsicOp::Punpcklwd128
            | IntrinsicOp::Punpckhwd128 => {
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
            // SSE4.1 widening conversions are unary.  They were historically
            // routed through emit_sse_binary_128, which asserted args.len() >= 2
            // and turned valid _mm_cvtepu8_epi16/_mm_cvtepu16_epi32 programs
            // into a compiler panic.
            IntrinsicOp::Pmovzxbw128 | IntrinsicOp::Pmovzxwd128 => {
                if let Some(dptr) = dest_ptr {
                    let inst = if matches!(op, IntrinsicOp::Pmovzxbw128) {
                        "pmovzxbw"
                    } else {
                        "pmovzxwd"
                    };
                    self.sse_load_arg(&args[0], "xmm0");
                    self.state
                        .emit_fmt(format_args!("    {} %xmm0, %xmm0", inst));
                    self.sse_store_dest(dptr, "xmm0");
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
                        self.state
                            .emit_fmt(format_args!("    palignr ${}, %xmm1, %xmm0", imm));
                    } else {
                        self.state
                            .emit_fmt(format_args!("    {} %xmm1, %xmm0", inst));
                    }
                    self.sse_store_dest(dptr, "xmm0");
                }
            }
            // SSE2 variable-shift (count in xmm register)
            IntrinsicOp::Psllw128 | IntrinsicOp::Psrlw128 => {
                if let Some(dptr) = dest_ptr {
                    let inst = if matches!(op, IntrinsicOp::Psllw128) {
                        "psllw"
                    } else {
                        "psrlw"
                    };
                    self.sse_load_arg(&args[0], "xmm0");
                    self.sse_load_arg(&args[1], "xmm1");
                    self.state
                        .emit_fmt(format_args!("    {} %xmm1, %xmm0", inst));
                    self.sse_store_dest(dptr, "xmm0");
                }
            }

            // pabsb/pabsw/pabsd are UNARY (2-operand AT&T: src, dst). Fixes the
            // A latent bug where Pabsb128 was dispatched through the binary
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
                    self.state
                        .emit_fmt(format_args!("    {} %xmm0, %xmm0", inst));
                    self.sse_store_dest(dptr, "xmm0");
                }
            }
            // --- AVX2 256-bit integer ops ---
            IntrinsicOp::Paddb256
            | IntrinsicOp::Paddw256
            | IntrinsicOp::Paddd256
            | IntrinsicOp::Psubb256
            | IntrinsicOp::Psubw256
            | IntrinsicOp::Psubusw256
            | IntrinsicOp::Psadbw256
            | IntrinsicOp::Pmaddubsw256
            | IntrinsicOp::Pmaddwd256
            | IntrinsicOp::Pcmpeqb256
            | IntrinsicOp::Pcmpgtb256
            | IntrinsicOp::Pshufb256
            | IntrinsicOp::Pmaxub256
            | IntrinsicOp::Pminub256
            | IntrinsicOp::Pxor256
            | IntrinsicOp::Por256
            | IntrinsicOp::Pand256 => {
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
                        IntrinsicOp::Paddb256
                            | IntrinsicOp::Paddw256
                            | IntrinsicOp::Paddd256
                            | IntrinsicOp::Psadbw256
                            | IntrinsicOp::Pmaddwd256
                            | IntrinsicOp::Pcmpeqb256
                            | IntrinsicOp::Pmaxub256
                            | IntrinsicOp::Pminub256
                            | IntrinsicOp::Pxor256
                            | IntrinsicOp::Por256
                            | IntrinsicOp::Pand256
                    );
                    self.emit_avx_binary_256(dptr, args, inst, commutative);
                }
            }
            // vpabsb/vpabsw are UNARY (2-operand AT&T: src, dst).
            IntrinsicOp::Pabsb256 | IntrinsicOp::Pabsw256 => {
                if let Some(dptr) = dest_ptr {
                    let inst = if matches!(op, IntrinsicOp::Pabsb256) {
                        "vpabsb"
                    } else {
                        "vpabsw"
                    };
                    if let Some(mem) = self.vec_arg_mem(&args[0]) {
                        self.state
                            .emit_fmt(format_args!("    {} {}, %ymm0", inst, mem));
                    } else {
                        self.avx_load_arg(&args[0]);
                        self.state
                            .emit_fmt(format_args!("    {} %ymm0, %ymm0", inst));
                    }
                    self.avx_store_dest(dptr);
                }
            }
            IntrinsicOp::Psllidi256
            | IntrinsicOp::Psrlidi256
            | IntrinsicOp::Psllwi256
            | IntrinsicOp::Psrlwi256 => {
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
                    let inst = if matches!(op, IntrinsicOp::Loadu256) {
                        "vmovdqu"
                    } else {
                        "vmovdqa"
                    };
                    self.avx_load_arg(&args[0]);
                    self.state
                        .emit_fmt(format_args!("    {} %ymm0, %ymm0", inst));
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
                    self.state.emit_fmt(format_args!(
                        "    vinserti128 ${}, %xmm1, %ymm0, %ymm0",
                        lane
                    ));
                    self.avx_store_dest(dptr);
                }
            }
            IntrinsicOp::SetEpi8_256
            | IntrinsicOp::SetEpi16_256
            | IntrinsicOp::SetEpi32_256
            | IntrinsicOp::SetEpi64x256 => {
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
                        let mask = if bits == 8 {
                            u64::MAX
                        } else {
                            (1u64 << (bits * 8)) - 1
                        };
                        let operand = self.vec_const_rip_operand((v as u64) & mask, bits);
                        self.state
                            .emit_fmt(format_args!("    {} {}, %ymm0", inst, operand));
                    } else if self.avx512_enabled {
                        // EVEX GPR-source vpbroadcast: 1 uop on port 5, no movd.
                        // operand_to_reg materialises through a 64-bit `movq`,
                        // so it must be handed the 64-bit register name; only
                        // the broadcast operand itself is width-specific.
                        self.operand_to_reg(&args[0], "rax");
                        let reg = if bits == 8 { "rax" } else { "eax" };
                        self.state
                            .emit_fmt(format_args!("    {} %{}, %ymm0", inst, reg));
                    } else {
                        // AVX2: vmovd + xmm-source vpbroadcast (matches GCC/LLVM).
                        //
                        // Always load through %rax: operand_to_reg emits
                        // `movq %src, %dst` and passing "eax" produced the
                        // invalid `movq %r8, %eax`, which the integrated
                        // assembler rightly rejected -- zlib-ng's
                        // slide_hash_avx2.c (`_mm256_set1_epi16((short)wsize)`)
                        // failed to build because of it. The narrowing to 32
                        // or 8 bits is expressed by the movd/movq that follows.
                        self.operand_to_reg(&args[0], "rax");
                        if bits == 8 {
                            self.state.emit("    movq %rax, %xmm0");
                        } else {
                            self.state.emit("    movd %eax, %xmm0");
                        }
                        self.state
                            .emit_fmt(format_args!("    {} %xmm0, %ymm0", inst));
                    }
                    self.avx_store_dest(dptr);
                }
            }
            // --- AVX-VNNI / INT8 / INT16 3-op dot products (128 + 256) ---
            IntrinsicOp::Dpbusd128
            | IntrinsicOp::Dpbusds128
            | IntrinsicOp::Dpwusd128
            | IntrinsicOp::Dpwusds128
            | IntrinsicOp::Dpbssd128
            | IntrinsicOp::Dpbssds128
            | IntrinsicOp::Dpbsud128
            | IntrinsicOp::Dpbsuds128
            | IntrinsicOp::Dpbuud128
            | IntrinsicOp::Dpbuuds128
            | IntrinsicOp::Dpwuud128
            | IntrinsicOp::Dpwuuds128
            | IntrinsicOp::Dpwssd128
            | IntrinsicOp::Dpwssds128
            | IntrinsicOp::Dpbusd256
            | IntrinsicOp::Dpbusds256
            | IntrinsicOp::Dpwusd256
            | IntrinsicOp::Dpwusds256
            | IntrinsicOp::Dpbssd256
            | IntrinsicOp::Dpbssds256
            | IntrinsicOp::Dpbsud256
            | IntrinsicOp::Dpbsuds256
            | IntrinsicOp::Dpbuud256
            | IntrinsicOp::Dpbuuds256
            | IntrinsicOp::Dpwuud256
            | IntrinsicOp::Dpwuuds256
            | IntrinsicOp::Dpwssd256
            | IntrinsicOp::Dpwssds256 => {
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
                    let is256 = matches!(
                        op,
                        IntrinsicOp::Dpbusd256
                            | IntrinsicOp::Dpbusds256
                            | IntrinsicOp::Dpwusd256
                            | IntrinsicOp::Dpwusds256
                            | IntrinsicOp::Dpbssd256
                            | IntrinsicOp::Dpbssds256
                            | IntrinsicOp::Dpbsud256
                            | IntrinsicOp::Dpbsuds256
                            | IntrinsicOp::Dpbuud256
                            | IntrinsicOp::Dpbuuds256
                            | IntrinsicOp::Dpwuud256
                            | IntrinsicOp::Dpwuuds256
                            | IntrinsicOp::Dpwssd256
                            | IntrinsicOp::Dpwssds256
                    );
                    if is256 {
                        self.avx_load_arg_to(&args[0], "ymm0");
                        self.avx_load_arg_to(&args[1], "ymm1");
                        self.avx_load_arg_to(&args[2], "ymm2");
                        self.state
                            .emit_fmt(format_args!("    {} %ymm2, %ymm1, %ymm0", inst));
                        self.avx_store_dest(dptr);
                    } else {
                        self.sse_load_arg(&args[0], "xmm0");
                        self.sse_load_arg(&args[1], "xmm1");
                        self.sse_load_arg(&args[2], "xmm2");
                        self.state
                            .emit_fmt(format_args!("    {} %xmm2, %xmm1, %xmm0", inst));
                        self.sse_store_dest(dptr, "xmm0");
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
                    self.state
                        .emit_fmt(format_args!("    {} ${}, %xmm1, %xmm0", inst, imm));
                    self.sse_store_dest(dptr, "xmm0");
                }
            }
            // --- VAES 256-bit + VPCLMULQDQ 256-bit ---
            IntrinsicOp::Aesenc256
            | IntrinsicOp::Aesenclast256
            | IntrinsicOp::Aesdec256
            | IntrinsicOp::Aesdeclast256 => {
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
                            self.state.emit_fmt(format_args!(
                                "    vmovdqu {}, %ymm1",
                                self.slot_ref(slot.0)
                            ));
                        } else {
                            self.operand_to_reg(&args[1], "rcx");
                            self.state.emit("    vmovdqu (%rcx), %ymm1");
                        }
                    } else {
                        self.operand_to_reg(&args[1], "rcx");
                        self.state.emit("    vmovdqu (%rcx), %ymm1");
                    }
                    self.state
                        .emit_fmt(format_args!("    {} %ymm1, %ymm0, %ymm0", inst));
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
                            self.state.emit_fmt(format_args!(
                                "    vmovdqu {}, %ymm1",
                                self.slot_ref(slot.0)
                            ));
                        } else {
                            self.operand_to_reg(&args[1], "rcx");
                            self.state.emit("    vmovdqu (%rcx), %ymm1");
                        }
                    } else {
                        self.operand_to_reg(&args[1], "rcx");
                        self.state.emit("    vmovdqu (%rcx), %ymm1");
                    }
                    let imm = self.operand_to_imm_i64(&args[2]);
                    self.state
                        .emit_fmt(format_args!("    vpclmulqdq ${}, %ymm1, %ymm0, %ymm0", imm));
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
            IntrinsicOp::Paddusb128
            | IntrinsicOp::Paddsb128
            | IntrinsicOp::Paddusw128
            | IntrinsicOp::Paddsw128
            | IntrinsicOp::Psubsw128
            | IntrinsicOp::Pandn128
            | IntrinsicOp::Pcmpeqw128
            | IntrinsicOp::Pcmpgtd128
            | IntrinsicOp::Pavgb128
            | IntrinsicOp::Pavgw128
            | IntrinsicOp::Pminsw128
            | IntrinsicOp::Pmaxsw128
            | IntrinsicOp::Pmulhuw128
            | IntrinsicOp::Paddq128
            | IntrinsicOp::Psubq128
            | IntrinsicOp::Punpckldq128
            | IntrinsicOp::Punpckhdq128
            | IntrinsicOp::Punpcklqdq128
            | IntrinsicOp::Punpckhqdq128 => {
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
            IntrinsicOp::Pmulld256
            | IntrinsicOp::Psubd256
            | IntrinsicOp::Paddq256
            | IntrinsicOp::Psubq256
            | IntrinsicOp::Pandn256
            | IntrinsicOp::Pcmpeqd256
            | IntrinsicOp::Pcmpeqq256
            | IntrinsicOp::Pcmpgtd256
            | IntrinsicOp::Pcmpgtq256
            | IntrinsicOp::AddPs256
            | IntrinsicOp::SubPs256
            | IntrinsicOp::MulPs256
            | IntrinsicOp::AddPd256
            | IntrinsicOp::SubPd256
            | IntrinsicOp::MulPd256
            | IntrinsicOp::Punpcklbw256
            | IntrinsicOp::Punpckhbw256
            | IntrinsicOp::Punpcklwd256
            | IntrinsicOp::Punpckhwd256
            | IntrinsicOp::Punpckldq256
            | IntrinsicOp::Punpckhdq256
            | IntrinsicOp::Punpcklqdq256
            | IntrinsicOp::Punpckhqdq256
            | IntrinsicOp::Pmullw256
            | IntrinsicOp::Pmulhw256
            | IntrinsicOp::Pminsd256
            | IntrinsicOp::Pmaxsd256
            | IntrinsicOp::Packssdw256
            | IntrinsicOp::Packuswb256
            | IntrinsicOp::Phaddw256
            | IntrinsicOp::Phaddd256
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
                    self.state
                        .emit_fmt(format_args!("    vextracti128 ${}, %ymm0, %xmm0", lane));
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
                    self.state
                        .emit_fmt(format_args!("    vperm2i128 ${}, %ymm1, %ymm0, %ymm0", imm));
                    self.avx_store_dest(dptr);
                }
            }
            IntrinsicOp::Permute4x64 => {
                if let Some(dptr) = dest_ptr {
                    self.avx_load_arg(&args[0]);
                    let imm = self.operand_to_imm_i64(&args[1]);
                    self.state
                        .emit_fmt(format_args!("    vpermq ${}, %ymm0, %ymm0", imm));
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
                    self.state
                        .emit_fmt(format_args!("    vpshufd ${}, %ymm0, %ymm0", imm));
                    self.avx_store_dest(dptr);
                }
            }
            IntrinsicOp::Pslldqi256 | IntrinsicOp::Psrldqi256 => {
                if let Some(dptr) = dest_ptr {
                    self.avx_load_arg(&args[0]);
                    let imm = self.operand_to_imm_i64(&args[1]) & 0xff;
                    let inst = if matches!(op, IntrinsicOp::Pslldqi256) {
                        "vpslldq"
                    } else {
                        "vpsrldq"
                    };
                    self.state
                        .emit_fmt(format_args!("    {} ${}, %ymm0, %ymm0", inst, imm));
                    self.avx_store_dest(dptr);
                }
            }
            IntrinsicOp::Psllqi256
            | IntrinsicOp::Psrlqi256
            | IntrinsicOp::Psrawi256
            | IntrinsicOp::Psradi256 => {
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
                    self.state
                        .emit_fmt(format_args!("    {} ${}, %ymm0, %ymm0", inst, imm));
                    self.avx_store_dest(dptr);
                }
            }
            IntrinsicOp::Pmovzxbw256
            | IntrinsicOp::Pmovzxbd256
            | IntrinsicOp::Pmovzxwd256
            | IntrinsicOp::Pmovsxbw256
            | IntrinsicOp::Pmovsxbd256
            | IntrinsicOp::Pmovsxwd256 => {
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
                    self.state
                        .emit_fmt(format_args!("    {} %xmm0, %ymm0", inst));
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
            IntrinsicOp::Psllwi128
            | IntrinsicOp::Psrlwi128
            | IntrinsicOp::Psrawi128
            | IntrinsicOp::Psradi128
            | IntrinsicOp::Pslldi128
            | IntrinsicOp::Psrldi128 => {
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
            IntrinsicOp::Pinsrw128
            | IntrinsicOp::Pinsrd128
            | IntrinsicOp::Pinsrb128
            | IntrinsicOp::Pinsrq128 => {
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
            IntrinsicOp::Pextrw128
            | IntrinsicOp::Pextrd128
            | IntrinsicOp::Pextrb128
            | IntrinsicOp::Pextrq128 => {
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
                        self.state
                            .emit_fmt(format_args!("    movdqu {}, %xmm0", mem));
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

                    // FMA3: load C, fused multiply-add with B, store back.
                    //
                    // Every move here uses the VEX encoding even though the
                    // 128-bit legacy forms are the same length. vfmadd231pd is
                    // VEX-only, so a legacy movsd/unpcklpd/movupd next to it
                    // would straddle the two domains; and this kernel runs
                    // inside loops that elsewhere emit 256-bit ymm code, where
                    // a dirty upper half turns each legacy instruction into an
                    // AVX->SSE state transition (~70 cycles on Intel). VEX-128
                    // zeroes the upper bits by definition, so no transition can
                    // occur. See scripts/check_avx_sse_transitions.py.
                    self.state
                        .emit_fmt(format_args!("    vmovsd (%{}), %xmm1", a_name)); // xmm1 = A scalar
                    self.state.emit("    vunpcklpd %xmm1, %xmm1, %xmm1"); // xmm1 = {A, A}
                    self.state
                        .emit_fmt(format_args!("    vmovupd (%{}), %xmm0", c_name)); // xmm0 = {C[j], C[j+1]}
                    self.state
                        .emit_fmt(format_args!("    vfmadd231pd (%{}), %xmm1, %xmm0", b_name));
                    self.state
                        .emit_fmt(format_args!("    vmovupd %xmm0, (%{})", c_name)); // store back

                    self.state.reg_cache.invalidate_all();
                }
            }
            IntrinsicOp::FmaF64x2Hoisted => {
                // Two-wide hoisted FMA group: C[0..4] += broadcast * B[0..4].
                // Primarily emitted by the AArch64 two-wide vectorizer (two
                // NEON fmla per group); reachable on x86 via LCCC_FORCE_SSE2.
                // This was a silent no-op stub here — the program only stayed
                // correct because the (then-buggy) remainder computation
                // restarted at element 0 and redid ALL the work scalar. With
                // the remainder start fixed (IV*4 for the group scheme) a
                // no-op body would silently drop the whole accumulation, so
                // emit the real thing: one 4-lane FMA via ymm, semantically
                // identical to the AArch64 pair of fmla v.2d.
                // args[0] = B pointer, dest_ptr = C pointer; the broadcast
                // factor is already in ymm1 (BroadcastLoadF64).
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                if let Some(c_ptr) = dest_ptr {
                    // Prefer the register allocator's assignments (same
                    // value-cache conflation hazard as FmaF64x2 above: B and
                    // C GEPs share the offset value but not the base).
                    let b_name = if let Some(r) = self.operand_reg(&args[0]) {
                        super::emit::phys_reg_name(r)
                    } else {
                        self.operand_to_reg(&args[0], "rdx");
                        "rdx"
                    };
                    let c_name = if let Some(r) = self.dest_reg(c_ptr) {
                        super::emit::phys_reg_name(r)
                    } else {
                        self.value_to_reg(c_ptr, "rax");
                        "rax"
                    };
                    self.state
                        .emit_fmt(format_args!("    vmovupd (%{}), %ymm0", c_name));
                    self.state
                        .emit_fmt(format_args!("    vfmadd231pd (%{}), %ymm1, %ymm0", b_name));
                    self.state
                        .emit_fmt(format_args!("    vmovupd %ymm0, (%{})", c_name));
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
                    let b_val = match &args[1] {
                        Operand::Value(v) => Some(v.0),
                        _ => None,
                    };
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

                        self.state.emit("    vmovsd (%rcx), %xmm1");
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

                        self.state
                            .emit_fmt(format_args!("    vmovsd (%{}), %xmm1", a_name));
                        self.state.emit("    vbroadcastsd %xmm1, %ymm1");
                        self.state
                            .emit_fmt(format_args!("    vmovupd (%{}), %ymm0", c_name));
                        self.state
                            .emit_fmt(format_args!("    vfmadd231pd (%{}), %ymm1, %ymm0", b_name));
                        self.state
                            .emit_fmt(format_args!("    vmovupd %ymm0, (%{})", c_name));
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
                    self.operand_to_reg(&args[0], "rdx"); // B ptr → %rdx
                    self.value_to_reg(c_ptr, "rax"); // C ptr → %rax

                    self.state.emit("    vmovupd (%rax), %ymm0"); // Load C[j..j+3]
                    self.state.emit("    vfmadd231pd (%rdx), %ymm1, %ymm0"); // ymm0 = ymm1*B + ymm0
                    self.state.emit("    vmovupd %ymm0, (%rax)"); // Store C[j..j+3]
                }
            }
            IntrinsicOp::BroadcastLoadF64 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                // Load scalar F64 from pointer and broadcast to ymm1.
                // Placed before the vectorized j-loop.
                self.operand_to_reg(&args[0], "rcx");
                self.state.emit("    vmovsd (%rcx), %xmm1");
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
                self.operand_to_reg(&args[0], "rcx"); // A ptr → %rcx
                self.operand_to_reg(&args[1], "rax"); // C base → %rax
                self.operand_to_reg(&args[2], "rdx"); // B base → %rdx
                self.operand_to_reg(&args[3], "rsi"); // byte offset → %rsi

                // Load A, broadcast
                self.state.emit("    vmovsd (%rcx), %xmm1");
                self.state.emit("    vbroadcastsd %xmm1, %ymm1");

                // FMA with SIB addressing
                self.state.emit("    vmovupd (%rax,%rsi), %ymm0"); // Load C[j..j+3]
                self.state.emit("    vfmadd231pd (%rdx,%rsi), %ymm1, %ymm0"); // ymm0 = A*B + C
                self.state.emit("    vmovupd %ymm0, (%rax,%rsi)"); // Store C[j..j+3]

                self.state.reg_cache.invalidate_all();
            }
            IntrinsicOp::FmaF64x4HoistedSIB => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                // Optimal quad-FMA: SIB + hoisted broadcast.
                // args[0] = C base pointer (row base, loop-invariant)
                // args[1] = B base pointer (row base, loop-invariant)
                // args[2] = byte offset (j*8, shared across 4 chunks)
                // args[3] = optional displacement (0,32,64,96) as const
                //           If present, emits disp(%base,%off) without extra leaq.
                //           If absent, falls back to 3-arg form where offset
                //           may itself be Add(base, disp) — we fold that too.
                //
                // Result per iteration (godlike):
                //   movslq %r12d, %r10
                //   vmovupd (%rbx,%r10), %ymm0 / vfmadd (%r14,%r10), %ymm1, %ymm0 / vmovupd %ymm0, (%rbx,%r10)
                //   vmovupd 32(%rbx,%r10), %ymm0 / vfmadd 32(%r14,%r10), %ymm1, %ymm0 / ...
                //   vmovupd 64(%rbx,%r10), %ymm0 / ...
                //   vmovupd 96(%rbx,%r10), %ymm0 / ...
                // Total: 1 movslq + 12 FMA + loop control = ~15 insns vs GCC 25 total func.

                // Extract displacement from 4th arg if present
                let mut disp: i64 = 0;
                if args.len() >= 4 {
                    if let Operand::Const(c) = &args[3] {
                        if let Some(d) = c.to_i64() {
                            disp = d;
                        }
                    }
                } else {
                    // Backward compat: try to fold offset = base + const
                    if let Operand::Value(v) = &args[2] {
                        if let Some(inst) = self.get_defining_instruction(v.0) {
                            if let crate::ir::reexports::Instruction::BinOp {
                                op: crate::ir::reexports::IrBinOp::Add,
                                lhs,
                                rhs,
                                ..
                            } = inst
                            {
                                let try_extract =
                                    |a: &Operand, b: &Operand| -> Option<(Operand, i64)> {
                                        match (a, b) {
                                            (Operand::Value(_), Operand::Const(c)) => {
                                                c.to_i64().map(|d| (a.clone(), d))
                                            }
                                            (Operand::Const(c), Operand::Value(_)) => {
                                                c.to_i64().map(|d| (b.clone(), d))
                                            }
                                            _ => None,
                                        }
                                    };
                                if let Some((_, d)) = try_extract(lhs, rhs) {
                                    disp = d;
                                }
                            }
                        }
                    }
                }

                // If we folded from Add, we need base offset Value, not the Add result.
                // For 4-arg form, base is args[2]; for 3-arg folded form, base is lhs of Add.
                let off_operand: Operand = if args.len() >= 4 {
                    args[2].clone()
                } else if disp != 0 {
                    // Extract base from Add
                    if let Operand::Value(v) = &args[2] {
                        if let Some(crate::ir::reexports::Instruction::BinOp { lhs, rhs, .. }) =
                            self.get_defining_instruction(v.0)
                        {
                            if matches!(lhs, Operand::Value(_)) && matches!(rhs, Operand::Const(_))
                            {
                                lhs.clone()
                            } else if matches!(rhs, Operand::Value(_))
                                && matches!(lhs, Operand::Const(_))
                            {
                                rhs.clone()
                            } else {
                                args[2].clone()
                            }
                        } else {
                            args[2].clone()
                        }
                    } else {
                        args[2].clone()
                    }
                } else {
                    args[2].clone()
                };

                let c_name = if let Some(r) = self.operand_reg(&args[0]) {
                    super::emit::phys_reg_name(r)
                } else {
                    self.operand_to_reg(&args[0], "rax");
                    "rax"
                };
                let b_name = if let Some(r) = self.operand_reg(&args[1]) {
                    super::emit::phys_reg_name(r)
                } else {
                    self.operand_to_reg(&args[1], "rdx");
                    "rdx"
                };
                let off_name = if let Some(r) = self.operand_reg(&off_operand) {
                    super::emit::phys_reg_name(r)
                } else {
                    self.operand_to_reg(&off_operand, "rsi");
                    "rsi"
                };

                if disp == 0 {
                    self.state.emit_fmt(format_args!(
                        "    vmovupd (%{},%{}), %ymm0",
                        c_name, off_name
                    ));
                    self.state.emit_fmt(format_args!(
                        "    vfmadd231pd (%{},%{}), %ymm1, %ymm0",
                        b_name, off_name
                    ));
                    self.state.emit_fmt(format_args!(
                        "    vmovupd %ymm0, (%{},%{})",
                        c_name, off_name
                    ));
                } else {
                    self.state.emit_fmt(format_args!(
                        "    vmovupd {}(%{},%{}), %ymm0",
                        disp, c_name, off_name
                    ));
                    self.state.emit_fmt(format_args!(
                        "    vfmadd231pd {}(%{},%{}), %ymm1, %ymm0",
                        disp, b_name, off_name
                    ));
                    self.state.emit_fmt(format_args!(
                        "    vmovupd %ymm0, {}(%{},%{})",
                        disp, c_name, off_name
                    ));
                }

                self.state.reg_cache.invalidate_all();
            }

            // --- Vector loads for reduction patterns ---
            IntrinsicOp::LoadF64x4 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                // Load 4 packed doubles: vmovupd (%base + %offset), %ymm0
                if let Some(dptr) = dest_ptr {
                    self.value_to_reg(dptr, "rdx"); // Load dest FIRST into %rdx
                    self.operand_to_reg(&args[0], "rax"); // base pointer
                    self.operand_to_reg(&args[1], "rcx"); // byte offset
                    self.state.emit("    vmovupd (%rax,%rcx), %ymm0");
                    self.state.emit("    vmovupd %ymm0, (%rdx)"); // Store to %rdx
                }
            }
            IntrinsicOp::LoadF64x2 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                // Load 2 packed doubles: movupd (%base + %offset), %xmm0
                if let Some(dptr) = dest_ptr {
                    self.value_to_reg(dptr, "rdx"); // Load dest FIRST into %rdx
                    self.operand_to_reg(&args[0], "rax");
                    self.operand_to_reg(&args[1], "rcx");
                    self.state.emit("    movupd (%rax,%rcx), %xmm0");
                    self.state.emit("    movupd %xmm0, (%rdx)"); // Store to %rdx
                }
            }
            IntrinsicOp::LoadI32x8 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                // Load 8 packed ints: vmovdqu (%base + %offset), %ymm0
                if let Some(dptr) = dest_ptr {
                    self.value_to_reg(dptr, "rdx"); // Load dest FIRST into %rdx
                    self.operand_to_reg(&args[0], "rax");
                    self.operand_to_reg(&args[1], "rcx");
                    self.state.emit("    vmovdqu (%rax,%rcx), %ymm0");
                    self.state.emit("    vmovdqu %ymm0, (%rdx)"); // Store to %rdx
                }
            }
            IntrinsicOp::LoadI32x4 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                // Load 4 packed ints: movdqu (%base + %offset), %xmm0
                if let Some(dptr) = dest_ptr {
                    self.value_to_reg(dptr, "rdx"); // Load dest FIRST into %rdx
                    self.operand_to_reg(&args[0], "rax");
                    self.operand_to_reg(&args[1], "rcx");
                    self.state.emit("    movdqu (%rax,%rcx), %xmm0");
                    self.state.emit("    movdqu %xmm0, (%rdx)"); // Store to %rdx
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
                self.state.emit("    vmovupd (%rax), %ymm0"); // Load 4 doubles
                self.state.emit("    vextractf128 $1, %ymm0, %xmm1"); // Extract upper 128 bits
                self.state.emit("    vaddpd %xmm1, %xmm0, %xmm0"); // Add upper + lower (4→2)
                self.state.emit("    vunpckhpd %xmm0, %xmm0, %xmm1"); // Shuffle element 1 to position 0
                self.state.emit("    vaddsd %xmm1, %xmm0, %xmm0"); // Final scalar add (2→1)
                self.state.emit("    vmovq %xmm0, %rax"); // Extract to GPR
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
            }
            IntrinsicOp::HorizontalAddF64x2 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                // Reduce 2×F64 → 1×F64
                self.operand_to_reg(&args[0], "rax");
                self.state.emit("    movupd (%rax), %xmm0"); // Load {lo, hi}
                self.state.emit("    movapd %xmm0, %xmm1"); // copy
                self.state.emit("    unpckhpd %xmm0, %xmm1"); // xmm1 = {hi, hi}
                self.state.emit("    addsd %xmm1, %xmm0"); // xmm0.lo = lo + hi
                self.state.emit("    movq %xmm0, %rax"); // Extract to GPR
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
            }
            IntrinsicOp::HorizontalAddI32x8 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                // Reduce 8×I32 → 1×I32
                self.operand_to_reg(&args[0], "rax");
                self.state.emit("    vmovdqu (%rax), %ymm0"); // Load 8 ints
                self.state.emit("    vextracti128 $1, %ymm0, %xmm1"); // Extract upper 128 (8→4)
                self.state.emit("    vpaddd %xmm1, %xmm0, %xmm0"); // Add halves (8→4)
                self.state.emit("    vpsrldq $8, %xmm0, %xmm1"); // Shift 8 bytes (4→2)
                self.state.emit("    vpaddd %xmm1, %xmm0, %xmm0"); // Add (4→2)
                self.state.emit("    vpsrldq $4, %xmm0, %xmm1"); // Shift 4 bytes (2→1)
                self.state.emit("    vpaddd %xmm1, %xmm0, %xmm0"); // Add (2→1)
                self.state.emit("    vmovd %xmm0, %eax"); // Extract to GPR
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
            }
            IntrinsicOp::HorizontalAddI32x4 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                // Reduce 4×I32 → 1×I32
                self.operand_to_reg(&args[0], "rax");
                self.state.emit("    movdqu (%rax), %xmm0"); // Load 4 ints
                self.state.emit("    movdqa %xmm0, %xmm1"); // copy
                self.state.emit("    psrldq $8, %xmm1"); // xmm1 = {0,0,a,b}
                self.state.emit("    paddd %xmm1, %xmm0"); // Add (4→2)
                self.state.emit("    movdqa %xmm0, %xmm1"); // copy
                self.state.emit("    psrldq $4, %xmm1"); // xmm1 = {0,a,b,a+c}
                self.state.emit("    paddd %xmm1, %xmm0"); // Add (2→1)
                self.state.emit("    movd %xmm0, %eax"); // Extract to GPR
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
            }

            // --- Register-based vector operations (SSA-friendly) ---
            IntrinsicOp::VecLoadF64x4 => {
                // %dest_vec = load_vector(base_ptr, offset) - AVX2 4×F64.
                // Store through avx_store_dest so a single-use result can be
                // deferred and folded by the next VecAdd/VecMul. Reuse
                // register-allocated base/offset GPRs (reduction hot loops).
                // An optional third argument is a stencil-tap displacement.
                // An XMM-homed destination loads directly into its home.
                let disp = Self::vec_disp_arg(args, 2);
                let mem = self.vec_mem_operand(&args[0], &args[1], disp);
                self.state.dirty_upper_ymm = true;
                let mut loaded_home = false;
                if let Some(d) = dest {
                    if let Some(&reg) = self.reg_assignments.get(&d.0) {
                        if is_xmm_reg(reg) {
                            let name = phys_reg_name_256(reg);
                            self.state
                                .emit_fmt(format_args!("    vmovupd {}, %{}", mem, name));
                            self.state.vector_values.insert(d.0);
                            self.state.vec_live_regs.insert(d.0, name);
                            self.state.vec_last_store_val = Some(d.0);
                            self.state.vec_last_store_reg = true;
                            self.state.vec_last_store_reg_name = Some(name);
                            loaded_home = true;
                        }
                    }
                }
                if !loaded_home {
                    // A deferred result may still sit in %ymm0 (the defer
                    // analysis lets VLFOLD-elided loads be crossed; when the
                    // elision bails at emit time this load must not clobber
                    // the pending value): commit it first.
                    self.flush_pending_vec_store_impl();
                    self.state
                        .emit_fmt(format_args!("    vmovupd {}, %ymm0", mem));
                }
                if let Some(d) = dest {
                    self.state.vector_values.insert(d.0);
                    if !loaded_home {
                        self.avx_store_dest(d);
                    }
                }
            }
            IntrinsicOp::VecLoadI64x2 => {
                // Load two I64 lanes (movdqu). Same addressing as VecLoadF64x2.
                let (base, index) = self.vec_load_addr_regs(&args[0], &args[1]);
                match &index {
                    Some(ix) => self
                        .state
                        .emit_fmt(format_args!("    movdqu (%{},%{}), %xmm0", base, ix)),
                    None => self
                        .state
                        .emit_fmt(format_args!("    movdqu (%{}), %xmm0", base)),
                }
                if let Some(d) = dest {
                    self.state.vector_values.insert(d.0);
                    self.sse_store_dest(d, "xmm0");
                }
            }
            IntrinsicOp::VecLoadF64x2 => {
                // %dest_vec = load_vector(base_ptr, offset) - SSE2 2×F64.
                // An optional third argument is a stencil-tap displacement.
                // An XMM-homed destination loads directly into its home.
                let disp = Self::vec_disp_arg(args, 2);
                let mem = self.vec_mem_operand(&args[0], &args[1], disp);
                let mut loaded_home = false;
                if let Some(d) = dest {
                    if let Some(&reg) = self.reg_assignments.get(&d.0) {
                        if is_xmm_reg(reg) {
                            let name = phys_reg_name(reg);
                            self.state
                                .emit_fmt(format_args!("    movupd {}, %{}", mem, name));
                            self.state.vector_values.insert(d.0);
                            self.state.vec_live_regs.insert(d.0, name);
                            self.state.vec_last_store_val = Some(d.0);
                            self.state.vec_last_store_reg = true;
                            self.state.vec_last_store_reg_name = Some(name);
                            loaded_home = true;
                        }
                    }
                }
                if !loaded_home {
                    self.state
                        .emit_fmt(format_args!("    movupd {}, %xmm0", mem));
                }
                if let Some(d) = dest {
                    self.state.vector_values.insert(d.0);
                    if !loaded_home {
                        self.sse_store_dest(d, "xmm0");
                    }
                }
            }
            IntrinsicOp::VecLoadI32x8 => {
                // Defer-aware store; reuse register-allocated base/offset.
                // Commit a pending deferred %ymm0 result before clobbering
                // the scratch register (see VecLoadF64x4).
                self.flush_pending_vec_store_impl();
                let (base, index) = self.vec_load_addr_regs(&args[0], &args[1]);
                match &index {
                    Some(ix) => self
                        .state
                        .emit_fmt(format_args!("    vmovdqu (%{},%{}), %ymm0", base, ix)),
                    None => self
                        .state
                        .emit_fmt(format_args!("    vmovdqu (%{}), %ymm0", base)),
                }
                if let Some(d) = dest {
                    self.state.vector_values.insert(d.0);
                    self.avx_store_dest(d);
                }
            }
            IntrinsicOp::VecLoadI32x4 => {
                let (base, index) = self.vec_load_addr_regs(&args[0], &args[1]);
                match &index {
                    Some(ix) => self
                        .state
                        .emit_fmt(format_args!("    movdqu (%{},%{}), %xmm0", base, ix)),
                    None => self
                        .state
                        .emit_fmt(format_args!("    movdqu (%{}), %xmm0", base)),
                }
                if let Some(d) = dest {
                    self.state.vector_values.insert(d.0);
                    self.sse_store_dest(d, "xmm0");
                }
            }

            IntrinsicOp::VecAddF64x4 => {
                // route through the defer-aware, memory-operand-folding
                // emitter so single-use loads fold into the add (vaddpd slot,
                // %ymm1, %ymm0) instead of load+load+add round-trips.
                if let Some(d) = dest {
                    self.emit_avx_binary_256(d, args, "vaddpd", true);
                }
            }
            IntrinsicOp::VecAddF64x2 => {
                if let Some(d) = dest {
                    self.emit_sse_binary_128(d, args, "addpd");
                }
            }
            IntrinsicOp::VecAddI64x2 => {
                if let Some(d) = dest {
                    self.emit_sse_binary_128(d, args, "paddq");
                }
            }
            IntrinsicOp::VecWidenAddI32x4ToI64x2 => {
                // dest(I64x2 accumulator) += sext(load4×I32(base, off)).
                // args = [accumulator, base, byte_offset].
                //
                // Lane math (full I64 precision per lane), v12 Fix D:
                //   xmm0 = 4×I32 {a0,a1,a2,a3}
                //   vpmovsxdq xmm0 → xmm1   = {sext(a0), sext(a1)}   (low half)
                //   vpunpckhqdq xmm0,xmm0,xmm0 = {a2,a3,a2,a3} (in-place, frees xmm2)
                //   vpmovsxdq xmm0 → xmm0   = {sext(a2), sext(a3)}   (high half, in-place)
                //   vpaddq xmm0, xmm1, xmm1  = per-lane I64 partial sums
                //   dst  = vpaddq(acc, xmm1)  — accumulator touched ONLY here.
                // Scratch is confined to the reserved pair xmm0/xmm1; the
                // accumulator's XMM home (xmm2..xmm15) is never clobbered, so
                // the v12 Fix C whitelist can safely keep the accumulator
                // register-resident. (A 256-bit vmovdqu would load EIGHT I32s
                // while the IV advances only four — lanes 4..7 double-counted.)
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                let (base, index) = self.vec_load_addr_regs(&args[1], &args[2]);
                match index {
                    Some(idx) => self
                        .state
                        .emit_fmt(format_args!("    vmovdqu (%{},%{}), %xmm0", base, idx)),
                    None => self
                        .state
                        .emit_fmt(format_args!("    vmovdqu (%{}), %xmm0", base)),
                }
                // Widen low half into xmm1, then in-place shuffle + widen
                // high half into xmm0 (freeing the old xmm0 contents), then
                // sum both halves into xmm1.
                self.state.emit("    vpmovsxdq %xmm0, %xmm1");
                self.state.emit("    vpunpckhqdq %xmm0, %xmm0, %xmm0");
                self.state.emit("    vpmovsxdq %xmm0, %xmm0");
                self.state.emit("    vpaddq %xmm0, %xmm1, %xmm1");
                if let (Some(d), Operand::Value(acc)) = (dest, &args[0]) {
                    let acc_reg = self
                        .reg_assignments
                        .get(&acc.0)
                        .copied()
                        .filter(|r| is_xmm_reg(*r))
                        .map(|r| phys_reg_name(r));
                    let dst_reg = self
                        .reg_assignments
                        .get(&d.0)
                        .copied()
                        .filter(|r| is_xmm_reg(*r))
                        .map(|r| phys_reg_name(r));
                    match (acc_reg, dst_reg) {
                        (Some(a), Some(dst)) => {
                            if dst != a {
                                self.state
                                    .emit_fmt(format_args!("    vmovdqa %{}, %{}", a, dst));
                            }
                            self.state
                                .emit_fmt(format_args!("    vpaddq %xmm1, %{}, %{}", dst, dst));
                            self.state.vector_values.insert(d.0);
                            self.state.vec_live_regs.insert(d.0, dst);
                            self.state.vec_last_store_val = Some(d.0);
                            self.state.vec_last_store_reg = true;
                            self.state.vec_last_store_reg_name = Some(dst);
                        }
                        _ => {
                            // Slot-homed accumulator/dest: xmm0 is free now
                            // (high-half sum already folded into xmm1), so
                            // reuse it for the accumulator round-trip — never
                            // touch an allocatable XMM (xmm2..xmm15).
                            if let Some(slot) = self.state.get_slot(acc.0) {
                                self.state
                                    .out
                                    .emit_instr_rbp_reg("    vmovdqu", slot.0, "xmm0");
                                self.state.emit("    vpaddq %xmm1, %xmm0, %xmm0");
                                if let Some(dslot) = self.state.get_slot(d.0) {
                                    self.state.out.emit_instr_reg_rbp(
                                        "    vmovdqu",
                                        "xmm0",
                                        dslot.0,
                                    );
                                }
                            } else {
                                // No slot, no register (dead acc): fold in-place.
                                self.state.emit("    vpaddq %xmm1, %xmm1, %xmm1");
                            }
                        }
                    }
                }
            }
            IntrinsicOp::VecWidenMaskedAddI32x4ToI64x2 => {
                // dest(I64x2 accumulator) += sext(load4×I32(base, off))
                // where lane > guard_rhs. args = [acc, base, off, guard_rhs].
                //
                // v12 Fix D lane math (scratch confined to xmm0/xmm1):
                //   vmovdqu (mem)             → xmm0 = {a0,a1,a2,a3}  (values)
                //   <broadcast rhs → xmm1>
                //   vpcmpgtd xmm1,xmm0 → xmm1 = {m0,m1,m2,m3} (0/-1 per lane)
                //   vpand xmm1,xmm0 → xmm0    = {a0&m0,a1&m1,a2&m2,a3&m3} (I32)
                //   vpmovsxdq xmm0 → xmm1     = {sext(a0&m0), sext(a1&m1)}
                //   vpunpckhqdq xmm0,xmm0,xmm0 = {a2&m2,a3&m3,..} (in-place)
                //   vpmovsxdq xmm0 → xmm0     = {sext(a2&m2), sext(a3&m3)}
                //   vpaddq xmm0,xmm1 → xmm1   = per-lane I64 partial sums
                //   dst = vpaddq(acc, xmm1)
                // Masking at I32 (before widen) is equivalent to masking at
                // I64 (after widen) ONLY because the mask is 0/-1 from
                // vpcmpgtd — sext(a & m) == sext(a) & sext(m) when m∈{0,-1}.
                // This confines scratch to xmm0/xmm1 so the accumulator's
                // XMM home (xmm2..xmm15) is never clobbered (v12 Fix C safe).
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                let (base, index) = self.vec_load_addr_regs(&args[1], &args[2]);
                match index {
                    Some(idx) => self
                        .state
                        .emit_fmt(format_args!("    vmovdqu (%{},%{}), %xmm0", base, idx)),
                    None => self
                        .state
                        .emit_fmt(format_args!("    vmovdqu (%{}), %xmm0", base)),
                }
                // Build the mask in xmm1 = broadcast(guard_rhs).
                match &args[3] {
                    Operand::Const(c) if c.to_i64() == Some(0) => {
                        self.state.emit("    vpxor %xmm1, %xmm1, %xmm1");
                    }
                    Operand::Const(c) => {
                        if let Some(v) = c.to_i64() {
                            self.state
                                .emit_fmt(format_args!("    movl ${}, %eax", v as i32));
                        } else {
                            self.state.emit("    xorl %eax, %eax");
                        }
                        self.state.emit("    vmovd %eax, %xmm1");
                        self.state.emit("    vpshufd $0, %xmm1, %xmm1");
                    }
                    op => {
                        // Scalar value operand: materialize into %eax.
                        self.operand_to_reg(op, "rax");
                        self.state.emit("    vmovd %eax, %xmm1");
                        self.state.emit("    vpshufd $0, %xmm1, %xmm1");
                    }
                }
                // mask = lanes > rhs (AT&T: vpcmpgtd src2=rhs, src1=lanes).
                self.state.emit("    vpcmpgtd %xmm1, %xmm0, %xmm1");
                // Apply mask to I32 values in-place, then widen-then-sum
                // exactly like the non-masked path (xmm0/xmm1 only).
                self.state.emit("    vpand %xmm1, %xmm0, %xmm0");
                self.state.emit("    vpmovsxdq %xmm0, %xmm1");
                self.state.emit("    vpunpckhqdq %xmm0, %xmm0, %xmm0");
                self.state.emit("    vpmovsxdq %xmm0, %xmm0");
                self.state.emit("    vpaddq %xmm0, %xmm1, %xmm1");
                if let (Some(d), Operand::Value(acc)) = (dest, &args[0]) {
                    let acc_reg = self
                        .reg_assignments
                        .get(&acc.0)
                        .copied()
                        .filter(|r| is_xmm_reg(*r))
                        .map(|r| phys_reg_name(r));
                    let dst_reg = self
                        .reg_assignments
                        .get(&d.0)
                        .copied()
                        .filter(|r| is_xmm_reg(*r))
                        .map(|r| phys_reg_name(r));
                    match (acc_reg, dst_reg) {
                        (Some(a), Some(dst)) => {
                            if dst != a {
                                self.state
                                    .emit_fmt(format_args!("    vmovdqa %{}, %{}", a, dst));
                            }
                            self.state
                                .emit_fmt(format_args!("    vpaddq %xmm1, %{}, %{}", dst, dst));
                            self.state.vector_values.insert(d.0);
                            self.state.vec_live_regs.insert(d.0, dst);
                            self.state.vec_last_store_val = Some(d.0);
                            self.state.vec_last_store_reg = true;
                            self.state.vec_last_store_reg_name = Some(dst);
                        }
                        _ => {
                            // Slot-homed: reuse xmm0 (free after the fold
                            // into xmm1) for the accumulator round-trip.
                            if let Some(slot) = self.state.get_slot(acc.0) {
                                self.state
                                    .out
                                    .emit_instr_rbp_reg("    vmovdqu", slot.0, "xmm0");
                                self.state.emit("    vpaddq %xmm1, %xmm0, %xmm0");
                                if let Some(dslot) = self.state.get_slot(d.0) {
                                    self.state.out.emit_instr_reg_rbp(
                                        "    vmovdqu",
                                        "xmm0",
                                        dslot.0,
                                    );
                                }
                            } else {
                                self.state.emit("    vpaddq %xmm1, %xmm1, %xmm1");
                            }
                        }
                    }
                }
            }
            IntrinsicOp::VecMaskedAddI32x8 => {
                // dest(I32x8 accumulator) += lanes(load8×I32(base, off))
                // where lane > guard_rhs.  args = [acc, base, off,
                // guard_rhs].  Equal-width sibling of the masked widening
                // path:
                //   vmovdqu (mem)            → ymm0 = {a0..a7}
                //   <broadcast rhs → ymm1>
                //   vpcmpgtd ymm1, ymm0 → ymm1 = {m0..m7} (0/-1 per lane)
                //   vpand ymm1, ymm0 → ymm0   = {a&m}  (zero for lanes ≤ rhs)
                //   vpaddd ymm0, acc → dest
                // Scratch confined to ymm0/ymm1 so a register-homed I32x8
                // accumulator (ymm2..ymm15) is never clobbered — the same
                // discipline as the widening masked path.  Masking at I32 is
                // exact: the mask is 0/-1 from vpcmpgtd (lanes > rhs), and
                // vpand zero-masks strictly-below lanes before the fold, so
                // the guarded scalar semantics (skip lane when lane ≤ rhs)
                // hold bit-for-bit.
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                let (base, index) = self.vec_load_addr_regs(&args[1], &args[2]);
                match index {
                    Some(idx) => self
                        .state
                        .emit_fmt(format_args!("    vmovdqu (%{},%{}), %ymm0", base, idx)),
                    None => self
                        .state
                        .emit_fmt(format_args!("    vmovdqu (%{}), %ymm0", base)),
                }
                // Build the mask in ymm1 = broadcast(guard_rhs).
                match &args[3] {
                    Operand::Const(c) if c.to_i64() == Some(0) => {
                        self.state.emit("    vpxor %ymm1, %ymm1, %ymm1");
                    }
                    Operand::Const(c) => {
                        if let Some(v) = c.to_i64() {
                            self.state
                                .emit_fmt(format_args!("    movl ${}, %eax", v as i32));
                        } else {
                            self.state.emit("    xorl %eax, %eax");
                        }
                        self.state.emit("    vmovd %eax, %xmm1");
                        self.state.emit("    vpbroadcastd %xmm1, %ymm1");
                    }
                    op => {
                        // Scalar value operand: materialize into %eax.
                        self.operand_to_reg(op, "rax");
                        self.state.emit("    vmovd %eax, %xmm1");
                        self.state.emit("    vpbroadcastd %xmm1, %ymm1");
                    }
                }
                // mask = lanes > rhs (AT&T: vpcmpgtd src2=rhs, src1=lanes).
                self.state.emit("    vpcmpgtd %ymm1, %ymm0, %ymm1");
                // Apply mask to the loaded lanes in-place, then fold.
                self.state.emit("    vpand %ymm1, %ymm0, %ymm0");
                if let (Some(d), Operand::Value(acc)) = (dest, &args[0]) {
                    let acc_reg = self
                        .reg_assignments
                        .get(&acc.0)
                        .copied()
                        .filter(|r| is_xmm_reg(*r))
                        .map(|r| phys_reg_name_256(r));
                    let dst_reg = self
                        .reg_assignments
                        .get(&d.0)
                        .copied()
                        .filter(|r| is_xmm_reg(*r))
                        .map(|r| phys_reg_name_256(r));
                    match (acc_reg, dst_reg) {
                        (Some(a), Some(dst)) => {
                            if dst != a {
                                self.state
                                    .emit_fmt(format_args!("    vmovdqa %{}, %{}", a, dst));
                            }
                            self.state
                                .emit_fmt(format_args!("    vpaddd %ymm0, %{}, %{}", dst, dst));
                            self.state.vector_values.insert(d.0);
                            self.state.vec_live_regs.insert(d.0, dst);
                            self.state.vec_last_store_val = Some(d.0);
                            self.state.vec_last_store_reg = true;
                            self.state.vec_last_store_reg_name = Some(dst);
                        }
                        _ => {
                            // Slot-homed: ymm0 holds the masked lanes; load the
                            // accumulator into ymm1 (mask consumed), fold,
                            // round-trip through the slot.
                            if let Some(slot) = self.state.get_slot(acc.0) {
                                self.state
                                    .out
                                    .emit_instr_rbp_reg("    vmovdqu", slot.0, "ymm1");
                                self.state.emit("    vpaddd %ymm0, %ymm1, %ymm1");
                                if let Some(dslot) = self.state.get_slot(d.0) {
                                    self.state.out.emit_instr_reg_rbp(
                                        "    vmovdqu",
                                        "ymm1",
                                        dslot.0,
                                    );
                                }
                            } else {
                                self.state.emit("    vpaddd %ymm0, %ymm0, %ymm0");
                            }
                        }
                    }
                }
            }
            IntrinsicOp::VecMulI64x2 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                self.sse_load_arg(&args[0], "xmm0");
                self.sse_load_arg(&args[1], "xmm1");
                self.state.emit("    movq %xmm0, %rax");
                self.state.emit("    movq %xmm1, %rcx");
                self.state.emit("    imulq %rcx, %rax");
                self.state.emit("    movq %rax, %xmm2");
                self.state.emit("    pshufd $0xee, %xmm0, %xmm0");
                self.state.emit("    pshufd $0xee, %xmm1, %xmm1");
                self.state.emit("    movq %xmm0, %rax");
                self.state.emit("    movq %xmm1, %rcx");
                self.state.emit("    imulq %rcx, %rax");
                self.state.emit("    movq %rax, %xmm0");
                self.state.emit("    punpcklqdq %xmm0, %xmm2");
                self.state.emit("    movdqa %xmm2, %xmm0");
                if let Some(d) = dest {
                    self.state.vector_values.insert(d.0);
                    self.sse_store_dest(d, "xmm0");
                }
            }
            IntrinsicOp::VecStoreI64x2 => {
                self.emit_vec_store_addr(args, dest_ptr, "movdqu", "xmm0");
            }
            IntrinsicOp::VecBroadcastI64x2 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                match &args[0] {
                    Operand::Value(v) => {
                        if let Some(&reg) = self.reg_assignments.get(&v.0) {
                            if !is_xmm_reg(reg) {
                                self.state.emit_fmt(format_args!(
                                    "    movq %{}, %xmm0",
                                    phys_reg_name(reg)
                                ));
                            } else {
                                self.state.emit_fmt(format_args!(
                                    "    movdqa %{}, %xmm0",
                                    phys_reg_name(reg)
                                ));
                            }
                        } else if let Some(slot) = self.state.get_slot(v.0) {
                            self.state
                                .out
                                .emit_instr_rbp_reg("    movq", slot.0 as i64, "xmm0");
                        } else {
                            self.operand_to_reg(&args[0], "rax");
                            self.state.emit("    movq %rax, %xmm0");
                        }
                    }
                    _ => {
                        self.operand_to_reg(&args[0], "rax");
                        self.state.emit("    movq %rax, %xmm0");
                    }
                }
                self.state.emit("    unpcklpd %xmm0, %xmm0");
                if let Some(d) = dest {
                    self.state.vector_values.insert(d.0);
                    self.sse_store_dest(d, "xmm0");
                }
            }
            IntrinsicOp::VecLoadI64x4
            | IntrinsicOp::VecAddI64x4
            | IntrinsicOp::VecHorizontalAddI64x4
            | IntrinsicOp::VecZeroI64x4 => {
                let _ = (dest, dest_ptr, args);
            }
            IntrinsicOp::VecMulF64x4 => {
                if let Some(d) = dest {
                    self.emit_avx_binary_256(d, args, "vmulpd", true);
                }
            }
            IntrinsicOp::VecMulF64x2 => {
                if let Some(d) = dest {
                    self.emit_sse_binary_128(d, args, "mulpd");
                }
            }
            IntrinsicOp::VecSubF64x4 => {
                if let Some(d) = dest {
                    // Non-commutative: preserve operand order.
                    self.emit_avx_binary_256(d, args, "vsubpd", false);
                }
            }
            IntrinsicOp::VecSubF64x2 => {
                if let Some(d) = dest {
                    self.emit_sse_binary_128(d, args, "subpd");
                }
            }
            IntrinsicOp::VecSubF32x8 => {
                if let Some(d) = dest {
                    self.emit_avx_binary_256(d, args, "vsubps", false);
                }
            }
            IntrinsicOp::VecSubF32x4 => {
                if let Some(d) = dest {
                    self.emit_sse_binary_128(d, args, "subps");
                }
            }
            IntrinsicOp::VecDivF64x4 => {
                if let Some(d) = dest {
                    self.emit_avx_binary_256(d, args, "vdivpd", false);
                }
            }
            IntrinsicOp::VecDivF64x2 => {
                if let Some(d) = dest {
                    self.emit_sse_binary_128(d, args, "divpd");
                }
            }
            IntrinsicOp::VecDivF32x8 => {
                if let Some(d) = dest {
                    self.emit_avx_binary_256(d, args, "vdivps", false);
                }
            }
            IntrinsicOp::VecDivF32x4 => {
                if let Some(d) = dest {
                    self.emit_sse_binary_128(d, args, "divps");
                }
            }
            IntrinsicOp::VecSqrtF64x4 | IntrinsicOp::VecSqrtF32x8 => {
                // Unary AVX: stream the operand through %ymm0.
                if let Some(d) = dest {
                    self.flush_pending_vec_store_impl();
                    self.state.invalidate_vec_peephole();
                    self.avx_load_arg_to(&args[0], "ymm0");
                    let inst = if matches!(op, IntrinsicOp::VecSqrtF64x4) {
                        "vsqrtpd"
                    } else {
                        "vsqrtps"
                    };
                    self.state
                        .emit_fmt(format_args!("    {} %ymm0, %ymm0", inst));
                    self.state.vec_last_store_val = Some(d.0);
                    self.state.vec_last_store_reg = true;
                    self.state.vec_last_store_reg_name = Some("ymm0");
                    self.avx_store_dest(d);
                }
            }
            IntrinsicOp::VecSqrtF64x2 | IntrinsicOp::VecSqrtF32x4 => {
                // Unary SSE2: stream the operand through %xmm0.
                if let Some(d) = dest {
                    self.flush_pending_vec_store_impl();
                    self.state.invalidate_vec_peephole();
                    self.sse_load_arg(&args[0], "xmm0");
                    let inst = if matches!(op, IntrinsicOp::VecSqrtF64x2) {
                        "sqrtpd"
                    } else {
                        "sqrtps"
                    };
                    self.state
                        .emit_fmt(format_args!("    {} %xmm0, %xmm0", inst));
                    self.sse_store_dest(d, "xmm0");
                }
            }
            IntrinsicOp::VecFmaF64x4 => {
                if let Some(d) = dest {
                    self.emit_avx_reduction_fma(d, args, "vfmadd231pd");
                }
            }
            IntrinsicOp::VecMaddF64x4 => {
                if let Some(d) = dest {
                    self.emit_avx_map_fma(d, args, "vfmadd132pd");
                }
            }
            IntrinsicOp::VecAddI32x8 | IntrinsicOp::VecAddI32x4 => {
                // defer-aware emitters (vpaddd is 3-op VEX, paddd is 2-op).
                if let Some(d) = dest {
                    match op {
                        IntrinsicOp::VecAddI32x8 => {
                            self.emit_avx_binary_256(d, args, "vpaddd", true)
                        }
                        IntrinsicOp::VecAddI32x4 => self.emit_sse_binary_128(d, args, "paddd"),
                        _ => unreachable!(),
                    }
                }
            }
            IntrinsicOp::VecMulI32x4 => {
                if let Some(d) = dest {
                    self.emit_sse_binary_128(d, args, "pmulld");
                }
            }
            IntrinsicOp::VecMulI32x8 => {
                if let Some(d) = dest {
                    self.emit_avx_binary_256(d, args, "vpmulld", true);
                }
            }
            IntrinsicOp::VecMaxI32x8 => {
                // Lane-wise signed max of two 8×I32 vectors (vpmaxsd). Used by
                // the max-reduction vectorizer: the accumulator (running max)
                // and the freshly-loaded 8 lanes fold into a new accumulator.
                if let Some(d) = dest {
                    self.emit_avx_binary_256(d, args, "vpmaxsd", true);
                }
            }
            IntrinsicOp::VecBroadcastI32x4 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                self.operand_to_reg(&args[0], "rax");
                self.state.emit("    movd %eax, %xmm0");
                self.state.emit("    pshufd $0x00, %xmm0, %xmm0");
                if let Some(d) = dest {
                    self.state.vector_values.insert(d.0);
                    self.sse_store_dest(d, "xmm0");
                }
            }
            IntrinsicOp::VecBroadcastI32x8 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                self.operand_to_reg(&args[0], "rax");
                self.state.emit("    movd %eax, %xmm0");
                self.state.emit("    vpbroadcastd %xmm0, %ymm0");
                if let Some(d) = dest {
                    self.state.vector_values.insert(d.0);
                    self.avx_store_dest(d);
                    self.flush_pending_vec_store_impl();
                }
            }
            IntrinsicOp::VecStoreI32x4 => {
                // Peek register residency BEFORE invalidating the peephole.
                let in_reg = matches!(&args[0], Operand::Value(v)
                    if self.state.sse_last_store_reg && self.state.sse_last_store_val == Some(v.0));
                if !in_reg {
                    self.flush_pending_vec_store_impl();
                    if let Operand::Value(v) = &args[0] {
                        if let Some(addr) = self.state.resolve_slot_addr(v.0) {
                            if let crate::backend::state::SlotAddr::Direct(slot) = addr {
                                self.state.emit_fmt(format_args!(
                                    "    movdqu {}, %xmm0",
                                    self.slot_ref(slot.0)
                                ));
                            }
                        }
                    }
                } else if let Operand::Value(v) = &args[0] {
                    if self.state.pending_vec_store.map(|(p, _, _)| p) == Some(v.0) {
                        self.state.pending_vec_store = None;
                    }
                }
                self.state.invalidate_vec_peephole();
                self.emit_vec_store_addr(args, dest_ptr, "movdqu", "xmm0");
            }
            IntrinsicOp::VecStoreI32x8 => {
                // Peek register residency BEFORE invalidating the peephole —
                // otherwise every map/store pair pays a dead vmovdqu round-trip.
                let in_reg = matches!(&args[0], Operand::Value(v)
                    if self.state.vec_last_store_reg && self.state.vec_last_store_val == Some(v.0));
                if !in_reg {
                    self.flush_pending_vec_store_impl();
                    if let Operand::Value(v) = &args[0] {
                        if let Some(addr) = self.state.resolve_slot_addr(v.0) {
                            if let crate::backend::state::SlotAddr::Direct(slot) = addr {
                                self.state.emit_fmt(format_args!(
                                    "    vmovdqu {}, %ymm0",
                                    self.slot_ref(slot.0)
                                ));
                            }
                        }
                    }
                } else if let Operand::Value(v) = &args[0] {
                    if self.state.pending_vec_store.map(|(p, _, _)| p) == Some(v.0) {
                        self.state.pending_vec_store = None;
                    }
                }
                self.state.invalidate_vec_peephole();
                self.emit_vec_store_addr(args, dest_ptr, "vmovdqu", "ymm0");
            }

            IntrinsicOp::VecBroadcastF64x4 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                let mut done = false;
                if let Operand::Value(v) = &args[0] {
                    if let Some(addr) = self.state.resolve_slot_addr(v.0) {
                        if let crate::backend::state::SlotAddr::Direct(slot) = addr {
                            self.state.emit_fmt(format_args!(
                                "    vbroadcastsd {}, %ymm0",
                                self.slot_ref(slot.0)
                            ));
                            done = true;
                        }
                    }
                    if !done {
                        if let Some(&reg) = self.reg_assignments.get(&v.0) {
                            if is_xmm_reg(reg) {
                                self.state.emit_fmt(format_args!(
                                    "    vbroadcastsd %{}, %ymm0",
                                    phys_reg_name(reg)
                                ));
                                done = true;
                            }
                        }
                    }
                }
                if !done {
                    self.emit_fp_operand_to_xmm(&args[0], IrType::F64, "xmm0");
                    self.state.emit("    vbroadcastsd %xmm0, %ymm0");
                }
                if let Some(d) = dest {
                    self.state.vector_values.insert(d.0);
                    self.avx_store_dest(d);
                    self.flush_pending_vec_store_impl();
                }
            }
            IntrinsicOp::VecBroadcastF64x2 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                self.emit_fp_operand_to_xmm(&args[0], IrType::F64, "xmm0");
                self.state.emit("    unpcklpd %xmm0, %xmm0");
                if let Some(d) = dest {
                    self.state.vector_values.insert(d.0);
                    self.sse_store_dest(d, "xmm0");
                }
            }
            IntrinsicOp::VecBroadcastF32x8 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                // scalar f32 in args[0] -> all 8 lanes via vbroadcastss.
                // Prefer memory form; fall back to XMM then broadcast.
                let mut done = false;
                if let Operand::Value(v) = &args[0] {
                    if let Some(addr) = self.state.resolve_slot_addr(v.0) {
                        if let crate::backend::state::SlotAddr::Direct(slot) = addr {
                            self.state.emit_fmt(format_args!(
                                "    vbroadcastss {}, %ymm0",
                                self.slot_ref(slot.0)
                            ));
                            done = true;
                        }
                    }
                    // Value may already live in an XMM home from ParamRef/float path.
                    if !done {
                        if let Some(&reg) = self.reg_assignments.get(&v.0) {
                            if is_xmm_reg(reg) {
                                let name = phys_reg_name(reg);
                                self.state
                                    .emit_fmt(format_args!("    vbroadcastss %{}, %ymm0", name));
                                done = true;
                            }
                        }
                    }
                }
                if !done {
                    self.emit_fp_operand_to_xmm(&args[0], IrType::F32, "xmm0");
                    self.state.emit("    vbroadcastss %xmm0, %ymm0");
                }
                if let Some(d) = dest {
                    self.state.vector_values.insert(d.0);
                    self.avx_store_dest(d);
                    // Consecutive broadcasts share %ymm0; materialise now.
                    self.flush_pending_vec_store_impl();
                }
            }
            IntrinsicOp::VecBroadcastF32x4 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                self.emit_fp_operand_to_xmm(&args[0], IrType::F32, "xmm0");
                self.state.emit("    shufps $0x00, %xmm0, %xmm0");
                if let Some(d) = dest {
                    self.state.vector_values.insert(d.0);
                    self.sse_store_dest(d, "xmm0");
                }
            }
            IntrinsicOp::VecStoreF32x8 => {
                let in_reg = matches!(&args[0], Operand::Value(v)
                    if self.state.vec_last_store_reg && self.state.vec_last_store_val == Some(v.0));
                if !in_reg {
                    self.flush_pending_vec_store_impl();
                    if let Operand::Value(v) = &args[0] {
                        if let Some(addr) = self.state.resolve_slot_addr(v.0) {
                            if let crate::backend::state::SlotAddr::Direct(slot) = addr {
                                self.state.emit_fmt(format_args!(
                                    "    vmovups {}, %ymm0",
                                    self.slot_ref(slot.0)
                                ));
                            }
                        }
                    }
                } else if let Operand::Value(v) = &args[0] {
                    if self.state.pending_vec_store.map(|(p, _, _)| p) == Some(v.0) {
                        self.state.pending_vec_store = None;
                    }
                }
                self.state.invalidate_vec_peephole();
                self.emit_vec_store_addr(args, dest_ptr, "vmovups", "ymm0");
            }
            IntrinsicOp::VecStoreF32x4 => {
                let in_reg = matches!(&args[0], Operand::Value(v)
                    if self.state.sse_last_store_reg && self.state.sse_last_store_val == Some(v.0));
                if !in_reg {
                    self.flush_pending_vec_store_impl();
                    if let Operand::Value(v) = &args[0] {
                        if let Some(addr) = self.state.resolve_slot_addr(v.0) {
                            if let crate::backend::state::SlotAddr::Direct(slot) = addr {
                                self.state.emit_fmt(format_args!(
                                    "    movups {}, %xmm0",
                                    self.slot_ref(slot.0)
                                ));
                            }
                        }
                    }
                } else if let Operand::Value(v) = &args[0] {
                    if self.state.pending_vec_store.map(|(p, _, _)| p) == Some(v.0) {
                        self.state.pending_vec_store = None;
                    }
                }
                self.state.invalidate_vec_peephole();
                self.emit_vec_store_addr(args, dest_ptr, "movups", "xmm0");
            }
            IntrinsicOp::VecStoreF64x4 => {
                let in_reg = matches!(&args[0], Operand::Value(v)
                    if self.state.vec_last_store_reg && self.state.vec_last_store_val == Some(v.0));
                if !in_reg {
                    self.flush_pending_vec_store_impl();
                    if let Operand::Value(v) = &args[0] {
                        if let Some(addr) = self.state.resolve_slot_addr(v.0) {
                            if let crate::backend::state::SlotAddr::Direct(slot) = addr {
                                self.state.emit_fmt(format_args!(
                                    "    vmovupd {}, %ymm0",
                                    self.slot_ref(slot.0)
                                ));
                            }
                        }
                    }
                } else if let Operand::Value(v) = &args[0] {
                    if self.state.pending_vec_store.map(|(p, _, _)| p) == Some(v.0) {
                        self.state.pending_vec_store = None;
                    }
                }
                self.state.invalidate_vec_peephole();
                self.emit_vec_store_addr(args, dest_ptr, "vmovupd", "ymm0");
            }
            IntrinsicOp::VecStoreF64x2 => {
                let in_reg = matches!(&args[0], Operand::Value(v)
                    if self.state.sse_last_store_reg && self.state.sse_last_store_val == Some(v.0));
                if !in_reg {
                    self.flush_pending_vec_store_impl();
                    if let Operand::Value(v) = &args[0] {
                        if let Some(addr) = self.state.resolve_slot_addr(v.0) {
                            if let crate::backend::state::SlotAddr::Direct(slot) = addr {
                                self.state.emit_fmt(format_args!(
                                    "    movupd {}, %xmm0",
                                    self.slot_ref(slot.0)
                                ));
                            }
                        }
                    }
                } else if let Operand::Value(v) = &args[0] {
                    if self.state.pending_vec_store.map(|(p, _, _)| p) == Some(v.0) {
                        self.state.pending_vec_store = None;
                    }
                }
                self.state.invalidate_vec_peephole();
                self.emit_vec_store_addr(args, dest_ptr, "movupd", "xmm0");
            }
            IntrinsicOp::VecHorizontalAddF64x4 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                // %scalar = horizontal_add(%vec) - AVX2 4×F64 → F64.
                // The generic loader handles both a protected stack home and
                // a width-aware register assignment.
                self.avx_load_arg(&args[0]);
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
                self.sse_load_arg(&args[0], "xmm0");
                self.state.emit("    movapd %xmm0, %xmm1");
                self.state.emit("    unpckhpd %xmm0, %xmm1"); // xmm1 = {hi, hi}
                self.state.emit("    addsd %xmm1, %xmm0"); // xmm0.lo = lo + hi
                self.state.emit("    movq %xmm0, %rax");
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
            }
            IntrinsicOp::VecHorizontalAddI64x2 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                // 2×I64 → I64 horizontal sum
                self.sse_load_arg(&args[0], "xmm0");
                self.state.emit("    pshufd $0xEE, %xmm0, %xmm1"); // xmm1 = {hi, hi}
                self.state.emit("    paddq %xmm1, %xmm0");
                self.state.emit("    movq %xmm0, %rax");
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
            }
            IntrinsicOp::VecHorizontalAddI32x8 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                // %scalar = horizontal_add(%vec) - AVX2 8×I32 → I32.
                // The generic loader handles both a protected stack home and
                // a width-aware register assignment.
                self.avx_load_arg(&args[0]);
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
            IntrinsicOp::VecHorizontalMaxI32x8 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                // %scalar = horizontal_max(%vec) - AVX2 8×I32 → I32.
                // x86 has no single smaxv, so reduce in three vpmaxsd steps.
                // CRITICAL: use vpshufd (lane permute, preserves sign bits)
                // — NOT vpsrldq (byte shift, zero-fills). A zero-fill would
                // compute max(lane, 0) which is WRONG for all-negative data
                // (find_max on negative integers): max(-5, 0) = 0 ≠ -5.
                // vpshufd keeps every lane's real value so the final max is
                // the true signed maximum.
                self.avx_load_arg(&args[0]);
                // Halve 8→4: max of low 128 and high 128.
                self.state.emit("    vextracti128 $1, %ymm0, %xmm1");
                self.state.emit("    vpmaxsd %xmm1, %xmm0, %xmm0");
                // Reduce 4→2: permute lanes {2,3,0,1}, max-merge.
                self.state.emit("    vpshufd $0x4e, %xmm0, %xmm1");
                self.state.emit("    vpmaxsd %xmm1, %xmm0, %xmm0");
                // Reduce 2→1: permute lanes {1,0,3,2}, max-merge.
                self.state.emit("    vpshufd $0xb1, %xmm0, %xmm1");
                self.state.emit("    vpmaxsd %xmm1, %xmm0, %xmm0");
                self.state.emit("    vmovd %xmm0, %eax");
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
            }
            IntrinsicOp::VecHorizontalAddI32x4 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                // %scalar = horizontal_add(%vec) - SSE2 4×I32 → I32
                self.sse_load_arg(&args[0], "xmm0");
                self.state.emit("    movdqa %xmm0, %xmm1");
                self.state.emit("    psrldq $8, %xmm1"); // xmm1 = {0,0,a,b}
                self.state.emit("    paddd %xmm1, %xmm0");
                self.state.emit("    movdqa %xmm0, %xmm1");
                self.state.emit("    psrldq $4, %xmm1"); // xmm1 = {0,a,b,a+c}
                self.state.emit("    paddd %xmm1, %xmm0");
                self.state.emit("    movd %xmm0, %eax");
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
            }

            // ---- F32 reduction vector ops (8-wide AVX2 / 4-wide SSE2) ----
            IntrinsicOp::VecLoadF32x8 | IntrinsicOp::VecLoadF32x4 => {
                // Defer-aware store (single-use result folds into next op);
                // reuse register-allocated base/offset GPRs. An optional
                // third argument is a constant displacement (stencil tap),
                // folded into the SIB operand: `disp(%base,%idx)`.
                //
                // When the destination has an XMM home (defer-overflow or
                // broadcast promotion), load DIRECTLY into the home's YMM
                // name — no `%ymm0` staging move (OP-05a stencils).
                let is8 = matches!(op, IntrinsicOp::VecLoadF32x8);
                let disp = Self::vec_disp_arg(args, 2);
                let mem = self.vec_mem_operand(&args[0], &args[1], disp);
                let mut loaded_home = false;
                if let Some(d) = dest {
                    if let Some(&reg) = self.reg_assignments.get(&d.0) {
                        if is_xmm_reg(reg) {
                            let name = phys_reg_name_256(reg);
                            if is8 {
                                self.state.dirty_upper_ymm = true;
                                self.state
                                    .emit_fmt(format_args!("    vmovups {}, %{}", mem, name));
                            } else {
                                let n128 = phys_reg_name(reg);
                                self.state
                                    .emit_fmt(format_args!("    movups {}, %{}", mem, n128));
                            }
                            self.state.vector_values.insert(d.0);
                            self.state.vec_live_regs.insert(d.0, name);
                            self.state.vec_last_store_val = Some(d.0);
                            self.state.vec_last_store_reg = true;
                            self.state.vec_last_store_reg_name = Some(name);
                            loaded_home = true;
                        }
                    }
                }
                if !loaded_home {
                    // Commit a pending deferred %ymm0 result before
                    // clobbering the scratch register (see VecLoadF64x4).
                    self.flush_pending_vec_store_impl();
                    if is8 {
                        self.state
                            .emit_fmt(format_args!("    vmovups {}, %ymm0", mem));
                    } else {
                        self.state
                            .emit_fmt(format_args!("    movups {}, %xmm0", mem));
                    }
                }
                if let Some(d) = dest {
                    self.state.vector_values.insert(d.0);
                    if !loaded_home {
                        if is8 {
                            self.avx_store_dest(d);
                        } else {
                            self.sse_store_dest(d, "xmm0");
                        }
                    }
                }
            }
            IntrinsicOp::VecAddF32x8 | IntrinsicOp::VecAddF32x4 => {
                if let Some(d) = dest {
                    match op {
                        IntrinsicOp::VecAddF32x8 => {
                            self.emit_avx_binary_256(d, args, "vaddps", true)
                        }
                        IntrinsicOp::VecAddF32x4 => self.emit_sse_binary_128(d, args, "addps"),
                        _ => unreachable!(),
                    }
                }
            }
            IntrinsicOp::VecMulF32x8 | IntrinsicOp::VecMulF32x4 => {
                if let Some(d) = dest {
                    match op {
                        IntrinsicOp::VecMulF32x8 => {
                            self.emit_avx_binary_256(d, args, "vmulps", true)
                        }
                        IntrinsicOp::VecMulF32x4 => self.emit_sse_binary_128(d, args, "mulps"),
                        _ => unreachable!(),
                    }
                }
            }
            IntrinsicOp::VecFmaF32x8 => {
                if let Some(d) = dest {
                    self.emit_avx_reduction_fma(d, args, "vfmadd231ps");
                }
            }
            IntrinsicOp::VecMaddF32x8 => {
                if let Some(d) = dest {
                    self.emit_avx_map_fma(d, args, "vfmadd132ps");
                }
            }
            IntrinsicOp::FixedDistanceF32x8 => {
                if let Some(d) = dest {
                    self.emit_fixed_distance(d, args, false);
                }
            }
            IntrinsicOp::FixedDistanceF64x4 => {
                if let Some(d) = dest {
                    self.emit_fixed_distance(d, args, true);
                }
            }
            IntrinsicOp::VecHorizontalAddF32x8 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                // 8×F32 → F32: cross-lane-safe halving reduction.
                self.avx_load_arg(&args[0]);
                self.state.emit("    vextractf128 $1, %ymm0, %xmm1");
                self.state.emit("    vaddps %xmm1, %xmm0, %xmm0"); // [s0 s1 s2 s3]
                self.state.emit("    vmovshdup %xmm0, %xmm1"); // [s1 s1 s3 s3]
                self.state.emit("    vaddps %xmm1, %xmm0, %xmm0"); // [s0+s1, .., s2+s3, ..]
                self.state.emit("    vshufps $0xAA, %xmm0, %xmm0, %xmm1"); // lanes {2,2,2,2}
                self.state.emit("    vaddss %xmm1, %xmm0, %xmm0"); // (s0+s1)+(s2+s3)
                self.state.emit("    vmovd %xmm0, %eax");
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
            }
            IntrinsicOp::VecHorizontalAddF32x4 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                // 4×F32 → F32 (SSE2-only instruction sequence).
                self.sse_load_arg(&args[0], "xmm0");
                self.state.emit("    movaps %xmm0, %xmm1");
                self.state.emit("    movhlps %xmm0, %xmm1"); // xmm1 = [s2 s3 s2 s3]
                self.state.emit("    addps %xmm1, %xmm0"); // [s0+s2, s1+s3, ..]
                self.state.emit("    movaps %xmm0, %xmm1"); // refresh shuffle source
                self.state.emit("    shufps $0x55, %xmm0, %xmm1"); // [s1+s3 ×4]
                self.state.emit("    addss %xmm1, %xmm0"); // (s0+s2)+(s1+s3)
                self.state.emit("    movd %xmm0, %eax");
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
            }
            IntrinsicOp::VecZeroF32x8 | IntrinsicOp::VecZeroF32x4 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                let is8 = matches!(op, IntrinsicOp::VecZeroF32x8);
                if let Some(d) = dest {
                    self.state.vector_values.insert(d.0);
                    let assigned = self
                        .reg_assignments
                        .get(&d.0)
                        .copied()
                        .filter(|r| is_xmm_reg(*r));
                    if let Some(reg) = assigned {
                        let name = if is8 {
                            phys_reg_name_256(reg)
                        } else {
                            phys_reg_name(reg)
                        };
                        if is8 {
                            self.state.emit_fmt(format_args!(
                                "    vxorps %{}, %{}, %{}",
                                name, name, name
                            ));
                        } else {
                            self.state
                                .emit_fmt(format_args!("    xorps %{}, %{}", name, name));
                        }
                        self.state.vec_live_regs.insert(d.0, name);
                    } else if is8 {
                        self.state.emit("    vxorps %ymm0, %ymm0, %ymm0");
                        self.avx_store_dest(d);
                    } else {
                        self.state.emit("    xorps %xmm0, %xmm0");
                        self.sse_store_dest(d, "xmm0");
                    }
                }
            }
            IntrinsicOp::VecZeroF64x4 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                // %dest_vec = {0.0, 0.0, 0.0, 0.0} - AVX2 4×F64
                if let Some(d) = dest {
                    self.state.vector_values.insert(d.0);
                    if let Some(&reg) = self.reg_assignments.get(&d.0).filter(|r| is_xmm_reg(**r)) {
                        let name = phys_reg_name_256(reg);
                        self.state
                            .emit_fmt(format_args!("    vxorpd %{}, %{}, %{}", name, name, name));
                        self.state.vec_live_regs.insert(d.0, name);
                    } else {
                        self.state.emit("    vxorpd %ymm0, %ymm0, %ymm0");
                        self.avx_store_dest(d);
                    }
                }
            }
            IntrinsicOp::VecZeroF64x2 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                // %dest_vec = {0.0, 0.0} - SSE2 2×F64
                if let Some(d) = dest {
                    self.state.vector_values.insert(d.0);
                    if let Some(&reg) = self.reg_assignments.get(&d.0).filter(|r| is_xmm_reg(**r)) {
                        let name = phys_reg_name(reg);
                        self.state
                            .emit_fmt(format_args!("    xorpd %{}, %{}", name, name));
                        self.state.vec_live_regs.insert(d.0, name);
                    } else {
                        self.state.emit("    xorpd %xmm0, %xmm0");
                        self.sse_store_dest(d, "xmm0");
                    }
                }
            }
            IntrinsicOp::VecZeroI64x2 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                if let Some(d) = dest {
                    self.state.vector_values.insert(d.0);
                    if let Some(&reg) = self.reg_assignments.get(&d.0).filter(|r| is_xmm_reg(**r)) {
                        let name = phys_reg_name(reg);
                        self.state
                            .emit_fmt(format_args!("    pxor %{}, %{}", name, name));
                        self.state.vec_live_regs.insert(d.0, name);
                    } else {
                        self.state.emit("    pxor %xmm0, %xmm0");
                        self.sse_store_dest(d, "xmm0");
                    }
                }
            }
            IntrinsicOp::VecZeroI32x8 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                // %dest_vec = {0, 0, 0, 0, 0, 0, 0, 0} - AVX2 8×I32
                if let Some(d) = dest {
                    self.state.vector_values.insert(d.0);
                    if let Some(&reg) = self.reg_assignments.get(&d.0).filter(|r| is_xmm_reg(**r)) {
                        let name = phys_reg_name_256(reg);
                        self.state
                            .emit_fmt(format_args!("    vpxor %{}, %{}, %{}", name, name, name));
                        self.state.vec_live_regs.insert(d.0, name);
                    } else {
                        self.state.emit("    vpxor %ymm0, %ymm0, %ymm0");
                        self.avx_store_dest(d);
                    }
                }
            }
            IntrinsicOp::VecZeroI32x4 => {
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                // %dest_vec = {0, 0, 0, 0} - SSE2 4×I32
                if let Some(d) = dest {
                    self.state.vector_values.insert(d.0);
                    if let Some(&reg) = self.reg_assignments.get(&d.0).filter(|r| is_xmm_reg(**r)) {
                        let name = phys_reg_name(reg);
                        self.state
                            .emit_fmt(format_args!("    pxor %{}, %{}", name, name));
                        self.state.vec_live_regs.insert(d.0, name);
                    } else {
                        self.state.emit("    pxor %xmm0, %xmm0");
                        self.sse_store_dest(d, "xmm0");
                    }
                }
            }
            // AArch64-only intrinsics must never reach the x86 emitter; fall
            // through to the loud "unhandled" diagnostic below rather than
            // silently no-opping (which would miscompile). This covers
            // Lev's VecSadalp/VecSmlal* widening reductions too — the sadalp
            // transform is gated to the AArch64 pipeline entry.
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

    /// Emit a complete fixed-width squared distance directly to xmm0. The SLP
    /// legality proof guarantees contiguous full vectors and an immediate FP
    /// return; folding the second base into `vsubp*` avoids transient vector
    /// homes and exactly matches the target's three-operand dataflow.
    fn emit_fixed_distance(&mut self, dest: &Value, args: &[Operand], f64_lanes: bool) {
        assert!(args.len() == 2, "fixed distance expects two base pointers");
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
        let is_gpr = |r: PhysReg| (1..=16).contains(&r.0);
        let a_reg = self.operand_reg(&args[0]).filter(|r| is_gpr(*r));
        let b_reg = self.operand_reg(&args[1]).filter(|r| is_gpr(*r));
        let (a, b) = match (a_reg, b_reg) {
            (Some(a), Some(b)) => (phys_reg_name(a).to_string(), phys_reg_name(b).to_string()),
            (Some(a), None) => {
                self.operand_to_reg(&args[1], "rax");
                (phys_reg_name(a).to_string(), "rax".to_string())
            }
            (None, Some(b)) => {
                self.operand_to_reg(&args[0], "rax");
                ("rax".to_string(), phys_reg_name(b).to_string())
            }
            (None, None) => {
                self.operand_to_reg(&args[0], "rax");
                self.state.emit("    movq %rax, %rdi");
                self.operand_to_reg(&args[1], "rax");
                ("rdi".to_string(), "rax".to_string())
            }
        };
        if f64_lanes {
            self.state
                .emit_fmt(format_args!("    vmovupd (%{}), %ymm0", a));
            self.state
                .emit_fmt(format_args!("    vsubpd (%{}), %ymm0, %ymm0", b));
            self.state.emit("    vmulpd %ymm0, %ymm0, %ymm0");
            self.state.emit("    vextractf128 $1, %ymm0, %xmm1");
            self.state.emit("    vaddpd %xmm1, %xmm0, %xmm0");
            self.state.emit("    vshufpd $1, %xmm0, %xmm0, %xmm1");
            self.state.emit("    vaddsd %xmm1, %xmm0, %xmm0");
        } else {
            self.state
                .emit_fmt(format_args!("    vmovups (%{}), %ymm0", a));
            self.state
                .emit_fmt(format_args!("    vsubps (%{}), %ymm0, %ymm0", b));
            self.state.emit("    vmulps %ymm0, %ymm0, %ymm0");
            self.state.emit("    vextractf128 $1, %ymm0, %xmm1");
            self.state.emit("    vaddps %xmm1, %xmm0, %xmm0");
            self.state.emit("    vshufpd $1, %xmm0, %xmm0, %xmm1");
            self.state.emit("    vaddps %xmm1, %xmm0, %xmm0");
            self.state.emit("    vmovshdup %xmm0, %xmm1");
            self.state.emit("    vaddss %xmm1, %xmm0, %xmm0");
        }
        self.state.emit("    vzeroupper");
        // Cleaned here; a later 256-bit op re-arms the epilogue emission.
        self.state.dirty_upper_ymm = false;
        self.state.direct_fp_result = Some(dest.0);
    }

    /// Emit a contract-legal affine map as `input * scale + bias`.  Broadcast
    /// scale/bias values normally have assigned YMM families, while the packed
    /// input streams through ymm0 from the preceding load.
    ///
    /// Scratch discipline: %ymm0/%ymm1 are RESERVED (never RA-assigned — the
    /// XMM pool starts at xmm2), so they are the only safe scratch registers.
    /// %ymm2..%ymm15 are assigned homes whose contents stay live across loop
    /// iterations: using one as scratch clobbers a live vector and every
    /// following iteration reads the wrong broadcast (the map-tree kernels
    /// with non-broadcast FMA operands exposed exactly this). The fallback
    /// therefore uses each operand's assigned home directly, a slot-homed
    /// operand as the memory source, and copies at most ONE operand through
    /// %ymm1.
    fn emit_avx_map_fma(&mut self, dest: &Value, args: &[Operand], mnemonic: &str) {
        let folded = self
            .state
            .pending_vec_memfold
            .as_ref()
            .map(|(pv, _, _)| *pv)
            .filter(|pv| args.iter().any(|a| matches!(a, Operand::Value(v) if v.0 == *pv)));
        self.emit_avx_map_fma_inner(dest, args, mnemonic);
        if folded.is_some() {
            self.state.pending_vec_memfold = None;
        }
    }

    fn emit_avx_map_fma_inner(&mut self, dest: &Value, args: &[Operand], mnemonic: &str) {
        assert!(args.len() == 3, "{} expects input, scale, bias", mnemonic);
        // VLFOLD forms (ICX saxpy shape). With the scale in an XMM home:
        //   bias elided : input streams in %ymm0 → `vfmadd213 mem, %s, %ymm0`
        //                 (= s*input + mem)
        //   input elided: bias  streams in %ymm0 → `vfmadd231 mem, %s, %ymm0`
        //                 (= s*mem + bias)
        if let (Some((pv, mem, _)), Operand::Value(m0), Operand::Value(m1), Operand::Value(bias)) = (
            self.state.pending_vec_memfold.clone(),
            &args[0],
            &args[1],
            &args[2],
        ) {
            // The multiplicands commute: whichever of args[0]/args[1] has an
            // XMM home (the loop-invariant broadcast) is the register source,
            // the other one is the streamed element vector.
            let home = |this: &Self, v: &Value| {
                this.reg_assignments
                    .get(&v.0)
                    .copied()
                    .filter(|r| is_xmm_reg(*r))
            };
            let (scale_reg, streamed_mul) = match (home(self, m0), home(self, m1)) {
                (Some(r), None) => (Some(r), Some((m1, &args[1]))),
                (None, Some(r)) => (Some(r), Some((m0, &args[0]))),
                _ => (None, None),
            };
            if let (Some(scale_reg), Some((streamed, streamed_arg))) = (scale_reg, streamed_mul) {
                let form = if bias.0 == pv {
                    Some((mnemonic.replace("132", "213"), streamed_arg))
                } else if streamed.0 == pv {
                    Some((mnemonic.replace("132", "231"), &args[2]))
                } else {
                    None
                };
                if let Some((form, streamed)) = form {
                    self.avx_load_arg(streamed);
                    self.state.emit_fmt(format_args!(
                        "    {} {}, %{}, %ymm0",
                        form,
                        mem,
                        phys_reg_name_256(scale_reg)
                    ));
                    self.state.pending_vec_memfold = None;
                    self.state.vec_last_store_reg = false;
                    self.avx_store_dest(dest);
                    return;
                }
            }
        }
        if let (Operand::Value(input), Operand::Value(scale), Operand::Value(bias)) =
            (&args[0], &args[1], &args[2])
        {
            let input_held = self.state.vec_last_store_reg
                && self.state.vec_last_store_val == Some(input.0)
                && self.state.vec_last_store_reg_name == Some("ymm0");
            let scale_reg = self
                .reg_assignments
                .get(&scale.0)
                .copied()
                .filter(|r| is_xmm_reg(*r));
            let bias_reg = self
                .reg_assignments
                .get(&bias.0)
                .copied()
                .filter(|r| is_xmm_reg(*r));
            if let (true, Some(scale_reg), Some(bias_reg)) = (input_held, scale_reg, bias_reg) {
                self.state.emit_fmt(format_args!(
                    "    {} %{}, %{}, %ymm0",
                    mnemonic,
                    phys_reg_name_256(scale_reg),
                    phys_reg_name_256(bias_reg)
                ));
                if self.state.pending_vec_store.map(|(p, _, _)| p) == Some(input.0) {
                    self.state.pending_vec_store = None;
                }
                self.state.vec_last_store_reg = false;
                self.avx_store_dest(dest);
                return;
            }
        }

        // Fallback: the input streams through %ymm0. Every operand source is
        // chosen so that no RA-assigned home (%ymm2..%ymm15) is ever written:
        // homed operands are read directly as VEX sources, a slot-homed
        // operand is read as the memory source (legal in the multiplier
        // position), and at most ONE operand is copied through the reserved
        // %ymm1.
        self.avx_load_arg(&args[0]);

        let operand_reg_source = |this: &Self, arg: &Operand| -> Option<String> {
            let Operand::Value(v) = arg else {
                return None;
            };
            if let Some(&reg) = this.reg_assignments.get(&v.0) {
                if is_xmm_reg(reg) {
                    return Some(format!("%{}", phys_reg_name_256(reg)));
                }
                return None;
            }
            // A value provably sitting in a non-reserved YMM register within
            // this block can be read directly.
            if let Some(&held) = this.state.vec_live_regs.get(&v.0) {
                if held != "ymm0" && held != "ymm1" {
                    return Some(format!("%{}", held));
                }
                return None;
            }
            None
        };

        let scale_reg = operand_reg_source(self, &args[1]);
        let bias_reg = operand_reg_source(self, &args[2]);

        match (scale_reg, bias_reg) {
            (Some(scale), Some(bias)) => {
                self.state
                    .emit_fmt(format_args!("    {} {}, {}, %ymm0", mnemonic, scale, bias));
                self.state.vec_last_store_reg = false;
                self.avx_store_dest(dest);
            }
            (Some(scale), None) => {
                self.avx_load_arg_to(&args[2], "ymm1");
                self.state
                    .emit_fmt(format_args!("    {} {}, %ymm1, %ymm0", mnemonic, scale));
                self.state.vec_last_store_reg = false;
                self.avx_store_dest(dest);
            }
            (None, Some(bias)) => {
                self.avx_load_arg_to(&args[1], "ymm1");
                self.state
                    .emit_fmt(format_args!("    {} %ymm1, {}, %ymm0", mnemonic, bias));
                self.state.vec_last_store_reg = false;
                self.avx_store_dest(dest);
            }
            (None, None) => {
                // Neither operand homed: the bias is copied through %ymm1 and
                // the scale is read from memory (the multiplier position
                // accepts a memory source). After `avx_load_arg(input)` every
                // non-homed operand is slot-homed — a deferred register store
                // was flushed to its slot by the input load — so the memory
                // operand always exists. (A value tracked in the reserved
                // %ymm0/%ymm1 pair cannot survive the input load either.)
                // Bias first: the scale's memory operand is unaffected.
                self.avx_load_arg_to(&args[2], "ymm1");
                let Operand::Value(sv) = &args[1] else {
                    unreachable!("vfmadd132 scale operand must be a value");
                };
                let scale_mem = self.value_ptr_mem_operand(sv.0).unwrap_or_else(|| {
                    unreachable!("vfmadd132 scale operand must be homed, tracked, or slot-homed")
                });
                self.state
                    .emit_fmt(format_args!("    {} {}, %ymm1, %ymm0", mnemonic, scale_mem));
                self.state.vec_last_store_reg = false;
                self.avx_store_dest(dest);
            }
        }
    }

    /// Emit one fused AVX reduction step directly from two memory streams:
    /// `dest = acc + load(a_base+a_off) * load(b_base+b_off)`.
    ///
    /// Keeping one multiplicand in YMM0 and folding the other into FMA avoids
    /// the old load-A stack home that had to be flushed before load B. The
    /// vectorizer only creates this intrinsic under a fast-contraction contract.
    pub(super) fn emit_avx_reduction_fma(
        &mut self,
        dest: &Value,
        args: &[Operand],
        mnemonic: &str,
    ) {
        assert!(
            args.len() == 5,
            "{} expects accumulator plus two base/offset pairs",
            mnemonic
        );

        let assigned = self
            .reg_assignments
            .get(&dest.0)
            .copied()
            .filter(|r| is_xmm_reg(*r));
        // Fallback scratch MUST come from the reserved pair (ymm0/ymm1):
        // ymm2 is PhysReg(20), the FIRST allocatable SIMD register — using it
        // as scratch clobbers whichever live value the allocator parked
        // there whenever this dest itself missed allocation. ymm0 carries
        // the A-stream load below, so the accumulator staging uses ymm1.
        let target = assigned.map(phys_reg_name_256).unwrap_or("ymm1");
        let acc_same = match (&args[0], assigned) {
            (Operand::Value(acc), Some(reg)) => {
                self.reg_assignments.get(&acc.0).is_some_and(|r| *r == reg)
            }
            _ => false,
        };
        if !acc_same {
            self.avx_load_arg_to(&args[0], target);
        }

        let (a_base, a_index) = self.vec_load_addr_regs(&args[1], &args[2]);
        match a_index {
            Some(index) => self
                .state
                .emit_fmt(format_args!("    vmovdqu (%{},%{}), %ymm0", a_base, index)),
            None => self
                .state
                .emit_fmt(format_args!("    vmovdqu (%{}), %ymm0", a_base)),
        }
        let (b_base, b_index) = self.vec_load_addr_regs(&args[3], &args[4]);
        match b_index {
            Some(index) => self.state.emit_fmt(format_args!(
                "    {} (%{},%{}), %ymm0, %{}",
                mnemonic, b_base, index, target
            )),
            None => self.state.emit_fmt(format_args!(
                "    {} (%{}), %ymm0, %{}",
                mnemonic, b_base, target
            )),
        }

        self.state.vector_values.insert(dest.0);
        if let Some(reg) = assigned {
            let name = phys_reg_name_256(reg);
            self.state.vec_live_regs.insert(dest.0, name);
            self.state.vec_last_store_val = Some(dest.0);
            self.state.vec_last_store_reg = true;
            self.state.vec_last_store_reg_name = Some(name);
        } else {
            // target is the reserved ymm1 here (assigned==None).
            self.state
                .emit_fmt(format_args!("    vmovdqa %{}, %ymm0", target));
            self.avx_store_dest(dest);
        }
    }

    /// Emit AVX binary 256-bit op: load ymm0 from arg0 ptr, ymm1 from arg1 ptr,
    /// apply the given AVX instruction, store result ymm0 to dest_ptr.
    pub(super) fn emit_avx_binary_256(
        &mut self,
        dest_ptr: &Value,
        args: &[Operand],
        avx_inst: &str,
        commutative: bool,
    ) {
        let folded = self
            .state
            .pending_vec_memfold
            .as_ref()
            .map(|(pv, _, _)| *pv)
            .filter(|pv| args.iter().any(|a| matches!(a, Operand::Value(v) if v.0 == *pv)));
        self.emit_avx_binary_256_inner(dest_ptr, args, avx_inst, commutative);
        // VLFOLD: every path of the consumer either used the memory operand
        // or materialised the load; the elided value is consumed now.
        if folded.is_some() {
            self.state.pending_vec_memfold = None;
        }
    }

    fn emit_avx_binary_256_inner(
        &mut self,
        dest_ptr: &Value,
        args: &[Operand],
        avx_inst: &str,
        commutative: bool,
    ) {
        // VLFOLD register forms. `acc' = op(acc, load)` with acc/acc' in one
        // YMM family becomes the single ICX reduction instruction
        // `op mem, %ymmA, %ymmA`; a homed non-accumulator source (map
        // broadcast invariant) gives `op mem, %ymmS, %ymmD` for a homed
        // destination or `op mem, %ymmS, %ymm0` + home store otherwise —
        // never a per-iteration `vmovdqa %ymmS, %ymm1` copy.
        if let Some((pv, mem, _)) = self.state.pending_vec_memfold.clone() {
            if let (Operand::Value(x), Operand::Value(y)) = (&args[0], &args[1]) {
                let other = if y.0 == pv {
                    Some(x)
                } else if x.0 == pv && commutative {
                    Some(y)
                } else {
                    None
                };
                let other_reg = other
                    .and_then(|o| self.reg_assignments.get(&o.0).copied())
                    .filter(|r| is_xmm_reg(*r));
                if let Some(oreg) = other_reg {
                    let src = phys_reg_name_256(oreg);
                    self.state.dirty_upper_ymm = true;
                    let dest_home = self
                        .reg_assignments
                        .get(&dest_ptr.0)
                        .copied()
                        .filter(|r| is_xmm_reg(*r));
                    if let Some(dest_reg) = dest_home {
                        let dst = phys_reg_name_256(dest_reg);
                        self.state.emit_fmt(format_args!(
                            "    {} {}, %{}, %{}",
                            avx_inst, mem, src, dst
                        ));
                        self.state.vec_live_regs.insert(dest_ptr.0, dst);
                        self.state.vec_last_store_val = Some(dest_ptr.0);
                        self.state.vec_last_store_reg = true;
                        self.state.vec_last_store_reg_name = Some(dst);
                        self.state.reg_cache.invalidate_acc();
                    } else {
                        // %ymm0 may hold a deferred value of a different def:
                        // commit it before overwriting the scratch register.
                        self.flush_pending_vec_store_impl();
                        self.state.emit_fmt(format_args!(
                            "    {} {}, %{}, %ymm0",
                            avx_inst, mem, src
                        ));
                        self.state.vec_last_store_reg = false;
                        self.avx_store_dest(dest_ptr);
                    }
                    self.state.pending_vec_memfold = None;
                    return;
                }
            }
        }

        // In AT&T VEX syntax only the first textual source may be memory.
        // Preserve operand order for non-commutative operations.
        //
        // A value still provably in a register (last-stored/deferred) must NOT
        // be folded as a memory operand — its slot contents may be stale under
        // the deferred-store optimization. The loaders route those through
        // the register cache instead. This also removes the old slot-only
        // reuse check, which could fire across coalesced slots holding
        // DIFFERENT values (latent miscompile class).
        assert!(
            args.len() >= 2,
            "emit_avx_binary_256: malformed intrinsic {} ({} args)",
            avx_inst,
            args.len()
        );

        // Direct loop-accumulator update. The allocator coalesces args[0], the
        // backedge result, and dest into one YMM family. args[1] is commonly a
        // deferred load/product still in ymm0, so the three-operand VEX form
        // can update the accumulator without any register-renaming moves.
        if let Some(&dest_reg) = self.reg_assignments.get(&dest_ptr.0) {
            if is_xmm_reg(dest_reg) {
                if let (Operand::Value(acc), Operand::Value(fresh)) = (&args[0], &args[1]) {
                    let acc_same = self
                        .reg_assignments
                        .get(&acc.0)
                        .is_some_and(|r| *r == dest_reg);
                    let fresh_held = self.state.vec_last_store_reg
                        && self.state.vec_last_store_val == Some(fresh.0);
                    if acc_same && fresh_held {
                        let held = self.state.vec_last_store_reg_name.unwrap_or("ymm0");
                        let target = phys_reg_name_256(dest_reg);
                        self.state.emit_fmt(format_args!(
                            "    {} %{}, %{}, %{}",
                            avx_inst, held, target, target
                        ));
                        if self.state.pending_vec_store.map(|(p, _, _)| p) == Some(fresh.0) {
                            self.state.pending_vec_store = None;
                        }
                        self.state.vec_live_regs.insert(dest_ptr.0, target);
                        self.state.vec_last_store_val = Some(dest_ptr.0);
                        self.state.vec_last_store_reg = true;
                        self.state.vec_last_store_reg_name = Some(target);
                        return;
                    }
                }
            }
        }

        // All-homed fast path (OP-05a stencils): when BOTH operands and the
        // destination carry XMM homes (defer-overflow promotion), the
        // three-operand VEX form computes register-to-register with no
        // `%ymm0/%ymm1` staging at all: `op %ymmS, %ymmR, %ymmD`.
        if let (Operand::Value(a0), Operand::Value(a1)) = (&args[0], &args[1]) {
            if let (Some(&r0), Some(&r1)) = (
                self.reg_assignments.get(&a0.0),
                self.reg_assignments.get(&a1.0),
            ) {
                if let Some(&rd) = self.reg_assignments.get(&dest_ptr.0) {
                    if is_xmm_reg(r0) && is_xmm_reg(r1) && is_xmm_reg(rd) {
                        let n0 = phys_reg_name_256(r0);
                        let n1 = phys_reg_name_256(r1);
                        let nd = phys_reg_name_256(rd);
                        self.state
                            .emit_fmt(format_args!("    {} %{}, %{}, %{}", avx_inst, n1, n0, nd));
                        self.state.dirty_upper_ymm = true;
                        self.state.vec_live_regs.insert(dest_ptr.0, nd);
                        self.state.vec_last_store_val = Some(dest_ptr.0);
                        self.state.vec_last_store_reg = true;
                        self.state.vec_last_store_reg_name = Some(nd);
                        self.state.reg_cache.invalidate_acc();
                        return;
                    }
                }
            }
        }

        // Map kernels keep loop-invariant broadcasts in assigned YMM families
        // and stream the current element value through %ymm0.  Use that family
        // directly as the VEX source instead of copying it through %ymm1 on
        // every iteration.
        if let (Operand::Value(current), Operand::Value(invariant)) = (&args[0], &args[1]) {
            let current_held = self.state.vec_last_store_reg
                && self.state.vec_last_store_val == Some(current.0)
                && self.state.vec_last_store_reg_name == Some("ymm0");
            if current_held {
                if let Some(&reg) = self.reg_assignments.get(&invariant.0) {
                    if is_xmm_reg(reg) {
                        self.state.emit_fmt(format_args!(
                            "    {} %{}, %ymm0, %ymm0",
                            avx_inst,
                            phys_reg_name_256(reg)
                        ));
                        if self.state.pending_vec_store.map(|(p, _, _)| p) == Some(current.0) {
                            self.state.pending_vec_store = None;
                        }
                        self.state.vec_last_store_reg = false;
                        self.avx_store_dest(dest_ptr);
                        return;
                    }
                }
            }
        }

        let mem_of = |this: &Self, arg: &Operand| -> Option<String> {
            if let Some(mem) = this.memfold_operand(arg) {
                return Some(mem);
            }
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
        // Commutative fast path: args[1] is the last-stored/deferred value
        // (still in %ymm0) and args[0] is a foldable memory operand. Then
        // `op m0, %ymm0, %ymm0` computes dst = ymm0 op m0 with no register
        // rename (saves one vmovdqa per reduction iteration). The deferred
        // value flows into dst through %ymm0, so its pending slot store is
        // void — exactly the same consumption contract as avx_load_arg_to.
        if commutative
            && self.state.vec_last_store_reg_name == Some("ymm0")
            && matches!(&args[1], Operand::Value(v)
                if self.state.vec_last_store_reg && self.state.vec_last_store_val == Some(v.0))
        {
            if let Some(m0) = mem_of(self, &args[0]) {
                self.state
                    .emit_fmt(format_args!("    {} {}, %ymm0, %ymm0", avx_inst, m0));
                if let Operand::Value(v) = &args[1] {
                    if self.state.pending_vec_store.map(|(p, _, _)| p) == Some(v.0) {
                        self.state.pending_vec_store = None;
                    }
                }
                self.state.vec_last_store_reg = false;
                self.avx_store_dest(dest_ptr);
                return;
            }
        }
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
